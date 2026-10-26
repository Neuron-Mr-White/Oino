//! Conversions between ACP content and Oino messages: prompt blocks in (modes §6.2), the
//! transcript replayed by `session/load`, and the prompt stop reason.

use std::collections::HashMap;
use std::path::Path;

use agent_client_protocol::schema::v1::ContentBlock;
use oino_ai::{
    AgentMessage, AssistantContent, ImageContent, JsonObject, JsonValue, StopReason,
    UserContentField,
};
use serde_json::json;

use crate::tools::{content_block, tool_kind, tool_locations, tool_title};

/// A prompt as Oino takes it: one text plus image attachments.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PromptInput {
    /// Message text (blocks joined by blank lines).
    pub text: String,
    /// Image attachments.
    pub images: Vec<ImageContent>,
}

/// Percent-decodes a `file://` URI path (`%20` → space).
fn decode_uri_path(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Some(byte) = std::str::from_utf8(&bytes[i + 1..i + 3])
                .ok()
                .and_then(|hex| u8::from_str_radix(hex, 16).ok())
        {
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The text a `resource_link` becomes: `@path` for files, the URI otherwise.
fn link_text(uri: &str) -> String {
    uri.strip_prefix("file://").map_or_else(
        || uri.to_owned(),
        |path| format!("@{}", decode_uri_path(path)),
    )
}

/// Converts ACP prompt blocks (text, image, embedded resource, resource link).
#[must_use]
pub fn prompt_input(blocks: &[ContentBlock]) -> PromptInput {
    let mut texts = Vec::new();
    let mut images = Vec::new();
    for block in blocks {
        let Ok(value) = serde_json::to_value(block) else {
            continue;
        };
        let field = |key: &str| value.get(key).and_then(JsonValue::as_str);
        match field("type") {
            Some("text") => texts.push(field("text").unwrap_or_default().to_owned()),
            Some("image") => {
                if let (Some(data), Some(mime)) = (field("data"), field("mimeType")) {
                    images.push(ImageContent {
                        data: data.to_owned(),
                        mime_type: mime.to_owned(),
                    });
                }
            }
            Some("resource") => {
                let resource = &value["resource"];
                let uri = resource["uri"].as_str().unwrap_or_default();
                match (resource["text"].as_str(), resource["blob"].as_str()) {
                    (Some(text), _) => {
                        texts.push(format!("<resource uri=\"{uri}\">\n{text}\n</resource>"));
                    }
                    (None, Some(blob)) => {
                        let mime = resource["mimeType"].as_str().unwrap_or_default();
                        if mime.starts_with("image/") {
                            images.push(ImageContent {
                                data: blob.to_owned(),
                                mime_type: mime.to_owned(),
                            });
                        } else {
                            texts.push(link_text(uri));
                        }
                    }
                    (None, None) => {}
                }
            }
            Some("resource_link") => texts.push(link_text(field("uri").unwrap_or_default())),
            _ => {}
        }
    }
    PromptInput {
        text: texts.join("\n\n"),
        images,
    }
}

/// ACP stop reason of a settled prompt (modes §6.2).
#[must_use]
pub fn stop_reason(aborted: bool, last_assistant: Option<StopReason>) -> &'static str {
    match last_assistant {
        _ if aborted => "cancelled",
        Some(StopReason::Aborted) => "cancelled",
        Some(StopReason::Length) => "max_tokens",
        _ => "end_turn",
    }
}

fn chunk(kind: &str, content: JsonValue) -> JsonValue {
    let mut update = json!({ "sessionUpdate": kind });
    update["content"] = content;
    update
}

/// `session/update` payloads replaying `messages` (the current branch) for `session/load`.
#[must_use]
pub fn replay(messages: &[AgentMessage], cwd: &Path) -> Vec<JsonValue> {
    let mut updates = Vec::new();
    let mut calls: HashMap<String, String> = HashMap::new();
    for message in messages {
        match message {
            AgentMessage::User(user) => match &user.content {
                UserContentField::Text(text) => {
                    updates.push(chunk(
                        "user_message_chunk",
                        json!({"type": "text", "text": text}),
                    ));
                }
                UserContentField::Blocks(blocks) => {
                    for block in blocks {
                        updates.push(chunk("user_message_chunk", content_block(block)));
                    }
                }
            },
            AgentMessage::Assistant(assistant) => {
                for block in &assistant.content {
                    match block {
                        AssistantContent::Text(text) if !text.text.is_empty() => {
                            updates.push(chunk(
                                "agent_message_chunk",
                                json!({"type": "text", "text": text.text}),
                            ));
                        }
                        AssistantContent::Thinking(thinking) if !thinking.thinking.is_empty() => {
                            updates.push(chunk(
                                "agent_thought_chunk",
                                json!({"type": "text", "text": thinking.thinking}),
                            ));
                        }
                        AssistantContent::ToolCall(call) => {
                            calls.insert(call.id.clone(), call.name.clone());
                            updates.push(tool_call_created(
                                &call.id,
                                &call.name,
                                &call.arguments,
                                "pending",
                                cwd,
                            ));
                        }
                        _ => {}
                    }
                }
            }
            AgentMessage::ToolResult(result) => {
                let content: Vec<JsonValue> = result
                    .content
                    .iter()
                    .map(|block| json!({"type": "content", "content": content_block(block)}))
                    .collect();
                let status = if result.is_error {
                    "failed"
                } else {
                    "completed"
                };
                if !calls.contains_key(&result.tool_call_id) {
                    updates.push(tool_call_created(
                        &result.tool_call_id,
                        &result.tool_name,
                        &JsonObject::new(),
                        status,
                        cwd,
                    ));
                }
                updates.push(json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": result.tool_call_id,
                    "status": status,
                    "content": content,
                }));
            }
            AgentMessage::CompactionSummary(summary) => updates.push(chunk(
                "agent_thought_chunk",
                json!({"type": "text", "text": summary.summary}),
            )),
            AgentMessage::BranchSummary(summary) => updates.push(chunk(
                "agent_thought_chunk",
                json!({"type": "text", "text": summary.summary}),
            )),
            _ => {}
        }
    }
    updates
}

