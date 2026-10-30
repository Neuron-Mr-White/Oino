//! Runs one job fire as `pi -p --mode json "<prompt>" --session-id <run id>`
//! (UNI-314, UNI-315).
//!
//! The host never attaches a tty: it captures stdout+stderr, keeps the
//! last 64 KB as the raw log, and — since C2 — also parses the `--mode
//! json` event stream on stdout as it arrives (see [`JsonStream`]) for:
//!
//! - **`session_path`**: pi's session header (`{"type":"session",
//!   "id":…,"timestamp":…}`) plus the job's `cwd` are enough to compute
//!   the exact session file pi is writing to, the same way pi itself
//!   does (`~/.pi/agent/sessions/<encoded-cwd>/<file-timestamp>_<id>
//!   .jsonl`, `PI_CODING_AGENT_DIR` overriding `~/.pi/agent`). We pass
//!   `--session-id <run id>` ourselves, so the id is always known in
//!   advance; the header's `timestamp` is the one piece we can't derive
//!   and must read off the stream. The path is verified to exist before
//!   being recorded — never a guess that might be wrong.
//! - **`cost_usd`**: the sum of every `cost.total` reported on the
//!   stream (`turn_end` message usage, standalone `usage` entries,
//!   `compaction_end` usage) — the session's total spend for this run.
//! - **summary**: the assistant's final turn text (`turn_end`'s
//!   message content), not a raw stdout tail — `--mode json` has no
//!   human-readable "final answer" line to grep for.
//!
//! A `time_limit_s` timeout kills the whole process group (so stray
//! children die too) and records `timeout`; a nonzero exit is `failed`,
//! zero is `ok`. A run whose accumulated cost exceeds the job's
//! `cost_cap_usd` is marked `failed` with reason `"cost cap"` — checked
//! after the process ends (no cheap way to abort mid-turn on a running
//! total without pi's own cost-cap flag, a possible future addition).

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use super::model::{Run, RunStatus};
use super::store;
use crate::data_dir::HostPaths;

/// The system prompt appended for every scheduled run.
pub const UNATTENDED: &str = "You are running unattended and once, on a schedule. Nobody can answer questions. Do the task, then end with a short result summary (2–5 lines).";

/// Incrementally parses a `--mode json` JSONL stream for the three
/// things a cron run cares about: the session header (for
/// `session_path`), the running cost total, and the latest turn's
/// final text (the summary). Fed raw bytes as they arrive; buffers a
/// partial trailing line across calls.
#[derive(Default)]
pub struct JsonStream {
    buf: Vec<u8>,
    pub session_id: Option<String>,
    pub session_timestamp: Option<String>,
    pub session_cwd: Option<String>,
    pub cost_usd: f64,
    pub summary: Option<String>,
}

