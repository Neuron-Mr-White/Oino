use crate::{
    host_store::{self, HostInfo},
    net::{parse_pair_link, parse_ticket, selected_path, NetState},
    path_kind::PathKind,
};
use serde::Serialize;
use tauri::{Manager, State};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use unipi_proto::{
    frame::{FrameKind, MAX_FRAME},
    AppErrorDto, Control, ProtoError, Request, PROTO_VERSION,
};
fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}
async fn op(
    state: &NetState,
    dir: &std::path::Path,
    relay: Option<&str>,
    ticket: &str,
    req: Request,
) -> Result<serde_json::Value, AppErrorDto> {
    let result = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let (mut send, mut recv) = state.open_op(dir, relay, ticket).await?;
        let hello = Control::Hello {
            proto: PROTO_VERSION
                .to_string()
                .try_into()
                .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
            req,
        };
        let bytes = serde_json::to_vec(&hello)
            .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
        send.write_all(&unipi_proto::frame::encode(FrameKind::Control, &bytes))
            .await
            .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
        send.finish()
            .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
        let kind = recv
            .read_u8()
            .await
            .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
        let len = recv
            .read_u32()
            .await
            .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?
            as usize;
        if kind != 0 || len > MAX_FRAME {
            return Err(ProtoError::Internal("Invalid reply frame".into()).to_app_error());
        }
        let mut bytes = vec![0; len];
        recv.read_exact(&mut bytes)
            .await
            .map_err(|e| ProtoError::Transport(e.to_string()).to_app_error())?;
        match serde_json::from_slice::<Control>(&bytes)
            .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?
        {
            Control::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
            Control::Err { error } => Err(error),
            _ => Err(ProtoError::Internal("Unexpected reply".into()).to_app_error()),
        }
    })
    .await;
    result.map_err(|_| ProtoError::Timeout("Host operation timed out".into()).to_app_error())?
}
async fn cached_op(
    state: &NetState,
    dir: &std::path::Path,
    ticket: &str,
    relay: Option<&str>,
    req: Request,
) -> Result<(serde_json::Value, PathKind), AppErrorDto> {
    let id = parse_ticket(ticket)?.endpoint_addr().id.to_string();
    crate::retry::retry_once(|fresh| {
        let req = req.clone();
        let id = id.clone();
        async move {
            if fresh {
                state.drop_connection(&id);
            }
            let value = op(state, dir, relay, ticket, req).await?;
            Ok((value, state.path_for(ticket)))
        }
    })
    .await
}
#[tauri::command]
pub async fn pair_host(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    link: String,
    device_name: String,
    relay: Option<String>,
) -> Result<HostInfo, AppErrorDto> {
    let parsed = parse_pair_link(&link)?;
    let ticket = parse_ticket(&parsed.ticket)?;
    let dir = directory(&app)?;
    let req = Request::Pair {
        code: parsed
            .code
            .try_into()
            .map_err(|e: String| ProtoError::PairingInvalid(e).to_app_error())?,
        name: Some(
            device_name
                .try_into()
                .map_err(|e: String| ProtoError::PairingInvalid(e).to_app_error())?,
        ),
    };
    let (value, conn) = cached_op(
        &state,
        &dir,
        &parsed.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        req,
    )
    .await?;
    let data: unipi_proto::PairData = serde_json::from_value(value)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    if data.host_id.as_str() != ticket.endpoint_addr().id.to_string() {
        return Err(ProtoError::PairingInvalid("Host identity mismatch".into()).to_app_error());
    }
    let host = HostInfo {
        id: data.host_id.as_str().into(),
        name: data.host_name.as_str().into(),
        ticket: parsed.ticket,
        paired_at: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0),
        last_path: Some(conn),
        transport: None,
    };
    host_store::upsert_host(&dir, host.clone())?;
    Ok(host)
}
#[tauri::command]
pub fn list_hosts(app: tauri::AppHandle) -> Result<Vec<HostInfo>, AppErrorDto> {
    crate::local_host::hosts(&directory(&app)?)
}
#[tauri::command]
pub fn forget_host(app: tauri::AppHandle, id: String) -> Result<(), AppErrorDto> {
    host_store::remove_host(&directory(&app)?, &id)?;
    Ok(())
}
#[derive(Serialize)]
pub struct SessionsResult {
    sessions: Vec<unipi_proto::SessionSummary>,
    path: PathKind,
}
#[derive(Serialize)]
pub struct HostStatus {
    path: PathKind,
    rtt_ms: Option<u64>,
}
async fn connection(
    app: &tauri::AppHandle,
    state: &NetState,
    id: &str,
    relay: Option<&str>,
) -> Result<iroh::endpoint::Connection, AppErrorDto> {
    let dir = directory(app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|h| h.id == id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    state
        .connection_for(&dir, relay.filter(|s| *s != "n0"), &host.ticket)
        .await
}
#[tauri::command]
pub async fn list_sessions(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    mux: unipi_proto::MuxKind,
    relay: Option<String>,
) -> Result<SessionsResult, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let (reply, conn) = cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::List { mux },
    )
    .await?;
    let data: unipi_proto::ListData = serde_json::from_value(reply)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    let path = conn;
    host_store::record_last_path(&directory(&app)?, &host_id, path)?;
    Ok(SessionsResult {
        sessions: data.sessions,
        path,
    })
}
#[derive(Serialize)]
pub struct MuxTreeResult {
    tree: unipi_proto::TreeData,
    path: PathKind,
}
#[tauri::command]
pub async fn mux_tree(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
) -> Result<MuxTreeResult, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let (reply, conn) = cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::Tree,
    )
    .await?;
    let tree: unipi_proto::TreeData = serde_json::from_value(reply)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    let path = conn;
    host_store::record_last_path(&directory(&app)?, &host_id, path)?;
    Ok(MuxTreeResult { tree, path })
}
/// Applies a multiplexer mutation/navigation on the host. The action
/// arrives as the flat wire object (`action` tag + payload) and is
/// schema-validated here before it reaches the protocol.
#[tauri::command]
pub async fn mux_action(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
    action: serde_json::Value,
) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let parsed: unipi_proto::MuxAction = serde_json::from_value(action)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    let (reply, _conn) = cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::MuxAction { action: parsed },
    )
    .await?;
    Ok(reply)
}

