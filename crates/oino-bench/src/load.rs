//! L1 heavy load (T8, T11), S-idle (T10, T12) and S-crash (T13).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::json;

use crate::fixture;
use crate::host::{self, Host, Sampler};
use crate::record::Metric;
use crate::scenario::{Ctx, LONG, SHORT, log, open_live, pinger, project, turn};
use crate::wire::{Stream, op};

/// Streaming chats in L1.
const L1_CHATS: usize = 10;
/// Subagent-like tool loops in L1.
const L1_LOOPS: usize = 30;
/// Tokens/s of every streaming chat.
const L1_TPS: u32 = 50;

fn mb(kib: u64) -> f64 {
    crate::counter_float(kib) / 1024.0
}

fn new_session(host: &Host, cwd: &std::path::Path, name: &str) -> Result<Stream> {
    Stream::open(
        &host.socks.app,
        &json!({"op": "session_new", "cwd": cwd.to_string_lossy(), "name": name}),
        LONG,
    )
}

/// L1: 10 chats streaming continuously, 30 tool loops over 3 projects (one of them on a
/// 40k-entry chat) and an `events_open` client subscribed to everything, while app.sock
/// and live.sock are pinged every 100 ms. Then 600 s of quiet (quick: 60 s).
#[allow(
    clippy::too_many_lines,
    reason = "Linear measurement protocol keeps setup, observation windows, and cleanup ordered; splitting changes benchmark review context."
)]
pub(crate) fn l1(ctx: &mut Ctx) -> Result<()> {
    log("L1");
    let cfg = ctx.config("l1");
    host::seed(&cfg)?;
    let projects: Vec<_> = (0..3)
        .map(|p| project(&cfg, &format!("proj-{p}")))
        .collect::<Result<_>>()?;
    let big = fixture::generate(&cfg.root.join("agent"), &projects[2], 40_000, 3)?;
    let host = Host::start(&cfg)?;
    let events = Stream::open(&host.socks.app, &json!({"op": "events_open"}), SHORT)?;
    let secs = ctx.load_secs;
    let started = Instant::now();
    let mut streams = Vec::new();
    for i in 0..L1_CHATS {
        let s = new_session(&host, &projects[i % 3], &format!("l1-chat-{i}"))?;
        admit(
            ctx,
            &s,
            &format!("l1-c{i}"),
            &format!("BENCH:stream secs=100000 tps={L1_TPS} [b:l1-c{i}]"),
        )?;
        streams.push(s);
    }
    let big_stream = open_live(&host, &big.session_id)?;
    admit(
        ctx,
        &big_stream,
        "l1-big",
        "BENCH:tools n=100000 delay_ms=300 [b:l1-big]",
    )?;
    streams.push(big_stream);
    for i in 1..L1_LOOPS {
        let s = new_session(&host, &projects[i % 3], &format!("l1-loop-{i}"))?;
        admit(
            ctx,
            &s,
            &format!("l1-t{i}"),
            &format!("BENCH:tools n=100000 delay_ms=300 [b:l1-t{i}]"),
        )?;
        streams.push(s);
    }
    let setup = started.elapsed();
    ctx.note(format!(
        "L1: {} sessions up in {:.1} s",
        streams.len(),
        setup.as_secs_f64()
    ));
    let requests_before = ctx.faux.requests();
    let measured_at = Instant::now();
    let sampler = Sampler::start(host.pid);
    let stop = Arc::new(AtomicBool::new(false));
    let pings = pinger(&host, Duration::from_millis(100), Arc::clone(&stop));
    let mut rss = Vec::new();
    while measured_at.elapsed() < Duration::from_secs(secs) {
        rss.push((
            measured_at.elapsed().as_secs_f64(),
            mb(host::snap(host.pid).total_rss_kib),
        ));
        std::thread::sleep(Duration::from_secs(1));
    }
    let second_half: Vec<_> = rss
        .iter()
        .copied()
        .filter(|(t, _)| *t >= crate::counter_float(secs) / 2.0)
        .collect();
    let growth = growth_per_hour(&second_half);
    ctx.push(Metric::value(
        "l1.second_half.rss_growth_pct_per_hour",
        "T11",
        "%/h",
        growth,
    ));
    ctx.note(format!(
        "L1 total RSS samples (seconds, MiB): {}",
        serde_json::to_string(&rss)?
    ));
    stop.store(true, Ordering::Relaxed);
    let (app, live, failed) = pings
        .join()
        .map_err(|_| anyhow::anyhow!("pinger panicked"))?;
    let (peak, cpu) = sampler.finish(host.pid);
    let requests = ctx.faux.requests() - requests_before;
    let deltas: u64 = streams.iter().map(|s| s.counters().2).sum();
    let max_gap = streams[..L1_CHATS]
        .iter()
        .map(|s| s.counters().4)
        .max()
        .unwrap_or_default();
    let frames_per_s = streams[..L1_CHATS]
        .iter()
        .map(|s| crate::counter_float(s.counters().0) / crate::counter_float(secs))
        .fold(0.0, f64::max);
    let closed = streams.iter().filter(|s| s.closed()).count();
    ctx.push(Metric::latency("l1.ping.app_ms", "T8", "p99", &app));
    ctx.push(Metric::latency("l1.ping.live_ms", "T8", "p99", &live));
    ctx.push(Metric::value(
        "l1.ping.failures",
        "T8",
        "count",
        crate::count_float(failed),
    ));
    ctx.push(Metric::value(
        "l1.host_rss_peak_mb",
        "T11",
        "MB",
        mb(peak.host_rss_kib),
    ));
    ctx.push(Metric::value(
        "l1.total_rss_peak_mb",
        "T11",
        "MB",
        mb(peak.total_rss_kib),
    ));
    ctx.push(Metric::value("l1.host_cpu_pct", "", "%", cpu));
    ctx.push(Metric::value(
        "l1.provider_requests_per_s",
        "",
        "count",
        crate::counter_float(requests) / crate::counter_float(secs),
    ));
    ctx.push(Metric::value(
        "l1.stream_max_frame_gap_ms",
        "",
        "ms",
        max_gap.as_secs_f64() * 1000.0,
    ));
    ctx.push(Metric::value(
        "l1.proxy.stream_frames_per_s_max",
        "",
        "count",
        frames_per_s,
    ));
    ctx.note(format!(
        "L1: {secs} s, {deltas} message_update deltas, {} events on the subscribed client, {closed} streams closed by the host",
        events.counters().0
    ));
    // A low rate still fails if the loops all stop at the repeat guard. Check
    // every loop in both halves, not just aggregate throughput or setup activity.
    let measured_end = measured_at + Duration::from_secs(secs);
    let min_rounds = (secs / 10).max(1) as usize;
    let mut stalled = Vec::new();
    for i in 0..L1_LOOPS {
        let tag = if i == 0 {
            "l1-big".to_owned()
        } else {
            format!("l1-t{i}")
        };
        // Each following provider request proves the previous tool round completed.
        let rounds = ctx.faux.rounds(&tag, measured_at).saturating_sub(1);
        let times = ctx.faux.round_times(&tag, measured_at, measured_end);
        // Include both edges: a loop silent for the final half cannot pass.
        let sustained = sustained(&times, crate::counter_float(secs), 10.0);
        std::fs::write(
            cfg.root.join(format!("loop-{i}.json")),
            serde_json::to_vec(&times)?,
        )?;
        ctx.note(format!(
            "L1 loop {i} request_seconds={}",
            serde_json::to_string(&times)?
        ));
        ctx.push(Metric::value(
            &format!("l1.loop.{i}.rounds"),
            "",
            "count",
            crate::count_float(rounds),
        ));
        if rounds < min_rounds
            || !sustained
            || streams[L1_CHATS + i].closed()
            || streams[L1_CHATS + i].saw("agent_settled", measured_at)
        {
            stalled.push(format!(
                "{tag}: {rounds} rounds (need {min_rounds}), sustained={sustained}"
            ));
        }
    }
    for (i, stream) in streams[..L1_CHATS].iter().enumerate() {
        if stream.closed() || stream.saw("agent_settled", measured_at) {
            stalled.push(format!(
                "streaming chat {i} stopped before the load window ended"
            ));
        }
    }
    ctx.note(format!(
        "L1 requires >= {min_rounds} provider rounds per loop and no request gap (including window edges) above 10 s"
    ));
    // Work stops: abort everything, drop the streams, watch memory come back.
    for s in &streams {
        let _ = s.send(&json!({"type": "abort"}));
    }
    // T12 begins only after every runtime confirms it has no running turn.
    let settling = Instant::now();
    let deadline = settling + LONG;
    for stream in &streams {
        settle(stream, deadline)?;
    }
    ctx.note(format!(
        "L1 all {} runtimes settled after {:.3} s; starting T12 tail",
        streams.len(),
        settling.elapsed().as_secs_f64()
    ));
    drop(streams);
    drop(events);
    std::thread::sleep(Duration::from_secs(ctx.idle_secs));
    let after = host::snap(host.pid);
    // Preserve attribution before stop() destroys the process's mappings. Each
    // invocation must use its own work directory (seed() replaces the old root).
    #[cfg(target_os = "linux")]
    {
        let smaps = std::fs::read_to_string(format!("/proc/{}/smaps", host.pid))?;
        std::fs::write(cfg.root.join("tail.smaps"), &smaps)?;
        std::fs::write(
            cfg.root.join("tail-mappings.json"),
            serde_json::to_vec_pretty(&group_smaps(&smaps))?,
        )?;
        std::fs::write(
            cfg.root.join("tail-processes.json"),
            serde_json::to_vec_pretty(
                &json!({"host": host.pid, "descendants": host::descendants(host.pid)}),
            )?,
        )?;
    }
    ctx.push(Metric::value(
        "l1.after_work.total_rss_mb",
        "T12",
        "MB",
        mb(after.total_rss_kib),
    ));
    host.stop();
    ensure!(
        growth.is_finite() && growth <= 10.0,
        "second-half RSS growth {growth:.2}%/h exceeds 10%/h"
    );
    ensure!(
        stalled.is_empty(),
        "L1 not sustained: {}",
        stalled.join("; ")
    );
    Ok(())
}

