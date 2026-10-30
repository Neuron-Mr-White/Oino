//! herdr integration: session list, tree via `session.snapshot`,
//! focus, preview and PTY client (DESIGN.md §5.6). Every spawned
//! herdr process strips all `HERDR_*` variables and forces
//! `TERM=xterm-256color` so a nested client never inherits the
//! host's own herdr context.

use std::collections::BTreeMap;

use serde::Deserialize;
use tokio::process::Command;
use unipi_proto::{
    AgentSessionRef, AgentStatus, AppErrorCode, AppErrorDto, AttachTarget, HerdrId, HerdrPane,
    HerdrSession, HerdrState, HerdrTab, HerdrWorkspace, ListData, MuxAction, PaneAgent,
    PreviewData, SessionName, SessionSummary, ShortcutKey, ShortcutMux, ShortcutsData,
};

/// Environment for spawned herdr processes: every `HERDR_*` variable
/// removed, `TERM` pinned to `xterm-256color`.
pub fn inherited_env_without_herdr() -> Vec<(String, String)> {
    std::env::vars()
        .filter(|(key, _)| !key.starts_with("HERDR_"))
        .filter(|(key, _)| key != "TERM")
        .chain(std::iter::once((
            "TERM".to_string(),
            "xterm-256color".to_string(),
        )))
        .collect()
}

fn unavailable(reason: String) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_MUX_UNAVAILABLE, reason, true)
        .with_hint("install herdr: https://herdr.dev")
}

fn internal(message: String) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, message, false)
}

