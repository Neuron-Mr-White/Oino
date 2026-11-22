//! `oino-bench host <scenario…|all> [--host-bin P] [--out DIR] [--label L] [--gate]
//! [--compare REC.json | --pair BASELINE_HOST] [--quick] [--load-secs N] [--no-real]` — the spec §6 v3-perf
//! scenarios against a real `unipi-host`. Writes `<out>/<label>.json` + `.md`.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};

use crate::faux::{self, Blackhole, Faux};
use crate::scenario::{self, Ctx, log};
use crate::{fixture, load, record};

/// Every scenario in run order (`all`).
const ALL: &[&str] = &[
    "list", "open", "send", "model", "ping", "idle", "crash", "disk", "l1",
];

fn run_one(ctx: &mut Ctx, name: &str) -> Result<()> {
    match name {
        "open" => scenario::s_open(ctx),
        "send" => scenario::s_send(ctx),
        "model" => scenario::s_model(ctx),
        "list" => scenario::s_list(ctx),
        "ping" => scenario::s_ping(ctx),
        "l1" => load::l1(ctx),
        "idle" => load::s_idle(ctx),
        "crash" => load::s_crash(ctx),
        "disk" => load::s_disk(ctx),
        other => bail!("unknown scenario '{other}' (known: {})", ALL.join(", ")),
    }
}

fn need(flags: &mut std::slice::Iter<'_, String>, flag: &str) -> Result<String> {
    flags
        .next()
        .cloned()
        .with_context(|| format!("{flag} needs a value"))
}

