//! Mixed-load soak. Each durable JSONL row is independently recoverable; resuming
//! starts a new isolated host segment and cannot certify uninterrupted uptime.
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail, ensure};
use serde_json::{Value, json};

use crate::faux::{self, Blackhole, Faux};
use crate::fixture;
use crate::host::{self, Host, HostConfig};
use crate::scenario::{LONG, SHORT, open_live, project};

fn percentile(values: &[f64]) -> Option<f64> {
    let mut values = values.to_vec();
    values.sort_by(f64::total_cmp);
    (!values.is_empty())
        .then(|| values[(values.len() - values.len() / 100 - 1).min(values.len() - 1)])
}

/// Least-squares slope, normalized by mean RSS in the fitted half.
fn slope(points: &[(f64, f64)]) -> Option<f64> {
    if points.len() < 2 {
        return None;
    }
    let n = crate::count_float(points.len());
    let x = points.iter().map(|p| p.0).sum::<f64>() / n;
    let y = points.iter().map(|p| p.1).sum::<f64>() / n;
    let variance = points.iter().map(|p| (p.0 - x).powi(2)).sum::<f64>();
    (variance > 0.0 && y > 0.0).then(|| {
        points.iter().map(|p| (p.0 - x) * (p.1 - y)).sum::<f64>() / variance * 3600.0 / y * 100.0
    })
}

fn append(file: &mut std::fs::File, rows: &mut Vec<Value>, row: Value) -> Result<()> {
    serde_json::to_writer(&mut *file, &row)?;
    file.write_all(b"\n")?;
    file.sync_data()?;
    rows.push(row);
    Ok(())
}

// Recover a torn final append only; malformed complete rows are errors.
fn recover(path: &Path) -> Result<Vec<Value>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let bytes = std::fs::read(path)?;
    let complete = bytes.iter().rposition(|b| *b == b'\n').map_or(0, |i| i + 1);
    let rows = bytes[..complete]
        .split(|b| *b == b'\n')
        .filter(|b| !b.is_empty())
        .map(serde_json::from_slice)
        .collect::<std::result::Result<Vec<Value>, _>>()?;
    std::fs::OpenOptions::new()
        .write(true)
        .open(path)?
        .set_len(complete as u64)?;
    Ok(rows)
}

fn proc_counts(pid: u32) -> (usize, u64) {
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd")).map_or(0, Iterator::count);
    let threads = std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Threads:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|s| s.parse().ok())
        })
        .unwrap_or(0);
    (fds, threads)
}

fn alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| {
        s.rsplit_once(')')
            .is_some_and(|(_, rest)| !rest.trim_start().starts_with('Z'))
    })
}

// Read only the newly appended log bytes; no transcript/credential content in results.
fn allocator(log: &Path, offset: &mut u64) -> Option<String> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::File::open(log).ok()?;
    file.seek(SeekFrom::Start(*offset)).ok()?;
    let mut reader = std::io::BufReader::new(file);
    let mut line = String::new();
    let mut last = None;
    while reader.read_line(&mut line).ok()? > 0 {
        if line.starts_with("memory_diagnostics ") {
            last = Some(line.trim().to_owned());
        }
        line.clear();
    }
    *offset = reader.stream_position().ok()?;
    last
}

