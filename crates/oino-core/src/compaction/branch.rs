//! Branch summarisation for tree navigation (pi `compaction/branch-summarization.js`).

use std::collections::HashSet;

use oino_ai::{AgentMessage, StopReason, Usage};
use oino_session::messages::{branch_summary_message, compaction_summary_message, custom_message};
use oino_session::{EntryId, EntryIndex, EntryKind, SessionEntry, convert_to_llm};
use serde::{Deserialize, Serialize};

use super::prompts::{BRANCH_SUMMARY_PREAMBLE, BRANCH_SUMMARY_PROMPT};
use super::summarize::{
    Summarizer, has_tool_call, response_text, summarization_context, summarization_failure,
};
use super::tokens::estimate_tokens;
use super::utils::{FileLists, FileOperations, format_file_operations, serialize_conversation};

/// pi `BranchSummaryDetails`: file tracking stored in `BranchSummaryEntry.details`.
pub type BranchSummaryDetails = FileLists;

/// Default `branchSummary.reserveTokens`.
pub const DEFAULT_BRANCH_SUMMARY_RESERVE_TOKENS: u64 = 16384;

/// Context window assumed when the model reports 0.
const FALLBACK_CONTEXT_WINDOW: u64 = 128_000;

/// pi `CollectEntriesResult`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CollectEntriesResult {
    /// Entries to summarise, oldest first.
    pub entries: Vec<SessionEntry>,
    /// Deepest entry on both paths.
    pub common_ancestor_id: Option<EntryId>,
}

/// pi `BranchPreparation`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchPreparation {
    /// Messages to summarise, oldest first.
    pub messages: Vec<AgentMessage>,
    /// File operations (cumulative with pi-generated branch summaries).
    pub file_ops: FileOperations,
    /// Estimated tokens of `messages`.
    pub total_tokens: u64,
}

/// pi `BranchSummaryResult`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryResult {
    /// Summary text (preamble and file lists included).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Provider usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Files only read.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_files: Option<Vec<String>>,
    /// Files modified.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_files: Option<Vec<String>>,
    /// The request was aborted.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aborted: Option<bool>,
    /// Failure text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Prompt options of [`generate_branch_summary`] (pi `GenerateBranchSummaryOptions` minus the
/// request fields carried by [`Summarizer`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BranchSummaryOptions {
    /// Extra instructions.
    pub custom_instructions: Option<String>,
    /// `custom_instructions` replace the default prompt instead of extending it.
    pub replace_instructions: bool,
    /// Tokens reserved when selecting history (`branchSummary.reserveTokens`).
    pub reserve_tokens: u64,
}

impl Default for BranchSummaryOptions {
    fn default() -> Self {
        Self {
            custom_instructions: None,
            replace_instructions: false,
            reserve_tokens: DEFAULT_BRANCH_SUMMARY_RESERVE_TOKENS,
        }
    }
}

/// pi `SessionManager.getBranch(id)`: root → `id` (empty for an unknown id).
fn branch_ids(index: &EntryIndex, id: &EntryId) -> Vec<EntryId> {
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut current = index.get(id);
    while let Some(entry) = current {
        if !seen.insert(&entry.id) {
            break;
        }
        path.push(entry.id.clone());
        current = entry
            .parent_id
            .as_ref()
            .and_then(|parent| index.get(parent));
    }
    path.reverse();
    path
}

/// pi `collectEntriesForBranchSummary`: entries from `old_leaf` back to (excluding) the
/// deepest common ancestor with `target`, oldest first. Compaction entries are included.
#[must_use]
pub fn collect_entries_for_branch_summary(
    index: &EntryIndex,
    old_leaf: Option<&EntryId>,
    target: &EntryId,
) -> CollectEntriesResult {
    let Some(old_leaf) = old_leaf else {
        return CollectEntriesResult {
            entries: Vec::new(),
            common_ancestor_id: None,
        };
    };
    let old_path: HashSet<EntryId> = branch_ids(index, old_leaf).into_iter().collect();
    let common_ancestor_id = branch_ids(index, target)
        .into_iter()
        .rev()
        .find(|id| old_path.contains(id));
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut current = Some(old_leaf.clone());
    while let Some(id) = current
        .take()
        .filter(|id| Some(id) != common_ancestor_id.as_ref())
    {
        let Some(entry) = index.get(&id).filter(|_| seen.insert(id.clone())) else {
            break;
        };
        entries.push(entry.clone());
        current.clone_from(&entry.parent_id);
    }
    entries.reverse();
    CollectEntriesResult {
        entries,
        common_ancestor_id,
    }
}

