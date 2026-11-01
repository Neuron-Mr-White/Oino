//! The desktop app's host supervisor (UNI-235; v2 host.md §5.1). At start it
//! decides, in this order:
//!
//! 1. **Adopt** — this PC's host answers on `app.sock` (or the Windows named
//!    pipe): it is already running (another app instance, or a manual
//!    `unipi-host serve`), so start nothing.
//! 2. **Spawn** — the app runs the host itself as a child:
//!    `unipi-host serve --mode app --exit-with-parent <our pid>`, logging to
//!    `<host dir>/host.log`, restarted with a 1, 2, 4 … 30 s backoff and a
//!    banner after 3 crashes in a row.
//! 3. **Unsupported** — no host binary anywhere (client-only mode).
//!
//! v2 has **no service install**: the systemd unit / LaunchAgent / logon task
//! branch (`StartService`) and `always_on.rs` are gone with DIRECTION §2
//! (owner Q4). A manual `unipi-host serve --mode manual` keeps working and is
//! adopted like any other running host.
//!
//! Everything that touches the real world (`Command`, a Tauri `AppHandle`) is
//! behind a small edge so the decision and the timing are unit-tested with fake
//! host binaries.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::sync::Mutex;
use unipi_proto::{AppErrorCode, AppErrorDto};

/// The app-managed host takes this lock-less-but-visible marker in host.log.
pub const LOG_NAME: &str = "host.log";
/// Restart delays grow 1, 2, 4 … 30 s; after this many failures the UI shows a
/// banner (the host is dead for more than a few seconds).
pub const BANNER_AFTER: u32 = 3;
/// The host's own exit code for "another serve owns this data dir"
/// (`crates/host/src/instance.rs`).
pub const EXIT_ALREADY_RUNNING: i32 = 75;

/// What the supervisor should do about this PC's host right now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Plan {
    /// A host answers on `app.sock`: leave it alone.
    Adopt,
    /// Run the host as our own child.
    Spawn,
    /// No host binary anywhere (client-only mode).
    Unsupported,
}

/// The facts [`plan`] reads. Kept a plain struct so tests build them directly.
#[derive(Debug, Clone, Default)]
pub struct HostState {
    /// `app.sock` answers a connection.
    pub socket_alive: bool,
    /// A host binary we could spawn (the bundled sidecar or one on PATH).
    pub bin: Option<PathBuf>,
}

/// §5.1's decision, in order: adopt a running host, else spawn our own.
pub fn plan(state: &HostState) -> Plan {
    if state.socket_alive {
        Plan::Adopt
    } else if state.bin.is_some() {
        Plan::Spawn
    } else {
        Plan::Unsupported
    }
}

/// Restart delay after `failures` consecutive crashes: 1, 2, 4, 8, 16, 30, 30…
pub fn backoff_secs(failures: u32) -> u64 {
    match failures {
        0 => 1,
        1 => 2,
        2 => 4,
        3 => 8,
        4 => 16,
        _ => 30,
    }
}

/// `"0.4.9"` → `(0, 4, 9)` for comparing versions.
pub fn version_tuple(version: &str) -> (u64, u64, u64) {
    let mut parts = version.split(['.', '-', '+']);
    let num = |p: Option<&str>| p.and_then(|p| p.parse().ok()).unwrap_or(0);
    (num(parts.next()), num(parts.next()), num(parts.next()))
}

/// Version skew between the binary we have (`ours`) and the one running
/// (`theirs`): `Some(kind)` when they differ.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Skew {
    /// Ours is newer: Settings offers "Update the background host".
    OursNewer,
    /// The running host is newer (an older app on a newer PC): read-only.
    TheirsNewer,
}

/// Compare two version strings; `None` when they are the same.
pub fn skew(ours: &str, theirs: Option<&str>) -> Option<Skew> {
    let theirs = theirs?;
    use std::cmp::Ordering::*;
    match version_tuple(ours).cmp(&version_tuple(theirs)) {
        Less => Some(Skew::TheirsNewer),
        Greater => Some(Skew::OursNewer),
        Equal => None,
    }
}

/// What the UI shows about this PC's host.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case", tag = "state")]
pub enum SupervisorState {
    /// Nothing to do: a host is running (adopted).
    Adopted,
    /// We run the host ourselves.
    AppManaged,
    /// The child keeps dying; `failures` in a row (banner at [`BANNER_AFTER`]).
    Failing { failures: u32, message: String },
    /// Nobody owns the host: another serve holds the lock.
    Held,
    /// No host binary on this PC.
    Unsupported,
}

impl SupervisorState {
    /// The banner line, when the app should nag (3 crashes in a row).
    pub fn banner(&self) -> Option<String> {
        match self {
            SupervisorState::Failing { failures, message } if *failures >= BANNER_AFTER => {
                Some(message.clone())
            }
            _ => None,
        }
    }
}

// ------------------------------------------------------------------ running

/// Runs one host child and streams its output into `<dir>/host.log`.
/// Returns the child's exit status.
/// Where one host child runs and writes its log.
pub struct ChildSpec<'a> {
    /// The host data dir (also where `host.log` goes).
    pub dir: &'a Path,
    /// The host binary.
    pub bin: &'a Path,
    /// The pid the host exits with (`--exit-with-parent`).
    pub parent_pid: u32,
    /// Filled with the child's pid so `Supervisor::stop` can kill it.
    pub child_slot: &'a Mutex<Option<u32>>,
}

