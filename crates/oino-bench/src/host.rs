//! A real `unipi-host serve` in an isolated root (HOME, `OINO_AGENT_DIR`, `OINO_HOST_DIR`,
//! XDG — never the owner's dirs), wired to the faux provider and the dead local servers,
//! plus `/proc` sampling of the host and every process it started.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, bail};
use serde_json::json;

use crate::faux;
use crate::wire::Socks;

/// Where the host runs and what it talks to.
pub(crate) struct HostConfig {
    pub(crate) bin: PathBuf,
    pub(crate) root: PathBuf,
    pub(crate) dist_extensions: PathBuf,
    pub(crate) faux_port: u16,
    pub(crate) refused_port: u16,
    pub(crate) blackhole_port: u16,
    /// Compiled-component cache shared between runs (a warm start, as on the owner PC).
    pub(crate) warm_cache: PathBuf,
}

/// A running host.
pub(crate) struct Host {
    child: Option<Child>,
    pub(crate) pid: u32,
    pub(crate) socks: Socks,
    pub(crate) root: PathBuf,
    pub(crate) agent_dir: PathBuf,
    /// spawn → `host_info` answered.
    pub(crate) startup: Duration,
    config_bin: PathBuf,
    env: Vec<(String, PathBuf)>,
    dist: PathBuf,
    warm_cache: PathBuf,
}