/// The shortcuts panel data: herdr defaults read from the host's
/// `herdr --default-config`, tmux prefix read live.
#[tauri::command]
pub async fn shortcuts(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
) -> Result<unipi_proto::ShortcutsData, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let (reply, _conn) = cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::Shortcuts,
    )
    .await?;
    serde_json::from_value(reply).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

#[derive(Serialize)]
pub struct MuxPreview {
    text: String,
}
#[tauri::command]
pub async fn mux_preview(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
    target: unipi_proto::AttachTarget,
    lines: u16,
) -> Result<MuxPreview, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let (reply, _conn) = cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::Preview {
            target,
            lines: unipi_proto::Lines::try_from(lines)
                .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
        },
    )
    .await?;
    let data: unipi_proto::PreviewData = serde_json::from_value(reply)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    Ok(MuxPreview {
        text: data.text.into_inner(),
    })
}
#[tauri::command]
pub async fn host_status(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
) -> Result<HostStatus, AppErrorDto> {
    // UNI-231: this PC's own host: the local socket answers → Local, no dial.
    if cfg!(unix) && crate::local_host::socket_for(&host_id).is_some() {
        let dir = directory(&app)?;
        let ticket = crate::local_host::hosts(&dir)?.into_iter().find(|h| h.id == host_id).map(|h| h.ticket);
        if let Some(ticket) = ticket {
            let started = std::time::Instant::now();
            if op(&state, &dir, None, &ticket, Request::FsRoots).await.is_ok() {
                return Ok(HostStatus { path: PathKind::Local, rtt_ms: Some(started.elapsed().as_millis() as u64) });
            }
        }
    }
    let conn = crate::retry::retry_once(|fresh| {
        let app = &app;
        let state = &state;
        let host_id = &host_id;
        let relay = &relay;
        async move {
            if fresh {
                state.drop_connection(host_id);
            }
            connection(app, state, host_id, relay.as_deref()).await
        }
    })
    .await?;
    let (path, rtt) = selected_path(&conn);
    if let Some(selected) = conn.paths().iter().find(|path| path.is_selected()) {
        let _ = std::fs::write(
            directory(&app)?.join("path-diagnostic.txt"),
            format!(
                "selected={:?} classified={:?}",
                selected.remote_addr(),
                path
            ),
        );
    }
    Ok(HostStatus {
        path,
        rtt_ms: rtt.map(|r| r.as_millis() as u64),
    })
}

#[derive(serde::Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case")]
pub enum RelayConfig {
    N0,
    Custom { url: String },
}
#[tauri::command]
pub async fn set_relay(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    cfg: RelayConfig,
) -> Result<(), AppErrorDto> {
    let relay = match cfg {
        RelayConfig::N0 => None,
        RelayConfig::Custom { url } => {
            if !url.starts_with("https://") {
                return Err(ProtoError::Settings("Relay URL must be HTTPS".into()).to_app_error());
            }
            Some(url)
        }
    };
    state.endpoint(&directory(&app)?, relay.as_deref()).await?;
    Ok(())
}

#[derive(Serialize)]
pub struct PairPreview {
    id: String,
}
#[tauri::command]
pub fn pairing_preview(link: String) -> Result<PairPreview, AppErrorDto> {
    let parsed = parse_pair_link(&link)?;
    let ticket = parse_ticket(&parsed.ticket)?;
    Ok(PairPreview {
        id: ticket.endpoint_addr().id.to_string(),
    })
}

/// UNI-161 (Get from this PC): asks the connected host for its own
/// ntfy server/topic/token plus the per-pairing E2E key.
#[tauri::command]
pub async fn notify_config(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
) -> Result<unipi_proto::NotifyConfigData, AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let (reply, _conn) = cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::NotifyConfig,
    )
    .await?;
    serde_json::from_value(reply).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

/// UNI-161: registers this phone's UnifiedPush endpoint with the host.
#[tauri::command]
pub async fn notify_register(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
    endpoint: String,
) -> Result<(), AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let endpoint: unipi_proto::NotifyEndpoint = endpoint
        .try_into()
        .map_err(|e: String| ProtoError::Internal(e).to_app_error())?;
    cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::NotifyRegister { endpoint },
    )
    .await?;
    Ok(())
}

/// UNI-161 (Test): runs the same encrypt-then-publish path on the host,
/// proving the whole chain without a live pi session.
#[tauri::command]
pub async fn notify_test(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    relay: Option<String>,
) -> Result<(), AppErrorDto> {
    let dir = directory(&app)?;
    let host = crate::local_host::hosts(&dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    cached_op(
        &state,
        &dir,
        &host.ticket,
        relay.as_deref().filter(|s| *s != "n0"),
        Request::NotifyTest,
    )
    .await?;
    Ok(())
}
