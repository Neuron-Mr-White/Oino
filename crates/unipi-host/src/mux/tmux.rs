//! tmux integration: tree, grouped-session attach and preview
//! (DESIGN.md §5.6). Every invocation goes through [`command`] so the
//! private test socket (`UNIPI_TMUX_SOCKET` → `tmux -L <name>`) is
//! honoured everywhere.

#[cfg(not(windows))]
use std::process::Stdio;

use tokio::process::Command;
use unipi_proto::{
    AgentSessionRef, AgentStatus, AppErrorCode, AppErrorDto, AttachTarget, MuxAction, PaneAgent,
    PreviewData, TmuxId, TmuxPane, TmuxSession, TmuxState, TmuxWindow,
};

/// Env var naming a private tmux socket for tests.
pub const TMUX_SOCKET_ENV: &str = "UNIPI_TMUX_SOCKET";

fn unavailable(reason: String) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_MUX_UNAVAILABLE, reason, true)
        .with_hint("install tmux: https://github.com/tmux/tmux")
}

fn internal(message: String) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, message, false)
}

/// tmux does not exist on Windows; say so instead of surfacing
/// "failed to run tmux: program not found" (the host's multiplexer there
/// is herdr).
#[cfg(windows)]
fn unsupported_here() -> AppErrorDto {
    AppErrorDto::new(
        AppErrorCode::E_UNSUPPORTED,
        "tmux isn't available on Windows — use herdr",
        false,
    )
}

/// Builds a `tmux` command honouring the private test socket.
pub fn command() -> Command {
    let mut command = Command::new("tmux");
    if let Ok(socket) = std::env::var(TMUX_SOCKET_ENV) {
        if !socket.is_empty() {
            command.arg("-L").arg(socket);
        }
    }
    command
}

pub(crate) async fn run(args: &[&str]) -> Result<Vec<u8>, AppErrorDto> {
    #[cfg(windows)]
    {
        let _ = args;
        return Err(unsupported_here());
    }
    #[cfg(not(windows))]
    {
        let output = command()
            .args(args)
            .stdin(Stdio::null())
            .output()
            .await
            .map_err(|error| {
                tracing::warn!(error = %error, "tmux is not runnable");
                unavailable(format!("failed to run tmux: {error}"))
            })?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            // "no server running" / "error connecting" mean tmux exists
            // but has no server: an available-but-empty tree.
            if stderr.contains("no server running") || stderr.contains("error connecting") {
                return Ok(Vec::new());
            }
            tracing::warn!(args = ?args, stderr = %stderr, "tmux command failed");
            return Err(unavailable(format!("tmux {} failed: {stderr}", args[0])));
        }
        Ok(output.stdout)
    }
}

/// Rows joined by the unit separator so names with spaces or colons
/// stay single fields.
const SESSION_FORMAT: &str = "#{session_id}\u{241f}#{session_name}\u{241f}#{session_attached}\u{241f}#{session_windows}\u{241f}#{session_created}\u{241f}#{session_activity}";
const WINDOW_FORMAT: &str =
    "#{session_id}\u{241f}#{window_id}\u{241f}#{window_index}\u{241f}#{window_name}\u{241f}#{window_active}";
const PANE_FORMAT: &str = "#{window_id}\u{241f}#{pane_id}\u{241f}#{pane_index}\u{241f}#{pane_active}\u{241f}#{pane_current_command}\u{241f}#{pane_current_path}\u{241f}#{pane_pid}\u{241f}#{pane_title}\u{241f}#{@unipi_status}\u{241f}#{@unipi_session}";

