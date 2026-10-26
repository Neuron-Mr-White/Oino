//! Loop configuration and hooks (pi-agent-core `AgentLoopConfig`).
//!
//! Hooks are traits so that `oino-core` can implement them on its session type. Every hook
//! returns a boxed future; an `Err(HookError)` from a loop-level hook ends the run the way a
//! thrown error ends a pi run (the [`crate::Agent`] turns it into a failure message). Errors
//! from tool hooks become error tool results instead, as in pi.

use std::sync::Arc;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::{
    AgentMessage, AssistantMessage, AssistantMessageEvent, Context, JsonObject, JsonValue, Message,
    Model, StreamOptions, ThinkingLevel, ToolCall, ToolResultMessage, Usage, UserContent,
};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::tool::{AgentTool, AgentToolResult};

/// Error raised by a loop hook; its text becomes the run's `errorMessage`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct HookError(pub String);

impl HookError {
    /// Error with the given message.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

/// Messages and executable tools of a run (pi `AgentContext`).
#[derive(Clone, Default)]
pub struct AgentContext {
    /// Transcript (all roles).
    pub messages: Vec<AgentMessage>,
    /// Tools the runtime can execute.
    pub tools: Vec<Arc<dyn AgentTool>>,
}

impl std::fmt::Debug for AgentContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentContext")
            .field("messages", &self.messages.len())
            .field(
                "tools",
                &self
                    .tools
                    .iter()
                    .map(|t| t.meta().name.clone())
                    .collect::<Vec<_>>(),
            )
            .finish()
    }
}

impl AgentContext {
    /// The tool with `name`, if executable.
    #[must_use]
    pub fn tool(&self, name: &str) -> Option<&Arc<dyn AgentTool>> {
        self.tools.iter().find(|tool| tool.meta().name == name)
    }
}

/// One provider request.
#[derive(Clone, Debug)]
pub struct StreamRequest {
    /// Model to call.
    pub model: Model,
    /// Normalised transcript.
    pub context: Context,
    /// Stream options with `reasoning` set from the thinking level.
    pub options: StreamOptions,
    /// Credential resolved for this request.
    pub api_key: Option<String>,
    /// Abort signal.
    pub cancel: CancellationToken,
}

/// Provider dispatch (pi `StreamFn`). The stream must end with `Done` or `Error`.
pub trait StreamFn: Send + Sync {
    /// Starts one request.
    fn stream(&self, request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent>;
}

/// `AgentMessage[]` → `Message[]` at the request boundary (pi `convertToLlm`).
pub type ConvertToLlm = Arc<dyn Fn(&[AgentMessage]) -> Vec<Message> + Send + Sync>;

/// pi `transformContext`: rewrites the transcript before conversion.
pub trait TransformContext: Send + Sync {
    /// Returns the messages to convert.
    fn transform<'a>(
        &'a self,
        messages: Vec<AgentMessage>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Vec<AgentMessage>, HookError>>;
}

/// pi `getApiKey`: resolves a short-lived credential per request.
pub trait GetApiKey: Send + Sync {
    /// Credential for `provider`, if any.
    fn api_key<'a>(&'a self, provider: &'a str)
    -> BoxFuture<'a, Result<Option<String>, HookError>>;
}

/// pi `BeforeToolCallContext`.
#[derive(Clone, Copy, Debug)]
pub struct BeforeToolCallContext<'a> {
    /// Assistant message that issued the call.
    pub assistant_message: &'a AssistantMessage,
    /// The call as issued.
    pub tool_call: &'a ToolCall,
    /// Validated arguments.
    pub args: &'a JsonObject,
    /// Current run context.
    pub context: &'a AgentContext,
}

/// pi `BeforeToolCallResult`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BeforeToolCallResult {
    /// Block the call.
    #[serde(default)]
    pub block: bool,
    /// Text of the error result when blocked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Mark the blocked result as terminating.
    #[serde(default)]
    pub terminate: bool,
    /// Replacement arguments for the call (Oino: pi handlers mutate `args` in place).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<JsonObject>,
}

