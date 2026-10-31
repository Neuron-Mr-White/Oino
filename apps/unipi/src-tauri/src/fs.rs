//! Filesystem browser/editor commands (UNI-155 "files — browser +
//! editor"): the UI's `src/lib/fs.ts` calls these, which forward
//! `fs_*` requests to the paired PC's `unipi-host` over the shared
//! iroh connection (same seam as `board.rs`/`blob.rs`).

use std::path::Path;
use std::time::Duration;

use serde::Serialize;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, Control, FsListData, FsReadData, FsRootsData, FsSearchData,
    FsStatData, FsWriteData, PathSegment, ProtoError, Request, SearchKind, SearchQuery, Sha256Hex,
    PROTO_VERSION,
};

use crate::net::NetState;

pub(crate) fn schema_err(field: &str, message: impl Into<String>) -> AppErrorDto {
    schema(field, message)
}

fn schema(field: &str, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
        .with_path(vec![PathSegment::Key(field.into())])
}

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

fn directory(app: &tauri::AppHandle) -> Result<std::path::PathBuf, AppErrorDto> {
    use tauri::Manager;
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

fn host_ticket(dir: &Path, host_id: &str) -> Result<String, AppErrorDto> {
    let host = crate::local_host::hosts(dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    Ok(host.ticket)
}

async fn open_stream(
    state: &NetState,
    dir: &Path,
    ticket: &str,
) -> Result<(crate::op_stream::OpSend, crate::op_stream::OpRecv), AppErrorDto> {
    let relay = state.current_relay().await;
    state.open_op(dir, relay.as_deref(), ticket).await
}

fn hello_frame(request: Request) -> Result<Vec<u8>, AppErrorDto> {
    let hello = Control::Hello {
        proto: PROTO_VERSION
            .to_string()
            .try_into()
            .map_err(|e: String| ProtoError::Internal(e).to_app_error())?,
        req: request,
    };
    let bytes = serde_json::to_vec(&hello)
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    Ok(encode(FrameKind::Control, &bytes))
}

async fn read_frame<R: AsyncRead + Unpin>(
    recv: &mut R,
    timeout: Option<Duration>,
) -> Result<Option<(u8, Vec<u8>)>, AppErrorDto> {
    let read = async {
        let kind = match recv.read_u8().await {
            Ok(kind) => kind,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(transport(error)),
        };
        let len = recv.read_u32().await.map_err(transport)? as usize;
        if len > MAX_FRAME {
            return Err(schema("frame", "oversize frame from host"));
        }
        let mut payload = vec![0u8; len];
        recv.read_exact(&mut payload).await.map_err(transport)?;
        Ok(Some((kind, payload)))
    };
    match timeout {
        Some(limit) => tokio::time::timeout(limit, read)
            .await
            .map_err(|_| ProtoError::Timeout("The PC stopped answering".into()).to_app_error())?,
        None => read.await,
    }
}

fn parse_control(payload: &[u8]) -> Result<Control, AppErrorDto> {
    serde_json::from_slice(payload).map_err(|e| schema("control", e.to_string()))
}

const FRAME_TIMEOUT: Duration = Duration::from_secs(30);

/// Runs one request/reply op (no streaming body): sends `hello`, closes
/// the send side, reads one control reply.
pub(crate) async fn simple_op(
    app: &tauri::AppHandle,
    state: &NetState,
    host_id: &str,
    request: Request,
) -> Result<serde_json::Value, AppErrorDto> {
    simple_op_within(app, state, host_id, request, FRAME_TIMEOUT).await
}

pub(crate) async fn simple_op_within(
    app: &tauri::AppHandle,
    state: &NetState,
    host_id: &str,
    request: Request,
    limit: Duration,
) -> Result<serde_json::Value, AppErrorDto> {
    let dir = directory(app)?;
    let ticket = host_ticket(&dir, host_id)?;
    let (mut send, mut recv) = open_stream(state, &dir, &ticket).await?;
    send.write_all(&hello_frame(request)?)
        .await
        .map_err(transport)?;
    let _ = send.shutdown().await;
    let (kind, payload) = read_frame(&mut recv, Some(limit))
        .await?
        .ok_or_else(|| transport("The PC closed the connection without a reply"))?;
    if kind != 0 {
        return Err(schema("frame", "expected a control reply"));
    }
    match parse_control(&payload)? {
        Control::Ok { data } => Ok(data.unwrap_or(serde_json::Value::Null)),
        Control::Err { error } => Err(error),
        _ => Err(schema("type", "unexpected reply")),
    }
}

/// UNI-217: one `git` request for the repo containing `path`; `cmd` is the
/// wire `GitCmd` (`{action, …}`), validated here. The reply data goes back
/// untouched (the UI parses it per action with zod). Network actions get
/// the host's 120 s budget plus slack.
#[tauri::command]
pub async fn git_op(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    path: String,
    outside: Option<bool>,
    cmd: serde_json::Value,
) -> Result<serde_json::Value, AppErrorDto> {
    let cmd: unipi_proto::GitCmd =
        serde_json::from_value(cmd).map_err(|e| schema("cmd", e.to_string()))?;
    let limit = if cmd.is_network() {
        Duration::from_secs(135)
    } else {
        Duration::from_secs(75)
    };
    let request = Request::Git {
        path: path.try_into().map_err(|e: String| schema("path", e))?,
        outside,
        cmd,
    };
    simple_op_within(&app, &state, &host_id, request, limit).await
}

/// `git_watch{root, on}` (v2 app.md §3.14, WP-A15): refcount the repo's fs
/// watcher on the host; `git_changed` hub events flow on `events_open`.
#[tauri::command]
pub async fn git_watch(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    root: String,
    on: Option<bool>,
) -> Result<serde_json::Value, AppErrorDto> {
    let request = Request::GitWatch {
        root: root.try_into().map_err(|e: String| schema("root", e))?,
        on: on.unwrap_or(true),
    };
    simple_op_within(&app, &state, &host_id, request, Duration::from_secs(30)).await
}

#[tauri::command]
pub async fn fs_roots(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
) -> Result<FsRootsData, AppErrorDto> {
    let data = simple_op(&app, &state, &host_id, Request::FsRoots).await?;
    serde_json::from_value(data).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn fs_list(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    path: String,
    hidden: Option<bool>,
    outside: Option<bool>,
) -> Result<FsListData, AppErrorDto> {
    let request = Request::FsList {
        path: path.try_into().map_err(|e: String| schema("path", e))?,
        hidden,
        outside,
    };
    let data = simple_op(&app, &state, &host_id, request).await?;
    serde_json::from_value(data).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn fs_stat(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    path: String,
    outside: Option<bool>,
) -> Result<FsStatData, AppErrorDto> {
    let request = Request::FsStat {
        path: path.try_into().map_err(|e: String| schema("path", e))?,
        outside,
    };
    let data = simple_op(&app, &state, &host_id, request).await?;
    serde_json::from_value(data).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

#[tauri::command]
pub async fn fs_mkdir(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    path: String,
    outside: Option<bool>,
) -> Result<(), AppErrorDto> {
    let request = Request::FsMkdir {
        path: path.try_into().map_err(|e: String| schema("path", e))?,
        outside,
    };
    simple_op(&app, &state, &host_id, request).await?;
    Ok(())
}

#[tauri::command]
pub async fn fs_rename(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    from: String,
    to: String,
    outside: Option<bool>,
) -> Result<(), AppErrorDto> {
    let request = Request::FsRename {
        from: from.try_into().map_err(|e: String| schema("from", e))?,
        to: to.try_into().map_err(|e: String| schema("to", e))?,
        outside,
    };
    simple_op(&app, &state, &host_id, request).await?;
    Ok(())
}

#[tauri::command]
pub async fn fs_delete(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    path: String,
    confirm: bool,
    outside: Option<bool>,
) -> Result<(), AppErrorDto> {
    let request = Request::FsDelete {
        path: path.try_into().map_err(|e: String| schema("path", e))?,
        confirm,
        outside,
    };
    simple_op(&app, &state, &host_id, request).await?;
    Ok(())
}

#[tauri::command]
pub async fn fs_search(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    root: String,
    query: String,
    kind: String,
    outside: Option<bool>,
) -> Result<FsSearchData, AppErrorDto> {
    let kind = match kind.as_str() {
        "name" => SearchKind::Name,
        "content" => SearchKind::Content,
        other => return Err(schema("kind", format!("unknown search kind: {other}"))),
    };
    let request = Request::FsSearch {
        root: root.try_into().map_err(|e: String| schema("root", e))?,
        query: SearchQuery::try_from(query).map_err(|e: String| schema("query", e))?,
        kind,
        outside,
    };
    let data = simple_op(&app, &state, &host_id, request).await?;
    serde_json::from_value(data).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsReadResult {
    pub size: u64,
    pub hash: String,
    pub mtime: i64,
    pub truncated: bool,
    pub text: String,
}

/// Reads a text file (`fs_read`): the host streams UTF-8 bytes in kind-1
/// frames after the control reply, same shape as `blob_get`.
#[tauri::command]
pub async fn fs_read(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    path: String,
    outside: Option<bool>,
    as_text: Option<bool>,
) -> Result<FsReadResult, AppErrorDto> {
    let dir = directory(&app)?;
    let ticket = host_ticket(&dir, &host_id)?;
    let (mut send, mut recv) = open_stream(&state, &dir, &ticket).await?;
    let request = Request::FsRead {
        path: path.try_into().map_err(|e: String| schema("path", e))?,
        range: None,
        outside,
        as_text,
    };
    send.write_all(&hello_frame(request)?)
        .await
        .map_err(transport)?;
    let _ = send.shutdown().await;

    let (kind, payload) = read_frame(&mut recv, Some(FRAME_TIMEOUT))
        .await?
        .ok_or_else(|| transport("The PC closed the download without a reply"))?;
    if kind != 0 {
        return Err(schema("frame", "expected a control reply"));
    }
    let read_data: FsReadData = match parse_control(&payload)? {
        Control::Ok { data } => serde_json::from_value(data.unwrap_or_default())
            .map_err(|e| schema("data", e.to_string()))?,
        Control::Err { error } => return Err(error),
        _ => return Err(schema("type", "unexpected fs_read reply")),
    };

    let mut bytes = Vec::new();
    loop {
        let Some((kind, payload)) = read_frame(&mut recv, Some(FRAME_TIMEOUT)).await? else {
            return Err(transport("The download was cut off"));
        };
        match kind {
            1 => bytes.extend_from_slice(&payload),
            0 => match parse_control(&payload)? {
                Control::Exit { .. } => break,
                Control::Err { error } => return Err(error),
                _ => return Err(schema("type", "unexpected fs_read frame")),
            },
            _ => return Err(schema("frame", "unexpected frame kind")),
        }
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| AppErrorDto::new(AppErrorCode::E_BINARY, "That file isn't text", false))?;
    Ok(FsReadResult {
        size: read_data.size,
        hash: read_data.hash,
        mtime: read_data.mtime,
        truncated: read_data.truncated,
        text,
    })
}

#[tauri::command]
pub async fn fs_write(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    path: String,
    text: String,
    base_hash: Option<String>,
    create: Option<bool>,
    outside: Option<bool>,
) -> Result<FsWriteData, AppErrorDto> {
    let request = Request::FsWrite {
        path: path.try_into().map_err(|e: String| schema("path", e))?,
        text,
        base_hash,
        create,
        outside,
    };
    let data = simple_op(&app, &state, &host_id, request).await?;
    serde_json::from_value(data).map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

/// Copies an already-uploaded blob (`blob_put_bytes`/`blob_put_file`) to
/// `to` on the host (`fs_import`, UNI-199): "upload, then place it" in
/// one extra round trip instead of a separate stream to write bytes.
#[tauri::command]
pub async fn fs_import(
    app: tauri::AppHandle,
    state: tauri::State<'_, NetState>,
    host_id: String,
    sha256: String,
    to: String,
    overwrite: Option<bool>,
    outside: Option<bool>,
) -> Result<(), AppErrorDto> {
    let request = Request::FsImport {
        sha256: Sha256Hex::try_from(sha256).map_err(|e: String| schema("sha256", e))?,
        to: to.try_into().map_err(|e: String| schema("to", e))?,
        overwrite,
        outside,
    };
    simple_op(&app, &state, &host_id, request).await?;
    Ok(())
}
