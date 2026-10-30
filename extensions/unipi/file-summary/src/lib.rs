//! `unipi.file-summary` — content-addressed file snapshots before `write`/`edit`, one summary
//! per turn, a turn-footer renderer and per-turn undo (app.md §3.13, zcode-audit D-1).
//!
//! | Piece | Behaviour |
//! |---|---|
//! | [`snapshot`] | on `tool_call` of the built-in `write`/`edit`: first touch of a path per turn reads the file (≤ 5 MB), zstd-compresses it into `<ext-session>/<sid>/blobs/<sha256>.zst` (dedup), records `existed:false` for a new file and `skipped:"too_large"` over the cap; `bash` and extension tools are not captured |
//! | [`store`] | blob store, kv in-progress index `turn:<turnEntryId>`, `unipi.file-summary/turn` and `/undo` custom entry payloads |
//! | `agent_end` | fills `afterSha` and the `similar` line counts `+add −del`, appends the turn custom entry, deletes the kv index, invalidates the footer |
//! | [`services`] | `turn` / `diff` / `undo` / `undoFrom` (`_oino/service_call{target:"unipi.file-summary"}`) |
//! | [`footer`] | turn-footer renderer `files` ("3 files +12 −04" + file rows) and the reserved id `unipi.file-summary/turn-files` for the app's native component |
//!
//! State is rebuildable from session `custom` entries + kv (DIRECTION §3.1.6): a host killed
//! mid-turn resumes with the in-progress index, and every blob is found by its sha256.
//!
//! Defaults taken where the spec leaves a choice: the display path of a snapshot is
//! project-relative when the file is inside the session cwd (absolute otherwise); `+`/`−`
//! counts are zero-padded to two digits ("+12 −04"); a `skipped` file is listed but never
//! undone; `undoFrom` also covers turns whose user message is no longer in the file (then only
//! the target turn is undone); a live turn without a `turnEntryId` renders the kv index of the
//! current user message, else the newest one.

#![allow(
    clippy::missing_errors_doc,
    reason = "fallible helpers forward the host import error (host-error text or JSON), documented once at the crate level"
)]
#![allow(
    clippy::unused_async_trait_impl,
    reason = "`Extension::init` is async in the SDK; registration here is synchronous"
)]

pub mod diff;
pub mod footer;
pub mod services;
pub mod snapshot;
pub mod store;

use std::collections::BTreeMap;
use std::sync::{Mutex, MutexGuard, PoisonError};

use oino_extension::prelude::*;
use oino_extension::wit::registry::RendererTarget;

use crate::store::{FileRecord, TURN_CUSTOM_TYPE, TurnRecord};

/// In-progress turn indexes: session id → turn entry id → touched files, first touch first.
type Index = BTreeMap<String, BTreeMap<String, Vec<FileRecord>>>;

/// The extension.
#[derive(Default)]
pub struct FileSummary {
    index: Mutex<Index>,
}

impl FileSummary {
    fn index(&self) -> MutexGuard<'_, Index> {
        self.index.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Files already snapshotted in this turn.
    fn files(&self, session_id: &str, turn_entry_id: &str) -> Vec<FileRecord> {
        self.index()
            .get(session_id)
            .and_then(|turns| turns.get(turn_entry_id))
            .cloned()
            .unwrap_or_default()
    }

    /// Restores the in-progress indexes of a resumed session from kv.
    fn reindex(&self, session_id: &str) {
        let mut index = self.index();
        let turns = index.entry(session_id.to_owned()).or_default();
        turns.clear();
        for record in store::load_indexes() {
            turns.insert(record.turn_entry_id, record.files);
        }
    }

    /// First snapshot of `path` in the current turn, before the tool runs, persisted to kv.
    fn capture(&self, cx: &Ctx, payload: &Value) {
        let tool = payload.get("toolName").and_then(Value::as_str);
        if !tool.is_some_and(snapshot::captures) {
            return;
        }
        let Some(path) = payload.get("input").and_then(snapshot::tool_path) else {
            return;
        };
        let Some(turn_entry_id) = snapshot::current_turn() else {
            return;
        };
        let Some(resolved) = snapshot::resolve(&cx.cwd(), path) else {
            return;
        };
        let session_id = cx.session_id().to_owned();
        let existing = self.files(&session_id, &turn_entry_id);
        let Some(record) = snapshot::snapshot(&session_id, &resolved, &existing) else {
            return;
        };
        let mut index = self.index();
        let files = index
            .entry(session_id)
            .or_default()
            .entry(turn_entry_id.clone())
            .or_default();
        if files.iter().any(|file| file.path == record.path) {
            return;
        }
        files.push(record);
        let turn = TurnRecord {
            turn_entry_id,
            files: files.clone(),
        };
        let _ = store::save_index(&turn);
    }

    /// `agent_end`: finishes the turn record from the in-memory and kv indexes and appends it.
    fn flush(&self, cx: &Ctx) {
        let session_id = cx.session_id().to_owned();
        let cwd = cx.cwd();
        let mut pending: BTreeMap<String, Vec<FileRecord>> = store::load_indexes()
            .into_iter()
            .map(|record| (record.turn_entry_id, record.files))
            .collect();
        let in_memory = self.index().remove(&session_id).unwrap_or_default();
        pending.extend(in_memory);
        let mut changed = Vec::new();
        for (turn_entry_id, files) in pending {
            let finished: Vec<FileRecord> = files
                .into_iter()
                .filter_map(|file| snapshot::finish(&session_id, &cwd, file))
                .collect();
            if finished.is_empty() {
                store::delete_index(&turn_entry_id);
                continue;
            }
            let turn = TurnRecord {
                turn_entry_id: turn_entry_id.clone(),
                files: finished,
            };
            if session::append_entry(TURN_CUSTOM_TYPE, Some(&turn)).is_ok() {
                store::delete_index(&turn_entry_id);
                changed.push(turn_entry_id);
            }
        }
        if !changed.is_empty() {
            let _ = ui::fire(&json!({
                "method": "invalidateTurnFooter",
                "key": footer::KEY,
                "turnEntryIds": changed,
            }));
        }
    }
}

impl Extension for FileSummary {
    async fn init(&'static self, cx: &mut Init<Self>) -> Result<()> {
        ui::register_turn_footer(footer::KEY)?;
        cx.renderer(
            RendererTarget::TurnFooter,
            footer::KEY,
            |_ext, request, cx| footer::render(&request, &cx),
        );
        cx.service("turn", |_ext, payload, _caller, _cx| async move {
            services::turn(&payload)
        });
        cx.service("diff", |_ext, payload, _caller, cx| async move {
            Ok(services::diff(&payload, &cx))
        });
        cx.service("undo", |_ext, payload, _caller, cx| async move {
            services::undo(&payload, &cx)
        });
        cx.service("undoFrom", |_ext, payload, _caller, cx| async move {
            services::undo_from(&payload, &cx)
        });
        cx.on(EventKind::SessionStart, |ext, _event, cx| async move {
            ext.reindex(cx.session_id());
            Ok(())
        });
        cx.on(EventKind::ToolCall, |ext, event, cx| async move {
            ext.capture(&cx, &event.payload);
            Ok(())
        });
        cx.on(EventKind::AgentEnd, |ext, _event, cx| async move {
            ext.flush(&cx);
            Ok(())
        });
        Ok(())
    }
}

oino_extension::export_extension!(FileSummary);
