//! Runs one job fire as a headless Oino run:
//! `oino -p "<prompt>" --mode json --session-id <run id> --append-system-prompt …`.
//!
//! The run's session gets the run id as its header id and file name
//! (`<agent>/sessions/--<cwd>--/<ts>_<run id>.jsonl`, pi's layout and `--session-id`
//! creating the session like pi does), so the run record can point at the exact file that
//! can be browsed afterwards (`oino --session <path>`, the desktop's session list,
//! `/cron runs`); [`find_session_file`] only reports a file that exists.
//!
//! stdout+stderr are captured into the run's capped log; the JSON event stream on stdout
//! is parsed as it arrives ([`JsonStream`]) for:
//!
//! - **`cost_usd`** — the sum of every `turn_end` message usage `cost.total`.
//! - **`summary`** — the last assistant turn's text.
//!
//! The session file is verified to exist before it is recorded — never a guess. A
//! `time_limit_s` timeout kills the whole process group (so stray children die too) and
//! records `timeout`; a nonzero exit is `failed`, zero is `ok` — unless the JSON stream
//! says the agent turn itself ended in an error (`stopReason: "error"`, a provider 401
//! for example), which fails the run with exit 1 and the provider's words in the run file
//! (F-03). A run whose accumulated cost exceeds the job's `cost_cap_usd` is recorded
//! `failed` with reason `"cost cap"`.
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde_json::Value;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

use crate::model::RunStatus;
use crate::paths::CronPaths;
use crate::store;

/// The system prompt appended for every scheduled run.
pub const UNATTENDED: &str = "You are running unattended and once, on a schedule. Nobody can answer questions. Do the task, then end with a short result summary (2–5 lines).";

/// Environment override pointing at the `oino` binary to run (tests, dev installs).
pub const OINO_BIN_VAR: &str = "OINO_CRON_BIN";

/// What to spawn for `oino`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OinoCommand {
    /// The program.
    pub program: PathBuf,
    /// Fixed leading arguments.
    pub args: Vec<String>,
}

/// Resolves the `oino` binary to spawn: `$OINO_CRON_BIN`, then this process's own
/// executable **when it is `oino` itself**, then an `oino` sitting **beside** this
/// executable (the bundle's `usr/bin/oino` next to `usr/bin/unipi-oino-host`), then `oino`
/// on `PATH`. `None` when nothing found. (The owner is no longer the `oino` binary —
/// under v2 it is unipi-host — so a foreign current executable must not be *spawned*, but
/// its directory is exactly where the bundled `oino` lives.)
#[must_use]
pub fn resolve_oino(
    env_bin: Option<std::ffi::OsString>,
    path_env: Option<&std::ffi::OsStr>,
    exe: Option<&Path>,
) -> Option<OinoCommand> {
    if let Some(from_env) = env_bin.filter(|v| !v.is_empty()) {
        return Some(OinoCommand {
            program: PathBuf::from(from_env),
            args: Vec::new(),
        });
    }
    let oino_name = if cfg!(windows) { "oino.exe" } else { "oino" };
    if let Some(exe) = exe
        && exe.is_file()
    {
        if exe.file_name().is_some_and(|n| n == oino_name) {
            return Some(OinoCommand {
                program: exe.to_path_buf(),
                args: Vec::new(),
            });
        }
        // v2 QA D-41: the host ships with `oino` beside it — use that bundled binary
        // before whatever a shell `PATH` would surface (a dev install, another version).
        if let Some(dir) = exe.parent() {
            let sibling = dir.join(oino_name);
            if sibling.is_file() {
                return Some(OinoCommand {
                    program: sibling,
                    args: Vec::new(),
                });
            }
        }
    }
    let path_env = path_env?;
    for dir in std::env::split_paths(&path_env) {
        if dir.as_os_str().is_empty() {
            continue;
        }
        let candidate = dir.join(if cfg!(windows) { "oino.exe" } else { "oino" });
        if candidate.is_file() {
            return Some(OinoCommand {
                program: candidate,
                args: Vec::new(),
            });
        }
    }
    None
}

