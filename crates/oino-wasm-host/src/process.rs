//! `process`: spawning, streaming, signals, process trees and stats (extension-api §5.4).
//!
//! # Platform support
//!
//! | Feature | Linux | macOS | Windows |
//! |---|---|---|---|
//! | `proc-stats` | `/proc` | `ps -o ppid=,stat=,rss=,time=,etime=` | PowerShell `Get-CimInstance Win32_Process` |
//! | `process-tree` | `/proc` parents | `ps -A -o pid=,ppid=` | `Get-CimInstance Win32_Process` |
//! | signals | `nix` (`kill`/`killpg`) | `nix` | `taskkill` (`/T` kills the tree) |
//!
//! Fields the platform does not expose are reported as `0`/empty: macOS has no per-process byte
//! counters, and Windows reports no process state or byte counters. The watchdog only needs CPU,
//! RSS and the child tree, all of which are available everywhere.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::stream::BoxStream;
use oino_extension_api::HostError;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{mpsc, oneshot, watch};

use crate::state::lock;
use crate::streams::receiver_stream;

/// Where a child's stdio goes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Output {
    Null,
    Pipe,
    File(PathBuf),
}

/// Stdin mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Input {
    Null,
    Pipe,
}

/// Spawn request (WIT `spawn-spec`, already permission-checked and path-resolved).
#[derive(Clone, Debug)]
pub(crate) struct Spawn {
    pub(crate) command: String,
    pub(crate) args: Vec<String>,
    pub(crate) cwd: PathBuf,
    pub(crate) env: Vec<(String, String)>,
    pub(crate) clear_env: bool,
    pub(crate) stdin: Input,
    pub(crate) stdout: Output,
    pub(crate) stderr: Output,
    pub(crate) new_process_group: bool,
    pub(crate) detached: bool,
    pub(crate) timeout: Option<Duration>,
}

/// Exit status (WIT `exit-status`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Exit {
    pub(crate) code: Option<i32>,
    pub(crate) signal: Option<i32>,
    pub(crate) timed_out: bool,
}

/// Signals (WIT `signal`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Sig {
    Term,
    Kill,
    Int,
    Hup,
}

/// A spawned child.
pub(crate) struct ChildHandle {
    pub(crate) pid: u32,
    group: bool,
    stdin: tokio::sync::Mutex<Option<ChildStdin>>,
    stdout: Mutex<Option<BoxStream<'static, Vec<u8>>>>,
    stderr: Mutex<Option<BoxStream<'static, Vec<u8>>>>,
    exit: watch::Receiver<Option<Exit>>,
    kill: Mutex<Option<oneshot::Sender<()>>>,
}

fn pipe_stream<R: AsyncRead + Unpin + Send + 'static>(
    mut reader: R,
) -> BoxStream<'static, Vec<u8>> {
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let mut buf = vec![0u8; 16 * 1024];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    receiver_stream(rx)
}

fn open_append(path: &Path) -> Result<Stdio, HostError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| HostError::io(&e))?;
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| HostError::io(&e))?;
    Ok(Stdio::from(file))
}

fn stdio_for(output: &Output) -> Result<Stdio, HostError> {
    match output {
        Output::Null => Ok(Stdio::null()),
        Output::Pipe => Ok(Stdio::piped()),
        Output::File(path) => open_append(path),
    }
}

#[cfg(unix)]
fn exit_of(status: std::process::ExitStatus, timed_out: bool) -> Exit {
    use std::os::unix::process::ExitStatusExt;
    Exit {
        code: status.code(),
        signal: status.signal(),
        timed_out,
    }
}

#[cfg(not(unix))]
fn exit_of(status: std::process::ExitStatus, timed_out: bool) -> Exit {
    Exit {
        code: status.code(),
        signal: None,
        timed_out,
    }
}

