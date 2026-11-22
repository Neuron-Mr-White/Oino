//! Blob channel commands (UNI-157 "media + attachments"): the UI's
//! `src/lib/blob.ts` calls these to upload/download bytes to/from the
//! paired PC's `unipi-host`, riding the same iroh connection as the board
//! relay. See `unipi_proto::blob` for the wire layout.
//!
//! - `blob_put_file { hostId, path, mime, name?, onProgress }` → reads a
//!   local file, hashes it, uploads it (or skips the upload if the host
//!   already has that sha256) → `{ sha256, size, deduped }`.
//! - `blob_get_file { hostId, sha256?, path?, destPath, onProgress }` →
//!   downloads a blob/allow-listed path to `destPath` → `{ size, mime }`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::ipc::Channel;
use tauri::{Manager, State};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, AppErrorDto, BlobGetData, BlobMime, BlobName, BlobPutData, BlobRange, Control,
    PathSegment, ProtoError, Request, Sha256Hex, PROTO_VERSION,
};

use crate::net::NetState;

fn schema(field: &str, message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
        .with_path(vec![PathSegment::Key(field.into())])
}

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

/// Progress events for both `blob_put_file` and `blob_get_file`.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BlobProgress {
    Progress { sent: u64, total: u64 },
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobPutResult {
    pub sha256: String,
    pub size: u64,
    pub deduped: bool,
    /// The blob's absolute path on the host; pass this as `images[].path`
    /// in a bridge `prompt` (the bridge reads it off disk directly).
    pub path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobGetResult {
    pub size: u64,
    pub mime: String,
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
    // fix-frames: an oversize reply is drained (the stream stays in sync) and reported as
    // retryable transport trouble, never a dead non-retryable schema error.
    let read = async {
        match crate::frames::read_lenient(recv).await? {
            crate::frames::FrameRead::Frame(kind, payload) => Ok(Some((kind, payload))),
            crate::frames::FrameRead::Skipped { len, .. } => {
                Err(crate::frames::oversize_error(len))
            }
            crate::frames::FrameRead::End => Ok(None),
        }
    };
    match timeout {
        Some(limit) => tokio::time::timeout(limit, read).await.map_err(|_| {
            ProtoError::Timeout("The PC stopped answering the blob request".into()).to_app_error()
        })?,
        None => read.await,
    }
}

fn parse_control(payload: &[u8]) -> Result<Control, AppErrorDto> {
    serde_json::from_slice(payload).map_err(|e| schema("control", e.to_string()))
}

/// How long the host may stay silent while answering (it may have to find
/// space on disk, evict old blobs, etc.).
const FRAME_TIMEOUT: Duration = Duration::from_secs(60);
const CHUNK: usize = 256 * 1024;

fn directory(app: &tauri::AppHandle) -> Result<PathBuf, AppErrorDto> {
    app.path()
        .app_data_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())
}

fn host_ticket(dir: &Path, host_id: &str) -> Result<(String, String), AppErrorDto> {
    let host = crate::local_host::hosts(dir)?
        .into_iter()
        .find(|host| host.id == host_id)
        .ok_or_else(|| ProtoError::NotFound("Host not found".into()).to_app_error())?;
    let id = crate::net::parse_ticket(&host.ticket)?
        .endpoint_addr()
        .id
        .to_string();
    Ok((host.ticket, id))
}

async fn open_stream(
    state: &NetState,
    dir: &Path,
    ticket: &str,
) -> Result<(crate::op_stream::OpSend, crate::op_stream::OpRecv), AppErrorDto> {
    let relay = state.current_relay().await;
    state.open_op(dir, relay.as_deref(), ticket).await
}

/// Hashes a local file and reads its size, in one streaming pass.
async fn sha256_file(path: &Path) -> Result<(String, u64), AppErrorDto> {
    let mut file = tokio::fs::File::open(path).await.map_err(|e| {
        ProtoError::NotFound(format!("can't read {}: {e}", path.display())).to_app_error()
    })?;
    let mut hasher = Sha256::new();
    let mut size = 0u64;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
        size += n as u64;
    }
    let digest = hasher.finalize();
    let mut hex = String::with_capacity(64);
    for b in digest {
        hex.push_str(&format!("{b:02x}"));
    }
    Ok((hex, size))
}