/// Incrementally parses a `--mode json` event stream for the run cost, the latest turn's
/// final text, and a failed turn (F-03): an assistant message whose `stopReason` is
/// `error` (a provider 401, rate limit, …) fails the run even when the process exits 0.
#[derive(Default)]
pub struct JsonStream {
    buf: Vec<u8>,
    /// Running cost total (USD).
    pub cost_usd: f64,
    /// The latest assistant turn's text.
    pub summary: Option<String>,
    /// The last turn's error (`stopReason: "error"`), if any.
    pub error: Option<String>,
}

impl JsonStream {
    /// Feeds one stdout chunk.
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
            Some("turn_end") => {
                let message = value.get("message");
                self.add_cost(message);
                self.add_turn_error(message);
                if let Some(content) = message
                    .and_then(|m| m.get("content"))
                    .and_then(Value::as_array)
                {
                    let text: String = content
                        .iter()
                        .filter(|c| c.get("type").and_then(Value::as_str) == Some("text"))
                        .filter_map(|c| c.get("text").and_then(Value::as_str))
                        .collect::<Vec<_>>()
                        .join("\n");
                    if !text.trim().is_empty() {
                        self.summary = Some(text.trim().to_owned());
                    }
                }
            }
            // The settle summary re-lists every turn message; the last assistant entry
            // is the truth about how the run ended (F-03).
            Some("agent_end") => {
                if let Some(message) =
                    value
                        .get("messages")
                        .and_then(Value::as_array)
                        .and_then(|m| {
                            m.iter().rev().find(|m| {
                                m.get("role").and_then(Value::as_str) == Some("assistant")
                            })
                        })
                {
                    self.add_turn_error(Some(message));
                }
            }
            _ => {}
        }
    }

    /// Records an assistant message's verdict (`stopReason`): an `error` turn keeps the
    /// provider's words, any other ended turn clears an earlier failure — the run's
    /// verdict is its LAST turn, so a retry that succeeds succeeds (F-03).
    fn add_turn_error(&mut self, message: Option<&Value>) {
        let stop = message
            .and_then(|m| m.get("stopReason"))
            .and_then(Value::as_str);
        if stop == Some("error") {
            self.error = Some(
                message
                    .and_then(|m| m.get("errorMessage"))
                    .and_then(Value::as_str)
                    .filter(|e| !e.is_empty())
                    .unwrap_or("the agent turn ended in an error")
                    .to_owned(),
            );
        } else if stop.is_some() {
            self.error = None;
        }
    }

    fn add_cost(&mut self, message: Option<&Value>) {
        if let Some(total) = message
            .and_then(|m| m.get("usage"))
            .and_then(|u| u.get("cost"))
            .and_then(|c| c.get("total"))
            .and_then(Value::as_f64)
        {
            self.cost_usd += total;
        }
    }
}

/// Finds the session file a run wrote: `<agent>/sessions/<encoded cwd>/*_<run id>.jsonl`
/// (falls back to scanning every session dir), only reporting a file that exists.
#[must_use]
pub fn find_session_file(agent_dir: &Path, cwd: &str, run_id: &str) -> Option<PathBuf> {
    let sessions = agent_dir.join("sessions");
    let encoded = store::encode_cwd(Path::new(cwd));
    if let Some(found) = glob_run_file(&sessions.join(encoded), run_id) {
        return Some(found);
    }
    let entries = std::fs::read_dir(&sessions).ok()?;
    for entry in entries.filter_map(Result::ok) {
        if entry.file_type().is_ok_and(|t| t.is_dir())
            && let Some(found) = glob_run_file(&entry.path(), run_id)
        {
            return Some(found);
        }
    }
    None
}

fn glob_run_file(dir: &Path, run_id: &str) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .find(|path| {
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("jsonl"))
                && path.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                    n.to_ascii_lowercase()
                        .ends_with(&format!("_{run_id}.jsonl"))
                })
        })
}

