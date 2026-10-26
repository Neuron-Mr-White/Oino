//! Token accounting (pi `calculateContextTokens`, `estimateTokens`, `estimateContextTokens`,
//! `estimateProjectedContextTokens`, `shouldCompact`).

use oino_ai::{
    AgentMessage, AssistantContent, StopReason, TypedText, Usage, UserContent, UserContentField,
    UserTextField,
};
use oino_session::context::current_system_message;
use oino_session::{EntryKind, SessionEntry, SessionProjection};
use serde::{Deserialize, Serialize};

use super::js;
use super::settings::CompactionSettings;

/// Characters counted for one image block.
const ESTIMATED_IMAGE_CHARS: usize = 4800;

/// pi `calculateContextTokens`: `totalTokens`, or the sum of the components when it is 0.
#[must_use]
pub fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input + usage.output + usage.cache_read + usage.cache_write
    }
}

/// Usage of a successful assistant message with non-zero context tokens.
fn assistant_usage(message: &AgentMessage) -> Option<&Usage> {
    match message {
        AgentMessage::Assistant(assistant)
            if !matches!(
                assistant.stop_reason,
                StopReason::Aborted | StopReason::Error
            ) && calculate_context_tokens(&assistant.usage) > 0 =>
        {
            Some(&assistant.usage)
        }
        _ => None,
    }
}

/// pi `getLastAssistantUsage`: usage of the newest valid assistant message entry.
#[must_use]
pub fn last_assistant_usage(entries: &[&SessionEntry]) -> Option<Usage> {
    entries.iter().rev().find_map(|entry| match &entry.kind {
        EntryKind::Message { message } => assistant_usage(message).copied(),
        _ => None,
    })
}

/// pi `ContextUsageEstimate`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ContextUsageEstimate {
    /// Estimated total context tokens.
    pub tokens: u64,
    /// Tokens reported by the latest applicable assistant usage.
    pub usage_tokens: u64,
    /// Estimated tokens after that usage.
    pub trailing_tokens: u64,
    /// Index of the message that provided usage.
    pub last_usage_index: Option<usize>,
}

fn ceil_quarter(chars: usize) -> u64 {
    u64::try_from(chars.div_ceil(4)).unwrap_or(u64::MAX)
}

fn user_field_chars(content: &UserContentField) -> usize {
    match content {
        UserContentField::Text(text) => js::len16(text),
        UserContentField::Blocks(blocks) => blocks_chars(blocks),
    }
}

fn blocks_chars(blocks: &[UserContent]) -> usize {
    blocks
        .iter()
        .map(|block| match block {
            UserContent::Text(text) => js::len16(&text.text),
            UserContent::Image(_) => ESTIMATED_IMAGE_CHARS,
        })
        .sum()
}

fn system_chars(content: &UserTextField) -> usize {
    match content {
        UserTextField::Text(text) => js::len16(text),
        UserTextField::Blocks(blocks) => blocks
            .iter()
            .map(|TypedText::Text(text)| js::len16(&text.text))
            .sum(),
    }
}

/// pi `estimateTokens`: `ceil(chars / 4)` per message (UTF-16 characters; 4800 per image;
/// tool calls count name + JSON arguments; system messages count sections and `toolsAdded`).
#[must_use]
pub fn estimate_tokens(message: &AgentMessage) -> u64 {
    let chars = match message {
        AgentMessage::System(system) => {
            let sections: usize = system
                .sections
                .iter()
                .flatten()
                .filter_map(|(_, text)| text.as_deref())
                .map(js::len16)
                .sum();
            let tools = system
                .tools_added
                .as_ref()
                .and_then(|tools| serde_json::to_value(tools).ok())
                .map_or(0, |value| js::len16(&js::stringify(&value)));
            system_chars(&system.content) + sections + tools
        }
        AgentMessage::User(user) => user_field_chars(&user.content),
        AgentMessage::Assistant(assistant) => assistant
            .content
            .iter()
            .map(|block| match block {
                AssistantContent::Text(text) => js::len16(&text.text),
                AssistantContent::Thinking(thinking) => js::len16(&thinking.thinking),
                AssistantContent::ToolCall(call) => {
                    let arguments = serde_json::Value::Object(call.arguments.clone());
                    js::len16(&call.name) + js::len16(&js::stringify(&arguments))
                }
            })
            .sum(),
        AgentMessage::Custom(custom) => user_field_chars(&custom.content),
        AgentMessage::ToolResult(result) => blocks_chars(&result.content),
        AgentMessage::BashExecution(bash) => js::len16(&bash.command) + js::len16(&bash.output),
        AgentMessage::BranchSummary(summary) => js::len16(&summary.summary),
        AgentMessage::CompactionSummary(summary) => js::len16(&summary.summary),
    };
    ceil_quarter(chars)
}