/// Uploads in-memory `bytes` to the paired host (the composer's shrunk
/// image blobs: no temp file needed), skipping the body entirely when the
/// host already has that sha256 (dedup).
#[tauri::command]
pub async fn blob_put_bytes(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    bytes: Vec<u8>,
    mime: String,
    name: Option<String>,
    on_progress: Channel<BlobProgress>,
) -> Result<BlobPutResult, AppErrorDto> {
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let mut sha256_hex = String::with_capacity(64);
    for b in digest {
        sha256_hex.push_str(&format!("{b:02x}"));
    }
    let size = bytes.len() as u64;
    let sha256 = Sha256Hex::try_from(sha256_hex.clone()).map_err(|e| schema("sha256", e))?;
    let mime = BlobMime::try_from(mime).map_err(|e| schema("mime", e))?;
    let name = name
        .map(BlobName::try_from)
        .transpose()
        .map_err(|e| schema("name", e))?;

    let dir = directory(&app)?;
    let (ticket, _id) = host_ticket(&dir, &host_id)?;
    let (mut send, mut recv) = open_stream(&state, &dir, &ticket).await?;

    let request = Request::BlobPut {
        sha256: sha256.clone(),
        size: size.try_into().map_err(|e: String| schema("size", e))?,
        mime,
        name,
        offset: None,
    };
    send.write_all(&hello_frame(request)?)
        .await
        .map_err(transport)?;

    let (kind, payload) = read_frame(&mut recv, Some(FRAME_TIMEOUT))
        .await?
        .ok_or_else(|| transport("The PC closed the upload without a reply"))?;
    if kind != 0 {
        return Err(schema("frame", "expected a control reply"));
    }
    let put_data: BlobPutData = match parse_control(&payload)? {
        Control::Ok { data } => serde_json::from_value(data.unwrap_or_default())
            .map_err(|e| schema("data", e.to_string()))?,
        Control::Err { error } => return Err(error),
        _ => return Err(schema("type", "unexpected blob_put reply")),
    };
    if put_data.have {
        return Ok(BlobPutResult {
            sha256: sha256_hex,
            size,
            deduped: true,
            path: put_data.path,
        });
    }

    let mut sent = 0u64;
    for piece in bytes.chunks(CHUNK) {
        send.write_all(&encode(FrameKind::Input, piece))
            .await
            .map_err(transport)?;
        sent += piece.len() as u64;
        let _ = on_progress.send(BlobProgress::Progress { sent, total: size });
    }
    let _ = send.shutdown().await;

    let (kind, payload) = read_frame(&mut recv, Some(FRAME_TIMEOUT))
        .await?
        .ok_or_else(|| transport("The PC closed the upload before confirming"))?;
    if kind != 0 {
        return Err(schema("frame", "expected an exit control"));
    }
    match parse_control(&payload)? {
        Control::Exit { .. } => Ok(BlobPutResult {
            sha256: sha256_hex,
            size,
            deduped: false,
            path: put_data.path,
        }),
        Control::Err { error } => Err(error),
        _ => Err(schema("type", "unexpected blob_put end")),
    }
}

/// UNI-278: a big file picked in the page (a `File` from <input type=file>)
/// has no path the core can read, and sending it whole over the IPC as a JSON
/// number array took ~5× its size in memory (the WebView died on big files).
/// The page streams it here in base64 chunks instead — `upload_stage_begin`,
/// `upload_stage_chunk` × N, then `blob_put_file` on the staged path, which
/// streams it to the host from disk. Staged files live in the app cache and
/// are removed after the upload (or by the next begin if left behind).
fn staging_dir(app: &tauri::AppHandle) -> Result<PathBuf, AppErrorDto> {
    Ok(app
        .path()
        .app_cache_dir()
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?
        .join("upload-staging"))
}

#[tauri::command]
pub async fn upload_stage_begin(
    app: tauri::AppHandle,
    name: String,
) -> Result<String, AppErrorDto> {
    let dir = staging_dir(&app)?;
    // Leftovers from an interrupted upload (older than an hour).
    if let Ok(mut entries) = tokio::fs::read_dir(&dir).await {
        while let Ok(Some(e)) = entries.next_entry().await {
            let old = e
                .metadata()
                .await
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > Duration::from_secs(3600));
            if old {
                let _ = tokio::fs::remove_file(e.path()).await;
            }
        }
    }
    tokio::fs::create_dir_all(&dir)
        .await
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let path = dir.join(format!("{stamp}-{}", crate::push::safe_name(&name)));
    tokio::fs::File::create(&path)
        .await
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    Ok(path.to_string_lossy().into_owned())
}

/// Only paths `upload_stage_begin` handed out.
fn staged(app: &tauri::AppHandle, path: &str) -> Result<PathBuf, AppErrorDto> {
    let dir = staging_dir(app)?;
    let p = PathBuf::from(path);
    if p.parent() != Some(dir.as_path()) || !p.is_file() {
        return Err(AppErrorDto::new(
            unipi_proto::AppErrorCode::E_UNAUTHORIZED,
            "not a staged upload",
            false,
        ));
    }
    Ok(p)
}

