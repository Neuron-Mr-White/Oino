//! pi-ai `Message` and the coding-agent `AgentMessage` union.

use indexmap::IndexMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::content::{AssistantContent, UserContent, UserContentField, UserTextField};
use crate::json::{JsonObject, JsonValue};
use crate::primitives::{StopReason, ThinkingLevel, TimestampMs, Usage};

/// Tool declaration carried by system messages (pi `Tool`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolDecl {
    /// Tool name.
    pub name: String,
    /// Model-facing description.
    pub description: String,
    /// JSON schema of the arguments.
    #[serde(default)]
    pub parameters: JsonValue,
    /// Constrained-sampling preference.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub constrained_sampling: Option<ConstrainedSampling>,
}

/// pi `false | ConstrainedSamplingConfig`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ConstrainedSampling {
    /// Literal `false`: disabled.
    Disabled(FalseLiteral),
    /// Configuration object.
    Config(ConstrainedSamplingConfig),
}

/// The JSON literal `false`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct FalseLiteral;

impl Serialize for FalseLiteral {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_bool(false)
    }
}

impl<'de> Deserialize<'de> for FalseLiteral {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        if bool::deserialize(deserializer)? {
            Err(serde::de::Error::custom("expected `false`"))
        } else {
            Ok(Self)
        }
    }
}

/// `{type:"json_schema", strict}` or `{type:"grammar", variants}`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConstrainedSamplingConfig {
    /// `"json_schema"` or `"grammar"`.
    #[serde(rename = "type")]
    pub kind: String,
    /// Strictness for `json_schema`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub strict: Option<StrictMode>,
    /// Grammar per format (`openai_lark`, `openai_regex`) for `grammar`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub variants: Option<IndexMap<String, String>>,
}

/// Constrained-sampling strictness.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrictMode {
    /// Use strict mode when supported.
    Prefer,
    /// Fail when strict mode is unsupported.
    Require,
    /// Never strict.
    Off,
}

/// `{name}` reference used by `toolsRemoved`.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ToolRef {
    /// Tool name.
    pub name: String,
}

/// Ordered prompt sections; `None` (JSON `null`) removes a section during replay.
pub type Sections = IndexMap<String, Option<String>>;

/// System prompt and tool loadout (pi `SystemMessage`).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SystemMessage {
    /// Free-form prompt text.
    #[serde(default)]
    pub content: UserTextField,
    /// Section patch, order significant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sections: Option<Sections>,
    /// Tools declared (or redeclared) by this message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_added: Option<Vec<ToolDecl>>,
    /// Tools removed by this message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tools_removed: Option<Vec<ToolRef>>,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
}

/// User message (pi `UserMessage`).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessage {
    /// String or blocks.
    #[serde(default)]
    pub content: UserContentField,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
}

/// Assistant message (pi `AssistantMessage`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantMessage {
    /// Content blocks.
    #[serde(default)]
    pub content: Vec<AssistantContent>,
    /// Wire API (`anthropic-messages`, …).
    pub api: String,
    /// Provider id.
    pub provider: String,
    /// Model id that answered.
    pub model: String,
    /// Model reported by the provider response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_model: Option<String>,
    /// Provider response id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub response_id: Option<String>,
    /// Provider-specific effort string applied.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider_thinking_level: Option<String>,
    /// pi thinking level requested for this response.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level: Option<ThinkingLevel>,
    /// Non-fatal diagnostics.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<AssistantDiagnostic>>,
    /// Usage (required).
    #[serde(default)]
    pub usage: Usage,
    /// Stop reason (required).
    pub stop_reason: StopReason,
    /// Deferred response handle.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deferred: Option<DeferredHandle>,
    /// Error text for `error`/`aborted`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_message: Option<String>,
    /// Provider's raw stop reason.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_stop_reason: Option<String>,
    /// Provider end-of-turn flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end_turn: Option<bool>,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
    /// Wall time of the request.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

