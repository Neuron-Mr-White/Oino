//! Files pushed from the PC (UNI-214, `unipi-host push`), phone/desktop side.
//!
//! Commands (the UI's `src/lib/push.ts`):
//! - `push_list { hostId, wait }` → `PushListData` (long-poll ≤ 30 s).
//! - `push_fetch { hostId, path, sha256, size, name, onProgress }` →
//!   `{ localPath }`: streams `blob_get{path}` into the app cache
//!   (`<cache>/pushes/…`, which Android's FileProvider exposes to the
//!   installer), hashing as it goes; a size or sha256 mismatch deletes the
//!   file and fails with `E_BLOB_REJECTED` (`detail.reason = "checksum"`).
//! - `push_done { hostId, id, outcome, detail? }` → tells the PC.
//! - `fs_download { hostId, path, onProgress }` → `{ localPath, size }`: the
//!   same streamed download for any readable host file (Files → Download,
//!   UNI-225), no size cap, no checksum.
//! - desktop only: `push_save_as { src, name }` (native Save dialog →
//!   the saved path, or `null` when cancelled) and `push_open { path }`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use sha2::{Digest, Sha256};
use tauri::ipc::Channel;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, AppErrorDto, BlobGetData, BlobPath, Control, ProtoError, PushListData,
    PushOutcome, Request, PROTO_VERSION, PUSH_WAIT_MAX,
};

use crate::blob::BlobProgress;

fn transport(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Transport(error.to_string()).to_app_error()
}

fn internal(error: impl std::fmt::Display) -> AppErrorDto {
    ProtoError::Internal(error.to_string()).to_app_error()
}

fn schema(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false)
}

fn hello_frame(request: Request) -> Result<Vec<u8>, AppErrorDto> {
    let hello = Control::Hello {
        proto: PROTO_VERSION
            .to_string()
            .try_into()
            .map_err(|e: String| internal(e))?,
        req: request,
    };
    Ok(encode(
        FrameKind::Control,
        &serde_json::to_vec(&hello).map_err(internal)?,
    ))
}

async fn read_frame<R: AsyncRead + Unpin>(
    recv: &mut R,
    limit: Duration,
) -> Result<Option<(u8, Vec<u8>)>, AppErrorDto> {
    let read = async {
        let kind = match recv.read_u8().await {
            Ok(kind) => kind,
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => return Ok(None),
            Err(error) => return Err(transport(error)),
        };
        let len = recv.read_u32().await.map_err(transport)? as usize;
        if len > MAX_FRAME {
            return Err(schema("oversize frame from the PC"));
        }
        let mut payload = vec![0u8; len];
        recv.read_exact(&mut payload).await.map_err(transport)?;
        Ok(Some((kind, payload)))
    };
    tokio::time::timeout(limit, read)
        .await
        .map_err(|_| ProtoError::Timeout("The PC stopped answering".into()).to_app_error())?
}

async fn request_reply<R, W>(
    send: &mut W,
    recv: &mut R,
    request: Request,
    limit: Duration,
) -> Result<Option<serde_json::Value>, AppErrorDto>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
{
    send.write_all(&hello_frame(request)?)
        .await
        .map_err(transport)?;
    let _ = send.shutdown().await;
    let (kind, payload) = read_frame(recv, limit)
        .await?
        .ok_or_else(|| transport("The PC closed the request without a reply"))?;
    if kind != 0 {
        return Err(schema("expected a control reply"));
    }
    match serde_json::from_slice::<Control>(&payload).map_err(|e| schema(e.to_string()))? {
        Control::Ok { data } => Ok(data),
        Control::Err { error } => Err(error),
        _ => Err(schema("unexpected reply")),
    }
}

/// A file name safe to use in our cache dir.
pub(crate) fn safe_name(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .take(120)
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Why a fetched file was refused.
pub(crate) fn checksum_error(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_BLOB_REJECTED, message, true)
        .with_detail(serde_json::json!({"reason":"checksum"}))
}

