//! Fast session open (perf-open): history first, runtime later.
//!
//! Waking a runtime parses the **whole** session file (seconds and GBs for a 200 MB chat) and
//! shipping the whole branch blew the 1 MiB frame. An open now:
//!
//! 1. answers with the **tail page** of the current branch read straight from the file
//!    through the entry offset index ([`oino_session::manager::OffsetIndex`]): no full parse,
//!    `page` says how many entries the branch holds and where the next older page starts;
//! 2. marks the snapshot `waking` with a **provisional state** (name, cwd, the model of the
//!    last assistant reply) and wakes the runtime in the background; the `{kind:"woken"}`
//!    event carries the real `state`/`commands`/`ui`/`topics` (a failure: `wake_failed`);
//! 3. answers `get_entries{before, limit}` ("load earlier") and `get_entries{ids}` (the
//!    full-output refetch) from the runtime when it is live, else from the file — bounded
//!    for the frame either way.
//!
//! Commands that need the runtime wait for the wake ([`Live::get`]).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use oino_session::manager::{offsets_dir, OffsetIndex, PageRequest};
use serde_json::{json, Value};
use tokio::sync::watch;
use unipi_proto::session::HistoryPage;
use unipi_proto::{AppErrorCode, AppErrorDto};

use super::bound;
use super::card::SessionCard;
use super::hub::HubSession;

/// Entries in the snapshot's tail page and in each "load earlier" page.
pub const PAGE_LIMIT: usize = 80;
/// Raw-bytes cap of one page read (before bounding): parsing stays a few tens of ms.
pub const PAGE_RAW_BYTES: u64 = 8 << 20;
/// Total budget of a `get_entries{ids}` reply (one frame, after the envelope).
pub const IDS_BUDGET: usize = 900 * 1024;

/// A session the stream serves: live already, or waking in the background.
#[derive(Clone)]
pub enum Live {
    /// The runtime is ours.
    Ready(Arc<HubSession>),
    /// The background wake publishes its outcome here.
    Waking(watch::Receiver<Option<Result<Arc<HubSession>, String>>>),
}

impl Live {
    /// The runtime, waiting for the wake when it is still running.
    ///
    /// # Errors
    ///
    /// The wake failed (the session could not start) or its task went away.
    pub async fn get(&self) -> Result<Arc<HubSession>, AppErrorDto> {
        match self {
            Self::Ready(session) => Ok(Arc::clone(session)),
            Self::Waking(rx) => {
                let mut rx = rx.clone();
                loop {
                    if let Some(outcome) = rx.borrow_and_update().clone() {
                        return outcome.map_err(|error| {
                            AppErrorDto::new(
                                AppErrorCode::E_INTERNAL,
                                format!("This chat could not start: {error}"),
                                true,
                            )
                        });
                    }
                    if rx.changed().await.is_err() {
                        return Err(AppErrorDto::new(
                            AppErrorCode::E_INTERNAL,
                            "This chat could not start",
                            true,
                        ));
                    }
                }
            }
        }
    }

    /// The runtime when it is already up (never waits).
    #[must_use]
    pub fn ready(&self) -> Option<Arc<HubSession>> {
        match self {
            Self::Ready(session) => Some(Arc::clone(session)),
            Self::Waking(rx) => rx
                .borrow()
                .as_ref()
                .and_then(|outcome| outcome.as_ref().ok().cloned()),
        }
    }
}

/// Where a session's file and its persisted offset index live.
#[derive(Clone, Debug)]
pub struct SessionFile {
    /// The session file.
    pub path: PathBuf,
    /// Directory of persisted offset indexes (beside the sessions root).
    pub offsets: PathBuf,
}

impl SessionFile {
    /// The file of `card`, indexed beside `sessions_root` (or beside the file's own
    /// project folder when it lives elsewhere).
    #[must_use]
    pub fn new(path: &Path, sessions_root: &Path) -> Self {
        let root = if path.starts_with(sessions_root) {
            sessions_root.to_path_buf()
        } else {
            path.parent()
                .and_then(Path::parent)
                .map_or_else(|| sessions_root.to_path_buf(), Path::to_path_buf)
        };
        Self {
            path: path.to_path_buf(),
            offsets: offsets_dir(&root),
        }
    }

    fn index(&self) -> Result<Arc<OffsetIndex>, String> {
        OffsetIndex::load(&self.path, Some(&self.offsets)).map_err(|error| error.to_string())
    }

