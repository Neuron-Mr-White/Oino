//! Host scenarios (spec §6): S-open, S-send, S-model, S-list, S-ping. Each runs a fresh
//! `unipi-host serve` in its own root and drives it over app.sock like the app does.
//! L1 / S-idle / S-crash live in [`crate::load`].

use std::path::PathBuf;
use std::time::{Duration, Instant};

use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};

use crate::faux::{Blackhole, Faux};
use crate::fixture::{self, Fixture};
use crate::host::{Host, HostConfig};
use crate::record::Metric;
use crate::wire::{Stream, op};

/// Shared run state.
pub(crate) struct Ctx {
    pub(crate) bin: PathBuf,
    pub(crate) dist: PathBuf,
    pub(crate) work: PathBuf,
    pub(crate) warm_cache: PathBuf,
    pub(crate) faux: Faux,
    pub(crate) blackhole: Blackhole,
    pub(crate) refused: u16,
    /// Run-count multiplier (`--quick` = 0.2).
    pub(crate) scale: f64,
    /// Post-work reclamation delay (quick: 60 s; acceptance: 600 s).
    pub(crate) idle_secs: u64,
    /// L1 duration.
    pub(crate) load_secs: u64,
    pub(crate) real: Option<PathBuf>,
    pub(crate) metrics: Vec<Metric>,
    pub(crate) notes: Vec<String>,
}

/// Long waits (a 40k wake, a turn).
pub(crate) const LONG: Duration = Duration::from_secs(120);
/// Control-op timeout.
pub(crate) const SHORT: Duration = Duration::from_secs(30);

impl Ctx {
    /// Fresh root for `name`.
    pub(crate) fn config(&self, name: &str) -> HostConfig {
        HostConfig {
            bin: self.bin.clone(),
            root: self.work.join(name),
            dist_extensions: self.dist.clone(),
            faux_port: self.faux.port,
            refused_port: self.refused,
            blackhole_port: self.blackhole.port,
            warm_cache: self.warm_cache.clone(),
        }
    }

    /// `n` scaled, at least `min`.
    pub(crate) fn runs(&self, n: usize, min: usize) -> usize {
        scaled_count(n, self.scale, min)
    }

    pub(crate) fn push(&mut self, metric: Metric) {
        log(&format!(
            "  {:<44} n={:<3} p50={:>9.1} p95={:>9.1} max={:>9.1} {}",
            metric.id, metric.n, metric.p50, metric.p95, metric.max, metric.unit
        ));
        self.metrics.push(metric);
    }

    pub(crate) fn note(&mut self, note: String) {
        log(&format!("  note: {note}"));
        self.notes.push(note);
    }
}

#[allow(clippy::print_stderr, reason = "progress log of a bench run")]
pub(crate) fn log(line: &str) {
    eprintln!("[{}] {line}", chrono::Local::now().format("%H:%M:%S"));
}

/// A project dir under the host's home (sessions are grouped per cwd = per project).
pub(crate) fn project(host_cfg: &HostConfig, name: &str) -> Result<PathBuf> {
    let dir = host_cfg.root.join("home/work").join(name);
    std::fs::create_dir_all(&dir)?;
    Ok(dir)
}

/// The fixture set of S-open / S-send: generated 100 / 5k / 40k plus the real chat.
pub(crate) fn fixtures(ctx: &Ctx, cfg: &HostConfig) -> Result<Vec<(String, Fixture)>> {
    let agent = cfg.root.join("agent");
    let mut out = Vec::new();
    for (label, n, seed, proj) in [
        ("100", 100, 1, "proj-a"),
        ("5k", 5_000, 2, "proj-b"),
        ("40k", 40_000, 3, "proj-c"),
    ] {
        out.push((
            label.to_owned(),
            fixture::generate(&agent, &project(cfg, proj)?, n, seed)?,
        ));
    }
    if let Some(real) = &ctx.real {
        out.push((
            "real72mb".to_owned(),
            fixture::copy_real(&agent, &project(cfg, "proj-real")?, real)?,
        ));
    }
    Ok(out)
}

fn successful_wake(name: &str) -> Result<()> {
    ensure!(name == "woken", "runtime wake failed: {name}");
    Ok(())
}

fn open_req(id: &str) -> Value {
    json!({"op": "session_open", "session": id})
}

/// Closes a session's runtime so the next open is record-only again.
pub(crate) fn close_runtime(host: &Host, id: &str) {
    let _ = op(
        &host.socks.app,
        &json!({"op": "session_close", "session": id}),
        SHORT,
    );
}

