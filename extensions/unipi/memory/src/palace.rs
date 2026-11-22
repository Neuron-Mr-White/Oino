//! `MemPalace` install, daemon and CLI plumbing (unipi `mempalace.ts`, `daemon.ts`). Never fails
//! into the session: every path degrades to markdown-only.

use std::path::{Path, PathBuf};
use std::time::Duration;

use oino_extension::wit::process as wp;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::reader::Reader;

/// Oldest `MemPalace` driven (`mine` files payload).
pub const MIN_MEMPALACE: &str = "3.10.0";
/// `PyPI` check interval.
pub const UPDATE_CHECK_TTL_MS: i64 = 24 * 60 * 60 * 1000;
const PYPI_URL: &str = "https://pypi.org/pypi/mempalace/json";
/// `mempalace` on `PATH`.
pub const CLI: &str = "mempalace";
/// `mempalace-mcp` on `PATH`.
pub const MCP: &str = "mempalace-mcp";

/// unipi `compareVersions` (numeric dotted parts).
#[must_use]
pub fn compare_versions(a: &str, b: &str) -> std::cmp::Ordering {
    let pa: Vec<&str> = a.trim().split('.').collect();
    let pb: Vec<&str> = b.trim().split('.').collect();
    let num = |s: Option<&&str>| -> i64 {
        s.map(|x| {
            x.chars()
                .take_while(char::is_ascii_digit)
                .collect::<String>()
        })
        .and_then(|x| x.parse().ok())
        .unwrap_or(0)
    };
    for i in 0..pa.len().max(pb.len()) {
        let (x, y) = (num(pa.get(i)), num(pb.get(i)));
        if x != y {
            return x.cmp(&y);
        }
    }
    std::cmp::Ordering::Equal
}

/// Extracts `3.10.2` from `uv tool list` / `--version` output (`mempalace v3.10.2`,
/// `MemPalace 3.11.0 (rs …)`).
#[must_use]
pub fn parse_version(out: &str) -> Option<String> {
    for line in out.lines() {
        let l = line.trim();
        let lower = l.to_ascii_lowercase();
        if !(lower.starts_with(CLI)
            || l.chars()
                .next()
                .is_some_and(|c| c.is_ascii_digit() || c == 'v'))
        {
            continue;
        }
        for tok in l.split_whitespace() {
            let t = tok.trim_start_matches('v');
            if t.chars().next().is_some_and(|c| c.is_ascii_digit()) && t.contains('.') {
                return Some(t.trim_end_matches([',', ')']).to_owned());
            }
        }
    }
    None
}

/// `MemPalace 3.11.0 (rs 0.6.0-parity.1)` — the Rust port reports itself this way.
#[must_use]
pub fn is_rust_version(text: &str) -> bool {
    text.contains("(rs ")
}

/// Daemon palace key: `sha256(palace)[:24]`.
#[must_use]
pub fn palace_key(palace: &str) -> String {
    Sha256::digest(palace.as_bytes())
        .iter()
        .take(12)
        .fold(String::new(), |mut s, b| {
            use std::fmt::Write as _;
            let _ = write!(s, "{b:02x}");
            s
        })
}

/// Host paths the backend needs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Places {
    /// Memory root (host).
    pub memory_root: Option<String>,
    /// Palace (host), when `~/.mempalace` is granted.
    pub palace: Option<String>,
    /// Guest path of `~/.mempalace` (daemon state).
    pub mempalace_guest: Option<PathBuf>,
}

/// An install.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Install {
    /// Version.
    pub version: String,
    /// The Rust port (`mempalace-rs`); `uv` does not manage it.
    #[serde(default)]
    pub rust: bool,
}

