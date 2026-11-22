//! Model picks and the catalogue without a runtime (v3-perf §4.5, T6/T7).
//!
//! - `get_available_models` on a chat whose runtime is not up answers from the on-disk
//!   catalogue snapshot ([`oino_models::ModelCache::read_catalogue`]), which every live answer
//!   refreshes — the picker never waits for a wake or the network.
//! - `set_model` on such a chat appends the `model_change` entry straight to the session file
//!   under the session's single-writer lease (the same entry core writes; the runtime restores
//!   it when it comes up). A runtime that already holds the lease answers instead.

use std::io::{Read as _, Seek as _, SeekFrom, Write as _};
use std::path::Path;

use oino_models::ModelCache;
use oino_session::manager::{LeaseInfo, OffsetIndex, SessionLease};
use oino_session::{EntryId, EntryKind, IsoTimestamp, SessionEntry};
use serde_json::{json, Value};

use super::open::SessionFile;

/// The catalogue snapshot (`None`: none written yet).
#[must_use]
pub fn cached_catalogue(agent_dir: &Path) -> Option<Vec<Value>> {
    ModelCache::for_agent_dir(agent_dir).read_catalogue()
}

/// Stores a live `get_available_models` reply as the catalogue snapshot (rows without the
/// per-request readiness marks).
pub fn store_catalogue(agent_dir: &Path, reply: &Value) {
    let Some(rows) = reply.get("models").and_then(Value::as_array) else {
        return;
    };
    let rows: Vec<Value> = rows
        .iter()
        .map(|row| {
            let mut row = row.clone();
            if let Some(object) = row.as_object_mut() {
                object.remove("ready");
                object.remove("reason");
            }
            row
        })
        .collect();
    if let Err(error) = ModelCache::for_agent_dir(agent_dir).write_catalogue(&rows) {
        tracing::warn!(%error, "model catalogue snapshot not written");
    }
}

/// Appends `model_change` for `provider/model_id` when the catalogue lists it and the lease is
/// free. Returns the model row (the reply and the `model_changed` event); `None` = take the
/// runtime path (unknown model, a runtime holds the session, a torn or pre-v3 file).
#[must_use]
pub fn append_model_change(
    agent_dir: &Path,
    file: &SessionFile,
    provider: &str,
    model_id: &str,
) -> Option<Value> {
    let mut model = cached_catalogue(agent_dir)?.into_iter().find(|row| {
        row.get("provider").and_then(Value::as_str) == Some(provider)
            && row.get("id").and_then(Value::as_str) == Some(model_id)
    })?;
    if let Some(object) = model.as_object_mut() {
        object.remove("kind");
    }
    let holder = LeaseInfo::for_process("oino desktop", &agent_dir.join("state"));
    let lease = SessionLease::acquire(&file.path, &holder).ok()?;
    let written = append_under_lease(&file.path, &file.offsets, provider, model_id);
    lease.release();
    match written {
        Ok(()) => Some(model),
        Err(error) => {
            tracing::info!(%error, "set_model: record append skipped, waking instead");
            None
        }
    }
}

fn append_under_lease(
    path: &Path,
    offsets: &Path,
    provider: &str,
    model_id: &str,
) -> Result<(), String> {
    let index = OffsetIndex::load(path, Some(offsets)).map_err(|e| e.to_string())?;
    let mut handle = std::fs::OpenOptions::new()
        .read(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    // A torn tail needs core's repair: leave it to the runtime.
    let len = handle.metadata().map_err(|e| e.to_string())?.len();
    if len > 0 {
        let mut last = [0u8; 1];
        handle
            .seek(SeekFrom::Start(len - 1))
            .and_then(|_| handle.read_exact(&mut last))
            .map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            return Err("the file does not end with a complete line".into());
        }
    }
    let parent = index.leaf_id().map(str::to_owned);
    // Core's `push`: the active compaction generation comes from the parent.
    let compaction_summary_id = match &parent {
        Some(id) => index
            .entries_by_id(path, std::slice::from_ref(id))
            .map_err(|e| e.to_string())?
            .first()
            .and_then(|entry| {
                if entry.get("type").and_then(Value::as_str) == Some("compaction_state") {
                    Some(id.clone())
                } else {
                    entry
                        .get("compactionSummaryId")
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                }
            })
            .map(EntryId::new),
        None => None,
    };
    let entry = SessionEntry {
        id: EntryId::generate(|candidate| index.position(candidate).is_some()),
        parent_id: parent.map(EntryId::new),
        timestamp: IsoTimestamp::now(),
        compaction_summary_id,
        kind: EntryKind::ModelChange {
            provider: provider.to_owned(),
            model_id: model_id.to_owned(),
        },
    };
    let mut line = serde_json::to_string(&entry).map_err(|e| e.to_string())?;
    line.push('\n');
    let before = oino_session::manager::append_stamp(path).map_err(|e| e.to_string())?;
    handle
        .write_all(line.as_bytes())
        .map_err(|e| e.to_string())?;
    oino_session::manager::record_owned_append(path, before);
    handle.sync_data().map_err(|e| e.to_string())
}

/// The `model_changed` event a record-path pick publishes (core's event shape).
#[must_use]
pub fn model_changed_event(model: &Value) -> Value {
    json!({"type": "model_changed", "model": model, "source": "set"})
}