/// Opens a session and waits until its runtime is ready (`woken` on a cold open).
pub(crate) fn open_live(host: &Host, id: &str) -> Result<Stream> {
    let stream = Stream::open(&host.socks.app, &open_req(id), LONG)?;
    if stream.snapshot["waking"] == true {
        let marker = stream
            .wait_marker(&["woken", "wake_failed"], stream.opened_at, LONG)
            .context("runtime never woke")?;
        successful_wake(&marker.name)?;
    }
    // Real historical fixtures may resume automatically during wake. Wait for
    // that turn to settle before attempting benchmark work, without aborting it.
    let state = stream.request(&json!({"type": "get_state"}), LONG)?.0;
    if state["data"]["isStreaming"] == true || state["data"]["isRunning"] == true {
        stream
            .wait_marker(&["agent_settled"], stream.opened_at, LONG)
            .context("automatic recovery did not settle")?;
    }
    Ok(stream)
}

/// Sends a prompt and waits for the turn to settle.
pub(crate) fn turn(stream: &Stream, message: &str) -> Result<()> {
    let since = Instant::now();
    stream.request(&json!({"type": "prompt", "message": message}), LONG)?;
    if stream
        .wait_marker(&["agent_settled"], since, LONG)
        .is_none()
    {
        log(&format!(
            "unsettled turn lifecycle: {:?}",
            stream.markers_since(since)
        ));
        log(&format!(
            "unsettled turn state: {:?}",
            stream.request(&json!({"type": "get_state"}), SHORT)
        ));
        anyhow::bail!("turn never settled");
    }
    Ok(())
}

// ------------------------------------------------------------------ S-open (T2, T3)

/// S-open: record-only opens of every fixture size (page served → `woken`), then opens
/// of an already-live chat.
pub(crate) fn s_open(ctx: &mut Ctx) -> Result<()> {
    log("S-open");
    let cfg = ctx.config("s-open");
    crate::host::seed(&cfg)?;
    let fixtures = fixtures(ctx, &cfg)?;
    let host = Host::start(&cfg)?;
    for (label, f) in &fixtures {
        let runs = ctx.runs(51, 3);
        let (mut page, mut ready, mut first) = (Vec::new(), Vec::new(), None);
        for i in 0..runs {
            let stream = Stream::open(&host.socks.app, &open_req(&f.session_id), LONG)?;
            page.push(stream.open_latency);
            if stream.snapshot["waking"] == true {
                let m = stream
                    .wait_marker(&["woken", "wake_failed"], stream.opened_at, LONG)
                    .context("runtime never woke")?;
                successful_wake(&m.name)?;
                let took = m.at - stream.opened_at;
                if i == 0 {
                    first = Some(took);
                } else {
                    ready.push(took);
                }
            } else if i == 0 {
                ctx.note(format!("open.{label}: first open was not record-only"));
            }
            drop(stream);
            close_runtime(&host, &f.session_id);
        }
        ctx.push(Metric::latency(
            &format!("open.cold.{label}.page_ms"),
            "T3",
            "p95",
            &page,
        ));
        ctx.push(Metric::latency(
            &format!("open.cold.{label}.ready_ms"),
            "T3",
            "p95",
            &ready,
        ));
        if let Some(first) = first {
            ctx.push(Metric::latency(
                &format!("open.cold.{label}.first_ready_ms"),
                "",
                "max",
                &[first],
            ));
        }
    }
    // T2: the chat is live (another window holds it); each open is a full runtime snapshot.
    for (label, f) in fixtures.iter().filter(|(l, _)| l == "100" || l == "40k") {
        let holder = open_live(&host, &f.session_id)?;
        let mut live = Vec::new();
        for _ in 0..ctx.runs(50, 5) {
            let stream = Stream::open(&host.socks.app, &open_req(&f.session_id), LONG)?;
            live.push(stream.open_latency);
        }
        drop(holder);
        ctx.push(Metric::latency(
            &format!("open.live.{label}.snapshot_ms"),
            "T2",
            "p95",
            &live,
        ));
    }
    host.stop();
    Ok(())
}

// ------------------------------------------------------------------ S-send (T5)