#[tauri::command]
pub async fn upload_stage_chunk(
    app: tauri::AppHandle,
    path: String,
    data: String,
) -> Result<u64, AppErrorDto> {
    use base64::Engine;
    let p = staged(&app, &path)?;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(data.as_bytes())
        .map_err(|e| schema("data", e.to_string()))?;
    let mut f = tokio::fs::OpenOptions::new()
        .append(true)
        .open(&p)
        .await
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    f.write_all(&bytes)
        .await
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    f.flush()
        .await
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    Ok(f.metadata().await.map(|m| m.len()).unwrap_or(0))
}

#[tauri::command]
pub async fn upload_stage_drop(app: tauri::AppHandle, path: String) -> Result<(), AppErrorDto> {
    if let Ok(p) = staged(&app, &path) {
        let _ = tokio::fs::remove_file(p).await;
    }
    Ok(())
}

/// Uploads `path`'s bytes to the paired host, skipping the body entirely
/// when the host already has that sha256 (dedup).
#[tauri::command]
pub async fn blob_put_file(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    path: String,
    mime: String,
    name: Option<String>,
    on_progress: Channel<BlobProgress>,
) -> Result<BlobPutResult, AppErrorDto> {
    let src = PathBuf::from(&path);
    let (sha256_hex, size) = sha256_file(&src).await?;
    let sha256 = Sha256Hex::try_from(sha256_hex.clone()).map_err(|e| schema("sha256", e))?;
    let mime = BlobMime::try_from(mime).map_err(|e| schema("mime", e))?;
    let name = name
        .map(BlobName::try_from)
        .transpose()
        .map_err(|e| schema("name", e))?;

    let dir = directory(&app)?;
    let (ticket, _id) = host_ticket(&dir, &host_id)?;
    let (mut send, mut recv) = open_stream(&state, &dir, &ticket).await?;

    let request = Request::BlobPut {
        sha256: sha256.clone(),
        size: size.try_into().map_err(|e: String| schema("size", e))?,
        mime,
        name,
        offset: None,
    };
    send.write_all(&hello_frame(request)?)
        .await
        .map_err(transport)?;

    let (kind, payload) = read_frame(&mut recv, Some(FRAME_TIMEOUT))
        .await?
        .ok_or_else(|| transport("The PC closed the upload without a reply"))?;
    if kind != 0 {
        return Err(schema("frame", "expected a control reply"));
    }
    let put_data: BlobPutData = match parse_control(&payload)? {
        Control::Ok { data } => serde_json::from_value(data.unwrap_or_default())
            .map_err(|e| schema("data", e.to_string()))?,
        Control::Err { error } => return Err(error),
        _ => return Err(schema("type", "unexpected blob_put reply")),
    };
    if put_data.have {
        return Ok(BlobPutResult {
            sha256: sha256_hex,
            size,
            deduped: true,
            path: put_data.path,
        });
    }

    let mut file = tokio::fs::File::open(&src).await.map_err(|e| {
        ProtoError::NotFound(format!("can't read {}: {e}", src.display())).to_app_error()
    })?;
    let mut sent = 0u64;
    let mut buf = vec![0u8; CHUNK];
    loop {
        let n = file
            .read(&mut buf)
            .await
            .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
        if n == 0 {
            break;
        }
        send.write_all(&encode(FrameKind::Input, &buf[..n]))
            .await
            .map_err(transport)?;
        sent += n as u64;
        let _ = on_progress.send(BlobProgress::Progress { sent, total: size });
    }
    let _ = send.shutdown().await;

    let (kind, payload) = read_frame(&mut recv, Some(FRAME_TIMEOUT))
        .await?
        .ok_or_else(|| transport("The PC closed the upload before confirming"))?;
    if kind != 0 {
        return Err(schema("frame", "expected an exit control"));
    }
    match parse_control(&payload)? {
        Control::Exit { .. } => Ok(BlobPutResult {
            sha256: sha256_hex,
            size,
            deduped: false,
            path: put_data.path,
        }),
        Control::Err { error } => Err(error),
        _ => Err(schema("type", "unexpected blob_put end")),
    }
}