/// The exit outcome of a run, before it is written to a [`crate::model::Run`].
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    /// Final status.
    pub status: RunStatus,
    /// Exit code, when there was one.
    pub exit_code: Option<i32>,
    /// Assistant summary text.
    pub summary: Option<String>,
    /// Why the run failed or was skipped.
    pub reason: Option<String>,
    /// The run's session file.
    pub session_path: Option<String>,
    /// Total cost in USD.
    pub cost_usd: Option<f64>,
}

/// What to run for one job fire.
#[derive(Debug, Clone, PartialEq)]
pub struct RunSpec {
    /// The prompt.
    pub prompt: String,
    /// Working directory (must exist).
    pub cwd: String,
    /// Model override.
    pub model: Option<String>,
    /// Thinking level override.
    pub thinking: Option<String>,
    /// Wall-clock limit in seconds.
    pub time_limit_s: u64,
    /// Cost cap (`None` = none).
    pub cost_cap_usd: Option<f64>,
}

/// Runs one fire of a job, capturing output into the run's log as it streams in, and
/// returns the outcome. `on_spawned` receives the child's pid.
///
/// # Errors
/// Never returns `Err` for a run-level failure — those are [`Outcome`]s; only log-write
/// failures bubble up as [`CronError`](crate::CronError).
#[allow(
    clippy::too_many_lines,
    reason = "one linear pipeline: spawn, pump both pipes, enforce the limit, harvest"
)]
pub async fn run(
    paths: &CronPaths,
    job_id: &str,
    run_id: &str,
    spec: &RunSpec,
    on_spawned: impl FnOnce(u32) + Send + 'static,
) -> Outcome {
    let Some(oino) = resolve_oino(
        std::env::var_os(OINO_BIN_VAR),
        std::env::var_os("PATH").as_deref(),
        std::env::current_exe().ok().as_deref(),
    ) else {
        let message = "couldn't find the oino binary to run the job (install it beside the host, set OINO_CRON_BIN, or add oino to PATH)".to_owned();
        let _ = store::append_log(paths, job_id, run_id, message.as_bytes());
        return Outcome {
            status: RunStatus::Failed,
            exit_code: None,
            summary: None,
            reason: Some(message),
            session_path: None,
            cost_usd: None,
        };
    };
    // The session directory the child writes to (it creates the file itself).
    let _ = std::fs::create_dir_all(paths.agent_dir.join("sessions"));
    let mut cmd = Command::new(&oino.program);
    for arg in &oino.args {
        cmd.arg(arg);
    }
    cmd.arg("-p").arg(&spec.prompt);
    cmd.arg("--mode").arg("json");
    cmd.arg("--session-id").arg(run_id);
    cmd.arg("--append-system-prompt").arg(UNATTENDED);
    if let Some(model) = &spec.model {
        cmd.arg("--model").arg(model);
    }
    if let Some(thinking) = &spec.thinking {
        cmd.arg("--thinking").arg(thinking);
    }
    cmd.current_dir(&spec.cwd);
    cmd.env(crate::paths::AGENT_DIR_VAR, &paths.agent_dir);
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => {
            let message = format!("couldn't start oino ({}): {e}", oino.program.display());
            let _ = store::append_log(paths, job_id, run_id, message.as_bytes());
            return Outcome {
                status: RunStatus::Failed,
                exit_code: None,
                summary: None,
                reason: Some(message),
                session_path: None,
                cost_usd: None,
            };
        }
    };
    if let Some(pid) = child.id() {
        on_spawned(pid);
    }
    let Some(stdout) = child.stdout.take() else {
        return Outcome {
            status: RunStatus::Failed,
            exit_code: None,
            summary: None,
            reason: Some("oino's stdout disappeared".to_owned()),
            session_path: None,
            cost_usd: None,
        };
    };
    let Some(stderr) = child.stderr.take() else {
        return Outcome {
            status: RunStatus::Failed,
            exit_code: None,
            summary: None,
            reason: Some("oino's stderr disappeared".to_owned()),
            session_path: None,
            cost_usd: None,
        };
    };
    let stream = std::sync::Arc::new(tokio::sync::Mutex::new(JsonStream::default()));

    let pump = |mut reader: tokio::process::ChildStdout,
                paths: CronPaths,
                job_id: String,
                run_id: String,
                stream: std::sync::Arc<tokio::sync::Mutex<JsonStream>>| async move {
        let mut buf = [0u8; 8192];
        loop {
            match reader.read(&mut buf).await {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = store::append_log(&paths, &job_id, &run_id, &buf[..n]);
                    if let Ok(mut s) = stream.try_lock() {
                        s.feed(&buf[..n]);
                    }
                }
            }
        }
    };
    let pump_err = |mut reader: tokio::process::ChildStderr,
                    paths: CronPaths,
                    job_id: String,
                    run_id: String| async move {
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
    let out_task = tokio::spawn(pump(
        stdout,
        paths.clone(),
        job_id.to_owned(),
        run_id.to_owned(),
        stream.clone(),
    ));
    let err_task = tokio::spawn(pump_err(
        stderr,
        paths.clone(),
        job_id.to_owned(),
        run_id.to_owned(),
    ));

    let limit = Duration::from_secs(spec.time_limit_s.max(1));
    let wait = tokio::time::timeout(limit, child.wait()).await;
    // On a timeout the child (and anything it spawned) is still alive, so its pipes are
    // still open: the pump tasks would never see EOF. Kill the process group *before*
    // waiting for them.
    if wait.is_err() {
        kill_group(&child);
        let _ = child.wait().await;
    }
    let _ = out_task.await;
    let _ = err_task.await;

    let (summary, cost_usd) = {
        let s = stream.lock().await;
        (s.summary.clone().filter(|s| !s.is_empty()), s.cost_usd)
    };
    // A failed agent turn (provider 401, rate limit, …) fails the run even when the
    // process exited 0 (F-03): exit 1, and the provider's words land in the run file.
    let turn_error = stream.lock().await.error.clone();
    let session_path = find_session_file(&paths.agent_dir, &spec.cwd, run_id)
        .map(|p| p.to_string_lossy().into_owned());

    // The cost cap is checked after the process ends: refuse to record the run as `ok`
    // once it is known to have gone over.
    if let (Some(cap), true) = (spec.cost_cap_usd, cost_usd > 0.0)
        && cost_usd > cap
    {
        return Outcome {
            status: RunStatus::Failed,
            exit_code: None,
            summary,
            reason: Some(format!("cost cap: ${cost_usd:.4} > ${cap:.4}")),
            session_path,
            cost_usd: Some(cost_usd),
        };
    }

    match wait {
        Ok(Ok(status)) if !status.success() => {
            // G-07: a nonzero exit names its cause — the JSON turn error, else the run
            // log's last error-looking line (the provider's 401 body lands there).
            let log_hint = log_error_hint(paths, job_id, run_id);
            exit_outcome(
                false,
                status.code(),
                turn_error,
                summary,
                cost_usd,
                session_path,
                &status.to_string(),
                log_hint,
            )
        }
        Ok(Ok(status)) => exit_outcome(
            true,
            status.code(),
            turn_error,
            summary,
            cost_usd,
            session_path,
            &status.to_string(),
            None,
        ),
        Ok(Err(e)) => Outcome {
            status: RunStatus::Failed,
            exit_code: None,
            summary,
            reason: Some(format!("couldn't wait for oino: {e}")),
            session_path,
            cost_usd: Some(cost_usd),
        },
        Err(_) => Outcome {
            status: RunStatus::Timeout,
            exit_code: None,
            summary,
            reason: Some(format!("exceeded the {}s time limit", spec.time_limit_s)),
            session_path,
            cost_usd: Some(cost_usd),
        },
    }
}

