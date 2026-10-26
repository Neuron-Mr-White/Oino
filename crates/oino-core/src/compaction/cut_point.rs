//! Cut-point detection (pi `findCutPoint`, `findTurnStartIndex` and the projection-based
//! variant used by `prepareCompaction`).

use oino_ai::AgentMessage;
use oino_session::{EntryKind, ProjectedEntry, SessionEntry, entry_to_context_messages};
use serde::{Deserialize, Serialize};

use super::tokens::estimate_all;

/// pi `CutPointResult` (`turnStartIndex: -1` is `None`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CutPointResult {
    /// Index of the first entry to keep.
    pub first_kept_entry_index: usize,
    /// Index of the entry that starts the split turn.
    pub turn_start_index: Option<usize>,
    /// Whether the cut splits a turn (the kept range starts mid-turn).
    pub is_split_turn: bool,
}

/// Roles at which a cut may happen (never a tool result).
const fn is_cut_point_message(message: &AgentMessage) -> bool {
    !matches!(
        message,
        AgentMessage::ToolResult(_) | AgentMessage::System(_)
    )
}

/// Roles that start a turn (user-like input).
const fn is_turn_start_message(message: &AgentMessage) -> bool {
    matches!(
        message,
        AgentMessage::User(_)
            | AgentMessage::BashExecution(_)
            | AgentMessage::Custom(_)
            | AgentMessage::BranchSummary(_)
            | AgentMessage::CompactionSummary(_)
    )
}

const fn is_compaction(entry: &SessionEntry) -> bool {
    matches!(entry.kind, EntryKind::Compaction(_))
}

fn raw_messages(entry: &SessionEntry) -> Vec<AgentMessage> {
    entry_to_context_messages(entry, None)
}

fn is_turn_start_entry(entry: &SessionEntry) -> bool {
    !is_compaction(entry) && raw_messages(entry).iter().any(is_turn_start_message)
}

/// pi `findTurnStartIndex`: the nearest turn-start entry at or before `entry_index`, not
/// before `start_index`.
#[must_use]
pub fn find_turn_start_index(
    entries: &[&SessionEntry],
    entry_index: usize,
    start_index: usize,
) -> Option<usize> {
    (start_index..=entry_index).rev().find(|&index| {
        entries
            .get(index)
            .is_some_and(|entry| is_turn_start_entry(entry))
    })
}

/// Raw or projected entry view shared by both cut-point walks.
trait CutEntry {
    fn source(&self) -> &SessionEntry;
    fn messages(&self) -> Vec<AgentMessage>;
}

impl CutEntry for &SessionEntry {
    fn source(&self) -> &SessionEntry {
        self
    }
    fn messages(&self) -> Vec<AgentMessage> {
        raw_messages(self)
    }
}

impl CutEntry for ProjectedEntry {
    fn source(&self) -> &SessionEntry {
        &self.source_entry
    }
    fn messages(&self) -> Vec<AgentMessage> {
        self.messages.clone()
    }
}

struct Walk {
    cut_index: usize,
    exceeded_budget: bool,
}

/// Valid cut points and the budget walk (newest first) common to both variants.
fn walk_budget<E: CutEntry>(
    entries: &[E],
    start: usize,
    end: usize,
    keep_recent_tokens: u64,
) -> Option<Walk> {
    let cut_points: Vec<usize> = (start..end)
        .filter(|&index| {
            let entry = &entries[index];
            !is_compaction(entry.source()) && entry.messages().iter().any(is_cut_point_message)
        })
        .collect();
    let (&first, &last) = (cut_points.first()?, cut_points.last()?);
    let mut accumulated = 0;
    for index in (start..end).rev() {
        let tokens = estimate_all(&entries[index].messages());
        if tokens == 0 {
            continue;
        }
        accumulated += tokens;
        if accumulated >= keep_recent_tokens {
            let cut_index = cut_points
                .iter()
                .copied()
                .find(|&candidate| candidate >= index)
                .unwrap_or(last);
            return Some(Walk {
                cut_index,
                exceeded_budget: true,
            });
        }
    }
    Some(Walk {
        cut_index: first,
        exceeded_budget: false,
    })
}

