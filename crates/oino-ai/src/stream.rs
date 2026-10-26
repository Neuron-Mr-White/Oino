//! Streaming events (pi `AssistantMessageEvent`), the partial-free provider form crossing the
//! WIT boundary ([`ProviderEvent`]) and request types ([`Context`], [`StreamOptions`]).

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::content::{AssistantContent, TextContent, ThinkingContent, ToolCall};
use crate::json::{JsonObject, JsonValue, js_f64_opt};
use crate::message::{AssistantMessage, Message};
use crate::primitives::{CacheRetention, StopReason, ThinkingLevel};

/// Successful terminal reason (`done.reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DoneReason {
    /// End of turn.
    Stop,
    /// Token limit.
    Length,
    /// Tool calls requested.
    ToolUse,
    /// Deferred response.
    Deferred,
}

impl From<DoneReason> for StopReason {
    fn from(reason: DoneReason) -> Self {
        match reason {
            DoneReason::Stop => Self::Stop,
            DoneReason::Length => Self::Length,
            DoneReason::ToolUse => Self::ToolUse,
            DoneReason::Deferred => Self::Deferred,
        }
    }
}

/// Failure terminal reason (`error.reason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ErrorReason {
    /// Aborted by the caller.
    Aborted,
    /// Provider/transport error.
    Error,
}

impl From<ErrorReason> for StopReason {
    fn from(reason: ErrorReason) -> Self {
        match reason {
            ErrorReason::Aborted => Self::Aborted,
            ErrorReason::Error => Self::Error,
        }
    }
}