pub async fn run_child(spec: ChildSpec<'_>) -> Result<std::process::ExitStatus, AppErrorDto> {
    let ChildSpec {
        dir,
        bin,
        parent_pid,
        child_slot,
    } = spec;
    use tokio::io::AsyncWriteExt;
    std::fs::create_dir_all(dir).map_err(|e| io_err("create the host dir", e))?;
    let mut log = tokio::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(LOG_NAME))
        .await
        .map_err(|e| io_err("open host.log", e))?;
    let mut command = tokio::process::Command::new(bin);
    command
        .args([
            "serve",
            "--mode",
            "app",
            "--exit-with-parent",
            &parent_pid.to_string(),
        ])
        .env("UNIPI_HOST_DIR", dir)
        // The host sees the *merged* PATH, so mise/nvm Node, herdr and tmux
        // resolve as they do in the user's shell. Empty until the start-up
        // probe has landed, and then we leave the inherited PATH alone.
        .envs({
            let merged = crate::shell_path::merged_path();
            (!merged.is_empty()).then_some(("PATH", merged))
        })
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command.spawn().map_err(|e| {
        AppErrorDto::new(
            AppErrorCode::E_INTERNAL,
            format!("could not start {}: {e}", bin.display()),
            false,
        )
    })?;
    // Remember the child (its own process group on Unix) so `stop()` can kill it.
    *child_slot.lock().await = Some(child.id().expect("a spawned child has a pid"));
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let mut out = BufReader::new(stdout).lines();
    let mut err = BufReader::new(stderr).lines();
    let pump = tokio::spawn(async move {
        // Both pipes are drained: one closing must not cut the other short.
        let mut out_done = false;
        let mut err_done = false;
        while !out_done || !err_done {
            tokio::select! {
                line = out.next_line(), if !out_done => match line {
                    Ok(Some(l)) => { let _ = log.write_all(l.as_bytes()).await; let _ = log.write_all(b"\n").await; }
                    _ => out_done = true,
                },
                line = err.next_line(), if !err_done => match line {
                    Ok(Some(l)) => { let _ = log.write_all(l.as_bytes()).await; let _ = log.write_all(b"\n").await; }
                    _ => err_done = true,
                },
            }
        }
    });
    let status = child
        .wait()
        .await
        .map_err(|e| io_err("wait for the host", e))?;
    *child_slot.lock().await = None;
    let _ = tokio::time::timeout(Duration::from_secs(2), pump).await;
    Ok(status)
}

fn io_err(what: &str, e: std::io::Error) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, format!("{what}: {e}"), false)
}

/// The last `lines` of `<dir>/host.log` (for the UI's viewer).
pub fn log_tail(dir: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(dir.join(LOG_NAME)) else {
        return String::new();
    };
    text.lines()
        .rev()
        .take(lines.max(1))
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect::<Vec<_>>()
        .join("\n")
}

/// A file is executable by its owner?
pub fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        path.metadata()
            .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

// ------------------------------------------------------------- decide

/// The sidecar's file name inside a bundle: `bundle.externalBin` strips the
/// target triple (Windows keeps the extension).
pub const SIDECAR_NAME: &str = if cfg!(windows) {
    "unipi-oino-host.exe"
} else {
    "unipi-oino-host"
};

/// The directories [`sidecar`] looks in, in order. tauri-bundler puts an
/// `externalBin` next to the app exe — `/usr/bin` for the deb and the rpm, the
/// AppImage's `usr/bin`, `Contents/MacOS`, the exe dir on Windows — and the
/// resource dir is the second candidate (a dev build, and `../lib/UniPi` for
/// the Linux bundles).
pub fn sidecar_dirs<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Vec<PathBuf> {
    use tauri::Manager;
    let mut dirs = Vec::new();
    if let Some(exe) = std::env::current_exe()
        .ok()
        .as_deref()
        .and_then(Path::parent)
    {
        dirs.push(exe.to_path_buf());
    }
    if let Ok(resource) = app.path().resource_dir() {
        dirs.push(resource);
    }
    dirs
}

/// [`sidecar`]'s lookup, as directories to search — so tests can build a
/// bundle layout without a Tauri `AppHandle`.
pub fn sidecar_in(dirs: &[PathBuf]) -> Option<PathBuf> {
    dirs.iter()
        .map(|d| d.join(SIDECAR_NAME))
        .find(|p| is_executable(p))
}

/// Where the bundled sidecar lives (UNI-236: `bundle.externalBin` =
/// `binaries/unipi-host`), or `None` when this build has no sidecar — we then
/// fall back to a `unipi-host` on PATH / `~/.local/bin`.
pub fn sidecar<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    sidecar_in(&sidecar_dirs(app))
}

