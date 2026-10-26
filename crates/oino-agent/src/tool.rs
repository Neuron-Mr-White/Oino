//! Tool contract (pi-agent-core `AgentTool`, `AgentToolResult`).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use futures::future::BoxFuture;
use oino_ai::{ConstrainedSampling, JsonObject, JsonValue, ToolDecl, Usage, UserContent};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

/// How a tool may be scheduled relative to other calls of the same assistant message
/// (pi `ToolExecutionMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// Runs concurrently with other calls of the batch.
    #[default]
    Parallel,
    /// Forces the whole batch to run sequentially.
    Sequential,
}

/// Whether a recorded call may be replayed (pi `AgentTool.replay`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Replay {
    /// Never replay.
    Never,
    /// Replay is side-effect free.
    Safe,
}

/// What the loop needs to know about a tool; core adds prompt and exposure metadata.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolDefinitionMeta {
    /// Model-facing name (unique per context).
    pub name: String,
    /// Human label for frontends.
    pub label: String,
    /// Model-facing description.
    pub description: String,
    /// JSON Schema of the arguments.
    pub parameters: JsonValue,
    /// JSON Schema of `structuredContent`.
    pub output_schema: Option<JsonValue>,
    /// Scheduling inside a tool batch.
    pub execution_mode: ExecutionMode,
    /// Replay policy.
    pub replay: Option<Replay>,
    /// Constrained-sampling preference sent with the declaration.
    pub constrained_sampling: Option<ConstrainedSampling>,
}

impl ToolDefinitionMeta {
    /// A parallel tool with no output schema, replay policy or sampling preference.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        label: impl Into<String>,
        description: impl Into<String>,
        parameters: JsonValue,
    ) -> Self {
        Self {
            name: name.into(),
            label: label.into(),
            description: description.into(),
            parameters,
            output_schema: None,
            execution_mode: ExecutionMode::Parallel,
            replay: None,
            constrained_sampling: None,
        }
    }

    /// pi `toToolDeclaration`: the model-visible declaration recorded in system messages.
    #[must_use]
    pub fn declaration(&self) -> ToolDecl {
        ToolDecl {
            name: self.name.clone(),
            description: self.description.clone(),
            parameters: self.parameters.clone(),
            constrained_sampling: self.constrained_sampling.clone(),
        }
    }
}

/// Result of a tool execution (pi `AgentToolResult`). Also used for partial updates.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentToolResult {
    /// Text and image blocks sent to the model.
    #[serde(default)]
    pub content: Vec<UserContent>,
    /// Tool-specific details for frontends (not sent to the model).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
    /// Machine-readable result matching the tool's output schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<JsonValue>,
    /// Usage of nested model calls made by the tool.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// The tool reports failure without raising an error.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub is_error: bool,
    /// Ask the loop to stop after this batch (honoured only when every result asks).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub terminate: bool,
}

impl AgentToolResult {
    /// A text-only result with no details.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![UserContent::text(text)],
            ..Self::default()
        }
    }

    /// pi `createErrorToolResult`: `{content:[{type:"text",text}], details:{}}`.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![UserContent::text(message)],
            details: Some(JsonValue::Object(JsonObject::new())),
            ..Self::default()
        }
    }
}

/// Partial results streamed while a tool runs.
pub type ToolUpdate = AgentToolResult;

/// Failure raised by a tool; the loop turns it into an error result with this text.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ToolError(pub String);

impl ToolError {
    /// Error with the given message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl From<String> for ToolError {
    fn from(message: String) -> Self {
        Self(message)
    }
}

impl From<&str> for ToolError {
    fn from(message: &str) -> Self {
        Self(message.to_owned())
    }
}

/// One validated call handed to [`AgentTool::execute`].
#[derive(Clone, Debug, PartialEq)]
pub struct ToolInvocation {
    /// Provider tool-call id (nested calls use `<parent>/<n>`).
    pub tool_call_id: String,
    /// Validated (and coerced) arguments.
    pub args: JsonObject,
    /// Calling tool for nested calls.
    pub parent_tool_call_id: Option<String>,
}

/// Sink for partial results (pi `onUpdate`). Cheap to clone; updates sent after the call
/// settled are dropped.
#[derive(Clone, Debug)]
pub struct UpdateSink {
    tx: Option<mpsc::UnboundedSender<ToolUpdate>>,
    open: Arc<AtomicBool>,
}

impl UpdateSink {
    /// A sink that discards every update.
    #[must_use]
    pub fn discard() -> Self {
        Self {
            tx: None,
            open: Arc::new(AtomicBool::new(false)),
        }
    }

    /// A sink feeding `tx`, plus the switch that closes it.
    pub(crate) fn channel() -> (Self, mpsc::UnboundedReceiver<ToolUpdate>, Arc<AtomicBool>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let open = Arc::new(AtomicBool::new(true));
        (
            Self {
                tx: Some(tx),
                open: Arc::clone(&open),
            },
            rx,
            open,
        )
    }

    /// Publishes a partial result.
    pub fn send(&self, update: ToolUpdate) {
        if !self.open.load(Ordering::Acquire) {
            return;
        }
        if let Some(tx) = &self.tx {
            // A closed receiver means the call settled; dropping the update is pi's behaviour.
            let _ = tx.send(update);
        }
    }

    /// Whether updates are still accepted.
    #[must_use]
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::Acquire)
    }
}

/// An executable tool (pi `AgentTool`).
pub trait AgentTool: Send + Sync {
    /// Static metadata.
    fn meta(&self) -> &ToolDefinitionMeta;

    /// pi `prepareArguments`: normalises raw arguments before validation.
    fn prepare_arguments(&self, raw: JsonObject) -> JsonObject {
        raw
    }

    /// Runs the call. `Err` becomes an error result with the error text.
    fn execute(
        &self,
        call: ToolInvocation,
        updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>>;
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn error_result_matches_pi_shape() {
        let value = serde_json::to_value(AgentToolResult::error("boom")).unwrap();
        assert_eq!(
            value,
            json!({"content":[{"type":"text","text":"boom"}],"details":{}})
        );
    }

    #[test]
    fn closed_sink_drops_updates() {
        let (sink, mut rx, open) = UpdateSink::channel();
        sink.send(AgentToolResult::text("a"));
        open.store(false, Ordering::Release);
        sink.send(AgentToolResult::text("b"));
        assert_eq!(rx.try_recv().ok(), Some(AgentToolResult::text("a")));
        assert!(rx.try_recv().is_err());
    }
}
