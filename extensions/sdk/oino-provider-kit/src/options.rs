//! pi-ai `api/simple-options.js` and `utils/estimate.js`: max-token clamping against the
//! context window, thinking budgets and sampling-parameter resolution.

use oino_ai::transcript::system_message_text;
use oino_ai::{
    AssistantContent, JsonObject, Message, Model, StopReason, ThinkingBudgets, ThinkingLevel,
    ToolDecl, ToolRef, Usage, UserContent, UserContentField,
};
use serde::Serialize;

use crate::util::{clamp_thinking_level, js_len};

const CONTEXT_SAFETY_TOKENS: u64 = 4096;
const CHARS_PER_TOKEN: f64 = 3.5;
const ESTIMATED_IMAGE_CHARS: usize = 4800;
/// Tokens always left for the answer when a thinking budget shares the response ceiling.
pub const MIN_ANSWER_TOKENS: u64 = 1024;

/// pi `clampMaxTokensToContext`.
#[must_use]
pub fn clamp_max_tokens_to_context(model: &Model, messages: &[Message], max_tokens: u64) -> u64 {
    if model.context_window == 0 {
        return max_tokens.max(1);
    }
    let available = i128::from(model.context_window)
        - i128::from(estimate_context_tokens(messages))
        - i128::from(CONTEXT_SAFETY_TOKENS);
    let available = u64::try_from(available.max(1)).unwrap_or(1);
    max_tokens.min(available)
}

/// pi `resolveSamplingParams`.
#[must_use]
pub fn resolve_sampling_params(
    model: &Model,
    level: ThinkingLevel,
    request: Option<&JsonObject>,
) -> Option<JsonObject> {
    let effective = clamp_thinking_level(model, level);
    let by_level = model
        .sampling_params_by_thinking_level
        .as_ref()
        .and_then(|map| map.get(&effective));
    if model.sampling_params.is_none() && by_level.is_none() && request.is_none() {
        return None;
    }
    let mut merged = JsonObject::new();
    for source in [model.sampling_params.as_ref(), by_level, request]
        .into_iter()
        .flatten()
    {
        for (key, value) in source {
            merged.insert(key.clone(), value.clone());
        }
    }
    Some(merged)
}

/// pi `thinkingBudgetForLevel` (`xhigh`/`max` use the `high` budget).
#[must_use]
pub fn thinking_budget_for_level(level: ThinkingLevel, custom: Option<&ThinkingBudgets>) -> u64 {
    let custom = custom.cloned().unwrap_or_default();
    match level {
        ThinkingLevel::Minimal => custom.minimal.unwrap_or(1024),
        ThinkingLevel::Low => custom.low.unwrap_or(2048),
        ThinkingLevel::Medium => custom.medium.unwrap_or(8192),
        ThinkingLevel::High | ThinkingLevel::Xhigh | ThinkingLevel::Max => {
            custom.high.unwrap_or(16384)
        }
        // pi indexes the budget table with "off" and gets `undefined`.
        ThinkingLevel::Off => 0,
    }
}

/// pi `clampThinkingBudgetToAnswerRoom`.
#[must_use]
pub fn clamp_thinking_budget_to_answer_room(budget: u64, ceiling: u64) -> u64 {
    budget.min(ceiling.saturating_sub(MIN_ANSWER_TOKENS))
}

/// pi `adjustMaxTokensForThinking` → `(maxTokens, thinkingBudget)`.
#[must_use]
pub fn adjust_max_tokens_for_thinking(
    base_max_tokens: Option<u64>,
    model_max_tokens: u64,
    level: ThinkingLevel,
    custom: Option<&ThinkingBudgets>,
) -> (u64, u64) {
    let mut budget = thinking_budget_for_level(level, custom);
    let max_tokens = base_max_tokens.map_or(model_max_tokens, |base| {
        (base + budget).min(model_max_tokens)
    });
    if max_tokens <= budget {
        budget = clamp_thinking_budget_to_answer_room(budget, max_tokens);
    }
    (max_tokens, budget)
}

fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input + usage.output + usage.cache_read + usage.cache_write
    }
}

fn ceil_tokens(chars: usize) -> u64 {
    #[allow(
        clippy::cast_precision_loss,
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "character counts are small; matches Math.ceil(chars / 3.5)"
    )]
    let tokens = (chars as f64 / CHARS_PER_TOKEN).ceil() as u64;
    tokens
}

fn content_chars(content: &[UserContent]) -> usize {
    content
        .iter()
        .map(|block| match block {
            UserContent::Text(text) => js_len(&text.text),
            UserContent::Image(_) => ESTIMATED_IMAGE_CHARS,
        })
        .sum()
}

fn tools_tokens<T: Serialize>(tools: Option<&Vec<T>>) -> u64 {
    match tools {
        Some(tools) if !tools.is_empty() => {
            ceil_tokens(js_len(&serde_json::to_string(tools).unwrap_or_default()))
        }
        _ => 0,
    }
}

/// pi `estimateMessageTokens`.
#[must_use]
pub fn estimate_message_tokens(message: &Message) -> u64 {
    match message {
        Message::System(system) => {
            ceil_tokens(js_len(&system_message_text(system)))
                + tools_tokens::<ToolDecl>(system.tools_added.as_ref())
                + tools_tokens::<ToolRef>(system.tools_removed.as_ref())
        }
        Message::User(user) => match &user.content {
            UserContentField::Text(text) => ceil_tokens(js_len(text)),
            UserContentField::Blocks(blocks) => ceil_tokens(content_chars(blocks)),
        },
        Message::ToolResult(result) => ceil_tokens(content_chars(&result.content)),
        Message::Assistant(assistant) => {
            let chars = assistant
                .content
                .iter()
                .map(|block| match block {
                    AssistantContent::Text(text) => js_len(&text.text),
                    AssistantContent::Thinking(thinking) => js_len(&thinking.thinking),
                    AssistantContent::ToolCall(call) => {
                        js_len(&call.name)
                            + js_len(&serde_json::to_string(&call.arguments).unwrap_or_default())
                    }
                })
                .sum();
            ceil_tokens(chars)
        }
    }
}

/// pi `estimateContextTokens(...).tokens`.
#[must_use]
pub fn estimate_context_tokens(messages: &[Message]) -> u64 {
    let mut latest_prefix = i64::MIN;
    let mut usage_info: Option<(u64, usize)> = None;
    for (index, message) in messages.iter().enumerate() {
        if let Message::Assistant(assistant) = message {
            let applies = assistant.timestamp >= latest_prefix;
            let tokens = calculate_context_tokens(&assistant.usage);
            if applies
                && !matches!(
                    assistant.stop_reason,
                    StopReason::Aborted | StopReason::Error
                )
                && tokens > 0
            {
                usage_info = Some((tokens, index));
            }
        }
        latest_prefix = latest_prefix.max(message.timestamp());
    }
    match usage_info {
        Some((tokens, index)) => {
            tokens
                + messages[index + 1..]
                    .iter()
                    .map(estimate_message_tokens)
                    .sum::<u64>()
        }
        None => messages.iter().map(estimate_message_tokens).sum(),
    }
}