/// Which `mempalace` / `mempalace-mcp` binaries the extension runs.
///
/// The `mempalaceBin` setting may point at the `mempalace` file itself, at the folder holding
/// it (the Rust port installs `mempalace` and `mempalace-mcp` side by side), or be empty
/// (`PATH`). The first candidate that answers `--version` is used and `mempalace-mcp` comes
/// from the same folder; a stale path degrades to `PATH` (Python), never to markdown-only.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Bins {
    /// `mempalace` executable (an absolute path, or the bare name for `PATH`).
    pub cli: String,
    /// `mempalace-mcp` candidates in order.
    pub mcp: Vec<String>,
    /// Version the CLI reported (`None` = it did not run).
    pub version: Option<String>,
    /// The CLI is the Rust port.
    pub rust: bool,
}

impl Bins {
    /// `PATH` only (no `mempalaceBin`): the Python install or a switched entry point.
    #[must_use]
    pub fn path() -> Self {
        Self {
            cli: CLI.into(),
            mcp: vec![MCP.into()],
            version: None,
            rust: false,
        }
    }

    /// The CLI is runnable.
    #[must_use]
    pub fn runs(&self) -> bool {
        self.version.is_some()
    }

    /// `(cli, mcp)` candidates for a `mempalaceBin` value: the path, `<path>/mempalace`, `PATH`.
    #[must_use]
    pub fn candidates(configured: &str) -> (Vec<String>, Vec<String>) {
        let raw = expand_home(configured.trim());
        let raw = raw.trim_end_matches('/');
        if raw.is_empty() {
            return (vec![CLI.into()], vec![MCP.into()]);
        }
        let name = Path::new(raw)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        let cli_named = name == CLI || name == "mempalace.exe";
        let mcp = if cli_named {
            Path::new(raw)
                .parent()
                .map_or_else(|| MCP.into(), |d| d.join(MCP).display().to_string())
        } else {
            format!("{raw}/{MCP}")
        };
        (
            vec![raw.to_owned(), format!("{raw}/{CLI}"), CLI.into()],
            vec![mcp, MCP.into()],
        )
    }

    /// Probes the candidates for `configured`; falls back to `PATH`.
    pub async fn resolve(configured: &str) -> Self {
        let (cli, mcp) = Self::candidates(configured);
        for c in cli {
            if let Some((version, rust)) = probe_version(&c).await {
                return Self {
                    cli: c,
                    mcp,
                    version: Some(version),
                    rust,
                };
            }
        }
        Self::path()
    }
}

/// Expands `~/…` through the granted `~/.mempalace` preopen (the guest has no `$HOME`).
fn expand_home(path: &str) -> String {
    let Some(rest) = path.strip_prefix("~/") else {
        return path.to_owned();
    };
    crate::preopen_host(crate::MEMPALACE_GUEST).map_or_else(
        || path.to_owned(),
        |palace| {
            Path::new(&palace).parent().map_or_else(
                || path.to_owned(),
                |home| home.join(rest).display().to_string(),
            )
        },
    )
}

async fn exec(cmd: &str, args: &[&str], timeout_ms: u64) -> Option<wp::ExecResult> {
    wp::exec(
        cmd.to_owned(),
        args.iter().map(ToString::to_string).collect(),
        wp::ExecOptions {
            cwd: None,
            timeout_ms: Some(timeout_ms),
            env: vec![("MALLOC_ARENA_MAX".into(), "2".into())],
        },
    )
    .await
    .ok()
}

/// Runs and reports success.
pub async fn run_ok(cmd: &str, args: &[&str], timeout_ms: u64) -> bool {
    exec(cmd, args, timeout_ms)
        .await
        .is_some_and(|r| r.code == Some(0) && !r.killed)
}

/// `--version` of one executable → `(version, is the Rust port)`.
async fn probe_version(cli: &str) -> Option<(String, bool)> {
    let r = exec(cli, &["--version"], 5_000).await?;
    let text = format!("{}\n{}", r.stdout, r.stderr);
    if r.code != Some(0) && text.trim().is_empty() {
        return None;
    }
    Some((
        parse_version(&text).unwrap_or_else(|| "unknown".into()),
        is_rust_version(&text),
    ))
}

/// unipi `ensureMempalace` over the resolved binaries; `None` = not installed.
#[must_use]
pub fn ensure_install(cache: &Path, bins: &Bins) -> Option<Install> {
    let install = Install {
        version: bins.version.clone()?,
        rust: bins.rust,
    };
    write_install(cache, &install);
    Some(install)
}