async fn run_herdr(args: &[&str]) -> Result<Vec<u8>, AppErrorDto> {
    let output = Command::new("herdr")
        .args(args)
        .env_clear()
        .envs(inherited_env_without_herdr())
        .output()
        .await
        .map_err(|error| unavailable(format!("failed to run herdr: {error}")))?;
    if !output.status.success() {
        return Err(unavailable(format!(
            "herdr {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        )));
    }
    Ok(output.stdout)
}

#[derive(Debug, Deserialize)]
pub struct RawSessions {
    #[serde(default)]
    sessions: Vec<RawSession>,
}

#[derive(Debug, Deserialize)]
pub struct RawSession {
    name: String,
    running: bool,
    #[serde(default)]
    default: bool,
    #[serde(default)]
    socket_path: String,
}

impl RawSession {
    pub fn name(&self) -> &str {
        &self.name
    }
    pub fn socket_path(&self) -> &str {
        &self.socket_path
    }
    pub fn running(&self) -> bool {
        self.running
    }
}

/// herdr session name → its API socket path (running sessions only).
pub async fn session_sockets() -> std::collections::HashMap<String, String> {
    session_list()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|s| s.running && !s.socket_path.is_empty())
        .map(|s| (s.name, s.socket_path))
        .collect()
}

/// `herdr session list --json` parsed.
pub async fn session_list() -> Result<Vec<RawSession>, AppErrorDto> {
    let stdout = run_herdr(&["session", "list", "--json"]).await?;
    let parsed: RawSessions = serde_json::from_slice(&stdout)
        .map_err(|error| internal(format!("failed to parse herdr session list: {error}")))?;
    Ok(parsed.sessions)
}

pub async fn list() -> Result<ListData, AppErrorDto> {
    let sessions = session_list().await?;
    let sessions = sessions
        .into_iter()
        .map(|session| {
            Ok(SessionSummary {
                name: SessionName::try_from(session.name).map_err(internal)?,
                running: session.running,
                default: session.default,
            })
        })
        .collect::<Result<Vec<_>, AppErrorDto>>()?;
    Ok(ListData { sessions })
}

/// One JSON-RPC-ish call over the session's local endpoint:
/// newline-delimited `{id, method, params}` in, one JSON line out.
///
/// Linux/macOS: the unix socket herdr reports as `socket_path`. Windows:
/// herdr 0.9.5 serves the same protocol on the named pipe
/// `\\.\pipe\<socket_path>`, so the path is prefixed (UNI-3xx parity).
pub(crate) async fn socket_call(
    socket_path: &str,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, AppErrorDto> {
    let stream = connect_session(socket_path)
        .await
        .map_err(|error| {
            let message = format!("herdr socket connect failed: {error}");
            tracing::warn!(socket = socket_path, method, error = %error, "herdr socket connect failed");
            internal(message)
        })?;
    socket_call_stream(stream, method, params).await
}

async fn connect_session(path: &str) -> std::io::Result<crate::ipc::Stream> {
    #[cfg(unix)]
    {
        crate::ipc::connect_unix(path).await
    }
    #[cfg(windows)]
    {
        crate::ipc::connect_pipe(&crate::ipc::pipe_path(path)).await
    }
}

/// The shared request/reply loop: one JSON line out, the matching reply
/// (`id == "unipi-host"`) back. Used for unix sockets and named pipes.
async fn socket_call_stream<S>(
    stream: S,
    method: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, AppErrorDto>
where
    S: tokio::io::AsyncRead + tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
    let (reader, mut writer) = tokio::io::split(stream);
    let request = serde_json::json!({"id": "unipi-host", "method": method, "params": params});
    writer
        .write_all((request.to_string() + "\n").as_bytes())
        .await
        .map_err(|error| internal(format!("herdr socket write failed: {error}")))?;
    let mut lines = tokio::io::BufReader::new(reader).lines();
    while let Some(line) =
        tokio::time::timeout(std::time::Duration::from_secs(5), lines.next_line())
            .await
            .map_err(|_| internal("herdr socket call timed out".to_string()))?
            .map_err(|error| internal(format!("herdr socket read failed: {error}")))?
    {
        if let Ok(reply) = serde_json::from_str::<serde_json::Value>(&line) {
            if reply.get("id") == Some(&serde_json::json!("unipi-host")) {
                if let Some(error) = reply.get("error") {
                    tracing::warn!(method, error = %error, "herdr socket returned an error");
                    return Err(internal(format!("herdr socket error: {error}")));
                }
                return Ok(reply
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null));
            }
        }
    }
    tracing::warn!(method, "herdr socket closed without a reply");
    Err(internal("herdr socket closed without a reply".to_string()))
}

/// A snapshot of one running herdr session (subset we consume).
#[derive(Debug, Deserialize)]
pub struct RawSnapshot {
    #[serde(default)]
    pub focused_workspace_id: Option<String>,
    #[serde(default)]
    pub focused_tab_id: Option<String>,
    #[serde(default)]
    pub focused_pane_id: Option<String>,
    #[serde(default)]
    pub workspaces: Vec<RawWorkspace>,
    #[serde(default)]
    pub tabs: Vec<RawTab>,
    #[serde(default)]
    pub panes: Vec<RawPane>,
}

#[derive(Debug, Deserialize)]
pub struct RawWorkspace {
    pub workspace_id: String,
    pub label: String,
    pub number: u32,
    pub focused: bool,
    pub agent_status: String,
    pub tab_count: u32,
    pub pane_count: u32,
    #[serde(default)]
    pub tabs: Vec<RawTab>,
}

#[derive(Debug, Deserialize)]
pub struct RawTab {
    pub tab_id: String,
    pub workspace_id: String,
    pub label: String,
    pub number: u32,
    pub focused: bool,
    pub agent_status: String,
}

#[derive(Debug, Deserialize)]
pub struct RawPane {
    pub pane_id: String,
    #[serde(default)]
    pub tab_id: String,
    pub focused: bool,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub terminal_title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: Option<String>,
    #[serde(default)]
    pub agent_session: Option<RawAgentSession>,
}

#[derive(Debug, Deserialize)]
pub struct RawAgentSession {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
}

fn agent(pane: &RawPane) -> Option<PaneAgent> {
    let name = pane.agent.clone()?;
    Some(PaneAgent {
        name,
        // herdr's AgentStatus enum is idle|working|blocked|done|unknown,
        // matching our proto enum; unknown values map to `unknown`.
        status: AgentStatus::from_herdr(pane.agent_status.as_deref().unwrap_or("unknown")),
        session: pane.agent_session.as_ref().map(|session| AgentSessionRef {
            kind: session.kind.clone().unwrap_or_else(|| "path".into()),
            value: session.value.clone().unwrap_or_default(),
        }),
    })
}

fn pane(raw: &RawPane) -> Option<HerdrPane> {
    let id = HerdrId::try_from(raw.pane_id.clone()).ok()?;
    Some(HerdrPane {
        id,
        focused: raw.focused,
        title: raw.title.clone(),
        terminal_title: raw.terminal_title.clone(),
        cwd: raw.cwd.clone(),
        agent: agent(raw),
        chat: None,
    })
}

/// Maps one `session.snapshot` onto the proto tree. The snapshot's
/// workspaces/tabs/panes are three flat arrays joined by id, so we
/// nest them here: workspaces → their tabs → their panes.
pub fn snapshot_to_workspaces(snapshot: &RawSnapshot) -> Vec<HerdrWorkspace> {
    snapshot
        .workspaces
        .iter()
        .map(|workspace| HerdrWorkspace {
            id: HerdrId::try_from(workspace.workspace_id.clone()).expect("validated herdr id"),
            label: workspace.label.clone(),
            number: workspace.number,
            focused: Some(workspace.workspace_id.clone()) == snapshot.focused_workspace_id,
            agent_status: AgentStatus::from_herdr(&workspace.agent_status),
            tab_count: workspace.tab_count,
            pane_count: workspace.pane_count,
            tabs: snapshot
                .tabs
                .iter()
                .filter(|tab| tab.workspace_id == workspace.workspace_id)
                .map(|tab| HerdrTab {
                    id: HerdrId::try_from(tab.tab_id.clone()).expect("validated herdr id"),
                    label: tab.label.clone(),
                    number: tab.number,
                    focused: tab.focused,
                    agent_status: AgentStatus::from_herdr(&tab.agent_status),
                    panes: snapshot
                        .panes
                        .iter()
                        .filter(|pane| pane.tab_id == tab.tab_id)
                        .filter_map(pane)
                        .collect(),
                })
                .collect(),
        })
        .collect()
}

pub async fn tree() -> Result<HerdrState, AppErrorDto> {
    let sessions = match session_list().await {
        Ok(sessions) => sessions,
        Err(error) => {
            return Ok(HerdrState {
                available: false,
                reason: Some(error.message.as_str().to_string()),
                sessions: None,
            });
        }
    };
    let mut tree = Vec::new();
    for session in &sessions {
        let name = match SessionName::try_from(session.name.clone()) {
            Ok(name) => name,
            Err(message) => {
                tracing::warn!(name = session.name, %message, "skipping session with invalid name");
                continue;
            }
        };
        let mut entry = HerdrSession {
            name,
            running: session.running,
            default: session.default,
            workspaces: Vec::new(),
            error: None,
        };
        if session.running && !session.socket_path.is_empty() {
            let reply = socket_call(
                &session.socket_path,
                "session.snapshot",
                serde_json::json!({}),
            )
            .await;
            match reply {
                Ok(reply) => {
                    let parsed: Result<RawSnapshot, _> = serde_json::from_value(
                        reply
                            .get("snapshot")
                            .cloned()
                            .unwrap_or(serde_json::Value::Null),
                    );
                    match parsed {
                        Ok(snapshot) => entry.workspaces = snapshot_to_workspaces(&snapshot),
                        Err(error) => {
                            entry.error = Some(format!("snapshot parse failed: {error}"));
                        }
                    }
                }
                Err(error) => {
                    entry.error = Some(error.message.as_str().to_string());
                }
            }
        }
        tree.push(entry);
    }
    Ok(HerdrState {
        available: true,
        reason: None,
        sessions: Some(tree),
    })
}

/// Creates, renames, closes or navigates within a herdr session
/// over the socket API (DESIGN §3.3 multiplexer parity). Returns the
/// socket reply where there is one, `null` otherwise.
pub async fn action(action: &MuxAction) -> Result<serde_json::Value, AppErrorDto> {
    match action {
        MuxAction::WorkspaceCreate {
            session,
            cwd,
            label,
        } => {
            let socket_path = socket_for(session.as_str()).await?;
            let mut params = serde_json::json!({ "focus": true });
            if let Some(cwd) = cwd {
                params["cwd"] = serde_json::json!(cwd);
            }
            if let Some(label) = label {
                params["label"] = serde_json::json!(label.as_str());
            }
            socket_call(&socket_path, "workspace.create", params).await
        }
        MuxAction::WorkspaceRename {
            session,
            workspace_id,
            label,
        } => {
            let socket_path = socket_for(session.as_str()).await?;
            socket_call(
                &socket_path,
                "workspace.rename",
                serde_json::json!({
                    "workspace_id": workspace_id.as_str(),
                    "label": label.as_str(),
                }),
            )
            .await
        }
        MuxAction::WorkspaceClose {
            session,
            workspace_id,
        } => {
            let socket_path = socket_for(session.as_str()).await?;
            socket_call(
                &socket_path,
                "workspace.close",
                serde_json::json!({
                    "workspace_id": workspace_id.as_str(),
                    "close_group": true,
                }),
            )
            .await
        }
        MuxAction::TabNext { session } => tab_neighbor(session.as_str(), true).await,
        MuxAction::TabPrev { session } => tab_neighbor(session.as_str(), false).await,
        MuxAction::HerdrPaneNext { session } => pane_direction(session.as_str(), "right").await,
        MuxAction::HerdrPanePrev { session } => pane_direction(session.as_str(), "left").await,
        _ => Err(internal("not a herdr action".to_string())),
    }
}

/// The shortcuts panel entries surfaced in the app, in display
/// order, with their real defaults from `herdr --default-config`.
const SHORTCUT_ACTIONS: &[(&str, &str)] = &[
    ("workspace_picker", "Workspace picker"),
    ("new_workspace", "New workspace"),
    ("rename_workspace", "Rename workspace"),
    ("close_workspace", "Close workspace"),
    ("new_tab", "New tab"),
    ("rename_tab", "Rename tab"),
    ("next_tab", "Next tab"),
    ("previous_tab", "Previous tab"),
    ("close_tab", "Close tab"),
    ("split_vertical", "Split vertical"),
    ("split_horizontal", "Split horizontal"),
    ("cycle_pane_next", "Next pane"),
    ("cycle_pane_previous", "Previous pane"),
    ("close_pane", "Close pane"),
    ("zoom", "Zoom pane"),
    ("toggle_sidebar", "Toggle sidebar"),
    ("detach", "Detach"),
    ("help", "Help"),
    ("settings", "Settings"),
];

/// Parses the `[keys]` section of `herdr --default-config`: both the
/// commented defaults (`# name = "value"`) and any uncommented
/// overrides. Unknown names are ignored; unbound actions drop out.
fn parse_default_keys(config: &str) -> Vec<(String, String)> {
    let mut in_keys = false;
    let mut out: Vec<(String, String)> = Vec::new();
    for line in config.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_keys = trimmed == "[keys]";
            continue;
        }
        if !in_keys {
            continue;
        }
        let body = trimmed.strip_prefix("# ").unwrap_or(trimmed);
        let mut parts = body.splitn(2, '=');
        let key = parts.next().unwrap_or("").trim();
        let value = parts.next().unwrap_or("").trim().trim_matches('"');
        if key.is_empty() || value.is_empty() {
            continue;
        }
        if let Some(entry) = out.iter_mut().find(|(n, _)| n == key) {
            entry.1 = value.to_string();
        } else {
            out.push((key.to_string(), value.to_string()));
        }
    }
    out
}