/// A bare program name not found on `PATH` falls back to the directory of the running
/// executable, where installs and app bundles ship companion binaries such as
/// `oino-kanboard` (GA-41). Anything else is returned unchanged.
pub(crate) fn resolve_program(command: &str) -> PathBuf {
    let bare = PathBuf::from(command);
    if command.is_empty() || bare.components().count() != 1 {
        return bare;
    }
    let names: Vec<String> = if cfg!(windows) && bare.extension().is_none() {
        vec![format!("{command}.exe"), command.to_owned()]
    } else {
        vec![command.to_owned()]
    };
    let on_path = std::env::var_os("PATH").is_some_and(|path| {
        std::env::split_paths(&path).any(|dir| names.iter().any(|n| dir.join(n).is_file()))
    });
    if on_path {
        return bare;
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf))
        .and_then(|dir| names.iter().map(|n| dir.join(n)).find(|p| p.is_file()))
        .unwrap_or(bare)
}

/// Tracks non-detached children so they die with the extension.
#[derive(Default)]
pub(crate) struct ProcessService {
    live: Mutex<Vec<(u32, bool)>>,
}

impl ProcessService {
    /// Spawns `spec`; `log` is the extension log file (for `inherit-log`).
    pub(crate) fn spawn(&self, spec: &Spawn) -> Result<Arc<ChildHandle>, HostError> {
        let mut cmd = Command::new(resolve_program(&spec.command));
        cmd.args(&spec.args).current_dir(&spec.cwd);
        if spec.clear_env {
            cmd.env_clear();
        }
        cmd.envs(spec.env.iter().map(|(k, v)| (k, v)));
        cmd.stdin(match spec.stdin {
            Input::Null => Stdio::null(),
            Input::Pipe => Stdio::piped(),
        });
        cmd.stdout(stdio_for(&spec.stdout)?);
        cmd.stderr(stdio_for(&spec.stderr)?);
        let group = spec.new_process_group || spec.detached;
        #[cfg(unix)]
        if group {
            cmd.process_group(0);
        }
        cmd.kill_on_drop(false);
        let mut child = cmd
            .spawn()
            .map_err(|e| HostError::Failed(format!("spawn {}: {e}", spec.command)))?;
        let pid = child.id().unwrap_or(0);
        let stdout = child.stdout.take().map(pipe_stream);
        let stderr = child.stderr.take().map(pipe_stream);
        let stdin = child.stdin.take();
        let (exit_tx, exit_rx) = watch::channel(None);
        let (kill_tx, kill_rx) = oneshot::channel::<()>();
        let timeout = spec.timeout;
        let detached = spec.detached;
        tokio::spawn(async move {
            let mut kill_rx = kill_rx;
            let mut kill_open = true;
            let deadline = async {
                match timeout {
                    Some(t) => tokio::time::sleep(t).await,
                    None => std::future::pending().await,
                }
            };
            tokio::pin!(deadline);
            let mut timed_out = false;
            let status = loop {
                tokio::select! {
                    status = child.wait() => break status,
                    () = &mut deadline, if !timed_out => {
                        timed_out = true;
                        kill_tree(pid, group);
                        let _ = child.start_kill();
                    }
                    asked = &mut kill_rx, if kill_open => {
                        // An explicit kill, or the guest dropped its handle: a detached child
                        // (a daemon) outlives its handle; any other child dies with it.
                        if asked.is_ok() || !detached {
                            let _ = child.start_kill();
                        }
                        kill_open = false;
                    }
                }
            };
            let exit = status.map_or(
                Exit {
                    code: None,
                    signal: None,
                    timed_out,
                },
                |s| exit_of(s, timed_out),
            );
            let _ = exit_tx.send(Some(exit));
        });
        if !spec.detached {
            lock(&self.live).push((pid, group));
        }
        Ok(Arc::new(ChildHandle {
            pid,
            group,
            stdin: tokio::sync::Mutex::new(stdin),
            stdout: Mutex::new(stdout),
            stderr: Mutex::new(stderr),
            exit: exit_rx,
            kill: Mutex::new(Some(kill_tx)),
        }))
    }

    /// Kills every tracked (non-detached) child tree.
    pub(crate) fn kill_all(&self) {
        for (pid, group) in lock(&self.live).drain(..) {
            kill_tree(pid, group);
        }
    }
}