/// Runs the host scenarios; exits non-zero when `--gate` finds a failure.
///
/// # Errors
///
/// Argument, scenario or report failures.
#[allow(clippy::print_stdout, reason = "the markdown table is the product")]
#[allow(
    clippy::too_many_lines,
    reason = "Linear measurement protocol keeps setup, observation windows, and cleanup ordered; splitting changes benchmark review context."
)]
pub(crate) fn run(args: &[String]) -> Result<()> {
    if args.first().is_some_and(|s| s == "soak") {
        return crate::soak::run(&args[1..]);
    }
    let root = PathBuf::from(std::env::var("OINO_BENCH_ROOT").unwrap_or_else(|_| ".".into()));
    let mut names = Vec::new();
    let mut host_commit = None;
    let mut bin = std::env::var_os("OINO_BENCH_HOST_BIN").map(PathBuf::from);
    let mut out = root.join("bench/records");
    let mut label = None;
    let mut gate = false;
    let mut compare = None;
    let mut pair = None;
    let mut scale = 1.0;
    let mut load_secs = 60;
    let mut runs = 1usize;
    let mut wait_quiet = false;
    let mut real = fixture::real_source();
    let mut flags = args.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--host-commit" => host_commit = Some(need(&mut flags, flag)?),
            "--host-bin" => bin = Some(need(&mut flags, flag)?.into()),
            "--out" => out = need(&mut flags, flag)?.into(),
            "--label" => label = Some(need(&mut flags, flag)?),
            "--gate" => gate = true,
            "--pair" => pair = Some(PathBuf::from(need(&mut flags, flag)?)),
            "--compare" => compare = Some(PathBuf::from(need(&mut flags, flag)?)),
            "--quick" => scale = 0.2,
            "--wait-quiet" => wait_quiet = true,
            "--runs" => {
                runs = need(&mut flags, flag)?.parse().context("--runs")?;
                if runs == 0 {
                    bail!("--runs must be positive");
                }
            }
            "--load-secs" => load_secs = need(&mut flags, flag)?.parse().context("--load-secs")?,
            "--no-real" => real = None,
            "--real" => real = Some(need(&mut flags, flag)?.into()),
            "all" => names.extend(ALL.iter().map(|s| (*s).to_owned())),
            other if other.starts_with("--") => bail!("unknown flag '{other}'"),
            other => names.push(
                other
                    .strip_prefix("s-")
                    .unwrap_or(other)
                    .to_ascii_lowercase(),
            ),
        }
    }
    if names.is_empty() {
        bail!(
            "usage: oino-bench host <{}|all> [--gate] [--host-bin P]",
            ALL.join("|")
        );
    }
    if load_secs == 0 {
        bail!("--load-secs must be positive");
    }
    if gate && scale < 1.0 {
        bail!("--quick cannot certify a gate (T12 requires 600 seconds)");
    }
    if pair.is_some() && compare.is_some() {
        bail!("--pair and --compare are mutually exclusive");
    }
    let quietness_start = wait_for_quiet(wait_quiet)?;
    let bin = bin.context("--host-bin (or OINO_BENCH_HOST_BIN) is required")?;
    let work = work_root(&root);
    std::fs::create_dir_all(&work)?;
    let mut ctx = Ctx {
        bin: bin.clone(),
        dist: std::env::var_os("OINO_DIST_EXTENSIONS")
            .map_or_else(|| "/usr/share/oino/extensions".into(), PathBuf::from),
        warm_cache: work.join("warm-cache"),
        work: work.clone(),
        faux: Faux::start()?,
        blackhole: Blackhole::start()?,
        refused: faux::refused_port(),
        scale,
        load_secs,
        idle_secs: if scale < 1.0 { 60 } else { 600 },
        real,
        metrics: Vec::new(),
        notes: Vec::new(),
    };
    if ctx.real.is_none() {
        ctx.note("the real 72 MB chat is absent: real-chat rows skipped".into());
    }
    let started = Instant::now();
    let mut failed = Vec::new();
    let mut pairs = Vec::new();
    let mut pair_failures = Vec::new();
    for name in &names {
        // Two adjacent AB batches by default; --runs repeats ABAB per scenario.
        for batch in 0..runs * if pair.is_some() { 2 } else { 1 } {
            let mut baseline = Vec::new();
            for is_baseline in if pair.is_some() {
                vec![true, false]
            } else {
                vec![false]
            } {
                ctx.bin = if is_baseline {
                    pair.as_ref()
                        .context("baseline arm requires comparison binary")?
                        .clone()
                } else {
                    bin.clone()
                };
                let arm = if is_baseline { "A" } else { "B" };
                ctx.work = work_root(&root).join(format!("{arm}-{name}-{batch}"));
                // Neither arm inherits the other's compiled cache.
                ctx.warm_cache = work_root(&root).join(format!("warm-{arm}"));
                log(&format!("{name} batch {} arm {arm}", batch + 1));
                let at = ctx.metrics.len();
                if let Err(error) = run_one(&mut ctx, name) {
                    ctx.note(format!("{arm} scenario {name} failed: {error:#}"));
                    failed.push(format!("{arm}:{name}:{batch}"));
                }
                if is_baseline {
                    baseline = ctx.metrics.drain(at..).collect();
                } else if pair.is_some() {
                    let candidate = &ctx.metrics[at..];
                    let previous = record::to_json(&json!({}), &baseline, &[]);
                    let verdicts = record::judge_pair(candidate, &baseline);
                    for verdict in &verdicts {
                        if verdict.status != "ok" {
                            pair_failures
                                .push(format!("{name}/{batch}/{}: {}", verdict.id, verdict.status));
                        }
                    }
                    // Missing A or B rows fail closed, even for diagnostic metrics.
                    for metric in baseline.iter().chain(candidate) {
                        if !baseline.iter().any(|m| m.id == metric.id)
                            || !candidate.iter().any(|m| m.id == metric.id)
                        {
                            pair_failures
                                .push(format!("{name}/{batch}/{}: missing arm", metric.id));
                        }
                    }
                    pairs.push(json!({"scenario":name,"batch":batch,
                        "baseline":previous,"candidate":record::to_json(&json!({}),candidate,&verdicts)}));
                }
            }
        }
    }
    let quietness_end = quietness()?;
    let noisy = is_noisy(&quietness_start) || is_noisy(&quietness_end);
    if noisy {
        ctx.note("WARNING: noisy machine: 1-min load > 0.5 × logical CPUs at start/end".into());
    }
    let mut pooled: Vec<record::Metric> = Vec::new();
    for metric in ctx.metrics.drain(..) {
        if let Some(existing) = pooled.iter_mut().find(|m| m.id == metric.id) {
            existing.merge(&metric);
        } else {
            pooled.push(metric);
        }
    }
    ctx.metrics = pooled;
    let budgets = record::budgets(&root.join("bench/budgets.toml"))?;
    for m in &mut ctx.metrics {
        if let Some((id, _)) = record::limit_for(&budgets, &m.id) {
            id.clone_into(&mut m.budget);
        }
    }
    let previous: Option<Value> = match &compare {
        Some(p) => Some(serde_json::from_slice(
            &std::fs::read(p).with_context(|| format!("read {}", p.display()))?,
        )?),
        None => None,
    };
    let mut verdicts = record::judge(&ctx.metrics, &budgets, previous.as_ref());
    if pair.is_some() {
        // Regression was judged above per adjacent pair, never against a pooled record.
        for verdict in &mut verdicts {
            if verdict.status == "FAIL baseline" {
                verdict.status = "ok";
            }
        }
    }
    let date = chrono::Local::now();
    let meta = json!({
        "date": date.to_rfc3339(),
        "commit": std::env::var("OINO_BENCH_GIT_HEAD").unwrap_or_default(),
        "host_bin": bin.display().to_string(),
        "host_commit": host_commit,
        "machine": machine(),
        "quietness_start": quietness_start,
        "quietness_end": quietness_end,
        "quality": if noisy { "noisy" } else { "quiet" },
        "runs": runs,
        "pair_host_bin": pair,
        "pairs": pairs,
        "pair_failures": pair_failures,
        "failed_scenarios": failed,
        "measurement_scope": "host wire proxies only; no app paint, webview frame time, supervisor, worker isolation, or worker isolation certification",
        "scenarios": names,
        "scale": scale,
        "load_secs": load_secs,
        "idle_secs": ctx.idle_secs,
        "wall_s": started.elapsed().as_secs(),
        "compared_with": compare.map(|p| p.display().to_string()),
        "notes": ctx.notes,
    });
    let label = label.unwrap_or_else(|| format!("run-{}", date.format("%Y%m%d-%H%M%S")));
    std::fs::create_dir_all(&out)?;
    let json_path = out.join(format!("{label}.json"));
    std::fs::write(
        &json_path,
        serde_json::to_vec_pretty(&record::to_json(&meta, &ctx.metrics, &verdicts))?,
    )?;
    let md = record::to_markdown(&meta, &ctx.metrics, &verdicts);
    std::fs::write(out.join(format!("{label}.md")), &md)?;
    println!("{md}");
    log(&format!(
        "wrote {} ({:.0} s)",
        json_path.display(),
        started.elapsed().as_secs_f64()
    ));
    let gate_fails: Vec<_> = verdicts
        .iter()
        .filter(|v| v.status != "ok")
        .map(|v| format!("{} ({})", v.id, v.status))
        .collect();
    if !failed.is_empty() {
        bail!("scenarios failed: {}", failed.join(", "));
    }
    if gate && noisy && pair.is_none() {
        bail!("gate: noisy machine; record is not acceptance evidence");
    }
    if gate && !pair_failures.is_empty() {
        bail!("paired regressions: {}", pair_failures.join(", "));
    }
    if gate && !gate_fails.is_empty() {
        bail!(
            "gate: {} failing metrics: {}",
            gate_fails.len(),
            gate_fails.join(", ")
        );
    }
    Ok(())
}

