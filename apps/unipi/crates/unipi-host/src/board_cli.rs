//! Board on the `unipi-kanboard` CLI (app.md §3.14 "Board" delta, WP-A22;
//! G-18; ext-abi §5.5).
//!
//! The v1 daemon relay is gone: the Board is a first-party screen and the host
//! is a thin wrapper that runs the CLI **one process per call**, `--json`,
//! actor pinned to `user` (only the agent's own calls say `--actor agent`),
//! with a subcommand allow-list. The §5.5 contract is decoded here — one JSON
//! value on stdout on success, the `{ok:false, error, kind}` stderr line on
//! failure — so the app sees a structured outcome, not exit codes.
//!
//! `board_watch{on}` refcounts a process-global file watcher over the board
//! home (the CLI crate's [`unipi_kanboard::service::watch_projects`], the same
//! one the deleted SSE stream used): one `BoardChanged{project}` per content
//! change, emitted on every open `events_open` stream. CLI writes and agent
//! writes both land here — the watcher is on the files, not the writer.

use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use tokio::process::Command;
use tokio::sync::broadcast;
use unipi_proto::{AppErrorCode, AppErrorDto, BoardCliData, BoardErrorKind, ProtoError};

/// How long one CLI call may take (local file ops; `validate` on a big board
/// is the slowest and stays well under this).
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// Buffer of the watcher's broadcast.
const EVENT_CAPACITY: usize = 64;

/// Subcommands the Board may run (the allow-list). Anything the agent's own
/// surface owns (`start`, `finish`, `reap`, `release`, …) is deliberately
/// absent: the app acts as the user, and the agent claims tasks through the
/// extension, not through the app.
pub const ALLOWED_SUBCOMMANDS: &[&str] = &[
    "project",
    "add",
    "list",
    "show",
    "move",
    "note",
    "attach",
    "attachments",
    "edit",
    "link",
    "unlink",
    "order",
    "duplicate",
    "search",
    "chain",
    "next",
    "validate",
    "status",
];

/// The `project` sub-subcommands allowed (`list`, `show`, `archive`,
/// `unarchive`, and `add` with an explicit `--root` inside the PC's allowed
/// folders — the Board's "Add project"; rebinding stays the CLI's).
const ALLOWED_PROJECT_SUBCOMMANDS: &[&str] = &["list", "show", "archive", "unarchive", "add"];

/// The flags `project add` may carry from the app (each takes one value).
const PROJECT_ADD_FLAGS: &[&str] = &["--root", "--name", "--prefix"];

/// One board change the watcher reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardChangedEvent {
    /// Project slug (its folder under `<home>/projects`).
    pub project: String,
}

/// Rejects argv the Board may not run.
///
/// # Errors
///
/// `E_SCHEMA` naming the refusal.
pub fn check_allowed(args: &[String]) -> Result<(), AppErrorDto> {
    let refused = |message: String| AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false);
    let Some(sub) = args.first() else {
        return Err(refused("board_cli needs a subcommand".to_string()));
    };
    if !ALLOWED_SUBCOMMANDS.contains(&sub.as_str()) {
        return Err(refused(format!(
            "`{sub}` is not a board_cli subcommand the app may run"
        )));
    }
    if sub == "project"
        && !args
            .get(1)
            .is_some_and(|next| ALLOWED_PROJECT_SUBCOMMANDS.contains(&next.as_str()))
    {
        return Err(refused(
            "board_cli `project` only allows list, show, archive, unarchive, add".to_string(),
        ));
    }
    if sub == "project" && args.get(1).is_some_and(|next| next == "add") {
        let root = project_add_root(args)
            .map_err(|message| refused(format!("board_cli `project add`: {message}")))?;
        if !std::path::Path::new(&root).is_absolute() {
            return Err(refused(
                "board_cli `project add` needs an absolute --root".to_string(),
            ));
        }
    }
    Ok(())
}

