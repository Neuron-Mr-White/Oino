//! Blob channel (UNI-157 "media + attachments"): `blob_put`/`blob_get`
//! streams. See `unipi_proto::blob` for the wire layout.
//!
//! Storage: uploaded blobs live under `<host dir>/blobs/<sha256>`, capped at
//! ~50 MiB each; the whole directory is capped at [`BLOB_DIR_CAP`] with an
//! LRU-by-mtime eviction sweep run after every successful put. `blob_get`
//! also serves paths inside the `fs_*` scope (`$HOME` + the extra safe
//! roots) and one pushed file, so a phone can open what the Files screen
//! shows. v2: the pi app-bridge attachment allow-list
//! (`~/.unipi/bridge/<pid>.files.json`) is gone with the bridge; session
//! attachments ride the blob channel itself.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use sha2::{Digest, Sha256};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use unipi_proto::{
    frame::FrameKind, AppErrorCode, AppErrorDto, BlobGetData, BlobMime, BlobName, BlobPath,
    BlobPutData, BlobRange, Control, Sha256Hex, BLOB_MAX_SIZE,
};

use crate::fs_scope::FsScope;
use crate::protocol::{read_frame, write_control};

/// Total size the blobs directory may grow to before old entries are
/// evicted (oldest mtime first) after a put.
pub const BLOB_DIR_CAP: u64 = 500 * 1024 * 1024;
/// UNI-278: a just-uploaded blob is never evicted for this long (Files →
/// Upload imports it right after; a big upload must not push itself out).
const FRESH_GRACE: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Output frames are split at this size (well under `MAX_FRAME`).
const CHUNK: usize = 256 * 1024;
/// How long one request body frame may take to arrive.
const BODY_FRAME_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

fn rejected(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_BLOB_REJECTED, message, false)
}

fn unauthorized(message: impl Into<String>) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_UNAUTHORIZED, message, false)
}

fn internal(error: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, error.to_string(), false)
}

fn blob_file(blobs_dir: &Path, sha256: &str) -> PathBuf {
    blobs_dir.join(sha256)
}