/// `uv tool install mempalace` (the Python fallback when no CLI runs at all).
pub async fn uv_install() -> bool {
    run_ok("uv", &["--version"], 5_000).await
        && run_ok("uv", &["tool", "install", "mempalace"], 180_000).await
}

/// Persists the install record.
pub fn write_install(cache: &Path, i: &Install) {
    if let Some(d) = cache.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let _ = std::fs::write(cache, serde_json::to_vec_pretty(i).unwrap_or_default());
}

/// Re-detects the version (after an upgrade).
pub async fn refresh_version(bins: &Bins, cache: &Path) -> Option<Install> {
    let (version, rust) = probe_version(&bins.cli).await?;
    let install = Install { version, rust };
    write_install(cache, &install);
    Some(install)
}

fn palace_args<'a>(p: &'a Places, rest: &[&'a str]) -> Vec<&'a str> {
    let mut v = Vec::new();
    if let Some(palace) = p.palace.as_deref() {
        v.push("--palace");
        v.push(palace);
    }
    v.extend_from_slice(rest);
    v
}

/// Daemon endpoint `(host, port, token)` from its state dir.
#[must_use]
pub fn daemon_endpoint(p: &Places) -> Option<(String, u16, String)> {
    let dir = p
        .mempalace_guest
        .as_ref()?
        .join("daemon")
        .join(palace_key(p.palace.as_deref()?));
    let ep: Value = serde_json::from_slice(&std::fs::read(dir.join("endpoint.json")).ok()?).ok()?;
    let token = std::fs::read_to_string(dir.join("token"))
        .ok()?
        .trim()
        .to_owned();
    let host = ep.get("host")?.as_str()?.to_owned();
    let port = u16::try_from(ep.get("port")?.as_u64()?).ok()?;
    (!token.is_empty()).then_some((host, port, token))
}

async fn daemon_request(
    p: &Places,
    is_post: bool,
    route: &str,
    body: Option<&Value>,
    timeout_ms: u32,
) -> Result<Value, String> {
    let (addr, port, bearer) = daemon_endpoint(p).ok_or("no daemon endpoint")?;
    let url = format!("http://{addr}:{port}{route}");
    let mut req = if is_post {
        oino_extension::http::post(&url)
    } else {
        oino_extension::http::get(&url)
    };
    req = req
        .header("authorization", &format!("Bearer {bearer}"))
        .header("content-type", "application/json")
        .timeout_ms(timeout_ms);
    if let Some(b) = body {
        req = req.body(b.to_string());
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    let status = resp.status();
    let text = resp.text().await;
    let parsed: Value = serde_json::from_str(&text).map_err(|_| {
        format!(
            "non-JSON response ({status}): {}",
            text.chars().take(200).collect::<String>()
        )
    })?;
    if !(200..300).contains(&status) {
        return Err(parsed
            .get("error")
            .and_then(Value::as_str)
            .map_or_else(|| format!("HTTP {status}"), ToOwned::to_owned));
    }
    Ok(parsed)
}

/// `{reachable, busy}` from `/health`.
pub async fn probe_daemon(p: &Places, timeout_ms: u32) -> (bool, bool) {
    match daemon_request(p, false, "/health", None, timeout_ms).await {
        Ok(h) if h.get("ok") == Some(&Value::Bool(true)) => {
            (true, h.get("active_job_id").is_some_and(|j| !j.is_null()))
        }
        _ => (false, false),
    }
}

/// Starts the daemon when allowed; true when healthy.
pub async fn ensure_daemon(p: &Places, bins: &Bins, auto_start: bool) -> bool {
    if probe_daemon(p, 300).await.0 {
        return true;
    }
    if !auto_start {
        return false;
    }
    let _ = run_ok(&bins.cli, &palace_args(p, &["daemon", "start"]), 15_000).await;
    let deadline = oino_extension::timers::now_ms() + 10_000;
    while oino_extension::timers::now_ms() < deadline {
        if probe_daemon(p, 500).await.0 {
            return true;
        }
        oino_extension::timers::sleep(Duration::from_millis(400)).await;
    }
    false
}

/// Write outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// In the palace.
    Filed,
    /// Accepted, still running.
    Queued,
    /// Only the md file.
    MarkdownOnly,
}

impl Outcome {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Filed => "filed",
            Self::Queued => "queued",
            Self::MarkdownOnly => "markdown-only",
        }
    }
}