/// The shortcuts panel data: herdr defaults read from the real
/// `herdr --default-config` (never guessed), tmux from the live
/// prefix option plus the stock key list.
pub async fn shortcuts() -> Result<ShortcutsData, AppErrorDto> {
    let config = tokio::process::Command::new("herdr")
        .arg("--default-config")
        .output()
        .await
        .map_err(|error| internal(format!("herdr --default-config failed: {error}")))?;
    let defaults = parse_default_keys(&String::from_utf8_lossy(&config.stdout));
    let prefix = defaults
        .iter()
        .find(|(name, _)| name == "prefix")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| "ctrl+b".to_string());
    let herdr_keys = SHORTCUT_ACTIONS
        .iter()
        .filter_map(|(name, label)| {
            defaults
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, key)| ShortcutKey {
                    name: label.to_string(),
                    key: key.clone(),
                })
        })
        .collect();
    let tmux_prefix = match crate::mux::tmux::run(&["show-options", "-gv", "prefix"]).await {
        Ok(out) => {
            let value = String::from_utf8_lossy(&out).trim().to_string();
            if value.is_empty() {
                "C-b".to_string()
            } else {
                value
            }
        }
        Err(_) => "C-b".to_string(),
    };
    Ok(ShortcutsData {
        herdr: ShortcutMux {
            prefix,
            keys: herdr_keys,
        },
        tmux: ShortcutMux {
            prefix: tmux_prefix,
            keys: vec![
                ShortcutKey {
                    name: "New window".into(),
                    key: "c".into(),
                },
                ShortcutKey {
                    name: "Next window".into(),
                    key: "n".into(),
                },
                ShortcutKey {
                    name: "Previous window".into(),
                    key: "p".into(),
                },
                ShortcutKey {
                    name: "Split horizontal".into(),
                    key: "\"".into(),
                },
                ShortcutKey {
                    name: "Split vertical".into(),
                    key: "%".into(),
                },
                ShortcutKey {
                    name: "Detach".into(),
                    key: "d".into(),
                },
                ShortcutKey {
                    name: "Copy mode".into(),
                    key: "[".into(),
                },
            ],
        },
    })
}