/// S-send: prompt sent on a live chat → the provider request arrives at the faux server.
pub(crate) fn s_send(ctx: &mut Ctx) -> Result<()> {
    log("S-send");
    let cfg = ctx.config("s-send");
    crate::host::seed(&cfg)?;
    let fixtures: Vec<_> = fixtures(ctx, &cfg)?
        .into_iter()
        .filter(|(l, _)| l != "5k")
        .collect();
    let host = Host::start(&cfg)?;
    for (label, f) in &fixtures {
        let stream = open_live(&host, &f.session_id)?;
        // A first turn warms the provider path (connection, model resolution).
        turn(&stream, &format!("BENCH warm [b:send-{label}-warm]"))?;
        let mut lat = Vec::new();
        let mut ack = Vec::new();
        let mut settle = Vec::new();
        for i in 0..ctx.runs(50, 5) {
            let tag = format!("send-{label}-{i}");
            let (id, at) = stream
                .send(&json!({"type": "prompt", "message": format!("BENCH answer [b:{tag}]")}))?;
            let (replied, reply) = stream
                .wait_reply(&id, LONG)
                .context("prompt ack missing on wire")?;
            ensure!(reply["type"] == "ok", "prompt rejected on wire: {reply}");
            ack.push(replied.saturating_duration_since(at));
            match ctx.faux.wait_request(&tag, at, LONG) {
                Some(arrived) => lat.push(arrived.saturating_duration_since(at)),
                None => anyhow::bail!("send.{label}: request {i} never reached the provider"),
            }
            if let Some(m) = stream.wait_marker(&["agent_settled"], at, LONG) {
                settle.push(m.at - at);
            }
        }
        drop(stream);
        ctx.push(Metric::latency(
            &format!("send.{label}.to_request_ms"),
            "T5",
            "p95",
            &lat,
        ));
        ctx.push(Metric::latency(
            &format!("send.{label}.prompt_ack_ms"),
            "T4",
            "p95",
            &ack,
        ));
        ctx.push(Metric::latency(
            &format!("send.{label}.to_settled_ms"),
            "",
            "p95",
            &settle,
        ));
    }
    host.stop();
    Ok(())
}

// ------------------------------------------------------------------ S-model (T6, T7)

/// S-model: the picker's model list, a switch on a live idle chat, and a switch on a
/// record-only chat (counts whether it woke the runtime).
pub(crate) fn s_model(ctx: &mut Ctx) -> Result<()> {
    log("S-model");
    let cfg = ctx.config("s-model");
    crate::host::seed(&cfg)?;
    let agent = cfg.root.join("agent");
    let live_fx = fixture::generate(&agent, &project(&cfg, "proj-a")?, 5_000, 11)?;
    let cold_fx = fixture::generate(&agent, &project(&cfg, "proj-a")?, 5_000, 12)?;
    let host = Host::start(&cfg)?;
    let stream = open_live(&host, &live_fx.session_id)?;
    let (mut list, mut switch) = (Vec::new(), Vec::new());
    for i in 0..ctx.runs(50, 5) {
        list.push(
            stream
                .request(&json!({"type": "get_available_models"}), SHORT)?
                .1,
        );
        let model = if i % 2 == 0 { "faux-2" } else { "faux-1" };
        switch.push(
            stream
                .request(
                    &json!({"type": "set_model", "provider": "bench", "modelId": model}),
                    SHORT,
                )?
                .1,
        );
    }
    drop(stream);
    ctx.push(Metric::latency("model.picker_list_ms", "T6", "p95", &list));
    ctx.push(Metric::latency(
        "model.switch.live_ack_ms",
        "T7",
        "p95",
        &switch,
    ));
    let (mut cold, mut wakes) = (Vec::new(), 0usize);
    let runs = ctx.runs(50, 3);
    for i in 0..runs {
        let stream = Stream::open(&host.socks.app, &open_req(&cold_fx.session_id), LONG)?;
        let model = if i % 2 == 0 { "faux-2" } else { "faux-1" };
        let (_, took) = stream.request(
            &json!({"type": "set_model", "provider": "bench", "modelId": model}),
            LONG,
        )?;
        cold.push(took);
        if stream.saw("woken", stream.opened_at) || stream.snapshot["waking"] != true {
            wakes += 1;
        }
        drop(stream);
        close_runtime(&host, &cold_fx.session_id);
    }
    ctx.push(Metric::latency(
        "model.switch.record_only_ack_ms",
        "T7",
        "p95",
        &cold,
    ));
    ctx.push(Metric::value(
        "model.switch.record_only_wakes",
        "T7",
        "count",
        crate::count_float(wakes),
    ));
    host.stop();
    Ok(())
}

// ------------------------------------------------------------------ S-list (T1)