/// Does `app.sock` answer a connection? (A socket nobody listens on fails
/// immediately; a stale file is not a running host.)
pub fn socket_alive(socket: &Path) -> bool {
    #[cfg(unix)]
    {
        std::os::unix::net::UnixStream::connect(socket).is_ok()
    }
    #[cfg(not(unix))]
    {
        // Windows: the local transport is the named pipe (§6.1); opening it is
        // the "answers" test (a pipe has no file to stat).
        std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(socket)
            .is_ok()
    }
}

/// Is the host that owns `dir` another app's child whose app is gone? (The
/// previous app instance after a restart: its host serves a few hundred ms more
/// before `--exit-with-parent` ends it — adopting it means "Offline" a moment
/// later.) Linux reads its `--exit-with-parent <pid>` from `/proc`; elsewhere
/// this is never known (false) and the adopt watch covers it.
pub fn orphaned_app_host(dir: &Path) -> bool {
    #[cfg(target_os = "linux")]
    {
        let Some(pid) = std::fs::read_to_string(dir.join("host.lock"))
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .filter(|v| v.get("mode").and_then(|m| m.as_str()) == Some("app"))
            .and_then(|v| v.get("pid").and_then(serde_json::Value::as_u64))
        else {
            return false;
        };
        let Ok(raw) = std::fs::read(format!("/proc/{pid}/cmdline")) else {
            return false;
        };
        parent_of_cmdline(&raw).is_some_and(|parent| !pid_alive(parent))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = dir;
        false
    }
}

/// The `--exit-with-parent <pid>` of a NUL-separated command line.
pub fn parent_of_cmdline(raw: &[u8]) -> Option<u32> {
    let args: Vec<&[u8]> = raw.split(|b| *b == 0).collect();
    let at = args.iter().position(|a| *a == b"--exit-with-parent")?;
    std::str::from_utf8(args.get(at + 1)?).ok()?.parse().ok()
}

#[cfg(target_os = "linux")]
fn pid_alive(pid: u32) -> bool {
    std::path::Path::new(&format!("/proc/{pid}")).exists()
}

/// A host that answers and is not on its way out (see [`orphaned_app_host`]).
fn serving(dir: &Path) -> bool {
    socket_alive(&crate::local_host::listen_addr(dir)) && !orphaned_app_host(dir)
}

/// Applies §5.1's decision to this PC right now and reports what it did.
/// Desktop only: the phone has no local host to supervise.
pub fn decide(host_dir: &Path, bin: Option<&Path>) -> (Plan, Option<Arc<Supervisor>>) {
    // `app.sock` on Unix, `\\.\pipe\unipi-oino-app` on Windows (§6.1).
    let state = HostState {
        socket_alive: serving(host_dir),
        bin: bin.map(Path::to_path_buf),
    };
    let plan = plan(&state);
    let Some(bin) = bin else {
        return (plan, None);
    };
    // Adopt or spawn, one supervisor: an adopted host is watched, and when it goes
    // away (the previous app's host shutting down after a restart, a manual serve
    // stopped) ours takes over at once — never "Offline" until the next launch.
    let supervisor = Supervisor::new(host_dir.to_path_buf(), bin.to_path_buf());
    // Either way the first ops wait for the local socket a moment: ours is starting,
    // or the adopted one may be the previous app's host on its way out.
    crate::local_host::expect_host_soon();
    supervisor.spawn();
    (plan, Some(supervisor))
}

/// How often an adopted host's socket is checked (and how soon ours replaces it).
pub const WATCH_EVERY: Duration = Duration::from_millis(250);
/// Exit 75 while nobody answers the socket: the previous owner is still shutting
/// down (or just starting). Retry this fast for [`LOCK_FAST_FOR`], then every
/// [`LOCK_SLOW_EVERY`].
pub const LOCK_FAST_EVERY: Duration = Duration::from_millis(200);
pub const LOCK_FAST_FOR: Duration = Duration::from_secs(15);
pub const LOCK_SLOW_EVERY: Duration = Duration::from_secs(2);

// ------------------------------------------------------------- supervisor

/// The app-managed host: one child at a time, restarted with backoff, with a
/// failure count the UI turns into a banner.
pub struct Supervisor {
    dir: PathBuf,
    bin: PathBuf,
    /// The running child's pid (its process group, so the whole browser-host
    /// tree goes with it) — killed by `stop()`.
    child: Mutex<Option<u32>>,
    /// Restart delay (tests pass a fast one).
    backoff: Arc<dyn Fn(u32) -> Duration + Send + Sync>,
    state: Mutex<SupervisorState>,
    /// Set to stop the loop (app exit, the lock moved to someone else).
    stop: tokio::sync::watch::Sender<bool>,
}

impl Supervisor {
    pub fn new(dir: PathBuf, bin: PathBuf) -> Arc<Supervisor> {
        Self::with_backoff(dir, bin, Arc::new(|f| Duration::from_secs(backoff_secs(f))))
    }

    pub fn with_backoff(
        dir: PathBuf,
        bin: PathBuf,
        backoff: Arc<dyn Fn(u32) -> Duration + Send + Sync>,
    ) -> Arc<Supervisor> {
        let (stop, _) = tokio::sync::watch::channel(false);
        Arc::new(Supervisor {
            dir,
            bin,
            backoff,
            child: Mutex::new(None),
            state: Mutex::new(SupervisorState::AppManaged),
            stop,
        })
    }