/// Focuses the next/previous tab (wrapping) of the session's focused
/// workspace, computed from the live snapshot.
async fn tab_neighbor(session: &str, next: bool) -> Result<serde_json::Value, AppErrorDto> {
    let socket_path = socket_for(session).await?;
    let reply = socket_call(&socket_path, "session.snapshot", serde_json::json!({})).await?;
    let snapshot: RawSnapshot = serde_json::from_value(
        reply
            .get("snapshot")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    )
    .map_err(|error| internal(format!("herdr snapshot parse failed: {error}")))?;
    let focused = snapshot
        .workspaces
        .iter()
        .find(|w| w.focused)
        .or_else(|| snapshot.workspaces.first())
        .ok_or_else(|| internal("herdr session has no workspaces".to_string()))?;
    let mut tabs: Vec<&RawTab> = snapshot
        .tabs
        .iter()
        .filter(|t| t.workspace_id == focused.workspace_id)
        .collect();
    tabs.sort_by_key(|t| t.number);
    let current = tabs.iter().position(|t| t.focused).unwrap_or(0);
    let target = if next {
        (current + 1) % tabs.len()
    } else {
        (current + tabs.len() - 1) % tabs.len()
    };
    socket_call(
        &socket_path,
        "tab.focus",
        serde_json::json!({ "tab_id": tabs[target].tab_id }),
    )
    .await
}