/// Handles one `blob_put` stream (see module docs). `blobs_dir` is
/// created on demand.
// v2 H1: pre-existing 8-arg blob upload entry point; reshaped with the H7 port
#[allow(clippy::too_many_arguments)]
pub async fn serve_put<R: AsyncRead + Unpin, W: AsyncWrite + Unpin>(
    blobs_dir: &Path,
    read: &mut R,
    write: &mut W,
    sha256: &Sha256Hex,
    size: u64,
    _mime: &BlobMime,
    _name: Option<&BlobName>,
    offset: Option<u64>,
) -> Result<(), AppErrorDto> {
    if size > BLOB_MAX_SIZE {
        return Err(rejected(format!(
            "blob is larger than the {BLOB_MAX_SIZE} byte cap"
        )));
    }
    std::fs::create_dir_all(blobs_dir).map_err(internal)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(blobs_dir) {
            let _ = std::fs::set_permissions(blobs_dir, std::fs::Permissions::from_mode(0o700));
            let _ = meta;
        }
    }
    let path = blob_file(blobs_dir, sha256.as_str());
    let path_str = path.to_string_lossy().into_owned();
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() == size {
            // Already have the exact blob: dedup, no bytes needed.
            write_control(
                write,
                &Control::Ok {
                    data: Some(
                        serde_json::to_value(BlobPutData {
                            have: true,
                            path: path_str,
                        })
                        .map_err(internal)?,
                    ),
                },
            )
            .await
            .map_err(internal)?;
            let _ = touch(&path);
            return Ok(());
        }
    }
    write_control(
        write,
        &Control::Ok {
            data: Some(
                serde_json::to_value(BlobPutData {
                    have: false,
                    path: path_str,
                })
                .map_err(internal)?,
            ),
        },
    )
    .await
    .map_err(internal)?;

    let start = offset.unwrap_or(0);
    if start > size {
        return Err(rejected("offset is past the blob's declared size"));
    }
    let tmp_path = path.with_extension("part");
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;
    {
        use std::io::{Read, Seek, SeekFrom, Write as _};
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&tmp_path)
            .map_err(internal)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = file.set_permissions(std::fs::Permissions::from_mode(0o600));
        }
        if start > 0 {
            // Re-hash the bytes we already hold so the final digest covers
            // the whole blob, not just the resumed tail.
            file.seek(SeekFrom::Start(0)).map_err(internal)?;
            let mut buf = [0u8; 64 * 1024];
            let mut remaining = start;
            while remaining > 0 {
                let want = remaining.min(buf.len() as u64) as usize;
                let n = file.read(&mut buf[..want]).map_err(internal)?;
                if n == 0 {
                    return Err(rejected("offset is past what the host actually holds"));
                }
                hasher.update(&buf[..n]);
                remaining -= n as u64;
            }
            written = start;
        }
        file.seek(SeekFrom::Start(start)).map_err(internal)?;
        file.set_len(start).map_err(internal)?;

        while written < size {
            let (kind, bytes) = tokio::time::timeout(BODY_FRAME_TIMEOUT, read_frame(read))
                .await
                .map_err(|_| {
                    AppErrorDto::new(
                        AppErrorCode::E_TIMEOUT,
                        "Upload stalled for 30 seconds",
                        true,
                    )
                })?
                .map_err(|e| rejected(format!("upload ended early: {e}")))?;
            if kind != FrameKind::Input as u8 {
                return Err(rejected("blob bytes must be kind-2 frames"));
            }
            if written + bytes.len() as u64 > size {
                return Err(rejected("uploaded more bytes than declared size"));
            }
            file.write_all(&bytes).map_err(internal)?;
            hasher.update(&bytes);
            written += bytes.len() as u64;
        }
        file.sync_all().map_err(internal)?;
    }
    if written != size {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(rejected("upload ended before reaching the declared size"));
    }
    let digest = hex_digest(hasher);
    if digest != sha256.as_str() {
        let _ = std::fs::remove_file(&tmp_path);
        return Err(rejected("uploaded bytes don't match the declared sha256"));
    }
    std::fs::rename(&tmp_path, &path).map_err(internal)?;
    evict_lru(blobs_dir, BLOB_DIR_CAP);
    write_control(write, &Control::Exit { code: Some(0) })
        .await
        .map_err(internal)
}

fn hex_digest(hasher: Sha256) -> String {
    let bytes = hasher.finalize();
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

fn touch(path: &Path) -> std::io::Result<()> {
    let now = std::fs::FileTimes::new().set_modified(SystemTime::now());
    let file = std::fs::OpenOptions::new().write(true).open(path)?;
    file.set_times(now)
}

/// Stores `bytes` in the blob store (content-addressed, deduplicated, under the
/// same LRU cap as uploads) and returns its sha256 hex. The host's own writes — a
/// session snapshot's oversized inline images become `mediaRef` blobs (v2 QA:
/// sessions larger than the 1 MiB frame failed `session_open`).
///
/// # Errors
///
/// Blob directory or write failures.
pub fn store_bytes(blobs_dir: &Path, bytes: &[u8]) -> std::io::Result<String> {
    use std::io::Write as _;

    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hex_digest(hasher);
    std::fs::create_dir_all(blobs_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(blobs_dir, std::fs::Permissions::from_mode(0o700));
    }
    let path = blob_file(blobs_dir, &digest);
    if std::fs::metadata(&path)
        .map(|meta| meta.len())
        .is_ok_and(|len| len == bytes.len() as u64)
    {
        let _ = touch(&path);
        return Ok(digest);
    }
    let tmp_path = path.with_extension("part");
    {
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(&tmp_path, &path)?;
    evict_lru(blobs_dir, BLOB_DIR_CAP);
    Ok(digest)
}

/// Deletes the oldest-mtime files in `dir` until its total size is under
/// `cap`. Best-effort: I/O errors are logged and skipped.
fn evict_lru(dir: &Path, cap: u64) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(PathBuf, u64, SystemTime)> = Vec::new();
    let mut total: u64 = 0;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_some_and(|e| e == "part") {
            continue;
        }
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        total += meta.len();
        files.push((path, meta.len(), mtime));
    }
    if total <= cap {
        return;
    }
    files.sort_by_key(|(_, _, mtime)| *mtime);
    let now = SystemTime::now();
    for (path, len, mtime) in files {
        if total <= cap {
            break;
        }
        if now.duration_since(mtime).is_ok_and(|age| age < FRESH_GRACE) {
            continue;
        }
        if std::fs::remove_file(&path).is_ok() {
            total = total.saturating_sub(len);
        }
    }
}