/// Streams a `blob_get` reply (after the hello) into `dest`, verifying the
/// size and sha256 the PC announced. On any failure `dest` is removed.
pub(crate) async fn receive_verified<R, F>(
    recv: &mut R,
    dest: &Path,
    expected_sha256: &str,
    expected_size: u64,
    progress: F,
) -> Result<u64, AppErrorDto>
where
    R: AsyncRead + Unpin,
    F: FnMut(u64, u64),
{
    receive_to_file(recv, dest, Some((expected_sha256, expected_size)), progress)
        .await
        .map(|(size, _)| size)
}

/// Streams a `blob_get` reply into `dest`. With `expected` (sha256, size) the
/// file is verified like a push; without, the size the PC announces is the
/// target (a plain "Download" from Files, UNI-225: no checksum is known).
/// On any failure `dest` is removed. Returns (bytes received, the PC's mime guess).
pub(crate) async fn receive_to_file<R, F>(
    recv: &mut R,
    dest: &Path,
    expected: Option<(&str, u64)>,
    mut progress: F,
) -> Result<(u64, String), AppErrorDto>
where
    R: AsyncRead + Unpin,
    F: FnMut(u64, u64),
{
    let result = async {
        let (kind, payload) = read_frame(recv, Duration::from_secs(60))
            .await?
            .ok_or_else(|| transport("The PC closed the download without a reply"))?;
        if kind != 0 {
            return Err(schema("expected a control reply"));
        }
        let data: BlobGetData =
            match serde_json::from_slice::<Control>(&payload).map_err(|e| schema(e.to_string()))? {
                Control::Ok { data } => serde_json::from_value(data.unwrap_or_default())
                    .map_err(|e| schema(e.to_string()))?,
                Control::Err { error } => return Err(error),
                _ => return Err(schema("unexpected blob_get reply")),
            };
        let expected_size = match expected {
            Some((_, size)) if data.size != size => {
                return Err(checksum_error(format!(
                    "The file on the PC changed (now {} bytes, pushed as {size}). Push it again.",
                    data.size
                )));
            }
            Some((_, size)) => size,
            None => data.size,
        };
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent).await.map_err(internal)?;
        }
        let mut out = tokio::fs::File::create(dest)
            .await
            .map_err(|e| internal(format!("can't write {}: {e}", dest.display())))?;
        let mut hasher = Sha256::new();
        let mut received = 0u64;
        loop {
            let Some((kind, payload)) = read_frame(recv, Duration::from_secs(60)).await? else {
                return Err(transport("The download was cut off"));
            };
            match kind {
                1 => {
                    received += payload.len() as u64;
                    if received > expected_size {
                        return Err(checksum_error("The PC sent more bytes than announced."));
                    }
                    hasher.update(&payload);
                    out.write_all(&payload).await.map_err(internal)?;
                    progress(received, expected_size);
                }
                0 => match serde_json::from_slice::<Control>(&payload)
                    .map_err(|e| schema(e.to_string()))?
                {
                    Control::Exit { .. } => break,
                    Control::Err { error } => return Err(error),
                    _ => return Err(schema("unexpected blob_get frame")),
                },
                _ => return Err(schema("unexpected frame kind")),
            }
        }
        out.flush().await.map_err(internal)?;
        out.sync_all().await.map_err(internal)?;
        if received != expected_size {
            return Err(transport("The download was cut off"));
        }
        if let Some((expected_sha256, _)) = expected {
            let digest: String = hasher
                .finalize()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect();
            if digest != expected_sha256 {
                return Err(checksum_error(
                    "The file didn't arrive intact (checksum mismatch).",
                ));
            }
        }
        Ok((received, data.mime))
    }
    .await;
    if result.is_err() {
        let _ = tokio::fs::remove_file(dest).await;
    }
    result
}

fn cache_dir(app: &tauri::AppHandle) -> Result<PathBuf, AppErrorDto> {
    use tauri::Manager;
    Ok(app.path().app_cache_dir().map_err(internal)?.join("pushes"))
}

fn host_opener(
    app: &tauri::AppHandle,
    host_id: &str,
) -> Result<crate::forward::Opener, AppErrorDto> {
    crate::forward::net_opener(app, host_id)
}

