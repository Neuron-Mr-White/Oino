//! In-process board access (round 5, #7): one entry point shared by the WIT
//! `kanboard.exec` host import (every extension host that embeds the wasm
//! runtime) and the desktop's board service. There is no daemon, HTTP port or
//! SSE stream any more — a caller sends the same argv/environment it used to
//! give the `oino-kanboard` binary and gets the same `--json` payload back,
//! computed under the library's file locks.
//!
//! Request shape (`exec` takes the same JSON over the WIT boundary):
//!
//! ```json
//! {
//!   "argv":  ["list", "--all"],
//!   "env":   {"OINO_KANBOARD_PROJECT": "demo-a1b2c3", "OINO_KANBOARD_SESSION": "…"},
//!   "cwd":   "/the/project/dir",
//!   "stdin": "text for a `-` argument (optional)"
//! }
//! ```
//!
//! Errors mirror the binary's `--json` error line: `{"ok": false, "error":
//! "…", "kind": "usage|rule|not_found|io"}`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use clap::Parser;
use serde_json::{Value, json};

use crate::cli::Cli;
use crate::error::Error;
use crate::run;
use crate::store::Layout;

/// One `exec` request: argv of the command (without the program name), the
/// `OINO_KANBOARD_*` environment the caller would have exported, the working
/// directory project resolution happens in, and optional stdin text.
#[derive(Debug, Clone, Default)]
pub struct ExecRequest {
    /// Subcommand argv (`["list", "--all"]`); global flags like `--actor` may
    /// be given here or through `env`.
    pub argv: Vec<String>,
    /// Environment on top of the host's (`OINO_KANBOARD_*` variables).
    pub env: BTreeMap<String, String>,
    /// Working directory (defaults to the host's cwd).
    pub cwd: Option<PathBuf>,
    /// Text a `-` argument reads.
    pub stdin: Option<String>,
}

/// Parses the JSON form of an [`ExecRequest`].
pub fn parse_request(value: &Value) -> Result<ExecRequest, Error> {
    let invalid = |m: &str| Error::usage(format!("kanboard exec request: {m}"));
    let argv: Vec<String> = match value.get("argv") {
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| invalid("argv entries must be strings"))
            })
            .collect::<Result<Vec<_>, _>>()?,
        Some(_) => return Err(invalid("argv must be an array of strings")),
        None => return Err(invalid("missing argv")),
    };
    let env = match value.get("env") {
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| {
                v.as_str()
                    .map(|s| (k.clone(), s.to_owned()))
                    .ok_or_else(|| invalid("env values must be strings"))
            })
            .collect::<Result<BTreeMap<_, _>, _>>()?,
        Some(_) => return Err(invalid("env must be an object of strings")),
        None => BTreeMap::new(),
    };
    let cwd = value
        .get("cwd")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from);
    let stdin = value.get("stdin").and_then(Value::as_str).map(str::to_owned);
    Ok(ExecRequest {
        argv,
        env,
        cwd,
        stdin,
    })
}

/// Environment variables that map onto the CLI's global flags; `exec` appends
/// them to argv when the caller passed them through `env` (the extension
/// exports the same variables for the agent's own binary calls).
const ENV_FLAGS: [(&str, &str); 4] = [
    ("OINO_KANBOARD_ACTOR", "--actor"),
    ("OINO_KANBOARD_SESSION", "--session"),
    ("OINO_KANBOARD_PROJECT", "--project"),
    ("OINO_KANBOARD_CHAIN_GATE", "--gate"),
];

/// The flag's long name, when `argv` already sets it.
fn has_flag(argv: &[String], flag: &str) -> bool {
    argv.iter()
        .any(|arg| arg == flag || arg.starts_with(&format!("{flag}=")))
}

/// Runs `request` in-process and returns the `--json` payload; rule/usage/IO
/// failures come back as the CLI's `{ok:false,error,kind}` object.
#[must_use]
pub fn exec(request: &ExecRequest) -> Value {
    let mut argv = Vec::with_capacity(request.argv.len() + ENV_FLAGS.len() * 2 + 1);
    argv.push("oino-kanboard".to_owned());
    argv.extend(request.argv.iter().cloned());
    // Environment variables alias their global flag when argv leaves it unset.
    for (var, flag) in ENV_FLAGS {
        if let Some(value) = request.env.get(var).filter(|v| !v.trim().is_empty())
            && !has_flag(&argv, flag)
        {
            argv.push(flag.to_owned());
            argv.push(value.clone());
        }
    }
    let cli = match Cli::try_parse_from(&argv) {
        Ok(cli) => cli,
        Err(err) => {
            return json!({
                "ok": false,
                "error": err.to_string().trim().to_owned(),
                "kind": "usage",
            });
        }
    };
    let cwd = request
        .cwd
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    let env = &request.env;
    let exec = run::Exec {
        // The request's env comes on top of the host's, like `process.exec`
        // `env` did for the binary.
        var: &|name| {
            env.get(name)
                .cloned()
                .or_else(|| std::env::var(name).ok())
        },
        cwd: &cwd,
        stdin: request.stdin.as_deref().or(Some("")),
        // An in-process `start` without a pid claims for this process — the
        // same process the reaper sees alive.
        owner_pid: std::process::id(),
    };
    match run::dispatch_in(&cli, &exec) {
        Ok(payload) => payload,
        Err(err) => err_payload(&err),
    }
}

/// `exec` over a JSON request — the WIT/host entry point.
#[must_use]
pub fn exec_json(request: &Value) -> Value {
    match parse_request(request) {
        Ok(parsed) => exec(&parsed),
        Err(err) => err_payload(&err),
    }
}

