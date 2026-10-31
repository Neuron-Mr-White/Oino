//! B-1 subagents benchmark (core.md §4): spawns `n` file-persisted children via
//! `AgentSession::spawn_agent` (seed: one user message, model `faux/faux-1`, extensions
//! `None` — the same spawn path the `subagents` extension uses), waits idle, then prompts
//! every child once in parallel. RSS/CPU are sampled around roots, spawn and turns, and
//! the durability barrier micro-cost is measured alongside. Writes
//! `subagents-<n>-<timestamp>.json` + a markdown table into the results dir.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use chrono::Local;
use futures::future;
use indexmap::IndexMap;
use oino_ai::{AgentMessage, JsonObject, UserContentField, UserMessage};
use oino_core::agents::{AgentSpec, ChildAgent, Persistence, Seed};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_settings::SettingsManager;
use serde_json::{Value, json};

use crate::{barrier, sample};

/// Concurrent-children cap per root session (`HARD_MAX_CONCURRENT`, agents/spec.rs).
/// n > cap fans out over `ceil(n / cap)` root sessions so all n children stay alive.
const CHILDREN_PER_ROOT: usize = 16;
/// Wall-clock guard for the parallel-turn phase.
const TURN_TIMEOUT: Duration = Duration::from_secs(600);
/// Wall-clock guard for the post-spawn idle wait.
const IDLE_TIMEOUT: Duration = Duration::from_secs(60);
/// Barrier appends for the micro-cost measurement (core.md §4).
const BARRIER_APPENDS: usize = 100;
/// Every child answers with this text (one faux turn each).
const CHILD_ANSWER: &str = "ok";

/// Runs `subagents --n <N> [--out <dir>]` and writes the report files.
///
/// # Errors
///
/// Argument, spawn, turn, sampling or report-write failures.
#[allow(
    clippy::print_stdout,
    reason = "the bench's stdout table is the product when run by hand"
)]
pub(crate) async fn run(args: &[String]) -> Result<()> {
    let mut n: Option<usize> = None;
    let mut out: Option<PathBuf> = None;
    let mut workdir: Option<PathBuf> = None;
    let mut flags = args.iter();
    while let Some(flag) = flags.next() {
        match flag.as_str() {
            "--n" => {
                n = Some(
                    flags
                        .next()
                        .context("--n needs a value")?
                        .parse()
                        .context("--n must be a positive integer")?,
                );
            }
            "--out" => out = Some(PathBuf::from(flags.next().context("--out needs a value")?)),
            "--workdir" => {
                workdir = Some(PathBuf::from(
                    flags.next().context("--workdir needs a value")?,
                ));
            }
            other => bail!(
                "unknown flag '{other}' (usage: subagents --n <N> [--out dir] [--workdir dir])"
            ),
        }
    }
    let n = n.ok_or_else(|| anyhow::anyhow!("--n is required (1, 10, 50 for the record)"))?;
    if n == 0 {
        bail!("--n must be at least 1");
    }
    let out = out.unwrap_or_else(|| PathBuf::from("bench/results"));
    // The temp agent dir must sit on real disk: on a tmpfs the fsync barrier costs 0 and
    // session writes never leave RAM. `--workdir` > OINO_BENCH_WORKDIR (set by xtask to
    // the repo's target dir) > the system temp dir.
    let workdir = workdir
        .or_else(|| std::env::var_os("OINO_BENCH_WORKDIR").map(PathBuf::from))
        .unwrap_or_else(std::env::temp_dir);

    let report = bench(n, &workdir).await?;
    let (json_path, md_path) = write_report(&out, n, &report)?;
    println!("wrote {}", json_path.display());
    println!("wrote {}", md_path.display());
    println!("{}", markdown(&report));
    Ok(())
}