/// Downloads a blob (by sha256) or an allow-listed path from the paired
/// host, writing it to `dest_path`.
#[tauri::command]
pub async fn blob_get_file(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    sha256: Option<String>,
    path: Option<String>,
    dest_path: String,
    on_progress: Channel<BlobProgress>,
) -> Result<BlobGetResult, AppErrorDto> {
    if sha256.is_none() && path.is_none() {
        return Err(schema("sha256", "blob_get_file needs sha256 or path"));
    }
    let sha256 = sha256
        .map(Sha256Hex::try_from)
        .transpose()
        .map_err(|e| schema("sha256", e))?;
    let path = path
        .map(unipi_proto::BlobPath::try_from)
        .transpose()
        .map_err(|e| schema("path", e))?;

    let dir = directory(&app)?;
    let (ticket, _id) = host_ticket(&dir, &host_id)?;
    let (mut send, mut recv) = open_stream(&state, &dir, &ticket).await?;

    let request = Request::BlobGet {
        sha256,
        path,
        range: None::<BlobRange>,
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
    let get_data: BlobGetData = match parse_control(&payload)? {
        Control::Ok { data } => serde_json::from_value(data.unwrap_or_default())
            .map_err(|e| schema("data", e.to_string()))?,
        Control::Err { error } => return Err(error),
        _ => return Err(schema("type", "unexpected blob_get reply")),
    };

    if let Some(parent) = Path::new(&dest_path).parent() {
        let _ = tokio::fs::create_dir_all(parent).await;
    }
    let mut out = tokio::fs::File::create(&dest_path).await.map_err(|e| {
        ProtoError::Internal(format!("can't write {dest_path}: {e}")).to_app_error()
    })?;
    let mut received = 0u64;
    loop {
        let Some((kind, payload)) = read_frame(&mut recv, Some(FRAME_TIMEOUT)).await? else {
            return Err(transport("The download was cut off"));
        };
        match kind {
            1 => {
                out.write_all(&payload)
                    .await
                    .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
                received += payload.len() as u64;
                let _ = on_progress.send(BlobProgress::Progress {
                    sent: received,
                    total: get_data.size,
                });
            }
            0 => match parse_control(&payload)? {
                Control::Exit { .. } => break,
                Control::Err { error } => return Err(error),
                _ => return Err(schema("type", "unexpected blob_get frame")),
            },
            _ => return Err(schema("frame", "unexpected frame kind")),
        }
    }
    out.flush()
        .await
        .map_err(|e| ProtoError::Internal(e.to_string()).to_app_error())?;
    Ok(BlobGetResult {
        size: get_data.size,
        mime: get_data.mime,
    })
}

/// Largest blob the previewer pulls straight into memory (bigger ones
/// should go through `blob_get_file` instead).
const MAX_INLINE_BYTES: u64 = 25 * 1024 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BlobGetBytesResult {
    pub mime: String,
    pub bytes: Vec<u8>,
}

/// Downloads a blob (by sha256) or an allow-listed path straight into
/// memory, for the previewer (no temp file). Capped at `MAX_INLINE_BYTES`.
#[tauri::command]
pub async fn blob_get_bytes(
    app: tauri::AppHandle,
    state: State<'_, NetState>,
    host_id: String,
    sha256: Option<String>,
    path: Option<String>,
) -> Result<BlobGetBytesResult, AppErrorDto> {
    if sha256.is_none() && path.is_none() {
        return Err(schema("sha256", "blob_get_bytes needs sha256 or path"));
    }
    let sha256 = sha256
        .map(Sha256Hex::try_from)
        .transpose()
        .map_err(|e| schema("sha256", e))?;
    let path = path
        .map(unipi_proto::BlobPath::try_from)
        .transpose()
        .map_err(|e| schema("path", e))?;

    let dir = directory(&app)?;
    let (ticket, _id) = host_ticket(&dir, &host_id)?;
    let (mut send, mut recv) = open_stream(&state, &dir, &ticket).await?;

    let request = Request::BlobGet {
        sha256,
        path,
        range: None::<BlobRange>,
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
    let get_data: BlobGetData = match parse_control(&payload)? {
        Control::Ok { data } => serde_json::from_value(data.unwrap_or_default())
            .map_err(|e| schema("data", e.to_string()))?,
        Control::Err { error } => return Err(error),
        _ => return Err(schema("type", "unexpected blob_get reply")),
    };
    if get_data.size > MAX_INLINE_BYTES {
        return Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "That file is too large to preview inline",
            false,
        ));
    }
    let mut out = Vec::with_capacity(get_data.size as usize);
    loop {
        let Some((kind, payload)) = read_frame(&mut recv, Some(FRAME_TIMEOUT)).await? else {
            return Err(transport("The download was cut off"));
        };
        match kind {
            1 => out.extend_from_slice(&payload),
            0 => match parse_control(&payload)? {
                Control::Exit { .. } => break,
                Control::Err { error } => return Err(error),
                _ => return Err(schema("type", "unexpected blob_get frame")),
            },
            _ => return Err(schema("frame", "unexpected frame kind")),
        }
    }
    Ok(BlobGetBytesResult {
        mime: get_data.mime,
        bytes: out,
    })
}
