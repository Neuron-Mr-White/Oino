//! `process`: spawning, streaming, signals, process trees and stats (extension-api §5.4).

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
        #[cfg(not(unix))]
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
    #[cfg(not(unix))]
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

#[cfg(not(target_os = "linux"))]
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

/// Stats for `pid`.
pub(crate) fn process_stats(pid: u32) -> Result<Stats, HostError> {
    if cfg!(target_os = "linux") {
        read_stat(pid).ok_or_else(|| HostError::NotFound(format!("process {pid}")))
    } else {
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