#[tauri::command]
pub async fn push_list(
    app: tauri::AppHandle,
    host_id: String,
    wait: Option<u32>,
) -> Result<PushListData, AppErrorDto> {
    let wait = wait.unwrap_or(0).min(PUSH_WAIT_MAX);
    let opener = host_opener(&app, &host_id)?;
    let limit = Duration::from_secs(wait as u64 + 15);
    let mut last = transport("not attempted");
    for fresh in [false, true] {
        let attempt = async {
            let (mut send, mut recv) = opener(fresh).await?;
            let request = Request::PushList {
                wait: Some(wait.try_into().map_err(|e: String| schema(e))?),
            };
            let data = request_reply(&mut send, &mut recv, request, limit).await?;
            serde_json::from_value::<PushListData>(data.unwrap_or_default()).map_err(internal)
        }
        .await;
        match attempt {
            Err(error) if error.code == AppErrorCode::E_TRANSPORT && !fresh => last = error,
            other => return other,
        }
    }
    Err(last)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PushFetchResult {
    pub local_path: String,
    pub size: u64,
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn push_fetch(
    app: tauri::AppHandle,
    host_id: String,
    id: String,
    path: String,
    sha256: String,
    size: u64,
    name: String,
    on_progress: Channel<BlobProgress>,
) -> Result<PushFetchResult, AppErrorDto> {
    let sha = unipi_proto::Sha256Hex::try_from(sha256.clone()).map_err(schema)?;
    let id = unipi_proto::PushId::try_from(id).map_err(schema)?;
    let blob_path = BlobPath::try_from(path).map_err(schema)?;
    let dir = cache_dir(&app)?;
    // One dir per push so two files with the same name never collide, and
    // stale pushes are easy to sweep.
    let _ = sweep(&dir, Duration::from_secs(2 * 24 * 60 * 60)).await;
    let dest = dir.join(id.as_str()).join(safe_name(&name));
    let opener = host_opener(&app, &host_id)?;
    let mut last = transport("not attempted");
    for fresh in [false, true] {
        let attempt = async {
            let (mut send, mut recv) = opener(fresh).await?;
            let request = Request::BlobGet {
                sha256: None,
                path: Some(blob_path.clone()),
                range: None,
            };
            send.write_all(&hello_frame(request)?)
                .await
                .map_err(transport)?;
            let _ = send.shutdown().await;
            let mut last_sent = 0u64;
            receive_verified(&mut recv, &dest, sha.as_str(), size, |done, total| {
                // ~1% steps: plenty for a progress bar, cheap over IPC.
                if done == total || done - last_sent >= (total / 100).max(256 * 1024) {
                    last_sent = done;
                    let _ = on_progress.send(BlobProgress::Progress { sent: done, total });
                }
            })
            .await
        }
        .await;
        match attempt {
            Ok(size) => {
                return Ok(PushFetchResult {
                    local_path: dest.to_string_lossy().into_owned(),
                    size,
                })
            }
            Err(error) if error.code == AppErrorCode::E_TRANSPORT && !fresh => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FsDownloadResult {
    pub local_path: String,
    pub size: u64,
    pub mime: String,
}

/// Downloads any host file the phone may read (Files → Download / Open
/// with…, UNI-225) into `<cache>/pushes/dl-<ms>/<name>`, streamed with
/// progress — no size cap, unlike `blob_get_bytes` (the previewer's 25 MB
/// in-memory read, whose "too large to preview inline" error used to be all
/// a Download of a big file ever showed). The result is handed off exactly
/// like a pushed file: Android `UniPiFiles.saveFile/openFile`, desktop
/// `push_save_as` / `push_open` (both only accept files under that dir).
#[tauri::command]
pub async fn fs_download(
    app: tauri::AppHandle,
    host_id: String,
    path: String,
    on_progress: Channel<BlobProgress>,
) -> Result<FsDownloadResult, AppErrorDto> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or("").to_string();
    let blob_path = BlobPath::try_from(path).map_err(schema)?;
    let dir = cache_dir(&app)?;
    let _ = sweep(&dir, Duration::from_secs(2 * 24 * 60 * 60)).await;
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let dest = dir.join(format!("dl-{stamp}")).join(safe_name(&name));
    let opener = host_opener(&app, &host_id)?;
    let mut last = transport("not attempted");
    for fresh in [false, true] {
        let attempt = async {
            let (mut send, mut recv) = opener(fresh).await?;
            let request = Request::BlobGet {
                sha256: None,
                path: Some(blob_path.clone()),
                range: None,
            };
            send.write_all(&hello_frame(request)?)
                .await
                .map_err(transport)?;
            let _ = send.shutdown().await;
            let mut last_sent = 0u64;
            receive_to_file(&mut recv, &dest, None, |done, total| {
                if done == total || done - last_sent >= (total / 100).max(256 * 1024) {
                    last_sent = done;
                    let _ = on_progress.send(BlobProgress::Progress { sent: done, total });
                }
            })
            .await
        }
        .await;
        match attempt {
            Ok((size, mime)) => {
                return Ok(FsDownloadResult {
                    local_path: dest.to_string_lossy().into_owned(),
                    size,
                    mime,
                })
            }
            Err(error) if error.code == AppErrorCode::E_TRANSPORT && !fresh => last = error,
            Err(error) => return Err(error),
        }
    }
    Err(last)
}

/// Removes push dirs older than `age` (best-effort).
async fn sweep(dir: &Path, age: Duration) -> std::io::Result<()> {
    let mut entries = tokio::fs::read_dir(dir).await?;
    while let Some(entry) = entries.next_entry().await? {
        let old = entry
            .metadata()
            .await
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|e| e > age);
        if old {
            let _ = tokio::fs::remove_dir_all(entry.path()).await;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn push_done(
    app: tauri::AppHandle,
    host_id: String,
    id: String,
    outcome: PushOutcome,
    detail: Option<String>,
) -> Result<(), AppErrorDto> {
    let id = unipi_proto::PushId::try_from(id).map_err(schema)?;
    let opener = host_opener(&app, &host_id)?;
    let mut last = transport("not attempted");
    for fresh in [false, true] {
        let attempt = async {
            let (mut send, mut recv) = opener(fresh).await?;
            let request = Request::PushDone {
                id: id.clone(),
                outcome,
                detail: detail.clone(),
            };
            request_reply(&mut send, &mut recv, request, Duration::from_secs(15))
                .await
                .map(|_| ())
        }
        .await;
        match attempt {
            Err(error) if error.code == AppErrorCode::E_TRANSPORT && !fresh => last = error,
            other => return other,
        }
    }
    Err(last)
}

/// Only files we fetched ourselves may be saved/opened.
fn fetched_file(app: &tauri::AppHandle, src: &str) -> Result<PathBuf, AppErrorDto> {
    let root = cache_dir(app)?;
    let path = PathBuf::from(src)
        .canonicalize()
        .map_err(|e| internal(format!("{src}: {e}")))?;
    let root = root.canonicalize().map_err(internal)?;
    if !path.starts_with(&root) || !path.is_file() {
        return Err(AppErrorDto::new(
            AppErrorCode::E_UNAUTHORIZED,
            "Not a pushed file",
            false,
        ));
    }
    Ok(path)
}

/// Desktop: native "Save as…" dialog, then copy. `None` when cancelled.
#[tauri::command]
pub async fn push_save_as(
    app: tauri::AppHandle,
    src: String,
    name: String,
) -> Result<Option<String>, AppErrorDto> {
    let src = fetched_file(&app, &src)?;
    #[cfg(desktop)]
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let default_dir = dirs_downloads();
        let file_name = safe_name(&name);
        app.run_on_main_thread(move || {
            let mut dialog = rfd::AsyncFileDialog::new()
                .set_file_name(&file_name)
                .set_title("Save file");
            if let Some(dir) = default_dir {
                dialog = dialog.set_directory(dir);
            }
            let pending = dialog.save_file();
            std::thread::spawn(move || {
                let picked =
                    tauri::async_runtime::block_on(pending).map(|h| h.path().to_path_buf());
                let _ = tx.send(picked);
            });
        })
        .map_err(internal)?;
        let Some(dest) = rx.await.map_err(internal)? else {
            return Ok(None);
        };
        tokio::fs::copy(&src, &dest)
            .await
            .map_err(|e| internal(format!("can't save to {}: {e}", dest.display())))?;
        Ok(Some(dest.to_string_lossy().into_owned()))
    }
    #[cfg(not(desktop))]
    {
        let _ = (src, name);
        Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "Save as… is desktop only (the phone uses Downloads)",
            false,
        ))
    }
}

#[cfg(desktop)]
fn dirs_downloads() -> Option<PathBuf> {
    let home = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE"))?;
    let dir = PathBuf::from(home).join("Downloads");
    dir.is_dir().then_some(dir)
}

/// Desktop: open a web link in the default browser (UNI-224). `window.open`
/// from the page would not reach the system browser.
#[tauri::command]
pub async fn open_url(app: tauri::AppHandle, url: String) -> Result<(), AppErrorDto> {
    if !is_web_link(&url) {
        return Err(schema("only http(s) and mailto links can be opened"));
    }
    #[cfg(target_os = "ios")]
    {
        // UNI-263: Safari (the ad hoc build's update page, other links).
        app.run_on_main_thread(move || {
            crate::ios_webview::open_in_safari(&url);
        })
        .map_err(internal)?;
        return Ok(());
    }
    #[cfg(not(target_os = "ios"))]
    let _ = &app;
    #[cfg(desktop)]
    {
        #[cfg(target_os = "linux")]
        let mut command = std::process::Command::new("xdg-open");
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(windows)]
        let mut command = {
            let mut c = std::process::Command::new("rundll32");
            c.arg("url.dll,FileProtocolHandler");
            c
        };
        command
            .arg(&url)
            .spawn()
            .map_err(|e| internal(format!("can't open {url}: {e}")))?;
        Ok(())
    }
    #[cfg(all(not(desktop), not(target_os = "ios")))]
    {
        Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "Links are opened by the Android bridge",
            false,
        ))
    }
}