impl AssistantMessage {
    /// Empty assistant message for the given model (`stopReason: "pending"`), the starting
    /// point for streamed partials.
    #[must_use]
    pub fn empty(
        api: impl Into<String>,
        provider: impl Into<String>,
        model: impl Into<String>,
        timestamp: TimestampMs,
    ) -> Self {
        Self {
            content: Vec::new(),
            api: api.into(),
            provider: provider.into(),
            model: model.into(),
            response_model: None,
            response_id: None,
            provider_thinking_level: None,
            thinking_level: None,
            diagnostics: None,
            usage: Usage::default(),
            stop_reason: StopReason::Pending,
            deferred: None,
            error_message: None,
            raw_stop_reason: None,
            end_turn: None,
            timestamp,
            duration_ms: None,
        }
    }
}

/// Assistant diagnostic (pi `AssistantMessageDiagnostic`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AssistantDiagnostic {
    /// Diagnostic kind.
    #[serde(rename = "type")]
    pub kind: String,
    /// Time recorded.
    pub timestamp: TimestampMs,
    /// Error info.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<DiagnosticError>,
    /// Extra details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonObject>,
}

/// pi `DiagnosticErrorInfo`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticError {
    /// Error class name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Message.
    pub message: String,
    /// Stack trace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
    /// String or number code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub code: Option<JsonValue>,
}

/// Deferred response handle (pi `DeferredHandle`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeferredHandle {
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub model_id: String,
    /// Wire API.
    pub api: String,
    /// Provider-side id.
    pub id: String,
    /// Expiry (epoch ms).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<i64>,
    /// Suggested poll delay.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub poll_after_ms: Option<u64>,
    /// Opaque provider data.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::json::keep_null"
    )]
    pub data: Option<JsonValue>,
}

/// Status of a nested tool call.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NestedStatus {
    /// Succeeded.
    Ok,
    /// Failed.
    Error,
    /// Did not finish.
    Unfinished,
}

/// One nested tool call (pi `NestedToolCallRecord`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NestedToolCallRecord {
    /// Call id.
    pub id: String,
    /// Tool name.
    pub name: String,
    /// Arguments (omitted when too large).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments: Option<JsonObject>,
    /// Size of omitted arguments.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arguments_bytes: Option<u64>,
    /// Outcome.
    pub status: NestedStatus,
    /// Wall time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    /// Error text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// pi `NestedToolCalls`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct NestedToolCalls {
    /// Recorded calls.
    pub calls: Vec<NestedToolCallRecord>,
    /// `false` when records were dropped.
    pub complete: bool,
}

/// Tool result (pi `ToolResultMessage`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolResultMessage {
    /// Id of the answered tool call.
    pub tool_call_id: String,
    /// Tool name.
    pub tool_name: String,
    /// Text/image blocks.
    #[serde(default)]
    pub content: Vec<UserContent>,
    /// Tool-specific details (not sent to the LLM).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::json::keep_null"
    )]
    pub details: Option<JsonValue>,
    /// Usage of nested model calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Nested tool calls.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nested_calls: Option<NestedToolCalls>,
    /// Error flag.
    pub is_error: bool,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
    /// Wall time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
}

/// LLM-level message (pi-ai `Message`).
#[allow(
    clippy::large_enum_variant,
    reason = "variants mirror pi shapes; boxing would leak into every match across crates"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum Message {
    /// System prompt / tools.
    #[serde(rename = "system")]
    System(SystemMessage),
    /// User turn.
    #[serde(rename = "user")]
    User(UserMessage),
    /// Assistant turn.
    #[serde(rename = "assistant")]
    Assistant(AssistantMessage),
    /// Tool result.
    #[serde(rename = "toolResult")]
    ToolResult(ToolResultMessage),
}

impl Message {
    /// Message timestamp (epoch ms).
    #[must_use]
    pub const fn timestamp(&self) -> TimestampMs {
        match self {
            Self::System(m) => m.timestamp,
            Self::User(m) => m.timestamp,
            Self::Assistant(m) => m.timestamp,
            Self::ToolResult(m) => m.timestamp,
        }
    }

