//! Client events (runtime.md §6): pi `AgentSessionEvent` plus the other client events.
//!
//! These types will move to `oino-protocol::events` once that crate exists; the JSON shapes
//! are already the wire shapes (pi field names, `type` tags).

use oino_agent::{AgentEvent, AgentToolResult};
use oino_ai::{
    AgentMessage, AssistantMessageEvent, JsonObject, Model, ThinkingLevel, ToolResultMessage,
};
use oino_session::SessionEntry;
use serde::{Deserialize, Serialize};

use crate::compaction::CompactionResult;
use crate::ui::{DismissReason, UiRequest};

/// Why compaction ran (pi `"manual" | "threshold" | "overflow"`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CompactionReason {
    /// `/compact`, RPC or an extension.
    Manual,
    /// Context crossed the threshold.
    Threshold,
    /// The provider reported a context overflow.
    Overflow,
}

/// What a summarisation retry belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "source")]
pub enum SummarizationSource {
    /// A branch summary.
    #[serde(rename = "branchSummary")]
    BranchSummary,
    /// A compaction.
    #[serde(rename = "compaction")]
    Compaction {
        /// Compaction trigger.
        reason: CompactionReason,
    },
}

/// pi `AgentSessionEvent`.
#[allow(
    clippy::large_enum_variant,
    reason = "variants mirror pi event shapes; events are moved through channels one at a time"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentSessionEvent {
    /// A run started.
    AgentStart,
    /// A run ended.
    #[serde(rename_all = "camelCase")]
    AgentEnd {
        /// Messages the run added.
        messages: Vec<AgentMessage>,
        /// An automatic retry follows.
        will_retry: bool,
    },
    /// The session became idle after activity.
    AgentSettled {
        /// The activity was aborted.
        aborted: bool,
    },
    /// A turn started.
    TurnStart,
    /// A turn ended.
    #[serde(rename_all = "camelCase")]
    TurnEnd {
        /// Assistant message of the turn.
        message: AgentMessage,
        /// Tool results (serialised with `role`).
        #[serde(with = "tagged_tool_results")]
        tool_results: Vec<ToolResultMessage>,
    },
    /// A message started.
    MessageStart {
        /// The message.
        message: AgentMessage,
    },
    /// The streaming message changed.
    #[serde(rename_all = "camelCase")]
    MessageUpdate {
        /// Partial message.
        message: AgentMessage,
        /// Provider event.
        assistant_message_event: AssistantMessageEvent,
    },
    /// A message is final.
    MessageEnd {
        /// The message.
        message: AgentMessage,
    },
    /// A tool call started.
    #[serde(rename_all = "camelCase")]
    ToolExecutionStart {
        /// Tool-call id.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Arguments.
        args: JsonObject,
        /// Calling tool for nested calls.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
    },
    /// A tool published a partial result.
    #[serde(rename_all = "camelCase")]
    ToolExecutionUpdate {
        /// Tool-call id.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Arguments.
        args: JsonObject,
        /// Partial result.
        partial_result: AgentToolResult,
        /// Calling tool for nested calls.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
    },
    /// A tool call settled.
    #[serde(rename_all = "camelCase")]
    ToolExecutionEnd {
        /// Tool-call id.
        tool_call_id: String,
        /// Tool name.
        tool_name: String,
        /// Result.
        result: AgentToolResult,
        /// Error flag.
        is_error: bool,
        /// Wall time.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        duration_ms: Option<u64>,
        /// Calling tool for nested calls.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_tool_call_id: Option<String>,
    },
    /// Complete steering / follow-up queues.
    #[serde(rename_all = "camelCase")]
    QueueUpdate {
        /// Steering texts.
        steering: Vec<String>,
        /// Follow-up texts.
        follow_up: Vec<String>,
    },
    /// Compaction started.
    CompactionStart {
        /// Trigger.
        reason: CompactionReason,
    },
    /// Compaction finished.
    #[serde(rename_all = "camelCase")]
    CompactionEnd {
        /// Trigger.
        reason: CompactionReason,
        /// Result (absent on failure).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        result: Option<CompactionResult>,
        /// Cancelled.
        aborted: bool,
        /// The interrupted turn is retried.
        will_retry: bool,
        /// Failure text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    },
    /// An extension appended an entry.
    EntryAppended {
        /// The entry.
        entry: SessionEntry,
    },
    /// The session name changed (absent = cleared).
    SessionInfoChanged {
        /// New name.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
    /// The thinking level changed.
    ThinkingLevelChanged {
        /// New level.
        level: ThinkingLevel,
    },
    /// An automatic retry is scheduled.
    #[serde(rename_all = "camelCase")]
    AutoRetryStart {
        /// Attempt number (1-based).
        attempt: u32,
        /// Retry budget.
        max_attempts: u32,
        /// Backoff delay.
        delay_ms: u64,
        /// Error that triggered the retry.
        error_message: String,
    },
    /// Automatic retries ended.
    #[serde(rename_all = "camelCase")]
    AutoRetryEnd {
        /// A retry succeeded.
        success: bool,
        /// Last attempt number.
        attempt: u32,
        /// Final error.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        final_error: Option<String>,
    },
    /// A summarisation retry is scheduled.
    #[serde(rename_all = "camelCase")]
    SummarizationRetryScheduled {
        /// Attempt number.
        attempt: u32,
        /// Retry budget.
        max_attempts: u32,
        /// Backoff delay.
        delay_ms: u64,
        /// Error that triggered the retry.
        error_message: String,
    },
    /// A summarisation retry attempt started.
    SummarizationRetryAttemptStart {
        /// What is being summarised.
        #[serde(flatten)]
        source: SummarizationSource,
    },
    /// Summarisation retries ended.
    SummarizationRetryFinished,
    /// Streaming output of a `!` command.
    BashExecutionUpdate {
        /// Caller-supplied id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        id: Option<String>,
        /// Appended output.
        delta: String,
    },
}

impl AgentSessionEvent {
    /// Maps a loop event to its session event (`agent_end` needs `will_retry`).
    #[must_use]
    pub fn from_agent(event: AgentEvent, will_retry: bool) -> Self {
        match event {
            AgentEvent::AgentStart => Self::AgentStart,
            AgentEvent::AgentEnd { messages } => Self::AgentEnd {
                messages,
                will_retry,
            },
            AgentEvent::TurnStart => Self::TurnStart,
            AgentEvent::TurnEnd {
                message,
                tool_results,
            } => Self::TurnEnd {
                message,
                tool_results,
            },
            AgentEvent::MessageStart { message } => Self::MessageStart { message },
            AgentEvent::MessageUpdate {
                message,
                assistant_message_event,
            } => Self::MessageUpdate {
                message,
                assistant_message_event,
            },
            AgentEvent::MessageEnd { message } => Self::MessageEnd { message },
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => Self::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
                parent_tool_call_id: None,
            },
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                tool_name,
                args,
                partial_result,
            } => Self::ToolExecutionUpdate {
                tool_call_id,
                tool_name,
                args,
                partial_result,
                parent_tool_call_id: None,
            },
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
                duration_ms,
            } => Self::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
                duration_ms,
                parent_tool_call_id: None,
            },
        }
    }
}