    /// One page of the current branch (`before` = an entry id, `None` = the tail).
    ///
    /// # Errors
    ///
    /// The file cannot be indexed (missing, pre-v3, I/O) or `before` is not on the branch.
    pub fn page(&self, before: Option<String>, limit: usize) -> Result<Page, String> {
        let index = self.index()?;
        let page = index
            .page(
                &self.path,
                &PageRequest {
                    before,
                    limit,
                    max_bytes: PAGE_RAW_BYTES,
                },
            )
            .map_err(|error| error.to_string())?;
        Ok(Page {
            entries: page.entries,
            leaf_id: page.leaf_id,
            page: HistoryPage {
                total: page.total as u64,
                has_more: page.has_more,
                cursor: page.cursor,
            },
        })
    }

    /// Entries by id (any branch).
    ///
    /// # Errors
    ///
    /// The file cannot be indexed or read.
    pub fn by_ids(&self, ids: &[String]) -> Result<Vec<Value>, String> {
        self.index()?
            .entries_by_id(&self.path, ids)
            .map_err(|error| error.to_string())
    }
}

/// A page of entries and where it sits in the branch.
#[derive(Clone, Debug)]
pub struct Page {
    /// Root-most first.
    pub entries: Vec<Value>,
    /// The branch leaf.
    pub leaf_id: Option<String>,
    /// Total / has-more / cursor.
    pub page: HistoryPage,
}