#[cfg(target_os = "linux")]
fn group_smaps(
    smaps: &str,
) -> std::collections::BTreeMap<String, std::collections::BTreeMap<String, u64>> {
    let mut groups =
        std::collections::BTreeMap::<String, std::collections::BTreeMap<String, u64>>::new();
    let mut mapping = String::new();
    for line in smaps.lines() {
        let fields: Vec<_> = line.split_whitespace().collect();
        if fields.first().is_some_and(|field| field.contains('-')) {
            mapping = if fields.len() > 5 {
                fields[5..].join(" ")
            } else {
                "[anonymous]".into()
            };
        } else if fields.len() == 3
            && fields[2] == "kB"
            && let Ok(kib) = fields[1].parse::<u64>()
        {
            *groups
                .entry(mapping.clone())
                .or_default()
                .entry(fields[0].trim_end_matches(':').into())
                .or_default() += kib;
        }
    }
    groups
}

#[cfg(all(test, target_os = "linux"))]
mod smaps_tests {
    #[test]
    fn aggregates_anonymous_and_file_mappings_separately() {
        let groups = super::group_smaps(
            "1000-2000 rw-p 0 00:00 0\nRss: 4 kB\nPrivate_Dirty: 4 kB\n2000-3000 rw-p 0 00:00 0\nRss: 8 kB\n3000-4000 r-xp 0 00:01 1 /a file\nRss: 2 kB\n",
        );
        assert_eq!(groups["[anonymous]"]["Rss"], 12);
        assert_eq!(groups["[anonymous]"]["Private_Dirty"], 4);
        assert_eq!(groups["/a file"]["Rss"], 2);
    }
}