impl JsonStream {
    pub fn feed(&mut self, chunk: &[u8]) {
        self.buf.extend_from_slice(chunk);
        while let Some(pos) = self.buf.iter().position(|&b| b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=pos).collect();
            let line = &line[..line.len().saturating_sub(1)]; // drop the \n
            if line.is_empty() {
                continue;
            }
            if let Ok(value) = serde_json::from_slice::<Value>(line) {
                self.handle(&value);
            }
        }
    }

    fn handle(&mut self, value: &Value) {
        match value.get("type").and_then(Value::as_str) {
            Some("session") => {
                self.session_id = value.get("id").and_then(Value::as_str).map(str::to_string);
                self.session_timestamp = value.get("timestamp").and_then(Value::as_str).map(str::to_string);
                self.session_cwd = value.get("cwd").and_then(Value::as_str).map(str::to_string);
            }
            Some("turn_end") => {
                self.add_cost(value.get("message").and_then(|m| m.get("usage")));
                if let Some(content) = value.get("message").and_then(|m| m.get("content")).and_then(Value::as_array) {
                    let text: String = content
                        .iter()
                        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|c| c.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.trim().is_empty() {
                        self.summary = Some(text.trim().to_string());
                    }
                }
            }
            Some("usage") => self.add_cost(value.get("usage")),
            Some("compaction_end") => self.add_cost(value.get("result").and_then(|r| r.get("usage"))),
            _ => {}
        }
    }

    fn add_cost(&mut self, usage: Option<&Value>) {
        if let Some(total) = usage.and_then(|u| u.get("cost")).and_then(|c| c.get("total")).and_then(Value::as_f64) {
            self.cost_usd += total;
        }
    }

    /// The exact session file pi is writing to, if the session header
    /// has arrived and that file actually exists — same encoding pi
    /// itself uses (`getDefaultSessionDirPath`/`sessionFile` in
    /// `session-manager.ts`): `<agent dir>/sessions/--<cwd with `/`,
    /// `\` and `:` turned into `-`>--/<timestamp with `:`/`.` turned
    /// into `-`>_<session id>.jsonl`.
    pub fn session_path(&self) -> Option<PathBuf> {
        let id = self.session_id.as_deref()?;
        let timestamp = self.session_timestamp.as_deref()?;
        let cwd = self.session_cwd.as_deref()?;
        let agent_dir = std::env::var("PI_CODING_AGENT_DIR")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
            .or_else(|| dirs_home().map(|h| h.join(".pi").join("agent")))?;
        let safe_cwd = {
            let stripped = cwd.trim_start_matches(['/', '\\']);
            let replaced: String = stripped.chars().map(|c| if c == '/' || c == '\\' || c == ':' { '-' } else { c }).collect();
            format!("--{replaced}--")
        };
        let file_timestamp: String = timestamp.chars().map(|c| if c == ':' || c == '.' { '-' } else { c }).collect();
        let path = agent_dir.join("sessions").join(safe_cwd).join(format!("{file_timestamp}_{id}.jsonl"));
        path.is_file().then_some(path)
    }
}

fn dirs_home() -> Option<PathBuf> {
    crate::fs::home_dir_opt()
}

/// Reads `PATH=` from `<host dir>/env` (UNI-239's `EnvironmentFile`), the
/// one place the host's merged PATH lives. Falls back to this process's
/// own `PATH` if the file is missing (manual/test runs).
pub fn host_path(paths: &HostPaths) -> String {
    let env_file = paths.root.join("env");
    if let Ok(text) = std::fs::read_to_string(&env_file) {
        for line in text.lines() {
            let line = line.trim();
            if let Some(value) = line.strip_prefix("PATH=") {
                return value.trim_matches('"').to_string();
            }
        }
    }
    std::env::var("PATH").unwrap_or_default()
}

/// What to spawn for `pi`: the program plus any fixed leading arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PiCommand {
    pub program: std::path::PathBuf,
    pub args: Vec<String>,
}