/// The `--root` of a `project add` argv, after checking every other argument
/// is one of [`PROJECT_ADD_FLAGS`] with its value (no positional, no other flag).
fn project_add_root(args: &[String]) -> Result<String, String> {
    let mut root = None;
    let mut rest = args.iter().skip(2);
    while let Some(arg) = rest.next() {
        let (flag, value) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (arg.as_str(), None),
        };
        if !PROJECT_ADD_FLAGS.contains(&flag) {
            return Err(format!("`{arg}` is not allowed"));
        }
        let value = match value {
            Some(value) => value,
            None => rest
                .next()
                .cloned()
                .ok_or_else(|| format!("{flag} needs a value"))?,
        };
        if flag == "--root" {
            root = Some(value);
        }
    }
    root.ok_or_else(|| "--root is required".to_string())
}

/// `project add` registers the folder the user picked: it must be inside the
/// PC's allowed folders (the same scope the folder browser lists), and the
/// CLI gets its canonical path. Other argv pass through unchanged.
///
/// # Errors
///
/// `E_SCOPE` / `E_NOT_FOUND` from the scope check.
pub fn scope_project_root(
    args: &[String],
    scope: &crate::fs::FsScope,
) -> Result<Vec<String>, AppErrorDto> {
    if !(args.first().is_some_and(|a| a == "project") && args.get(1).is_some_and(|a| a == "add")) {
        return Ok(args.to_vec());
    }
    let root = project_add_root(args)
        .map_err(|message| AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false))?;
    let canon = crate::fs::canonicalize_scoped(std::path::Path::new(&root), scope, false)?;
    if !canon.is_dir() {
        return Err(AppErrorDto::new(
            AppErrorCode::E_SCHEMA,
            format!("{} is not a folder", canon.display()),
            false,
        ));
    }
    let mut out: Vec<String> = vec![
        "project".into(),
        "add".into(),
        "--root".into(),
        canon.display().to_string(),
    ];
    let mut rest = args.iter().skip(2);
    while let Some(arg) = rest.next() {
        let (flag, value) = match arg.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => {
                (flag.to_owned(), Some(value.to_owned()))
            }
            _ => (arg.clone(), None),
        };
        let value = match value {
            Some(value) => value,
            None => rest.next().cloned().unwrap_or_default(),
        };
        if flag != "--root" {
            out.push(format!("{flag}={value}"));
        }
    }
    Ok(out)
}

/// The agent dir the extension bin dir and the default board home hang off
/// (`$OINO_AGENT_DIR`, else `$HOME` — the same base the CLI resolves).
fn agent_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("OINO_AGENT_DIR") {
        if !dir.trim().is_empty() {
            return Some(PathBuf::from(dir.trim()));
        }
    }
    std::env::var_os("HOME").map(PathBuf::from)
}

/// The board home the watcher watches — exactly what the CLI itself resolves
/// (`UNIPI_KANBOARD_HOME`, else `<agent dir>/state/unipi.kanboard`), so CLI
/// calls and the watcher always agree.
#[must_use]
pub fn board_home() -> PathBuf {
    if let Ok(home) = std::env::var("UNIPI_KANBOARD_HOME") {
        if !home.trim().is_empty() {
            return PathBuf::from(home.trim());
        }
    }
    let Some(dir) = agent_dir() else {
        return PathBuf::from("state/unipi.kanboard");
    };
    dir.join("state").join("unipi.kanboard")
}

/// Resolves the CLI binary: `$UNIPI_KANBOARD_BIN`, then the kanboard extension
/// package's `bin/`, then `PATH` (the installer puts the CLIs on PATH; the
/// package ships its platform build per ext-abi §5.2). A stale env override
/// that no longer exists does not shadow the real binary.
#[must_use]
pub fn resolve_binary() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("UNIPI_KANBOARD_BIN") {
        let path = PathBuf::from(path.trim());
        if path.is_file() {
            return Some(path);
        }
    }
    // The installed distribution's package first (the CLI ships inside the
    // bundled kanboard package), then a user-installed copy, then PATH.
    let roots = crate::sessions::services::dist_dirs()
        .into_iter()
        .chain(agent_dir().map(|dir| dir.join("extensions")));
    for root in roots {
        let packaged = root
            .join("unipi.kanboard")
            .join("bin")
            .join(executable_name());
        if packaged.is_file() {
            return Some(packaged);
        }
    }
    which_on_path()
}

#[cfg(windows)]
fn executable_name() -> &'static str {
    "unipi-kanboard.exe"
}
#[cfg(not(windows))]
fn executable_name() -> &'static str {
    "unipi-kanboard"
}