/// pi `getMessageFromEntry` (branch variant): conversation content of an entry; tool
/// results are skipped (their calls carry the context) and compactions become summaries.
fn message_from_entry(entry: &SessionEntry) -> Option<AgentMessage> {
    let timestamp = entry.timestamp.to_ms().unwrap_or(0);
    match &entry.kind {
        EntryKind::Message {
            message: AgentMessage::ToolResult(_),
        } => None,
        EntryKind::Message { message } => Some(message.clone()),
        EntryKind::CustomMessage {
            custom_type,
            content,
            display,
            details,
        } => Some(custom_message(
            custom_type,
            content.clone(),
            *display,
            details.clone(),
            timestamp,
        )),
        EntryKind::BranchSummary(summary) => Some(branch_summary_message(
            &summary.summary,
            Some(summary.from_id.as_str()),
            timestamp,
        )),
        EntryKind::Compaction(compaction) => Some(compaction_summary_message(
            &compaction.summary,
            compaction.tokens_before,
            timestamp,
        )),
        _ => None,
    }
}

/// pi `prepareBranchEntries`: file operations from every entry (seeded from pi-generated
/// branch summaries), then messages newest → oldest until `token_budget` (0 = unlimited); a
/// summary entry that overflows is still kept while under 90 % of the budget.
#[must_use]
pub fn prepare_branch_entries(entries: &[SessionEntry], token_budget: u64) -> BranchPreparation {
    let mut file_ops = FileOperations::new();
    for entry in entries {
        if let EntryKind::BranchSummary(summary) = &entry.kind
            && summary.from_hook != Some(true)
            && let Some(details) = &summary.details
        {
            file_ops.seed_from_details(details);
        }
    }
    let mut messages = Vec::new();
    let mut total_tokens = 0;
    for entry in entries.iter().rev() {
        let Some(message) = message_from_entry(entry) else {
            continue;
        };
        file_ops.extract_from_message(&message);
        let tokens = estimate_tokens(&message);
        if token_budget > 0 && total_tokens + tokens > token_budget {
            let is_summary = matches!(
                entry.kind,
                EntryKind::Compaction(_) | EntryKind::BranchSummary(_)
            );
            #[allow(
                clippy::cast_precision_loss,
                reason = "pi compares token counts as JS doubles"
            )]
            let under_ninety_percent = (total_tokens as f64) < token_budget as f64 * 0.9;
            if is_summary && under_ninety_percent {
                messages.push(message);
                total_tokens += tokens;
            }
            break;
        }
        messages.push(message);
        total_tokens += tokens;
    }
    messages.reverse();
    BranchPreparation {
        messages,
        file_ops,
        total_tokens,
    }
}

/// pi `generateBranchSummary`: summarises abandoned branch entries (oldest first) within
/// `context window − reserve_tokens`. Failures are reported in the result, not as errors.
/// The request carries no reasoning level or caller session id (as in pi).
pub async fn generate_branch_summary(
    entries: &[SessionEntry],
    summarizer: &Summarizer,
    options: &BranchSummaryOptions,
) -> BranchSummaryResult {
    let context_window = match summarizer.model.context_window {
        0 => FALLBACK_CONTEXT_WINDOW,
        window => window,
    };
    let token_budget = context_window.saturating_sub(options.reserve_tokens);
    let BranchPreparation {
        messages, file_ops, ..
    } = prepare_branch_entries(entries, token_budget);
    if messages.is_empty() {
        return BranchSummaryResult {
            summary: Some("No content to summarize".to_owned()),
            ..BranchSummaryResult::default()
        };
    }
    let conversation = serialize_conversation(&convert_to_llm(&messages));
    let custom = options
        .custom_instructions
        .as_deref()
        .filter(|text| !text.is_empty());
    let instructions = match custom {
        Some(custom) if options.replace_instructions => custom.to_owned(),
        Some(custom) => format!("{BRANCH_SUMMARY_PROMPT}\n\nAdditional focus: {custom}"),
        None => BRANCH_SUMMARY_PROMPT.to_owned(),
    };
    let prompt = format!("<conversation>\n{conversation}\n</conversation>\n\n{instructions}");
    let request_options = oino_ai::StreamOptions {
        headers: summarizer.headers.clone(),
        ..oino_ai::StreamOptions::default()
    };
    let response = summarizer
        .complete(summarization_context(&prompt), request_options)
        .await;
    if response.stop_reason == StopReason::Aborted {
        return BranchSummaryResult {
            aborted: Some(true),
            ..BranchSummaryResult::default()
        };
    }
    if let Some(failure) = summarization_failure(&response, "Branch summarization") {
        return error_result(failure);
    }
    if has_tool_call(&response) {
        return error_result("Branch summarization attempted to call a tool".to_owned());
    }
    let lists = file_ops.file_lists();
    let summary = format!(
        "{BRANCH_SUMMARY_PREAMBLE}{}{}",
        response_text(&response),
        format_file_operations(&lists.read_files, &lists.modified_files)
    );
    BranchSummaryResult {
        summary: Some(summary),
        usage: Some(response.usage),
        read_files: Some(lists.read_files),
        modified_files: Some(lists.modified_files),
        aborted: None,
        error: None,
    }
}

fn error_result(error: String) -> BranchSummaryResult {
    BranchSummaryResult {
        error: Some(error),
        ..BranchSummaryResult::default()
    }
}