fn finish<E: CutEntry>(
    entries: &[E],
    start: usize,
    mut cut_index: usize,
    contributes: impl Fn(&E) -> bool,
) -> CutPointResult {
    // Include adjacent metadata entries that do not affect context.
    while cut_index > start {
        let previous = &entries[cut_index - 1];
        if is_compaction(previous.source()) || contributes(previous) {
            break;
        }
        cut_index -= 1;
    }
    let starts_turn = |entry: &E| {
        !is_compaction(entry.source()) && entry.messages().iter().any(is_turn_start_message)
    };
    let cut_starts_turn = starts_turn(&entries[cut_index]);
    let turn_start_index = if cut_starts_turn {
        None
    } else {
        (start..=cut_index)
            .rev()
            .find(|&index| starts_turn(&entries[index]))
    };
    CutPointResult {
        first_kept_entry_index: cut_index,
        turn_start_index,
        is_split_turn: !cut_starts_turn && turn_start_index.is_some(),
    }
}

const fn no_cut(start: usize) -> CutPointResult {
    CutPointResult {
        first_kept_entry_index: start,
        turn_start_index: None,
        is_split_turn: false,
    }
}

/// pi `findCutPoint`: walk back from the newest entry in `start..end` accumulating estimated
/// tokens until `keep_recent_tokens`, then cut at the nearest valid cut point at or after
/// that entry (user, assistant, bash, custom or summary message; never a tool result).
#[must_use]
pub fn find_cut_point(
    entries: &[&SessionEntry],
    start: usize,
    end: usize,
    keep_recent_tokens: u64,
) -> CutPointResult {
    let end = end.min(entries.len());
    match walk_budget(entries, start, end, keep_recent_tokens) {
        None => no_cut(start),
        Some(walk) => finish(entries, start, walk.cut_index, |entry| {
            !raw_messages(entry).is_empty()
        }),
    }
}

/// pi `findProjectedCutPoint`: [`find_cut_point`] over the finalized projection, advancing
/// past a closed context-invisible suffix that contains an omitted assistant attempt
/// (recovery omissions).
pub(crate) fn find_projected_cut_point(
    entries: &[ProjectedEntry],
    start: usize,
    end: usize,
    keep_recent_tokens: u64,
) -> CutPointResult {
    let Some(walk) = walk_budget(entries, start, end, keep_recent_tokens) else {
        return no_cut(start);
    };
    let mut cut_index = walk.cut_index;
    let suffix = &entries[(cut_index + 1).min(end)..end];
    let intrinsically_visible = |entry: &ProjectedEntry| {
        !matches!(entry.source_entry.kind, EntryKind::ContextEdit { .. })
            && !raw_messages(&entry.source_entry).is_empty()
    };
    let omitted =
        |entry: &ProjectedEntry| intrinsically_visible(entry) && entry.messages.is_empty();
    let omitted_ids: Vec<_> = suffix
        .iter()
        .filter(|entry| omitted(entry))
        .map(|entry| &entry.source_entry.id)
        .collect();
    let has_external_replacement = suffix.iter().any(|entry| match &entry.source_entry.kind {
        EntryKind::ContextEdit {
            target_id,
            replacement: Some(_),
        } => !omitted_ids.contains(&target_id),
        _ => false,
    });
    let has_omitted_assistant = suffix.iter().any(|entry| {
        matches!(
            &entry.source_entry.kind,
            EntryKind::Message {
                message: AgentMessage::Assistant(_)
            }
        ) && omitted(entry)
    });
    let suffix_invisible = suffix.iter().all(|entry| {
        !is_compaction(&entry.source_entry) && (!intrinsically_visible(entry) || omitted(entry))
    });
    if walk.exceeded_budget
        && !has_external_replacement
        && has_omitted_assistant
        && suffix_invisible
    {
        cut_index += 1;
    }
    finish(entries, start, cut_index, |entry| {
        !entry.messages.is_empty()
    })
}