/// pi `beforeToolCall`.
pub trait BeforeToolCall: Send + Sync {
    /// Inspects (and may block) a validated call.
    fn before<'a>(
        &'a self,
        cx: BeforeToolCallContext<'a>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<BeforeToolCallResult>, HookError>>;
}

/// pi `AfterToolCallContext`.
#[derive(Clone, Copy, Debug)]
pub struct AfterToolCallContext<'a> {
    /// Assistant message that issued the call.
    pub assistant_message: &'a AssistantMessage,
    /// The call as issued.
    pub tool_call: &'a ToolCall,
    /// Validated arguments.
    pub args: &'a JsonObject,
    /// Result produced by the tool.
    pub result: &'a AgentToolResult,
    /// Error flag of the result.
    pub is_error: bool,
    /// Current run context.
    pub context: &'a AgentContext,
}

/// pi `AfterToolCallResult`: field-wise overrides.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AfterToolCallResult {
    /// Replacement content (drops `structuredContent` unless also replaced).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content: Option<Vec<UserContent>>,
    /// Replacement details.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
    /// Replacement structured content.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub structured_content: Option<JsonValue>,
    /// Replacement error flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub is_error: Option<bool>,
    /// Replacement usage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Replacement terminate flag.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminate: Option<bool>,
}

/// pi `afterToolCall`.
pub trait AfterToolCall: Send + Sync {
    /// Post-processes a result.
    fn after<'a>(
        &'a self,
        cx: AfterToolCallContext<'a>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<AfterToolCallResult>, HookError>>;
}

/// pi `AgentTurnContext` / `PrepareNextTurnContext`.
#[derive(Clone, Copy, Debug)]
pub struct AgentTurnContext<'a> {
    /// Assistant message of the turn.
    pub message: &'a AssistantMessage,
    /// Tool results of the turn.
    pub tool_results: &'a [ToolResultMessage],
    /// Context after the turn.
    pub context: &'a AgentContext,
    /// Messages added by the run so far.
    pub new_messages: &'a [AgentMessage],
}

/// pi `AgentTurnDecision`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "lowercase")]
pub enum TurnDecision {
    /// Request one more (context-only) turn unless work is already scheduled.
    Continue,
    /// End the run after this turn.
    End,
}

/// pi `finishTurn`: decides after each turn, before `turn_end`.
pub trait FinishTurn: Send + Sync {
    /// Decision for this turn.
    fn finish<'a>(
        &'a self,
        turn: AgentTurnContext<'a>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<TurnDecision>, HookError>>;
}

/// pi `AgentLoopTurnUpdate`.
#[derive(Clone, Debug, Default)]
pub struct AgentLoopTurnUpdate {
    /// Replacement context.
    pub context: Option<AgentContext>,
    /// Messages to add before the next request.
    pub messages: Option<Vec<AgentMessage>>,
    /// Replacement model.
    pub model: Option<Model>,
    /// Replacement thinking level.
    pub thinking_level: Option<ThinkingLevel>,
}

/// pi `prepareNextTurn`: runs after `turn_end` when the run continues (e.g. compaction).
pub trait PrepareNextTurn: Send + Sync {
    /// Optional update for the next turn.
    fn prepare<'a>(
        &'a self,
        turn: AgentTurnContext<'a>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<AgentLoopTurnUpdate>, HookError>>;
}

/// pi `PrepareRequestContext`.
#[derive(Clone, Copy, Debug)]
pub struct PrepareRequestContext<'a> {
    /// Context about to be sent.
    pub context: &'a AgentContext,
    /// Model about to be called.
    pub model: &'a Model,
    /// Thinking level about to be requested.
    pub thinking_level: ThinkingLevel,
}

