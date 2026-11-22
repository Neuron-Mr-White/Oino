//! The `turn` / `diff` / `undo` / `undoFrom` services the app calls with
//! `_oino/service_call{target:"unipi.file-summary", name}` (app.md §3.13).
//!
//! State is read back from the session `custom` entries, falling back to the kv in-progress
//! index of a turn that has not ended yet (DIRECTION §3.1.6): a restarted host loses nothing.

use std::path::Path;

use oino_extension::context::Ctx;
use oino_extension::session;
use serde_json::{Value, json};

use crate::Result;
use crate::snapshot;
use crate::store::{
    FileRecord, TURN_CUSTOM_TYPE, TurnRecord, UNDO_CUSTOM_TYPE, UndoRecord, delete_index,
    load_index, read_blob,
};

/// Turn-footer renderer key (`register-renderer{target: turn-footer, key: "files"}`).
pub const FOOTER_KEY: &str = "files";

/// One turn summary with its undo state.
#[derive(Clone, Debug, Default)]
pub struct Summary {
    /// The turn.
    pub record: TurnRecord,
    /// Paths an undo put back into their pre-turn state.
    pub undone_paths: Vec<String>,
    /// The summary comes from the kv in-progress index (turn still running).
    pub running: bool,
}

impl Summary {
    /// Files with their undo state, for the footer.
    #[must_use]
    pub fn is_undone(&self, path: &str) -> bool {
        self.undone_paths.iter().any(|undone| undone == path)
    }
}

/// The entries the turn/undo bookkeeping reads, in file order (perf-fix #1: not the whole
/// transcript): its own records, plus the user messages a turn id names when
/// `with_turn_messages` (undo needs their positions; positions index this list).
fn index_entries(with_turn_messages: bool) -> Vec<Value> {
    let mut types = vec!["custom".to_owned()];
    if with_turn_messages {
        types.push("message".to_owned());
    }
    session::query(&session::EntryQuery {
        types,
        custom_types: vec![TURN_CUSTOM_TYPE.into(), UNDO_CUSTOM_TYPE.into()],
        roles: vec!["user".into()],
        ..session::EntryQuery::default()
    })
    .unwrap_or_default()
}

/// The custom entries of `custom_type` in `entries`, with their positions.
fn custom_entries_in<T: serde::de::DeserializeOwned>(
    entries: &[Value],
    custom_type: &str,
) -> Vec<(usize, T)> {
    entries
        .iter()
        .enumerate()
        .filter(|(_, entry)| {
            entry.get("type").and_then(Value::as_str) == Some("custom")
                && entry.get("customType").and_then(Value::as_str) == Some(custom_type)
        })
        .filter_map(|(index, entry)| {
            let data = entry.get("data")?.clone();
            serde_json::from_value(data).ok().map(|data| (index, data))
        })
        .collect()
}

/// One custom entry of `custom_type` in file order, with its position.
fn custom_entries<T: serde::de::DeserializeOwned>(custom_type: &str) -> Vec<(usize, T)> {
    custom_entries_in(&index_entries(false), custom_type)
}

/// Every finished turn, in file order.
#[must_use]
pub fn turns() -> Vec<(usize, TurnRecord)> {
    custom_entries::<TurnRecord>(TURN_CUSTOM_TYPE)
}

/// Every undo, in file order.
#[must_use]
fn undos() -> Vec<(usize, UndoRecord)> {
    custom_entries::<UndoRecord>(UNDO_CUSTOM_TYPE)
}

/// Paths undone for `turn_entry_id` (restored or deleted).
#[must_use]
pub fn undone_paths(turn_entry_id: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for (_, undo) in undos() {
        if undo.turn_entry_id != turn_entry_id {
            continue;
        }
        for path in undo.undone_paths() {
            if !out.iter().any(|known| known == path) {
                out.push(path.to_owned());
            }
        }
    }
    out
}

/// Summary of one turn: the `custom` entry, else the kv in-progress index, else empty.
#[must_use]
pub fn summary(turn_entry_id: &str) -> Summary {
    let found = turns()
        .into_iter()
        .rev()
        .find(|(_, record)| record.turn_entry_id == turn_entry_id);
    let (record, running) = match found {
        Some((_, record)) => (record, false),
        None => match load_index(turn_entry_id) {
            Some(record) => (record, true),
            None => (
                TurnRecord {
                    turn_entry_id: turn_entry_id.to_owned(),
                    files: Vec::new(),
                },
                false,
            ),
        },
    };
    Summary {
        record,
        undone_paths: undone_paths(turn_entry_id),
        running,
    }
}

/// The summary of the turn that is running right now (a live turn has no entry id yet, so
/// the frontend sends none): the kv index of the current user message, else the newest
/// in-progress index of the session.
#[must_use]
pub fn run_key() -> Option<Summary> {
    if let Some(turn_entry_id) = snapshot::current_turn()
        && let Some(record) = load_index(&turn_entry_id)
    {
        return Some(Summary {
            undone_paths: undone_paths(&record.turn_entry_id),
            record,
            running: true,
        });
    }
    let mut candidates = crate::store::load_indexes();
    candidates.sort_by(|a, b| a.turn_entry_id.cmp(&b.turn_entry_id));
    let record = candidates.pop()?;
    Some(Summary {
        undone_paths: undone_paths(&record.turn_entry_id),
        record,
        running: true,
    })
}