/// The B-1 scenario itself, as one report value.
#[allow(clippy::too_many_lines, reason = "one block per measured phase")]
async fn bench(n: usize, workdir: &Path) -> Result<Value> {
    let clock = sample::Clock::detect();
    let started = Local::now();
    let run_started = Instant::now();
    let at_start = sample::Sample::take();

    std::fs::create_dir_all(workdir).with_context(|| format!("work dir {}", workdir.display()))?;
    let temp = tempfile::Builder::new()
        .prefix("oino-bench-")
        .tempdir_in(workdir)
        .with_context(|| format!("temp agent dir under {}", workdir.display()))?;
    let services_started = Instant::now();
    let services = services(temp.path()).await?;
    let services_wall = services_started.elapsed();
    let after_services = sample::Sample::take();

    // One root per CHILDREN_PER_ROOT children: `Tree::live` caps open children per root
    // (agents/spec.rs HARD_MAX_CONCURRENT); the fan-out keeps all n children alive at once.
    let roots = n.div_ceil(CHILDREN_PER_ROOT);
    let roots_started = Instant::now();
    let mut runtimes = Vec::with_capacity(roots);
    let mut roots_sessions = Vec::with_capacity(roots);
    for r in 0..roots {
        let runtime = AgentSessionRuntime::create(
            Arc::clone(&services),
            SessionOptions {
                source: SessionSource::New,
                name: Some(format!("bench-root-{r}")),
                ..SessionOptions::default()
            },
        )
        .await
        .with_context(|| format!("root session {r}"))?;
        roots_sessions.push(runtime.session());
        runtimes.push(runtime);
    }
    let roots_wall = roots_started.elapsed();
    let after_roots = sample::Sample::take();

    let spawn_started = Instant::now();
    let mut children: Vec<(usize, Arc<ChildAgent>)> = Vec::with_capacity(n);
    for i in 0..n {
        let root = &roots_sessions[i / CHILDREN_PER_ROOT];
        let child = root
            .spawn_agent("bench", spec(i))
            .await
            .with_context(|| format!("spawn child {i}"))?;
        children.push((i, child));
    }
    let spawn_wall = spawn_started.elapsed();
    tokio::time::timeout(
        IDLE_TIMEOUT,
        future::join_all(children.iter().map(|(_, child)| child.wait_idle())),
    )
    .await
    .with_context(|| format!("post-spawn idle wait exceeded {IDLE_TIMEOUT:?}"))?;
    let after_spawn = sample::Sample::take();

    let turns_started = Instant::now();
    let cpu_before_turns = sample::Sample::take();
    let turns = children.iter().map(|(i, child)| {
        let text = format!("task {i}");
        async move {
            child.prompt(text, None).await?;
            child.wait_idle().await;
            Ok::<(), anyhow::Error>(())
        }
    });
    tokio::time::timeout(TURN_TIMEOUT, future::try_join_all(turns))
        .await
        .with_context(|| format!("turn phase exceeded {TURN_TIMEOUT:?}"))?
        .context("a child turn failed")?;
    let turns_wall = turns_started.elapsed();
    let cpu_after_turns = sample::Sample::take();
    let after_turns = sample::Sample::take();

    let persisted = children
        .iter()
        .filter(|(_, child)| child.session_file().is_some())
        .count();
    let answered = children
        .iter()
        .filter(|(_, child)| child.last_assistant_text().as_deref() == Some(CHILD_ANSWER))
        .count();

    let barrier_started = Instant::now();
    let barrier_dir = temp.path().join("barrier");
    let cost = barrier::measure(temp.path(), &barrier_dir, BARRIER_APPENDS)
        .context("barrier micro-cost")?;
    let barrier_wall = barrier_started.elapsed();

    Ok(json!({
        "bench": "subagents",
        "wp": "C11",
        "spec": "docs/dev/v2/specs/core.md §4 B-1",
        "n": n,
        "roots": roots,
        "childrenPerRootCap": CHILDREN_PER_ROOT,
        "model": "faux/faux-1",
        "provider": "faux (in-process double, no app set)",
        "startedAt": started.to_rfc3339(),
        "durationMs": as_u64(run_started.elapsed().as_millis()),
        "host": host(clock),
        "git": { "head": std::env::var("OINO_BENCH_GIT_HEAD").ok() },
        "workdir": {
            "path": temp.path().display().to_string(),
            "fs": filesystem_of(temp.path()),
        },
        "phasesMs": {
            "services": as_u64(services_wall.as_millis()),
            "roots": as_u64(roots_wall.as_millis()),
            "spawn": as_u64(spawn_wall.as_millis()),
            "turns": as_u64(turns_wall.as_millis()),
            "barrier": as_u64(barrier_wall.as_millis()),
        },
        "rssKib": {
            "supported": at_start.rss_kib.is_some(),
            "atStart": at_start.rss_kib,
            "afterServices": after_services.rss_kib,
            "afterRoots": after_roots.rss_kib,
            "afterSpawn": after_spawn.rss_kib,
            "afterTurns": after_turns.rss_kib,
            "vmHwm": after_turns.vm_hwm_kib,
            "perChildSpawnKib": per_child(after_spawn.rss_kib, after_roots.rss_kib, n),
            "perChildTurnKib": per_child(after_turns.rss_kib, after_spawn.rss_kib, n),
            "perChildTotalKib": per_child(after_turns.rss_kib, after_roots.rss_kib, n),
        },
        "cpuMs": {
            "supported": cpu_before_turns.cpu_ticks.is_some(),
            "clkTck": clock.ticks_per_sec,
            "duringSpawn": clock.ms_between(after_roots, after_spawn),
            "duringTurns": clock.ms_between(cpu_before_turns, cpu_after_turns),
            "duringSpawnTicks": after_spawn
                .cpu_ticks
                .zip(after_roots.cpu_ticks)
                .map(|(a, b)| a.saturating_sub(b)),
            "duringTurnsTicks": cpu_after_turns
                .cpu_ticks
                .zip(cpu_before_turns.cpu_ticks)
                .map(|(a, b)| a.saturating_sub(b)),
        },
        "children": {
            "spawned": children.len(),
            "persistedFiles": persisted,
            "answered": answered,
        },
        "barrier": cost.to_json(),
    }))
}

