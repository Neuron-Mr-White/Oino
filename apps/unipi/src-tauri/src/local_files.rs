//! UNI-256: local files the desktop app's UI may attach — files dropped on
//! the window (Tauri drag & drop gives paths), files copied in the file
//! manager, and a screenshot on the clipboard.
//!
//! WebKitGTK hands the page an EMPTY `paste` event for an image clipboard
//! and a `text/uri-list` type whose data reads as "" — and a drop of files
//! from the file manager carries no `File`s. So the UI asks the core:
//!
//! - `clipboard_files()`: reads the GTK clipboard (the same one the webview
//!   uses) on the main thread — `x-special/gnome-copied-files` /
//!   `text/uri-list` → the copied files; `image/png` (or any image GTK can
//!   load, re-encoded as PNG) → a temp `pasted-<time>.png`. Nothing usable →
//!   an empty list (the UI then pastes the text as before).
//! - `local_files(paths)`: name/size/is_dir for paths from a drop.
//! - `local_file_bytes(path)`: the raw bytes (images: the composer shrinks
//!   and sends them inline) — only for paths the user handed over (dropped
//!   or pasted), never an arbitrary path the page asks for.
//!
//! Non-image files are uploaded straight from their path (`blob_put_file`).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::Serialize;
use unipi_proto::{AppErrorCode, AppErrorDto};

/// Paths the user handed to the app (dropped / pasted): the only ones
/// `local_file_bytes` reads.
static GRANTED: Mutex<Option<HashSet<PathBuf>>> = Mutex::new(None);

pub fn grant<I: IntoIterator<Item = PathBuf>>(paths: I) {
    if let Ok(mut g) = GRANTED.lock() {
        g.get_or_insert_with(HashSet::new).extend(paths);
    }
}

fn granted(path: &Path) -> bool {
    GRANTED
        .lock()
        .ok()
        .and_then(|g| g.as_ref().map(|s| s.contains(path)))
        .unwrap_or(false)
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LocalFile {
    pub path: String,
    pub name: String,
    pub size: u64,
    pub is_dir: bool,
}

fn describe(path: &Path) -> Option<LocalFile> {
    let meta = std::fs::metadata(path).ok()?;
    Some(LocalFile {
        path: path.to_string_lossy().into_owned(),
        name: path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.to_string_lossy().into_owned()),
        size: meta.len(),
        is_dir: meta.is_dir(),
    })
}

/// Pure: the local paths in a `text/uri-list` or `x-special/gnome-copied-files`
/// payload (`copy`/`cut` first line, `#` comments, CRLF; only `file://` URIs).
pub fn paths_from_uri_list(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            let rest = l.strip_prefix("file://")?;
            // file://host/path → only a local (empty or localhost) host.
            let path = if rest.starts_with('/') {
                rest
            } else {
                rest.strip_prefix("localhost")?
            };
            url::Url::parse(&format!("file://{path}"))
                .ok()?
                .to_file_path()
                .ok()
        })
        .collect()
}

/// Descriptions of dropped paths (the UI got them from Tauri's drag-drop event).
#[tauri::command]
pub fn local_files(paths: Vec<String>) -> Result<Vec<LocalFile>, AppErrorDto> {
    let paths: Vec<PathBuf> = paths.into_iter().map(PathBuf::from).collect();
    // Tauri already put them in its own scope on drop; we only hand back
    // what exists. Reading them still needs the drop grant (lib.rs).
    Ok(paths.iter().filter_map(|p| describe(p)).collect())
}

/// The bytes of a file the user dropped or pasted.
#[tauri::command]
pub fn local_file_bytes(path: String) -> Result<tauri::ipc::Response, AppErrorDto> {
    let p = PathBuf::from(&path);
    if !granted(&p) {
        return Err(AppErrorDto::new(
            AppErrorCode::E_SCOPE,
            "Only files you dropped or pasted can be read",
            false,
        ));
    }
    std::fs::read(&p)
        .map(tauri::ipc::Response::new)
        .map_err(|e| AppErrorDto::new(AppErrorCode::E_NOT_FOUND, format!("{path}: {e}"), false))
}

/// Files on the system clipboard (copied files, or a screenshot saved to a temp PNG).
#[tauri::command]
pub async fn clipboard_files(app: tauri::AppHandle) -> Result<Vec<LocalFile>, AppErrorDto> {
    let found = read_clipboard(&app).await?;
    let files: Vec<LocalFile> = found.iter().filter_map(|p| describe(p)).collect();
    grant(files.iter().map(|f| PathBuf::from(&f.path)));
    Ok(files)
}