/// The CLI's `--json` error line.
#[must_use]
pub fn err_payload(err: &Error) -> Value {
    json!({
        "ok": false,
        "error": err.to_string(),
        "kind": match err {
            Error::Usage(_) => "usage",
            Error::NotFound(_) => "not_found",
            Error::Rule(_) => "rule",
            _ => "io",
        },
    })
}

/// Whether a payload reports a failure (`{"ok": false, …}`).
#[must_use]
pub fn is_error(payload: &Value) -> bool {
    payload.get("ok") == Some(&Value::Bool(false)) || payload.get("error").is_some()
}

// ── file watch ─────────────────────────────────────────────────────────────

/// Watches `<home>/projects` and calls `changed(slug)` for content changes
/// (task `.md` files and `project.json`), coalesced to one call per 150 ms per
/// project — the refresh signal the daemon's SSE stream used to carry (round
/// 5). Writes through the CLI binary or the agent land here too.
///
/// Returns the watcher to keep alive; `Err` only when the OS refuses the watch
/// entirely (an exhausted inotify budget reports per-path instead).
pub fn watch_projects(
    layout: &Layout,
    mut changed: impl FnMut(&str) + Send + 'static,
) -> crate::Result<notify::RecommendedWatcher> {
    use notify::{RecursiveMode, Watcher as _};

    let projects = layout.projects_root();
    std::fs::create_dir_all(&projects)?;
    // Strip against the canonical root too: macOS reports FSEvents paths under
    // /private/var while the layout root is the /var symlink (and Linux keeps
    // the symlink), so accept either spelling of the watched directory.
    let canonical_root = std::fs::canonicalize(&projects).unwrap_or_else(|_| projects.clone());
    let (tx, rx) = std::sync::mpsc::channel::<PathBuf>();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let Ok(event) = event else { return };
        // Only content changes count. Access events would make the UI reload on
        // its own reads — an endless reload loop.
        if matches!(event.kind, notify::EventKind::Access(_)) {
            return;
        }
        for path in event.paths {
            let _ = tx.send(path);
        }
    })
    .map_err(|err| Error::Io(format!("cannot watch {}: {err}", projects.display())))?;
    watcher
        .watch(&projects, RecursiveMode::Recursive)
        .map_err(|err| Error::Io(format!("cannot watch {}: {err}", projects.display())))?;

    std::thread::Builder::new()
        .name("kanboard-watch".to_owned())
        .spawn(move || {
            // One write is often several events (tmp file, rename, close): coalesce.
            let mut last: BTreeMap<String, Instant> = BTreeMap::new();
            while let Ok(path) = rx.recv() {
                // Ignore lock files and temp files: only real content changes count.
                let name = path
                    .file_name()
                    .map(|name| name.to_string_lossy().to_string());
                let relevant = name.as_deref().is_some_and(|name| {
                    std::path::Path::new(name)
                        .extension()
                        .is_some_and(|ext| ext.eq_ignore_ascii_case("md"))
                        || name == "project.json"
                });
                if !relevant {
                    continue;
                }
                let Some(slug) = path
                    .strip_prefix(&canonical_root)
                    .or_else(|_| path.strip_prefix(&projects))
                    .ok()
                    .and_then(|rest| rest.components().next())
                    .map(|first| first.as_os_str().to_string_lossy().to_string())
                else {
                    continue;
                };
                if let Some(previous) = last.get(&slug)
                    && previous.elapsed() < Duration::from_millis(150)
                {
                    continue;
                }
                last.insert(slug.clone(), Instant::now());
                changed(&slug);
            }
        })
        .map_err(|err| Error::Io(format!("cannot spawn the kanboard watcher: {err}")))?;

    Ok(watcher)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_runs_a_request() {
        let tmp = tempfile::tempdir().expect("tmp");
        let home = tmp.path().join("kb");
        let request = json!({
            "argv": ["project", "add", "--root", tmp.path().to_string_lossy(), "--name", "Demo"],
            "env": {"OINO_KANBOARD_HOME": home.to_string_lossy()},
            "cwd": tmp.path().to_string_lossy(),
        });
        let payload = exec_json(&request);
        assert!(!is_error(&payload), "{payload}");
        let slug = payload["slug"].as_str().expect("slug");

        let payload = exec_json(&json!({
            "argv": ["add", "First task", "--status", "todo"],
            "env": {"OINO_KANBOARD_HOME": home.to_string_lossy(), "OINO_KANBOARD_PROJECT": slug},
            "cwd": tmp.path().to_string_lossy(),
        }));
        assert!(!is_error(&payload), "{payload}");
        assert_eq!(payload["status"], "todo");

        let payload = exec_json(&json!({
            "argv": ["list", "--all"],
            "env": {"OINO_KANBOARD_HOME": home.to_string_lossy(), "OINO_KANBOARD_PROJECT": slug},
        }));
        let tasks = payload["tasks"].as_array().expect("tasks");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0]["title"], "First task");
    }

    #[test]
    fn rule_errors_come_back_as_payloads() {
        let tmp = tempfile::tempdir().expect("tmp");
        let home = tmp.path().join("kb");
        let payload = exec_json(&json!({
            "argv": ["list"],
            "env": {"OINO_KANBOARD_HOME": home.to_string_lossy()},
            "cwd": tmp.path().to_string_lossy(),
        }));
        assert!(is_error(&payload));
        assert_eq!(payload["kind"], "not_found");
        assert_eq!(
            exec_json(&json!({"argv": ["frobnicate"]}))["kind"],
            "usage"
        );
    }
}
