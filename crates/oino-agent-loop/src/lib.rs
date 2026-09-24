#![doc = r#"Pure async agent loop for Oino.

`oino-agent-loop` consumes typed `AssistantStreamEvent`s, executes tools, and emits
ordered `AgentEvent`s. It intentionally does not know provider JSON payload formats,
API keys, sessions, filesystems, UI, or persistence. Harness/provider adapters own those
concerns and pass typed events into this crate.

## Boundary

`oino-agent-loop` owns turn orchestration only: building provider-neutral stream
requests, accumulating provider stream events into assistant messages, validating and
executing tool calls, draining steering/follow-up queues between turns, honoring aborts,
and emitting ordered lifecycle events. Providers own wire formats and auth, tools own
side effects, sessions own persistence, and UI/harness crates own presentation,
resource assembly, and policy decisions.

## Public API map

- [`AgentLoopConfig`] wires one run: model, thinking level, system prompt, tool
  registry, [`StreamProvider`], [`EventSink`], hook callbacks (`finish_turn`,
  `prepare_request`), queue drains, turn limit, [`CacheRetention`], and
  [`AbortSignal`].
- [`run_agent_loop`], [`run_agent_loop_with_context`], and
  [`run_agent_loop_continue`] are the entry points for a new prompt, prompt plus
  existing context, or continuing already-reconstructed messages.
- [`StreamProvider`] receives a provider-neutral [`StreamRequest`] and returns or
  streams [`oino_types::AssistantStreamEvent`] values.
- [`Tool`], [`ToolDefinition`], [`ToolCall`], [`ToolResult`],
  [`ToolUpdateCallback`], and [`ToolExecutionMode`] define the model-visible tool
  protocol. Sequential tools force the whole batch to run sequentially; otherwise the
  loop can execute calls concurrently and restore result order.
- [`EventSink`] receives [`AgentEvent`] values for transcript/session/UI subscribers;
  [`VecEventSink`] and [`NoopEventSink`] are small test/default sinks.
  [`AgentEvent::MessageDelta`] carries incremental [`ContentDelta`] updates and
  [`MessageAccumulator`] reconstructs message content for consumers.
- [`BeforeToolCallResult`] and the transform/before/after/queue callback type aliases
  are harness seams for context shaping, policy, mutation, and live steering without
  adding app dependencies to the pure loop.
- [`FauxStream`] and [`FakeTool`] are deterministic fixtures for loop tests and
  downstream crate smoke tests.

## Contributor rules

