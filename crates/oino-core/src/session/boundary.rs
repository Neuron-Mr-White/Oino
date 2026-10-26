//! Session boundaries (pi `_dispatchTurnEndBoundary`, `_runBeforeSettleBoundary`,
//! `_buildBoundaryContext`, `_applyBoundaryDrafts`, `_commitBoundaryDrafts`).

use oino_ai::{AgentMessage, AssistantMessage, JsonValue, StopReason, ToolResultMessage};
use oino_session::{
    CompactionDraft, CreateOptions, EditableContent, EntryId, SessionEntry, SessionManager,
};
use serde_json::json;

use std::sync::{Arc, Weak};
use std::time::Duration;

use super::{ActivityOutcome, AgentSession, Inner, lock};
use crate::events::{OinoEvent, RunState};

/// Which boundary is being built.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Boundary {
    TurnEnd,
    BeforeSettle,
}

impl Boundary {
    const fn name(self) -> &'static str {
        match self {
            Self::TurnEnd => "turn_end",
            Self::BeforeSettle => "agent_before_settle",
        }
    }
}

/// Applies boundary drafts to `manager` (pi `_applyBoundaryDrafts`).
///
/// # Errors
///
/// The first invalid draft.
pub(crate) fn apply_drafts(
    manager: &mut SessionManager,
    drafts: &[JsonValue],
) -> Result<Vec<EntryId>, String> {
    let mut appended = Vec::new();
    for draft in drafts {
        let text = |key: &str| {
            draft
                .get(key)
                .and_then(JsonValue::as_str)
                .map(str::to_owned)
        };
        let id = match draft.get("type").and_then(JsonValue::as_str) {
            Some("custom") => manager.append_custom_entry(
                &text("customType").ok_or("custom draft without customType")?,
                draft.get("data").cloned(),
            ),
            Some("custom_message") => manager.append_custom_message_entry(
                &text("customType").ok_or("custom_message draft without customType")?,
                draft
                    .get("content")
                    .and_then(|c| serde_json::from_value(c.clone()).ok())
                    .unwrap_or_default(),
                draft
                    .get("display")
                    .and_then(JsonValue::as_bool)
                    .unwrap_or(false),
                draft.get("details").cloned(),
            ),
            Some("context_edit") => {
                let target =
                    EntryId::new(text("targetId").ok_or("context_edit draft without targetId")?);
                let replacement = match draft.get("replacement") {
                    None | Some(JsonValue::Null) => None,
                    Some(value) => Some(
                        serde_json::from_value::<EditableContent>(value["content"].clone())
                            .map_err(|e| e.to_string())?,
                    ),
                };
                manager.append_context_edit(&target, replacement)
            }
            Some("compaction") => {
                let tokens_before = crate::compaction::estimate_projected_context_tokens(
                    &manager.build_session_projection(),
                    &manager.branch(None),
                )
                .tokens;
                manager.append_compaction(CompactionDraft {
                    summary: text("summary").unwrap_or_default(),
                    first_kept_entry_id: text("firstKeptEntryId").map(EntryId::new),
                    tokens_before,
                    details: draft.get("details").cloned(),
                    from_hook: Some(true),
                    usage: draft
                        .get("usage")
                        .and_then(|u| serde_json::from_value(u.clone()).ok()),
                    system_message: None,
                })
            }
            other => {
                return Err(format!(
                    "Unknown boundary entry type: {}",
                    other.unwrap_or("undefined")
                ));
            }
        }
        .map_err(|e| e.to_string())?;
        appended.push(id);
    }
    Ok(appended)
}

impl AgentSession {
    fn preview_manager(&self, drafts: &[JsonValue]) -> Result<SessionManager, String> {
        let (cwd, branch): (std::path::PathBuf, Vec<SessionEntry>) = {
            let manager = lock(&self.inner.manager);
            (
                manager.cwd().to_path_buf(),
                manager.branch(None).into_iter().cloned().collect(),
            )
        };
        let mut preview = SessionManager::in_memory(&cwd, CreateOptions::default(), branch);
        apply_drafts(&mut preview, drafts)?;
        Ok(preview)
    }

    /// pi `_buildBoundaryContext` (`BoundaryContextPreview` JSON).
    pub(crate) fn build_boundary_context(
        &self,
        drafts: &[JsonValue],
        boundary: Boundary,
    ) -> Result<JsonValue, String> {
        let projection = self.preview_manager(drafts)?.build_session_projection();
        let pending: Vec<AgentMessage> = self
            .inner
            .agent
            .peek_queued_messages()
            .into_iter()
            .chain(
                lock(&self.inner.book)
                    .pending_custom
                    .iter()
                    .cloned()
                    .map(AgentMessage::Custom),
            )
            .collect();
        let llm = oino_session::convert_to_llm(&projection.messages);
        let final_role = llm.last().map(oino_ai::Message::role);
        let has_context = llm
            .iter()
            .any(|m| !matches!(m, oino_ai::Message::System(_)));
        let context_can_continue = has_context && final_role != Some("assistant");
        let pending_custom = !lock(&self.inner.book).pending_custom.is_empty();
        let queued = self.inner.agent.has_queued_messages();
        let can_continue = context_can_continue
            || pending_custom
            || match boundary {
                Boundary::TurnEnd => queued,
                Boundary::BeforeSettle => final_role == Some("assistant") && queued,
            };
        Ok(json!({
            "contextEntries": projection.entries,
            "contextMessages": projection.messages,
            "llmMessages": llm,
            "pendingMessages": pending,
            "canContinue": can_continue,
        }))
    }