fn optional_turn(payload: &Value) -> Result<String> {
    payload
        .get("turnEntryId")
        .and_then(Value::as_str)
        .filter(|turn| !turn.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| oino_extension::Error::msg("turnEntryId is required"))
}

fn payload_path(payload: &Value) -> Result<String> {
    payload
        .get("path")
        .and_then(Value::as_str)
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| oino_extension::Error::msg("path is required"))
}

/// `turn{turnEntryId}` → `{turnEntryId, files, undone, undonePaths, running}`.
///
/// # Errors
///
/// `turnEntryId` is missing.
pub fn turn(payload: &Value) -> Result<Value> {
    let turn_entry_id = optional_turn(payload)?;
    let summary = summary(&turn_entry_id);
    Ok(json!({
        "turnEntryId": summary.record.turn_entry_id,
        "files": summary.record.files,
        "undone": !summary.undone_paths.is_empty(),
        "undonePaths": summary.undone_paths,
        "running": summary.running,
    }))
}

fn record_of(turn_entry_id: &str, path: &str) -> Option<FileRecord> {
    summary(turn_entry_id)
        .record
        .files
        .into_iter()
        .find(|file| file.path == path)
}

/// `diff{turnEntryId, path}` → `{turnEntryId, path, diff, truncated}` (unified, ≤ 1 MiB).
/// `diff` is `null` with a `reason` when there is no snapshot to compare against.
#[must_use]
pub fn diff(payload: &Value, cx: &Ctx) -> Value {
    let Ok(turn_entry_id) = optional_turn(payload) else {
        return json!({"diff": null, "reason": "turnEntryId is required"});
    };
    let Ok(path) = payload_path(payload) else {
        return json!({"turnEntryId": turn_entry_id, "diff": null, "reason": "path is required"});
    };
    let reply = |diff: Value, truncated: bool, reason: Option<&str>| {
        let mut out = json!({
            "turnEntryId": turn_entry_id,
            "path": path,
            "diff": diff,
            "truncated": truncated,
        });
        if let Some(reason) = reason {
            out["reason"] = json!(reason);
        }
        out
    };
    let Some(record) = record_of(&turn_entry_id, &path) else {
        return reply(Value::Null, false, Some("not_found"));
    };
    let before = if record.existed {
        let Some(bytes) = record
            .before_sha
            .as_deref()
            .and_then(|sha| read_blob(&session::session_id(), sha))
        else {
            return reply(Value::Null, false, Some("no_snapshot"));
        };
        bytes
    } else {
        Vec::new()
    };
    let Some(resolved) = snapshot::resolve(&cx.cwd(), &path) else {
        return reply(Value::Null, false, Some("unresolvable"));
    };
    let after = match std::fs::read(&resolved.guest) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => return reply(Value::Null, false, Some("unreadable")),
    };
    let (text, truncated) = crate::diff::unified(
        &path,
        &String::from_utf8_lossy(&before),
        &String::from_utf8_lossy(&after),
    );
    reply(json!(text), truncated, None)
}

fn fire_invalidate(turn_entry_ids: &[String]) {
    let request = if turn_entry_ids.is_empty() {
        json!({"method": "invalidateTurnFooter", "key": FOOTER_KEY})
    } else {
        json!({
            "method": "invalidateTurnFooter",
            "key": FOOTER_KEY,
            "turnEntryIds": turn_entry_ids,
        })
    };
    let _ = oino_extension::ui::fire(&request);
}