fn which_on_path() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(executable_name()))
        .find(|candidate| candidate.is_file())
}

/// `unipi-kanboard` is not resolvable — the shape the app's availability chip
/// maps to "not installed" (as the daemon's E_BOARD_UNAVAILABLE did).
fn not_installed() -> AppErrorDto {
    AppErrorDto::new(
        AppErrorCode::E_BOARD_UNAVAILABLE,
        "The board CLI (unipi-kanboard) was not found in this installation",
        false,
    )
    .with_hint("Reinstall UniPi: the installer ships the board with the app".to_owned())
    .with_detail(serde_json::json!({"reason": "not_installed"}))
}

/// Runs one CLI call and decodes the §5.5 outcome.
///
/// # Errors
///
/// `E_SCHEMA` for a disallowed argv, `E_BOARD_UNAVAILABLE` when the binary is
/// missing, `E_TIMEOUT` when it stalls, `E_TRANSPORT` when it cannot be
/// spawned — the CLI's own refusals are an `Ok` reply carrying
/// `{ok:false, error, kind}`.
pub async fn run_cli(args: &[String], project: Option<&str>) -> Result<BoardCliData, AppErrorDto> {
    check_allowed(args)?;
    let binary = resolve_binary().ok_or_else(not_installed)?;

    let mut argv: Vec<String> = vec!["--json".into(), "--actor".into(), "user".into()];
    if let Some(slug) = project {
        argv.push("--project".into());
        argv.push(slug.to_owned());
    }
    argv.extend(args.iter().cloned());

    let call = Command::new(&binary)
        .args(&argv)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .output();
    let output = match tokio::time::timeout(CALL_TIMEOUT, call).await {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => {
            return Err(
                ProtoError::Transport(format!("cannot run {}: {e}", binary.display()))
                    .to_app_error(),
            )
        }
        Err(_) => {
            return Err(
                ProtoError::Timeout("The board command did not finish in time".into())
                    .to_app_error(),
            )
        }
    };

    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    match output.status.code() {
        Some(0) => {
            let payload =
                serde_json::from_str(stdout.trim_start_matches('\u{feff}')).map_err(|e| {
                    AppErrorDto::new(
                        AppErrorCode::E_INTERNAL,
                        format!("unipi-kanboard printed invalid JSON: {e}"),
                        false,
                    )
                })?;
            Ok(BoardCliData::success(payload))
        }
        Some(1) | Some(2) => Ok(decode_failure(&stderr, output.status.code())),
        code => Err(AppErrorDto::new(
            AppErrorCode::E_INTERNAL,
            format!(
                "unipi-kanboard exited with {}: {}",
                code.map_or_else(|| "signal".to_string(), |c| c.to_string()),
                stderr.trim()
            ),
            false,
        )),
    }
}

/// The stderr JSON line → [`BoardCliData::failure`] (exit 2 is usage, 1 the
/// rest; a line that will not parse degrades to a failure carrying the text).
fn decode_failure(stderr: &str, code: Option<i32>) -> BoardCliData {
    let line = stderr
        .lines()
        .rev()
        .find(|line| line.starts_with('{') && line.ends_with('}'))
        .unwrap_or(stderr.trim());
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(line) {
        let error = value["error"]
            .as_str()
            .unwrap_or("the board refused the change");
        let kind = match value["kind"].as_str() {
            Some("usage") => BoardErrorKind::Usage,
            Some("not_found") => BoardErrorKind::NotFound,
            Some("rule") => BoardErrorKind::Rule,
            _ => BoardErrorKind::Io,
        };
        return BoardCliData::failure(error, kind);
    }
    let kind = if code == Some(2) {
        BoardErrorKind::Usage
    } else {
        BoardErrorKind::Io
    };
    let text = if line.is_empty() {
        format!(
            "unipi-kanboard exited with {}",
            code.map_or_else(|| "a signal".to_string(), |c| c.to_string())
        )
    } else {
        line.to_owned()
    };
    BoardCliData::failure(text, kind)
}

/// The process-global board watcher (refcounted by `board_watch`).
pub struct BoardWatchers {
    refs: Mutex<usize>,
    watcher: Mutex<Option<unipi_kanboard::service::Watcher>>,
    events: broadcast::Sender<BoardChangedEvent>,
}