    pub async fn state(&self) -> SupervisorState {
        self.state.lock().await.clone()
    }

    pub async fn set_state(&self, state: SupervisorState) {
        *self.state.lock().await = state;
    }

    /// Stops the loop and kills the running child (its process group on Unix),
    /// so an app-managed host never outlives the app.
    pub fn stop(&self) {
        let _ = self.stop.send_replace(true);
        self.kill_child();
    }

    fn kill_child(&self) {
        let pid = self
            .child
            .try_lock()
            .ok()
            .and_then(|mut guard| guard.take());
        if let Some(pid) = pid {
            #[cfg(unix)]
            {
                // The child was started in its own process group (process_group(0)).
                let _ = std::process::Command::new("kill")
                    .args(["-KILL", "--", &format!("-{pid}")])
                    .status();
            }
            #[cfg(not(unix))]
            let _ = pid;
        }
    }

    /// The supervise loop: run the host, and when it dies, back off and run it
    /// again — unless the lock moved on (someone else owns the host now, code
    /// 75) or we were told to stop.
    pub fn spawn(self: &Arc<Self>) {
        let me = Arc::clone(self);
        tokio::spawn(async move {
            let mut failures: u32 = 0;
            // Since when the lock has been held by a host that does not answer.
            let mut held_since: Option<std::time::Instant> = None;
            let stopped = || *me.stop.subscribe().borrow_and_update();
            let socket = crate::local_host::listen_addr(&me.dir);
            while !stopped() {
                // Someone already serves this dir (adopted at start, or a manual serve):
                // watch it, and take over the moment it stops answering.
                if serving(&me.dir) {
                    me.set_state(SupervisorState::Held).await;
                    held_since = None;
                    while !stopped() && socket_alive(&socket) {
                        tokio::time::sleep(WATCH_EVERY).await;
                    }
                    if stopped() {
                        return;
                    }
                    log::info!("host: the adopted host stopped answering; starting ours");
                }
                crate::local_host::expect_host_soon();
                let status = match run_child(ChildSpec {
                    dir: &me.dir,
                    bin: &me.bin,
                    parent_pid: std::process::id(),
                    child_slot: &me.child,
                })
                .await
                {
                    Ok(status) => status,
                    Err(error) => {
                        me.set_state(SupervisorState::Failing {
                            failures: failures + 1,
                            message: error.message.to_string(),
                        })
                        .await;
                        failures += 1;
                        tokio::time::sleep((me.backoff)(failures)).await;
                        continue;
                    }
                };
                *me.child.lock().await = None;
                if stopped() {
                    return;
                }
                if status.code() == Some(EXIT_ALREADY_RUNNING) {
                    // The lock is held. A host that answers is adopted (the top of the
                    // loop watches it); one that does not is the previous owner still
                    // shutting down — try again shortly instead of giving up.
                    let since = *held_since.get_or_insert_with(std::time::Instant::now);
                    me.set_state(SupervisorState::Held).await;
                    tokio::time::sleep(if since.elapsed() < LOCK_FAST_FOR {
                        LOCK_FAST_EVERY
                    } else {
                        LOCK_SLOW_EVERY
                    })
                    .await;
                    continue;
                }
                held_since = None;
                failures += 1;
                let delay = (me.backoff)(failures);
                me.set_state(SupervisorState::Failing {
                    failures,
                    message: failing_message(&status),
                })
                .await;
                tokio::time::sleep(delay).await;
            }
        });
    }
}