pub async fn tree() -> Result<TmuxState, AppErrorDto> {
    let session_rows = match run(&["list-sessions", "-F", SESSION_FORMAT]).await {
        Ok(rows) => rows,
        Err(error) => {
            return Ok(TmuxState {
                available: false,
                reason: Some(error.message.as_str().to_string()),
                sessions: None,
            });
        }
    };
    let window_rows = run(&["list-windows", "-a", "-F", WINDOW_FORMAT])
        .await
        .unwrap_or_default();
    let pane_rows = run(&["list-panes", "-a", "-F", PANE_FORMAT])
        .await
        .unwrap_or_default();

    let mut windows: std::collections::BTreeMap<String, Vec<TmuxWindow>> =
        std::collections::BTreeMap::new();
    for row in window_rows
        .split(|byte| *byte == b'\n')
        .filter(|row| !row.is_empty())
    {
        let row_text = String::from_utf8_lossy(row).to_string();
        let fields: Vec<&str> = row_text.split('\u{241f}').collect();
        if fields.len() != 5 {
            tracing::warn!(
                row = row_text,
                expected = 5,
                "skipping malformed tmux window row"
            );
            continue;
        }
        let window = TmuxWindow {
            id: TmuxId::try_from(fields[1].to_string()).map_err(internal)?,
            index: fields[2].parse().unwrap_or(0),
            name: fields[3].to_string(),
            active: fields[4] == "1",
            panes: Vec::new(),
        };
        windows
            .entry(fields[0].to_string())
            .or_default()
            .push(window);
    }

    let mut panes: std::collections::BTreeMap<String, Vec<TmuxPane>> =
        std::collections::BTreeMap::new();
    for row in pane_rows
        .split(|byte| *byte == b'\n')
        .filter(|row| !row.is_empty())
    {
        let row_text = String::from_utf8_lossy(row).to_string();
        let fields: Vec<&str> = row_text.split('\u{241f}').collect();
        if fields.len() < 10 {
            continue;
        }
        let pane = TmuxPane {
            id: TmuxId::try_from(fields[1].to_string()).map_err(internal)?,
            index: fields[2].parse().unwrap_or(0),
            active: fields[3] == "1",
            current_command: fields[4].to_string(),
            cwd: fields[5].to_string(),
            pid: fields[6].parse().unwrap_or(0),
            title: fields[7].to_string(),
            agent: pane_agent(fields[4], fields[8], fields[9]),
            chat: None,
        };
        panes.entry(fields[0].to_string()).or_default().push(pane);
    }
    for window_list in windows.values_mut() {
        for window in window_list.iter_mut() {
            if let Some(pane_list) = panes.remove(window.id.as_str()) {
                window.panes = pane_list;
            }
        }
    }
    let mut sessions = Vec::new();
    let session_text = String::from_utf8_lossy(&session_rows).to_string();
    for row in session_text.split('\n').filter(|row| !row.is_empty()) {
        let fields: Vec<&str> = row.split('\u{241f}').collect();
        if fields.len() < 6 {
            continue;
        }
        let (id, name) = (fields[0].to_string(), fields[1].to_string());
        // Our grouped sessions (created for phone attaches) never
        // belong in the tree.
        if name.starts_with("unipi-") {
            continue;
        }
        sessions.push(TmuxSession {
            id: TmuxId::try_from(id.clone()).map_err(internal)?,
            name,
            attached: fields[2].parse().unwrap_or(0),
            window_count: fields[3].parse().unwrap_or(0),
            created: fields[4].parse().unwrap_or(0),
            activity: fields[5].parse().unwrap_or(0),
            windows: windows.remove(&id).unwrap_or_default(),
        });
    }
    Ok(TmuxState {
        available: true,
        reason: None,
        sessions: Some(sessions),
    })
}

/// tmux knows nothing about agents: a pane is pi when its foreground
/// process is `pi`, with unknown status, unless the pane options
/// `@unipi_status` / `@unipi_session` say otherwise.
pub fn pane_agent(current_command: &str, status: &str, session: &str) -> Option<PaneAgent> {
    let reported = !status.is_empty() || !session.is_empty();
    if current_command != "pi" && !reported {
        return None;
    }
    Some(PaneAgent {
        name: "pi".into(),
        status: if status.is_empty() {
            AgentStatus::Unknown
        } else {
            AgentStatus::from_herdr(status)
        },
        session: if session.is_empty() {
            None
        } else {
            Some(AgentSessionRef {
                kind: "path".into(),
                value: session.into(),
            })
        },
    })
}

/// Creates a grouped session for a phone attach and selects the
/// requested window/pane. Returns its name.
pub async fn create_grouped(
    session: &str,
    window_id: Option<&TmuxId<'@'>>,
    pane_id: Option<&TmuxId<'%'>>,
) -> Result<String, AppErrorDto> {
    #[cfg(windows)]
    {
        let _ = (session, window_id, pane_id);
        return Err(unsupported_here());
    }
    #[cfg(not(windows))]
    {
    let device = std::env::var("UNIPI_DEVICE_TAG").unwrap_or_else(|_| "host".into());
    let device: String = device
        .chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .take(8)
        .collect();
    let grouped = format!("unipi-{device}-{}", chrono_short());
    let web = command()
        .args(["new-session", "-d", "-t", session, "-s", &grouped])
        .output()
        .await
        .map_err(|error| unavailable(format!("tmux new-session failed: {error}")))?;
    let status = &web.status;
    let status_web_output = &web.stderr;
    if !status.success() {
        return Err(unavailable(format!(
            "tmux new-session -t {session} failed: {}",
            String::from_utf8_lossy(status_web_output)
        )));
    }
    tracing::debug!(grouped, "tmux grouped session created");
    // NOTE: `destroy-unattached on` would destroy the grouped session
    // immediately: it is created detached (zero clients). The host
    // kills it explicitly once the attach stream ends instead.
    if let Some(window_id) = window_id {
        let _ = command()
            .args(["select-window", "-t", window_id.as_str()])
            .output()
            .await;
    }
    if let Some(pane_id) = pane_id {
        let _ = command()
            .args(["select-pane", "-t", pane_id.as_str()])
            .output()
            .await;
    }
    Ok(grouped)
    }
}