impl Drop for ProcessService {
    fn drop(&mut self) {
        self.kill_all();
    }
}

impl ChildHandle {
    pub(crate) async fn write_stdin(&self, data: &[u8]) -> Result<(), HostError> {
        let mut stdin = self.stdin.lock().await;
        let pipe = stdin
            .as_mut()
            .ok_or_else(|| HostError::InvalidArgument("stdin is not piped".into()))?;
        pipe.write_all(data).await.map_err(|e| HostError::io(&e))?;
        pipe.flush().await.map_err(|e| HostError::io(&e))
    }

    pub(crate) fn close_stdin(&self) {
        if let Ok(mut stdin) = self.stdin.try_lock() {
            stdin.take();
        }
    }

    pub(crate) fn take_stdout(&self) -> Option<BoxStream<'static, Vec<u8>>> {
        lock(&self.stdout).take()
    }

    pub(crate) fn take_stderr(&self) -> Option<BoxStream<'static, Vec<u8>>> {
        lock(&self.stderr).take()
    }

    pub(crate) async fn wait(&self) -> Exit {
        let mut rx = self.exit.clone();
        loop {
            if let Some(exit) = *rx.borrow() {
                return exit;
            }
            if rx.changed().await.is_err() {
                return rx.borrow().unwrap_or_default();
            }
        }
    }

    pub(crate) fn try_wait(&self) -> Option<Exit> {
        *self.exit.borrow()
    }

    pub(crate) fn signal(&self, sig: Sig, group: bool) -> Result<(), HostError> {
        if self.try_wait().is_some() {
            return Ok(());
        }
        #[cfg(unix)]
        {
            send_signal(self.pid, sig, group && self.group)
        }
        #[cfg(windows)]
        {
            signal_windows(self.pid, sig, group && self.group)
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = (sig, group);
            if let Some(tx) = lock(&self.kill).take() {
                let _ = tx.send(());
            }
            Ok(())
        }
    }

    /// Kills the child (tree when it leads a group).
    pub(crate) fn kill(&self) {
        kill_tree(self.pid, self.group);
        if let Some(tx) = lock(&self.kill).take() {
            let _ = tx.send(());
        }
    }
}

#[cfg(unix)]
fn send_signal(pid: u32, sig: Sig, group: bool) -> Result<(), HostError> {
    use nix::sys::signal::{Signal, kill, killpg};
    use nix::unistd::Pid;
    let signal = match sig {
        Sig::Term => Signal::SIGTERM,
        Sig::Kill => Signal::SIGKILL,
        Sig::Int => Signal::SIGINT,
        Sig::Hup => Signal::SIGHUP,
    };
    let raw = i32::try_from(pid).map_err(|_| HostError::InvalidArgument("pid".into()))?;
    let result = if group {
        killpg(Pid::from_raw(raw), signal)
    } else {
        kill(Pid::from_raw(raw), signal)
    };
    result.map_err(|e| HostError::Failed(format!("signal {pid}: {e}")))
}

/// SIGKILLs a process (its group when it leads one, plus descendants found via `/proc`).
pub(crate) fn kill_tree(pid: u32, group: bool) {
    if pid == 0 {
        return;
    }
    #[cfg(unix)]
    {
        for child in process_tree(pid).unwrap_or_default().into_iter().rev() {
            let _ = send_signal(child, Sig::Kill, false);
        }
        let _ = send_signal(pid, Sig::Kill, group);
    }
    #[cfg(windows)]
    {
        // `taskkill /T` walks the tree itself; `/F` is the SIGKILL equivalent.
        let _ = signal_windows(pid, Sig::Kill, group);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = group;
}

/// Result of [`exec`] (pi `exec` shape).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct ExecOutput {
    pub(crate) stdout: String,
    pub(crate) stderr: String,
    pub(crate) code: Option<i32>,
    pub(crate) killed: bool,
}