/// The wait outcome for a child that was waited on: a nonzero exit is `failed`, zero is
/// `ok` — unless the JSON stream says the agent turn itself ended in an error
/// (`stopReason: "error"`, a provider 401 for example), which fails the run with exit 1
/// and the provider's words in the reason (F-03). A nonzero exit without a stream error
/// names the log's last error line as the cause (G-07).
#[allow(clippy::too_many_arguments, reason = "one linear outcome shape")]
fn exit_outcome(
    success: bool,
    code: Option<i32>,
    turn_error: Option<String>,
    summary: Option<String>,
    cost_usd: f64,
    session_path: Option<String>,
    status_text: &str,
    log_hint: Option<String>,
) -> Outcome {
    if success {
        if let Some(error) = turn_error {
            return Outcome {
                status: RunStatus::Failed,
                exit_code: Some(1),
                summary,
                reason: Some(format!("agent turn failed: {error}")),
                session_path,
                cost_usd: Some(cost_usd),
            };
        }
        Outcome {
            status: RunStatus::Ok,
            exit_code: code,
            summary,
            reason: None,
            session_path,
            cost_usd: Some(cost_usd),
        }
    } else {
        let cause = turn_error
            .map(|error| format!("agent turn failed: {error}"))
            .or(log_hint);
        Outcome {
            status: RunStatus::Failed,
            exit_code: code,
            summary,
            reason: Some(match cause {
                Some(cause) => format!("{cause} (oino exited with status {status_text})"),
                None => format!("oino exited with status {status_text}"),
            }),
            session_path,
            cost_usd: Some(cost_usd),
        }
    }
}