#[cfg(not(windows))]
fn chrono_short() -> String {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("{:x}", now)
}

/// The PTY command attaching to the grouped session, ignoring the
/// phone's size when the policy is keep_pc.
pub fn attach_command(grouped: &str, keep_pc: bool) -> portable_pty::CommandBuilder {
    let mut command = portable_pty::CommandBuilder::new("tmux");
    if let Ok(socket) = std::env::var(TMUX_SOCKET_ENV) {
        if !socket.is_empty() {
            command.arg("-L");
            command.arg(socket);
        }
    }
    if keep_pc {
        command.args(["attach-session", "-f", "ignore-size", "-t", grouped]);
    } else {
        command.args(["attach-session", "-t", grouped]);
    }
    command.env("TERM", "xterm-256color");
    for (key, _) in std::env::vars() {
        if key.starts_with("TMUX") {
            command.env_remove(key);
        }
    }
    command
}

/// Kills a grouped session if it still exists (cleanup after the PTY
/// exits; run unconditionally, even when the stream errored).
pub async fn kill_grouped(grouped: &str) {
    let _ = command()
        .args(["kill-session", "-t", grouped])
        .output()
        .await;
}

/// Startup sweep: kills every `unipi-*` session with no client
/// attached (orphans from a crashed host or dropped stream).
/// Attached grouped sessions belong to a live phone and stay.
pub async fn sweep_orphans() {
    let Ok(output) = command()
        .args([
            "list-sessions",
            "-F",
            "#{session_name}\u{241f}#{session_attached}",
        ])
        .output()
        .await
    else {
        return;
    };
    if !output.status.success() {
        return; // no server running: nothing to sweep
    }
    tracing::debug!(rows=%String::from_utf8_lossy(&output.stdout), "sweep sees sessions");
    for row in String::from_utf8_lossy(&output.stdout).lines() {
        let Some((name, attached)) = row.rsplit_once('\u{241f}') else {
            continue;
        };
        if name.starts_with("unipi-") && attached == "0" {
            tracing::warn!(session = name, "killing orphaned grouped session");
            let _ = command().args(["kill-session", "-t", name]).output().await;
        }
    }
}

/// `capture-pane -p -e -S -<lines>` keeps ANSI escapes.
/// Creates, renames, kills or navigates within tmux sessions
/// (DESIGN §3.3 multiplexer parity). Window/pane navigation acts on
/// the phone's grouped session when one shares the target's group,
/// so the phone moves without moving the PC's view.
pub async fn action(action: &MuxAction) -> Result<serde_json::Value, AppErrorDto> {
    match action {
        MuxAction::TmuxSessionCreate { name, cwd } => {
            let mut args = vec!["new-session", "-d", "-s", name.as_str()];
            let cwd_arg;
            if let Some(cwd) = cwd {
                cwd_arg = cwd.clone();
                args.push("-c");
                args.push(&cwd_arg);
            }
            run(&args).await?;
            Ok(serde_json::Value::Null)
        }
        MuxAction::TmuxSessionRename { session, name } => {
            run(&["rename-session", "-t", session.as_str(), name.as_str()]).await?;
            Ok(serde_json::Value::Null)
        }
        MuxAction::TmuxSessionKill { session } => {
            run(&["kill-session", "-t", session.as_str()]).await?;
            Ok(serde_json::Value::Null)
        }
        MuxAction::WindowNext { session } => nav_window(session.as_str(), "+").await,
        MuxAction::WindowPrev { session } => nav_window(session.as_str(), "-").await,
        MuxAction::TmuxPaneNext { session } => nav_pane(session.as_str(), "+").await,
        MuxAction::TmuxPanePrev { session } => nav_pane(session.as_str(), "-").await,
        _ => Err(unavailable("not a tmux action".to_string())),
    }
}

/// The session window/pane navigation applies to: the unipi grouped
/// session sharing the target's group when one exists, else the
/// target session itself.
async fn nav_target(session: &str) -> Result<String, AppErrorDto> {
    let group = run(&["display-message", "-p", "-t", session, "#{session_group}"]).await?;
    let group = String::from_utf8_lossy(&group).trim().to_string();
    if group.is_empty() {
        return Ok(session.to_string());
    }
    let out = run(&[
        "list-sessions",
        "-F",
        "#{session_name}\u{241f}#{session_group}",
    ])
    .await?;
    for line in String::from_utf8_lossy(&out).lines() {
        let mut parts = line.split('\u{241f}');
        let (name, session_group) = (parts.next().unwrap_or(""), parts.next().unwrap_or(""));
        if session_group == group && name.starts_with("unipi-") {
            return Ok(name.to_string());
        }
    }
    Ok(session.to_string())
}