/// pi `AssistantMessageEvent` (with the live `partial` message).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    /// Stream started.
    Start {
        /// Live message.
        partial: AssistantMessage,
    },
    /// Text block opened.
    #[serde(rename_all = "camelCase")]
    TextStart {
        /// Block index.
        content_index: usize,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Text appended.
    #[serde(rename_all = "camelCase")]
    TextDelta {
        /// Block index.
        content_index: usize,
        /// Appended text.
        delta: String,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Text block closed (authoritative content).
    #[serde(rename_all = "camelCase")]
    TextEnd {
        /// Block index.
        content_index: usize,
        /// Final text.
        content: String,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Thinking block opened.
    #[serde(rename_all = "camelCase")]
    ThinkingStart {
        /// Block index.
        content_index: usize,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Thinking appended.
    #[serde(rename_all = "camelCase")]
    ThinkingDelta {
        /// Block index.
        content_index: usize,
        /// Appended text.
        delta: String,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Thinking block closed.
    #[serde(rename_all = "camelCase")]
    ThinkingEnd {
        /// Block index.
        content_index: usize,
        /// Final text.
        content: String,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Tool call opened.
    #[serde(rename = "toolcall_start", rename_all = "camelCase")]
    ToolcallStart {
        /// Block index.
        content_index: usize,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Tool-call argument JSON appended.
    #[serde(rename = "toolcall_delta", rename_all = "camelCase")]
    ToolcallDelta {
        /// Block index.
        content_index: usize,
        /// Appended argument JSON.
        delta: String,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Tool call closed (authoritative call).
    #[serde(rename = "toolcall_end", rename_all = "camelCase")]
    ToolcallEnd {
        /// Block index.
        content_index: usize,
        /// Final call.
        tool_call: ToolCall,
        /// Live message.
        partial: AssistantMessage,
    },
    /// Successful end.
    Done {
        /// Reason.
        reason: DoneReason,
        /// Final message.
        message: AssistantMessage,
    },
    /// Failed end.
    Error {
        /// Reason.
        reason: ErrorReason,
        /// Final (error) message.
        error: AssistantMessage,
    },
}

/// Provider-emitted event: [`AssistantMessageEvent`] without `partial` (D5). `toolcall_start`
/// may carry the call `id` and `toolName` early.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ProviderEvent {
    /// Stream started.
    Start {},
    /// Text block opened.
    #[serde(rename_all = "camelCase")]
    TextStart {
        /// Block index.
        content_index: usize,
    },
    /// Text appended.
    #[serde(rename_all = "camelCase")]
    TextDelta {
        /// Block index.
        content_index: usize,
        /// Appended text.
        delta: String,
    },
    /// Text block closed.
    #[serde(rename_all = "camelCase")]
    TextEnd {
        /// Block index.
        content_index: usize,
        /// Final text.
        content: String,
    },
    /// Thinking block opened.
    #[serde(rename_all = "camelCase")]
    ThinkingStart {
        /// Block index.
        content_index: usize,
    },
    /// Thinking appended.
    #[serde(rename_all = "camelCase")]
    ThinkingDelta {
        /// Block index.
        content_index: usize,
        /// Appended text.
        delta: String,
    },
    /// Thinking block closed.
    #[serde(rename_all = "camelCase")]
    ThinkingEnd {
        /// Block index.
        content_index: usize,
        /// Final text.
        content: String,
    },
    /// Tool call opened.
    #[serde(rename = "toolcall_start", rename_all = "camelCase")]
    ToolcallStart {
        /// Block index.
        content_index: usize,
        /// Call id when already known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Tool name when already known.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_name: Option<String>,
    },
    /// Tool-call argument JSON appended.
    #[serde(rename = "toolcall_delta", rename_all = "camelCase")]
    ToolcallDelta {
        /// Block index.
        content_index: usize,
        /// Appended argument JSON.
        delta: String,
    },
    /// Tool call closed.
    #[serde(rename = "toolcall_end", rename_all = "camelCase")]
    ToolcallEnd {
        /// Block index.
        content_index: usize,
        /// Final call.
        tool_call: ToolCall,
    },
    /// Successful end.
    Done {
        /// Reason.
        reason: DoneReason,
        /// Final message.
        message: AssistantMessage,
    },
    /// Failed end.
    Error {
        /// Reason.
        reason: ErrorReason,
        /// Final message.
        error: AssistantMessage,
    },
}

/// Rebuilds pi `partial` messages from [`ProviderEvent`]s so the host emits exact pi events.
#[derive(Clone, Debug)]
pub struct PartialAssembler {
    partial: AssistantMessage,
    tool_args: IndexMap<usize, String>,
}

impl PartialAssembler {
    /// Starts from `template` (api/provider/model/timestamp; content is cleared and
    /// `stopReason` set to `pending`).
    #[must_use]
    pub fn new(mut template: AssistantMessage) -> Self {
        template.content.clear();
        template.stop_reason = StopReason::Pending;
        Self {
            partial: template,
            tool_args: IndexMap::new(),
        }
    }

    /// Current partial message.
    #[must_use]
    pub const fn partial(&self) -> &AssistantMessage {
        &self.partial
    }

    /// Applies one provider event and returns the matching pi event.
    #[allow(
        clippy::too_many_lines,
        reason = "one arm per pi event keeps the 1:1 mapping readable"
    )]
    pub fn apply(&mut self, event: ProviderEvent) -> AssistantMessageEvent {
        match event {
            ProviderEvent::Start {} => AssistantMessageEvent::Start {
                partial: self.partial.clone(),
            },
            ProviderEvent::TextStart { content_index } => {
                self.put(
                    content_index,
                    AssistantContent::Text(TextContent::default()),
                );
                AssistantMessageEvent::TextStart {
                    content_index,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::TextDelta {
                content_index,
                delta,
            } => {
                if let Some(AssistantContent::Text(block)) = self.slot(content_index) {
                    block.text.push_str(&delta);
                }
                AssistantMessageEvent::TextDelta {
                    content_index,
                    delta,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::TextEnd {
                content_index,
                content,
            } => {
                if let Some(AssistantContent::Text(block)) = self.slot(content_index) {
                    block.text.clone_from(&content);
                }
                AssistantMessageEvent::TextEnd {
                    content_index,
                    content,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::ThinkingStart { content_index } => {
                self.put(
                    content_index,
                    AssistantContent::Thinking(ThinkingContent::default()),
                );
                AssistantMessageEvent::ThinkingStart {
                    content_index,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::ThinkingDelta {
                content_index,
                delta,
            } => {
                if let Some(AssistantContent::Thinking(block)) = self.slot(content_index) {
                    block.thinking.push_str(&delta);
                }
                AssistantMessageEvent::ThinkingDelta {
                    content_index,
                    delta,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::ThinkingEnd {
                content_index,
                content,
            } => {
                if let Some(AssistantContent::Thinking(block)) = self.slot(content_index) {
                    block.thinking.clone_from(&content);
                }
                AssistantMessageEvent::ThinkingEnd {
                    content_index,
                    content,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::ToolcallStart {
                content_index,
                id,
                tool_name,
            } => {
                self.put(
                    content_index,
                    AssistantContent::ToolCall(ToolCall {
                        id: id.unwrap_or_default(),
                        name: tool_name.unwrap_or_default(),
                        ..ToolCall::default()
                    }),
                );
                self.tool_args.insert(content_index, String::new());
                AssistantMessageEvent::ToolcallStart {
                    content_index,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::ToolcallDelta {
                content_index,
                delta,
            } => {
                let buffer = self.tool_args.entry(content_index).or_default();
                buffer.push_str(&delta);
                let parsed = serde_json::from_str::<JsonObject>(buffer).ok();
                if let (Some(arguments), Some(AssistantContent::ToolCall(call))) =
                    (parsed, self.slot(content_index))
                {
                    call.arguments = arguments;
                }
                AssistantMessageEvent::ToolcallDelta {
                    content_index,
                    delta,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::ToolcallEnd {
                content_index,
                tool_call,
            } => {
                self.tool_args.shift_remove(&content_index);
                self.put(content_index, AssistantContent::ToolCall(tool_call.clone()));
                AssistantMessageEvent::ToolcallEnd {
                    content_index,
                    tool_call,
                    partial: self.partial.clone(),
                }
            }
            ProviderEvent::Done { reason, message } => {
                self.partial = message.clone();
                AssistantMessageEvent::Done { reason, message }
            }
            ProviderEvent::Error { reason, error } => {
                self.partial = error.clone();
                AssistantMessageEvent::Error { reason, error }
            }
        }
    }

    fn put(&mut self, index: usize, block: AssistantContent) {
        let content = &mut self.partial.content;
        if index < content.len() {
            content[index] = block;
        } else {
            while content.len() < index {
                content.push(AssistantContent::Text(TextContent::default()));
            }
            content.push(block);
        }
    }

    fn slot(&mut self, index: usize) -> Option<&mut AssistantContent> {
        self.partial.content.get_mut(index)
    }
}

/// Normalised transcript sent to providers: system messages carry prompt and tools.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Context {
    /// Messages in order.
    pub messages: Vec<Message>,
}

/// Provider transport preference.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Transport {
    /// Server-sent events.
    Sse,
    /// WebSocket.
    Websocket,
    /// Cached WebSocket connection.
    WebsocketCached,
    /// Provider default.
    Auto,
}

/// Token budgets per thinking level (pi `ThinkingBudgets`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ThinkingBudgets {
    /// `minimal` budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub minimal: Option<u64>,
    /// `low` budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub low: Option<u64>,
    /// `medium` budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium: Option<u64>,
    /// `high` budget.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub high: Option<u64>,
}

/// Serializable subset of pi `SimpleStreamOptions`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StreamOptions {
    /// Requested thinking level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<ThinkingLevel>,
    /// Output token cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_tokens: Option<u64>,
    /// Sampling temperature.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "js_f64_opt")]
    pub temperature: Option<f64>,
    /// Extra sampling parameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling_params: Option<JsonObject>,
    /// Prompt cache retention.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cache_retention: Option<CacheRetention>,
    /// Session id (cache affinity).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// Transport preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transport: Option<Transport>,
    /// Thinking budgets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_budgets: Option<ThinkingBudgets>,
    /// Tool choice (`"auto"`, `"none"`, provider objects).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool_choice: Option<JsonValue>,
    /// Request timeout.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_ms: Option<u64>,
    /// Provider-level retries.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retries: Option<u32>,
    /// Retry delay cap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_retry_delay_ms: Option<u64>,
    /// Request metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<JsonObject>,
    /// Header overrides (`null` removes a header).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<IndexMap<String, Option<String>>>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn template() -> AssistantMessage {
        AssistantMessage::empty("faux", "faux", "faux-1", 0)
    }

    #[test]
    fn assembler_rebuilds_partials() {
        let mut assembler = PartialAssembler::new(template());
        let events: Vec<ProviderEvent> = serde_json::from_value(json!([
            {"type":"start"},
            {"type":"thinking_start","contentIndex":0},
            {"type":"thinking_delta","contentIndex":0,"delta":"pl"},
            {"type":"thinking_end","contentIndex":0,"content":"plan"},
            {"type":"text_start","contentIndex":1},
            {"type":"text_delta","contentIndex":1,"delta":"Hel"},
            {"type":"text_delta","contentIndex":1,"delta":"lo"},
            {"type":"toolcall_start","contentIndex":2,"id":"call_1","toolName":"read"},
            {"type":"toolcall_delta","contentIndex":2,"delta":"{\"path\":"},
            {"type":"toolcall_delta","contentIndex":2,"delta":"\"a\"}"},
        ]))
        .unwrap_or_default();
        assert_eq!(events.len(), 10);
        let mut last = None;
        for event in events {
            last = Some(assembler.apply(event));
        }
        let partial = assembler.partial();
        assert_eq!(partial.content.len(), 3);
        assert_eq!(
            serde_json::to_value(&partial.content).ok(),
            Some(
                json!([{"type":"thinking","thinking":"plan"},{"type":"text","text":"Hello"},
                {"type":"toolCall","id":"call_1","name":"read","arguments":{"path":"a"}}])
            )
        );
        let wire = last
            .and_then(|e| serde_json::to_value(e).ok())
            .unwrap_or_default();
        assert_eq!(wire["type"], json!("toolcall_delta"));
        assert_eq!(wire["contentIndex"], json!(2));
        assert_eq!(wire["partial"]["stopReason"], json!("pending"));
    }

    #[test]
    fn stream_options_wire_names() {
        let options = StreamOptions {
            reasoning: Some(ThinkingLevel::High),
            transport: Some(Transport::WebsocketCached),
            temperature: Some(1.0),
            ..StreamOptions::default()
        };
        assert_eq!(
            serde_json::to_value(options).ok(),
            Some(json!({"reasoning":"high","temperature":1,"transport":"websocket-cached"}))
        );
    }
}