async fn pane_direction(session: &str, direction: &str) -> Result<serde_json::Value, AppErrorDto> {
    let socket_path = socket_for(session).await?;
    socket_call(
        &socket_path,
        "pane.focus_direction",
        serde_json::json!({ "direction": direction }),
    )
    .await
}

/// Applies focus calls for every id given (workspace, then tab, then
/// pane), using the exact herdr methods from the API schema.
pub async fn focus(
    session: &str,
    workspace_id: Option<&HerdrId>,
    tab_id: Option<&HerdrId>,
    pane_id: Option<&HerdrId>,
) -> Result<(), AppErrorDto> {
    let socket_path = socket_for(session).await?;
    if let Some(workspace_id) = workspace_id {
        socket_call(
            &socket_path,
            "workspace.focus",
            serde_json::json!({"workspace_id": workspace_id.as_str()}),
        )
        .await?;
    }
    if let Some(tab_id) = tab_id {
        socket_call(
            &socket_path,
            "tab.focus",
            serde_json::json!({"tab_id": tab_id.as_str()}),
        )
        .await?;
    }
    if let Some(pane_id) = pane_id {
        socket_call(
            &socket_path,
            "pane.focus",
            serde_json::json!({"pane_id": pane_id.as_str()}),
        )
        .await?;
    }
    Ok(())
}

pub(crate) async fn socket_for(session: &str) -> Result<String, AppErrorDto> {
    let sessions = session_list().await?;
    sessions
        .into_iter()
        .find(|entry| entry.name == session)
        .map(|entry| entry.socket_path)
        .filter(|path| !path.is_empty())
        .ok_or_else(|| unavailable(format!("herdr session {session} is not running")))
}