Keep this crate provider-neutral and deterministic. New concepts should enter as typed
`oino-types` data or loop callbacks before any provider/app-specific behavior is added
elsewhere. Preserve event ordering around starts, updates, ends, tool results, queue
updates, and settlement; update fixture tests whenever that ordering changes. Do not add
filesystem, network, auth, TUI, session-path, or extension package logic here.
"#]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use futures::{stream::FuturesUnordered, StreamExt};
use oino_types::{
    transcript::{current_tools, normalize, Transcript},
    AssistantStreamEvent, ContentBlock, Message, Model, OinoId, ProviderMetadata, StopReason,
    ThinkingLevel, Usage,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    pin::Pin,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use thiserror::Error;
use tokio::sync::Mutex;
use uuid::Uuid;

pub type LoopResult<T> = Result<T, LoopError>;
pub type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

#[derive(Debug, Error)]
pub enum LoopError {
    #[error("stream error: {0}")]
    Stream(String),
    #[error("tool `{0}` not found")]
    ToolNotFound(String),
    #[error("tool error: {0}")]
    Tool(String),
    #[error("event sink error: {0}")]
    EventSink(String),
    #[error("aborted")]
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEvent {
    AgentStart {
        run_id: OinoId,
    },
    AgentEnd {
        run_id: OinoId,
        stop_reason: StopReason,
    },
    TurnStart {
        turn: u32,
    },
    TurnEnd {
        turn: u32,
        stop_reason: StopReason,
    },
    MessageStart {
        message_id: OinoId,
        role: String,
    },
    MessageDelta {
        message_id: OinoId,
        delta: ContentDelta,
    },
    MessageEnd {
        message: Message,
    },
    ToolExecutionStart {
        call: ToolCall,
    },
    ToolExecutionUpdate {
        call_id: OinoId,
        update: ToolUpdate,
    },
    ToolExecutionEnd {
        call_id: OinoId,
        result: ToolResult,
    },
    QueueUpdate {
        queue: String,
        pending: usize,
    },
    SavePoint {
        label: String,
    },
    Settled,
}

/// Incremental content update carried by [`AgentEvent::MessageDelta`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ContentDelta {
    /// Appended assistant text (also used for error message text).
    Text { text: String },
    /// Appended thinking text.
    Thinking { text: String },
    /// A completed tool-call block.
    ToolCall { block: ContentBlock },
}

/// Accumulates [`ContentDelta`]s into the content shape the loop's assistant
/// message ends with: one thinking block, one text block, then tool calls.
/// Consumers (TUI, harness) keep one per in-flight message instead of
/// hand-rolling reconstruction.
#[derive(Debug, Default)]
pub struct MessageAccumulator {
    text: String,
    thinking: String,
    tool_calls: Vec<ContentBlock>,
}

impl MessageAccumulator {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn apply(&mut self, delta: &ContentDelta) {
        match delta {
            ContentDelta::Text { text, .. } => self.text.push_str(text),
            ContentDelta::Thinking { text } => self.thinking.push_str(text),
            ContentDelta::ToolCall { block } => self.tool_calls.push(block.clone()),
        }
    }

    #[must_use]
    pub fn content(&self) -> Vec<ContentBlock> {
        let mut content = Vec::new();
        if !self.thinking.is_empty() {
            content.push(ContentBlock::Thinking {
                text: self.thinking.clone(),
                redacted: false,
                signature: None,
                provider_data: None,
            });
        }
        if !self.text.is_empty() {
            content.push(ContentBlock::Text {
                text: self.text.clone(),
                signature: None,
            });
        }
        content.extend(self.tool_calls.iter().cloned());
        content
    }
}

#[async_trait]
pub trait EventSink: Send + Sync {
    async fn emit(&self, event: AgentEvent) -> LoopResult<()>;
}

#[derive(Debug, Default)]
pub struct NoopEventSink;

#[async_trait]
impl EventSink for NoopEventSink {
    async fn emit(&self, _event: AgentEvent) -> LoopResult<()> {
        Ok(())
    }
}

#[derive(Debug, Default, Clone)]
pub struct VecEventSink {
    events: Arc<Mutex<Vec<AgentEvent>>>,
}

impl VecEventSink {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
    pub async fn events(&self) -> Vec<AgentEvent> {
        self.events.lock().await.clone()
    }
}

#[async_trait]
impl EventSink for VecEventSink {
    async fn emit(&self, event: AgentEvent) -> LoopResult<()> {
        self.events.lock().await.push(event);
        Ok(())
    }
}

/// Model-visible tool declaration. Defined in `oino-types` so `Message::System`
/// can carry declarations inside the transcript; re-exported here for
/// compatibility with existing tool implementations.
pub use oino_types::ToolDeclaration as ToolDefinition;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ToolExecutionMode {
    Parallel,
    Sequential,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolCall {
    pub id: OinoId,
    pub name: String,
    pub arguments: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolUpdate {
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ToolResult {
    pub call_id: OinoId,
    pub tool_name: String,
    pub content: Vec<ContentBlock>,
    pub is_error: bool,
    pub terminate: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<Value>,
    /// Tool names the execution exposed for subsequent turns (extension tools
    /// may declare additional tools at run time).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub added_tool_names: Vec<String>,
}

impl ToolResult {
    #[must_use]
    pub fn text(call: &ToolCall, text: impl Into<String>) -> Self {
        Self {
            call_id: call.id,
            tool_name: call.name.clone(),
            content: vec![ContentBlock::Text {
                text: text.into(),
                signature: None,
            }],
            is_error: false,
            terminate: false,
            details: None,
            added_tool_names: Vec::new(),
        }
    }
    #[must_use]
    pub fn error(call: &ToolCall, text: impl Into<String>) -> Self {
        Self {
            call_id: call.id,
            tool_name: call.name.clone(),
            content: vec![ContentBlock::Text {
                text: text.into(),
                signature: None,
            }],
            is_error: true,
            terminate: false,
            details: None,
            added_tool_names: Vec::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AbortSignal {
    flag: Arc<AtomicBool>,
}
impl AbortSignal {
    #[must_use]
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn abort(&self) {
        self.flag.store(true, Ordering::SeqCst);
    }
    #[must_use]
    pub fn is_aborted(&self) -> bool {
        self.flag.load(Ordering::SeqCst)
    }
}
impl Default for AbortSignal {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone)]
pub struct ToolUpdateCallback {
    sink: Arc<dyn EventSink>,
    call_id: OinoId,
}
impl ToolUpdateCallback {
    #[must_use]
    pub fn new(sink: Arc<dyn EventSink>, call_id: OinoId) -> Self {
        Self { sink, call_id }
    }
    pub async fn update(&self, update: ToolUpdate) -> LoopResult<()> {
        self.sink
            .emit(AgentEvent::ToolExecutionUpdate {
                call_id: self.call_id,
                update,
            })
            .await
    }
}

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    fn execution_mode(&self) -> ToolExecutionMode {
        ToolExecutionMode::Parallel
    }
    async fn prepare_arguments(&self, arguments: Value) -> LoopResult<Value> {
        Ok(arguments)
    }
    async fn execute(
        &self,
        call: ToolCall,
        updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult>;
}

/// Provider-neutral stream request. The transcript carries the system prompt
/// and tool declarations; `session_id` and `cache_retention` are cache-affinity
/// hints adapters may use.
#[derive(Debug, Clone)]
pub struct StreamRequest {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub transcript: Transcript,
    pub session_id: Option<String>,
    pub cache_retention: CacheRetention,
    /// Provider-side output cap override (used by cache warming: `Some(1)`).
    /// Providers that cannot express it may ignore it.
    pub max_output_tokens: Option<u32>,
}

/// How long provider-side prompt caches should be retained for this request.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CacheRetention {
    None,
    #[default]
    Short,
    Long,
}

pub type StreamEventSink =
    Arc<dyn Fn(AssistantStreamEvent) -> BoxFuture<'static, LoopResult<()>> + Send + Sync>;

#[async_trait]
pub trait StreamProvider: Send + Sync {
    async fn stream(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
    ) -> LoopResult<Vec<AssistantStreamEvent>>;

    async fn stream_events(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
        sink: StreamEventSink,
    ) -> LoopResult<()> {
        for event in self.stream(request, signal).await? {
            sink(event).await?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub enum BeforeToolCallResult {
    Allow(ToolCall),
    Block { reason: String, terminate: bool },
}

/// Context passed to [`FinishTurn`]. Owned clones keep the callback `Arc`-safe.
#[derive(Debug, Clone)]
pub struct TurnContext {
    /// The assistant message that ended the turn.
    pub message: Message,
    /// Tool result messages emitted for this turn.
    pub tool_results: Vec<Message>,
    /// Current transcript after the turn's messages were appended.
    pub transcript: Transcript,
}

/// Decision returned by a [`FinishTurn`] hook.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnDecision {
    /// Guarantee exactly one more provider request, satisfied by pending tool
    /// results, steering, or follow-up messages when present.
    Continue,
    /// Stop the run without draining queues.
    End,
}

/// What [`FinishTurn`] may return: an optional decision plus extra messages
/// appended to the transcript (with `MessageStart`/`MessageEnd` events) so they
/// are visible to the very next request. Both fields are ignored on hard exits.
#[derive(Debug, Clone, Default)]
pub struct TurnUpdate {
    pub decision: Option<TurnDecision>,
    pub messages: Vec<Message>,
}

pub type FinishTurn =
    Arc<dyn Fn(TurnContext) -> BoxFuture<'static, LoopResult<Option<TurnUpdate>>> + Send + Sync>;

/// Runtime state available immediately before a provider request.
#[derive(Debug, Clone)]
pub struct PrepareRequestContext {
    pub transcript: Transcript,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
}

/// Replacement state for the provider request being prepared. Fields left
/// `None` keep their current values; updates persist for the rest of the run.
#[derive(Debug, Clone, Default)]
pub struct RequestUpdate {
    pub transcript: Option<Transcript>,
    pub model: Option<Model>,
    pub thinking_level: Option<ThinkingLevel>,
}

pub type PrepareRequest = Arc<
    dyn Fn(PrepareRequestContext) -> BoxFuture<'static, LoopResult<Option<RequestUpdate>>>
        + Send
        + Sync,
>;

pub type TransformContext =
    Arc<dyn Fn(Vec<Message>) -> BoxFuture<'static, LoopResult<Vec<Message>>> + Send + Sync>;
pub type BeforeToolCall =
    Arc<dyn Fn(ToolCall) -> BoxFuture<'static, LoopResult<BeforeToolCallResult>> + Send + Sync>;
pub type AfterToolCall =
    Arc<dyn Fn(ToolResult) -> BoxFuture<'static, LoopResult<ToolResult>> + Send + Sync>;
pub type DrainMessages =
    Arc<dyn Fn() -> BoxFuture<'static, LoopResult<Vec<Message>>> + Send + Sync>;

#[derive(Clone)]
pub struct AgentLoopConfig {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub system_prompt: Option<String>,
    pub tools: BTreeMap<String, Arc<dyn Tool>>,
    pub stream: Arc<dyn StreamProvider>,
    pub event_sink: Arc<dyn EventSink>,
    pub transform_context: Option<TransformContext>,
    pub before_tool_call: Option<BeforeToolCall>,
    pub after_tool_call: Option<AfterToolCall>,
    pub tool_execution: ToolExecutionMode,
    pub finish_turn: Option<FinishTurn>,
    pub prepare_request: Option<PrepareRequest>,
    pub session_id: Option<String>,
    pub cache_retention: CacheRetention,
    pub get_steering_messages: Option<DrainMessages>,
    pub get_follow_up_messages: Option<DrainMessages>,
    pub max_turns: u32,
    pub abort_signal: AbortSignal,
}

impl AgentLoopConfig {
    #[must_use]
    pub fn new(model: Model, stream: Arc<dyn StreamProvider>) -> Self {
        Self {
            model,
            thinking_level: ThinkingLevel::default(),
            system_prompt: None,
            tools: BTreeMap::new(),
            stream,
            event_sink: Arc::new(NoopEventSink),
            transform_context: None,
            before_tool_call: None,
            after_tool_call: None,
            tool_execution: ToolExecutionMode::Parallel,
            finish_turn: None,
            prepare_request: None,
            session_id: None,
            cache_retention: CacheRetention::default(),
            get_steering_messages: None,
            get_follow_up_messages: None,
            max_turns: 16,
            abort_signal: AbortSignal::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct AgentLoopOutput {
    pub messages: Vec<Message>,
    pub stop_reason: StopReason,
}

pub async fn run_agent_loop(
    prompt: Message,
    config: AgentLoopConfig,
) -> LoopResult<AgentLoopOutput> {
    run_agent_loop_with_context(Vec::new(), prompt, config).await
}

pub async fn run_agent_loop_with_context(
    mut messages: Vec<Message>,
    prompt: Message,
    config: AgentLoopConfig,
) -> LoopResult<AgentLoopOutput> {
    let run_id = Uuid::new_v4();
    config
        .event_sink
        .emit(AgentEvent::AgentStart { run_id })
        .await?;
    config
        .event_sink
        .emit(AgentEvent::MessageStart {
            message_id: prompt.id(),
            role: "user".into(),
        })
        .await?;
    config
        .event_sink
        .emit(AgentEvent::MessageEnd {
            message: prompt.clone(),
        })
        .await?;
    messages.push(prompt);
    run_agent_loop_continue_inner(messages, config, run_id).await
}

pub async fn run_agent_loop_continue(
    messages: Vec<Message>,
    config: AgentLoopConfig,
) -> LoopResult<AgentLoopOutput> {
    let run_id = Uuid::new_v4();
    config
        .event_sink
        .emit(AgentEvent::AgentStart { run_id })
        .await?;
    run_agent_loop_continue_inner(messages, config, run_id).await
}

async fn run_agent_loop_continue_inner(
    mut messages: Vec<Message>,
    config: AgentLoopConfig,
    run_id: OinoId,
) -> LoopResult<AgentLoopOutput> {
    let mut final_stop = StopReason::Unknown;
    let mut turn: u32 = 0;
    let mut model = config.model.clone();
    let mut thinking_level = config.thinking_level;
    let mut explicit_continuation = false;
    // Steering may have been queued while the run was being set up.
    let mut pending = drain_steering(&config).await?;

    'outer: loop {
        let mut has_more_tool_calls = true;
        while has_more_tool_calls || !pending.is_empty() {
            if turn >= config.max_turns || config.abort_signal.is_aborted() {
                if config.abort_signal.is_aborted() {
                    final_stop = StopReason::Aborted;
                }
                break 'outer;
            }
            config
                .event_sink
                .emit(AgentEvent::TurnStart { turn })
                .await?;
            turn += 1;

            // Pending steering/follow-up messages are appended before the request.
            append_drained_messages(&mut messages, &config, std::mem::take(&mut pending)).await?;

            let context = if let Some(transform) = &config.transform_context {
                transform(messages.clone()).await?
            } else {
                messages.clone()
            };
            // Bridge: an existing System message means the transcript owns the
            // prompt and tools; otherwise fold config prompt/registry tools into
            // a request-only leading System message.
            let mut transcript = if context
                .iter()
                .any(|message| matches!(message, Message::System { .. }))
            {
                Transcript::new(context)
            } else {
                let declarations = config
                    .tools
                    .values()
                    .map(|tool| tool.definition())
                    .collect();
                normalize(config.system_prompt.clone(), declarations, context)
            };
            if let Some(prepare) = &config.prepare_request {
                let update = prepare(PrepareRequestContext {
                    transcript: transcript.clone(),
                    model: model.clone(),
                    thinking_level,
                })
                .await?;
                if let Some(update) = update {
                    if let Some(next_transcript) = update.transcript {
                        messages = next_transcript.clone().into_messages();
                        transcript = next_transcript;
                    }
                    if let Some(next_model) = update.model {
                        model = next_model;
                    }
                    if let Some(next_level) = update.thinking_level {
                        thinking_level = next_level;
                    }
                }
            }
            let available_tools: BTreeSet<String> = current_tools(transcript.messages())
                .into_iter()
                .map(|tool| tool.name)
                .collect();
            let request = StreamRequest {
                model: model.clone(),
                thinking_level,
                transcript,
                session_id: config.session_id.clone(),
                cache_retention: config.cache_retention,
                max_output_tokens: None,
            };
            let assistant = consume_stream(&config, request).await?;
            let stop_reason = assistant_stop(&assistant);
            final_stop = stop_reason.clone();
            messages.push(assistant.clone());

            let hard_exit = config.abort_signal.is_aborted()
                || matches!(
                    stop_reason,
                    StopReason::Aborted | StopReason::Error | StopReason::Length
                );

            let mut tool_result_messages: Vec<Message> = Vec::new();
            let mut all_terminate = false;
            if !hard_exit {
                let tool_calls = assistant_tool_calls(&assistant);
                if tool_calls.is_empty() {
                    has_more_tool_calls = false;
                } else {
                    let results = execute_tool_batch(tool_calls, &config, &available_tools).await?;
                    all_terminate = results.iter().all(|result| result.terminate);
                    for result in results {
                        let message = Message::ToolResult {
                            id: Uuid::new_v4(),
                            tool_call_id: result.call_id,
                            tool_name: result.tool_name.clone(),
                            content: result.content.clone(),
                            is_error: result.is_error,
                            terminate: result.terminate,
                            details: result.details.clone(),
                            usage: None,
                            added_tool_names: result.added_tool_names.clone(),
                        };
                        config
                            .event_sink
                            .emit(AgentEvent::MessageStart {
                                message_id: message.id(),
                                role: "tool_result".into(),
                            })
                            .await?;
                        config
                            .event_sink
                            .emit(AgentEvent::MessageEnd {
                                message: message.clone(),
                            })
                            .await?;
                        tool_result_messages.push(message.clone());
                        messages.push(message);
                    }
                    has_more_tool_calls = !all_terminate;
                }
            }

            // finish_turn runs after the assistant message and all tool results
            // are finalized, before TurnEnd. On hard exits its decision is ignored.
            let update = if let Some(hook) = &config.finish_turn {
                hook(TurnContext {
                    message: assistant.clone(),
                    tool_results: tool_result_messages,
                    transcript: Transcript::new(messages.clone()),
                })
                .await?
            } else {
                None
            };
            config
                .event_sink
                .emit(AgentEvent::TurnEnd {
                    turn: turn - 1,
                    stop_reason: stop_reason.clone(),
                })
                .await?;

            if hard_exit {
                break 'outer;
            }
            // Entries produced by the hook join the transcript immediately so
            // they are visible to the very next request.
            if let Some(update) = &update {
                for message in &update.messages {
                    config
                        .event_sink
                        .emit(AgentEvent::MessageStart {
                            message_id: message.id(),
                            role: message_role(message).into(),
                        })
                        .await?;
                    config
                        .event_sink
                        .emit(AgentEvent::MessageEnd {
                            message: message.clone(),
                        })
                        .await?;
                    messages.push(message.clone());
                }
            }
            let decision = update.and_then(|update| update.decision);
            if matches!(decision, Some(TurnDecision::End)) {
                break 'outer;
            }
            if all_terminate {
                final_stop = StopReason::ToolUse;
                break 'outer;
            }
            explicit_continuation = matches!(decision, Some(TurnDecision::Continue));
            pending = drain_steering(&config).await?;
            if has_more_tool_calls || !pending.is_empty() {
                explicit_continuation = false;
            }
        }

        // The inner loop is done making requests for this turn. Follow-up
        // messages get the next chance to satisfy a pending continuation.
        let follow_up = drain_follow_up(&config).await?;
        if !follow_up.is_empty() {
            explicit_continuation = false;
            pending = follow_up;
            continue;
        }
        if explicit_continuation {
            // `Continue` guarantees exactly one more request; with no natural
            // request pending, run one context-only turn.
            explicit_continuation = false;
            continue;
        }
        break;
    }

    config
        .event_sink
        .emit(AgentEvent::AgentEnd {
            run_id,
            stop_reason: final_stop.clone(),
        })
        .await?;
    Ok(AgentLoopOutput {
        messages,
        stop_reason: final_stop,
    })
}

#[derive(Debug, Default)]
struct PartialToolCall {
    name: Option<String>,
    arguments: String,
}

fn finalize_partial_tool_calls(
    content: &mut Vec<ContentBlock>,
    partial_order: &[OinoId],
    partial_tool_calls: &BTreeMap<OinoId, PartialToolCall>,
) {
    let finalized: BTreeSet<OinoId> = content
        .iter()
        .filter_map(|block| match block {
            ContentBlock::ToolCall { id, .. } => Some(*id),
            _ => None,
        })
        .collect();
    for id in partial_order {
        if finalized.contains(id) {
            continue;
        }
        let Some(partial) = partial_tool_calls.get(id) else {
            continue;
        };
        let Some(name) = partial.name.clone() else {
            continue;
        };
        let arguments = if partial.arguments.trim().is_empty() {
            Value::Object(Default::default())
        } else {
            match serde_json::from_str(&partial.arguments) {
                Ok(value) => value,
                Err(_) => Value::String(partial.arguments.clone()),
            }
        };
        content.push(ContentBlock::ToolCall {
            id: *id,
            name,
            arguments,
            native_id: None,
        });
    }
}

async fn consume_stream(config: &AgentLoopConfig, request: StreamRequest) -> LoopResult<Message> {
    let id = Uuid::new_v4();
    let accumulator = Arc::new(Mutex::new(StreamAccumulator::default()));
    config
        .event_sink
        .emit(AgentEvent::MessageStart {
            message_id: id,
            role: "assistant".into(),
        })
        .await?;

    let event_sink = Arc::clone(&config.event_sink);
    let abort_signal = config.abort_signal.clone();
    let handler_accumulator = Arc::clone(&accumulator);
    let handler = Arc::new(move |event| {
        let event_sink = Arc::clone(&event_sink);
        let abort_signal = abort_signal.clone();
        let accumulator = Arc::clone(&handler_accumulator);
        let fut: BoxFuture<'static, LoopResult<()>> = Box::pin(async move {
            let mut accumulator = accumulator.lock().await;
            accumulator
                .apply_event(event, id, &event_sink, &abort_signal)
                .await
        });
        fut
    });

    if let Err(err) = config
        .stream
        .stream_events(request, config.abort_signal.clone(), handler)
        .await
    {
        let mut accumulator = accumulator.lock().await;
        accumulator
            .apply_event(
                AssistantStreamEvent::Error {
                    message: err.to_string(),
                },
                id,
                &config.event_sink,
                &config.abort_signal,
            )
            .await?;
    }

    let mut accumulator = accumulator.lock().await;
    if config.abort_signal.is_aborted() {
        accumulator.stop_reason = StopReason::Aborted;
    }
    let message = accumulator.finish(id);
    config
        .event_sink
        .emit(AgentEvent::MessageEnd {
            message: message.clone(),
        })
        .await?;
    Ok(message)
}

#[derive(Debug)]
struct StreamAccumulator {
    content: Vec<ContentBlock>,
    text: String,
    thinking: String,
    thinking_signature: Option<String>,
    response_model: Option<String>,
    usage: Option<Usage>,
    provider: Option<ProviderMetadata>,
    stop_reason: StopReason,
    partial_tool_calls: BTreeMap<OinoId, PartialToolCall>,
    partial_order: Vec<OinoId>,
}

impl Default for StreamAccumulator {
    fn default() -> Self {
        Self {
            content: Vec::new(),
            text: String::new(),
            thinking: String::new(),
            thinking_signature: None,
            response_model: None,
            usage: None,
            provider: None,
            stop_reason: StopReason::Unknown,
            partial_tool_calls: BTreeMap::new(),
            partial_order: Vec::new(),
        }
    }
}

impl StreamAccumulator {
    async fn apply_event(
        &mut self,
        event: AssistantStreamEvent,
        message_id: OinoId,
        event_sink: &Arc<dyn EventSink>,
        abort_signal: &AbortSignal,
    ) -> LoopResult<()> {
        if abort_signal.is_aborted() {
            self.stop_reason = StopReason::Aborted;
            return Ok(());
        }
        match event {
            AssistantStreamEvent::Start { response_model, .. } => {
                self.response_model = response_model;
            }
            AssistantStreamEvent::TextDelta { delta } => {
                self.text.push_str(&delta);
                self.emit_delta(message_id, ContentDelta::Text { text: delta }, event_sink)
                    .await?;
            }
            AssistantStreamEvent::ThinkingDelta { delta } => {
                self.thinking.push_str(&delta);
                self.emit_delta(
                    message_id,
                    ContentDelta::Thinking { text: delta },
                    event_sink,
                )
                .await?;
            }
            AssistantStreamEvent::ThinkingSignature { signature } => {
                self.thinking_signature = Some(signature);
            }
            AssistantStreamEvent::ToolCallDelta {
                id,
                name,
                native_id: _,
                arguments_delta,
            } => {
                if !self.partial_tool_calls.contains_key(&id) {
                    self.partial_order.push(id);
                }
                let partial = self.partial_tool_calls.entry(id).or_default();
                if let Some(name) = name {
                    partial.name = Some(name);
                }
                partial.arguments.push_str(&arguments_delta);
            }
            AssistantStreamEvent::ToolCallDone {
                id,
                name,
                native_id,
                arguments,
            } => {
                if !self.partial_tool_calls.contains_key(&id) {
                    self.partial_order.push(id);
                }
                self.partial_tool_calls.insert(
                    id,
                    PartialToolCall {
                        name: Some(name.clone()),
                        arguments: serde_json::to_string(&arguments)
                            .unwrap_or_else(|_| arguments.to_string()),
                    },
                );
                let block = ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    native_id,
                };
                self.content.push(block.clone());
                self.emit_delta(message_id, ContentDelta::ToolCall { block }, event_sink)
                    .await?;
            }
            AssistantStreamEvent::Usage { usage: reported } => self.usage = Some(reported),
            AssistantStreamEvent::Done {
                stop_reason: reason,
                provider: meta,
            } => {
                self.stop_reason = reason;
                self.provider = meta;
            }
            AssistantStreamEvent::Error { message } => {
                self.text.push_str(&message);
                self.stop_reason = StopReason::Error;
                self.emit_delta(message_id, ContentDelta::Text { text: message }, event_sink)
                    .await?;
            }
            AssistantStreamEvent::Aborted => {
                self.stop_reason = StopReason::Aborted;
            }
        }
        Ok(())
    }

    async fn emit_delta(
        &self,
        message_id: OinoId,
        delta: ContentDelta,
        event_sink: &Arc<dyn EventSink>,
    ) -> LoopResult<()> {
        event_sink
            .emit(AgentEvent::MessageDelta { message_id, delta })
            .await
    }

    fn finish(&mut self, id: OinoId) -> Message {
        finalize_partial_tool_calls(
            &mut self.content,
            &self.partial_order,
            &self.partial_tool_calls,
        );
        if !self.text.is_empty() {
            self.content.insert(
                0,
                ContentBlock::Text {
                    text: self.text.clone(),
                    signature: None,
                },
            );
        }
        if !self.thinking.is_empty() {
            self.content.insert(
                0,
                ContentBlock::Thinking {
                    text: self.thinking.clone(),
                    redacted: false,
                    signature: self.thinking_signature.take(),
                    provider_data: None,
                },
            );
        }
        if self
            .content
            .iter()
            .any(|block| matches!(block, ContentBlock::ToolCall { .. }))
            && matches!(self.stop_reason, StopReason::Unknown | StopReason::EndTurn)
        {
            self.stop_reason = StopReason::ToolUse;
        }
        let error_message = if matches!(self.stop_reason, StopReason::Error | StopReason::Aborted) {
            Some(self.text.clone()).filter(|text| !text.is_empty())
        } else {
            None
        };
        Message::Assistant {
            id,
            content: std::mem::take(&mut self.content),
            stop_reason: Some(self.stop_reason.clone()),
            usage: self.usage.take(),
            provider: self.provider.take(),
            response_model: self.response_model.take(),
            error_message,
            provider_thinking_level: None,
        }
    }
}

fn assistant_stop(message: &Message) -> StopReason {
    if let Message::Assistant { stop_reason, .. } = message {
        stop_reason.clone().unwrap_or(StopReason::Unknown)
    } else {
        StopReason::Unknown
    }
}

fn assistant_tool_calls(message: &Message) -> Vec<ToolCall> {
    if let Message::Assistant { content, .. } = message {
        content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall {
                    id,
                    name,
                    arguments,
                    ..
                } => Some(ToolCall {
                    id: *id,
                    name: name.clone(),
                    arguments: arguments.clone(),
                }),
                _ => None,
            })
            .collect()
    } else {
        Vec::new()
    }
}

fn validate_arguments(schema: &Value, arguments: &Value) -> Result<(), String> {
    let Some(schema_object) = schema.as_object() else {
        return Ok(());
    };
    if let Some(expected_type) = schema_object.get("type").and_then(Value::as_str) {
        let matches_type = match expected_type {
            "object" => arguments.is_object(),
            "array" => arguments.is_array(),
            "string" => arguments.is_string(),
            "number" => arguments.is_number(),
            "integer" => arguments.as_i64().is_some() || arguments.as_u64().is_some(),
            "boolean" => arguments.is_boolean(),
            "null" => arguments.is_null(),
            _ => true,
        };
        if !matches_type {
            return Err(format!(
                "arguments did not match JSON schema type `{expected_type}`"
            ));
        }
    }
    if let Some(required) = schema_object.get("required").and_then(Value::as_array) {
        let Some(argument_object) = arguments.as_object() else {
            return Err("arguments must be an object to satisfy required properties".into());
        };
        for property in required.iter().filter_map(Value::as_str) {
            if !argument_object.contains_key(property) {
                return Err(format!("missing required argument `{property}`"));
            }
        }
    }
    Ok(())
}

async fn execute_tool_batch(
    calls: Vec<ToolCall>,
    config: &AgentLoopConfig,
    available_tools: &BTreeSet<String>,
) -> LoopResult<Vec<ToolResult>> {
    let mut prepared = Vec::with_capacity(calls.len());
    let mut sequential = matches!(config.tool_execution, ToolExecutionMode::Sequential);
    for call in calls {
        if !available_tools.contains(&call.name) {
            prepared.push((
                call.clone(),
                None,
                ToolResult::error(&call, format!("tool `{}` is not available", call.name)),
            ));
            continue;
        }
        let Some(tool) = config.tools.get(&call.name) else {
            prepared.push((
                call.clone(),
                None,
                ToolResult::error(&call, format!("tool `{}` is not available", call.name)),
            ));
            continue;
        };
        if matches!(tool.execution_mode(), ToolExecutionMode::Sequential) {
            sequential = true;
        }
        let prepared_args = match tool.prepare_arguments(call.arguments.clone()).await {
            Ok(args) => args,
            Err(err) => {
                prepared.push((
                    call.clone(),
                    None,
                    ToolResult::error(&call, err.to_string()),
                ));
                continue;
            }
        };
        if let Err(err) = validate_arguments(&tool.definition().input_schema, &prepared_args) {
            prepared.push((call.clone(), None, ToolResult::error(&call, err)));
            continue;
        }
        let mut prepared_call = ToolCall {
            arguments: prepared_args,
            ..call
        };
        if let Some(before) = &config.before_tool_call {
            match before(prepared_call.clone()).await? {
                BeforeToolCallResult::Allow(rewritten) => prepared_call = rewritten,
                BeforeToolCallResult::Block { reason, terminate } => {
                    let mut blocked = ToolResult::error(&prepared_call, reason);
                    blocked.terminate = terminate;
                    prepared.push((prepared_call, None, blocked));
                    continue;
                }
            }
        }
        prepared.push((
            prepared_call,
            Some(Arc::clone(tool)),
            ToolResult::error(
                &ToolCall {
                    id: Uuid::nil(),
                    name: String::new(),
                    arguments: Value::Null,
                },
                "placeholder",
            ),
        ));
    }

    if sequential {
        let mut results = Vec::with_capacity(prepared.len());
        for (call, tool, fallback) in prepared {
            results.push(execute_one(call, tool, fallback, config).await?);
        }
        Ok(results)
    } else {
        let mut futures = FuturesUnordered::new();
        for (idx, (call, tool, fallback)) in prepared.into_iter().enumerate() {
            futures.push(async move { (idx, call, tool, fallback) });
        }
        let mut task_futures = FuturesUnordered::new();
        while let Some((idx, call, tool, fallback)) = futures.next().await {
            let cfg = config.clone();
            task_futures.push(async move { (idx, execute_one(call, tool, fallback, &cfg).await) });
        }
        let mut by_index: BTreeMap<usize, ToolResult> = BTreeMap::new();
        while let Some((idx, result)) = task_futures.next().await {
            by_index.insert(idx, result?);
        }
        Ok(by_index.into_values().collect())
    }
}

async fn execute_one(
    call: ToolCall,
    tool: Option<Arc<dyn Tool>>,
    fallback: ToolResult,
    config: &AgentLoopConfig,
) -> LoopResult<ToolResult> {
    if call.id != Uuid::nil() {
        config
            .event_sink
            .emit(AgentEvent::ToolExecutionStart { call: call.clone() })
            .await?;
    }
    let mut result = if let Some(tool) = tool {
        let updates = ToolUpdateCallback::new(Arc::clone(&config.event_sink), call.id);
        match tool
            .execute(call.clone(), updates, config.abort_signal.clone())
            .await
        {
            Ok(result) => result,
            Err(err) => ToolResult::error(&call, err.to_string()),
        }
    } else {
        fallback
    };
    if let Some(after) = &config.after_tool_call {
        result = after(result).await?;
    }
    if call.id != Uuid::nil() {
        config
            .event_sink
            .emit(AgentEvent::ToolExecutionEnd {
                call_id: call.id,
                result: result.clone(),
            })
            .await?;
    }
    Ok(result)
}

async fn drain_steering(config: &AgentLoopConfig) -> LoopResult<Vec<Message>> {
    if let Some(drain) = &config.get_steering_messages {
        return drain().await;
    }
    Ok(Vec::new())
}

async fn drain_follow_up(config: &AgentLoopConfig) -> LoopResult<Vec<Message>> {
    if let Some(drain) = &config.get_follow_up_messages {
        return drain().await;
    }
    Ok(Vec::new())
}

async fn append_drained_messages(
    messages: &mut Vec<Message>,
    config: &AgentLoopConfig,
    drained: Vec<Message>,
) -> LoopResult<usize> {
    let count = drained.len();
    for message in drained {
        config
            .event_sink
            .emit(AgentEvent::MessageStart {
                message_id: message.id(),
                role: message_role(&message).into(),
            })
            .await?;
        config
            .event_sink
            .emit(AgentEvent::MessageEnd {
                message: message.clone(),
            })
            .await?;
        messages.push(message);
    }
    Ok(count)
}

fn message_role(message: &Message) -> &'static str {
    match message {
        Message::System { .. } => "system",
        Message::User { .. } => "user",
        Message::Assistant { .. } => "assistant",
        Message::ToolResult { .. } => "tool_result",
        Message::Custom { .. } => "custom",
        Message::CompactionSummary { .. } => "compaction_summary",
        Message::BranchSummary { .. } => "branch_summary",
    }
}

#[derive(Clone)]
pub struct FauxStream {
    events: Arc<Mutex<Vec<Vec<AssistantStreamEvent>>>>,
}
impl FauxStream {
    #[must_use]
    pub fn once(events: Vec<AssistantStreamEvent>) -> Self {
        Self {
            events: Arc::new(Mutex::new(vec![events])),
        }
    }
    #[must_use]
    pub fn turns(turns: Vec<Vec<AssistantStreamEvent>>) -> Self {
        Self {
            events: Arc::new(Mutex::new(turns)),
        }
    }
}

#[async_trait]
impl StreamProvider for FauxStream {
    async fn stream(
        &self,
        _request: StreamRequest,
        signal: AbortSignal,
    ) -> LoopResult<Vec<AssistantStreamEvent>> {
        if signal.is_aborted() {
            return Ok(vec![AssistantStreamEvent::Aborted]);
        }
        let mut locked = self.events.lock().await;
        if locked.is_empty() {
            return Ok(vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }]);
        }
        Ok(locked.remove(0))
    }
}

pub struct FakeTool {
    pub definition: ToolDefinition,
    pub mode: ToolExecutionMode,
    pub result: String,
    pub fail: bool,
    pub terminate: bool,
    pub delay_ms: u64,
}

impl FakeTool {
    #[must_use]
    pub fn new(name: impl Into<String>, result: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            definition: ToolDefinition {
                name,
                description: "fake tool".into(),
                input_schema: serde_json::json!({"type":"object"}),
            },
            mode: ToolExecutionMode::Parallel,
            result: result.into(),
            fail: false,
            terminate: false,
            delay_ms: 0,
        }
    }
}

