//! Compaction preparation and result types (`CompactionPreparation`, `prepareCompaction`,
//! `CompactionResult`, `CompactionDetails`).

use oino_ai::{AgentMessage, JsonValue, StopReason, Usage};
use oino_session::{
    EntryId, EntryIndex, EntryKind, ProjectedEntry, SessionEntry, build_session_projection,
};
use serde::{Deserialize, Serialize};

use super::cut_point::find_projected_cut_point;
use super::settings::CompactionSettings;
use super::tokens::estimate_projected_context_tokens;
use super::utils::{FileLists, FileOperations};

/// File tracking stored in `CompactionEntry.details`.
pub type CompactionDetails = FileLists;

/// Compaction result (the session manager adds id/parent when appending).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionResult {
    /// Summary text (file lists appended).
    pub summary: String,
    /// First entry kept verbatim.
    pub first_kept_entry_id: EntryId,
    /// Context tokens before compaction.
    pub tokens_before: u64,
    /// Estimated context tokens after compaction.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub estimated_tokens_after: Option<u64>,
    /// Usage of the summarisation request(s).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Implementation data ([`CompactionDetails`] for built-in summaries, free-form for extensions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
}

/// Compaction preparation (payload of `session_before_compact`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionPreparation {
    /// First entry to keep.
    pub first_kept_entry_id: EntryId,
    /// Messages summarised and dropped from context.
    pub messages_to_summarize: Vec<AgentMessage>,
    /// Messages of the split turn's prefix (summarised separately).
    pub turn_prefix_messages: Vec<AgentMessage>,
    /// Whether the cut splits a turn.
    pub is_split_turn: bool,
    /// Estimated context tokens before compaction.
    pub tokens_before: u64,
    /// Summary of the previous compaction (iterative update).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_summary: Option<String>,
    /// File operations of the summarised messages (cumulative with the previous compaction).
    pub file_ops: FileOperations,
    /// Settings used.
    pub settings: CompactionSettings,
}

/// Messages of a projected entry used for summaries (compaction entries and system messages
/// are prompt state, not conversation).
fn summary_messages(entries: &[ProjectedEntry]) -> Vec<AgentMessage> {
    entries
        .iter()
        .filter(|entry| {
            !matches!(
                entry.source_entry.kind,
                EntryKind::Compaction(_) | EntryKind::CompactionState(_)
            )
        })
        .flat_map(|entry| entry.messages.iter())
        .filter(|message| !matches!(message, AgentMessage::System(_)))
        .cloned()
        .collect()
}

/// `None` when the path ends with a compaction
/// or nothing would be summarised. `path` is the root → leaf branch.
#[must_use]
pub fn prepare_compaction(
    path: &[&SessionEntry],
    settings: &CompactionSettings,
) -> Option<CompactionPreparation> {
    if path.last().is_some_and(|entry| {
        matches!(
            entry.kind,
            EntryKind::Compaction(_) | EntryKind::CompactionState(_)
        )
    }) {
        return None;
    }
    let index: EntryIndex = path.iter().map(|entry| (*entry).clone()).collect();
    let projection = build_session_projection(&index, index.last_id());
    let projected = &projection.entries;
    if projected.is_empty() {
        return None;
    }
    // The newest compaction is projected first; older ones inside its kept range are empty.
    let previous_compaction = projected.iter().position(|entry| {
        matches!(
            entry.source_entry.kind,
            EntryKind::Compaction(_) | EntryKind::CompactionState(_)
        ) && !entry.messages.is_empty()
    });
    let (previous_summary, boundary_start) = match previous_compaction {
        Some(position) => match &projected[position].source_entry.kind {
            EntryKind::Compaction(compaction) => (Some(compaction.summary.clone()), position + 1),
            EntryKind::CompactionState(state) => (Some(state.summary_text.clone()), position + 1),
            _ => (None, 0),
        },
        None => (None, 0),
    };
    let tokens_before = estimate_projected_context_tokens(&projection, path).tokens;
    let cut = find_projected_cut_point(
        projected,
        boundary_start,
        projected.len(),
        settings.keep_recent_tokens,
    );
    // Metadata adjacent to a cut must not become an anchor. Snap backwards to
    // a safe turn boundary first; if that lands on `boundary_start` (empty summary
    // because a huge turn exceeds `keep_recent_tokens`), snap forward to the
    // earliest valid anchor after `boundary_start`.
    let valid_anchor_at = |i: usize| -> Option<&SessionEntry> {
        let candidate = &projected.get(i)?.source_entry;
        if candidate.id.as_str().is_empty()
            || !oino_session::context::is_compaction_anchor_entry(candidate)
        {
            return None;
        }
        if summary_messages(projected.get(boundary_start..i).unwrap_or_default()).is_empty() {
            return None;
        }
        let raw_index = path.iter().position(|entry| entry.id == candidate.id)?;
        oino_session::context::validate_compaction_anchor(
            path,
            &oino_session::CompactionAnchor {
                id: candidate.id.clone(),
                index: raw_index,
            },
        )
        .ok()?;
        Some(candidate)
    };
    let last_in_cut = cut
        .first_kept_entry_index
        .min(projected.len().saturating_sub(1));
    let (kept_index, first_kept) = (boundary_start..=last_in_cut)
        .rev()
        .find_map(|i| valid_anchor_at(i).map(|entry| (i, entry)))
        .or_else(|| {
            ((last_in_cut + 1)..projected.len())
                .find_map(|i| valid_anchor_at(i).map(|entry| (i, entry)))
        })?;
    let messages_to_summarize = summary_messages(
        projected
            .get(boundary_start..kept_index)
            .unwrap_or_default(),
    );
    let turn_prefix_messages = Vec::new();
    if messages_to_summarize.is_empty() {
        return None;
    }
    let mut file_ops = FileOperations::new();
    if let Some(EntryKind::Compaction(previous)) =
        previous_compaction.map(|position| &projected[position].source_entry.kind)
        && previous.from_hook != Some(true)
        && let Some(details) = &previous.details
    {
        file_ops.seed_from_details(details);
    }
    for message in messages_to_summarize.iter().chain(&turn_prefix_messages) {
        file_ops.extract_from_message(message);
    }
    Some(CompactionPreparation {
        first_kept_entry_id: first_kept.id.clone(),
        messages_to_summarize,
        turn_prefix_messages,
        is_split_turn: false,
        tokens_before,
        previous_summary,
        file_ops,
        settings: *settings,
    })
}