/// S-idle: a fresh host does a little work (3 chats in 3 projects, one tool turn each),
/// everything is closed, then RSS is sampled after 600 s (quick: 60 s).
pub(crate) fn s_idle(ctx: &mut Ctx) -> Result<()> {
    log("S-idle");
    let cfg = ctx.config("s-idle");
    host::seed(&cfg)?;
    let host = Host::start(&cfg)?;
    let quiet = Duration::from_secs(if ctx.scale < 1.0 { 10 } else { 20 });
    let empty_a = host::snap(host.pid);
    std::thread::sleep(quiet);
    let empty_b = host::snap(host.pid);
    ctx.push(Metric::value(
        "idle.empty.host_cpu_pct",
        "T10",
        "%",
        host::cpu_pct(empty_a, empty_b),
    ));
    ctx.push(Metric::value(
        "idle.empty.total_rss_mb",
        "",
        "MB",
        mb(empty_b.total_rss_kib),
    ));
    for p in 0..3 {
        let s = new_session(
            &host,
            &project(&cfg, &format!("proj-{p}"))?,
            &format!("idle-{p}"),
        )?;
        turn(&s, &format!("BENCH:tools n=2 [b:idle-{p}]"))?;
        let id = s.session_id();
        drop(s);
        let _ = op(
            &host.socks.app,
            &json!({"op": "session_close", "session": id}),
            SHORT,
        );
    }
    let worked = host::snap(host.pid);
    std::thread::sleep(Duration::from_secs(ctx.idle_secs).saturating_sub(quiet));
    let a = host::snap(host.pid);
    std::thread::sleep(quiet);
    let b = host::snap(host.pid);
    ctx.push(Metric::value(
        "idle.after_work.host_cpu_pct",
        "T10",
        "%",
        host::cpu_pct(a, b),
    ));
    ctx.push(Metric::value(
        "idle.after_work.total_rss_mb",
        "T12",
        "MB",
        mb(b.total_rss_kib),
    ));
    ctx.note(format!(
        "S-idle: RSS empty {:.0} MB → after 3 chats {:.0} MB → {:.0} s later {:.0} MB (host only {:.0} MB)",
        mb(empty_b.total_rss_kib),
        mb(worked.total_rss_kib),
        ctx.idle_secs,
        mb(b.total_rss_kib),
        mb(b.host_rss_kib)
    ));
    host.stop();
    Ok(())
}