#[async_trait]
impl Tool for FakeTool {
    fn definition(&self) -> ToolDefinition {
        self.definition.clone()
    }
    fn execution_mode(&self) -> ToolExecutionMode {
        self.mode
    }
    async fn execute(
        &self,
        call: ToolCall,
        updates: ToolUpdateCallback,
        signal: AbortSignal,
    ) -> LoopResult<ToolResult> {
        if self.delay_ms > 0 {
            tokio::time::sleep(std::time::Duration::from_millis(self.delay_ms)).await;
        }
        if signal.is_aborted() {
            return Ok(ToolResult {
                call_id: call.id,
                tool_name: call.name,
                content: vec![ContentBlock::Text {
                    text: "aborted".into(),
                    signature: None,
                }],
                is_error: true,
                terminate: true,
                details: None,
                added_tool_names: Vec::new(),
            });
        }
        updates
            .update(ToolUpdate {
                message: "fake update".into(),
                details: None,
            })
            .await?;
        if self.fail {
            return Err(LoopError::Tool(self.result.clone()));
        }
        let mut result = ToolResult::text(&call, self.result.clone());
        result.terminate = self.terminate;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> Model {
        Model::new("test", "faux")
    }

    #[tokio::test]
    async fn text_stream_event_order() {
        let sink = VecEventSink::new();
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::TextDelta {
                delta: "hel".into(),
            },
            AssistantStreamEvent::TextDelta { delta: "lo".into() },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            },
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        assert!(output.is_ok());
        let events = sink.events().await;
        assert!(matches!(
            events.first(),
            Some(AgentEvent::AgentStart { .. })
        ));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::MessageDelta { .. })));
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::AgentEnd { .. })));
    }

    #[tokio::test]
    async fn thinking_deltas_stream_as_message_updates() {
        let sink = VecEventSink::new();
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::ThinkingDelta {
                delta: "thinking".into(),
            },
            AssistantStreamEvent::TextDelta {
                delta: "answer".into(),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            },
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        let output = match run_agent_loop(Message::user_text("hi"), config).await {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert!(matches!(
            output.messages.last(),
            Some(Message::Assistant { content, .. })
                if content.iter().any(|block| matches!(
                    block,
                    ContentBlock::Thinking { text, .. } if text == "thinking"
                ))
        ));
        let events = sink.events().await;
        let mut accumulator = MessageAccumulator::new();
        for event in &events {
            if let AgentEvent::MessageDelta { delta, .. } = event {
                accumulator.apply(delta);
            }
        }
        assert!(accumulator.content().iter().any(|block| matches!(
            block,
            ContentBlock::Thinking { text, .. } if text == "thinking"
        )));
    }

    #[tokio::test]
    async fn stream_error_becomes_assistant_error() {
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Error {
            message: "boom".into(),
        }]));
        let config = AgentLoopConfig::new(model(), stream);
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert_eq!(output.stop_reason, StopReason::Error);
    }

    #[tokio::test]
    async fn tool_result_messages_preserve_source_order() {
        let call_a = Uuid::new_v4();
        let call_b = Uuid::new_v4();
        let stream = Arc::new(FauxStream::turns(vec![
            vec![
                AssistantStreamEvent::ToolCallDone {
                    id: call_a,
                    name: "a".into(),
                    native_id: None,
                    arguments: serde_json::json!({}),
                },
                AssistantStreamEvent::ToolCallDone {
                    id: call_b,
                    name: "b".into(),
                    native_id: None,
                    arguments: serde_json::json!({}),
                },
                AssistantStreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    provider: None,
                },
            ],
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        let mut a = FakeTool::new("a", "A");
        a.delay_ms = 20;
        let b = FakeTool::new("b", "B");
        config.tools.insert("a".into(), Arc::new(a));
        config.tools.insert("b".into(), Arc::new(b));
        let output = run_agent_loop(Message::user_text("tools"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        let tool_names: Vec<String> = output
            .messages
            .into_iter()
            .filter_map(|msg| match msg {
                Message::ToolResult { tool_name, .. } => Some(tool_name),
                _ => None,
            })
            .collect();
        assert_eq!(tool_names, vec!["a".to_string(), "b".to_string()]);
    }

    #[tokio::test]
    async fn run_id_matches_between_agent_start_and_end() {
        let sink = VecEventSink::new();
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        assert!(output.is_ok());
        let events = sink.events().await;
        let start_id = events.iter().find_map(|event| match event {
            AgentEvent::AgentStart { run_id } => Some(*run_id),
            _ => None,
        });
        let end_id = events.iter().find_map(|event| match event {
            AgentEvent::AgentEnd { run_id, .. } => Some(*run_id),
            _ => None,
        });
        assert_eq!(start_id, end_id);
    }

    #[tokio::test]
    async fn length_stop_is_reported() {
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::Length,
            provider: None,
        }]));
        let config = AgentLoopConfig::new(model(), stream);
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert_eq!(output.stop_reason, StopReason::Length);
    }

    #[tokio::test]
    async fn end_turn_drains_steering_and_continues() {
        let sink = VecEventSink::new();
        let queued = Arc::new(Mutex::new(Some(Message::user_text("steer now"))));
        let stream = Arc::new(FauxStream::turns(vec![
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
            vec![
                AssistantStreamEvent::TextDelta {
                    delta: "after steer".into(),
                },
                AssistantStreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    provider: None,
                },
            ],
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        config.max_turns = 3;
        let queued_messages = Arc::clone(&queued);
        let polls = Arc::new(Mutex::new(0usize));
        config.get_steering_messages = Some(Arc::new(move || {
            let queued_messages = Arc::clone(&queued_messages);
            let polls = Arc::clone(&polls);
            Box::pin(async move {
                *polls.lock().await += 1;
                // First poll happens before the first request; deliver the
                // steering message on the post-turn poll so it arrives mid-run.
                if *polls.lock().await < 2 {
                    return Ok(Vec::new());
                }
                Ok(queued_messages.lock().await.take().into_iter().collect())
            })
        }));

        let output = match run_agent_loop(Message::user_text("start"), config).await {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };

        let user_texts: Vec<String> = output
            .messages
            .iter()
            .filter_map(|message| match message {
                Message::User { content, .. } => content.iter().find_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        assert_eq!(
            user_texts,
            vec!["start".to_string(), "steer now".to_string()]
        );
        assert!(matches!(
            output.messages.last(),
            Some(Message::Assistant { content, .. })
                if content.iter().any(|block| matches!(
                    block,
                    ContentBlock::Text { text, .. } if text == "after steer"
                ))
        ));

        let ended_user_texts: Vec<String> = sink
            .events()
            .await
            .into_iter()
            .filter_map(|event| match event {
                AgentEvent::MessageEnd {
                    message: Message::User { content, .. },
                } => content.into_iter().find_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text),
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        assert_eq!(
            ended_user_texts,
            vec!["start".to_string(), "steer now".to_string()]
        );
    }

    #[tokio::test]
    async fn tool_call_deltas_are_accumulated() {
        let call = Uuid::new_v4();
        let stream = Arc::new(FauxStream::turns(vec![
            vec![
                AssistantStreamEvent::ToolCallDelta {
                    id: call,
                    name: Some("echo".into()),
                    native_id: None,
                    arguments_delta: "{\"value\":".into(),
                },
                AssistantStreamEvent::ToolCallDelta {
                    id: call,
                    name: None,
                    native_id: None,
                    arguments_delta: "42}".into(),
                },
                AssistantStreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    provider: None,
                },
            ],
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config
            .tools
            .insert("echo".into(), Arc::new(FakeTool::new("echo", "ok")));
        let output = run_agent_loop(Message::user_text("tools"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert!(output.messages.iter().any(|message| matches!(
            message,
            Message::ToolResult { tool_name, is_error: false, .. } if tool_name == "echo"
        )));
    }

    #[tokio::test]
    async fn preflight_is_source_order_and_parallel_end_events_follow_completion_order() {
        let call_a = Uuid::new_v4();
        let call_b = Uuid::new_v4();
        let stream = Arc::new(FauxStream::turns(vec![
            vec![
                AssistantStreamEvent::ToolCallDone {
                    id: call_a,
                    name: "a".into(),
                    native_id: None,
                    arguments: serde_json::json!({}),
                },
                AssistantStreamEvent::ToolCallDone {
                    id: call_b,
                    name: "b".into(),
                    native_id: None,
                    arguments: serde_json::json!({}),
                },
                AssistantStreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    provider: None,
                },
            ],
            vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }],
        ]));
        let sink = VecEventSink::new();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let seen_hook = Arc::clone(&seen);
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        config.before_tool_call = Some(Arc::new(move |call| {
            let seen = Arc::clone(&seen_hook);
            Box::pin(async move {
                seen.lock().await.push(call.name.clone());
                Ok(BeforeToolCallResult::Allow(call))
            })
        }));
        let mut a = FakeTool::new("a", "A");
        a.delay_ms = 20;
        let b = FakeTool::new("b", "B");
        config.tools.insert("a".into(), Arc::new(a));
        config.tools.insert("b".into(), Arc::new(b));
        let output = run_agent_loop(Message::user_text("tools"), config).await;
        assert!(output.is_ok());
        assert_eq!(*seen.lock().await, vec!["a".to_string(), "b".to_string()]);
        let end_order: Vec<String> = sink
            .events()
            .await
            .into_iter()
            .filter_map(|event| match event {
                AgentEvent::ToolExecutionEnd { result, .. } => Some(result.tool_name),
                _ => None,
            })
            .collect();
        assert_eq!(end_order, vec!["b".to_string(), "a".to_string()]);
    }

    #[tokio::test]
    async fn failed_tools_are_normalized_to_error_results() {
        let call = Uuid::new_v4();
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::ToolCallDone {
                id: call,
                name: "fail".into(),
                native_id: None,
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ]));
        let mut tool = FakeTool::new("fail", "kaput");
        tool.fail = true;
        let mut config = AgentLoopConfig::new(model(), stream);
        config.tools.insert("fail".into(), Arc::new(tool));
        let output = run_agent_loop(Message::user_text("tools"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert!(output.messages.iter().any(|message| matches!(
            message,
            Message::ToolResult { tool_name, is_error: true, content, .. }
                if tool_name == "fail"
                    && matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text.contains("kaput"))
        )));
    }

    #[tokio::test]
    async fn schema_validation_blocks_invalid_tool_arguments() {
        let call = Uuid::new_v4();
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::ToolCallDone {
                id: call,
                name: "needs_arg".into(),
                native_id: None,
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ]));
        let mut tool = FakeTool::new("needs_arg", "should not run");
        tool.definition.input_schema = serde_json::json!({
            "type": "object",
            "required": ["value"]
        });
        let mut config = AgentLoopConfig::new(model(), stream);
        config.tools.insert("needs_arg".into(), Arc::new(tool));
        let output = run_agent_loop(Message::user_text("tools"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert!(output.messages.iter().any(|message| matches!(
            message,
            Message::ToolResult { is_error: true, content, .. }
                if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text.contains("missing required argument"))
        )));
    }

    #[tokio::test]
    async fn tool_hooks_can_block_and_patch_results() {
        let call = Uuid::new_v4();
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::ToolCallDone {
                id: call,
                name: "blocked".into(),
                native_id: None,
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.tools.insert(
            "blocked".into(),
            Arc::new(FakeTool::new("blocked", "should not run")),
        );
        config.before_tool_call = Some(Arc::new(|_call| {
            Box::pin(async {
                Ok(BeforeToolCallResult::Block {
                    reason: "blocked by hook".into(),
                    terminate: false,
                })
            })
        }));
        config.after_tool_call = Some(Arc::new(|mut result| {
            Box::pin(async move {
                result.details = Some(serde_json::json!({"patched": true}));
                Ok(result)
            })
        }));
        let output = run_agent_loop(Message::user_text("tools"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert!(output.messages.iter().any(|message| matches!(
            message,
            Message::ToolResult { is_error: true, details: Some(details), .. }
                if details.get("patched") == Some(&serde_json::json!(true))
        )));
    }

    #[tokio::test]
    async fn abort_during_streaming_stops_as_aborted() {
        let signal = AbortSignal::new();
        signal.abort();
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::TextDelta {
            delta: "no".into(),
        }]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.abort_signal = signal;
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert_eq!(output.stop_reason, StopReason::Aborted);
    }

    /// Stream provider that records every request transcript/model/thinking level.
    struct RecordingStream {
        turns: Mutex<Vec<Vec<AssistantStreamEvent>>>,
        requests: Arc<Mutex<Vec<StreamRequest>>>,
    }

    impl RecordingStream {
        fn turns(turns: Vec<Vec<AssistantStreamEvent>>) -> Arc<Self> {
            Arc::new(Self {
                turns: Mutex::new(turns),
                requests: Arc::new(Mutex::new(Vec::new())),
            })
        }
    }

    #[async_trait]
    impl StreamProvider for RecordingStream {
        async fn stream(
            &self,
            request: StreamRequest,
            signal: AbortSignal,
        ) -> LoopResult<Vec<AssistantStreamEvent>> {
            self.requests.lock().await.push(request);
            if signal.is_aborted() {
                return Ok(vec![AssistantStreamEvent::Aborted]);
            }
            let mut locked = self.turns.lock().await;
            if locked.is_empty() {
                return Ok(vec![AssistantStreamEvent::Done {
                    stop_reason: StopReason::EndTurn,
                    provider: None,
                }]);
            }
            Ok(locked.remove(0))
        }
    }

    fn end_turn_done() -> Vec<AssistantStreamEvent> {
        vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }]
    }

    fn tool_use_turn(name: &str) -> Vec<AssistantStreamEvent> {
        vec![
            AssistantStreamEvent::ToolCallDone {
                id: Uuid::new_v4(),
                name: name.into(),
                native_id: None,
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ]
    }

    #[tokio::test]
    async fn finish_turn_runs_after_tool_results_and_before_turn_end() {
        let sink = VecEventSink::new();
        let stream = Arc::new(FauxStream::turns(vec![
            tool_use_turn("echo"),
            end_turn_done(),
        ]));
        let ordering = Arc::new(Mutex::new(Vec::<String>::new()));
        let ordering_hook = Arc::clone(&ordering);
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        let mut echo = FakeTool::new("echo", "ok");
        echo.terminate = true;
        config.tools.insert("echo".into(), Arc::new(echo));
        config.finish_turn = Some(Arc::new(move |context| {
            let ordering = Arc::clone(&ordering_hook);
            Box::pin(async move {
                assert_eq!(context.tool_results.len(), 1);
                assert!(matches!(
                    context.transcript.messages().last(),
                    Some(Message::ToolResult { .. })
                ));
                ordering.lock().await.push("finish_turn".into());
                Ok(None)
            })
        }));
        let output = run_agent_loop(Message::user_text("echo"), config).await;
        assert!(output.is_ok());
        let events = sink.events().await;
        let tool_result_end = events.iter().position(|event| {
            matches!(
                event,
                AgentEvent::MessageEnd {
                    message: Message::ToolResult { .. }
                }
            )
        });
        let turn_end = events
            .iter()
            .position(|event| matches!(event, AgentEvent::TurnEnd { .. }));
        assert!(tool_result_end.is_some());
        assert!(turn_end.is_some());
        assert!(tool_result_end < turn_end);
        assert_eq!(ordering.lock().await.as_slice(), ["finish_turn"]);
    }

    #[tokio::test]
    async fn finish_turn_runs_on_error_turn_but_decision_is_ignored() {
        let stream = Arc::new(FauxStream::turns(vec![
            vec![AssistantStreamEvent::Error {
                message: "boom".into(),
            }],
            end_turn_done(),
        ]));
        let ordering = Arc::new(Mutex::new(Vec::<String>::new()));
        let ordering_hook = Arc::clone(&ordering);
        let follow_up_polls = Arc::new(Mutex::new(0usize));
        let follow_up_counter = Arc::clone(&follow_up_polls);
        let sink = VecEventSink::new();
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        config.finish_turn = Some(Arc::new(move |context| {
            let ordering = Arc::clone(&ordering_hook);
            Box::pin(async move {
                assert!(matches!(
                    context.message,
                    Message::Assistant {
                        stop_reason: Some(StopReason::Error),
                        ..
                    }
                ));
                ordering.lock().await.push("finish_turn".into());
                Ok(Some(TurnUpdate {
                    decision: Some(TurnDecision::Continue),
                    messages: Vec::new(),
                }))
            })
        }));
        config.get_follow_up_messages = Some(Arc::new(move || {
            let counter = Arc::clone(&follow_up_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                Ok(vec![Message::user_text("queued")])
            })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert_eq!(output.stop_reason, StopReason::Error);
        let events = sink.events().await;
        let turn_ends = events
            .iter()
            .filter(|event| matches!(event, AgentEvent::TurnEnd { .. }))
            .count();
        assert_eq!(turn_ends, 1);
        assert_eq!(ordering.lock().await.as_slice(), ["finish_turn"]);
        assert_eq!(*follow_up_polls.lock().await, 0);
    }

    #[tokio::test]
    async fn finish_turn_end_skips_queue_polling_and_next_request() {
        let stream = Arc::new(FauxStream::turns(vec![
            tool_use_turn("noop"),
            end_turn_done(),
        ]));
        let steering_polls = Arc::new(Mutex::new(0usize));
        let steering_counter = Arc::clone(&steering_polls);
        let follow_up_polls = Arc::new(Mutex::new(0usize));
        let follow_up_counter = Arc::clone(&follow_up_polls);
        let mut config = AgentLoopConfig::new(model(), stream);
        config
            .tools
            .insert("noop".into(), Arc::new(FakeTool::new("noop", "done")));
        config.finish_turn = Some(Arc::new(|_context| {
            Box::pin(async {
                Ok(Some(TurnUpdate {
                    decision: Some(TurnDecision::End),
                    messages: Vec::new(),
                }))
            })
        }));
        config.get_steering_messages = Some(Arc::new(move || {
            let counter = Arc::clone(&steering_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                Ok(Vec::new())
            })
        }));
        config.get_follow_up_messages = Some(Arc::new(move || {
            let counter = Arc::clone(&follow_up_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                Ok(vec![Message::user_text("queued")])
            })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        // Only the first provider turn ran; queued follow-up was never drained.
        assert!(!output.messages.iter().any(|message| matches!(
            message,
            Message::User { content, .. }
                if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text == "queued")
        )));
        assert_eq!(*follow_up_polls.lock().await, 0);
        assert!(*steering_polls.lock().await <= 1);
    }

    #[tokio::test]
    async fn finish_turn_continue_makes_one_context_only_request() {
        let stream = RecordingStream::turns(vec![end_turn_done(), end_turn_done()]);
        let requests = Arc::clone(&stream.requests);
        let finish_calls = Arc::new(Mutex::new(0usize));
        let finish_counter = Arc::clone(&finish_calls);
        let mut config = AgentLoopConfig::new(model(), stream);
        config.finish_turn = Some(Arc::new(move |_context| {
            let counter = Arc::clone(&finish_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                if *counter.lock().await == 1 {
                    Ok(Some(TurnUpdate {
                        decision: Some(TurnDecision::Continue),
                        messages: Vec::new(),
                    }))
                } else {
                    Ok(None)
                }
            })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        assert!(output.is_ok());
        assert_eq!(requests.lock().await.len(), 2);
        assert_eq!(*finish_calls.lock().await, 2);
    }

    #[tokio::test]
    async fn finish_turn_continue_is_satisfied_by_tool_results() {
        let stream = RecordingStream::turns(vec![tool_use_turn("noop"), end_turn_done()]);
        let requests = Arc::clone(&stream.requests);
        let finish_calls = Arc::new(Mutex::new(0usize));
        let finish_counter = Arc::clone(&finish_calls);
        let mut config = AgentLoopConfig::new(model(), stream);
        config
            .tools
            .insert("noop".into(), Arc::new(FakeTool::new("noop", "done")));
        config.finish_turn = Some(Arc::new(move |_context| {
            let counter = Arc::clone(&finish_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                if *counter.lock().await == 1 {
                    Ok(Some(TurnUpdate {
                        decision: Some(TurnDecision::Continue),
                        messages: Vec::new(),
                    }))
                } else {
                    Ok(None)
                }
            })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        assert!(output.is_ok());
        assert_eq!(requests.lock().await.len(), 2);
        assert_eq!(*finish_calls.lock().await, 2);
    }

    #[tokio::test]
    async fn finish_turn_continue_is_satisfied_by_follow_up() {
        let stream = RecordingStream::turns(vec![end_turn_done(), end_turn_done()]);
        let requests = Arc::clone(&stream.requests);
        let finish_calls = Arc::new(Mutex::new(0usize));
        let finish_counter = Arc::clone(&finish_calls);
        let queued = Arc::new(Mutex::new(Some(Message::user_text("follow up"))));
        let queued_messages = Arc::clone(&queued);
        let mut config = AgentLoopConfig::new(model(), stream);
        config.finish_turn = Some(Arc::new(move |_context| {
            let counter = Arc::clone(&finish_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                if *counter.lock().await == 1 {
                    Ok(Some(TurnUpdate {
                        decision: Some(TurnDecision::Continue),
                        messages: Vec::new(),
                    }))
                } else {
                    Ok(None)
                }
            })
        }));
        config.get_follow_up_messages = Some(Arc::new(move || {
            let queued = Arc::clone(&queued_messages);
            Box::pin(async move { Ok(queued.lock().await.take().into_iter().collect()) })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        assert!(output.is_ok());
        assert_eq!(requests.lock().await.len(), 2);
        let second = &requests.lock().await[1];
        assert!(second.transcript.messages().iter().any(|message| matches!(
            message,
            Message::User { content, .. }
                if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text == "follow up")
        )));
    }

    #[tokio::test]
    async fn prepare_request_runs_after_pending_append_and_replaces_request_state() {
        let stream = RecordingStream::turns(vec![end_turn_done()]);
        let requests = Arc::clone(&stream.requests);
        let steering = Arc::new(Mutex::new(Some(Message::user_text("steering"))));
        let steering_messages = Arc::clone(&steering);
        let replacement = Model::new("test", "replacement");
        let replacement_model = replacement.clone();
        let mut config = AgentLoopConfig::new(model(), stream);
        config.get_steering_messages = Some(Arc::new(move || {
            let steering = Arc::clone(&steering_messages);
            Box::pin(async move { Ok(steering.lock().await.take().into_iter().collect()) })
        }));
        config.prepare_request = Some(Arc::new(move |context| {
            let replacement = replacement_model.clone();
            Box::pin(async move {
                assert!(context.transcript.messages().iter().any(|message| matches!(
                    message,
                    Message::User { content, .. }
                        if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text == "steering")
                )));
                Ok(Some(RequestUpdate {
                    transcript: None,
                    model: Some(replacement),
                    thinking_level: Some(ThinkingLevel::High),
                }))
            })
        }));
        let output = run_agent_loop(Message::user_text("prompt"), config).await;
        assert!(output.is_ok());
        let requests = requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].model, replacement);
        assert_eq!(requests[0].thinking_level, ThinkingLevel::High);
    }

    #[tokio::test]
    async fn prepare_request_does_not_poll_steering() {
        let stream = RecordingStream::turns(vec![end_turn_done(), end_turn_done()]);
        let requests = Arc::clone(&stream.requests);
        let queued = Arc::new(Mutex::new(Vec::<Message>::new()));
        let queued_steering = Arc::clone(&queued);
        let late = Message::user_text("late steering");
        let late_steering = late.clone();
        let prepare_calls = Arc::new(Mutex::new(0usize));
        let prepare_counter = Arc::clone(&prepare_calls);
        let mut config = AgentLoopConfig::new(model(), stream);
        config.get_steering_messages = Some(Arc::new(move || {
            let queued = Arc::clone(&queued_steering);
            Box::pin(async move { Ok(std::mem::take(&mut *queued.lock().await)) })
        }));
        config.get_follow_up_messages = Some(Arc::new(|| Box::pin(async { Ok(Vec::new()) })));
        config.prepare_request = Some(Arc::new(move |_context| {
            let counter = Arc::clone(&prepare_counter);
            let late = late_steering.clone();
            let queued = Arc::clone(&queued);
            Box::pin(async move {
                *counter.lock().await += 1;
                if *counter.lock().await == 1 {
                    queued.lock().await.push(late);
                }
                Ok(None)
            })
        }));
        // First turn ends; steering queued during the first prepare_request is
        // picked up only by the post-turn poll, not before the request.
        let finish_calls = Arc::new(Mutex::new(0usize));
        let finish_counter = Arc::clone(&finish_calls);
        config.finish_turn = Some(Arc::new(move |_context| {
            let counter = Arc::clone(&finish_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                if *counter.lock().await == 1 {
                    Ok(Some(TurnUpdate {
                        decision: Some(TurnDecision::Continue),
                        messages: Vec::new(),
                    }))
                } else {
                    Ok(None)
                }
            })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        assert!(output.is_ok());
        let requests = requests.lock().await;
        assert_eq!(requests.len(), 2);
        let contains_late = |request: &StreamRequest| {
            request.transcript.messages().iter().any(|message| matches!(
                message,
                Message::User { content, .. }
                    if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text == "late steering")
            ))
        };
        assert!(!contains_late(&requests[0]));
        assert!(contains_late(&requests[1]));
    }

    #[tokio::test]
    async fn blocked_terminate_skips_follow_up_request() {
        let stream = RecordingStream::turns(vec![tool_use_turn("blocked"), end_turn_done()]);
        let requests = Arc::clone(&stream.requests);
        let follow_up_polls = Arc::new(Mutex::new(0usize));
        let follow_up_counter = Arc::clone(&follow_up_polls);
        let mut config = AgentLoopConfig::new(model(), stream);
        config
            .tools
            .insert("blocked".into(), Arc::new(FakeTool::new("blocked", "no")));
        config.before_tool_call = Some(Arc::new(|_call| {
            Box::pin(async {
                Ok(BeforeToolCallResult::Block {
                    reason: "policy".into(),
                    terminate: true,
                })
            })
        }));
        config.get_follow_up_messages = Some(Arc::new(move || {
            let counter = Arc::clone(&follow_up_counter);
            Box::pin(async move {
                *counter.lock().await += 1;
                Ok(vec![Message::user_text("queued")])
            })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert_eq!(requests.lock().await.len(), 1);
        assert_eq!(output.stop_reason, StopReason::ToolUse);
        assert_eq!(*follow_up_polls.lock().await, 0);
    }

    #[tokio::test]
    async fn tools_not_in_transcript_are_not_available() {
        let call = Uuid::new_v4();
        let stream = Arc::new(FauxStream::turns(vec![
            vec![
                AssistantStreamEvent::ToolCallDone {
                    id: call,
                    name: "hidden".into(),
                    native_id: None,
                    arguments: serde_json::json!({}),
                },
                AssistantStreamEvent::Done {
                    stop_reason: StopReason::ToolUse,
                    provider: None,
                },
            ],
            end_turn_done(),
        ]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config
            .tools
            .insert("hidden".into(), Arc::new(FakeTool::new("hidden", "no")));
        // Transcript already carries a System message declaring only `read`, so
        // `hidden` is registered but not callable.
        let system = Message::System {
            id: Uuid::new_v4(),
            content: "sys".into(),
            sections: Vec::new(),
            tools_added: vec![ToolDefinition {
                name: "read".into(),
                description: "Read".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }],
            tools_removed: Vec::new(),
        };
        let output =
            run_agent_loop_with_context(vec![system], Message::user_text("go"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        assert!(output.messages.iter().any(|message| matches!(
            message,
            Message::ToolResult { is_error: true, content, .. }
                if matches!(content.first(), Some(ContentBlock::Text { text, .. }) if text.contains("tool `hidden` is not available"))
        )));
    }

    #[tokio::test]
    async fn message_accumulator_reproduces_streamed_content() {
        let call = Uuid::new_v4();
        let events = vec![
            AssistantStreamEvent::ThinkingDelta {
                delta: "think".into(),
            },
            AssistantStreamEvent::TextDelta {
                delta: "hel".into(),
            },
            AssistantStreamEvent::TextDelta { delta: "lo".into() },
            AssistantStreamEvent::ToolCallDone {
                id: call,
                name: "read".into(),
                native_id: None,
                arguments: serde_json::json!({"path": "a"}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ];
        let sink = VecEventSink::new();
        let stream = Arc::new(FauxStream::turns(vec![events, end_turn_done()]));
        let mut config = AgentLoopConfig::new(model(), stream);
        config.event_sink = Arc::new(sink.clone());
        config
            .tools
            .insert("read".into(), Arc::new(FakeTool::new("read", "ok")));
        let output = run_agent_loop(Message::user_text("hi"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        let mut accumulator = MessageAccumulator::new();
        for event in sink.events().await {
            if let AgentEvent::MessageDelta { delta, .. } = event {
                accumulator.apply(&delta);
            }
        }
        let streamed = accumulator.content();
        let assistant_content = output
            .messages
            .iter()
            .find_map(|message| match message {
                Message::Assistant { content, .. } => Some(content.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("assistant message missing"));
        assert_eq!(streamed, assistant_content);
        assert!(matches!(
            streamed.first(),
            Some(ContentBlock::Thinking { .. })
        ));
        assert!(matches!(streamed.get(1), Some(ContentBlock::Text { .. })));
        assert!(matches!(
            streamed.get(2),
            Some(ContentBlock::ToolCall { .. })
        ));
    }

    #[tokio::test]
    async fn start_and_thinking_signature_events_are_stored() {
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::Start {
                response_id: Some("req-1".into()),
                response_model: Some("test/model-x".into()),
            },
            AssistantStreamEvent::ThinkingDelta {
                delta: "reason".into(),
            },
            AssistantStreamEvent::ThinkingSignature {
                signature: "sig-1".into(),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            },
        ]));
        let config = AgentLoopConfig::new(model(), stream);
        let output = match run_agent_loop(Message::user_text("hi"), config).await {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        let assistant = output
            .messages
            .iter()
            .find_map(|message| match message {
                Message::Assistant { .. } => Some(message.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("assistant missing"));
        let Message::Assistant {
            content,
            response_model,
            ..
        } = assistant
        else {
            panic!("expected assistant");
        };
        assert_eq!(response_model.as_deref(), Some("test/model-x"));
        assert!(matches!(
            content.first(),
            Some(ContentBlock::Thinking {
                signature: Some(signature),
                ..
            }) if signature == "sig-1"
        ));
    }

    #[tokio::test]
    async fn turn_update_messages_reach_the_next_request() {
        let stream = RecordingStream::turns(vec![end_turn_done(), end_turn_done()]);
        let requests = Arc::clone(&stream.requests);
        let injected = Message::user_text("turn end entry");
        let injected_hook = injected.clone();
        let calls = Arc::new(Mutex::new(0usize));
        let calls_hook = Arc::clone(&calls);
        let mut config = AgentLoopConfig::new(model(), stream);
        config.finish_turn = Some(Arc::new(move |_context| {
            let injected = injected_hook.clone();
            let calls = Arc::clone(&calls_hook);
            Box::pin(async move {
                *calls.lock().await += 1;
                if *calls.lock().await == 1 {
                    Ok(Some(TurnUpdate {
                        decision: Some(TurnDecision::Continue),
                        messages: vec![injected],
                    }))
                } else {
                    Ok(None)
                }
            })
        }));
        let output = run_agent_loop(Message::user_text("run"), config).await;
        let output = match output {
            Ok(output) => output,
            Err(err) => panic!("loop failed: {err}"),
        };
        let requests = requests.lock().await;
        assert_eq!(requests.len(), 2);
        // The injected entry is in the transcript of the very next request...
        assert!(requests[1].transcript.messages().contains(&injected));
        // ...not in the first request...
        assert!(!requests[0].transcript.messages().contains(&injected));
        // ...and in the output messages.
        assert!(output.messages.contains(&injected));
    }
}