async fn job(p: &Places, kind: &str, payload: Value, wait_ms: i64) -> (Outcome, Option<String>) {
    let submitted = match daemon_request(
        p,
        true,
        "/jobs",
        Some(&json!({"kind": kind, "payload": payload})),
        10_000,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return (Outcome::MarkdownOnly, Some(e)),
    };
    let Some(id) = submitted
        .pointer("/job/id")
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
    else {
        return (Outcome::MarkdownOnly, Some("submit returned no job".into()));
    };
    let deadline = oino_extension::timers::now_ms() + wait_ms;
    while oino_extension::timers::now_ms() < deadline {
        let j = match daemon_request(p, false, &format!("/jobs/{id}"), None, 5_000).await {
            Ok(v) => v,
            Err(e) => return (Outcome::Queued, Some(e)),
        };
        let state = j
            .pointer("/job/state")
            .and_then(Value::as_str)
            .unwrap_or("vanished");
        if !matches!(state, "queued" | "running" | "deferred") {
            let ok = state == "succeeded"
                && j.pointer("/job/result/success")
                    .is_none_or(|s| s == &Value::Bool(true));
            if ok {
                return (Outcome::Filed, None);
            }
            let err = j
                .pointer("/job/result/error")
                .and_then(Value::as_str)
                .unwrap_or(state);
            return (
                Outcome::MarkdownOnly,
                Some(format!("{kind} job {state}: {err}")),
            );
        }
        oino_extension::timers::sleep(Duration::from_millis(250)).await;
    }
    (Outcome::Queued, Some("timed out".into()))
}

/// Files md files through a daemon mine job.
pub async fn file_through_daemon(
    p: &Places,
    bins: &Bins,
    auto: bool,
    source_dir: &str,
    files: &[String],
    wing: &str,
    wait_ms: i64,
) -> (Outcome, Option<String>) {
    if !ensure_daemon(p, bins, auto).await {
        return (Outcome::MarkdownOnly, Some("daemon unreachable".into()));
    }
    let mut payload = json!({"source": source_dir, "files": files, "wing": wing, "agent": "unipi"});
    if let Some(pp) = &p.palace {
        payload["palace_path"] = json!(pp);
    }
    job(p, "mine", payload, wait_ms).await
}

/// Deletes a source's drawers through a daemon job.
pub async fn delete_through_daemon(
    p: &Places,
    bins: &Bins,
    auto: bool,
    source: &str,
    wait_ms: i64,
) -> (Outcome, Option<String>) {
    if !ensure_daemon(p, bins, auto).await {
        return (Outcome::MarkdownOnly, Some("daemon unreachable".into()));
    }
    job(p, "mcp_tool", json!({"name": "mempalace_delete_by_source", "arguments": {"source_file": source, "dry_run": false}}), wait_ms).await
}

/// "is held by …" tail of a lock error.
#[must_use]
pub fn lock_holder(text: &str) -> Option<String> {
    let i = text.find("is held by ")?;
    Some(
        text[i + 11..]
            .lines()
            .next()
            .unwrap_or_default()
            .trim()
            .to_owned(),
    )
}

/// `mempalace mine <dir> --wing <w> --agent unipi` → `Ok` or `(heldBy, error)`.
///
/// # Errors
///
/// The lock holder (when the palace was busy) and the last output line.
pub async fn mine_direct(
    p: &Places,
    bins: &Bins,
    source_dir: &str,
    wing: &str,
) -> Result<(), (Option<String>, Option<String>)> {
    let r = exec(
        &bins.cli,
        &palace_args(p, &["mine", source_dir, "--wing", wing, "--agent", "unipi"]),
        120_000,
    )
    .await;
    match r {
        Some(r) if r.code == Some(0) => Ok(()),
        Some(r) => {
            let text = format!("{}\n{}", r.stdout, r.stderr);
            Err((
                lock_holder(&text),
                text.trim().lines().last().map(ToOwned::to_owned),
            ))
        }
        None => Err((None, Some("mempalace not runnable".into()))),
    }
}