/// Resolves `pi` on `path_list`. `None` if it isn't anywhere on it.
#[cfg(unix)]
pub fn resolve_pi(path_list: &str) -> Option<std::path::PathBuf> {
    for dir in path_list.split(':').filter(|d| !d.is_empty()) {
        let candidate = Path::new(dir).join("pi");
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    None
}

/// The paths to search for `pi`, in order: `path_list`, then (Windows only)
/// npm's global prefix `%APPDATA%\npm` — a plain `npm i -g` there is not on
/// PATH in a logon session.
fn pi_search_dirs(path_list: &str, is_windows: bool, appdata: Option<&Path>) -> Vec<std::path::PathBuf> {
    // The real platform splits with the stdlib (`;` on Windows); a caller
    // simulating the other platform gets the matching separator.
    let mut dirs: Vec<std::path::PathBuf> = if is_windows == cfg!(windows) {
        std::env::split_paths(path_list).collect()
    } else {
        let sep = if is_windows { ';' } else { ':' };
        path_list.split(sep).map(std::path::PathBuf::from).collect()
    };
    dirs.retain(|dir| !dir.as_os_str().is_empty());
    if is_windows {
        if let Some(appdata) = appdata {
            let npm = appdata.join("npm");
            if !dirs.iter().any(|dir| dir == &npm) {
                dirs.push(npm);
            }
        }
    }
    dirs
}

/// Resolves what to spawn for pi, for either platform. Pure so the Windows
/// rules are testable on Linux.
///
/// Linux/macOS: the first `pi` executable on the (colon-separated) list.
///
/// Windows (a bare `pi` there is `pi.cmd`, which can't be spawned without a
/// shell):
/// - `pi.exe` in a directory -> spawn it directly.
/// - `pi.cmd`/`pi.ps1`/`pi` whose directory also has
///   `node_modules/@earendil-works/pi-coding-agent/dist/cli.js` (npm's
///   global shims) -> spawn `node.exe` (found on the same list, then
///   `%ProgramFiles%\nodejs`) with that `cli.js` as the first argument.
/// - Never spawn a `.cmd`/`.ps1` itself.
pub fn resolve_pi_command(
    path_list: &str,
    is_windows: bool,
    appdata: Option<&Path>,
    program_files: Option<&Path>,
) -> Option<PiCommand> {
    let dirs = pi_search_dirs(path_list, is_windows, appdata);
    if !is_windows {
        return dirs
            .iter()
            .map(|dir| dir.join("pi"))
            .find(|candidate| candidate.is_file())
            .map(|program| PiCommand { program, args: Vec::new() });
    }
    for dir in &dirs {
        let exe = dir.join("pi.exe");
        if exe.is_file() {
            return Some(PiCommand { program: exe, args: Vec::new() });
        }
        let cli = dir.join("node_modules")
            .join("@earendil-works")
            .join("pi-coding-agent")
            .join("dist")
            .join("cli.js");
        let has_shim = ["pi.cmd", "pi.ps1", "pi"]
            .iter()
            .any(|name| dir.join(name).is_file());
        if !has_shim || !cli.is_file() {
            continue;
        }
        let Some(node) = resolve_node_exe(&dirs, program_files) else {
            continue;
        };
        return Some(PiCommand {
            program: node,
            args: vec![cli.display().to_string()],
        });
    }
    None
}

/// `node.exe` on the same search list, else `%ProgramFiles%\nodejs\node.exe`.
fn resolve_node_exe(dirs: &[std::path::PathBuf], program_files: Option<&Path>) -> Option<std::path::PathBuf> {
    if let Some(found) = dirs
        .iter()
        .map(|dir| dir.join("node.exe"))
        .find(|candidate| candidate.is_file())
    {
        return Some(found);
    }
    let candidate = program_files?.join("nodejs").join("node.exe");
    candidate.is_file().then_some(candidate)
}

/// The real PATH the child gets: on Windows npm's global prefix is added so
/// a spawned `node`/`pi` and its own tools resolve.
fn child_path(path_list: &str, is_windows: bool, appdata: Option<&Path>) -> String {
    if !is_windows {
        return path_list.to_string();
    }
    let dirs = pi_search_dirs(path_list, true, appdata);
    dirs.iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join(";")
}

/// The exit outcome of a run, before it is written to a [`Run`].
pub struct Outcome {
    pub status: RunStatus,
    pub exit_code: Option<i32>,
    pub summary: Option<String>,
    pub reason: Option<String>,
    pub session_path: Option<String>,
    pub cost_usd: Option<f64>,
}

/// What to run for one job fire: the prompt, cwd and pi options.
pub struct RunSpec<'a> {
    pub prompt: &'a str,
    pub cwd: &'a str,
    pub model: Option<&'a str>,
    pub thinking: Option<&'a str>,
    pub time_limit_s: u64,
    /// `None` = no cap enforced.
    pub cost_cap_usd: Option<f64>,
}

