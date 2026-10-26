//! Session discovery: listing, most-recent lookup and id search (pi `list`, `listAll`,
//! `findMostRecentSession`, `findById`).

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

use oino_ai::JsonValue;

use super::files::{cwd_session_dir, read_header};
use crate::entry::IsoTimestamp;
use crate::error::SessionError;
use crate::time::ms_to_system_time;

/// Summary of one session file (pi `SessionInfo`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionInfo {
    /// File path.
    pub path: PathBuf,
    /// Session id.
    pub id: String,
    /// Header cwd (empty when missing).
    pub cwd: String,
    /// Latest `session_info` name.
    pub name: Option<String>,
    /// Header `parentSession`.
    pub parent_session_path: Option<PathBuf>,
    /// Header timestamp.
    pub created: IsoTimestamp,
    /// Last user/assistant activity, else header time, else file mtime.
    pub modified: SystemTime,
    /// Number of `message` entries.
    pub message_count: usize,
    /// First user text, or `"(no messages)"`.
    pub first_message: String,
    /// All user/assistant texts joined with spaces (search corpus).
    pub all_messages_text: String,
}

/// Receives `(done, total)` while listing.
pub trait ProgressSink {
    /// Called after each file.
    fn progress(&self, done: usize, total: usize);
}

fn message_text(message: &JsonValue) -> Option<String> {
    match message.get("content")? {
        JsonValue::String(text) => Some(text.clone()),
        JsonValue::Array(blocks) => Some(
            blocks
                .iter()
                .filter(|b| b.get("type").and_then(JsonValue::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(JsonValue::as_str))
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

/// Builds the summary of one file; `None` when it is not a pi session file.
#[must_use]
pub fn session_info(path: &Path) -> Option<SessionInfo> {
    let file = fs::File::open(path).ok()?;
    let mtime = file.metadata().and_then(|m| m.modified()).ok();
    let mut header: Option<JsonValue> = None;
    let mut name = None;
    let mut message_count = 0;
    let mut first_message = String::new();
    let mut all_messages = Vec::new();
    let mut last_activity: Option<i64> = None;
    for line in BufReader::new(file).lines() {
        let Ok(line) = line else { break };
        let Ok(entry) = serde_json::from_str::<JsonValue>(&line) else {
            continue;
        };
        let kind = entry.get("type").and_then(JsonValue::as_str);
        if header.is_none() {
            if kind != Some("session") {
                return None;
            }
            header = Some(entry);
            continue;
        }
        if kind == Some("session_info") {
            name = entry
                .get("name")
                .and_then(JsonValue::as_str)
                .map(str::trim)
                .filter(|n| !n.is_empty())
                .map(str::to_owned);
        }
        if kind != Some("message") {
            continue;
        }
        message_count += 1;
        let Some(message) = entry.get("message") else {
            continue;
        };
        let role = message.get("role").and_then(JsonValue::as_str);
        if !matches!(role, Some("user" | "assistant")) || message.get("content").is_none() {
            continue;
        }
        let activity = message
            .get("timestamp")
            .and_then(JsonValue::as_i64)
            .or_else(|| {
                entry
                    .get("timestamp")
                    .and_then(JsonValue::as_str)
                    .and_then(|ts| IsoTimestamp::from_text(ts).to_ms())
            });
        if let Some(activity) = activity {
            last_activity = Some(last_activity.map_or(activity, |last| last.max(activity)));
        }
        let Some(text) = message_text(message).filter(|t| !t.is_empty()) else {
            continue;
        };
        if first_message.is_empty() && role == Some("user") {
            first_message.clone_from(&text);
        }
        all_messages.push(text);
    }
    let header = header?;
    let id = header.get("id").and_then(JsonValue::as_str)?.to_owned();
    let created = IsoTimestamp::from_text(
        header
            .get("timestamp")
            .and_then(JsonValue::as_str)
            .unwrap_or_default(),
    );
    let modified = last_activity
        .filter(|ms| *ms > 0)
        .or_else(|| created.to_ms())
        .map(ms_to_system_time)
        .or(mtime)
        .unwrap_or(SystemTime::UNIX_EPOCH);
    Some(SessionInfo {
        path: path.to_path_buf(),
        id,
        cwd: header
            .get("cwd")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned(),
        name,
        parent_session_path: header
            .get("parentSession")
            .and_then(JsonValue::as_str)
            .map(PathBuf::from),
        created,
        modified,
        message_count,
        first_message: if first_message.is_empty() {
            "(no messages)".into()
        } else {
            first_message
        },
        all_messages_text: all_messages.join(" "),
    })
}

/// `*.jsonl` files directly inside `dir` (missing dir ⇒ empty).
pub(crate) fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| entry.path())
                .filter(|path| path.extension().is_some_and(|ext| ext == "jsonl") && path.is_file())
                .collect()
        })
        .unwrap_or_default()
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

fn build_infos(
    files: &[PathBuf],
    progress: Option<&dyn ProgressSink>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<SessionInfo>, SessionError> {
    let mut infos = Vec::with_capacity(files.len());
    for (done, path) in files.iter().enumerate() {
        if cancel.is_some_and(|flag| flag.load(Ordering::Relaxed)) {
            return Err(SessionError::Cancelled);
        }
        infos.extend(session_info(path));
        if let Some(sink) = progress {
            sink.progress(done + 1, files.len());
        }
    }
    infos.sort_by_key(|info| std::cmp::Reverse(info.modified));
    Ok(infos)
}

/// Sessions of `cwd` (`<root>/--<cwd>--/`), newest modified first.
///
/// # Errors
///
/// [`SessionError::Cancelled`] when `cancel` is set.
pub(super) fn list(
    cwd: &Path,
    sessions_root: &Path,
    progress: Option<&dyn ProgressSink>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<SessionInfo>, SessionError> {
    let mut files = jsonl_files(&cwd_session_dir(sessions_root, cwd));
    files.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
    build_infos(&files, progress, cancel)
}

/// Sessions of every cwd directory under the root (dot-directories such as `.legacy` are
/// skipped), newest modified first.
///
/// # Errors
///
/// [`SessionError::Cancelled`] when `cancel` is set.
pub(super) fn list_all(
    sessions_root: &Path,
    progress: Option<&dyn ProgressSink>,
    cancel: Option<&AtomicBool>,
) -> Result<Vec<SessionInfo>, SessionError> {
    let mut files: Vec<(Option<SystemTime>, PathBuf)> = cwd_dirs(sessions_root)
        .iter()
        .flat_map(|dir| jsonl_files(dir))
        .map(|path| (mtime(&path), path))
        .collect();
    files.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.file_name().cmp(&a.1.file_name()))
    });
    let files: Vec<PathBuf> = files.into_iter().map(|(_, path)| path).collect();
    build_infos(&files, progress, cancel)
}