/// pi `ExtensionError` as a client event (`extension_error`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExtensionErrorEvent {
    /// Extension id/path or a `<runtime>`-style marker.
    pub extension_path: String,
    /// Event or call that failed.
    pub event: String,
    /// Error text.
    pub error: String,
    /// Stack trace, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stack: Option<String>,
}

/// Why a session was replaced.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ReplaceReason {
    /// New session.
    New,
    /// Switched to another file.
    Resume,
    /// Forked from an entry.
    Fork,
    /// Cloned at the leaf.
    Clone,
    /// Imported JSONL.
    Import,
}

/// What changed the model.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ModelChangeSource {
    /// `set_model`.
    Set,
    /// Cycling.
    Cycle,
    /// Restored from the session or defaults.
    Restore,
    /// An extension.
    Extension,
}

/// Coarse run state for frontends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    /// Nothing runs.
    Idle,
    /// An agent run is active.
    Running,
    /// Compaction or branch summary.
    Compacting,
    /// Waiting before an automatic retry.
    Retrying,
    /// Idle, but a wait source reports work in flight.
    Waiting,
}

/// Oino client events (runtime §6.2).
#[allow(
    clippy::large_enum_variant,
    reason = "events are moved through channels one at a time"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OinoEvent {
    /// The runtime replaced the session; clients reset view state.
    #[serde(rename_all = "camelCase")]
    SessionReplaced {
        /// New session file.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        session_file: Option<String>,
        /// New session id.
        session_id: String,
        /// Why.
        reason: ReplaceReason,
    },
    /// The selected model changed.
    #[serde(rename_all = "camelCase")]
    ModelChanged {
        /// Selected model.
        model: Model,
        /// Physical model under a virtual selection.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        routed_model: Option<Model>,
        /// What changed it.
        source: ModelChangeSource,
    },
    /// Coarse run state.
    #[serde(rename_all = "camelCase")]
    RunState {
        /// State.
        state: RunState,
        /// Wait reasons for `waiting`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        waiting_on: Option<Vec<String>>,
    },
    /// The active tools changed.
    ToolsChanged {
        /// Active tool names.
        active: Vec<String>,
    },
    /// Registered commands changed; refetch `get_commands`.
    CommandsChanged,
    /// A dialog closed (`extension_ui_dismiss`); other frontends close it too.
    ExtensionUiDismiss {
        /// Request id.
        id: String,
        /// Why it closed.
        reason: DismissReason,
    },
    /// Events were dropped for a slow subscriber.
    Lagged {
        /// Number of dropped events.
        dropped: u64,
    },
}

