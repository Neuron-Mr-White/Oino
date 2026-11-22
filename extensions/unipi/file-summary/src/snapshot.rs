//! Capture: resolve a `write`/`edit` target to a preopen and snapshot it before the tool runs
//! (app.md §3.13, zcode-audit D-1), then finish the turn record after `agent_end`.
//!
//! Only the built-in `write` and `edit` tools are captured; `bash`, extension tools and every
//! other tool are not (`ZCode` behaves the same). One snapshot per path and turn: the first
//! touch wins.

use std::path::{Component, Path, PathBuf};

use oino_extension::context::Ctx;
use oino_extension::fs::Preopen;
use serde_json::Value;

use crate::store::{
    FileRecord, MAX_SNAPSHOT_BYTES, TOO_LARGE, UNREADABLE, hash_hex, read_blob, store_blob,
};

/// Name of the preopen that holds the session cwd (the project).
const PROJECT: &str = "project";

/// One resolved tool target.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Resolved {
    /// Path in the guest namespace (a preopen path).
    pub guest: PathBuf,
    /// Path shown in summaries and custom entries.
    pub display: String,
}

/// Whether `tool` is a built-in tool whose target is snapshotted.
#[must_use]
pub fn captures(tool: &str) -> bool {
    matches!(tool, "write" | "edit")
}

/// The `path` argument of a `write`/`edit` call.
#[must_use]
pub fn tool_path(args: &Value) -> Option<&str> {
    args.get("path")
        .and_then(Value::as_str)
        .filter(|p| !p.is_empty())
}

/// Lexically normalises `.`/`..` (the target may not exist yet, so no `canonicalize`).
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

fn display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

/// Resolves `path` (relative to `cwd`) to the longest matching preopen. `None` when the path
/// lies outside every preopen granted to the extension (then it is not captured).
#[must_use]
pub fn resolve(cwd: &Path, path: &str) -> Option<Resolved> {
    let raw = Path::new(path);
    let absolute = if raw.is_absolute() {
        raw.to_path_buf()
    } else {
        cwd.join(raw)
    };
    let absolute = normalize(&absolute);
    let mut best: Option<(usize, Preopen, PathBuf)> = None;
    for preopen in oino_extension::fs::preopens() {
        let host = normalize(Path::new(&preopen.host_path));
        let Ok(rest) = absolute.strip_prefix(&host) else {
            continue;
        };
        let depth = host.as_os_str().len();
        if best
            .as_ref()
            .is_none_or(|(best_depth, _, _)| depth > *best_depth)
        {
            best = Some((depth, preopen, rest.to_path_buf()));
        }
    }
    let (_, preopen, rest) = best?;
    let guest = PathBuf::from(&preopen.guest_path).join(&rest);
    let display = if preopen.name == PROJECT {
        display_path(&rest)
    } else {
        display_path(&absolute)
    };
    Some(Resolved { guest, display })
}

/// The turn id: the id of the last user message entry that started the current run (equals
/// the app's `turnId`, ui-protocol §6.7).
#[must_use]
pub fn current_turn() -> Option<String> {
    // perf-fix #1 (this runs before every file tool call): the last user message only.
    let entries = oino_extension::session::query(
        &oino_extension::session::EntryQuery::messages(&["user"]).last(1),
    )
    .ok()?;
    entries
        .iter()
        .rev()
        .find(|entry| {
            entry.get("type").and_then(Value::as_str) == Some("message")
                && entry.pointer("/message/role").and_then(Value::as_str) == Some("user")
        })
        .and_then(|entry| entry.get("id").and_then(Value::as_str))
        .map(str::to_owned)
}