/// The run log's last error-looking line, flattened and clipped (G-07): a run that died
/// with a provider 401 keeps that line in its log even when no JSON turn error arrived.
#[must_use]
pub fn log_error_hint(paths: &CronPaths, job_id: &str, run_id: &str) -> Option<String> {
    use crate::store;
    let log = store::read_log(paths, job_id, run_id)?;
    let text = String::from_utf8_lossy(&log);
    let codes = [
        "400", "401", "403", "404", "429", "500", "502", "503", "504",
    ];
    text.lines()
        .rev()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .find(|line| {
            let lower = line.to_lowercase();
            lower.contains("error")
                || lower.contains("unauthorized")
                || lower.contains("forbidden")
                || codes.iter().any(|code| {
                    lower
                        .split(|c: char| !c.is_ascii_digit())
                        .any(|word| word == *code)
                })
        })
        .map(|line| {
            let flat = line.split_whitespace().collect::<Vec<_>>().join(" ");
            if flat.chars().count() <= 300 {
                flat
            } else {
                format!("{}…", flat.chars().take(299).collect::<String>())
            }
        })
}

/// Kills a run's process group (UNIX) or process tree (Windows).
pub fn kill_pid(pid: u32) {
    #[cfg(unix)]
    {
        let group = nix::unistd::Pid::from_raw(pid.cast_signed());
        let _ = nix::sys::signal::killpg(group, nix::sys::signal::Signal::SIGKILL);
    }
    #[cfg(not(unix))]
    {
        let _ = std::process::Command::new("taskkill")
            .args(["/T", "/F", "/PID", &pid.to_string()])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

#[cfg(unix)]
fn kill_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        kill_pid(pid);
    }
}

#[cfg(not(unix))]
fn kill_group(child: &tokio::process::Child) {
    if let Some(pid) = child.id() {
        kill_pid(pid);
    }
}