/// Fallback preparation for a single oversized or multi-tool turn that has no later
/// turn-boundary entry yet. Returns `first_kept_entry_id = ""` so `commit_compaction`
/// materializes a non-displayed boundary entry only after summarization succeeds.
#[must_use]
pub fn prepare_single_turn_compaction(
    path: &[&SessionEntry],
    settings: &CompactionSettings,
) -> Option<CompactionPreparation> {
    if path.last().is_some_and(|entry| {
        matches!(
            entry.kind,
            EntryKind::Compaction(_) | EntryKind::CompactionState(_)
        )
    }) || oino_session::context::has_pending_tool_calls(path)
    {
        return None;
    }
    let index: EntryIndex = path.iter().map(|entry| (*entry).clone()).collect();
    let projection = build_session_projection(&index, index.last_id());
    let projected = &projection.entries;
    if projected.is_empty() {
        return None;
    }
    let previous_compaction = projected.iter().position(|entry| {
        matches!(
            entry.source_entry.kind,
            EntryKind::Compaction(_) | EntryKind::CompactionState(_)
        ) && !entry.messages.is_empty()
    });
    let (previous_summary, boundary_start) = match previous_compaction {
        Some(position) => match &projected[position].source_entry.kind {
            EntryKind::Compaction(compaction) => (Some(compaction.summary.clone()), position + 1),
            EntryKind::CompactionState(state) => (Some(state.summary_text.clone()), position + 1),
            _ => (None, 0),
        },
        None => (None, 0),
    };
    let mut end = projected.len();
    if end > boundary_start
        && let Some(AgentMessage::Assistant(assistant)) =
            projected[end - 1].source_entry.message()
        && matches!(assistant.stop_reason, StopReason::Error | StopReason::Aborted)
    {
        end -= 1;
    }
    let messages_to_summarize =
        summary_messages(projected.get(boundary_start..end).unwrap_or_default());
    if messages_to_summarize.len() < 2 {
        return None;
    }
    let tokens_before = estimate_projected_context_tokens(&projection, path).tokens;
    let has_tool_round = messages_to_summarize
        .iter()
        .any(|m| matches!(m, AgentMessage::ToolResult(_)));
    if !has_tool_round && tokens_before <= 4_096 {
        return None;
    }
    let mut file_ops = FileOperations::new();
    if let Some(EntryKind::Compaction(previous)) =
        previous_compaction.map(|position| &projected[position].source_entry.kind)
        && previous.from_hook != Some(true)
        && let Some(details) = &previous.details
    {
        file_ops.seed_from_details(details);
    }
    for message in &messages_to_summarize {
        file_ops.extract_from_message(message);
    }
    Some(CompactionPreparation {
        first_kept_entry_id: EntryId::new(""),
        messages_to_summarize,
        turn_prefix_messages: Vec::new(),
        is_split_turn: false,
        tokens_before,
        previous_summary,
        file_ops,
        settings: *settings,
    })
}