    /// pi role string.
    #[must_use]
    pub const fn role(&self) -> &'static str {
        match self {
            Self::System(_) => "system",
            Self::User(_) => "user",
            Self::Assistant(_) => "assistant",
            Self::ToolResult(_) => "toolResult",
        }
    }
}

/// `!` bash execution recorded by the coding agent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BashExecutionMessage {
    /// Command line.
    pub command: String,
    /// Combined output.
    #[serde(default)]
    pub output: String,
    /// Exit code (`None` when killed / unknown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// Cancelled by the user.
    #[serde(default)]
    pub cancelled: bool,
    /// Output was truncated.
    #[serde(default)]
    pub truncated: bool,
    /// Path of the full output when truncated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_output_path: Option<String>,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
    /// `!!` commands: never sent to the LLM.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exclude_from_context: Option<bool>,
}

/// Extension-injected message (always in LLM context; `display` is UI only).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessage {
    /// Extension-defined type.
    pub custom_type: String,
    /// String or blocks.
    #[serde(default)]
    pub content: UserContentField,
    /// Whether frontends render it.
    #[serde(default)]
    pub display: bool,
    /// Extension metadata (not sent to the LLM).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::json::keep_null"
    )]
    pub details: Option<JsonValue>,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
}

/// Summary of an abandoned branch.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchSummaryMessage {
    /// Summary text.
    pub summary: String,
    /// Previous leaf id; serialised as `null` when `None` (field always present).
    #[serde(default)]
    pub from_id: Option<String>,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
}

/// Summary that replaces compacted history.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CompactionSummaryMessage {
    /// Summary text.
    pub summary: String,
    /// Context tokens before compaction.
    #[serde(default)]
    pub tokens_before: u64,
    /// Creation time.
    #[serde(default)]
    pub timestamp: TimestampMs,
}

/// Coding-agent message union (pi `AgentMessage`).
#[allow(
    clippy::large_enum_variant,
    reason = "variants mirror pi shapes; boxing would leak into every match across crates"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum AgentMessage {
    /// System prompt / tools.
    #[serde(rename = "system")]
    System(SystemMessage),
    /// User turn.
    #[serde(rename = "user")]
    User(UserMessage),
    /// Assistant turn.
    #[serde(rename = "assistant")]
    Assistant(AssistantMessage),
    /// Tool result.
    #[serde(rename = "toolResult")]
    ToolResult(ToolResultMessage),
    /// `!` command.
    #[serde(rename = "bashExecution")]
    BashExecution(BashExecutionMessage),
    /// Extension message.
    #[serde(rename = "custom")]
    Custom(CustomMessage),
    /// Branch summary.
    #[serde(rename = "branchSummary")]
    BranchSummary(BranchSummaryMessage),
    /// Compaction summary.
    #[serde(rename = "compactionSummary")]
    CompactionSummary(CompactionSummaryMessage),
}

impl AgentMessage {
    /// The LLM message when this is one of the four pi-ai roles.
    #[must_use]
    pub fn as_llm(&self) -> Option<Message> {
        match self {
            Self::System(m) => Some(Message::System(m.clone())),
            Self::User(m) => Some(Message::User(m.clone())),
            Self::Assistant(m) => Some(Message::Assistant(m.clone())),
            Self::ToolResult(m) => Some(Message::ToolResult(m.clone())),
            _ => None,
        }
    }