/// Whether `path` may be read by `blob_get{path}`: under the blobs dir, or
/// (UNI-155: files opened from the Files screen) any path in `scope`
/// (`$HOME` + the extra safe roots like `/tmp`, UNI-220) once canonicalized
/// — same scope rule as `fs_read`.
pub fn path_allowed(path: &Path, blobs_dir: &Path, scope: Option<&FsScope>) -> bool {
    if path.starts_with(blobs_dir) {
        return true;
    }
    if let Some(scope) = scope {
        if let Ok(canon) = dunce::canonicalize(path) {
            if scope.allows(&canon) {
                return true;
            }
        }
    }
    false
}

/// Handles one `blob_get` stream. Exactly one of `sha256`/`path` is used
/// (checked by the caller: schema allows both omitted/combined, this
/// enforces the actual allow-list).
pub async fn serve_get<W: AsyncWrite + Unpin>(
    blobs_dir: &Path,
    scope: Option<&FsScope>,
    write: &mut W,
    sha256: Option<&Sha256Hex>,
    path: Option<&BlobPath>,
    range: Option<BlobRange>,
) -> Result<(), AppErrorDto> {
    serve_get_with(blobs_dir, scope, None, write, sha256, path, range).await
}

/// [`serve_get`] plus one extra exactly-allowed path (UNI-214: a file
/// pushed to this device, which may live outside `$HOME`).
#[allow(clippy::too_many_arguments)]
pub async fn serve_get_with<W: AsyncWrite + Unpin>(
    blobs_dir: &Path,
    scope: Option<&FsScope>,
    pushed: Option<&Path>,
    write: &mut W,
    sha256: Option<&Sha256Hex>,
    path: Option<&BlobPath>,
    range: Option<BlobRange>,
) -> Result<(), AppErrorDto> {
    let (resolved, mime_guess): (PathBuf, &str) = if let Some(sha) = sha256 {
        (
            blob_file(blobs_dir, sha.as_str()),
            "application/octet-stream",
        )
    } else if let Some(p) = path {
        let candidate = PathBuf::from(p.as_str());
        if pushed != Some(candidate.as_path()) && !path_allowed(&candidate, blobs_dir, scope) {
            return Err(unauthorized("That path isn't available to the phone"));
        }
        let mime = mime_guess_from_ext(&candidate);
        (candidate, mime)
    } else {
        return Err(rejected("blob_get needs sha256 or path"));
    };
    let mut file = tokio::fs::File::open(&resolved).await.map_err(|e| {
        AppErrorDto::new(
            AppErrorCode::E_NOT_FOUND,
            format!("blob not found: {e}"),
            false,
        )
    })?;
    let total = file.metadata().await.map_err(internal)?.len();
    let (start, end) = match range {
        Some(r) => {
            if r.start > r.end || r.end >= total {
                return Err(rejected("range is out of bounds"));
            }
            (r.start, r.end)
        }
        None => (0, total.saturating_sub(1)),
    };
    let len = if total == 0 { 0 } else { end - start + 1 };
    use tokio::io::AsyncSeekExt;
    if start > 0 {
        file.seek(std::io::SeekFrom::Start(start))
            .await
            .map_err(internal)?;
    }
    let data = BlobGetData {
        size: len,
        mime: mime_guess.to_string(),
    };
    write_control(
        write,
        &Control::Ok {
            data: Some(serde_json::to_value(data).map_err(internal)?),
        },
    )
    .await
    .map_err(internal)?;
    let mut remaining = len;
    let mut buf = vec![0u8; CHUNK];
    while remaining > 0 {
        let want = remaining.min(CHUNK as u64) as usize;
        let n = file.read(&mut buf[..want]).await.map_err(internal)?;
        if n == 0 {
            break;
        }
        write
            .write_all(&unipi_proto::frame::encode(FrameKind::Output, &buf[..n]))
            .await
            .map_err(internal)?;
        remaining -= n as u64;
    }
    write_control(write, &Control::Exit { code: Some(0) })
        .await
        .map_err(internal)
}