/// http(s)/mailto only, and nothing that could be read as a command-line flag.
fn is_web_link(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    (lower.starts_with("http://") || lower.starts_with("https://") || lower.starts_with("mailto:"))
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

/// Desktop: open a fetched file with the system's default app.
#[tauri::command]
pub async fn push_open(app: tauri::AppHandle, path: String) -> Result<(), AppErrorDto> {
    let path = fetched_file(&app, &path)?;
    #[cfg(desktop)]
    {
        #[cfg(target_os = "linux")]
        let mut command = std::process::Command::new("xdg-open");
        #[cfg(target_os = "macos")]
        let mut command = std::process::Command::new("open");
        #[cfg(windows)]
        let mut command = {
            let mut c = std::process::Command::new("cmd");
            c.args(["/C", "start", ""]);
            c
        };
        command
            .arg(&path)
            .spawn()
            .map_err(|e| internal(format!("can't open {}: {e}", path.display())))?;
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = path;
        Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "Open is handled by the Android bridge",
            false,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;

    fn ctrl(control: &Control) -> Vec<u8> {
        encode(FrameKind::Control, &serde_json::to_vec(control).unwrap())
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect()
    }

    async fn run(
        body: &[u8],
        announced: u64,
        expected_sha: &str,
        expected_size: u64,
    ) -> (
        Result<u64, AppErrorDto>,
        PathBuf,
        tempfile::TempDir,
        Vec<u64>,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("p1").join("app.apk");
        let (mut host, mut phone) = duplex(1 << 20);
        let mut wire = ctrl(&Control::Ok {
            data: Some(serde_json::json!({"size": announced, "mime": "application/octet-stream"})),
        });
        for chunk in body.chunks(7) {
            wire.extend(encode(FrameKind::Output, chunk));
        }
        wire.extend(ctrl(&Control::Exit { code: Some(0) }));
        host.write_all(&wire).await.unwrap();
        drop(host);
        let mut seen = Vec::new();
        let result = receive_verified(&mut phone, &dest, expected_sha, expected_size, |d, _| {
            seen.push(d)
        })
        .await;
        (result, dest, dir, seen)
    }

    #[tokio::test]
    async fn verified_download_lands_with_progress() {
        let body = b"PK\x03\x04 pretend this is an apk".to_vec();
        let (result, dest, _dir, seen) =
            run(&body, body.len() as u64, &sha(&body), body.len() as u64).await;
        assert_eq!(result.unwrap(), body.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(*seen.last().unwrap(), body.len() as u64);
    }

    #[tokio::test]
    async fn sha_mismatch_is_rejected_and_deleted() {
        let body = b"tampered bytes".to_vec();
        let (result, dest, _dir, _) =
            run(&body, body.len() as u64, &"0".repeat(64), body.len() as u64).await;
        let error = result.unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_BLOB_REJECTED);
        assert_eq!(error.detail.unwrap()["reason"], "checksum");
        assert!(!dest.exists(), "a bad file never stays on disk");
    }

    #[tokio::test]
    async fn size_change_on_the_pc_is_rejected_before_download() {
        let body = b"abc".to_vec();
        let (result, dest, _dir, seen) = run(&body, 3, &sha(&body), 4).await;
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_BLOB_REJECTED);
        assert!(seen.is_empty() && !dest.exists());
    }

    /// UNI-225: a plain Download has no checksum; the announced size is the target, any size works.
    #[tokio::test]
    async fn unverified_download_takes_the_announced_size() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("dl-1").join("big.bin");
        let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let (mut host, mut phone) = duplex(1 << 20);
        let writer = tokio::spawn(async move {
            let mut wire = ctrl(&Control::Ok {
                data: Some(
                    serde_json::json!({"size": body.len(), "mime": "application/octet-stream"}),
                ),
            });
            for chunk in body.chunks(64 * 1024) {
                wire.extend(encode(FrameKind::Output, chunk));
            }
            wire.extend(ctrl(&Control::Exit { code: Some(0) }));
            host.write_all(&wire).await.unwrap();
            body
        });
        let mut seen = Vec::new();
        let (got, mime) = receive_to_file(&mut phone, &dest, None, |d, t| seen.push((d, t)))
            .await
            .unwrap();
        assert_eq!(mime, "application/octet-stream");
        let body = writer.await.unwrap();
        assert_eq!(got, body.len() as u64);
        assert_eq!(std::fs::read(&dest).unwrap(), body);
        assert_eq!(
            seen.last().unwrap(),
            &(body.len() as u64, body.len() as u64)
        );

        // Cut off before the announced size: an error, and nothing left behind.
        let dest = dir.path().join("dl-2").join("cut.bin");
        let (mut host, mut phone) = duplex(1 << 16);
        let mut wire = ctrl(&Control::Ok {
            data: Some(serde_json::json!({"size": 10, "mime": "x"})),
        });
        wire.extend(encode(FrameKind::Output, b"12345"));
        host.write_all(&wire).await.unwrap();
        drop(host);
        assert_eq!(
            receive_to_file(&mut phone, &dest, None, |_, _| {})
                .await
                .unwrap_err()
                .code,
            AppErrorCode::E_TRANSPORT
        );
        assert!(!dest.exists());
    }

    #[test]
    fn web_links_only() {
        assert!(is_web_link("https://example.com/a?b=c"));
        assert!(is_web_link("mailto:a@b.c"));
        assert!(!is_web_link("file:///etc/passwd"));
        assert!(!is_web_link("--help"));
        assert!(!is_web_link("https://x.com/ a"));
    }

    #[tokio::test]
    async fn host_error_and_cut_off_stream() {
        let dir = tempfile::tempdir().unwrap();
        let dest = dir.path().join("x");
        let (mut host, mut phone) = duplex(1 << 16);
        let err = AppErrorDto::new(
            AppErrorCode::E_UNAUTHORIZED,
            "That path isn't available to the phone",
            false,
        );
        host.write_all(&ctrl(&Control::Err { error: err }))
            .await
            .unwrap();
        drop(host);
        let result = receive_verified(&mut phone, &dest, &"0".repeat(64), 3, |_, _| {}).await;
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_UNAUTHORIZED);

        let (mut host, mut phone) = duplex(1 << 16);
        let mut wire = ctrl(&Control::Ok {
            data: Some(serde_json::json!({"size": 10, "mime": "x"})),
        });
        wire.extend(encode(FrameKind::Output, b"12345"));
        host.write_all(&wire).await.unwrap();
        drop(host);
        let result = receive_verified(&mut phone, &dest, &"0".repeat(64), 10, |_, _| {}).await;
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_TRANSPORT);
        assert!(!dest.exists());
    }

    #[test]
    fn safe_names() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\x\\app.apk"), "app.apk");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name("a:b?.txt"), "a_b_.txt");
    }
}