/// Runs one fire of `job` (prompt + cwd + model/thinking + time limit),
/// capturing output into the run's log and returning the outcome.
/// `paths`/`job_id`/`run_id` are where the log is appended as it streams
/// in, so `cron runs`/`cron show` can tail a run while it's in progress.
pub async fn run(paths: &HostPaths, job_id: &str, run_id: &str, spec: &RunSpec<'_>, on_spawned: impl FnOnce(u32)) -> Outcome {
    let RunSpec { prompt, cwd, model, thinking, time_limit_s, cost_cap_usd } = *spec;
    let raw_path = host_path(paths);
    let is_windows = cfg!(windows);
    let appdata = std::env::var_os("APPDATA").filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let program_files = std::env::var_os("ProgramFiles").filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    let path_list = child_path(&raw_path, is_windows, appdata.as_deref());
    let Some(pi_cmd) = resolve_pi_command(&raw_path, is_windows, appdata.as_deref(), program_files.as_deref()) else {
        let message = format!("pi not found on PATH ({raw_path})");
        let _ = store::append_log(paths, job_id, run_id, message.as_bytes());
        return Outcome { status: RunStatus::Failed, exit_code: None, summary: None, reason: Some(message), session_path: None, cost_usd: None };
    };
    let pi_bin = pi_cmd.program.clone();

    let mut cmd = Command::new(&pi_bin);
    for arg in &pi_cmd.args {
        cmd.arg(arg);
    }
    cmd.arg("-p").arg(prompt);
    cmd.arg("--mode").arg("json");
    cmd.arg("--session-id").arg(run_id);
    cmd.arg("--append-system-prompt").arg(UNATTENDED);
    if let Some(model) = model {
        cmd.arg("--model").arg(model);
    }
    if let Some(thinking) = thinking {
        cmd.arg("--thinking").arg(thinking);
    }
    cmd.current_dir(cwd);
    cmd.env("PATH", &path_list);
    cmd.env("UNIPI_CRON_JOB", job_id);
    cmd.env("UNIPI_CRON_RUN", run_id);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let message = format!("couldn't start pi ({}): {e}", pi_bin.display());
            let _ = store::append_log(paths, job_id, run_id, message.as_bytes());
            return Outcome { status: RunStatus::Failed, exit_code: None, summary: None, reason: Some(message), session_path: None, cost_usd: None };
        }
    };

    if let Some(pid) = child.id() {
        on_spawned(pid);
    }
    let stdout = child.stdout.take().expect("piped");
    let stderr = child.stderr.take().expect("piped");
    let json_stream = std::sync::Arc::new(tokio::sync::Mutex::new(JsonStream::default()));

    let pump = |mut reader: tokio::process::ChildStdout, paths: HostPaths, job_id: String, run_id: String, stream: std::sync::Arc<tokio::sync::Mutex<JsonStream>>| async move {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = store::append_log(&paths, &job_id, &run_id, &buf[..n]);
                    stream.lock().await.feed(&buf[..n]);
                }
            }
        }
    };
    let pump_err = |mut reader: tokio::process::ChildStderr, paths: HostPaths, job_id: String, run_id: String| async move {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = store::append_log(&paths, &job_id, &run_id, &buf[..n]);
                }
            }
        }
    };
    let out_task = tokio::spawn(pump(stdout, paths.clone(), job_id.to_string(), run_id.to_string(), json_stream.clone()));
    let err_task = tokio::spawn(pump_err(stderr, paths.clone(), job_id.to_string(), run_id.to_string()));

    let limit = Duration::from_secs(time_limit_s.max(1));
    let wait = tokio::time::timeout(limit, child.wait()).await;
    // On a timeout the child (and anything it spawned) is still alive, so
    // its stdout/stderr pipes are still open: the pump tasks below would
    // never see EOF. Kill the process group *before* waiting for them.
    if wait.is_err() {
        kill_group(&child);
        let _ = child.wait().await;
    }
    let _ = out_task.await;
    let _ = err_task.await;

    let (summary, session_path, cost_usd) = {
        let stream = json_stream.lock().await;
        let summary = stream.summary.clone().filter(|s| !s.is_empty());
        let session_path = stream.session_path().map(|p| p.to_string_lossy().into_owned());
        let cost_usd = Some(stream.cost_usd);
        (summary, session_path, cost_usd)
    };

    // The cost cap is checked after the process ends: there is no cheap
    // way to abort pi mid-turn on a running total, only to refuse to
    // record the run as `ok` once it's known to have gone over.
    if let (Some(cap), Some(cost)) = (cost_cap_usd, cost_usd) {
        if cost > cap {
            let reason = format!("cost cap: ${cost:.4} > ${cap:.4}");
            return Outcome { status: RunStatus::Failed, exit_code: None, summary, reason: Some(reason), session_path, cost_usd };
        }
    }

    match wait {
        Ok(Ok(status)) => {
            let code = status.code();
            if status.success() {
                Outcome { status: RunStatus::Ok, exit_code: code, summary, reason: None, session_path, cost_usd }
            } else {
                Outcome { status: RunStatus::Failed, exit_code: code, summary, reason: Some(format!("pi exited with status {status}")), session_path, cost_usd }
            }
        }
        Ok(Err(e)) => Outcome { status: RunStatus::Failed, exit_code: None, summary, reason: Some(format!("couldn't wait for pi: {e}")), session_path, cost_usd },
        Err(_) => Outcome { status: RunStatus::Timeout, exit_code: None, summary, reason: Some(format!("exceeded the {time_limit_s}s time limit")), session_path, cost_usd },
    }
}