/// S-crash: two chats stream and one runs a tool loop; the host is `kill -9`ed mid-turn
/// (today the host is the worker), restarted on the same root, and each chat reopened.
/// Records restart → `host_info`, reopen latency, whether the chat says it was
/// interrupted (`crashed` event or an interrupted marker in the file) and whether it can
/// run a turn again.
pub(crate) fn s_crash(ctx: &mut Ctx) -> Result<()> {
    for down_secs in [0, 4] {
        crash_restart(ctx, down_secs)?;
    }
    Ok(())
}

#[allow(
    clippy::too_many_lines,
    reason = "Linear measurement protocol keeps setup, observation windows, and cleanup ordered; splitting changes benchmark review context."
)]
fn crash_restart(ctx: &mut Ctx, down_secs: u64) -> Result<()> {
    log(&format!("S-crash (down {down_secs} s)"));
    let metrics_at = ctx.metrics.len();
    let cfg = ctx.config(&format!("s-crash-{down_secs}"));
    host::seed(&cfg)?;
    let dir = project(&cfg, "proj-a")?;
    let mut host = Host::start(&cfg)?;
    let mut ids = Vec::new();
    let mut streams = Vec::new();
    for i in 0..3 {
        let s = new_session(&host, &dir, &format!("crash-{i}"))?;
        // A finished first turn materialises the session file.
        turn(&s, &format!("BENCH answer [b:crash-warm-{i}]"))?;
        let msg = if i == 2 {
            "BENCH:tools n=1000 delay_ms=200 [b:crash-t]".to_owned()
        } else {
            format!("BENCH:stream secs=60 tps={L1_TPS} [b:crash-s{i}]")
        };
        s.send(&json!({"type": "prompt", "message": msg}))?;
        ids.push(s.session_id());
        streams.push(s);
    }
    std::thread::sleep(Duration::from_secs(3));
    let orphans_before = host::descendants(host.pid).len();
    host.kill9();
    drop(streams);
    let killed = Instant::now();
    std::thread::sleep(Duration::from_secs(down_secs));
    let restarting = Instant::now();
    host.restart()?;
    ctx.push(Metric::latency(
        "crash.restart_to_ping_ms",
        "T13",
        "max",
        &[restarting.elapsed()],
    ));
    // A reconnect must receive the recovered card without opening or waking a chat.
    // All sessions share a restart→marker deadline (not two seconds per session).
    let deadline = restarting + Duration::from_secs(2);
    let events = Stream::open(&host.socks.app, &json!({"op": "events_open"}), SHORT)?;
    let mut marker_times = Vec::new();
    for id in &ids {
        let name = format!("interrupted:{id}");
        if let Some(marker) = events.wait_marker(
            &[&name],
            killed,
            deadline.saturating_duration_since(Instant::now()),
        ) {
            marker_times.push(marker.at.saturating_duration_since(killed));
        }
    }
    let marked = marker_times.len();
    // Startup recovery fsyncs the marker before publication. Verify its presence in
    // the authoritative JSONL before any session_open can repair it on demand.
    let mut durable = Vec::new();
    collect_session_text(&host.agent_dir.join("sessions"), &mut durable)?;
    let durable_missing = ids
        .iter()
        .filter(|id| {
            !durable.iter().any(|(path, text)| {
                path.contains(id.as_str())
                    && text.lines().any(|line| {
                        serde_json::from_str::<serde_json::Value>(line)
                            .is_ok_and(|v| v["customType"] == "oino.turn-interrupted")
                    })
            })
        })
        .count();
    ctx.push(Metric::latency(
        "crash.restart_to_durable_and_pushed_ms",
        "T13",
        "max",
        &if durable_missing == 0 && marked == ids.len() {
            vec![restarting.elapsed()]
        } else {
            vec![]
        },
    ));
    ctx.push(Metric::value(
        "crash.markers_not_durable",
        "T13",
        "count",
        crate::count_float(durable_missing),
    ));
    ctx.push(Metric::latency(
        "crash.restart_to_marker_ms",
        "T13",
        "max",
        &marker_times
            .iter()
            .map(|d| d.saturating_sub(restarting.duration_since(killed)))
            .collect::<Vec<_>>(),
    ));
    let mut recovered = Vec::new();
    let (mut reopen, mut resumed) = (Vec::new(), 0usize);
    for id in &ids {
        let stream = open_live(&host, id).with_context(|| format!("reopen {id}"))?;
        reopen.push(stream.open_latency);
        recovered.push(stream);
    }
    // Do not include the preceding chat's resume turn in marker latency.
    for (i, stream) in recovered.iter().enumerate() {
        if turn(stream, &format!("BENCH answer [b:crash-after-{i}]")).is_ok() {
            resumed += 1;
        }
    }
    ctx.push(Metric::latency(
        "crash.kill_to_marker_ms",
        "T13",
        "max",
        &marker_times,
    ));
    ctx.push(Metric::value(
        "crash.chats_not_resumed",
        "T13",
        "count",
        crate::count_float(ids.len() - resumed),
    ));
    ctx.push(Metric::latency("crash.reopen_ms", "T13", "max", &reopen));
    ctx.push(Metric::value(
        "crash.chats_not_marked_interrupted",
        "T13",
        "count",
        crate::count_float(ids.len() - marked),
    ));
    ctx.push(Metric::value(
        "crash.chats_usable_after",
        "",
        "count",
        crate::count_float(resumed),
    ));
    ctx.note(format!(
        "S-crash: {} chats, {marked} marked interrupted, {resumed} ran a turn after restart; host had {orphans_before} child processes at the kill",
        ids.len()
    ));
    host.stop();
    if down_secs != 0 {
        for metric in &mut ctx.metrics[metrics_at..] {
            metric.id = metric.id.replacen("crash.", "crash.delayed.", 1);
        }
    }
    Ok(())
}