/// pi `AgentRequestUpdate`.
#[derive(Clone, Debug, Default)]
pub struct AgentRequestUpdate {
    /// Replacement context.
    pub context: Option<AgentContext>,
    /// Replacement model.
    pub model: Option<Model>,
    /// Replacement thinking level.
    pub thinking_level: Option<ThinkingLevel>,
}

/// pi `prepareRequest`: runs before every request, including the first (virtual routing).
pub trait PrepareRequest: Send + Sync {
    /// Optional update for this request.
    fn prepare<'a>(
        &'a self,
        request: PrepareRequestContext<'a>,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<AgentRequestUpdate>, HookError>>;
}

/// pi `getSteeringMessages` / `getFollowUpMessages`.
pub trait QueueSource: Send + Sync {
    /// Drains the next messages (possibly none).
    fn drain(&self) -> BoxFuture<'_, Vec<AgentMessage>>;
}

/// pi `ToolExecutionMode` of the loop.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecution {
    /// Prepare sequentially, execute concurrently.
    #[default]
    Parallel,
    /// One call after another.
    Sequential,
}

/// Configuration of one loop run (pi `AgentLoopConfig`).
///
/// Provider-level callbacks (`onPayload`, `onResponse`, `onProviderStreamEvent`) belong to the
/// [`StreamFn`] implementation in Oino, which owns the provider hooks.
#[derive(Clone)]
pub struct AgentLoopConfig {
    /// Model of the first request.
    pub model: Model,
    /// Thinking level of the first request (`Off` sends no `reasoning`).
    pub thinking_level: ThinkingLevel,
    /// Base stream options (`reasoning` is overwritten per request).
    pub stream_options: StreamOptions,
    /// Provider dispatch.
    pub stream: Arc<dyn StreamFn>,
    /// Request-boundary conversion.
    pub convert_to_llm: ConvertToLlm,
    /// Context rewrite before conversion.
    pub transform_context: Option<Arc<dyn TransformContext>>,
    /// Per-request credential.
    pub get_api_key: Option<Arc<dyn GetApiKey>>,
    /// Static credential fallback (pi `apiKey`).
    pub api_key: Option<String>,
    /// Tool gate.
    pub before_tool_call: Option<Arc<dyn BeforeToolCall>>,
    /// Tool result post-processing.
    pub after_tool_call: Option<Arc<dyn AfterToolCall>>,
    /// Turn decision.
    pub finish_turn: Option<Arc<dyn FinishTurn>>,
    /// Per-request update.
    pub prepare_request: Option<Arc<dyn PrepareRequest>>,
    /// Between-turn update.
    pub prepare_next_turn: Option<Arc<dyn PrepareNextTurn>>,
    /// Steering queue.
    pub get_steering_messages: Option<Arc<dyn QueueSource>>,
    /// Follow-up queue.
    pub get_follow_up_messages: Option<Arc<dyn QueueSource>>,
    /// Batch scheduling.
    pub tool_execution: ToolExecution,
}

impl AgentLoopConfig {
    /// A config with no hooks and pi's default conversion (system/user/assistant/toolResult).
    #[must_use]
    pub fn new(model: Model, stream: Arc<dyn StreamFn>) -> Self {
        Self {
            model,
            thinking_level: ThinkingLevel::Off,
            stream_options: StreamOptions::default(),
            stream,
            convert_to_llm: Arc::new(default_convert_to_llm),
            transform_context: None,
            get_api_key: None,
            api_key: None,
            before_tool_call: None,
            after_tool_call: None,
            finish_turn: None,
            prepare_request: None,
            prepare_next_turn: None,
            get_steering_messages: None,
            get_follow_up_messages: None,
            tool_execution: ToolExecution::Parallel,
        }
    }
}

/// pi-agent-core `defaultConvertToLlm`: keeps the four LLM roles.
#[must_use]
pub fn default_convert_to_llm(messages: &[AgentMessage]) -> Vec<Message> {
    messages.iter().filter_map(AgentMessage::as_llm).collect()
}