/// Every event a client receives, in one ordered stream.
#[allow(
    clippy::large_enum_variant,
    reason = "events are moved through channels one at a time"
)]
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ClientEvent {
    /// pi session event.
    Session(AgentSessionEvent),
    /// Extension UI request (`extension_ui_request`).
    Ui(UiRequest),
    /// Extension failure (`extension_error`).
    ExtensionError(TaggedExtensionError),
    /// Oino addition.
    Oino(OinoEvent),
}

/// [`ExtensionErrorEvent`] with its `type: "extension_error"` tag.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename = "extension_error")]
pub struct TaggedExtensionError {
    /// The error.
    #[serde(flatten)]
    pub error: ExtensionErrorEvent,
}

impl From<AgentSessionEvent> for ClientEvent {
    fn from(event: AgentSessionEvent) -> Self {
        Self::Session(event)
    }
}

impl From<OinoEvent> for ClientEvent {
    fn from(event: OinoEvent) -> Self {
        Self::Oino(event)
    }
}

impl From<ExtensionErrorEvent> for ClientEvent {
    fn from(error: ExtensionErrorEvent) -> Self {
        Self::ExtensionError(TaggedExtensionError { error })
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn wire_shapes() {
        let event = ClientEvent::Session(AgentSessionEvent::SummarizationRetryAttemptStart {
            source: SummarizationSource::Compaction {
                reason: CompactionReason::Overflow,
            },
        });
        assert_eq!(
            serde_json::to_value(&event).unwrap(),
            json!({"type":"summarization_retry_attempt_start","source":"compaction","reason":"overflow"})
        );
        let error: ClientEvent = ExtensionErrorEvent {
            extension_path: "x".into(),
            event: "input".into(),
            error: "boom".into(),
            stack: None,
        }
        .into();
        assert_eq!(
            serde_json::to_value(&error).unwrap(),
            json!({"type":"extension_error","extensionPath":"x","event":"input","error":"boom"})
        );
        assert_eq!(
            serde_json::to_value(ClientEvent::Session(AgentSessionEvent::QueueUpdate {
                steering: vec!["a".into()],
                follow_up: vec![]
            }))
            .unwrap(),
            json!({"type":"queue_update","steering":["a"],"followUp":[]})
        );
    }
}