#[cfg(target_os = "linux")]
async fn read_clipboard(app: &tauri::AppHandle) -> Result<Vec<PathBuf>, AppErrorDto> {
    let (tx, rx) = tokio::sync::oneshot::channel::<ClipData>();
    app.run_on_main_thread(move || gtk_read(tx))
        .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, format!("clipboard: {e}"), true))?;
    let data = tokio::time::timeout(std::time::Duration::from_secs(5), rx)
        .await
        .map_err(|_| {
            AppErrorDto::new(
                AppErrorCode::E_TIMEOUT,
                "The clipboard did not answer",
                true,
            )
        })?
        .unwrap_or(ClipData::None);
    match data {
        ClipData::None => Ok(Vec::new()),
        ClipData::Uris(text) => Ok(paths_from_uri_list(&text)),
        ClipData::Png(bytes) => {
            let dir = std::env::temp_dir().join("unipi-paste");
            std::fs::create_dir_all(&dir)
                .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))?;
            let stamp = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let path = dir.join(format!("pasted-{stamp}.png"));
            std::fs::write(&path, bytes)
                .map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false))?;
            Ok(vec![path])
        }
    }
}

/// WebView2 / WKWebView expose clipboard images and files to the page themselves.
#[cfg(not(target_os = "linux"))]
async fn read_clipboard(_app: &tauri::AppHandle) -> Result<Vec<PathBuf>, AppErrorDto> {
    Ok(Vec::new())
}

#[cfg(target_os = "linux")]
enum ClipData {
    None,
    Uris(String),
    Png(Vec<u8>),
}

/// Pure: which clipboard target to read, in order of preference.
pub fn pick_target(targets: &[String]) -> Option<&'static str> {
    let has = |t: &str| targets.iter().any(|x| x == t);
    if has("x-special/gnome-copied-files") {
        Some("x-special/gnome-copied-files")
    } else if has("text/uri-list") {
        Some("text/uri-list")
    } else if has("image/png") {
        Some("image/png")
    } else if targets.iter().any(|t| t.starts_with("image/")) {
        Some("image/*")
    } else {
        None
    }
}

#[cfg(target_os = "linux")]
fn gtk_read(tx: tokio::sync::oneshot::Sender<ClipData>) {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.request_contents(&gtk::gdk::Atom::intern("TARGETS"), move |cb, sel| {
        let targets: Vec<String> = sel
            .targets()
            .unwrap_or_default()
            .iter()
            .map(|a| a.name().to_string())
            .collect();
        log::info!("clipboard targets: {targets:?}");
        match pick_target(&targets) {
            Some("image/*") => cb.request_image(move |_, pixbuf| {
                let png = pixbuf.and_then(|p| p.save_to_bufferv("png", &[]).ok());
                let _ = tx.send(png.map(ClipData::Png).unwrap_or(ClipData::None));
            }),
            Some(target) => {
                let is_image = target == "image/png";
                cb.request_contents(&gtk::gdk::Atom::intern(target), move |_, sel| {
                    let data = sel.data();
                    let out = if data.is_empty() {
                        ClipData::None
                    } else if is_image {
                        ClipData::Png(data)
                    } else {
                        ClipData::Uris(String::from_utf8_lossy(&data).into_owned())
                    };
                    let _ = tx.send(out);
                })
            }
            None => {
                let _ = tx.send(ClipData::None);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_lists() {
        assert_eq!(
            paths_from_uri_list("copy\nfile:///tmp/a%20b.png\nfile:///home/x/n.txt"),
            vec![
                PathBuf::from("/tmp/a b.png"),
                PathBuf::from("/home/x/n.txt")
            ]
        );
        assert_eq!(
            paths_from_uri_list("# comment\r\nfile:///tmp/x\r\n"),
            vec![PathBuf::from("/tmp/x")]
        );
        assert_eq!(
            paths_from_uri_list("file://localhost/tmp/y"),
            vec![PathBuf::from("/tmp/y")]
        );
        assert!(
            paths_from_uri_list("https://example.com/a.png\nfile://otherhost/tmp/z\ncut")
                .is_empty()
        );
    }

    #[test]
    fn targets() {
        let t = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            pick_target(&t(&["text/plain", "image/png"])),
            Some("image/png")
        );
        assert_eq!(pick_target(&t(&["image/jpeg"])), Some("image/*"));
        assert_eq!(
            pick_target(&t(&[
                "text/uri-list",
                "x-special/gnome-copied-files",
                "text/plain"
            ])),
            Some("x-special/gnome-copied-files")
        );
        assert_eq!(
            pick_target(&t(&["text/uri-list", "image/png"])),
            Some("text/uri-list")
        );
        assert_eq!(pick_target(&t(&["UTF8_STRING", "text/plain"])), None);
    }

    #[test]
    fn grant_gates_reads() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("x.bin");
        std::fs::write(&f, b"hi").unwrap();
        assert!(local_file_bytes(f.to_string_lossy().into_owned()).is_err());
        grant([f.clone()]);
        assert!(local_file_bytes(f.to_string_lossy().into_owned()).is_ok());
        let d = local_files(vec![
            f.to_string_lossy().into_owned(),
            "/nope/missing".into(),
        ])
        .unwrap();
        assert_eq!(d.len(), 1);
        assert_eq!(
            (d[0].name.as_str(), d[0].size, d[0].is_dir),
            ("x.bin", 2, false)
        );
    }
}
