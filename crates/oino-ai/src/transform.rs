//! Provider-side transcript transformation (pi-ai `api/transform-messages.js`): drop foreign
//! reasoning and signatures, downgrade images for text-only models, synthesize missing tool
//! results, and hold mid-tool-call system messages until the results are in.

use std::collections::{HashMap, HashSet};

use crate::content::{AssistantContent, TextContent, ToolCall, UserContent};
use crate::message::{AssistantMessage, Message, ToolResultMessage};
use crate::model::Model;
use crate::primitives::{InputModality, StopReason, TimestampMs};

const NON_VISION_USER_IMAGE_PLACEHOLDER: &str = "(image omitted: model does not support images)";
const NON_VISION_TOOL_IMAGE_PLACEHOLDER: &str =
    "(tool image omitted: model does not support images)";
const ERROR_TOOL_IMAGE_PLACEHOLDER: &str = "(image attached after the tool results)";
const MISSING_TOOL_RESULT_TEXT: &str = "No result provided";

/// Callback renaming tool-call ids for a target model (pi `normalizeToolCallId`).
pub type NormalizeToolCallId<'a> = &'a dyn Fn(&str, &Model, &AssistantMessage) -> String;

/// Images split out of an error tool result (pi `splitErrorToolResultImages`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SplitToolImages {
    /// Content with images replaced by a placeholder.
    pub content: Vec<UserContent>,
    /// Caption for the follow-up user message.
    pub caption: String,
    /// The images.
    pub images: Vec<UserContent>,
}

/// pi `splitErrorToolResultImages`.
#[must_use]
pub fn split_error_tool_result_images(message: &ToolResultMessage) -> Option<SplitToolImages> {
    if !message.is_error {
        return None;
    }
    let images: Vec<UserContent> = message
        .content
        .iter()
        .filter(|block| matches!(block, UserContent::Image(_)))
        .cloned()
        .collect();
    if images.is_empty() {
        return None;
    }
    Some(SplitToolImages {
        content: replace_images(&message.content, ERROR_TOOL_IMAGE_PLACEHOLDER),
        caption: format!(
            "Images from the error result of tool call {} ({}):",
            message.tool_call_id, message.tool_name
        ),
        images,
    })
}

fn replace_images(content: &[UserContent], placeholder: &str) -> Vec<UserContent> {
    let mut result = Vec::with_capacity(content.len());
    let mut previous_was_placeholder = false;
    for block in content {
        match block {
            UserContent::Image(_) => {
                if !previous_was_placeholder {
                    result.push(UserContent::text(placeholder));
                }
                previous_was_placeholder = true;
            }
            UserContent::Text(text) => {
                previous_was_placeholder = text.text == placeholder;
                result.push(block.clone());
            }
        }
    }
    result
}

fn downgrade_images(message: Message, model: &Model) -> Message {
    if model.input.contains(&InputModality::Image) {
        return message;
    }
    match message {
        Message::User(mut user) => {
            if let crate::content::UserContentField::Blocks(blocks) = &user.content {
                user.content = crate::content::UserContentField::Blocks(replace_images(
                    blocks,
                    NON_VISION_USER_IMAGE_PLACEHOLDER,
                ));
            }
            Message::User(user)
        }
        Message::ToolResult(mut result) => {
            result.content = replace_images(&result.content, NON_VISION_TOOL_IMAGE_PLACEHOLDER);
            Message::ToolResult(result)
        }
        other => other,
    }
}

fn transform_assistant(
    mut message: AssistantMessage,
    model: &Model,
    normalize: Option<NormalizeToolCallId<'_>>,
    id_map: &mut HashMap<String, String>,
) -> AssistantMessage {
    let same_model =
        message.provider == model.provider && message.api == model.api && message.model == model.id;
    let snapshot = message.clone();
    message.content = std::mem::take(&mut message.content)
        .into_iter()
        .filter_map(|block| match block {
            AssistantContent::Thinking(thinking) => {
                if !same_model {
                    return None;
                }
                if thinking.redacted == Some(true) || thinking.thinking_signature.is_some() {
                    return Some(AssistantContent::Thinking(thinking));
                }
                (!thinking.thinking.trim().is_empty())
                    .then_some(AssistantContent::Thinking(thinking))
            }
            AssistantContent::Text(text) if !same_model => {
                Some(AssistantContent::Text(TextContent::new(text.text)))
            }
            AssistantContent::ToolCall(mut call) if !same_model => {
                call.thought_signature = None;
                if let Some(normalize) = normalize {
                    let normalized = normalize(&call.id, model, &snapshot);
                    if normalized != call.id {
                        id_map.insert(call.id.clone(), normalized.clone());
                        call.id = normalized;
                    }
                }
                Some(AssistantContent::ToolCall(call))
            }
            other => Some(other),
        })
        .collect();
    message
}