/// Kills the whole process group pi started (so any children it spawned
/// die too), not just the `pi` process itself.
#[cfg(unix)]
fn kill_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        let pgid = nix::unistd::Pid::from_raw(pid as i32);
        let _ = nix::sys::signal::killpg(pgid, nix::sys::signal::Signal::SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        kill_tree(pid);
    }
}

/// Windows: kill a process and everything it started (`pi` spawns tools).
/// `TerminateProcess` needs a process handle, not a pid, and only ends the
/// one process — `taskkill /T /F` does the whole tree.
#[cfg(not(unix))]
pub fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/T", "/F", "/PID", &pid.to_string()])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
}

/// Apply a `Run` record from an [`Outcome`] plus timing.
pub fn finish(run: &mut Run, outcome: Outcome, started_at: u64, ended_at: u64) {
    run.status = outcome.status;
    run.exit_code = outcome.exit_code;
    run.summary = outcome.summary;
    run.reason = outcome.reason;
    run.started_at = Some(started_at);
    run.ended_at = Some(ended_at);
    run.session_path = outcome.session_path;
    run.cost_usd = outcome.cost_usd;
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A windows-shaped temp tree: `dirs` are search dirs, files are created
    /// verbatim (so a `pi.cmd` shim and its `node_modules/.../cli.js` exist).
    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, b"x").unwrap();
    }

    #[test]
    fn resolves_pi_exe_on_windows_first() {
        let tmp = tempfile::tempdir().unwrap();
        let dir_a = tmp.path().join("a");
        let dir_b = tmp.path().join("b");
        let exe = dir_b.join("pi.exe");
        touch(&exe);
        let list = format!("{};{}", dir_a.display(), dir_b.display());
        assert_eq!(
            resolve_pi_command(&list, true, None, None),
            Some(PiCommand { program: exe, args: vec![] })
        );
    }

    #[test]
    fn windows_npm_shim_spawns_node_with_the_cli() {
        let tmp = tempfile::tempdir().unwrap();
        let npm_dir = tmp.path().join("npm");
        let cli = npm_dir
            .join("node_modules")
            .join("@earendil-works")
            .join("pi-coding-agent")
            .join("dist")
            .join("cli.js");
        touch(&cli);
        touch(&npm_dir.join("pi.cmd"));
        touch(&npm_dir.join("pi.ps1"));
        let node = npm_dir.join("node.exe");
        touch(&node);
        // %APPDATA%\npm is searched even when it is not on PATH.
        let appdata = tmp.path();
        let found = resolve_pi_command(r"C:\Windows\system32", true, Some(appdata), None).unwrap();
        assert_eq!(found.program, node);
        assert_eq!(found.args, vec![cli.display().to_string()]);
    }

    #[test]
    fn windows_shim_without_cli_is_skipped() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("npm");
        touch(&dir.join("pi.cmd"));
        touch(&dir.join("node.exe"));
        // A `pi.cmd` with no pi-coding-agent next to it is some other pi.
        assert_eq!(resolve_pi_command(&dir.display().to_string(), true, None, None), None);
    }

    #[test]
    fn windows_shim_falls_back_to_program_files_node() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("npm");
        let cli = dir
            .join("node_modules")
            .join("@earendil-works")
            .join("pi-coding-agent")
            .join("dist")
            .join("cli.js");
        touch(&cli);
        touch(&dir.join("pi.cmd"));
        let program_files = tmp.path().join("pf");
        let node = program_files.join("nodejs").join("node.exe");
        touch(&node);
        let found = resolve_pi_command(&dir.display().to_string(), true, None, Some(&program_files)).unwrap();
        assert_eq!(found.program, node);
        assert_eq!(found.args, vec![cli.display().to_string()]);
    }

    #[test]
    fn unix_resolution_is_unchanged() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("pi");
        touch(&exe);
        // A `pi.exe` or npm shim must not win on unix.
        touch(&tmp.path().join("pi.exe"));
        let list = format!("/does/not/exist:{}", tmp.path().display());
        assert_eq!(
            resolve_pi_command(&list, false, None, None),
            Some(PiCommand { program: exe, args: vec![] })
        );
    }

    #[test]
    #[cfg(unix)]
    fn resolve_pi_finds_the_first_match() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("pi");
        std::fs::write(&bin, "#!/bin/sh\necho hi\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let path_list = format!("/does/not/exist:{}", tmp.path().display());
        assert_eq!(resolve_pi(&path_list), Some(bin));
        assert_eq!(resolve_pi("/does/not/exist"), None);
    }

    #[test]
    fn host_path_reads_the_env_file() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(tmp.path());
        std::fs::write(tmp.path().join("env"), "PATH=/a:/b\n").unwrap();
        assert_eq!(host_path(&paths), "/a:/b");
    }

    #[test]
    fn json_stream_sums_cost_and_keeps_the_final_turn_text() {
        let mut s = JsonStream::default();
        s.feed(br#"{"type":"session","id":"abc","timestamp":"2024-01-01T00:00:00.000Z","cwd":"/tmp/job"}"#);
        s.feed(b"\n");
        s.feed(br#"{"type":"usage","usage":{"cost":{"total":0.01}}}"#);
        s.feed(b"\n");
        s.feed(br#"{"type":"turn_end","message":{"content":[{"type":"text","text":"first"}],"usage":{"cost":{"total":0.02}}}}"#);
        s.feed(b"\n");
        s.feed(br#"{"type":"turn_end","message":{"content":[{"type":"text","text":"final answer"}],"usage":{"cost":{"total":0.005}}}}"#);
        s.feed(b"\n");
        assert_eq!(s.summary.as_deref(), Some("final answer"));
        assert!((s.cost_usd - 0.035).abs() < 1e-9, "cost_usd: {}", s.cost_usd);
        assert_eq!(s.session_id.as_deref(), Some("abc"));
    }

    #[test]
    fn json_stream_handles_a_split_line_across_feeds() {
        let mut s = JsonStream::default();
        let line = br#"{"type":"turn_end","message":{"content":[{"type":"text","text":"hi"}],"usage":{"cost":{"total":0.1}}}}"#;
        s.feed(&line[..20]);
        s.feed(&line[20..]);
        s.feed(b"\n");
        assert_eq!(s.summary.as_deref(), Some("hi"));
        assert!((s.cost_usd - 0.1).abs() < 1e-9);
    }

    #[test]
    fn session_path_only_resolves_to_a_file_that_actually_exists() {
        let tmp = tempfile::tempdir().unwrap();
        let agent_dir = tmp.path().join("agent");
        let cwd = "/home/me/project";
        let session_dir = agent_dir.join("sessions").join("--home-me-project--");
        std::fs::create_dir_all(&session_dir).unwrap();
        let file = session_dir.join("2024-01-01T00-00-00-000Z_abc.jsonl");
        std::fs::write(&file, "{}").unwrap();

        std::env::set_var("PI_CODING_AGENT_DIR", &agent_dir);
        let mut s = JsonStream {
            session_id: Some("abc".to_string()),
            session_timestamp: Some("2024-01-01T00:00:00.000Z".to_string()),
            session_cwd: Some(cwd.to_string()),
            ..Default::default()
        };
        assert_eq!(s.session_path(), Some(file));

        // A session id/timestamp with no matching file on disk → None,
        // never a guessed path that might not exist.
        s.session_id = Some("missing".to_string());
        assert_eq!(s.session_path(), None);
        std::env::remove_var("PI_CODING_AGENT_DIR");
    }
}