/// `AgentSpec` per core.md §4 B-1: file persistence, one seeded user message, faux model,
/// no extensions (the `subagents` extension's spawn path).
fn spec(i: usize) -> AgentSpec {
    AgentSpec {
        label: format!("bench-{i}"),
        model: Some("faux/faux-1".into()),
        seed: Seed::Messages(vec![AgentMessage::User(UserMessage {
            content: UserContentField::Text("task".into()),
            ..UserMessage::default()
        })]),
        persistence: Persistence::File { session_dir: None },
        ..AgentSpec::default()
    }
}

/// `Services` with the in-process faux provider (no app set, core.md §4 B-1).
async fn services(dir: &Path) -> Result<Arc<Services>> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "bench");
    let provider = FauxProvider::default();
    provider.route(|_| Some(FauxResponse::Text(CHILD_ANSWER.into())));
    let instances: Vec<Arc<dyn ExtensionInstance>> =
        vec![Arc::new(FauxComponent(Arc::new(provider)))];
    let settings: JsonObject = serde_json::from_value(json!({
        "extensionSettings": { "subagents": { "maxConcurrent": CHILDREN_PER_ROOT } }
    }))
    .context("bench settings")?;
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: dir.join("agent"),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Rpc,
        has_ui: false,
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(instances)),
        approve: Some(false),
        models: Some(models),
        settings: Some(SettingsManager::in_memory(settings)),
    })
    .await
    .context("Services::create")
}

/// Markdown summary table for one run.
#[must_use]
pub(crate) fn markdown(report: &Value) -> String {
    let rss = &report["rssKib"];
    let kib = |key: &str| {
        rss[key]
            .as_u64()
            .map_or_else(|| "—".into(), |v| v.to_string())
    };
    let per = |key: &str| {
        rss[key]
            .as_f64()
            .map_or_else(|| "—".into(), |v| format!("{v:.1}"))
    };
    let phases = &report["phasesMs"];
    let ms = |key: &str| {
        phases[key]
            .as_u64()
            .map_or_else(|| "—".into(), |v| v.to_string())
    };
    let cpu = &report["cpuMs"];
    let barrier = &report["barrier"];
    let host = &report["host"];
    let head = report["git"]["head"]
        .as_str()
        .map_or_else(|| "unknown".into(), ToString::to_string);
    let workdir_fs = report["workdir"]["fs"]
        .as_str()
        .unwrap_or("unknown filesystem");
    format!(
        "# oino-bench — subagents (n = {n})\n\
         \n\
         - {at} · HEAD `{head}` · {os}/{arch} · kernel {kernel} · {hostname} · {cpus} cpus · workdir fs: {workdir_fs}\n\
         \n\
         | Metric | Value |\n\
         |---|---:|\n\
         | Roots (≤ {cap} children each) | {roots} |\n\
         | Children spawned / file-persisted / answered | {spawned} / {persisted} / {answered} |\n\
         | RSS at start (KiB) | {at_start} |\n\
         | RSS after services (KiB) | {after_services} |\n\
         | RSS after roots (KiB) | {after_roots} |\n\
         | RSS after spawn (KiB) | {after_spawn} |\n\
         | RSS after turns (KiB) | {after_turns} |\n\
         | RSS high-water (KiB) | {vm_hwm} |\n\
         | ΔRSS per child — spawn (KiB) | {per_spawn} |\n\
         | ΔRSS per child — turn (KiB) | {per_turn} |\n\
         | ΔRSS per child — total (KiB) | {per_total} |\n\
         | CPU during spawn (ms) | {cpu_spawn} |\n\
         | CPU during turns (ms) | {cpu_turns} |\n\
         | Barrier over {appends} appends (µs mean / min / max) | {mean} / {min} / {max} |\n\
         | Wall: services / roots / spawn / turns / barrier (ms) | {s_w} / {r_w} / {sp_w} / {t_w} / {b_w} |\n\
         \n\
         Barrier method: {barrier_impl}\n",
        n = report["n"],
        at = report["startedAt"].as_str().unwrap_or_default(),
        os = host["os"].as_str().unwrap_or_default(),
        arch = host["arch"].as_str().unwrap_or_default(),
        kernel = host["kernel"].as_str().unwrap_or("—"),
        hostname = host["hostname"].as_str().unwrap_or("—"),
        cpus = host["cpus"],
        cap = report["childrenPerRootCap"],
        roots = report["roots"],
        spawned = report["children"]["spawned"],
        persisted = report["children"]["persistedFiles"],
        answered = report["children"]["answered"],
        at_start = kib("atStart"),
        after_services = kib("afterServices"),
        after_roots = kib("afterRoots"),
        after_spawn = kib("afterSpawn"),
        after_turns = kib("afterTurns"),
        vm_hwm = kib("vmHwm"),
        per_spawn = per("perChildSpawnKib"),
        per_turn = per("perChildTurnKib"),
        per_total = per("perChildTotalKib"),
        cpu_spawn = cpu["duringSpawn"]
            .as_u64()
            .map_or("—".into(), |v| v.to_string()),
        cpu_turns = cpu["duringTurns"]
            .as_u64()
            .map_or("—".into(), |v| v.to_string()),
        appends = barrier["appends"],
        mean = barrier["meanMicros"],
        min = barrier["minMicros"],
        max = barrier["maxMicros"],
        s_w = ms("services"),
        r_w = ms("roots"),
        sp_w = ms("spawn"),
        t_w = ms("turns"),
        b_w = ms("barrier"),
        barrier_impl = barrier["impl"].as_str().unwrap_or_default(),
        workdir_fs = workdir_fs,
    )
}