/// The UI's line for a host that keeps dying.
pub fn failing_message(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("The host on this PC stopped (exit code {code}). Restarting…"),
        None => "The host on this PC was killed. Restarting…".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_bin(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join(name);
        std::fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        path
    }

    #[test]
    fn the_decision_is_adopt_or_spawn_only() {
        let dir = tempfile::tempdir().unwrap();
        let bin = fake_bin(dir.path(), "unipi-host", "exit 0");
        // A live socket always wins (someone else's serve is adopted).
        let alive = HostState {
            socket_alive: true,
            bin: Some(bin.clone()),
        };
        assert_eq!(plan(&alive), Plan::Adopt);
        // Nothing listening and a binary: run it as our child.
        let none = HostState {
            socket_alive: false,
            bin: Some(bin),
        };
        assert_eq!(plan(&none), Plan::Spawn);
        // Nothing to run.
        let no_bin = HostState::default();
        assert_eq!(plan(&no_bin), Plan::Unsupported);
    }

    #[test]
    fn reads_the_parent_of_an_app_host() {
        assert_eq!(
            parent_of_cmdline(
                b"/usr/bin/unipi-oino-host\0serve\0--mode\0app\0--exit-with-parent\x00123\0"
            ),
            Some(123)
        );
        assert_eq!(
            parent_of_cmdline(b"unipi-oino-host\0serve\0--mode\0manual\0"),
            None
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn an_app_host_whose_app_is_gone_is_not_adopted() {
        let dir = tempfile::tempdir().unwrap();
        // Our own process stands in for the host: its cmdline has no parent flag.
        std::fs::write(
            dir.path().join("host.lock"),
            format!(
                r#"{{"pid":{},"version":"1","mode":"app","started_at":0}}"#,
                std::process::id()
            ),
        )
        .unwrap();
        assert!(!orphaned_app_host(dir.path()));
        // A manual host is never "orphaned".
        std::fs::write(
            dir.path().join("host.lock"),
            r#"{"pid":1,"version":"1","mode":"manual","started_at":0}"#,
        )
        .unwrap();
        assert!(!orphaned_app_host(dir.path()));
    }

    #[test]
    fn backoff_grows_to_thirty_seconds() {
        let s: Vec<u64> = (0..7).map(backoff_secs).collect();
        assert_eq!(s, vec![1, 2, 4, 8, 16, 30, 30]);
    }

    #[test]
    fn versions_compare_numerically_not_textually() {
        assert_eq!(version_tuple("0.4.9"), (0, 4, 9));
        assert_eq!(version_tuple("0.10.0"), (0, 10, 0));
        assert_eq!(version_tuple("1.0.0-alpha.3"), (1, 0, 0));
        assert_eq!(skew("0.4.10", Some("0.4.9")), Some(Skew::OursNewer));
        assert_eq!(skew("0.10.0", Some("0.9.0")), Some(Skew::OursNewer));
        assert_eq!(skew("0.4.9", Some("0.4.10")), Some(Skew::TheirsNewer));
        assert_eq!(skew("0.4.9", Some("0.4.9")), None);
        assert_eq!(skew("0.4.9", None), None, "no running version = no skew");
    }

    #[test]
    fn a_banner_only_after_three_failures() {
        let crashed = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 1"])
            .status()
            .unwrap();
        let killed = std::process::Command::new("/bin/sh")
            .args(["-c", "kill -TERM $$"])
            .status()
            .unwrap();
        assert!(
            failing_message(&crashed).contains("exit code 1"),
            "{crashed:?}"
        );
        assert!(failing_message(&killed).contains("killed"), "{killed:?}");
        let message = failing_message(&crashed);
        assert!(SupervisorState::Failing {
            failures: 2,
            message: message.clone()
        }
        .banner()
        .is_none());
        assert!(SupervisorState::Failing {
            failures: 3,
            message: message.clone()
        }
        .banner()
        .is_some());
        assert!(SupervisorState::AppManaged.banner().is_none());
        assert!(SupervisorState::Adopted.banner().is_none());
    }

    #[tokio::test]
    async fn spawn_runs_the_host_as_an_app_child_and_logs_it() {
        let dir = tempfile::tempdir().unwrap();
        let host_dir = tempfile::tempdir().unwrap();
        // The fake host records its argv, prints two lines, then exits.
        let argv = dir.path().join("argv.txt");
        let bin = fake_bin(
            dir.path(),
            "unipi-host",
            &format!(
                "echo \"$@\" > {}\necho one\necho two\nexit 0",
                argv.display()
            ),
        );
        let slot: Mutex<Option<u32>> = Mutex::new(None);
        let status = run_child(ChildSpec {
            dir: host_dir.path(),
            bin: &bin,
            parent_pid: std::process::id(),
            child_slot: &slot,
        })
        .await
        .unwrap();
        assert!(status.success());
        let logged = log_tail(host_dir.path(), 10);
        assert!(logged.contains("one") && logged.contains("two"), "{logged}");
        let args = std::fs::read_to_string(&argv).unwrap();
        assert!(
            args.contains("serve") && args.contains("--mode") && args.contains("app"),
            "{args}"
        );
        assert!(
            args.contains(&format!("--exit-with-parent {}", std::process::id())),
            "{args}"
        );
    }

    #[tokio::test]
    async fn a_host_that_exits_75_means_someone_else_owns_the_dir() {
        let dir = tempfile::tempdir().unwrap();
        let host_dir = tempfile::tempdir().unwrap();
        let bin = fake_bin(
            dir.path(),
            "unipi-host",
            &format!(
                "echo 'already running (pid 42, mode manual)' >&2\nexit {}",
                EXIT_ALREADY_RUNNING
            ),
        );
        let slot: Mutex<Option<u32>> = Mutex::new(None);
        let status = run_child(ChildSpec {
            dir: host_dir.path(),
            bin: &bin,
            parent_pid: std::process::id(),
            child_slot: &slot,
        })
        .await
        .unwrap();
        assert_eq!(status.code(), Some(EXIT_ALREADY_RUNNING));
        assert!(log_tail(host_dir.path(), 5).contains("already running"));
    }

    #[tokio::test]
    async fn crashes_restart_with_backoff_and_banner_after_three() {
        let dir = tempfile::tempdir().unwrap();
        let host_dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("runs.txt");
        // Counts its own starts: crash four times, then stay up.
        let bin = fake_bin(
            dir.path(),
            "unipi-host",
            &format!(
                "n=$(cat {mark} 2>/dev/null | wc -l); n=$((n+1)); echo run >> {mark}; if [ $n -ge 5 ]; then sleep 30; else exit 1; fi",
                mark = mark.display()
            ),
        );
        let supervisor = Supervisor::with_backoff(
            host_dir.path().to_path_buf(),
            bin,
            Arc::new(|_| Duration::from_millis(10)),
        );
        supervisor.spawn();
        // Four crashes (the banner fires at the third), then a healthy child.
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while supervisor.state().await.banner().is_none() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let state = supervisor.state().await;
        assert!(state.banner().is_some(), "{state:?}");
        let runs = std::fs::read_to_string(&mark).unwrap().lines().count();
        assert!(runs >= 3, "expected at least 3 starts, got {runs}");
        supervisor.stop();
        // The third child is still running: stop() ends the loop and kills it
        // (kill_on_drop), so no fake host survives the test.
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[tokio::test]
    async fn a_held_lock_is_retried_until_the_previous_host_is_gone() {
        let dir = tempfile::tempdir().unwrap();
        let host_dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("runs.txt");
        // The previous host is still shutting down for the first two starts (75), then ours runs.
        let bin = fake_bin(
            dir.path(),
            "unipi-host",
            &format!(
                "n=$(cat {mark} 2>/dev/null | wc -l); n=$((n+1)); echo run >> {mark}; if [ $n -le 2 ]; then exit {code}; else sleep 30; fi",
                mark = mark.display(),
                code = EXIT_ALREADY_RUNNING
            ),
        );
        let supervisor = Supervisor::with_backoff(
            host_dir.path().to_path_buf(),
            bin,
            Arc::new(|_| Duration::from_millis(10)),
        );
        supervisor.spawn();
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while std::fs::read_to_string(&mark).map_or(0, |m| m.lines().count()) < 3
            && std::time::Instant::now() < deadline
        {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(std::fs::read_to_string(&mark).unwrap().lines().count(), 3);
        supervisor.stop();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn an_adopted_host_that_stops_answering_is_replaced_by_ours() {
        let dir = tempfile::tempdir().unwrap();
        let host_dir = tempfile::tempdir().unwrap();
        let mark = dir.path().join("runs.txt");
        let bin = fake_bin(
            dir.path(),
            "unipi-host",
            &format!("echo run >> {}; sleep 30", mark.display()),
        );
        // Someone else's host answers on app.sock: adopted, not spawned.
        let socket = crate::local_host::listen_addr(host_dir.path());
        let listener = std::os::unix::net::UnixListener::bind(&socket).unwrap();
        let (plan, supervisor) = decide(host_dir.path(), Some(&bin));
        assert_eq!(plan, Plan::Adopt);
        let supervisor = supervisor.expect("an adopted host is watched");
        tokio::time::sleep(Duration::from_millis(400)).await;
        assert!(
            !mark.exists(),
            "nothing spawned while the adopted host answers"
        );
        assert_eq!(supervisor.state().await, SupervisorState::Held);
        // It goes away: ours starts within a watch tick or two.
        drop(listener);
        let _ = std::fs::remove_file(&socket);
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        while !mark.exists() && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(mark.exists(), "our host was not started");
        supervisor.stop();
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    #[test]
    fn the_bundled_sidecar_beats_one_on_path() {
        let tmp = tempfile::tempdir().unwrap();
        // A bundle layout: the sidecar next to the app exe (where tauri-bundler
        // puts an `externalBin`), and a different host on PATH.
        let exe_dir = tmp.path().join("usr/bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let bundled = fake_bin(&exe_dir, SIDECAR_NAME, "echo bundled");
        let path_dir = tmp.path().join("path");
        std::fs::create_dir_all(&path_dir).unwrap();
        let on_path = fake_bin(&path_dir, crate::sharing::BIN_NAME, "echo on-path");
        assert_eq!(sidecar_in(&[exe_dir]), Some(bundled.clone()));
        // The sidecar wins, even with a perfectly good host on PATH.
        assert_eq!(
            host_bin_in(Some(bundled.clone()), || Some(on_path.clone())),
            Some(bundled)
        );
        assert_eq!(
            host_bin_in(None, || Some(on_path.clone())),
            Some(on_path.clone())
        );
        // No sidecar anywhere: the fallback (PATH / ~/.local/bin) is used.
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        let found = sidecar_in(&[empty]);
        assert_eq!(found, None);
        assert_eq!(
            host_bin_in(found, || crate::sharing::find_bin_in(
                None,
                None,
                None,
                Some(path_dir.as_os_str())
            )),
            Some(on_path)
        );
    }
}

// ---------------------------------------------------------------- commands

/// The app-managed host, once started. Tauri state (the tray, the settings UI
/// and the commands all read it).
#[derive(Default, Clone)]
pub struct Shared(pub Arc<tokio::sync::Mutex<Option<Arc<Supervisor>>>>);

/// What the UI sees about this PC's host (Settings → This computer).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostRuntime {
    /// `adopt` | `spawn` | `unsupported`.
    pub plan: String,
    /// `app` | `manual`, from `host.lock` (absent when nobody runs; v2 has no service).
    pub run_by: Option<String>,
    /// The running host's version, from `host.lock`.
    pub version: Option<String>,
    /// The host binary we would use (the sidecar, else the one on PATH).
    pub bin: Option<String>,
    /// `ours-newer` | `theirs-newer`: the version-skew card's state.
    pub skew: Option<Skew>,
    /// The supervisor's state (app-managed only).
    pub supervisor: Option<SupervisorState>,
    /// The banner line, when the host keeps dying.
    pub banner: Option<String>,
    /// The last lines of `<host dir>/host.log`.
    pub log_tail: Vec<String>,
}

/// `host.lock`'s owner info, read without holding the lock.
fn lock_owner(dir: &Path) -> Option<(String, String)> {
    let raw = std::fs::read_to_string(dir.join("host.lock")).ok()?;
    let value: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let mode = value.get("mode")?.as_str()?.to_string();
    let version = value.get("version")?.as_str()?.to_string();
    Some((mode, version))
}

fn plan_name(plan: Plan) -> &'static str {
    match plan {
        Plan::Adopt => "adopt",
        Plan::Spawn => "spawn",
        Plan::Unsupported => "unsupported",
    }
}

/// The host binary to run: the bundled sidecar when there is one, else one on
/// PATH / `~/.local/bin`. Split out of [`host_bin`] so tests can pass a sidecar
/// (and its fallback) without a Tauri `AppHandle`.
pub fn host_bin(app: Option<&tauri::AppHandle>) -> Option<PathBuf> {
    host_bin_in(app.and_then(sidecar), crate::sharing::bin)
}

/// The sidecar when we have one, else `otherwise` (a host on PATH /
/// `~/.local/bin`).
pub fn host_bin_in(
    sidecar: Option<PathBuf>,
    otherwise: impl FnOnce() -> Option<PathBuf>,
) -> Option<PathBuf> {
    sidecar.or_else(otherwise)
}

/// Starts the host for this PC (called from `setup`, and again by
/// `host_restart`). Adopts a running one, or spawns our own child.
pub async fn start(app: Option<&tauri::AppHandle>, dir: &Path, shared: &Shared) -> Plan {
    // The sidecar is what this build ships (§5.1 step 2); a PATH host is only
    // ever the fallback.
    let bundled = app.and_then(sidecar);
    if let Some(bin) = &bundled {
        log::info!("host: using the bundled sidecar {}", bin.display());
    }
    let bin = host_bin_in(bundled, crate::sharing::bin);
    let (plan, supervisor) = decide(dir, bin.as_deref());
    if let Some(s) = supervisor {
        *shared.0.lock().await = Some(s);
    }
    plan
}

/// `$HOME` (`%USERPROFILE%` on Windows).
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// `host_runtime`: everything Settings → This computer shows about the host.
#[tauri::command]
pub async fn host_runtime(
    shared: tauri::State<'_, Shared>,
    host_dir: Option<String>,
    app: tauri::AppHandle,
) -> Result<HostRuntime, AppErrorDto> {
    let dir = host_dir
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::local_host::host_dir().unwrap_or_default());
    let guard = shared.0.lock().await;
    let supervisor = guard.as_ref();
    let (run_by, version) = lock_owner(&dir).unzip();
    let running_version = version.clone();
    let app_handle = || Some(app.clone());
    let bin = host_bin(Some(&app));
    let state = match supervisor {
        Some(s) => Some(s.state().await),
        None => None,
    };
    let socket_up = socket_alive(&crate::local_host::listen_addr(&dir));
    let (plan, supervisor_state) = match &state {
        Some(SupervisorState::Held) => (Plan::Adopt, Some(SupervisorState::Adopted)),
        Some(other) => (Plan::Spawn, Some(other.clone())),
        None if socket_up => (Plan::Adopt, Some(SupervisorState::Adopted)),
        None if bin.is_some() => (Plan::Spawn, Some(SupervisorState::AppManaged)),
        None => (Plan::Unsupported, Some(SupervisorState::Unsupported)),
    };
    // The tray's §5.3 line follows the host: how many phones are paired.
    #[cfg(desktop)]
    if let Some(handle) = app_handle() {
        crate::desktop::set_phone_count(&handle, crate::local_host::paired_devices());
    }
    let _ = &supervisor_state;
    Ok(HostRuntime {
        plan: plan_name(plan).to_string(),
        run_by,
        version,
        skew: skew(env!("CARGO_PKG_VERSION"), running_version.as_deref()),
        banner: state.as_ref().and_then(|s| s.banner()),
        supervisor: supervisor_state,
        bin: bin.map(|b| b.display().to_string()),
        log_tail: log_tail(&dir, 40).lines().map(str::to_string).collect(),
    })
}

/// `host_log`: the tail of `<host dir>/host.log`, newest last.
#[tauri::command]
pub fn host_log(host_dir: Option<String>, lines: Option<usize>) -> Vec<String> {
    let dir = host_dir
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::local_host::host_dir().unwrap_or_default());
    log_tail(&dir, lines.unwrap_or(200))
        .lines()
        .map(str::to_string)
        .collect()
}

/// `host_restart`: stop our child (if any) and apply the §3.2 decision again.
#[tauri::command]
pub async fn host_restart(
    shared: tauri::State<'_, Shared>,
    host_dir: Option<String>,
) -> Result<String, AppErrorDto> {
    let dir = host_dir
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::local_host::host_dir().unwrap_or_default());
    if let Some(s) = shared.0.lock().await.take() {
        s.stop();
    }
    Ok(plan_name(start(None, &dir, &shared).await).to_string())
}

/// Against the real `unipi-host` binary and a throwaway host dir: the spawn
/// path runs it as our own child (`--mode app`), `host.lock` says so, and the
/// local socket answers (so the next decision is "adopt").
#[cfg(all(test, unix))]
mod e2e {
    use super::*;

    #[tokio::test(flavor = "multi_thread")]
    async fn spawns_a_real_host_as_an_app_child() {
        let bin =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/debug/unipi-host");
        if !bin.exists() {
            eprintln!("SKIP: build unipi-host first ({})", bin.display());
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let host_dir = dir.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let socket = host_dir.join("app.sock");
        assert_eq!(
            plan(&HostState {
                socket_alive: socket_alive(&socket),
                bin: Some(bin.clone())
            }),
            Plan::Spawn
        );

        let supervisor = Supervisor::new(host_dir.clone(), bin.clone());
        supervisor.spawn();
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while !socket_alive(&socket) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            socket_alive(&socket),
            "the app-managed host never answered on app.sock"
        );
        assert!(
            matches!(supervisor.state().await, SupervisorState::AppManaged),
            "{:?}",
            supervisor.state().await
        );
        assert_eq!(
            plan(&HostState {
                socket_alive: socket_alive(&socket),
                bin: Some(bin.clone())
            }),
            Plan::Adopt
        );
        let (mode, version) = lock_owner(&host_dir).expect("host.lock owner");
        assert_eq!(mode, "app", "the host records who started it");
        assert!(!version.is_empty());
        assert!(
            log_tail(&host_dir, 50).lines().count() > 0,
            "its output goes to host.log"
        );
        // Stopping the supervisor ends the child (kill_on_drop), so the lock is
        // released and nothing survives the test.
        supervisor.stop();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while lock_owner_live(&host_dir) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            !lock_owner_live(&host_dir),
            "the host must not outlive the app"
        );
    }

    /// Is a live host holding the dir's lock right now?
    fn lock_owner_live(dir: &Path) -> bool {
        let Ok(file) = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(dir.join("host.lock"))
        else {
            return false;
        };
        file.try_lock().is_err()
    }

    /// The **bundled sidecar** (UNI-236): resolved the way `sidecar()` does — a
    /// copy next to the app exe, in a `binaries/`-shaped dir — then run as our
    /// own child. The static musl build is the one `unipi-host-sidecar.sh`
    /// puts in `apps/mobile/src-tauri/binaries/`.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_bundled_sidecar_runs_as_the_app_managed_host() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        // Only a host built for this machine can be executed here.
        let triples: [&str; 2] = if cfg!(target_arch = "x86_64") {
            ["x86_64-unknown-linux-musl", ""]
        } else {
            ["aarch64-unknown-linux-musl", ""]
        };
        let built = triples
            .iter()
            .map(|t| {
                if t.is_empty() {
                    manifest.join("../../target/release/unipi-host")
                } else {
                    manifest.join(format!("../../target/{t}/release/unipi-host"))
                }
            })
            .find(|p| is_executable(p));
        let Some(built) = built else {
            eprintln!("SKIP: build the sidecar first (apps/unipi/src-tauri/unipi-host-sidecar.sh)");
            return;
        };

        let tmp = tempfile::tempdir().unwrap();
        let exe_dir = tmp.path().join("usr/bin");
        std::fs::create_dir_all(&exe_dir).unwrap();
        let bundled = exe_dir.join(SIDECAR_NAME);
        std::fs::copy(&built, &bundled).expect("copy the sidecar into the bundle layout");
        assert_eq!(
            sidecar_in(&[exe_dir]),
            Some(bundled.clone()),
            "the sidecar is found next to the exe"
        );

        let host_dir = tmp.path().join("host");
        std::fs::create_dir_all(&host_dir).unwrap();
        let socket = host_dir.join("app.sock");
        assert_eq!(
            plan(&HostState {
                socket_alive: socket_alive(&socket),
                bin: Some(bundled.clone())
            }),
            Plan::Spawn
        );
        let supervisor = Supervisor::new(host_dir.clone(), bundled.clone());
        supervisor.spawn();
        let deadline = std::time::Instant::now() + Duration::from_secs(25);
        while !socket_alive(&socket) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            socket_alive(&socket),
            "the app-managed host never answered on app.sock"
        );
        assert_eq!(
            lock_owner(&host_dir).expect("host.lock owner").0,
            "app",
            "the host records who started it"
        );
        // The next decision adopts it: nothing is spawned twice.
        assert_eq!(
            plan(&HostState {
                socket_alive: socket_alive(&socket),
                bin: Some(bundled)
            }),
            Plan::Adopt
        );
        supervisor.stop();
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while lock_owner_live(&host_dir) && std::time::Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(
            !lock_owner_live(&host_dir),
            "the host must not outlive the app"
        );
    }
}

/// Is the host running as *our* child right now (`host.lock` says mode "app")?
/// Read-only and sync, for the tray and the Quit confirm.
pub fn app_managed_blocking() -> bool {
    lock_owner(&crate::local_host::host_dir().unwrap_or_default())
        .map(|(mode, _)| mode == "app")
        .unwrap_or(false)
}