/// Seeds `root` (models.json, settings) — fixtures go into `agent/sessions` before start.
///
/// # Errors
///
/// I/O failures.
pub(crate) fn seed(config: &HostConfig) -> Result<()> {
    let _ = std::fs::remove_dir_all(&config.root);
    for dir in [
        "home/work",
        "agent",
        "host",
        "xdg/data",
        "xdg/config",
        "xdg/cache",
        "xdg/state",
    ] {
        std::fs::create_dir_all(config.root.join(dir))?;
    }
    let model = |id: &str| {
        json!({"id": id, "name": id, "reasoning": false, "input": ["text"], "contextWindow": 2_000_000,
               "maxTokens": 8192, "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}})
    };
    let models = json!({"providers": {
        "bench": {"baseUrl": format!("http://127.0.0.1:{}/v1", config.faux_port), "api": "openai-completions",
                  "apiKey": "sk-bench", "compat": {"supportsDeveloperRole": false, "supportsReasoningEffort": false,
                  "supportsUsageInStreaming": true, "maxTokensField": "max_tokens"},
                  "models": [model(faux::MODEL), model("faux-2")]},
        // The dead local servers (spec §2: 4,404 failed `list-models` in 11 days): one
        // refuses connections, one accepts and never answers.
        "ollama": {"baseUrl": format!("http://127.0.0.1:{}/v1", config.refused_port)},
        "lmstudio": {"baseUrl": format!("http://127.0.0.1:{}/v1", config.blackhole_port)}
    }});
    let settings = json!({
        "defaultProvider": "bench", "defaultModel": faux::MODEL, "defaultThinkingLevel": "off",
        "enabledModels": ["bench/faux-1", "bench/faux-2"],
        "quietStartup": true, "defaultProjectTrust": "trusted",
        "extensionSettings": {"permission": {"defaultMode": "full"}, "dream": {"enabled": false}},
        "autonomy": {"defaultLevel": "high"}
    });
    let agent = config.root.join("agent");
    std::fs::write(
        agent.join("models.json"),
        serde_json::to_vec_pretty(&models)?,
    )?;
    std::fs::write(
        agent.join("settings.json"),
        serde_json::to_vec_pretty(&settings)?,
    )?;
    if config.warm_cache.is_dir() {
        copy_dir(&config.warm_cache, &agent.join("cache"))?;
    }
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

impl Host {
    /// Starts the host on a seeded root and waits for `host_info`.
    ///
    /// # Errors
    ///
    /// Spawn failure or no answer within 60 s.
    pub(crate) fn start(config: &HostConfig) -> Result<Self> {
        let root = config.root.clone();
        let env = vec![
            ("HOME".to_owned(), root.join("home")),
            (
                "UNIPI_CLOAKBROWSER_CACHE".to_owned(),
                root.join("xdg/cache/unipi/cloakbrowser"),
            ),
            (
                "CLOAKBROWSER_CACHE_DIR".to_owned(),
                root.join("home/.cloakbrowser"),
            ),
            ("OINO_AGENT_DIR".to_owned(), root.join("agent")),
            ("OINO_HOST_DIR".to_owned(), root.join("host")),
            ("XDG_DATA_HOME".to_owned(), root.join("xdg/data")),
            ("XDG_CONFIG_HOME".to_owned(), root.join("xdg/config")),
            ("XDG_CACHE_HOME".to_owned(), root.join("xdg/cache")),
            ("XDG_STATE_HOME".to_owned(), root.join("xdg/state")),
        ];
        let mut host = Self {
            child: None,
            pid: 0,
            socks: Socks {
                app: root.join("host/app.sock"),
                live: root.join("host/live.sock"),
            },
            agent_dir: root.join("agent"),
            root,
            startup: Duration::ZERO,
            config_bin: config.bin.clone(),
            env,
            dist: config.dist_extensions.clone(),
            warm_cache: config.warm_cache.clone(),
        };
        host.spawn()?;
        Ok(host)
    }

    /// Isolated CLI operation against this host, never the owner's profile.
    pub(crate) fn browser_start(&self) -> Result<()> {
        let mut cmd = Command::new(&self.config_bin);
        cmd.args(["browser", "start"])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn()?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(30) {
            if child.try_wait()?.is_some() {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let _ = child.kill();
        let _ = child.wait();
        anyhow::bail!("browser start CLI timed out")
    }

    fn spawn(&mut self) -> Result<()> {
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(self.root.join("host.out"))?;
        let mut cmd = Command::new(&self.config_bin);
        cmd.args(["serve", "--mode", "manual"])
            // Decision-model hooks otherwise inherit real credentials and call the
            // network on each prompt (the chat provider alone is not the whole turn).
            .env_remove("OPENROUTER_API_KEY")
            .env_remove("TYPESAFE_API_KEY")
            .current_dir(self.root.join("home/work"))
            .env("OINO_DIST_EXTENSIONS", &self.dist)
            .env(
                "RUST_LOG",
                std::env::var("BENCH_RUST_LOG").unwrap_or_else(|_| "info".into()),
            )
            .stdin(Stdio::null())
            .stdout(log.try_clone()?)
            .stderr(log);
        for (k, v) in &self.env {
            cmd.env(k, v);
        }
        let started = Instant::now();
        let child = cmd
            .spawn()
            .with_context(|| format!("spawn {}", self.config_bin.display()))?;
        self.pid = child.id();
        self.child = Some(child);
        while started.elapsed() < Duration::from_secs(60) {
            if self.socks.ping_app(Duration::from_secs(2)).is_ok() {
                self.startup = started.elapsed();
                return Ok(());
            }
            if let Some(child) = self.child.as_mut()
                && let Ok(Some(status)) = child.try_wait()
            {
                bail!(
                    "host exited during start: {status} (see {}/host.out)",
                    self.root.display()
                );
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        bail!(
            "host never answered host_info (see {}/host.out)",
            self.root.display()
        )
    }

    /// `kill -9` the host (S-crash); its children are left to their own fate.
    pub(crate) fn kill9(&mut self) {
        if let Some(mut child) = self.child.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }

    /// Starts the host again on the same root (the supervisor's restart).
    ///
    /// # Errors
    ///
    /// As [`Host::start`].
    pub(crate) fn restart(&mut self) -> Result<()> {
        self.kill9();
        self.spawn()
    }

    /// Stops the host (SIGTERM, then SIGKILL) and keeps its compiled-component cache.
    pub(crate) fn stop(mut self) {
        if let Some(mut child) = self.child.take() {
            let pid = nix::unistd::Pid::from_raw(i32::try_from(child.id()).unwrap_or(0));
            let descendants = descendants(child.id());
            let _ = nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM);
            let deadline = Instant::now() + Duration::from_secs(10);
            while Instant::now() < deadline {
                if matches!(child.try_wait(), Ok(Some(_))) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            let _ = child.kill();
            let _ = child.wait();
            for pid in descendants {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(i32::try_from(pid).unwrap_or(0)),
                    nix::sys::signal::Signal::SIGKILL,
                );
            }
        }
        let cache = self.agent_dir.join("cache");
        if !self.warm_cache.is_dir() && cache.is_dir() {
            let _ = copy_dir(&cache, &self.warm_cache);
        }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.kill9();
    }
}

// ------------------------------------------------------------------ /proc sampling

fn clk_tck() -> u64 {
    nix::unistd::sysconf(nix::unistd::SysconfVar::CLK_TCK)
        .ok()
        .flatten()
        .and_then(|v| u64::try_from(v).ok())
        .filter(|v| *v > 0)
        .unwrap_or(100)
}

/// (ppid, utime+stime ticks) of a pid.
fn stat(pid: u32) -> Option<(u32, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let (_, rest) = stat.rsplit_once(')')?;
    let f: Vec<&str> = rest.split_whitespace().collect();
    Some((
        f.get(1)?.parse().ok()?,
        f.get(11)?.parse::<u64>().ok()? + f.get(12)?.parse::<u64>().ok()?,
    ))
}

fn rss_kib(pid: u32) -> u64 {
    std::fs::read_to_string(format!("/proc/{pid}/status"))
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("VmRSS:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse().ok())
        })
        .unwrap_or(0)
}

/// Every live descendant of `root`.
pub(crate) fn descendants(root: u32) -> Vec<u32> {
    let mut parents = Vec::new();
    if let Ok(dir) = std::fs::read_dir("/proc") {
        for entry in dir.flatten() {
            if let Some(pid) = entry
                .file_name()
                .to_str()
                .and_then(|s| s.parse::<u32>().ok())
                && let Some((ppid, _)) = stat(pid)
            {
                parents.push((pid, ppid));
            }
        }
    }
    let mut out = Vec::new();
    let mut frontier = vec![root];
    while let Some(p) = frontier.pop() {
        for (pid, ppid) in &parents {
            if *ppid == p && !out.contains(pid) {
                out.push(*pid);
                frontier.push(*pid);
            }
        }
    }
    out
}

/// One snapshot: host RSS, host + descendants RSS, CPU ticks of both.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Snap {
    pub(crate) host_rss_kib: u64,
    pub(crate) total_rss_kib: u64,
    pub(crate) host_ticks: u64,
    pub(crate) at: Option<Instant>,
}

/// Samples `pid` and its descendants now.
pub(crate) fn snap(pid: u32) -> Snap {
    let mut s = Snap {
        host_rss_kib: rss_kib(pid),
        host_ticks: stat(pid).map_or(0, |(_, t)| t),
        at: Some(Instant::now()),
        ..Snap::default()
    };
    s.total_rss_kib = s.host_rss_kib + descendants(pid).into_iter().map(rss_kib).sum::<u64>();
    s
}

/// Host CPU % (of one core) between two snaps.
pub(crate) fn cpu_pct(a: Snap, b: Snap) -> f64 {
    let (Some(ta), Some(tb)) = (a.at, b.at) else {
        return 0.0;
    };
    let secs = (tb - ta).as_secs_f64();
    if secs <= 0.0 {
        return 0.0;
    }
    let ticks = crate::counter_float(b.host_ticks.saturating_sub(a.host_ticks));
    ticks / crate::counter_float(clk_tck()) / secs * 100.0
}

/// Background peak sampler.
pub(crate) struct Sampler {
    stop: Arc<AtomicBool>,
    peak: Arc<Mutex<Snap>>,
    first: Snap,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Sampler {
    /// Samples every 500 ms until [`Sampler::finish`].
    pub(crate) fn start(pid: u32) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let peak = Arc::new(Mutex::new(Snap::default()));
        let (s, p) = (Arc::clone(&stop), Arc::clone(&peak));
        let thread = std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                let now = snap(pid);
                if let Ok(mut peak) = p.lock() {
                    peak.host_rss_kib = peak.host_rss_kib.max(now.host_rss_kib);
                    peak.total_rss_kib = peak.total_rss_kib.max(now.total_rss_kib);
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
        Self {
            stop,
            peak,
            first: snap(pid),
            thread: Some(thread),
        }
    }

    /// (peak snap, host CPU % over the sampled window).
    pub(crate) fn finish(mut self, pid: u32) -> (Snap, f64) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
        let last = snap(pid);
        let peak = self.peak.lock().map(|p| *p).unwrap_or_default();
        (peak, cpu_pct(self.first, last))
    }
}
