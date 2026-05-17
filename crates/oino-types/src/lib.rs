#![doc = r#"Shared Oino runtime types.

This crate contains model-visible data (messages and content blocks) and runtime-visible
metadata (usage, stream events, stop reasons). It deliberately has no dependency on the
agent loop, session manager, harness, providers, filesystem, or async runtime.
"#]
#![forbid(unsafe_code)]

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;
use uuid::Uuid;

/// Stable identifier used for messages, content blocks, tool calls, and session entries.
pub type OinoId = Uuid;

/// A model selected by provider/model identifiers plus opaque metadata.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Model {
    pub provider: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

impl Model {
    #[must_use]
    pub fn new(provider: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            name: name.into(),
            metadata: BTreeMap::new(),
        }
    }

    #[must_use]
    pub fn identifier(&self) -> String {
        format!("{}:{}", self.provider, self.name)
    }

    #[must_use]
    pub fn from_identifier(identifier: &str) -> Option<Self> {
        let (provider, name) = identifier.split_once(':')?;
        let provider = provider.trim();
        let name = name.trim();
        if provider.is_empty()
            || name.is_empty()
            || provider.chars().any(char::is_whitespace)
            || name.chars().any(char::is_whitespace)
            || name.contains(':')
        {
            return None;
        }
        Some(Self::new(provider, name))
    }
}

/// Thinking/reasoning effort requested from a model adapter.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ThinkingLevel {
    #[default]
    Off,
    Minimal,
    Low,
    Medium,
    High,
    XHigh,
}

/// Token/cost usage reported by a provider adapter.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: Option<UsageCost>,
}

/// Monetary usage, normally normalized by provider adapters.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct UsageCost {
    pub amount: f64,
    pub currency: String,
}

/// Why a turn stopped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    EndTurn,
    ToolUse,
    Length,
    Error,
    Aborted,
    Unknown,
}

/// Opaque provider-side metadata that should not leak into pure loop serialization.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProviderMetadata {
    pub request_id: Option<String>,
    pub model: Option<Model>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub values: BTreeMap<String, Value>,
}

/// A transcript message. Most variants are model-visible after context reconstruction;
/// `Custom` is runtime-only unless converted by a harness/session rule.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Message {
    User {
        id: OinoId,
        content: Vec<ContentBlock>,
    },
    Assistant {
        id: OinoId,
        content: Vec<ContentBlock>,
        stop_reason: Option<StopReason>,
        usage: Option<Usage>,
        provider: Option<ProviderMetadata>,
    },
    ToolResult {
        id: OinoId,
        tool_call_id: OinoId,
        tool_name: String,
        content: Vec<ContentBlock>,
        is_error: bool,
        terminate: bool,
        details: Option<Value>,
    },
    Custom {
        id: OinoId,
        name: String,
        payload: Value,
        model_visible: bool,
    },
    CompactionSummary {
        id: OinoId,
        summary: String,
    },
    BranchSummary {
        id: OinoId,
        summary: String,
    },
}

impl Message {
    #[must_use]
    pub fn id(&self) -> OinoId {
        match self {
            Self::User { id, .. }
            | Self::Assistant { id, .. }
            | Self::ToolResult { id, .. }
            | Self::Custom { id, .. }
            | Self::CompactionSummary { id, .. }
            | Self::BranchSummary { id, .. } => *id,
        }
    }

    #[must_use]
    pub fn user_text(text: impl Into<String>) -> Self {
        Self::User {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text { text: text.into() }],
        }
    }

    #[must_use]
    pub fn assistant_text(text: impl Into<String>, stop_reason: StopReason) -> Self {
        Self::Assistant {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Text { text: text.into() }],
            stop_reason: Some(stop_reason),
            usage: None,
            provider: None,
        }
    }
}

/// Content blocks inside messages. `ToolCall` is model-visible in assistant messages;
/// `Thinking` visibility is decided by provider/harness policy.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Image {
        media_type: String,
        data: String,
    },
    Thinking {
        text: String,
        redacted: bool,
    },
    ToolCall {
        id: OinoId,
        name: String,
        arguments: Value,
    },
}

/// Typed events emitted by provider adapters into the pure agent loop.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AssistantStreamEvent {
    TextDelta {
        delta: String,
    },
    ThinkingDelta {
        delta: String,
    },
    ToolCallDelta {
        id: OinoId,
        name: Option<String>,
        arguments_delta: String,
    },
    ToolCallDone {
        id: OinoId,
        name: String,
        arguments: Value,
    },
    Usage {
        usage: Usage,
    },
    Done {
        stop_reason: StopReason,
        provider: Option<ProviderMetadata>,
    },
    Error {
        message: String,
    },
    Aborted,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip<T>(value: &T) -> T
    where
        T: Serialize + for<'de> Deserialize<'de>,
    {
        let encoded = match serde_json::to_string(value) {
            Ok(encoded) => encoded,
            Err(err) => panic!("serialize failed: {err}"),
        };
        match serde_json::from_str(&encoded) {
            Ok(decoded) => decoded,
            Err(err) => panic!("deserialize failed: {err}"),
        }
    }

    #[test]
    fn model_identifier_uses_provider_colon_name() {
        let model = Model::new("openrouter", "xai/glm-5.1");
        assert_eq!(model.identifier(), "openrouter:xai/glm-5.1");
        assert_eq!(
            Model::from_identifier("openrouter:xai/glm-5.1"),
            Some(model)
        );
        assert_eq!(Model::from_identifier("xai/glm-5.1"), None);
        assert_eq!(Model::from_identifier("openrouter:"), None);
        assert_eq!(Model::from_identifier("openrouter:xai/glm 5.1"), None);
    }

    #[test]
    fn message_json_round_trip() {
        let msg = Message::Assistant {
            id: Uuid::new_v4(),
            content: vec![
                ContentBlock::Text {
                    text: "hello".into(),
                },
                ContentBlock::ToolCall {
                    id: Uuid::new_v4(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path":"README.md"}),
                },
            ],
            stop_reason: Some(StopReason::ToolUse),
            usage: Some(Usage {
                input_tokens: 1,
                output_tokens: 2,
                cache_read_tokens: 0,
                cache_write_tokens: 0,
                cost: None,
            }),
            provider: None,
        };
        assert_eq!(msg, round_trip(&msg));
    }

    #[test]
    fn stream_event_json_round_trip() {
        let event = AssistantStreamEvent::ToolCallDone {
            id: Uuid::new_v4(),
            name: "bash".into(),
            arguments: serde_json::json!({"command":"true"}),
        };
        assert_eq!(event, round_trip(&event));
    }
}