/// A `tool_call` creation update.
#[must_use]
pub fn tool_call_created(
    id: &str,
    name: &str,
    args: &JsonObject,
    status: &str,
    cwd: &Path,
) -> JsonValue {
    json!({
        "sessionUpdate": "tool_call",
        "toolCallId": id,
        "title": if args.is_empty() { name.to_owned() } else { tool_title(name, args) },
        "kind": tool_kind(name),
        "status": status,
        "rawInput": args,
        "locations": tool_locations(name, args, cwd),
    })
}

/// Clips `text` to `max` characters with an ellipsis (session titles).
#[must_use]
pub fn clip(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or_default().trim();
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let mut clipped: String = line.chars().take(max.saturating_sub(1)).collect();
    clipped.push('…');
    clipped
}

/// The last assistant stop reason among `messages`.
#[must_use]
pub fn last_assistant_stop(messages: &[AgentMessage]) -> Option<(StopReason, Option<String>)> {
    messages.iter().rev().find_map(|message| match message {
        AgentMessage::Assistant(assistant) => {
            Some((assistant.stop_reason, assistant.error_message.clone()))
        }
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn blocks(value: JsonValue) -> Vec<ContentBlock> {
        serde_json::from_value(value).unwrap()
    }

    #[test]
    fn prompt_blocks() {
        let input = prompt_input(&blocks(json!([
            {"type": "text", "text": "Explain"},
            {"type": "resource_link", "uri": "file:///tmp/my%20file.rs", "name": "my file.rs"},
            {"type": "resource", "resource": {"uri": "file:///a.rs", "text": "fn a() {}"}},
            {"type": "image", "data": "AAAA", "mimeType": "image/png"},
        ])));
        assert_eq!(
            input.text,
            "Explain\n\n@/tmp/my file.rs\n\n<resource uri=\"file:///a.rs\">\nfn a() {}\n</resource>"
        );
        assert_eq!(input.images.len(), 1);
        assert_eq!(input.images[0].mime_type, "image/png");
    }

    #[test]
    fn stop_reasons() {
        assert_eq!(stop_reason(true, Some(StopReason::Stop)), "cancelled");
        assert_eq!(stop_reason(false, Some(StopReason::Length)), "max_tokens");
        assert_eq!(stop_reason(false, Some(StopReason::Error)), "end_turn");
        assert_eq!(stop_reason(false, None), "end_turn");
    }

    #[test]
    fn clipping() {
        assert_eq!(clip("short", 80), "short");
        assert_eq!(clip("abcdef", 4), "abc…");
        assert_eq!(clip("first\nsecond", 80), "first");
    }
}