/// pi `transformMessages(messages, model, normalizeToolCallId)`. `now` stamps synthesized
/// tool results (pi uses `Date.now()`).
#[must_use]
pub fn transform_messages(
    messages: Vec<Message>,
    model: &Model,
    normalize: Option<NormalizeToolCallId<'_>>,
    now: TimestampMs,
) -> Vec<Message> {
    let mut id_map = HashMap::new();
    let transformed: Vec<Message> = messages
        .into_iter()
        .map(|message| downgrade_images(message, model))
        .map(|message| match message {
            Message::ToolResult(mut result) => {
                if let Some(normalized) = id_map.get(&result.tool_call_id) {
                    result.tool_call_id.clone_from(normalized);
                }
                Message::ToolResult(result)
            }
            Message::Assistant(assistant) => Message::Assistant(transform_assistant(
                assistant,
                model,
                normalize,
                &mut id_map,
            )),
            other => other,
        })
        .collect();

    let mut pass = PendingToolCalls::default();
    for message in transformed {
        match message {
            Message::Assistant(assistant) => {
                pass.close(now);
                if matches!(
                    assistant.stop_reason,
                    StopReason::Error | StopReason::Aborted
                ) {
                    continue;
                }
                let calls: Vec<ToolCall> = assistant
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        AssistantContent::ToolCall(call) => Some(call.clone()),
                        _ => None,
                    })
                    .collect();
                if !calls.is_empty() {
                    pass.pending = calls;
                    pass.answered.clear();
                }
                pass.result.push(Message::Assistant(assistant));
            }
            Message::ToolResult(result) => {
                pass.answered.insert(result.tool_call_id.clone());
                pass.result.push(Message::ToolResult(result));
            }
            Message::System(system) => {
                if pass.pending.is_empty() {
                    pass.result.push(Message::System(system));
                } else {
                    pass.held.push(Message::System(system));
                }
            }
            Message::User(user) => {
                pass.close(now);
                pass.result.push(Message::User(user));
            }
        }
    }
    pass.close(now);
    pass.result
}

#[derive(Default)]
struct PendingToolCalls {
    result: Vec<Message>,
    pending: Vec<ToolCall>,
    answered: HashSet<String>,
    held: Vec<Message>,
}

impl PendingToolCalls {
    fn close(&mut self, now: TimestampMs) {
        if !self.pending.is_empty() {
            for call in std::mem::take(&mut self.pending) {
                if !self.answered.contains(&call.id) {
                    self.result.push(Message::ToolResult(ToolResultMessage {
                        tool_call_id: call.id,
                        tool_name: call.name,
                        content: vec![UserContent::text(MISSING_TOOL_RESULT_TEXT)],
                        details: None,
                        usage: None,
                        nested_calls: None,
                        is_error: true,
                        timestamp: now,
                        duration_ms: None,
                    }));
                }
            }
            self.answered.clear();
        }
        self.result.append(&mut self.held);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn parse<T: serde::de::DeserializeOwned>(raw: serde_json::Value) -> Option<T> {
        serde_json::from_value(raw).ok()
    }

    #[test]
    fn drops_foreign_thinking_and_fills_missing_results() {
        let model: Option<Model> = parse(
            json!({"id":"m2","name":"m2","api":"a","provider":"p","baseUrl":"",
            "input":["text"],"cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0},"reasoning":false,"contextWindow":1,"maxTokens":1}),
        );
        let messages: Option<Vec<Message>> = parse(json!([
            {"role":"assistant","content":[{"type":"thinking","thinking":"t","thinkingSignature":"s"},
                {"type":"text","text":"x","textSignature":"sig"},
                {"type":"toolCall","id":"c1","name":"read","arguments":{},"thoughtSignature":"ts"}],
             "api":"a","provider":"p","model":"m1","usage":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"totalTokens":0,
             "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"toolUse","timestamp":1},
            {"role":"system","content":"late","timestamp":2},
            {"role":"user","content":[{"type":"image","data":"AA","mimeType":"image/png"}],"timestamp":3}
        ]));
        let (Some(model), Some(messages)) = (model, messages) else {
            unreachable!("fixtures parse")
        };
        let out = transform_messages(messages, &model, None, 42);
        let wire = serde_json::to_value(&out).unwrap_or_default();
        assert_eq!(
            wire[0]["content"],
            json!([{"type":"text","text":"x"},{"type":"toolCall","id":"c1","name":"read","arguments":{}}])
        );
        assert_eq!(wire[1]["role"], json!("toolResult"));
        assert_eq!(wire[1]["timestamp"], json!(42));
        assert_eq!(wire[2]["role"], json!("system"));
        assert_eq!(
            wire[3]["content"],
            json!([{"type":"text","text":NON_VISION_USER_IMAGE_PLACEHOLDER}])
        );
    }
}