fn work_root(root: &std::path::Path) -> PathBuf {
    std::env::var_os("OINO_BENCH_WORKDIR")
        .map_or_else(|| root.join("target/bench"), PathBuf::from)
        .join("host")
}

fn machine() -> String {
    let cpu = std::fs::read_to_string("/proc/cpuinfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("model name"))
                .map(|l| l.split(':').nth(1).unwrap_or("").trim().to_owned())
        })
        .unwrap_or_default();
    let mem = std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|s| {
            s.lines()
                .next()
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u64>().ok())
        })
        .map_or(0, |kib| kib / 1024 / 1024);
    format!(
        "{cpu}, {} threads, {mem} GiB",
        std::thread::available_parallelism().map_or(0, usize::from)
    )
}

/// Linux logical CPUs (not the process's cgroup scheduling quota).
fn quietness() -> Result<Value> {
    let load: Vec<f64> = std::fs::read_to_string("/proc/loadavg")?
        .split_whitespace()
        .take(3)
        .map(str::parse)
        .collect::<std::result::Result<_, _>>()?;
    let cores = std::fs::read_to_string("/proc/cpuinfo")?
        .lines()
        .filter(|line| line.starts_with("processor") && line.contains(':'))
        .count();
    if cores == 0 {
        bail!("cannot determine CPU count");
    }
    Ok(json!({"loadavg": load, "cpu_count": cores}))
}

fn is_noisy(sample: &Value) -> bool {
    sample["loadavg"][0].as_f64().unwrap_or(f64::INFINITY)
        > sample["cpu_count"].as_f64().unwrap_or(0.0) * 0.5
}

fn wait_for_quiet(wait: bool) -> Result<Value> {
    let started = Instant::now();
    loop {
        let sample = quietness()?;
        if !is_noisy(&sample) {
            return Ok(sample);
        }
        log(&format!("WARNING: noisy machine: {sample}"));
        if !wait {
            return Ok(sample);
        }
        if started.elapsed() >= Duration::from_secs(600) {
            bail!("machine still noisy after 10 minutes; no benchmark started");
        }
        std::thread::sleep(Duration::from_secs(10));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quiet_threshold_uses_half_the_cpu_count() {
        assert!(!is_noisy(&json!({"loadavg": [16.0], "cpu_count": 32})));
        assert!(is_noisy(&json!({"loadavg": [16.01], "cpu_count": 32})));
    }
}