fn current_sha(path: &Path) -> std::io::Result<Option<String>> {
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => crate::store::hash_file(path).map(Some),
        Ok(_) => Ok(Some(String::new())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

/// Undoes one turn's files. Never touches a path a later change modified unless `force`.
fn undo_turn(
    session_id: &str,
    record: &TurnRecord,
    paths: Option<&[String]>,
    force: bool,
    cwd: &Path,
) -> UndoRecord {
    let mut undo = UndoRecord {
        turn_entry_id: record.turn_entry_id.clone(),
        paths: paths.map(<[String]>::to_vec),
        force,
        ..UndoRecord::default()
    };
    for file in &record.files {
        if file.is_skipped() {
            continue;
        }
        if let Some(paths) = paths
            && !paths.iter().any(|wanted| wanted == &file.path)
        {
            continue;
        }
        let Some(resolved) = snapshot::resolve(cwd, &file.path) else {
            undo.failed.push(file.path.clone());
            continue;
        };
        let Ok(current) = current_sha(&resolved.guest) else {
            undo.failed.push(file.path.clone());
            continue;
        };
        if current != file.after_sha && !force {
            undo.conflicts.push(file.path.clone());
            continue;
        }
        if file.existed {
            let Some(bytes) = file
                .before_sha
                .as_deref()
                .and_then(|sha| read_blob(session_id, sha))
            else {
                undo.failed.push(file.path.clone());
                continue;
            };
            let written = unipi_kit::paths::ensure_parent(&resolved.guest)
                .map_err(|error| oino_extension::Error::msg(error.to_string()))
                .and_then(|()| {
                    oino_extension::fs::atomic_write(&resolved.guest.to_string_lossy(), &bytes)
                });
            if written.is_ok() {
                undo.restored.push(file.path.clone());
            } else {
                undo.failed.push(file.path.clone());
            }
        } else {
            match std::fs::remove_file(&resolved.guest) {
                Ok(()) => undo.deleted.push(file.path.clone()),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                    undo.deleted.push(file.path.clone());
                }
                Err(_) => undo.failed.push(file.path.clone()),
            }
        }
    }
    undo
}

fn append_undo(undo: &UndoRecord) {
    if let Err(error) = session::append_entry(UNDO_CUSTOM_TYPE, Some(undo)) {
        let _ = oino_extension::ui::fire(&json!({
            "method": "notify",
            "message": format!("file summary: could not record the undo: {error}"),
            "notifyType": "error",
        }));
    }
}

/// `undo{turnEntryId, paths?, force?}` → `{restored, deleted, conflicts, failed, found}`.
///
/// # Errors
///
/// `turnEntryId` is missing.
pub fn undo(payload: &Value, cx: &Ctx) -> Result<Value> {
    let turn_entry_id = optional_turn(payload)?;
    let paths: Option<Vec<String>> = payload
        .get("paths")
        .and_then(|value| serde_json::from_value(value.clone()).ok());
    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let Some(record) = turns()
        .into_iter()
        .rev()
        .find(|(_, record)| record.turn_entry_id == turn_entry_id)
        .map(|(_, record)| record)
    else {
        return Ok(json!({
            "restored": [], "deleted": [], "conflicts": [], "failed": [], "found": false
        }));
    };
    let session_id = session::session_id();
    let undo = undo_turn(&session_id, &record, paths.as_deref(), force, &cx.cwd());
    append_undo(&undo);
    fire_invalidate(&[turn_entry_id]);
    Ok(json!({
        "restored": undo.restored,
        "deleted": undo.deleted,
        "conflicts": undo.conflicts,
        "failed": undo.failed,
        "found": true,
    }))
}

/// `undoFrom{turnEntryId, force?}` → undoes this and every later turn, newest first.
///
/// # Errors
///
/// `turnEntryId` is missing.
pub fn undo_from(payload: &Value, cx: &Ctx) -> Result<Value> {
    let turn_entry_id = optional_turn(payload)?;
    let force = payload
        .get("force")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let entries = index_entries(true);
    let position = entries
        .iter()
        .position(|entry| entry.get("id").and_then(Value::as_str) == Some(turn_entry_id.as_str()));
    let mut later: Vec<(usize, TurnRecord)> = custom_entries_in(&entries, TURN_CUSTOM_TYPE)
        .into_iter()
        .filter(|(index, _)| position.is_none_or(|position| *index > position))
        .collect();
    later.sort_by_key(|(index, _)| std::cmp::Reverse(*index));
    let session_id = session::session_id();
    let cwd = cx.cwd();
    let mut all = UndoRecord {
        turn_entry_id: turn_entry_id.clone(),
        force,
        ..UndoRecord::default()
    };
    let mut undone_turns = Vec::new();
    for (_, record) in later {
        let undo = undo_turn(&session_id, &record, None, force, &cwd);
        append_undo(&undo);
        all.restored.extend(undo.restored);
        all.deleted.extend(undo.deleted);
        all.conflicts.extend(undo.conflicts);
        all.failed.extend(undo.failed);
        undone_turns.push(record.turn_entry_id);
    }
    if undone_turns.is_empty() {
        return Ok(json!({
            "turns": [],
            "restored": [], "deleted": [], "conflicts": [], "failed": [],
        }));
    }
    fire_invalidate(&undone_turns);
    Ok(json!({
        "turns": undone_turns,
        "restored": all.restored,
        "deleted": all.deleted,
        "conflicts": all.conflicts,
        "failed": all.failed,
    }))
}

/// Deletes the kv index of a finished turn.
pub fn clear_index(turn_entry_id: &str) {
    delete_index(turn_entry_id);
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, reason = "tests assert by unwrapping")]
    use super::*;

    #[test]
    fn summary_is_undone_by_path() {
        let summary = Summary {
            undone_paths: vec!["a".into()],
            ..Summary::default()
        };
        assert!(summary.is_undone("a"));
        assert!(!summary.is_undone("b"));
    }

    #[test]
    fn required_payload_fields() {
        assert!(optional_turn(&json!({})).is_err());
        assert!(optional_turn(&json!({"turnEntryId": ""})).is_err());
        assert_eq!(optional_turn(&json!({"turnEntryId": "t1"})).unwrap(), "t1");
        assert!(payload_path(&json!({"path": ""})).is_err());
    }
}