/// pi `exec(command, args, {timeout, cwd})`.
pub(crate) async fn exec(
    command: &str,
    args: &[String],
    cwd: &Path,
    env: &[(String, String)],
    timeout: Option<Duration>,
) -> Result<ExecOutput, HostError> {
    let mut cmd = Command::new(resolve_program(command));
    cmd.args(args)
        .current_dir(cwd)
        .envs(env.iter().map(|(k, v)| (k, v)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);
    let child = cmd
        .spawn()
        .map_err(|e| HostError::Failed(format!("spawn {command}: {e}")))?;
    let pid = child.id().unwrap_or(0);
    let output = child.wait_with_output();
    let result = match timeout {
        Some(t) => tokio::time::timeout(t, output).await.ok(),
        None => Some(output.await),
    };
    match result {
        Some(Ok(out)) => Ok(ExecOutput {
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            code: out.status.code(),
            killed: out.status.code().is_none(),
        }),
        Some(Err(e)) => Err(HostError::io(&e)),
        None => {
            kill_tree(pid, true);
            Ok(ExecOutput {
                killed: true,
                ..ExecOutput::default()
            })
        }
    }
}

/// Descendants of `pid` (breadth-first), Linux `/proc` scan.
pub(crate) fn process_tree(pid: u32) -> Result<Vec<u32>, HostError> {
    let parents = proc_parents()?;
    let mut out = Vec::new();
    let mut frontier = vec![pid];
    while let Some(p) = frontier.pop() {
        for &(child, parent) in &parents {
            if parent == p && !out.contains(&child) {
                out.push(child);
                frontier.push(child);
            }
        }
    }
    Ok(out)
}

#[cfg(target_os = "linux")]
fn proc_parents() -> Result<Vec<(u32, u32)>, HostError> {
    let dir = std::fs::read_dir("/proc").map_err(|e| HostError::io(&e))?;
    Ok(dir
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str()?.parse::<u32>().ok())
        .filter_map(|pid| Some((pid, read_stat(pid)?.ppid)))
        .collect())
}

#[cfg(target_os = "macos")]
fn proc_parents() -> Result<Vec<(u32, u32)>, HostError> {
    let out = run_tool("ps", &["-A", "-o", "pid=,ppid="])?;
    parse_ps_parents(&out).ok_or_else(|| HostError::Failed("ps output".into()))
}

#[cfg(windows)]
fn proc_parents() -> Result<Vec<(u32, u32)>, HostError> {
    let script = "Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId | ConvertTo-Json -Compress";
    let out = run_powershell(script)?;
    parse_cim_parents(&out).ok_or_else(|| HostError::Failed("Win32_Process output".into()))
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn proc_parents() -> Result<Vec<(u32, u32)>, HostError> {
    Err(HostError::unsupported("process-tree on this platform"))
}

/// WIT `proc-stats`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Stats {
    pub(crate) pid: u32,
    pub(crate) ppid: u32,
    pub(crate) state: String,
    pub(crate) cpu_ms: u64,
    pub(crate) rss_bytes: u64,
    pub(crate) read_bytes: u64,
    pub(crate) write_bytes: u64,
    pub(crate) start_time_ms: u64,
}

const CLOCK_TICKS: u64 = 100;
const PAGE_SIZE: u64 = 4096;

