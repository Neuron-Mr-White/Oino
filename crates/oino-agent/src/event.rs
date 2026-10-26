//! Loop events (pi-agent-core `AgentEvent`).

use futures::future::BoxFuture;
use oino_ai::{AgentMessage, AssistantMessageEvent, JsonObject, ToolResultMessage};
use serde::{Deserialize, Serialize};

use crate::tool::AgentToolResult;

/// One lifecycle event of an agent run, serialised exactly like pi.
#[allow(
    clippy::large_enum_variant,
    reason = "variants mirror pi event shapes; events are moved, not stored in bulk"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    /// A run started.
    AgentStart,
    /// A run finished; `messages` are the messages the run added.
    AgentEnd {
        /// Messages added by the run.
        messages: Vec<AgentMessage>,
    },
    /// A turn (one assistant response plus its tool calls) started.
    TurnStart,
    /// A turn finished.
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        /// The assistant message of the turn.
        message: AgentMessage,
        /// Tool results of the turn in assistant source order (serialised with `role`).
        #[serde(with = "tagged_tool_results")]
        tool_results: Vec<ToolResultMessage>,
    },
    /// A message (any role) entered the transcript or started streaming.
    MessageStart {
        /// Snapshot of the message.
        message: AgentMessage,
    },
    /// The streaming assistant message changed.
    #[serde(rename_all = "camelCase")]
    MessageUpdate {
        /// Snapshot of the partial message.
        message: AgentMessage,
        /// The provider event that caused the change.
        assistant_message_event: AssistantMessageEvent,
    },
    /// A message is final.
    MessageEnd {
        /// The final message.
        message: AgentMessage,
    },
    /// A tool call started (before argument validation).
    #[serde(rename_all = "camelCase")]
    ToolExecutionStart {
        /// Tool-call id.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Raw arguments from the model.
        args: JsonObject,
    },
    /// A running tool published a partial result.
    #[serde(rename_all = "camelCase")]
    ToolExecutionUpdate {
        /// Tool-call id.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Raw arguments from the model.
        args: JsonObject,
        /// The partial result.
        partial_result: AgentToolResult,
    },
    /// A tool call settled.
    #[serde(rename_all = "camelCase")]
    ToolExecutionEnd {
        /// Tool-call id.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Final result.
        result: AgentToolResult,
        /// Error flag.
        is_error: bool,
        /// Execution wall time (absent when the tool never ran).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
    },
}

impl AgentEvent {
    /// pi `type` string.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::AgentStart => "agent_start",
            Self::AgentEnd { .. } => "agent_end",
            Self::TurnStart => "turn_start",
            Self::TurnEnd { .. } => "turn_end",
            Self::MessageStart { .. } => "message_start",
            Self::MessageUpdate { .. } => "message_update",
            Self::MessageEnd { .. } => "message_end",
            Self::ToolExecutionStart { .. } => "tool_execution_start",
            Self::ToolExecutionUpdate { .. } => "tool_execution_update",
            Self::ToolExecutionEnd { .. } => "tool_execution_end",
        }
    }
}

/// Serialises tool results as pi does (`{"role":"toolResult",…}`).
mod tagged_tool_results {
    use oino_ai::{AgentMessage, ToolResultMessage};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub(super) fn serialize<S: Serializer>(
        results: &[ToolResultMessage],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let tagged: Vec<AgentMessage> = results
            .iter()
            .cloned()
            .map(AgentMessage::ToolResult)
            .collect();
        tagged.serialize(serializer)
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<ToolResultMessage>, D::Error> {
        let values = Vec::<serde_json::Value>::deserialize(deserializer)?;
        values
            .into_iter()
            .map(|mut value| {
                if let Some(object) = value.as_object_mut() {
                    object.shift_remove("role");
                }
                serde_json::from_value(value).map_err(serde::de::Error::custom)
            })
            .collect()
    }
}

/// Receiver of loop events. `emit` is awaited before the loop continues (pi `emit`).
pub trait EventSink: Send + Sync {
    /// Handles one event.
    fn emit(&self, event: AgentEvent) -> BoxFuture<'_, ()>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_shapes() {
        assert_eq!(
            serde_json::to_value(AgentEvent::AgentStart).unwrap(),
            json!({"type":"agent_start"})
        );
        let end = AgentEvent::ToolExecutionEnd {
            tool_call_id: "c1".into(),
            tool_name: "read".into(),
            result: AgentToolResult::error("x"),
            is_error: true,
            duration_ms: None,
        };
        assert_eq!(
            serde_json::to_value(end).unwrap(),
            json!({"type":"tool_execution_end","toolCallId":"c1","toolName":"read",
                "result":{"content":[{"type":"text","text":"x"}],"details":{}},"isError":true})
        );
    }
}