/// One-shot write-mode MCP server → `mempalace_delete_by_source`.
///
/// # Errors
///
/// The server's error text.
pub async fn delete_via_write_mcp(p: &Places, bins: &Bins, source: &str) -> Result<(), String> {
    let r = Reader::new(p.palace.as_deref(), false, 0, bins.mcp.clone());
    let res = r
        .call_tool(
            "mempalace_delete_by_source",
            json!({"source_file": source, "dry_run": false}),
        )
        .await;
    r.kill().await;
    if let Some(e) = res.get("error") {
        return Err(e.as_str().map_or_else(|| e.to_string(), ToOwned::to_owned));
    }
    if res.get("success") == Some(&Value::Bool(false)) {
        return Err("delete failed".into());
    }
    Ok(())
}

/// `mempalace wake-up --wing <w>` (falls back to the unscoped wake-up).
pub async fn wake_up(p: &Places, bins: &Bins, wing: &str) -> Option<String> {
    for args in [
        palace_args(p, &["wake-up", "--wing", wing]),
        palace_args(p, &["wake-up"]),
    ] {
        if let Some(r) = exec(&bins.cli, &args, 15_000).await
            && r.code == Some(0)
            && !r.stdout.trim().is_empty()
        {
            return Some(r.stdout.trim().chars().take(64 * 1024).collect());
        }
    }
    None
}

/// Update state file.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateState {
    /// Last check.
    pub checked_at: i64,
    /// Latest seen.
    pub latest_version: String,
}

/// Due for a `PyPI` check.
#[must_use]
pub fn update_due(state: Option<&UpdateState>, now: i64) -> bool {
    state.is_none_or(|s| now - s.checked_at >= UPDATE_CHECK_TTL_MS)
}

/// Result of an auto-update attempt: `(updated, current, latest, reason)`.
pub async fn maybe_auto_update(
    p: &Places,
    bins: &Bins,
    state_file: &Path,
    install: &Install,
    force: bool,
) -> (bool, String, Option<String>, &'static str) {
    if install.rust || bins.cli != CLI {
        // `uv tool upgrade` would install/upgrade the Python venv, not the binary in use.
        return (false, install.version.clone(), None, "not-python");
    }
    let now = oino_extension::timers::now_ms();
    let st: Option<UpdateState> = std::fs::read(state_file)
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    if !force && !update_due(st.as_ref(), now) {
        return (false, install.version.clone(), None, "recent");
    }
    let latest = match oino_extension::http::get(PYPI_URL)
        .timeout_ms(8_000)
        .send()
        .await
    {
        Ok(r) if r.status() == 200 => r.json::<Value>().await.ok().and_then(|v| {
            v.pointer("/info/version")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned)
        }),
        _ => None,
    };
    let prev = st.map(|s| s.latest_version).unwrap_or_default();
    let _ = std::fs::write(
        state_file,
        serde_json::to_vec_pretty(&UpdateState {
            checked_at: now,
            latest_version: latest.clone().unwrap_or(prev),
        })
        .unwrap_or_default(),
    );
    let Some(latest) = latest else {
        return (false, install.version.clone(), None, "lookup-failed");
    };
    let current = probe_version(&bins.cli)
        .await
        .map_or_else(|| install.version.clone(), |(v, _)| v);
    if compare_versions(&latest, &current) != std::cmp::Ordering::Greater {
        return (false, current, Some(latest), "up-to-date");
    }
    if !run_ok("uv", &["--version"], 5_000).await {
        return (false, current, Some(latest), "uv-missing");
    }
    let was_running = probe_daemon(p, 300).await.0;
    if was_running {
        let _ = run_ok(&bins.cli, &palace_args(p, &["daemon", "stop"]), 30_000).await;
    }
    let ok = run_ok("uv", &["tool", "upgrade", "mempalace"], 300_000).await;
    if was_running {
        let _ = run_ok(&bins.cli, &palace_args(p, &["daemon", "start"]), 30_000).await;
    }
    if ok {
        (true, current, Some(latest), "updated")
    } else {
        (false, current, Some(latest), "upgrade-failed")
    }
}