impl Page {
    /// From an `_oino/entries_page` reply (`{entries, leafId, total, hasMore, cursor}`).
    #[must_use]
    pub fn from_reply(reply: &Value) -> Option<Self> {
        let entries = reply.get("entries")?.as_array()?.clone();
        Some(Self {
            page: HistoryPage {
                total: reply
                    .get("total")
                    .and_then(Value::as_u64)
                    .unwrap_or(entries.len() as u64),
                has_more: reply
                    .get("hasMore")
                    .and_then(Value::as_bool)
                    .unwrap_or(false),
                cursor: reply
                    .get("cursor")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            leaf_id: reply
                .get("leafId")
                .and_then(Value::as_str)
                .map(str::to_owned),
            entries,
        })
    }

    /// Bounds the page for one frame: big blocks cut / referenced out
    /// ([`bound::bound_entries`]), then the oldest entries dropped until it fits — the
    /// cursor moves to the new first entry so they come with the next older page.
    pub fn bound(&mut self, blobs_dir: Option<&Path>) {
        if let Some(blobs_dir) = blobs_dir {
            let _ = bound::bound_entries(&mut self.entries, blobs_dir);
        }
        if bound::fit_entries(&mut self.entries, bound::ENTRIES_BUDGET) > 0 {
            self.page.has_more = true;
            self.page.cursor = self
                .entries
                .first()
                .and_then(|entry| entry.get("id"))
                .and_then(Value::as_str)
                .map(str::to_owned);
        }
    }

    /// The `get_entries{before}` reply the app reads.
    #[must_use]
    pub fn reply(&self) -> Value {
        json!({
            "entries": self.entries,
            "leafId": self.leaf_id,
            "total": self.page.total,
            "hasMore": self.page.has_more,
            "cursor": self.page.cursor,
        })
    }
}

/// Bounds a `get_entries{ids}` reply (the full-output refetch): whole entries when they fit
/// one frame, else each clipped to its share (flagged `truncated`).
pub fn bound_full(entries: &mut [Value], blobs_dir: Option<&Path>) {
    if bound::serialized_len(entries) <= IDS_BUDGET {
        return;
    }
    let Some(blobs_dir) = blobs_dir else { return };
    let share = IDS_BUDGET / entries.len().max(1);
    let _ = bound::bound_entries_with(entries, share, IDS_BUDGET, blobs_dir);
}

/// The `get_state` stand-in shown while the runtime wakes: the card's name / cwd / file /
/// message count, and the model and thinking level the page's newest entries name.
#[must_use]
pub fn provisional_state(card: &SessionCard, entries: &[Value]) -> Value {
    let mut state = json!({
        "sessionId": card.session_id,
        "sessionFile": card.file.display().to_string(),
        "cwd": card.cwd,
        "isStreaming": false,
        "isCompacting": false,
        "messageCount": card.message_count,
        "pendingMessageCount": 0,
    });
    if let Some(name) = &card.name {
        state["sessionName"] = json!(name);
    }
    for entry in entries.iter().rev() {
        match entry.get("type").and_then(Value::as_str) {
            Some("model_change") if state.get("model").is_none() => {
                if let (Some(provider), Some(id)) = (
                    entry.get("provider").and_then(Value::as_str),
                    entry.get("modelId").and_then(Value::as_str),
                ) {
                    state["model"] = json!({"provider": provider, "id": id, "name": id});
                }
            }
            Some("thinking_level_change") if state.get("thinkingLevel").is_none() => {
                if let Some(level) = entry.get("thinkingLevel").and_then(Value::as_str) {
                    state["thinkingLevel"] = json!(level);
                }
            }
            Some("message") if state.get("model").is_none() => {
                let message = &entry["message"];
                if message.get("role").and_then(Value::as_str) == Some("assistant") {
                    if let (Some(provider), Some(id)) = (
                        message.get("provider").and_then(Value::as_str),
                        message.get("model").and_then(Value::as_str),
                    ) {
                        state["model"] = json!({"provider": provider, "id": id, "name": id});
                    }
                }
            }
            _ => {}
        }
        if state.get("model").is_some() && state.get("thinkingLevel").is_some() {
            break;
        }
    }
    state
}

/// The app's `get_entries` with paging or ids (`before`/`limit`/`ids`): served here, never
/// as core's whole-history `get_entries`. `None` = a plain `get_entries{since?}` (core's).
#[must_use]
pub fn paged_request(cmd: &Value) -> Option<(Option<String>, usize, Option<Vec<String>>)> {
    if cmd.get("type").and_then(Value::as_str) != Some("get_entries") {
        return None;
    }
    let ids = cmd.get("ids").and_then(Value::as_array).map(|ids| {
        ids.iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    let before = cmd.get("before").and_then(Value::as_str).map(str::to_owned);
    let limit = cmd.get("limit").and_then(Value::as_u64);
    if ids.is_none() && before.is_none() && limit.is_none() {
        return None;
    }
    let limit = limit.map_or(PAGE_LIMIT, |limit| {
        usize::try_from(limit).unwrap_or(PAGE_LIMIT)
    });
    Some((before, limit.clamp(1, 500), ids))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card() -> SessionCard {
        serde_json::from_value(json!({
            "sessionId": "s1", "file": "/tmp/s1.jsonl", "name": "Chat", "cwd": "/tmp",
            "updatedAt": 0, "messageCount": 4, "state": {"kind": "idle"},
        }))
        .unwrap()
    }

    #[test]
    fn provisional_state_names_the_last_model_and_level() {
        let entries = vec![
            json!({"type": "thinking_level_change", "id": "t", "thinkingLevel": "high"}),
            json!({"type": "message", "id": "a", "message": {"role": "assistant", "provider": "p", "model": "m1"}}),
            json!({"type": "message", "id": "u", "message": {"role": "user"}}),
        ];
        let state = provisional_state(&card(), &entries);
        assert_eq!(
            state["model"],
            json!({"provider": "p", "id": "m1", "name": "m1"})
        );
        assert_eq!(state["thinkingLevel"], "high");
        assert_eq!(state["sessionName"], "Chat");
        assert_eq!(state["isStreaming"], false);
    }

    #[test]
    fn only_paged_get_entries_are_intercepted() {
        assert!(paged_request(&json!({"type": "get_entries"})).is_none());
        assert!(paged_request(&json!({"type": "get_entries", "since": "x"})).is_none());
        assert!(paged_request(&json!({"type": "get_state", "limit": 3})).is_none());
        assert_eq!(
            paged_request(&json!({"type": "get_entries", "before": "e9", "limit": 5})),
            Some((Some("e9".into()), 5, None))
        );
        assert_eq!(
            paged_request(&json!({"type": "get_entries", "ids": ["a"]})),
            Some((None, PAGE_LIMIT, Some(vec!["a".into()])))
        );
    }

    #[test]
    fn a_page_over_the_frame_budget_drops_its_oldest_and_moves_the_cursor() {
        let mut page = Page {
            entries: (0..40)
                .map(
                    |i| json!({"id": format!("e{i}"), "type": "custom", "pad": "p".repeat(30_000)}),
                )
                .collect(),
            leaf_id: Some("e39".into()),
            page: HistoryPage {
                total: 100,
                has_more: true,
                cursor: Some("e0".into()),
            },
        };
        page.bound(None);
        assert!(bound::serialized_len(&page.entries) <= bound::ENTRIES_BUDGET);
        let first = page.entries[0]["id"].as_str().unwrap().to_owned();
        assert_ne!(first, "e0");
        assert_eq!(page.page.cursor.as_deref(), Some(first.as_str()));
        assert!(page.page.has_more);
    }
}