async fn nav_window(session: &str, step: &str) -> Result<serde_json::Value, AppErrorDto> {
    let target = nav_target(session).await?;
    run(&["select-window", "-t", &format!("{target}:{step}")]).await?;
    Ok(serde_json::Value::Null)
}

async fn nav_pane(session: &str, step: &str) -> Result<serde_json::Value, AppErrorDto> {
    let target = nav_target(session).await?;
    run(&["select-pane", "-t", &format!("{target}:.{step}")]).await?;
    Ok(serde_json::Value::Null)
}

pub async fn preview(target: &AttachTarget, lines: u16) -> Result<PreviewData, AppErrorDto> {
    #[cfg(windows)]
    {
        let _ = (target, lines);
        return Err(unsupported_here());
    }
    #[cfg(not(windows))]
    {
    let AttachTarget::Tmux {
        session, pane_id, ..
    } = target
    else {
        return Err(internal(
            "tmux preview called with a non-tmux target".into(),
        ));
    };
    let target_spec = match pane_id {
        Some(pane_id) => pane_id.as_str().to_string(),
        None => format!("{}:", session.as_str()),
    };
    let mut args: Vec<String> = vec![
        "capture-pane".into(),
        "-p".into(),
        "-e".into(),
        "-S".into(),
        format!("-{lines}"),
        "-t".into(),
        target_spec,
    ];
    if let Ok(socket) = std::env::var(TMUX_SOCKET_ENV) {
        if !socket.is_empty() {
            args.insert(0, socket.clone());
            args.insert(0, "-L".into());
        }
    }
    let output = Command::new("tmux")
        .args(&args)
        .output()
        .await
        .map_err(|error| unavailable(format!("tmux capture-pane failed: {error}")))?;
    if !output.status.success() {
        return Err(unavailable(format!(
            "tmux capture-pane failed: {}",
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    Ok(PreviewData {
        text: text.try_into().map_err(internal)?,
    })
    }
}

#[test]
fn ambiguous_names_survive_the_separator() {
    // Session name with ::: (tmux itself maps colons in session
    // names to _, but be robust), window name ending in :, pane
    // title ending in :.
    let session_row = "$9\u{241f}a:::b\u{241f}1\u{241f}1\u{241f}1700000000\u{241f}1700000000";
    let window_row = "$9\u{241f}@4\u{241f}0\u{241f}x:\u{241f}1";
    let pane_row = "@4\u{241f}%8\u{241f}0\u{241f}0\u{241f}pi\u{241f}/home/u/p\u{241f}9\u{241f}foo:\u{241f}\u{241f}";

    let session_fields: Vec<&str> = session_row.split('\u{241f}').collect();
    assert_eq!(session_fields.len(), 6);
    assert_eq!(session_fields[1], "a:::b");

    let window_fields: Vec<&str> = window_row.split('\u{241f}').collect();
    assert_eq!(window_fields.len(), 5);
    assert_eq!(window_fields[3], "x:");

    let pane_fields: Vec<&str> = pane_row.split('\u{241f}').collect();
    assert_eq!(pane_fields.len(), 10);
    assert_eq!(pane_fields[7], "foo:");
    let agent = pane_agent(pane_fields[4], pane_fields[8], pane_fields[9]);
    assert!(agent.is_some());
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_rows_with_spaces_and_colons() {
        let session_row =
            "$3\u{241f}my work: lab\u{241f}2\u{241f}2\u{241f}1700000000\u{241f}1700000001";
        let fields: Vec<&str> = session_row.split('\u{241f}').collect();
        assert_eq!(fields[1], "my work: lab");
        assert_eq!(fields[2], "2");

        let pane_row = "%7\u{241f}0\u{241f}1\u{241f}pi\u{241f}/home/u/my dir: ok\u{241f}4242\u{241f}foo:\u{241f}\u{241f}";
        let fields: Vec<&str> = pane_row.split('\u{241f}').collect();
        assert_eq!(fields[3], "pi");
        assert_eq!(fields[4], "/home/u/my dir: ok");
        let agent = pane_agent(fields[3], fields[7], fields[8]);
        let agent = agent.expect("pi pane gets an agent");
        assert!(matches!(agent.status, AgentStatus::Unknown));
        assert!(agent.session.is_none());
    }

    #[test]
    fn pane_agent_uses_unipi_options_when_present() {
        let agent = pane_agent("vi", "working", "/home/u/.pi/agent/sessions/a.json").unwrap();
        assert!(matches!(agent.status, AgentStatus::Working));
        assert_eq!(agent.session.as_ref().unwrap().kind, "path");
        assert!(pane_agent("vi", "", "").is_none());
    }
}