/// Writes `<out>/subagents-<n>-<timestamp>.{json,md}` and returns both paths.
///
/// # Errors
///
/// Directory creation, JSON encoding or file-write failures.
fn write_report(out: &Path, n: usize, report: &Value) -> Result<(PathBuf, PathBuf)> {
    std::fs::create_dir_all(out).with_context(|| format!("create {}", out.display()))?;
    let stamp = Local::now().format("%Y%m%d-%H%M%S");
    let base = out.join(format!("subagents-{n}-{stamp}"));
    let json_path = PathBuf::from(format!("{}.json", base.display()));
    let md_path = PathBuf::from(format!("{}.md", base.display()));
    std::fs::write(&json_path, serde_json::to_string_pretty(report)?.as_bytes())
        .with_context(|| format!("write {}", json_path.display()))?;
    std::fs::write(&md_path, markdown(report))
        .with_context(|| format!("write {}", md_path.display()))?;
    Ok((json_path, md_path))
}

/// Host/machine metadata for the report.
fn host(clock: sample::Clock) -> Value {
    json!({
        "os": std::env::consts::OS,
        "arch": std::env::consts::ARCH,
        "kernel": read_trimmed("/proc/sys/kernel/osrelease"),
        "hostname": read_trimmed("/proc/sys/kernel/hostname"),
        "cpus": std::thread::available_parallelism().ok().map(std::num::NonZero::get),
        "clkTck": clock.ticks_per_sec,
    })
}

/// First line of a file, trimmed (`None` when missing — non-Linux /proc entries).
fn read_trimmed(path: &str) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Filesystem type backing `path` (longest mount-point prefix of `/proc/self/mounts`).
fn filesystem_of(path: &Path) -> Option<String> {
    let target = path.canonicalize().ok()?;
    let mounts = std::fs::read_to_string("/proc/self/mounts").ok()?;
    let mut best: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let (Some(device), Some(mount), Some(fs_type)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        // `/proc/self/mounts` octal-escapes spaces in mount points; unescape them.
        let mount = PathBuf::from(mount.replace("\\040", " "));
        if target.starts_with(&mount) {
            let depth = mount.components().count();
            if best.as_ref().is_none_or(|(d, _)| depth > *d) {
                best = Some((depth, format!("{fs_type} ({device})")));
            }
        }
    }
    best.map(|(_, fs_type)| fs_type)
}

/// Per-child KiB delta `(after − before) / n`, 1 decimal, negative allowed.
#[allow(clippy::cast_precision_loss, reason = "1-decimal KiB reporting")]
fn per_child(after: Option<u64>, before: Option<u64>, n: usize) -> Option<f64> {
    let delta = i64::try_from(after?).ok()? - i64::try_from(before?).ok()?;
    Some(delta as f64 / u64::try_from(n).unwrap_or(u64::MAX) as f64)
}

/// `u128` → `u64` (saturating; durations never overflow realistically).
fn as_u64(value: u128) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}