/// S-list: `sessions_list` (the sidebar's data) with 300 sessions over 3 projects plus
/// the big fixtures, first call after start and steady state.
pub(crate) fn s_list(ctx: &mut Ctx) -> Result<()> {
    log("S-list");
    let cfg = ctx.config("s-list");
    crate::host::seed(&cfg)?;
    let agent = cfg.root.join("agent");
    for p in 0..3u32 {
        let dir = project(&cfg, &format!("proj-{p}"))?;
        for i in 0..100u32 {
            fixture::generate(
                &agent,
                &dir,
                60 + (i as usize % 7) * 20,
                1_000 + p * 1_000 + i,
            )?;
        }
    }
    fixture::generate(&agent, &project(&cfg, "proj-0")?, 40_000, 3)?;
    let host = Host::start(&cfg)?;
    let (_, first) = op(&host.socks.app, &json!({"op": "sessions_list"}), LONG)?;
    ctx.push(Metric::latency("list.first_ms", "T1", "max", &[first]));
    let mut lat = Vec::new();
    let mut cards = 0;
    for _ in 0..ctx.runs(50, 10) {
        let (reply, took) = op(&host.socks.app, &json!({"op": "sessions_list"}), LONG)?;
        cards = reply["data"].as_array().map_or(0, Vec::len);
        lat.push(took);
    }
    ctx.push(Metric::latency("list.warm_ms", "T1", "p95", &lat));
    ctx.push(Metric::value(
        "list.cards",
        "",
        "count",
        crate::count_float(cards),
    ));
    host.stop();
    Ok(())
}

// ------------------------------------------------------------------ S-ping (T8)

/// Pings app.sock and live.sock every `every` until `stop`; returns (app, live, failures).
pub(crate) fn pinger(
    host: &Host,
    every: Duration,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> std::thread::JoinHandle<(Vec<Duration>, Vec<Duration>, usize)> {
    let socks = host.socks.clone();
    std::thread::spawn(move || {
        let (mut app, mut live, mut failed) = (Vec::new(), Vec::new(), 0);
        while !stop.load(std::sync::atomic::Ordering::Relaxed) {
            match socks.ping_app(Duration::from_secs(20)) {
                Ok(d) => app.push(d),
                Err(_) => failed += 1,
            }
            match socks.ping_live(Duration::from_secs(5)) {
                Ok(d) => live.push(d),
                Err(_) => failed += 1,
            }
            std::thread::sleep(every);
        }
        (app, live, failed)
    })
}

/// S-ping: `host_info` over app.sock and live.sock on a quiet host, then while a 40k-entry
/// chat wakes and runs a turn (the 1.0.6 kill trigger).
pub(crate) fn s_ping(ctx: &mut Ctx) -> Result<()> {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    log("S-ping");
    let cfg = ctx.config("s-ping");
    crate::host::seed(&cfg)?;
    let big = match &ctx.real {
        Some(real) => {
            fixture::copy_real(&cfg.root.join("agent"), &project(&cfg, "proj-real")?, real)?
        }
        None => fixture::generate(
            &cfg.root.join("agent"),
            &project(&cfg, "proj-c")?,
            40_000,
            3,
        )?,
    };
    let host = Host::start(&cfg)?;
    let stop = Arc::new(AtomicBool::new(false));
    let quiet = pinger(&host, Duration::from_millis(50), Arc::clone(&stop));
    std::thread::sleep(Duration::from_secs(5));
    stop.store(true, Ordering::Relaxed);
    let (app, live, _) = quiet
        .join()
        .map_err(|_| anyhow::anyhow!("pinger panicked"))?;
    ctx.push(Metric::latency("ping.quiet.app_ms", "", "p99", &app));
    ctx.push(Metric::latency("ping.quiet.live_ms", "", "p99", &live));
    let stop = Arc::new(AtomicBool::new(false));
    let busy = pinger(&host, Duration::from_millis(50), Arc::clone(&stop));
    let busy_started = Instant::now();
    let work = (|| -> Result<()> {
        for i in 0..ctx.runs(3, 1) {
            let stream = open_live(&host, &big.session_id)?;
            turn(&stream, &format!("BENCH:tools n=3 [b:ping-{i}]"))?;
            drop(stream);
            close_runtime(&host, &big.session_id);
        }
        Ok(())
    })();
    std::thread::sleep(Duration::from_secs(5).saturating_sub(busy_started.elapsed()));
    stop.store(true, Ordering::Relaxed);
    let (app, live, failed) = busy
        .join()
        .map_err(|_| anyhow::anyhow!("pinger panicked"))?;
    ctx.push(Metric::latency("ping.big_wake.app_ms", "T8", "p99", &app));
    ctx.push(Metric::latency("ping.big_wake.live_ms", "T8", "p99", &live));
    ctx.push(Metric::value(
        "ping.big_wake.failures",
        "T8",
        "count",
        crate::count_float(failed),
    ));
    host.stop();
    work
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "Rounded nonnegative scale becomes a saturating workload count, bounded below by min."
)]
fn scaled_count(n: usize, scale: f64, min: usize) -> usize {
    (crate::count_float(n).mul_add(scale, 0.0).round() as usize).max(min)
}

#[cfg(test)]
mod wake_tests {
    #[test]
    fn failed_wake_is_not_readiness() {
        assert!(super::successful_wake("woken").is_ok());
        assert!(super::successful_wake("wake_failed").is_err());
    }
}