fn collect_session_text(dir: &std::path::Path, files: &mut Vec<(String, String)>) -> Result<()> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            collect_session_text(&entry.path(), files)?;
        } else if entry.path().extension().is_some_and(|ext| ext == "jsonl") {
            files.push((
                entry.path().to_string_lossy().into_owned(),
                std::fs::read_to_string(entry.path())?,
            ));
        }
    }
    Ok(())
}

/// Least-squares slope normalized by mean RSS, expressed as percent per hour.
fn growth_per_hour(samples: &[(f64, f64)]) -> f64 {
    if samples.len() < 2 {
        return f64::NAN;
    }
    let n = crate::count_float(samples.len());
    let x = samples.iter().map(|(x, _)| x).sum::<f64>() / n;
    let y = samples.iter().map(|(_, y)| y).sum::<f64>() / n;
    let variance = samples.iter().map(|(t, _)| (t - x).powi(2)).sum::<f64>();
    if variance <= 0.0 || y <= 0.0 {
        return f64::NAN;
    }
    samples
        .iter()
        .map(|(t, rss)| (t - x) * (rss - y))
        .sum::<f64>()
        / variance
        * 3600.0
        / y
        * 100.0
}

fn settle(stream: &Stream, deadline: Instant) -> Result<()> {
    loop {
        ensure!(
            Instant::now() < deadline,
            "runtime {} did not settle before T12 tail",
            stream.session_id()
        );
        let state = stream
            .request(
                &json!({"type":"get_state"}),
                deadline.saturating_duration_since(Instant::now()),
            )?
            .0;
        let data = &state["data"];
        ensure!(
            data["isStreaming"].is_boolean() || data["isRunning"].is_boolean(),
            "missing runtime state: {state}"
        );
        if data["isStreaming"] != true && data["isRunning"] != true {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn admit(ctx: &mut Ctx, stream: &Stream, tag: &str, message: &str) -> Result<()> {
    let since = Instant::now();
    let mut causes = Vec::new();
    for attempt in 1..=3 {
        // Check late arrivals before retrying: never duplicate an admitted running turn.
        if ctx.faux.rounds(tag, since) > 0 {
            return Ok(());
        }
        let reply = stream.request(&json!({"type":"prompt", "message":message}), LONG);
        if ctx
            .faux
            .wait_request(tag, since, Duration::from_secs(10))
            .is_some()
        {
            ctx.note(format!(
                "admitted {tag} attempt={attempt}; prior causes={causes:?}"
            ));
            return Ok(());
        }
        let state = stream.request(&json!({"type":"get_state"}), SHORT);
        causes.push(format!(
            "attempt {attempt}: reply={reply:?}, state={state:?}, closed={}",
            stream.closed()
        ));
        // Abort an accepted-but-silent turn and establish settlement before retry.
        stream
            .request(&json!({"type":"abort"}), SHORT)
            .with_context(|| format!("{tag} admission causes: {causes:?}"))?;
        settle(stream, Instant::now() + LONG)
            .with_context(|| format!("{tag} admission causes: {causes:?}"))?;
    }
    anyhow::bail!(
        "{tag} never started (no provider request): {}",
        causes.join("; ")
    )
}

/// Every loop must remain active across the entire window, including both edges.
fn sustained(times: &[f64], duration: f64, max_gap: f64) -> bool {
    !times.is_empty()
        && times
            .iter()
            .copied()
            .chain(std::iter::once(duration))
            .scan(0.0, |last, at| {
                let gap = at - *last;
                *last = at;
                Some(gap)
            })
            .all(|gap| (0.0..=max_gap).contains(&gap))
}

/// T14: drive production retention through an isolated host, using sparse files
/// and a deliberately nonfunctional browser (cache eviction precedes spawn).
/// This tests retention, not browser rendering or download availability.
#[allow(
    clippy::too_many_lines,
    reason = "Linear measurement protocol keeps setup, observation windows, and cleanup ordered; splitting changes benchmark review context."
)]
pub(crate) fn s_disk(ctx: &mut Ctx) -> Result<()> {
    use std::fs::{File, FileTimes};
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};
    let cfg = ctx.config("s-disk");
    host::seed(&cfg)?;
    let agent = cfg.root.join("agent");
    let tmp = agent.join("tmp");
    let trash = agent.join("trash");
    let cache = cfg.root.join("xdg/cache/unipi/cloakbrowser");
    for dir in [&tmp, &trash, &cache] {
        std::fs::create_dir_all(dir)?;
    }
    for (name, aged) in [("old", true), ("recent", false)] {
        let file = File::create(tmp.join(name))?;
        if aged {
            file.set_times(FileTimes::new().set_modified(UNIX_EPOCH))?;
        }
    }
    for (name, stamp) in [
        ("expired", Some(1)),
        (
            "recent",
            Some(SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs()),
        ),
        ("legacy", None),
    ] {
        std::fs::write(trash.join(format!("{name}.jsonl")), "fixture")?;
        if let Some(stamp) = stamp {
            std::fs::write(trash.join(format!("{name}.trashed-at")), stamp.to_string())?;
        }
    }
    // Oversized inherited logs may be discarded or capped at startup. Every
    // retained file must still satisfy the byte cap.
    File::create(cfg.root.join("host/host.log"))?.set_len(50 * 1024 * 1024 + 1)?;
    for i in 1..=3 {
        File::create(cfg.root.join(format!("host/host.log.{i}")))?.set_len(1024)?;
    }
    File::create(cache.join("obsolete-build"))?.set_len(301 * 1024 * 1024)?;
    std::fs::create_dir_all(cfg.root.join("bin"))?;
    let browser = cfg.root.join("bin/fake-chrome");
    std::fs::write(&browser, "#!/bin/sh\nexit 1\n")?;
    std::fs::set_permissions(&browser, std::fs::Permissions::from_mode(0o755))?;
    std::fs::write(
        cfg.root.join("host/config.json"),
        serde_json::to_vec(
            &json!({"browser":{"enabled":true,"engine":"chrome","path":browser,"headless":true}}),
        )?,
    )?;
    let host = Host::start(&cfg)?;
    // Startup pruning is best effort on a blocking thread, so allow up to 5 s.
    let deadline = Instant::now() + Duration::from_secs(5);
    while (tmp.join("old").exists() || trash.join("expired.jsonl").exists())
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(50));
    }
    ctx.push(Metric::value(
        "disk.tmp_failures",
        "T14",
        "count",
        crate::count_float(
            usize::from(tmp.join("old").exists()) + usize::from(!tmp.join("recent").exists()),
        ),
    ));
    ctx.push(Metric::value(
        "disk.trash_failures",
        "T14",
        "count",
        crate::count_float(
            usize::from(trash.join("expired.jsonl").exists())
                + usize::from(!trash.join("recent.jsonl").exists())
                + usize::from(!trash.join("legacy.jsonl").exists()),
        ),
    ));
    // Discarding an inherited oversized log is approved: archive existence is
    // not a retention criterion. Check individual sizes and archive count separately.
    let mut bad_logs = 0;
    let mut archives = 0;
    for entry in std::fs::read_dir(cfg.root.join("host"))?.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if name == "host.log"
            || name
                .strip_prefix("host.log.")
                .is_some_and(|n| n.parse::<u32>().is_ok())
        {
            bad_logs += usize::from(entry.metadata()?.len() > 50 * 1024 * 1024);
            archives += usize::from(name != "host.log");
        }
    }
    ctx.push(Metric::value(
        "disk.log_failures",
        "T14",
        "count",
        crate::count_float(bad_logs + usize::from(archives > 3)),
    ));
    host.browser_start()?;
    let bytes: u64 = std::fs::read_dir(&cache)?
        .flatten()
        .filter_map(|e| e.metadata().ok())
        .map(|m| m.len())
        .sum();
    ctx.push(Metric::value(
        "disk.browser_cache_bytes",
        "T14",
        "bytes",
        crate::counter_float(bytes),
    ));
    ctx.note("T14 browser is a fake executable: production launch must evict obsolete cache before its expected spawn failure; legacy trash retained, timestamped trash uses current 90-day policy".into());
    host.stop();
    Ok(())
}

#[cfg(test)]
mod sustained_tests {
    #[test]
    fn rss_fit_detects_growth_flat_and_shrinking_series() {
        let fit = super::growth_per_hour;
        assert!(fit(&[]).is_nan());
        assert!(fit(&[(1.0, 0.0), (2.0, 0.0)]).is_nan());
        assert_eq!(fit(&[(0.0, 100.0), (1800.0, 100.0)]), 0.0);
        assert!(fit(&[(0.0, 100.0), (1800.0, 110.0)]) > 10.0);
        assert!(fit(&[(0.0, 110.0), (1800.0, 100.0)]) < 0.0);
        assert!(fit(&[(0.0, 100.0), (1800.0, 104.0)]) < 10.0);
    }

    #[test]
    fn loop_must_reach_end_and_cover_middle() {
        assert!(!super::sustained(&[1.0, 2.0, 901.0], 1800.0, 10.0));
        assert!(!super::sustained(&[1.0, 59.0], 60.0, 10.0));
        assert!(!super::sustained(&[], 60.0, 10.0));
        assert!(super::sustained(
            &(1..=60).map(f64::from).collect::<Vec<_>>(),
            60.0,
            10.0
        ));
    }
}