/// Parses `/proc/<pid>/stat` (+ `io`, `statm`).
#[cfg_attr(
    not(target_os = "linux"),
    allow(dead_code, reason = "Linux-only implementation")
)]
fn read_stat(pid: u32) -> Option<Stats> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let close = stat.rfind(')')?;
    let fields: Vec<&str> = stat[close + 2..].split_whitespace().collect();
    let num = |i: usize| {
        fields
            .get(i)
            .and_then(|f| f.parse::<u64>().ok())
            .unwrap_or(0)
    };
    let state = fields.first().copied().unwrap_or("?").to_owned();
    let parent = u32::try_from(num(1)).unwrap_or(0);
    let cpu_ms = (num(11) + num(12)) * 1000 / CLOCK_TICKS;
    let start_ticks = num(19);
    let rss_pages = std::fs::read_to_string(format!("/proc/{pid}/statm"))
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .unwrap_or(0);
    let io = std::fs::read_to_string(format!("/proc/{pid}/io")).unwrap_or_default();
    let io_field = |name: &str| {
        io.lines()
            .find_map(|l| l.strip_prefix(name)?.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    let btime = std::fs::read_to_string("/proc/stat")
        .ok()
        .and_then(|s| {
            s.lines()
                .find_map(|l| l.strip_prefix("btime ")?.trim().parse::<u64>().ok())
        })
        .unwrap_or(0);
    Some(Stats {
        pid,
        ppid: parent,
        state,
        cpu_ms,
        rss_bytes: rss_pages * PAGE_SIZE,
        read_bytes: io_field("read_bytes:"),
        write_bytes: io_field("write_bytes:"),
        start_time_ms: btime * 1000 + start_ticks * 1000 / CLOCK_TICKS,
    })
}

// ── platform helpers (macOS `ps`, Windows `Get-CimInstance`) ────────────────

/// Runs a short-lived tool and returns its trimmed stdout.
#[cfg(any(target_os = "macos", windows))]
fn run_tool(program: &str, args: &[&str]) -> Result<String, HostError> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| HostError::Failed(format!("{program}: {e}")))?;
    if !out.status.success() {
        return Err(HostError::Failed(format!(
            "{program} exited with {:?}",
            out.status.code()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// Runs a bounded PowerShell script (CIM queries take ~100–300 ms).
#[cfg(windows)]
fn run_powershell(script: &str) -> Result<String, HostError> {
    run_tool(
        "powershell.exe",
        &[
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ],
    )
}

/// Milliseconds since the Unix epoch (wall clock).
#[cfg(target_os = "macos")]
fn unix_now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

/// `ps -A -o pid=,ppid=` → `(pid, ppid)` pairs.
#[cfg(any(target_os = "macos", test))]
fn parse_ps_parents(out: &str) -> Option<Vec<(u32, u32)>> {
    let mut pairs = Vec::new();
    for line in out.lines() {
        let mut fields = line.split_whitespace();
        let child: u32 = fields.next()?.parse().ok()?;
        let parent: u32 = fields.next()?.parse().ok()?;
        pairs.push((child, parent));
    }
    Some(pairs)
}

/// `ps -p <pid> -o ppid=,stat=,rss=,time=,etime=` → [`Stats`].
///
/// `time` is the CPU time, `etime` the elapsed wall time (`[[dd-]hh:]mm:ss`), so the start time is
/// `now - etime`. macOS exposes no per-process read/write byte counters: those stay `0`.
#[cfg(any(target_os = "macos", test))]
fn parse_ps_stats(pid: u32, out: &str, now_ms: u64) -> Option<Stats> {
    let line = out.lines().find(|l| !l.trim().is_empty())?;
    let fields: Vec<&str> = line.split_whitespace().collect();
    let [parent, state, rss, cpu, elapsed, ..] = fields.as_slice() else {
        return None;
    };
    let parent: u32 = parent.parse().ok()?;
    let rss_kib: u64 = rss.parse().ok()?;
    Some(Stats {
        pid,
        ppid: parent,
        state: (*state).to_owned(),
        cpu_ms: parse_clock_ms(cpu)?,
        rss_bytes: rss_kib.saturating_mul(1024),
        read_bytes: 0,
        write_bytes: 0,
        start_time_ms: now_ms.saturating_sub(parse_clock_ms(elapsed)?),
    })
}

/// `[[dd-]hh:]mm:ss[.frac]` (or plain seconds) → milliseconds.
#[cfg(any(target_os = "macos", test))]
fn parse_clock_ms(text: &str) -> Option<u64> {
    let (days, rest) = match text.split_once('-') {
        Some((days, rest)) => (days.parse::<u64>().ok()?, rest),
        None => (0, text),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    let seconds = parts.last()?.replace(',', ".");
    let seconds: f64 = seconds.parse().ok()?;
    let mut ms = seconds * 1000.0;
    let mut unit = 60_000.0;
    for part in parts.iter().rev().skip(1) {
        ms += part.parse::<f64>().ok()? * unit;
        unit *= 60.0;
    }
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss,
        reason = "clock components are far below f64's exact integer range"
    )]
    let ms = (ms + (days * 86_400_000) as f64) as u64;
    Some(ms)
}

/// `Get-CimInstance Win32_Process | Select ProcessId,ParentProcessId | ConvertTo-Json` → pairs.
#[cfg(any(windows, test))]
fn parse_cim_parents(out: &str) -> Option<Vec<(u32, u32)>> {
    let value: serde_json::Value = serde_json::from_str(out).ok()?;
    let items = match value {
        serde_json::Value::Array(items) => items,
        object @ serde_json::Value::Object(_) => vec![object],
        _ => return None,
    };
    let mut pairs = Vec::new();
    for row in items {
        let child = row.get("ProcessId").and_then(serde_json::Value::as_u64)?;
        let parent = row
            .get("ParentProcessId")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        pairs.push((
            u32::try_from(child).ok()?,
            u32::try_from(parent).unwrap_or(0),
        ));
    }
    Some(pairs)
}

/// One `Win32_Process` row → [`Stats`]: `WorkingSetSize` bytes, CPU from `KernelModeTime` +
/// `UserModeTime` (100 ns units), start from `CreationDate` (ISO 8601). Windows exposes no
/// process state or byte counters.
#[cfg(any(windows, test))]
fn parse_cim_stats(out: &str) -> Option<Stats> {
    let value: serde_json::Value = serde_json::from_str(out).ok()?;
    let row = if value.is_array() {
        value.as_array()?.first()?.clone()
    } else {
        value
    };
    let num = |key: &str| row.get(key).and_then(serde_json::Value::as_u64);
    let pid = u32::try_from(num("ProcessId")?).ok()?;
    let cpu_100ns = num("KernelModeTime").unwrap_or(0) + num("UserModeTime").unwrap_or(0);
    let start_time_ms = row
        .get("CreationDate")
        .and_then(serde_json::Value::as_str)
        .and_then(parse_iso_ms)
        .unwrap_or(0);
    Some(Stats {
        pid,
        ppid: u32::try_from(num("ParentProcessId").unwrap_or(0)).unwrap_or(0),
        state: "R".to_owned(),
        cpu_ms: cpu_100ns / 10_000,
        rss_bytes: num("WorkingSetSize").unwrap_or(0),
        read_bytes: 0,
        write_bytes: 0,
        start_time_ms,
    })
}

/// `YYYY-MM-DDTHH:MM:SS[.frac](Z|±HH:MM)` → epoch milliseconds.
#[cfg(any(windows, test))]
fn parse_iso_ms(text: &str) -> Option<u64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let number = |range: std::ops::Range<usize>| text.get(range)?.parse::<i64>().ok();
    let (year, month, day) = (number(0..4)?, number(5..7)?, number(8..10)?);
    let (hour, minute, second) = (number(11..13)?, number(14..16)?, number(17..19)?);
    let days = days_from_civil(year, month, day);
    let mut ms = days * 86_400_000 + hour * 3_600_000 + minute * 60_000 + second * 1000;
    if let Some(rest) = text.get(19..) {
        if let Some(frac) = rest.strip_prefix('.') {
            let digits: String = frac.chars().take_while(char::is_ascii_digit).collect();
            let millis = format!("{digits:0<3}");
            ms += millis.get(..3)?.parse::<i64>().ok()?;
            let offset = frac.get(digits.len()..).unwrap_or_default();
            ms -= offset_minutes(offset)? * 60_000;
        } else if !rest.is_empty() {
            ms -= offset_minutes(rest)? * 60_000;
        }
    }
    u64::try_from(ms).ok()
}

/// `Z`, `+HH:MM`, `-HH:MM`, `+HHMM` → minutes east of UTC (`0` when absent).
#[cfg(any(windows, test))]
fn offset_minutes(text: &str) -> Option<i64> {
    let sign = match text.as_bytes().first()? {
        b'+' => 1,
        b'-' => -1,
        // `Z`, or a value without an offset.
        _ => return Some(0),
    };
    let digits: String = text
        .chars()
        .skip(1)
        .filter(char::is_ascii_digit)
        .take(4)
        .collect();
    let hours: i64 = digits.get(0..2)?.parse().ok()?;
    let minutes: i64 = digits.get(2..4).unwrap_or("0").parse().ok()?;
    Some(sign * (hours * 60 + minutes))
}

/// Days since 1970-01-01 of a civil date (Howard Hinnant's `days_from_civil`).
#[cfg(any(windows, test))]
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = if year >= 0 { year } else { year - 399 } / 400;
    let year_of_era = year - era * 400;
    let month_index = (month + 9) % 12;
    let day_of_year = (153 * month_index + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// `taskkill` on Windows: `TERM` asks politely, `KILL` forces; both take the tree with `/T`.
#[cfg(windows)]
fn signal_windows(pid: u32, sig: Sig, group: bool) -> Result<(), HostError> {
    let mut command = std::process::Command::new("taskkill");
    command.arg("/PID").arg(pid.to_string());
    if group {
        command.arg("/T");
    }
    if matches!(sig, Sig::Kill) {
        command.arg("/F");
    }
    let out = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| HostError::Failed(format!("taskkill {pid}: {e}")))?;
    if out.success() {
        Ok(())
    } else {
        Err(HostError::Failed(format!("taskkill {pid} failed")))
    }
}

/// Stats for `pid`.
pub(crate) fn process_stats(pid: u32) -> Result<Stats, HostError> {
    #[cfg(target_os = "linux")]
    {
        read_stat(pid).ok_or_else(|| HostError::NotFound(format!("process {pid}")))
    }
    #[cfg(target_os = "macos")]
    {
        let out = run_tool(
            "ps",
            &[
                "-p",
                &pid.to_string(),
                "-o",
                "ppid=,stat=,rss=,time=,etime=",
            ],
        )?;
        parse_ps_stats(pid, &out, unix_now_ms())
            .ok_or_else(|| HostError::NotFound(format!("process {pid}")))
    }
    #[cfg(windows)]
    {
        let script = format!(
            "$p = Get-CimInstance Win32_Process -Filter 'ProcessId={pid}';\
             if ($p) {{ $p | Select-Object ProcessId,ParentProcessId,WorkingSetSize,KernelModeTime,\
             UserModeTime,CreationDate | ConvertTo-Json -Compress }}"
        );
        let out = run_powershell(&script)?;
        parse_cim_stats(&out).ok_or_else(|| HostError::NotFound(format!("process {pid}")))
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = pid;
        Err(HostError::unsupported("process-stats on this platform"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn spec(command: &str, args: &[&str]) -> Spawn {
        Spawn {
            command: command.into(),
            args: args.iter().map(ToString::to_string).collect(),
            cwd: std::env::temp_dir(),
            env: vec![],
            clear_env: false,
            stdin: Input::Pipe,
            stdout: Output::Pipe,
            stderr: Output::Pipe,
            new_process_group: true,
            detached: false,
            timeout: None,
        }
    }

    #[cfg(unix)]
    #[test]
    fn bare_names_fall_back_to_the_executable_dir() {
        assert_eq!(resolve_program("/bin/sh"), PathBuf::from("/bin/sh"));
        assert_eq!(resolve_program("sh"), PathBuf::from("sh"));
        let exe = std::env::current_exe().unwrap();
        let name = exe.file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(resolve_program(&name), exe);
        assert_eq!(
            resolve_program("no-such-program-xyz"),
            PathBuf::from("no-such-program-xyz")
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn spawn_stream_wait() {
        let svc = ProcessService::default();
        let child = svc.spawn(&spec("cat", &[])).unwrap();
        child.write_stdin(b"hello").await.unwrap();
        child.close_stdin();
        let out: Vec<u8> = child.take_stdout().unwrap().concat().await;
        assert_eq!(out, b"hello");
        assert_eq!(child.wait().await.code, Some(0));
    }

    /// macOS `ps` rows (pure parsing, so the platform paths are covered on any host).
    #[test]
    fn ps_output_parsing() {
        let parents = parse_ps_parents("    1     0\n  421     1\n  555   421\n").unwrap();
        assert_eq!(parents, vec![(1, 0), (421, 1), (555, 421)]);
        let stats =
            parse_ps_stats(421, "     1 S+   123456  0:01.25 02:03:04\n", 10_000_000).unwrap();
        assert_eq!(stats.ppid, 1);
        assert_eq!(stats.state, "S+");
        assert_eq!(stats.rss_bytes, 123_456 * 1024);
        assert_eq!(stats.cpu_ms, 1_250);
        assert_eq!(stats.read_bytes, 0);
        assert_eq!(stats.write_bytes, 0);
        // Started `02:03:04` before `now`.
        assert_eq!(
            stats.start_time_ms,
            10_000_000 - (2 * 3_600_000 + 3 * 60_000 + 4_000)
        );
        assert!(parse_ps_stats(1, "", 0).is_none());
        assert_eq!(parse_clock_ms("03:04"), Some(184_000));
        assert_eq!(parse_clock_ms("1-00:00:01"), Some(86_401_000));
        assert_eq!(parse_clock_ms("0,50"), Some(500));
    }

    /// `Get-CimInstance Win32_Process` JSON (pure parsing).
    #[test]
    fn cim_output_parsing() {
        let row = r#"{"ProcessId":421,"ParentProcessId":1,"WorkingSetSize":52428800,
            "KernelModeTime":1500000,"UserModeTime":2500000,"CreationDate":"2023-10-27T12:30:45.5000000+02:00"}"#;
        let stats = parse_cim_stats(row).unwrap();
        assert_eq!((stats.pid, stats.ppid), (421, 1));
        assert_eq!(stats.rss_bytes, 52_428_800);
        assert_eq!(stats.cpu_ms, 400);
        assert_eq!(stats.start_time_ms, 1_698_402_645_500);
        assert!(parse_cim_stats("[]").is_none());
        let parents = parse_cim_parents(
            r#"[{"ProcessId":1,"ParentProcessId":0},{"ProcessId":421,"ParentProcessId":1}]"#,
        )
        .unwrap();
        assert_eq!(parents, vec![(1, 0), (421, 1)]);
        assert_eq!(parse_iso_ms("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso_ms("1970-01-02T00:00:00Z"), Some(86_400_000));
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_and_tree_kill() {
        let svc = ProcessService::default();
        let mut s = spec("sh", &["-c", "sleep 30 & sleep 30"]);
        s.timeout = Some(Duration::from_millis(200));
        let child = svc.spawn(&s).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let tree = process_tree(child.pid).unwrap();
        assert_ne!(tree.len(), 0);
        let exit = child.wait().await;
        assert!(exit.timed_out);
        tokio::time::sleep(Duration::from_millis(50)).await;
        for pid in tree {
            assert!(
                read_stat(pid).is_none_or(|s| s.state == "Z"),
                "{pid} survived"
            );
        }
        assert!(process_stats(std::process::id()).unwrap().rss_bytes > 0);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn exec_shape() {
        let out = exec(
            "sh",
            &["-c".into(), "echo hi; echo err >&2; exit 3".into()],
            &std::env::temp_dir(),
            &[],
            None,
        )
        .await
        .unwrap();
        assert_eq!(
            (
                out.stdout.as_str(),
                out.stderr.as_str(),
                out.code,
                out.killed
            ),
            ("hi\n", "err\n", Some(3), false)
        );
        let out = exec(
            "sleep",
            &["5".into()],
            &std::env::temp_dir(),
            &[],
            Some(Duration::from_millis(50)),
        )
        .await
        .unwrap();
        assert!(out.killed);
    }
}