fn cwd_dirs(sessions_root: &Path) -> Vec<PathBuf> {
    fs::read_dir(sessions_root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|entry| !entry.file_name().to_string_lossy().starts_with('.'))
                .map(|entry| entry.path())
                .filter(|path| path.is_dir())
                .collect()
        })
        .unwrap_or_default()
}

/// Newest (by mtime) valid session file of `cwd`.
#[must_use]
pub(super) fn find_most_recent(cwd: &Path, sessions_root: &Path) -> Option<PathBuf> {
    let mut files: Vec<(Option<SystemTime>, PathBuf)> =
        jsonl_files(&cwd_session_dir(sessions_root, cwd))
            .into_iter()
            .map(|path| (mtime(&path), path))
            .collect();
    files.sort_by_key(|(time, _)| std::cmp::Reverse(*time));
    files
        .into_iter()
        .map(|(_, path)| path)
        .find(|path| read_header(path).is_some())
}

fn search(files: &[PathBuf], id: &str) -> Result<Option<PathBuf>, SessionError> {
    let headers: Vec<(PathBuf, String)> = files
        .iter()
        .filter_map(|path| read_header(path).map(|h| (path.clone(), h.id)))
        .collect();
    if let Some((path, _)) = headers.iter().find(|(_, header_id)| header_id == id) {
        return Ok(Some(path.clone()));
    }
    if id.len() < 4 {
        return Ok(None);
    }
    let matches: Vec<PathBuf> = headers
        .into_iter()
        .filter(|(_, header_id)| header_id.starts_with(id))
        .map(|(path, _)| path)
        .collect();
    match matches.len() {
        0 => Ok(None),
        1 => Ok(matches.into_iter().next()),
        _ => Err(SessionError::AmbiguousId(matches)),
    }
}

/// Finds a session by id: exact header id, then a unique prefix (≥ 4 chars) in the cwd
/// directory, then the same over all directories.
///
/// # Errors
///
/// [`SessionError::AmbiguousId`] when a prefix matches several files.
pub(super) fn find_by_id(
    cwd: &Path,
    id: &str,
    sessions_root: &Path,
) -> Result<Option<PathBuf>, SessionError> {
    let cwd_dir = cwd_session_dir(sessions_root, cwd);
    if let Some(found) = search(&jsonl_files(&cwd_dir), id)? {
        return Ok(Some(found));
    }
    let all: Vec<PathBuf> = cwd_dirs(sessions_root)
        .iter()
        .filter(|dir| **dir != cwd_dir)
        .flat_map(|dir| jsonl_files(dir))
        .collect();
    search(&all, id)
}