static SHARED: OnceLock<BoardWatchers> = OnceLock::new();

impl BoardWatchers {
    fn new() -> Self {
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        Self {
            refs: Mutex::new(0),
            watcher: Mutex::new(None),
            events,
        }
    }

    /// The host's one registry (`board_watch`, tests).
    #[must_use]
    pub fn shared() -> &'static Self {
        SHARED.get_or_init(Self::new)
    }

    /// A new subscriber: every change after now (a lagging receiver is
    /// skipped — the app re-reads the whole project anyway).
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<BoardChangedEvent> {
        self.events.subscribe()
    }

    /// Turns the watcher on or off; the last `off` stops it. Returns whether
    /// it is running after the call.
    pub fn set(&self, on: bool) -> bool {
        let mut refs = lock(&self.refs);
        if on {
            *refs += 1;
            if *refs == 1 && lock(&self.watcher).is_none() && !self.start() {
                // The OS refused the watch: leave the refcount honest.
                *refs = 0;
            }
        } else {
            *refs = refs.saturating_sub(1);
            if *refs == 0 {
                *lock(&self.watcher) = None;
            }
        }
        *refs > 0
    }

    /// Starts [`unipi_kanboard::service::watch_projects`] over the board home.
    /// Its events fan out to this registry's broadcast and, when this process
    /// serves sessions, the hub's `events_open` stream. Returns success.
    fn start(&self) -> bool {
        let layout = unipi_kanboard::store::Layout::with_home(board_home());
        let (tx, rx) = std::sync::mpsc::channel::<BoardChangedEvent>();
        match unipi_kanboard::service::watch_projects(&layout, move |slug| {
            let _ = tx.send(BoardChangedEvent {
                project: slug.to_owned(),
            });
        }) {
            Ok(watcher) => {
                *lock(&self.watcher) = Some(watcher);
                let fanout = std::thread::Builder::new()
                    .name("board-watch".to_owned())
                    .spawn(move || {
                        while let Ok(event) = rx.recv() {
                            let registry = BoardWatchers::shared();
                            let _ = registry.events.send(event.clone());
                            if let Some(service) = crate::sessions::SessionService::shared() {
                                service.hub().emit_board_changed(&event.project);
                            }
                        }
                    });
                if fanout.is_err() {
                    *lock(&self.watcher) = None;
                    return false;
                }
                true
            }
            Err(err) => {
                tracing::warn!(home = %layout.home.display(), error = %err, "cannot watch the board");
                false
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allow_list_covers_the_app_surface_only() {
        for args in [
            vec!["list"],
            vec!["project", "list"],
            vec!["add", "T-1", "--status", "todo"],
            vec!["move", "T-1", "done"],
            vec!["validate"],
            vec!["project", "add", "--root", "/home/u/repo"],
            vec!["project", "add", "--root=/home/u/repo", "--name", "Repo"],
        ] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            assert!(check_allowed(&args).is_ok(), "{args:?}");
        }
        for args in [
            vec!["start", "T-1"],
            vec!["finish", "T-1", "--comment", "x"],
            vec!["reap"],
            vec!["release", "T-1", "--to", "todo", "--comment", "x"],
            vec!["project", "add", "--root", "."],
            vec!["project", "add"],
            vec!["project", "add", "--root", "/x", "--force"],
            vec!["project", "add", "/x"],
            vec!["project", "rebind", "p", "--root", "."],
            vec!["frobnicate"],
            vec![],
        ] {
            let args: Vec<String> = args.into_iter().map(String::from).collect();
            let error = check_allowed(&args).unwrap_err();
            assert_eq!(error.code, AppErrorCode::E_SCHEMA, "{args:?}");
        }
    }

    #[test]
    fn failure_lines_decode_per_contract() {
        let data = decode_failure(
            "{\"ok\":false,\"error\":\"no project\",\"kind\":\"not_found\"}\n",
            Some(1),
        );
        assert_eq!(
            data,
            BoardCliData::failure("no project", BoardErrorKind::NotFound)
        );
        let data = decode_failure("unipi-kanboard frobnicate\nusage text…", Some(2));
        // No JSON line: degrade to usage with the stderr text.
        assert_eq!(data.kind, Some(BoardErrorKind::Usage));
        assert!(data.error.unwrap().contains("frobnicate"));
    }
}