/// Snapshots one path before the tool runs. `None` when the path already has a record in
/// `existing` (once per path per turn) or cannot be resolved.
#[must_use]
pub fn snapshot(
    session_id: &str,
    resolved: &Resolved,
    existing: &[FileRecord],
) -> Option<FileRecord> {
    if existing.iter().any(|file| file.path == resolved.display) {
        return None;
    }
    let mut record = FileRecord {
        path: resolved.display.clone(),
        existed: false,
        before_sha: None,
        after_sha: None,
        add: 0,
        del: 0,
        skipped: None,
    };
    let metadata = match std::fs::metadata(&resolved.guest) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Some(record),
        Err(_) => {
            record.skipped = Some(UNREADABLE.to_owned());
            return Some(record);
        }
    };
    record.existed = true;
    if !metadata.is_file() {
        record.skipped = Some(UNREADABLE.to_owned());
        return Some(record);
    }
    if metadata.len() > MAX_SNAPSHOT_BYTES {
        record.skipped = Some(TOO_LARGE.to_owned());
        return Some(record);
    }
    let Ok(bytes) = std::fs::read(&resolved.guest) else {
        record.skipped = Some(UNREADABLE.to_owned());
        return Some(record);
    };
    let sha = hash_hex(&bytes);
    if store_blob(session_id, &sha, &bytes).is_err() {
        record.skipped = Some(UNREADABLE.to_owned());
        return Some(record);
    }
    record.before_sha = Some(sha);
    Some(record)
}

/// The file as the turn left it.
enum After {
    /// The path does not exist (deleted or never created).
    Missing,
    /// Read, hashed and stored under `sha`.
    Data { bytes: Vec<u8>, sha: String },
    /// Exists but is too large or unreadable.
    Skipped(&'static str),
}

fn read_after(session_id: &str, resolved: &Resolved) -> After {
    let metadata = match std::fs::metadata(&resolved.guest) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return After::Missing,
        Err(_) => return After::Skipped(UNREADABLE),
    };
    if !metadata.is_file() {
        return After::Skipped(UNREADABLE);
    }
    if metadata.len() > MAX_SNAPSHOT_BYTES {
        return After::Skipped(TOO_LARGE);
    }
    let Ok(bytes) = std::fs::read(&resolved.guest) else {
        return After::Skipped(UNREADABLE);
    };
    let sha = hash_hex(&bytes);
    if store_blob(session_id, &sha, &bytes).is_err() {
        return After::Skipped(UNREADABLE);
    }
    After::Data { bytes, sha }
}

/// Fills `afterSha` and the `+add −del` counts of one record at `agent_end`. `None` when the
/// turn left the path exactly as it was (created and removed again, or never created).
#[must_use]
pub fn finish(session_id: &str, cwd: &Path, mut record: FileRecord) -> Option<FileRecord> {
    if record.is_skipped() {
        return Some(record);
    }
    let Some(resolved) = resolve(cwd, &record.path) else {
        record.skipped = Some(UNREADABLE.to_owned());
        return Some(record);
    };
    let before_text = record
        .before_sha
        .as_deref()
        .and_then(|sha| read_blob(session_id, sha))
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default();
    match read_after(session_id, &resolved) {
        After::Missing => {
            if !record.existed {
                return None;
            }
            let (add, del) = crate::diff::line_counts(&before_text, "");
            record.add = add;
            record.del = del;
        }
        After::Skipped(reason) => {
            record.skipped = Some(reason.to_owned());
        }
        After::Data { bytes, sha } => {
            let after_text = String::from_utf8_lossy(&bytes);
            let (add, del) = crate::diff::line_counts(&before_text, &after_text);
            record.add = add;
            record.del = del;
            record.after_sha = Some(sha);
        }
    }
    Some(record)
}

/// The extension's call context cwd as a host path.
#[must_use]
pub fn cwd(cx: &Ctx) -> PathBuf {
    cx.cwd()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests assert by unwrapping")]
    use super::*;
    use serde_json::json;

    #[test]
    fn only_write_and_edit_capture() {
        assert!(captures("write"));
        assert!(captures("edit"));
        assert!(!captures("bash"));
        assert!(!captures("read"));
        assert!(!captures("my_extension_write"));
    }

    #[test]
    fn path_argument() {
        assert_eq!(tool_path(&json!({"path": "src/a.rs"})), Some("src/a.rs"));
        assert_eq!(tool_path(&json!({"path": ""})), None);
        assert_eq!(tool_path(&json!({})), None);
    }

    #[test]
    fn normalizes_dot_segments() {
        assert_eq!(normalize(Path::new("/p/./a/../b")), PathBuf::from("/p/b"));
        assert_eq!(display_path(Path::new("/p/a/b")), "/p/a/b");
        assert_eq!(display_path(Path::new("a/b")), "a/b");
    }
}