#[cfg(test)]
#[allow(
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::needless_pass_by_value,
    clippy::unnecessary_wraps,
    clippy::assert_is_empty,
    clippy::too_many_lines,
    clippy::float_cmp,
    reason = "test assertions"
)]
mod tests {
    use std::cmp::Ordering;

    use super::*;

    #[test]
    fn versions() {
        assert_eq!(compare_versions("3.10.0", "3.9.9"), Ordering::Greater);
        assert_eq!(compare_versions("3.10", "3.10.0"), Ordering::Equal);
        assert_eq!(compare_versions("unknown", "3.10.0"), Ordering::Less);
        assert_eq!(
            parse_version("mempalace v3.10.2\n- mempalace\n- mempalace-mcp"),
            Some("3.10.2".into())
        );
        assert_eq!(parse_version("3.11.0"), Some("3.11.0".into()));
        assert_eq!(
            parse_version("MemPalace 3.11.0 (rs 0.6.0-parity.1)"),
            Some("3.11.0".into())
        );
        assert!(is_rust_version("MemPalace 3.11.0 (rs 0.6.0-parity.1)"));
        assert!(!is_rust_version("mempalace v3.11.0"));
        assert_eq!(parse_version("nothing here"), None);
        assert_eq!(palace_key("/p").len(), 24);
        assert_eq!(
            lock_holder("MineAlreadyRunning: lock is held by PID 12 (/x)\nmore").as_deref(),
            Some("PID 12 (/x)")
        );
        assert!(update_due(None, 0));
        assert!(!update_due(
            Some(&UpdateState {
                checked_at: 0,
                latest_version: String::new()
            }),
            1000
        ));
    }

    #[test]
    fn bins() {
        assert_eq!(Bins::candidates("").0, [CLI]);
        assert_eq!(Bins::candidates("").1, [MCP]);
        let (cli, mcp) = Bins::candidates("/opt/mempalace-rs/bin");
        assert_eq!(
            cli,
            [
                "/opt/mempalace-rs/bin",
                "/opt/mempalace-rs/bin/mempalace",
                CLI
            ]
        );
        assert_eq!(mcp, ["/opt/mempalace-rs/bin/mempalace-mcp", MCP]);
        let (cli, mcp) = Bins::candidates("/opt/mempalace-rs/bin/mempalace");
        assert_eq!(cli[0], "/opt/mempalace-rs/bin/mempalace");
        assert_eq!(mcp[0], "/opt/mempalace-rs/bin/mempalace-mcp");
        // `PATH` only: nothing to probe, the record then has no version.
        assert!(!Bins::path().runs());
        assert_eq!(Bins::path().cli, CLI);
    }

    #[test]
    fn endpoint() {
        let d = crate::files::tmp("daemon");
        let p = Places {
            memory_root: None,
            palace: Some("/h/.mempalace/palace".into()),
            mempalace_guest: Some(d.clone()),
        };
        assert!(daemon_endpoint(&p).is_none());
        let sd = d.join("daemon").join(palace_key("/h/.mempalace/palace"));
        std::fs::create_dir_all(&sd).unwrap_or_default();
        std::fs::write(
            sd.join("endpoint.json"),
            "{\"host\": \"127.0.0.1\", \"port\": 8123}",
        )
        .unwrap_or_default();
        std::fs::write(sd.join("token"), "tok\n").unwrap_or_default();
        assert_eq!(
            daemon_endpoint(&p),
            Some(("127.0.0.1".into(), 8123, "tok".into()))
        );
    }
}