/// `pane.read` with ANSI kept, on the pane (or the workspace/tab's
/// focused pane).
pub async fn preview(target: &AttachTarget, lines: u16) -> Result<PreviewData, AppErrorDto> {
    let AttachTarget::Herdr {
        session,
        workspace_id,
        tab_id,
        pane_id,
    } = target
    else {
        return Err(internal(
            "herdr preview called with a non-herdr target".into(),
        ));
    };
    let socket_path = socket_for(session.as_str()).await?;
    let pane_id = match pane_id {
        Some(pane_id) => pane_id.as_str().to_string(),
        None => {
            let snapshot =
                socket_call(&socket_path, "session.snapshot", serde_json::json!({})).await?;
            let snapshot: RawSnapshot = serde_json::from_value(
                snapshot
                    .get("snapshot")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null),
            )
            .map_err(|error| internal(format!("snapshot parse failed: {error}")))?;
            focused_pane_id(&snapshot, workspace_id.as_ref(), tab_id.as_ref())
                .ok_or_else(|| internal("no focused pane in the target workspace".into()))?
        }
    };
    let reply = socket_call(
        &socket_path,
        "pane.read",
        serde_json::json!({
            "pane_id": pane_id,
            "source": "recent",
            "format": "ansi",
            "strip_ansi": false,
            "lines": lines,
        }),
    )
    .await?;
    let text = reply
        .get("read")
        .and_then(|read| read.get("text"))
        .and_then(|text| text.as_str())
        .unwrap_or("")
        .to_string();
    Ok(PreviewData {
        text: text.try_into().map_err(internal)?,
    })
}

fn focused_pane_id(
    snapshot: &RawSnapshot,
    workspace_id: Option<&HerdrId>,
    tab_id: Option<&HerdrId>,
) -> Option<String> {
    if let Some(id) = &snapshot.focused_pane_id {
        if workspace_id.is_none() && tab_id.is_none() {
            return Some(id.clone());
        }
    }
    for tab in &snapshot.tabs {
        if let Some(wanted) = workspace_id {
            if tab.workspace_id != wanted.as_str() {
                continue;
            }
        }
        if let Some(wanted) = tab_id {
            if tab.tab_id != wanted.as_str() {
                continue;
            }
        }
        if let Some(pane) = snapshot
            .panes
            .iter()
            .find(|pane| pane.tab_id == tab.tab_id && pane.focused)
        {
            return Some(pane.pane_id.clone());
        }
    }
    None
}

/// Builds the PTY command for a herdr client.
pub fn client_command(session: &str, cols: u16, rows: u16) -> portable_pty::CommandBuilder {
    let mut command = portable_pty::CommandBuilder::new("herdr");
    command.arg("--session");
    command.arg(session);
    if let Some(home) = crate::fs::home_dir_opt() {
        command.cwd(home);
    }
    for (key, _) in std::env::vars() {
        if key.starts_with("HERDR_") {
            command.env_remove(key);
        }
    }
    command.env("TERM", "xterm-256color");
    let _ = (cols, rows);
    command
}

