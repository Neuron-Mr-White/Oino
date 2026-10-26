//! Compaction preparation and result types (pi `CompactionPreparation`, `prepareCompaction`,
//! `CompactionResult`, `CompactionDetails`).

use oino_ai::{AgentMessage, JsonValue, Usage};
use oino_session::{
    EntryId, EntryIndex, EntryKind, ProjectedEntry, SessionEntry, build_session_projection,
};
use serde::{Deserialize, Serialize};

use super::cut_point::find_projected_cut_point;
use super::settings::CompactionSettings;
use super::tokens::estimate_projected_context_tokens;
use super::utils::{FileLists, FileOperations};

/// pi `CompactionDetails`: file tracking stored in `CompactionEntry.details`.
pub type CompactionDetails = FileLists;

/// pi `CompactionResult` (the session manager adds id/parent when appending).
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
    /// Implementation data ([`CompactionDetails`] for pi summaries, free-form for extensions).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
}

/// pi `CompactionPreparation` (payload of `session_before_compact`).
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
        .filter(|entry| !matches!(entry.source_entry.kind, EntryKind::Compaction(_)))
        .flat_map(|entry| entry.messages.iter())
        .filter(|message| !matches!(message, AgentMessage::System(_)))
        .cloned()
        .collect()
}

/// pi `prepareCompaction(pathEntries, settings)`: `None` when the path ends with a compaction
/// or nothing would be summarised. `path` is the root → leaf branch.
#[must_use]
pub fn prepare_compaction(
    path: &[&SessionEntry],
    settings: &CompactionSettings,
) -> Option<CompactionPreparation> {
    if path
        .last()
        .is_some_and(|entry| matches!(entry.kind, EntryKind::Compaction(_)))
    {
        return None;
    }
    let index: EntryIndex = path.iter().map(|entry| (*entry).clone()).collect();
    let projection = build_session_projection(&index, index.last_id());
    let projected = &projection.entries;
    // The newest compaction is projected first; older ones inside its kept range are empty.
    let previous_compaction = projected.iter().position(|entry| {
        matches!(entry.source_entry.kind, EntryKind::Compaction(_)) && !entry.messages.is_empty()
    });
    let (previous_summary, boundary_start) = match previous_compaction {
        Some(position) => match &projected[position].source_entry.kind {
            EntryKind::Compaction(compaction) => (Some(compaction.summary.clone()), position + 1),
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
    let first_kept = &projected.get(cut.first_kept_entry_index)?.source_entry;
    if first_kept.id.as_str().is_empty() {
        return None;
    }
    let turn_start = cut.turn_start_index.filter(|_| cut.is_split_turn);
    let history_end = turn_start.unwrap_or(cut.first_kept_entry_index);
    let messages_to_summarize = summary_messages(
        projected
            .get(boundary_start..history_end)
            .unwrap_or_default(),
    );
    let turn_prefix_messages = turn_start.map_or_else(Vec::new, |turn_start| {
        summary_messages(&projected[turn_start..cut.first_kept_entry_index])
    });
    if messages_to_summarize.is_empty() && turn_prefix_messages.is_empty() {
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
        is_split_turn: cut.is_split_turn,
        tokens_before,
        previous_summary,
        file_ops,
        settings: *settings,
    })
}