/// Sum of [`estimate_tokens`] over `messages`.
pub(crate) fn estimate_all<'a>(messages: impl IntoIterator<Item = &'a AgentMessage>) -> u64 {
    messages.into_iter().map(estimate_tokens).sum()
}

/// pi `estimateContextTokens`: the newest valid assistant usage plus estimates for the
/// messages after it, or a full estimate when no usage exists.
#[must_use]
pub fn estimate_context_tokens(messages: &[AgentMessage]) -> ContextUsageEstimate {
    let usage = messages
        .iter()
        .enumerate()
        .rev()
        .find_map(|(index, message)| assistant_usage(message).map(|usage| (index, usage)));
    let Some((index, usage)) = usage else {
        let estimated = estimate_all(messages);
        return ContextUsageEstimate {
            tokens: estimated,
            usage_tokens: 0,
            trailing_tokens: estimated,
            last_usage_index: None,
        };
    };
    let usage_tokens = calculate_context_tokens(usage);
    let trailing_tokens = estimate_all(&messages[index + 1..]);
    ContextUsageEstimate {
        tokens: usage_tokens + trailing_tokens,
        usage_tokens,
        trailing_tokens,
        last_usage_index: Some(index),
    }
}

/// pi `estimateProjectedContextTokens`: like [`estimate_context_tokens`] over the projection,
/// but usage recorded before the latest `context_edit`/`compaction` on `branch` is not
/// trusted; then the current system checkpoint plus every non-system message is estimated.
#[must_use]
pub fn estimate_projected_context_tokens(
    projection: &SessionProjection,
    branch: &[&SessionEntry],
) -> ContextUsageEstimate {
    let estimate = estimate_context_tokens(&projection.messages);
    if let Some(usage_index) = estimate.last_usage_index {
        let mut first_message = 0;
        let mut usage_entry_id = None;
        for entry in &projection.entries {
            let next = first_message + entry.messages.len();
            if usage_index < next {
                usage_entry_id = Some(&entry.source_entry.id);
                break;
            }
            first_message = next;
        }
        let usage_entry_index = usage_entry_id
            .and_then(|id| branch.iter().position(|entry| &entry.id == id))
            .map_or(-1, |index| i64::try_from(index).unwrap_or(i64::MAX));
        let latest_invalidating = branch
            .iter()
            .rposition(|entry| {
                matches!(
                    entry.kind,
                    EntryKind::ContextEdit { .. } | EntryKind::Compaction(_)
                )
            })
            .map_or(-1, |index| i64::try_from(index).unwrap_or(i64::MAX));
        if usage_entry_index > latest_invalidating {
            return estimate;
        }
    }
    let system = current_system_message(&projection.messages)
        .map_or(0, |system| estimate_tokens(&AgentMessage::System(system)));
    let tokens = system
        + estimate_all(
            projection
                .messages
                .iter()
                .filter(|message| !matches!(message, AgentMessage::System(_))),
        );
    ContextUsageEstimate {
        tokens,
        usage_tokens: 0,
        trailing_tokens: tokens,
        last_usage_index: None,
    }
}

/// pi `shouldCompact`: `enabled && contextTokens > contextWindow − reserveTokens`.
#[must_use]
pub fn should_compact(
    context_tokens: u64,
    context_window: u64,
    settings: &CompactionSettings,
) -> bool {
    #[allow(
        clippy::cast_possible_wrap,
        reason = "token counts are far below i64::MAX; pi compares JS doubles"
    )]
    let threshold = context_window as i64 - settings.reserve_tokens as i64;
    #[allow(clippy::cast_possible_wrap, reason = "see above")]
    let tokens = context_tokens as i64;
    settings.enabled && tokens > threshold
}