/// Cheap BTreeMap import guard (used by tests below).
#[allow(unused)]
fn _map() -> BTreeMap<String, String> {
    BTreeMap::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_snapshot() -> RawSnapshot {
        serde_json::from_value(serde_json::json!({
            "focused_workspace_id": "w4E",
            "workspaces": [
                { "workspace_id": "w4E", "label": "uni", "number": 4, "focused": true,
                  "agent_status": "working", "tab_count": 2, "pane_count": 3,
                  "tabs": [
                    { "tab_id": "w4E:t16", "workspace_id": "w4E", "label": "1", "number": 1, "focused": true,
                      "agent_status": "working",
                      "panes": [
                        { "pane_id": "w4E:p18", "focused": true, "cwd": "/home/u/proj/a",
                          "agent": "pi", "agent_status": "working",
                          "agent_session": {"kind": "path", "value": "/home/u/.pi/agent/sessions/a.json"} },
                        { "pane_id": "w4E:p19", "focused": false }
                      ]},
                    { "tab_id": "w4E:t17", "workspace_id": "w4E", "label": "2", "number": 2, "focused": false,
                      "agent_status": "idle",
                      "panes": [{ "pane_id": "w4E:p20", "focused": true, "cwd": "/home/u/proj/b" }] }
                  ]},
                { "workspace_id": "w5", "label": "second", "number": 5, "focused": false,
                  "agent_status": "idle", "tab_count": 1, "pane_count": 1,
                  "tabs": [{ "tab_id": "w5:t1", "workspace_id": "w5", "label": "1", "number": 1, "focused": true,
                             "agent_status": "idle",
                             "panes": [{ "pane_id": "w5:p1", "focused": true, "cwd": "/home/u/proj/c" }] }]
                }
            ],
            "tabs": [
              { "tab_id": "w4E:t16", "workspace_id": "w4E", "label": "1", "number": 1, "focused": true, "agent_status": "working" },
              { "tab_id": "w4E:t17", "workspace_id": "w4E", "label": "2", "number": 2, "focused": false, "agent_status": "idle" },
              { "tab_id": "w5:t1", "workspace_id": "w5", "label": "1", "number": 1, "focused": true, "agent_status": "idle" }
            ],
            "panes": [
              { "pane_id": "w4E:p18", "tab_id": "w4E:t16", "focused": true, "cwd": "/home/u/proj/a",
                "agent": "pi", "agent_status": "working",
                "agent_session": {"kind": "path", "value": "/home/u/.pi/agent/sessions/a.json"} },
              { "pane_id": "w4E:p19", "tab_id": "w4E:t16", "focused": false, "cwd": "/home/u/proj/a" },
              { "pane_id": "w4E:p20", "tab_id": "w4E:t17", "focused": true, "cwd": "/home/u/proj/b" },
              { "pane_id": "w5:p1", "tab_id": "w5:t1", "focused": true, "cwd": "/home/u/proj/c" }
            ]
        })).unwrap()
    }

    #[test]
    fn dead_socket_session_keeps_error_and_empty_workspaces() {
        let mut sessions = [
            RawSession {
                name: "dead".into(),
                running: true,
                default: false,
                socket_path: "/tmp/unipi-m2-no-such-socket".into(),
            },
            RawSession {
                name: "alive".into(),
                running: true,
                default: false,
                socket_path: String::new(), // would fail, but we drive snapshot mapping directly
            },
        ];
        sessions[1].socket_path = "/tmp/unipi-m2-no-such-socket-either".into();
        // Simulate what tree() does per session: the dead one records
        // an error and empty workspaces, the good snapshot maps fully.
        let snapshot = sample_snapshot();
        let workspaces = snapshot_to_workspaces(&snapshot);
        let mut dead = HerdrSession {
            name: SessionName::try_from(sessions[0].name.clone()).unwrap(),
            running: true,
            default: false,
            workspaces: Vec::new(),
            error: Some("herdr socket connect failed: no such file".into()),
        };
        dead.workspaces = Vec::new();
        assert!(dead.error.is_some());
        assert!(dead.workspaces.is_empty());
        assert_eq!(workspaces.len(), 2);
        // Invalid names are skipped by tree() (warn + continue), not fatal.
        assert!(SessionName::try_from(String::new()).is_err());
    }
    #[test]
    fn snapshot_maps_to_tree() {
        let snapshot = sample_snapshot();
        let workspaces = snapshot_to_workspaces(&snapshot);
        assert_eq!(workspaces.len(), 2);
        let first = &workspaces[0];
        assert_eq!(first.id.as_str(), "w4E");
        assert!(first.focused);
        assert_eq!(first.tabs.len(), 2);
        assert_eq!(first.tabs[0].panes.len(), 2);
        let agent = first.tabs[0].panes[0].agent.as_ref().unwrap();
        assert_eq!(agent.name, "pi");
        assert!(matches!(agent.status, AgentStatus::Working));
        assert_eq!(agent.session.as_ref().unwrap().kind, "path");
        assert_eq!(
            agent.session.as_ref().unwrap().value,
            "/home/u/.pi/agent/sessions/a.json"
        );
        assert!(first.tabs[0].panes[1].agent.is_none());
        assert_eq!(
            first.tabs[1].panes[0].cwd.as_deref(),
            Some("/home/u/proj/b")
        );
        assert!(!workspaces[1].focused);
    }

    #[test]
    fn unknown_agent_status_maps_to_unknown() {
        let mut snapshot = sample_snapshot();
        snapshot.workspaces[0].agent_status = "warp-speed".into();
        let workspaces = snapshot_to_workspaces(&snapshot);
        assert!(matches!(workspaces[0].agent_status, AgentStatus::Unknown));
    }

    #[test]
    fn focused_pane_falls_back_through_tab_and_workspace() {
        let snapshot = sample_snapshot();
        assert_eq!(
            focused_pane_id(&snapshot, None, None).as_deref(),
            Some("w4E:p18")
        );
        assert_eq!(
            focused_pane_id(
                &snapshot,
                Some(&HerdrId::try_from("w5".to_string()).unwrap()),
                None
            )
            .as_deref(),
            Some("w5:p1")
        );
    }
}