    fn can_continue(&self, boundary: Boundary) -> bool {
        self.build_boundary_context(&[], boundary)
            .ok()
            .and_then(|c| c["canContinue"].as_bool())
            .unwrap_or(false)
    }

    pub(crate) fn commit_boundary_drafts(&self, drafts: &[JsonValue]) {
        if drafts.is_empty() {
            return;
        }
        let appended = apply_drafts(&mut lock(&self.inner.manager), drafts);
        self.refresh_context();
        match appended {
            Ok(ids) => {
                for id in ids {
                    self.publish_entry(&id);
                }
            }
            Err(error) => self.runner().emit_error("<boundary>", "boundary", error),
        }
    }

    fn report_invalid_continuation(&self, boundary: Boundary) {
        let name = boundary.name();
        self.runner().emit_error(
            "<boundary>",
            name,
            format!("{name} requested continuation without runnable model context"),
        );
    }

    /// pi `_dispatchTurnEndBoundary`; returns whether an extension asked to continue.
    pub(crate) async fn dispatch_turn_end_boundary(
        &self,
        message: &AssistantMessage,
        tool_results: &[ToolResultMessage],
    ) -> bool {
        let outcome = match message.stop_reason {
            StopReason::Aborted => ActivityOutcome::Aborted,
            StopReason::Error => ActivityOutcome::Error,
            _ => ActivityOutcome::Completed,
        };
        let (turn_index, entry_id, tool_ids) = {
            let mut book = lock(&self.inner.book);
            book.last_outcome = outcome;
            let ids: Vec<String> = tool_results
                .iter()
                .filter_map(|result| {
                    book.tool_result_entries
                        .iter()
                        .find(|(call, _)| *call == result.tool_call_id)
                        .map(|(_, id)| id.as_str().to_owned())
                })
                .collect();
            (book.turn_index, book.last_assistant_entry.clone(), ids)
        };
        let runner = self.runner();
        if !runner.has_handlers("turn_end") {
            return false;
        }
        let Some(entry_id) = entry_id else {
            runner.emit_error(
                "<boundary>",
                "turn_end",
                "turn_end could not resolve the persisted assistant entry ID",
            );
            return false;
        };
        let mut base = serde_json::Map::new();
        base.insert("type".into(), json!("turn_end"));
        base.insert("turnIndex".into(), json!(turn_index));
        base.insert(
            "message".into(),
            json!(AgentMessage::Assistant(message.clone())),
        );
        base.insert(
            "toolResults".into(),
            json!(
                tool_results
                    .iter()
                    .cloned()
                    .map(AgentMessage::ToolResult)
                    .collect::<Vec<_>>()
            ),
        );
        base.insert("messageEntryId".into(), json!(entry_id.as_str()));
        base.insert("toolResultEntryIds".into(), json!(tool_ids));
        base.insert("outcome".into(), json!(outcome));
        let session = self.clone();
        let build =
            move |drafts: &[JsonValue]| session.build_boundary_context(drafts, Boundary::TurnEnd);
        let result = runner.emit_boundary(base, &build).await;
        self.commit_boundary_drafts(&result.entries);
        if result.continue_run && !self.can_continue(Boundary::TurnEnd) {
            self.report_invalid_continuation(Boundary::TurnEnd);
            return false;
        }
        result.continue_run
    }

    /// pi `_runBeforeSettleBoundary`; returns whether the run continues. When the handlers
    /// leave the run settling, the turn arbiter (runtime §7.6) may still continue it.
    pub(crate) async fn run_before_settle_boundary(&self) -> bool {
        let runner = self.runner();
        if !runner.has_handlers("agent_before_settle") {
            if self.inner.agent.has_queued_messages() {
                return true;
            }
            return self.run_arbiter().await;
        }
        let outcome = {
            let mut book = lock(&self.inner.book);
            book.is_before_settle = true;
            book.abort_during_before_settle = false;
            book.last_outcome
        };
        let mut base = serde_json::Map::new();
        base.insert("type".into(), json!("agent_before_settle"));
        base.insert("outcome".into(), json!(outcome));
        let session = self.clone();
        let build = move |drafts: &[JsonValue]| {
            session.build_boundary_context(drafts, Boundary::BeforeSettle)
        };
        let result = runner.emit_boundary(base, &build).await;
        self.commit_boundary_drafts(&result.entries);
        self.flush_pending_custom_messages();
        let can_continue = self.can_continue(Boundary::BeforeSettle);
        let aborted = {
            let mut book = lock(&self.inner.book);
            book.is_before_settle = false;
            book.abort_during_before_settle
        };
        if aborted {
            return false;
        }
        let should_continue = result.continue_run || self.inner.agent.has_queued_messages();
        if should_continue && !can_continue {
            if result.continue_run {
                self.report_invalid_continuation(Boundary::BeforeSettle);
            }
            return false;
        }
        if should_continue {
            return true;
        }
        self.run_arbiter().await
    }