fn mime_guess_from_ext(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
    {
        Some(ext) => match ext.as_str() {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "webp" => "image/webp",
            "gif" => "image/gif",
            "pdf" => "application/pdf",
            "txt" | "log" => "text/plain",
            "md" => "text/markdown",
            "json" => "application/json",
            "mp4" => "video/mp4",
            "mp3" => "audio/mpeg",
            "wav" => "audio/wav",
            "apk" => "application/vnd.android.package-archive",
            _ => "application/octet-stream",
        },
        None => "application/octet-stream",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::duplex;
    use unipi_proto::frame::{encode, Decoder};

    async fn read_control<R: AsyncRead + Unpin>(read: &mut R) -> Control {
        let (kind, bytes) = crate::protocol::read_frame(read).await.unwrap();
        assert_eq!(kind, 0);
        serde_json::from_slice(&bytes).unwrap()
    }

    fn sha(s: &str) -> Sha256Hex {
        Sha256Hex::try_from(s.to_string()).unwrap()
    }

    fn digest_of(bytes: &[u8]) -> String {
        let mut h = Sha256::new();
        h.update(bytes);
        hex_digest(h)
    }

    #[tokio::test]
    async fn put_then_dedup_then_get_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        let payload = b"hello world, this is a blob".to_vec();
        let digest = digest_of(&payload);
        let mime = BlobMime::try_from("text/plain".to_string()).unwrap();

        // Upload.
        let (mut client, mut server) = duplex(1 << 16);
        let d1 = sha(&digest);
        let p1 = payload.clone();
        let blobs1 = blobs.clone();
        let mime1 = mime.clone();
        let task = tokio::spawn(async move {
            serve_put(
                &blobs1,
                &mut client,
                &mut server,
                &d1,
                p1.len() as u64,
                &mime1,
                None,
                None,
            )
            .await
        });
        // We need the other end to read/write; swap roles below using a second pair.
        drop(task);

        // Simpler: drive serve_put directly against a duplex pair where the
        // "client" side is simulated manually.
        let (mut to_host, mut from_phone) = duplex(1 << 16); // phone writes here, host reads from_phone
        let (mut to_phone, mut from_host) = duplex(1 << 16); // host writes here, phone reads from_host
        let d = sha(&digest);
        let m = mime.clone();
        let blobs2 = blobs.clone();
        let payload2 = payload.clone();
        let server_task = tokio::spawn(async move {
            serve_put(
                &blobs2,
                &mut from_phone,
                &mut to_phone,
                &d,
                payload2.len() as u64,
                &m,
                None,
                None,
            )
            .await
        });
        // Read first ok (have:false)
        let ok1 = read_control(&mut from_host).await;
        match ok1 {
            Control::Ok { data } => assert_eq!(data.unwrap()["have"], false),
            other => panic!("{other:?}"),
        }
        // Send body as kind-2 frames.
        to_host
            .write_all(&encode(FrameKind::Input, &payload))
            .await
            .unwrap();
        let exit = read_control(&mut from_host).await;
        assert!(matches!(exit, Control::Exit { code: Some(0) }));
        server_task.await.unwrap().unwrap();

        assert!(blobs.join(&digest).exists());

        // Second put of the same bytes: dedup.
        let (to_host2, mut from_phone2) = duplex(1 << 16);
        let (mut to_phone2, mut from_host2) = duplex(1 << 16);
        let d2 = sha(&digest);
        let m2 = mime.clone();
        let blobs3 = blobs.clone();
        let len = payload.len() as u64;
        let server_task2 = tokio::spawn(async move {
            serve_put(
                &blobs3,
                &mut from_phone2,
                &mut to_phone2,
                &d2,
                len,
                &m2,
                None,
                None,
            )
            .await
        });
        let ok2 = read_control(&mut from_host2).await;
        match ok2 {
            Control::Ok { data } => assert_eq!(data.unwrap()["have"], true),
            other => panic!("{other:?}"),
        }
        drop(to_host2);
        server_task2.await.unwrap().unwrap();

        // Get it back.
        let (mut to_phone3, mut from_host3) = duplex(1 << 16);
        let d3 = sha(&digest);
        let blobs4 = blobs.clone();
        let get_task = tokio::spawn(async move {
            serve_get(&blobs4, None, &mut to_phone3, Some(&d3), None, None).await
        });
        let ok3 = read_control(&mut from_host3).await;
        let size = match ok3 {
            Control::Ok { data } => data.unwrap()["size"].as_u64().unwrap(),
            other => panic!("{other:?}"),
        };
        assert_eq!(size, payload.len() as u64);
        let mut decoder = Decoder::new();
        let mut got = Vec::new();
        loop {
            let mut buf = [0u8; 4096];
            let n = tokio::io::AsyncReadExt::read(&mut from_host3, &mut buf)
                .await
                .unwrap();
            if n == 0 {
                break;
            }
            decoder.push(&buf[..n]).unwrap();
            while let Some(frame) = decoder.pop().unwrap() {
                match frame.kind {
                    unipi_proto::frame::FrameKind::Output => got.extend_from_slice(&frame.payload),
                    unipi_proto::frame::FrameKind::Control => {
                        let c: Control = serde_json::from_slice(&frame.payload).unwrap();
                        assert!(matches!(c, Control::Exit { code: Some(0) }));
                        break;
                    }
                    _ => {}
                }
            }
            if got.len() == payload.len() {
                // Drain the trailing exit control frame.
                let n2 = tokio::io::AsyncReadExt::read(&mut from_host3, &mut buf)
                    .await
                    .unwrap_or(0);
                if n2 > 0 {
                    decoder.push(&buf[..n2]).unwrap();
                }
                break;
            }
        }
        assert_eq!(got, payload);
        get_task.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn sha256_mismatch_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        let mime = BlobMime::try_from("text/plain".to_string()).unwrap();
        let (mut to_host, mut from_phone) = duplex(1 << 16);
        let (mut to_phone, mut from_host) = duplex(1 << 16);
        let wrong = sha(&"0".repeat(64));
        let payload = b"not matching this digest".to_vec();
        let blobs2 = blobs.clone();
        let len = payload.len() as u64;
        let wrong_for_task = wrong.clone();
        let server_task = tokio::spawn(async move {
            serve_put(
                &blobs2,
                &mut from_phone,
                &mut to_phone,
                &wrong_for_task,
                len,
                &mime,
                None,
                None,
            )
            .await
        });
        let _ = read_control(&mut from_host).await; // have:false
        to_host
            .write_all(&encode(FrameKind::Input, &payload))
            .await
            .unwrap();
        drop(to_host);
        let result = server_task.await.unwrap();
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_BLOB_REJECTED);
        assert!(!blobs.join(wrong.as_str()).exists());
    }

    #[tokio::test]
    async fn resume_with_offset_appends() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        let full = b"0123456789ABCDEF".to_vec();
        let digest = digest_of(&full);
        let mime = BlobMime::try_from("application/octet-stream".to_string()).unwrap();
        let d = sha(&digest);

        // Write a partial tmp file to simulate a prior interrupted attempt.
        let tmp_path = blobs.join(&digest).with_extension("part");
        std::fs::write(&tmp_path, &full[..8]).unwrap();

        let (mut to_host, mut from_phone) = duplex(1 << 16);
        let (mut to_phone, mut from_host) = duplex(1 << 16);
        let blobs2 = blobs.clone();
        let len = full.len() as u64;
        let rest = full[8..].to_vec();
        let server_task = tokio::spawn(async move {
            serve_put(
                &blobs2,
                &mut from_phone,
                &mut to_phone,
                &d,
                len,
                &mime,
                None,
                Some(8),
            )
            .await
        });
        let _ = read_control(&mut from_host).await; // have:false
        to_host
            .write_all(&encode(FrameKind::Input, &rest))
            .await
            .unwrap();
        let exit = read_control(&mut from_host).await;
        assert!(matches!(exit, Control::Exit { code: Some(0) }));
        server_task.await.unwrap().unwrap();
        assert_eq!(std::fs::read(blobs.join(&digest)).unwrap(), full);
    }

    #[tokio::test]
    async fn size_over_cap_is_rejected_before_any_io() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path().join("blobs");
        let mime = BlobMime::try_from("application/octet-stream".to_string()).unwrap();
        let d = sha(&"1".repeat(64));
        let (mut r, mut w) = duplex(16);
        let result = serve_put(
            &blobs,
            &mut r,
            &mut w,
            &d,
            BLOB_MAX_SIZE + 1,
            &mime,
            None,
            None,
        )
        .await;
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().code, AppErrorCode::E_BLOB_REJECTED);
    }

    #[test]
    fn evicts_oldest_first_until_under_cap() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path();
        for (name, size, age_secs) in [("a", 100usize, 3000u64), ("b", 100, 2000), ("c", 100, 1000)]
        {
            let path = blobs.join(name);
            std::fs::write(&path, vec![0u8; size]).unwrap();
            let past = SystemTime::now() - std::time::Duration::from_secs(age_secs);
            let times = std::fs::FileTimes::new().set_modified(past);
            std::fs::OpenOptions::new()
                .write(true)
                .open(&path)
                .unwrap()
                .set_times(times)
                .unwrap();
        }
        evict_lru(blobs, 150);
        assert!(!blobs.join("a").exists(), "oldest evicted first");
        assert!(blobs.join("c").exists(), "newest kept");
    }

    /// UNI-278: a just-uploaded (big) blob is never evicted by its own put.
    #[test]
    fn a_fresh_blob_survives_eviction() {
        let dir = tempfile::tempdir().unwrap();
        let blobs = dir.path();
        std::fs::write(blobs.join("old"), vec![0u8; 100]).unwrap();
        let past = SystemTime::now() - std::time::Duration::from_secs(3600);
        std::fs::OpenOptions::new()
            .write(true)
            .open(blobs.join("old"))
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(past))
            .unwrap();
        std::fs::write(blobs.join("big-new"), vec![0u8; 1000]).unwrap();
        evict_lru(blobs, 150);
        assert!(!blobs.join("old").exists());
        assert!(
            blobs.join("big-new").exists(),
            "over the cap on its own, but fresh: kept"
        );
    }

    #[test]
    fn path_allowed_checks_home_scope_for_files_screen() {
        let tmp = tempfile::tempdir().unwrap();
        let blobs = tmp.path().join("blobs");
        std::fs::create_dir_all(&blobs).unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let under_home = home.join("notes.txt");
        std::fs::write(&under_home, "x").unwrap();
        let outside = tmp.path().join("outside.txt");
        std::fs::write(&outside, "x").unwrap();
        let scope = FsScope::home_only(&home);
        assert!(path_allowed(&under_home, &blobs, Some(&scope)));
        assert!(!path_allowed(&outside, &blobs, Some(&scope)));
    }

    /// UNI-220: a file link to an extra safe root (e.g. /tmp) is served,
    /// but a symlink in it pointing elsewhere is not.
    #[test]
    fn path_allowed_serves_extra_safe_roots() {
        let tmp = tempfile::tempdir().unwrap();
        let blobs = tmp.path().join("blobs");
        let home = tmp.path().join("home");
        let extra = tmp.path().join("scratch");
        let outside = tmp.path().join("outside");
        for d in [&blobs, &home, &extra, &outside] {
            std::fs::create_dir_all(d).unwrap();
        }
        let shot = extra.join("shot.png");
        std::fs::write(&shot, "x").unwrap();
        let secret = outside.join("s.txt");
        std::fs::write(&secret, "s").unwrap();
        let scope = FsScope {
            home: home.clone(),
            extra: vec![extra.canonicalize().unwrap()],
            deny: vec![],
        };
        assert!(path_allowed(&shot, &blobs, Some(&scope)));
        assert!(!path_allowed(&secret, &blobs, Some(&scope)));
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&secret, extra.join("link.txt")).unwrap();
            assert!(!path_allowed(&extra.join("link.txt"), &blobs, Some(&scope)));
        }
        assert!(!path_allowed(
            Path::new("/etc/passwd"),
            &blobs,
            Some(&scope)
        ));
    }
}