/// Applies an [`Outcome`] plus timing to a [`crate::model::Run`] record.
pub fn finish_run(run: &mut crate::model::Run, outcome: Outcome, started_at: u64, ended_at: u64) {
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
    use chrono::{TimeZone, Utc};

    #[test]
    fn resolves_env_override_first() {
        let bin = PathBuf::from("/opt/my-oino");
        let found = resolve_oino(Some(bin.clone().into_os_string()), None, None).unwrap();
        assert_eq!(found.program, bin);
        // An empty override does not count.
        assert!(resolve_oino(Some(String::new().into()), None, None).is_none());
    }

    #[test]
    fn resolves_current_exe_then_path() {
        let tmp = tempfile::tempdir().unwrap();
        let exe = tmp.path().join("oino");
        std::fs::write(&exe, b"#! /bin/sh\n").unwrap();
        let found = resolve_oino(None, None, Some(&exe)).unwrap();
        assert_eq!(found.program, exe);
        // A foreign current executable (the v2 owner is unipi-host) must not be spawned
        // itself, and a directory without `oino` beside it resolves nothing …
        let foreign_dir = tempfile::tempdir().unwrap();
        let foreign = foreign_dir.path().join("unipi-host");
        std::fs::write(&foreign, b"#! /bin/sh\n").unwrap();
        assert!(resolve_oino(None, None, Some(&foreign)).is_none());
        // … but a bundled `oino` beside it wins (D-41), even over one on PATH.
        let sibling = foreign_dir.path().join(oino_sibling_name());
        std::fs::write(&sibling, b"#! /bin/sh\n").unwrap();
        let elsewhere = tempfile::tempdir().unwrap();
        let on_path = elsewhere.path().join(oino_sibling_name());
        std::fs::write(&on_path, b"#! /bin/sh\n").unwrap();
        let path = std::env::join_paths([&elsewhere.path()]).unwrap();
        let found = resolve_oino(None, Some(path.as_os_str()), Some(&foreign)).unwrap();
        assert_eq!(found.program, sibling);
        // No exe → PATH search.
        let found = resolve_oino(None, Some(path.as_os_str()), None).unwrap();
        assert_eq!(found.program, on_path);
        assert!(resolve_oino(None, None, None).is_none());
    }

    /// G-07: a failed run records the actual cause (the provider's words), not only the
    /// exit status.
    #[test]
    fn a_failed_run_records_the_cause_not_just_the_status() {
        // The JSON turn error wins when the stream carried one.
        let outcome = exit_outcome(
            false,
            Some(1),
            Some("401 {\"type\":\"error\"}".into()),
            None,
            0.0,
            None,
            "exit status: 1",
            None,
        );
        assert_eq!(outcome.status, RunStatus::Failed);
        assert_eq!(
            outcome.reason.as_deref(),
            Some(
                "agent turn failed: 401 {\"type\":\"error\"} (oino exited with status exit status: 1)"
            )
        );
        // Else the run log's last error line is the cause.
        let outcome = exit_outcome(
            false,
            Some(1),
            None,
            None,
            0.0,
            None,
            "exit status: 1",
            Some("401 {\"message\":\"User not found.\",\"code\":401}".into()),
        );
        assert_eq!(
            outcome.reason.as_deref(),
            Some(
                "401 {\"message\":\"User not found.\",\"code\":401} (oino exited with status exit status: 1)"
            )
        );
        // Without either, the old wording stands.
        let outcome = exit_outcome(
            false,
            Some(1),
            None,
            None,
            0.0,
            None,
            "exit status: 1",
            None,
        );
        assert_eq!(
            outcome.reason.as_deref(),
            Some("oino exited with status exit status: 1")
        );
    }

    /// The log hint is the LAST error-looking line of the run log, clipped.
    #[test]
    fn the_log_hint_is_the_last_error_line() {
        let tmp = tempfile::tempdir().unwrap();
        let paths = CronPaths::new(tmp.path());
        let _ = store::append_log(
            &paths,
            "j1",
            "r1",
            b"{\"type\":\"system\"}\nsha256 4014d1 is a hash, not a status\n",
        );
        let _ = store::append_log(
            &paths,
            "j1",
            "r1",
            b"Error: 401 {\"message\":\"User not found.\",\"code\":401}\n",
        );
        let hint = log_error_hint(&paths, "j1", "r1").unwrap();
        assert!(
            hint.contains("User not found"),
            "the provider's line wins: {hint}"
        );
        // A long line is clipped to a readable hint.
        let long = format!("Error: 401 and {}\n", "x".repeat(600));
        let _ = store::append_log(&paths, "j1", "r1", long.as_bytes());
        let hint = log_error_hint(&paths, "j1", "r1").unwrap();
        assert!(hint.chars().count() <= 300, "{}", hint.chars().count());
        // No log, no hint.
        assert_eq!(log_error_hint(&paths, "j1", "r-missing"), None);
    }

    /// The `oino` file name `resolve_oino` looks for on this platform.
    fn oino_sibling_name() -> std::ffi::OsString {
        if cfg!(windows) {
            "oino.exe".into()
        } else {
            "oino".into()
        }
    }

    #[test]
    fn json_stream_sums_cost_and_keeps_the_final_turn_text() {
        let mut s = JsonStream::default();
        s.feed(br#"{"type":"turn_start"}"#);
        s.feed(b"\n");
        s.feed(br#"{"type":"turn_end","message":{"content":[{"type":"text","text":"first"}],"usage":{"cost":{"total":0.01}}}}"#);
        s.feed(b"\n");
        s.feed(br#"{"type":"turn_end","message":{"content":[{"type":"text","text":"final answer"}],"usage":{"cost":{"total":0.005}}}}"#);
        s.feed(b"\n");
        assert_eq!(s.summary.as_deref(), Some("final answer"));
        assert!(
            (s.cost_usd - 0.015).abs() < 1e-9,
            "cost_usd: {}",
            s.cost_usd
        );
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
    fn session_file_is_found_by_run_id_and_must_exist() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path();
        let cwd_dir = agent.join("sessions").join("--home-me-project--");
        std::fs::create_dir_all(&cwd_dir).unwrap();
        let file = cwd_dir.join("2024-01-01T00-00-00-000Z_r1.jsonl");
        std::fs::write(&file, b"{}").unwrap();
        let found = find_session_file(agent, "/home/me/project", "r1");
        assert_eq!(found.as_ref(), Some(&file));
        // Unknown id → None, never a guess.
        assert_eq!(find_session_file(agent, "/home/me/project", "nope"), None);
        // Wrong cwd → found via the fallback scan.
        assert_eq!(
            find_session_file(agent, "/elsewhere", "r1").as_ref(),
            Some(&file)
        );
        // A different cwd with no match anywhere → None.
        std::fs::remove_file(&file).unwrap();
        assert_eq!(find_session_file(agent, "/home/me/project", "r1"), None);
    }

    /// The run id is the session id, so the file pi's layout names after it is found.
    #[test]
    fn the_run_id_names_the_session_file() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("sessions/--home-me-project--");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("2026-10-28T03-00-07-623Z_cabc123.jsonl");
        std::fs::write(&path, b"{}").unwrap();
        assert_eq!(
            find_session_file(tmp.path(), "/home/me/project", "cabc123").as_ref(),
            Some(&path)
        );
    }

    /// A fake provider 401 (F-03): a stream whose turn ended in `stopReason: "error"`
    /// fails a zero-exit run — failed, exit 1, the provider's words in the reason.
    #[test]
    fn an_error_turn_fails_a_zero_exit_run() {
        let mut stream = JsonStream::default();
        stream.feed(
            br#"{"type":"turn_end","message":{"role":"assistant","stopReason":"error","errorMessage":"401 {\"type\":\"error\"}","usage":{"cost":{"total":0.0}}}}"#,
        );
        stream.feed(b"\n");
        stream.feed(br#"{"type":"agent_settled","aborted":false}"#);
        stream.feed(b"\n");
        assert_eq!(
            stream.error.as_deref(),
            Some("401 {\"type\":\"error\"}"),
            "the provider's words are kept"
        );
        let outcome = exit_outcome(
            true,
            Some(0),
            stream.error.clone(),
            stream.summary.clone(),
            stream.cost_usd,
            Some("/s.jsonl".into()),
            "exit status: 0",
            None,
        );
        assert_eq!(outcome.status, RunStatus::Failed);
        assert_eq!(outcome.exit_code, Some(1));
        assert!(
            outcome
                .reason
                .as_deref()
                .unwrap()
                .starts_with("agent turn failed: 401"),
            "reason: {:?}",
            outcome.reason
        );
    }

    /// A healthy stream (turn ends in `stop`) keeps a zero-exit run `ok`.
    #[test]
    fn a_clean_turn_keeps_a_zero_exit_run_ok() {
        let mut stream = JsonStream::default();
        stream.feed(
            br#"{"type":"turn_end","message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"all done"}],"usage":{"cost":{"total":0.01}}}}"#,
        );
        stream.feed(b"\n");
        assert_eq!(stream.error, None);
        assert_eq!(stream.summary.as_deref(), Some("all done"));
        let outcome = exit_outcome(
            true,
            Some(0),
            stream.error.clone(),
            stream.summary.clone(),
            stream.cost_usd,
            None,
            "exit status: 0",
            None,
        );
        assert_eq!(outcome.status, RunStatus::Ok);
        assert_eq!(outcome.exit_code, Some(0));
        assert_eq!(outcome.reason, None);
    }

    /// The settle summary's last assistant message counts too: an `agent_end` whose last
    /// turn errored fails the run even without a `turn_end` carrying it.
    #[test]
    fn agent_end_error_is_caught() {
        let mut stream = JsonStream::default();
        stream.feed(
            br#"{"type":"agent_end","messages":[{"role":"user","content":[]},{"role":"assistant","stopReason":"error","errorMessage":"401 user not found"}],"willRetry":false}"#,
        );
        stream.feed(b"\n");
        assert_eq!(stream.error.as_deref(), Some("401 user not found"));
    }

    /// The verdict is the run's LAST turn: a turn that fails then a turn that succeeds
    /// (auto-retry) leaves no error; the reverse ends failed.
    #[test]
    fn the_last_turn_decides() {
        let mut stream = JsonStream::default();
        let failed = br#"{"type":"turn_end","message":{"role":"assistant","stopReason":"error","errorMessage":"boom"}}"#;
        let ok = br#"{"type":"turn_end","message":{"role":"assistant","stopReason":"stop","content":[{"type":"text","text":"recovered"}]}}"#;
        stream.feed(&failed[..failed.len() - 2]);
        stream.feed(b"}\n");
        stream.feed(&ok[..]);
        stream.feed(b"\n");
        assert_eq!(stream.error, None, "a recovered run is ok");
        assert_eq!(stream.summary.as_deref(), Some("recovered"));
        // The reverse order stays failed.
        let mut stream = JsonStream::default();
        stream.feed(&ok[..]);
        stream.feed(b"\n");
        stream.feed(&failed[..]);
        stream.feed(b"\n");
        assert_eq!(stream.error.as_deref(), Some("boom"));
    }

    #[test]
    fn finish_run_applies_the_outcome() {
        let mut run = crate::model::Run {
            id: "r".into(),
            job_id: "j".into(),
            status: RunStatus::Running,
            fired_at: 0,
            started_at: None,
            ended_at: None,
            exit_code: None,
            summary: None,
            reason: None,
            session_path: None,
            cost_usd: None,
            missed: false,
            pid: None,
            notify_errors: Vec::new(),
            wake: None,
        };
        let outcome = Outcome {
            status: RunStatus::Ok,
            exit_code: Some(0),
            summary: Some("done".into()),
            reason: None,
            session_path: Some("/s.jsonl".into()),
            cost_usd: Some(0.5),
        };
        finish_run(&mut run, outcome, 10, 20);
        assert_eq!(run.status, RunStatus::Ok);
        assert_eq!(run.ended_at, Some(20));
        assert_eq!(run.session_path.as_deref(), Some("/s.jsonl"));
        let _ = Utc.timestamp_opt(0, 0).unwrap();
    }
}