    /// pi role string.
    #[must_use]
    pub const fn role(&self) -> &'static str {
        match self {
            Self::System(_) => "system",
            Self::User(_) => "user",
            Self::Assistant(_) => "assistant",
            Self::ToolResult(_) => "toolResult",
            Self::BashExecution(_) => "bashExecution",
            Self::Custom(_) => "custom",
            Self::BranchSummary(_) => "branchSummary",
            Self::CompactionSummary(_) => "compactionSummary",
        }
    }

    /// Message timestamp (epoch ms).
    #[must_use]
    pub const fn timestamp(&self) -> TimestampMs {
        match self {
            Self::System(m) => m.timestamp,
            Self::User(m) => m.timestamp,
            Self::Assistant(m) => m.timestamp,
            Self::ToolResult(m) => m.timestamp,
            Self::BashExecution(m) => m.timestamp,
            Self::Custom(m) => m.timestamp,
            Self::BranchSummary(m) => m.timestamp,
            Self::CompactionSummary(m) => m.timestamp,
        }
    }

    /// The system message, if this is one.
    #[must_use]
    pub const fn as_system(&self) -> Option<&SystemMessage> {
        match self {
            Self::System(m) => Some(m),
            _ => None,
        }
    }
}

impl From<Message> for AgentMessage {
    fn from(message: Message) -> Self {
        match message {
            Message::System(m) => Self::System(m),
            Message::User(m) => Self::User(m),
            Message::Assistant(m) => Self::Assistant(m),
            Message::ToolResult(m) => Self::ToolResult(m),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip(raw: &JsonValue) -> Option<JsonValue> {
        let message: AgentMessage = serde_json::from_value(raw.clone()).ok()?;
        serde_json::to_value(message).ok()
    }

    #[test]
    fn pi_doc_examples_round_trip() {
        for raw in [
            json!({"role":"system","content":"","sections":{"preamble":"You are…","tools":"<tools>","cwd":"/p"},
                   "toolsAdded":[{"name":"read","description":"...","parameters":{}}],"timestamp":1_733_234_400_000_i64}),
            json!({"role":"system","content":"","sections":{"skills":"<skills>","gone":null},"toolsRemoved":[{"name":"write"}],"timestamp":1}),
            json!({"role":"user","content":"Hello","timestamp":1}),
            json!({"role":"assistant","content":[{"type":"text","text":"Hi!"}],"api":"anthropic-messages","provider":"anthropic",
                   "model":"claude-sonnet-4-5","usage":{"input":1,"output":2,"cacheRead":0,"cacheWrite":0,"totalTokens":3,
                   "cost":{"input":0,"output":0,"cacheRead":0,"cacheWrite":0,"total":0}},"stopReason":"stop","timestamp":2}),
            json!({"role":"toolResult","toolCallId":"call_123","toolName":"bash","content":[{"type":"text","text":"output"}],"isError":false,"timestamp":3}),
            json!({"role":"bashExecution","command":"ls","output":"","exitCode":0,"cancelled":false,"truncated":false,"timestamp":4}),
            json!({"role":"custom","customType":"x","content":"c","display":true,"details":{"a":1},"timestamp":5}),
            json!({"role":"branchSummary","summary":"s","fromId":null,"timestamp":6}),
            json!({"role":"compactionSummary","summary":"s","tokensBefore":10,"timestamp":7}),
        ] {
            assert_eq!(round_trip(&raw), Some(raw));
        }
    }

    #[test]
    fn constrained_sampling_false_and_config() {
        let decl: ToolDecl = serde_json::from_value(
            json!({"name":"a","description":"b","parameters":{},"constrainedSampling":false}),
        )
        .unwrap_or_else(|_| ToolDecl {
            name: String::new(),
            description: String::new(),
            parameters: JsonValue::Null,
            constrained_sampling: None,
        });
        assert_eq!(
            decl.constrained_sampling,
            Some(ConstrainedSampling::Disabled(FalseLiteral))
        );
        let raw = json!({"type":"json_schema","strict":"prefer"});
        let config: Option<ConstrainedSampling> = serde_json::from_value(raw.clone()).ok();
        assert!(matches!(config, Some(ConstrainedSampling::Config(_))));
        assert_eq!(serde_json::to_value(config).ok(), Some(raw));
        assert!(serde_json::from_value::<FalseLiteral>(json!(true)).is_err());
    }
}