    /// Turn arbiter (runtime §7.6, D9): when the run would settle normally with nothing
    /// pending, wait sources may hold the session in `waiting`, else the highest-priority
    /// nudge is appended as a `custom_message` and the run continues. Disabled in children.
    async fn run_arbiter(&self) -> bool {
        if self.is_child() || self.abort_requested() {
            return false;
        }
        let (outcome, pending_custom) = {
            let book = lock(&self.inner.book);
            (book.last_outcome, !book.pending_custom.is_empty())
        };
        if outcome != ActivityOutcome::Completed
            || pending_custom
            || self.inner.agent.has_queued_messages()
            || self.pending_message_count() > 0
        {
            return false;
        }
        let runner = self.runner();
        let reasons = runner.arbiter_wait_reasons().await;
        if !reasons.is_empty() {
            self.set_waiting(reasons);
            return false;
        }
        let Some(nudge) = runner.arbiter_nudge().await else {
            return false;
        };
        if self.abort_requested() {
            return false;
        }
        let mut draft = json!({
            "type": "custom_message",
            "customType": nudge["customType"],
            "content": nudge.get("content").cloned().unwrap_or_else(|| json!("")),
            "display": nudge.get("display").and_then(JsonValue::as_bool).unwrap_or(true),
        });
        if let Some(details) = nudge.get("details").filter(|d| !d.is_null()) {
            draft["details"] = details.clone();
        }
        self.commit_boundary_drafts(&[draft]);
        if !self.can_continue(Boundary::BeforeSettle) {
            self.report_invalid_continuation(Boundary::BeforeSettle);
            return false;
        }
        true
    }

    /// Wait reasons reported at the last settle (empty when not waiting).
    #[must_use]
    pub fn waiting_on(&self) -> Vec<String> {
        lock(&self.inner.arbiter).waiting_on.clone()
    }

    fn set_waiting(&self, reasons: Vec<String>) {
        let start_poll = {
            let mut state = lock(&self.inner.arbiter);
            state.waiting_on.clone_from(&reasons);
            !std::mem::replace(&mut state.polling, true)
        };
        self.inner.hub.publish(OinoEvent::RunState {
            state: RunState::Waiting,
            waiting_on: Some(reasons),
        });
        if start_poll {
            let inner = Arc::downgrade(&self.inner);
            tokio::spawn(poll_wait_sources(inner));
        }
    }

    /// Leaves `waiting` (a run started or the wait sources went quiet).
    pub(crate) fn clear_waiting(&self, next: RunState) {
        let was_waiting = {
            let mut state = lock(&self.inner.arbiter);
            !std::mem::take(&mut state.waiting_on).is_empty()
        };
        if was_waiting {
            self.inner.hub.publish(OinoEvent::RunState {
                state: next,
                waiting_on: None,
            });
        }
    }
}

/// Arbiter bookkeeping of a session.
#[derive(Debug, Default)]
pub(crate) struct ArbiterState {
    pub(crate) waiting_on: Vec<String>,
    polling: bool,
}

/// How often wait sources are re-asked while the session waits.
const WAIT_POLL: Duration = Duration::from_secs(2);

/// Re-asks the wait sources while the session is idle and waiting; leaves `waiting` when
/// they report nothing.
async fn poll_wait_sources(inner: Weak<Inner>) {
    loop {
        tokio::time::sleep(WAIT_POLL).await;
        let Some(session) = inner.upgrade().map(AgentSession::from_inner) else {
            return;
        };
        if session.waiting_on().is_empty() || !session.is_idle() {
            break;
        }
        let reasons = session.runner().arbiter_wait_reasons().await;
        if reasons.is_empty() {
            session.clear_waiting(RunState::Idle);
            break;
        }
        let changed = {
            let mut state = lock(&session.inner.arbiter);
            if state.waiting_on.is_empty() {
                false
            } else {
                let changed = state.waiting_on != reasons;
                state.waiting_on.clone_from(&reasons);
                changed
            }
        };
        if changed {
            session.inner.hub.publish(OinoEvent::RunState {
                state: RunState::Waiting,
                waiting_on: Some(reasons),
            });
        }
    }
    if let Some(inner) = inner.upgrade() {
        lock(&inner.arbiter).polling = false;
    }
}