fn workload(
    host: &Host,
    ids: &[String],
    stop: &AtomicBool,
    tx: &mpsc::Sender<Value>,
) -> Result<()> {
    let mut cycle = 0usize;
    while !stop.load(Ordering::Relaxed) {
        // Alternate both long chats with a short chat. Long chats remain unviewed
        // for >300 s between visits, exercising the normal idle eviction deadline.
        let slot = if cycle.is_multiple_of(10) {
            (cycle / 10) % 2
        } else {
            2
        };
        let stream = open_live(host, &ids[slot])?;
        stream.request(
            &json!({"type":"set_model", "provider":"bench", "modelId":faux::MODEL}),
            SHORT,
        )?;
        let at = Instant::now();
        let heavy = cycle.is_multiple_of(12);
        let message = if heavy {
            "BENCH:tools n=6 delay_ms=500 [b:soak-heavy]"
        } else {
            "BENCH:stream secs=4 tps=25 [b:soak-stream]"
        };
        stream.request(&json!({"type":"prompt", "message":message}), LONG)?;
        if cycle % 4 == 1 {
            stream.request(
                &json!({"type":"follow_up", "message":"BENCH:stream secs=1 tps=10 [b:soak-queue]"}),
                SHORT,
            )?;
            stream.request(
                &json!({"type":"steer", "message":"Keep the response concise."}),
                SHORT,
            )?;
        }
        stream
            .wait_marker(&["agent_settled"], at, LONG)
            .context("soak turn did not settle")?;
        ensure!(
            !stream.saw("error", at) && !stream.saw("crashed", at),
            "turn emitted error/crashed"
        );
        let tool_ends = stream
            .markers_since(at)
            .iter()
            .filter(|(n, _)| n == "tool_execution_end")
            .count();
        ensure!(
            !heavy || tool_ends > 0,
            "extension-heavy turn executed no tools"
        );
        tx.send(json!({"kind":"turn", "slot":slot, "cycle":cycle, "latency_ms":at.elapsed().as_secs_f64()*1000.0,
            "queued_and_steered":cycle%4==1, "tool_ends":tool_ends, "deltas":stream.counters().2}))?;
        drop(stream); // App disconnect; next open reconnects, without forcing eviction.
        cycle += 1;
        let pause = 20 + (cycle as u64 * 17 % 41);
        for _ in 0..pause * 10 {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    Ok(())
}

/// `host soak --hours N --host-bin PATH [--out DIR] [--resume] [--real PATH]`.
/// A sub-eight-hour run is a validation only, never an eight-hour certification.
#[allow(clippy::print_stdout, reason = "benchmark summary")]
#[allow(
    clippy::too_many_lines,
    reason = "Linear measurement protocol keeps setup, observation windows, and cleanup ordered; splitting changes benchmark review context."
)]
pub(crate) fn run(args: &[String]) -> Result<()> {
    let mut hours = 8.0f64;
    let mut bin = std::env::var_os("OINO_BENCH_HOST_BIN").map(PathBuf::from);
    let mut out = PathBuf::from("bench/records/soak");
    let mut real = fixture::real_source();
    let mut resume = false;
    let mut flags = args.iter();
    while let Some(flag) = flags.next() {
        if flag == "--resume" {
            resume = true;
            continue;
        }
        let value = flags
            .next()
            .with_context(|| format!("{flag} needs a value"))?;
        match flag.as_str() {
            "--hours" => hours = value.parse()?,
            "--host-bin" => bin = Some(value.into()),
            "--out" => out = value.into(),
            "--real" => real = Some(value.into()),
            _ => bail!("unknown soak option {flag}"),
        }
    }
    ensure!(
        hours.is_finite() && hours > 0.0 && hours <= 168.0,
        "--hours must be in (0,168]"
    );
    let bin = std::fs::canonicalize(bin.context("--host-bin required")?)?;
    let real = real.context("soak requires the real 72 MB fixture (--real)")?;
    std::fs::create_dir_all(&out)?;
    let journal = out.join("samples.jsonl");
    ensure!(
        resume || !journal.exists(),
        "output already exists; use --resume or a fresh --out"
    );
    let mut rows = recover(&journal)?;
    let segments = rows.iter().filter(|r| r["kind"] == "start").count();
    let prior = rows
        .iter()
        .filter_map(|r| r["elapsed_s"].as_f64())
        .fold(0.0, f64::max);
    let duration = hours * 3600.0;
    ensure!(duration > prior, "requested duration already recorded");
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&journal)?;
    let faux = Faux::start()?;
    let blackhole = Blackhole::start()?;
    let work = tempfile::Builder::new().prefix("oino-soak-").tempdir()?;
    let cfg = HostConfig {
        bin,
        root: work.path().join("run"),
        dist_extensions: std::env::var_os("OINO_DIST_EXTENSIONS")
            .map_or_else(|| "/usr/share/oino/extensions".into(), PathBuf::from),
        faux_port: faux.port,
        refused_port: faux::refused_port(),
        blackhole_port: blackhole.port,
        warm_cache: work.path().join("cache"),
    };
    host::seed(&cfg)?;
    let cwd = project(&cfg, "soak")?;
    let large = fixture::generate(&cfg.root.join("agent"), &cwd, 40_000, 3)?;
    let real = fixture::copy_real(&cfg.root.join("agent"), &project(&cfg, "real")?, &real)?;
    let small = fixture::generate(&cfg.root.join("agent"), &project(&cfg, "short")?, 100, 1)?;
    // OINO_MEMORY_DIAGNOSTICS=1 on the benchmark command enables the glibc
    // host's existing diagnostic thread; unsupported allocators stay null.
    let host = Host::start(&cfg)?;
    append(
        &mut file,
        &mut rows,
        json!({"kind":"start", "segment":segments, "elapsed_s":prior,
        "date":chrono::Utc::now().to_rfc3339(), "pid":host.pid, "hours":hours,
        "host_bin":cfg.bin, "dist":cfg.dist_extensions, "commit":std::env::var("OINO_BENCH_GIT_HEAD").ok(),
        "sampling_s":30, "ping_interval_ms":100, "real_bytes":std::fs::metadata(&real.path)?.len(),
        "interrupted_resume":segments>0}),
    )?;
    let started = Instant::now();
    let stop = AtomicBool::new(false);
    let mut died = false;
    let mut log_offset = 0;
    let (tx, rx) = mpsc::channel();
    let ids = vec![large.session_id, real.session_id, small.session_id];
    let measured = std::thread::scope(|scope| -> Result<()> {
        let worker = scope.spawn(|| {
            if let Err(e) = workload(&host, &ids, &stop, &tx) {
                let _ = tx.send(json!({"kind":"error", "error":format!("{e:#}")}));
            }
        });
        let measured = (|| -> Result<()> {
            let mut app = Vec::new();
            let mut live = Vec::new();
            let mut errors = Vec::new();
            let mut next = 30.0;
            while prior + started.elapsed().as_secs_f64() < duration {
                let tick = Instant::now();
                if !alive(host.pid) {
                    died = true;
                    append(
                        &mut file,
                        &mut rows,
                        json!({"kind":"host_died", "elapsed_s":prior+started.elapsed().as_secs_f64()}),
                    )?;
                    break;
                }
                for (name, result, values) in [
                    ("app", host.socks.ping_app(Duration::from_secs(2)), &mut app),
                    (
                        "live",
                        host.socks.ping_live(Duration::from_secs(2)),
                        &mut live,
                    ),
                ] {
                    match result {
                        Ok(d) => values.push(d.as_secs_f64() * 1000.0),
                        Err(e) => errors.push(format!("{name}: {e:#}")),
                    }
                }
                let mut failed = false;
                for mut event in rx.try_iter() {
                    event["elapsed_s"] = json!(prior + started.elapsed().as_secs_f64());
                    failed |= event["kind"] == "error";
                    append(&mut file, &mut rows, event)?;
                }
                let elapsed = started.elapsed().as_secs_f64();
                if elapsed >= next || failed || prior + elapsed + 0.1 >= duration {
                    let snap = host::snap(host.pid);
                    let mut pids = host::descendants(host.pid);
                    pids.push(host.pid);
                    let (fds, threads) = pids
                        .iter()
                        .map(|pid| proc_counts(*pid))
                        .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
                    append(
                        &mut file,
                        &mut rows,
                        json!({"kind":"sample", "elapsed_s":prior+elapsed,
                        "host_rss_kib":snap.host_rss_kib, "total_rss_kib":snap.total_rss_kib,
                        "processes":pids.len(), "fds":fds, "threads":threads,
                        "app_p99_ms":percentile(&app), "live_p99_ms":percentile(&live),
                        "app_n":app.len(), "live_n":live.len(), "ping_errors":errors,
                        "jemalloc":allocator(&host.root.join("host.out"), &mut log_offset)}),
                    )?;
                    app.clear();
                    live.clear();
                    errors.clear();
                    next += 30.0;
                }
                if failed {
                    break;
                }
                std::thread::sleep(Duration::from_millis(100).saturating_sub(tick.elapsed()));
            }
            Ok(())
        })();
        stop.store(true, Ordering::Relaxed);
        worker
            .join()
            .map_err(|_| anyhow::anyhow!("load worker panicked"))?;
        measured
    });
    // A turn can finish (or fail) while the sampler is stopping. Never lose its
    // final event, or turn a sampler failure into an absent summary.
    if let Err(error) = measured {
        append(
            &mut file,
            &mut rows,
            json!({"kind":"error",
            "elapsed_s":prior+started.elapsed().as_secs_f64(), "error":format!("{error:#}")}),
        )?;
    }
    for mut event in rx.try_iter() {
        event["elapsed_s"] = json!(prior + started.elapsed().as_secs_f64());
        append(&mut file, &mut rows, event)?;
    }
    if !died && !alive(host.pid) {
        died = true;
        append(
            &mut file,
            &mut rows,
            json!({"kind":"host_died",
            "elapsed_s":prior+started.elapsed().as_secs_f64()}),
        )?;
    }
    // Keep logs even on early workload failure. Host::stop only stops owned PIDs.
    std::fs::copy(
        host.root.join("host.out"),
        out.join(format!("host-{segments}.log")),
    )?;
    host.stop();
    let elapsed = prior + started.elapsed().as_secs_f64();
    append(
        &mut file,
        &mut rows,
        json!({"kind":"end", "elapsed_s":elapsed, "host_died":died}),
    )?;
    let samples: Vec<_> = rows.iter().filter(|r| r["kind"] == "sample").collect();
    let points: Vec<_> = samples
        .iter()
        .filter_map(|r| Some((r["elapsed_s"].as_f64()?, r["total_rss_kib"].as_f64()?)))
        .filter(|p| p.0 >= duration / 2.0)
        .collect();
    let growth = slope(&points);
    let pings_ok = !samples.is_empty()
        && samples.iter().all(|r| {
            ["app_p99_ms", "live_p99_ms"]
                .iter()
                .all(|k| r[k].as_f64().is_some_and(|v| v <= 50.0))
                && r["ping_errors"].as_array().is_some_and(Vec::is_empty)
        });
    let turns = rows.iter().filter(|r| r["kind"] == "turn").count();
    let errors = rows
        .iter()
        .filter(|r| r["kind"] == "error" || r["kind"] == "host_died")
        .count();
    let pass = elapsed >= duration
        && samples
            .last()
            .and_then(|r| r["elapsed_s"].as_f64())
            .is_some_and(|last| last >= duration - 1.0)
        && segments == 0
        && !died
        && errors == 0
        && turns > 0
        && pings_ok
        && growth.is_some_and(|s| s <= 10.0);
    let summary = json!({"pass":pass, "eight_hour_certified":pass && hours>=8.0, "requested_hours":hours,
        "elapsed_s":elapsed, "segments":segments+1, "samples":samples.len(), "turns":turns,
        "errors":errors, "ping_windows_pass":pings_ok, "second_half_rss_pct_per_hour":growth,
        "slope_definition":"OLS RSS/time, divided by second-half mean RSS", "rows":rows});
    std::fs::write(
        out.join("summary.json"),
        serde_json::to_vec_pretty(&summary)?,
    )?;
    let markdown = format!(
        "# Host soak\n\n- Pass: {pass}\n- Eight-hour certified: {}\n- Requested: {hours} h; elapsed: {elapsed:.1} s\n- Samples: {}; turns: {turns}; errors: {errors}\n- T8 all windows ≤50 ms: {pings_ok}\n- Second-half RSS slope: {growth:?} %/h (OLS / mean RSS)\n- Segments: {} (resumes never certify uninterrupted uptime)\n- Allocator telemetry: nullable existing host diagnostics; no injected allocator calls\n\nRaw rows: samples.jsonl; complete record: summary.json; host logs: host-*.log.\n",
        pass && hours >= 8.0,
        samples.len(),
        segments + 1
    );
    std::fs::write(out.join("summary.md"), &markdown)?;
    println!("{markdown}");
    ensure!(
        pass,
        "soak failed (see {})",
        out.join("summary.md").display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slope_and_percentile() -> Result<()> {
        assert_eq!(slope(&[(0.0, 100.0), (3600.0, 100.0)]), Some(0.0));
        assert!(
            (slope(&[(0.0, 100.0), (3600.0, 110.0)]).context("slope missing")? - 1000.0 / 105.0)
                .abs()
                < 1e-9
        );
        assert_eq!(slope(&[(0.0, 100.0)]), None);
        assert_eq!(percentile(&[]), None);
        assert_eq!(
            percentile(&(1..=100).map(f64::from).collect::<Vec<_>>()),
            Some(99.0)
        );
        Ok(())
    }
    #[test]
    fn recovery_discards_only_torn_tail() -> Result<()> {
        let tmp = tempfile::NamedTempFile::new()?;
        std::fs::write(tmp.path(), b"{\"kind\":\"sample\"}\n{\"torn\"")?;
        assert_eq!(recover(tmp.path())?.len(), 1);
        assert_eq!(std::fs::read(tmp.path())?, b"{\"kind\":\"sample\"}\n");
        std::fs::write(tmp.path(), b"broken\n")?;
        assert!(recover(tmp.path()).is_err());
        Ok(())
    }
}
