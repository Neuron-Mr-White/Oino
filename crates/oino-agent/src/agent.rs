//! Stateful agent (pi-agent-core `Agent`).
//!
//! [`Agent`] owns the transcript, reduces loop events into [`AgentState`], awaits listeners in
//! subscription order, and exposes the steering / follow-up queues. It is cheap to clone; all
//! clones share one agent.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};

use futures::future::BoxFuture;
use indexmap::IndexSet;
use oino_ai::transcript::{SystemState, initial_system_message, system_message_text};
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, ImageContent, Model, StopReason,
    StreamOptions, ThinkingLevel, UserContent, UserContentField, UserMessage,
};
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::agent_loop::{AgentLoopError, agent_loop, agent_loop_continue, now_ms};
use crate::config::{
    AfterToolCall, AgentContext, AgentLoopConfig, BeforeToolCall, ConvertToLlm, FinishTurn,
    GetApiKey, PrepareNextTurn, PrepareRequest, QueueSource, StreamFn, ToolExecution,
    TransformContext, default_convert_to_llm,
};
use crate::event::{AgentEvent, EventSink};
use crate::tool::AgentTool;

/// How queued messages are drained (pi `QueueMode`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum QueueMode {
    /// Every queued message at once.
    #[serde(rename = "all")]
    All,
    /// One message per turn (pi default).
    #[default]
    #[serde(rename = "one-at-a-time")]
    OneAtATime,
}

/// Operation refused by the agent (pi error texts).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AgentError {
    /// `prompt` while a run is active.
    #[error(
        "Agent is already processing a prompt. Use steer() or followUp() to queue messages, or wait for completion."
    )]
    PromptWhileBusy,
    /// `continue_run` while a run is active.
    #[error("Agent is already processing. Wait for completion before continuing.")]
    ContinueWhileBusy,
    /// `reset` while a run is active.
    #[error("Agent is already processing. Wait for completion before resetting.")]
    ResetWhileBusy,
    /// `continue_run` on an empty (or system-only) transcript.
    #[error("No messages to continue from")]
    NoMessages,
    /// `continue_run` after an assistant message with empty queues.
    #[error("Cannot continue from message role: assistant")]
    CannotContinue,
}

/// Snapshot of the agent state (pi `AgentState`).
#[derive(Clone)]
pub struct AgentState {
    /// Model of the next request.
    pub model: Model,
    /// Thinking level of the next request.
    pub thinking_level: ThinkingLevel,
    /// Executable tools.
    pub tools: Vec<Arc<dyn AgentTool>>,
    /// Transcript.
    pub messages: Vec<AgentMessage>,
    /// A run is active.
    pub is_streaming: bool,
    /// The message currently streaming.
    pub streaming_message: Option<AgentMessage>,
    /// Tool calls started but not settled, in start order.
    pub pending_tool_calls: IndexSet<String>,
    /// Error of the last failed turn.
    pub error_message: Option<String>,
}

impl std::fmt::Debug for AgentState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentState")
            .field("model", &self.model.key())
            .field("thinking_level", &self.thinking_level)
            .field("messages", &self.messages.len())
            .field("is_streaming", &self.is_streaming)
            .field("error_message", &self.error_message)
            .finish_non_exhaustive()
    }
}

impl AgentState {
    /// pi `state.systemPrompt`: the replayed prompt text of the transcript.
    #[must_use]
    pub fn system_prompt(&self) -> String {
        system_prompt_of(&self.messages)
    }
}

/// Replayed system prompt text of `messages`.
#[must_use]
pub fn system_prompt_of(messages: &[AgentMessage]) -> String {
    SystemState::fold(messages.iter().filter_map(|m| match m {
        AgentMessage::System(system) => Some(system),
        _ => None,
    }))
    .into_message()
    .map(|message| system_message_text(&message))
    .unwrap_or_default()
}

/// pi-agent-core `DEFAULT_MODEL`.
#[must_use]
pub fn unknown_model() -> Model {
    Model {
        id: "unknown".into(),
        name: "unknown".into(),
        api: "unknown".into(),
        provider: "unknown".into(),
        base_url: String::new(),
        input: Vec::new(),
        input_limits: None,
        cost: oino_ai::ModelCost::default(),
        headers: None,
        kind: None,
        reasoning: false,
        thinking_level_map: None,
        prompt_cache: None,
        context_window: 0,
        max_tokens: 0,
        sampling_params: None,
        sampling_params_by_thinking_level: None,
        compat: None,
    }
}

/// Hooks forwarded to every run.
#[derive(Clone, Default)]
pub struct AgentHooks {
    /// Context rewrite.
    pub transform_context: Option<Arc<dyn TransformContext>>,
    /// Per-request credential.
    pub get_api_key: Option<Arc<dyn GetApiKey>>,
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
}

/// Construction options (pi `AgentOptions`).
#[derive(Clone)]
pub struct AgentOptions {
    /// Initial system prompt (becomes the leading system message).
    pub system_prompt: String,
    /// Initial model.
    pub model: Model,
    /// Initial thinking level.
    pub thinking_level: ThinkingLevel,
    /// Initial tools.
    pub tools: Vec<Arc<dyn AgentTool>>,
    /// Initial transcript.
    pub messages: Vec<AgentMessage>,
    /// Provider dispatch.
    pub stream: Arc<dyn StreamFn>,
    /// Request-boundary conversion.
    pub convert_to_llm: ConvertToLlm,
    /// Hooks.
    pub hooks: AgentHooks,
    /// Steering drain mode.
    pub steering_mode: QueueMode,
    /// Follow-up drain mode.
    pub follow_up_mode: QueueMode,
    /// Base stream options (`sessionId`, `transport`, budgets, retry caps).
    pub stream_options: StreamOptions,
    /// Batch scheduling.
    pub tool_execution: ToolExecution,
}

impl AgentOptions {
    /// Options with pi defaults for `stream`.
    #[must_use]
    pub fn new(stream: Arc<dyn StreamFn>) -> Self {
        Self {
            system_prompt: String::new(),
            model: unknown_model(),
            thinking_level: ThinkingLevel::Off,
            tools: Vec::new(),
            messages: Vec::new(),
            stream,
            convert_to_llm: Arc::new(default_convert_to_llm),
            hooks: AgentHooks::default(),
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            stream_options: StreamOptions {
                transport: Some(oino_ai::Transport::Auto),
                ..StreamOptions::default()
            },
            tool_execution: ToolExecution::Parallel,
        }
    }
}

/// Listener of agent events (pi `subscribe` callback). Awaited in subscription order.
pub trait AgentListener: Send + Sync {
    /// Handles one event of the active run.
    fn on_event<'a>(
        &'a self,
        event: &'a AgentEvent,
        cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, ()>;
}

/// Unsubscribes on drop.
#[must_use = "dropping the subscription unsubscribes"]
pub struct Subscription {
    inner: Weak<Inner>,
    id: u64,
}

impl Drop for Subscription {
    fn drop(&mut self) {
        if let Some(inner) = self.inner.upgrade() {
            lock(&inner.listeners).retain(|(id, _)| *id != self.id);
        }
    }
}

impl std::fmt::Debug for Subscription {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscription")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

#[derive(Debug)]
struct PendingQueue {
    messages: Vec<AgentMessage>,
    mode: QueueMode,
}

impl PendingQueue {
    const fn new(mode: QueueMode) -> Self {
        Self {
            messages: Vec::new(),
            mode,
        }
    }

    fn peek(&self) -> Vec<AgentMessage> {
        match self.mode {
            QueueMode::All => self.messages.clone(),
            QueueMode::OneAtATime => self.messages.first().cloned().into_iter().collect(),
        }
    }

    fn drain(&mut self) -> Vec<AgentMessage> {
        let drained = self.peek();
        self.messages.drain(..drained.len());
        drained
    }
}

struct ActiveRun {
    cancel: CancellationToken,
}

struct Settings {
    stream: Arc<dyn StreamFn>,
    convert_to_llm: ConvertToLlm,
    hooks: AgentHooks,
    stream_options: StreamOptions,
    tool_execution: ToolExecution,
}

struct Inner {
    state: Mutex<AgentState>,
    listeners: Mutex<Vec<(u64, Arc<dyn AgentListener>)>>,
    steering: Mutex<PendingQueue>,
    follow_up: Mutex<PendingQueue>,
    settings: Mutex<Settings>,
    active: Mutex<Option<ActiveRun>>,
    idle: watch::Sender<bool>,
    next_listener: AtomicU64,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Stateful wrapper around the agent loop (pi `Agent`).
#[derive(Clone)]
pub struct Agent {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Agent")
            .field("state", &*lock(&self.inner.state))
            .finish_non_exhaustive()
    }
}

impl Agent {
    /// Creates an agent; a leading system message is added for the prompt and tools unless the
    /// transcript already starts with one.
    #[must_use]
    pub fn new(options: AgentOptions) -> Self {
        let mut messages = options.messages;
        let declarations: Vec<_> = options
            .tools
            .iter()
            .map(|t| t.meta().declaration())
            .collect();
        if !matches!(messages.first(), Some(AgentMessage::System(_)))
            && let Some(initial) = initial_system_message(&options.system_prompt, &declarations)
        {
            messages.insert(0, AgentMessage::System(initial));
        }
        let (idle, _) = watch::channel(true);
        Self {
            inner: Arc::new(Inner {
                state: Mutex::new(AgentState {
                    model: options.model,
                    thinking_level: options.thinking_level,
                    tools: options.tools,
                    messages,
                    is_streaming: false,
                    streaming_message: None,
                    pending_tool_calls: IndexSet::new(),
                    error_message: None,
                }),
                listeners: Mutex::new(Vec::new()),
                steering: Mutex::new(PendingQueue::new(options.steering_mode)),
                follow_up: Mutex::new(PendingQueue::new(options.follow_up_mode)),
                settings: Mutex::new(Settings {
                    stream: options.stream,
                    convert_to_llm: options.convert_to_llm,
                    hooks: options.hooks,
                    stream_options: options.stream_options,
                    tool_execution: options.tool_execution,
                }),
                active: Mutex::new(None),
                idle,
                next_listener: AtomicU64::new(1),
            }),
        }
    }

    /// Registers a listener; dropping the returned [`Subscription`] removes it.
    pub fn subscribe(&self, listener: Arc<dyn AgentListener>) -> Subscription {
        let id = self.inner.next_listener.fetch_add(1, Ordering::Relaxed);
        lock(&self.inner.listeners).push((id, listener));
        Subscription {
            inner: Arc::downgrade(&self.inner),
            id,
        }
    }

    /// Snapshot of the state.
    #[must_use]
    pub fn state(&self) -> AgentState {
        lock(&self.inner.state).clone()
    }

    /// Runs `f` on the state under the lock (avoids cloning the transcript).
    pub fn with_state<R>(&self, f: impl FnOnce(&AgentState) -> R) -> R {
        f(&lock(&self.inner.state))
    }

    /// Copy of the transcript.
    #[must_use]
    pub fn messages(&self) -> Vec<AgentMessage> {
        lock(&self.inner.state).messages.clone()
    }

    /// Replaces the transcript (copy-on-set).
    pub fn set_messages(&self, messages: Vec<AgentMessage>) {
        lock(&self.inner.state).messages = messages;
    }

    /// Appends one message to the transcript.
    pub fn append_message(&self, message: AgentMessage) {
        lock(&self.inner.state).messages.push(message);
    }

    /// Replaces the last transcript message (pi in-place `message_end` replacement).
    pub fn replace_last_message(&self, message: AgentMessage) {
        if let Some(last) = lock(&self.inner.state).messages.last_mut() {
            *last = message;
        }
    }

    /// Replaces the executable tools.
    pub fn set_tools(&self, tools: Vec<Arc<dyn AgentTool>>) {
        lock(&self.inner.state).tools = tools;
    }

    /// Current model.
    #[must_use]
    pub fn model(&self) -> Model {
        lock(&self.inner.state).model.clone()
    }

    /// Sets the model of the next request.
    pub fn set_model(&self, model: Model) {
        lock(&self.inner.state).model = model;
    }

    /// Current thinking level.
    #[must_use]
    pub fn thinking_level(&self) -> ThinkingLevel {
        lock(&self.inner.state).thinking_level
    }

    /// Sets the thinking level of the next request.
    pub fn set_thinking_level(&self, level: ThinkingLevel) {
        lock(&self.inner.state).thinking_level = level;
    }

    /// pi `state.systemPrompt`.
    #[must_use]
    pub fn system_prompt(&self) -> String {
        system_prompt_of(&lock(&self.inner.state).messages)
    }

    /// Whether a run is active.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        lock(&self.inner.state).is_streaming
    }

    /// Replaces the hooks.
    pub fn set_hooks(&self, hooks: AgentHooks) {
        lock(&self.inner.settings).hooks = hooks;
    }

    /// Updates the base stream options (`sessionId`, `transport`, budgets, …).
    pub fn update_stream_options(&self, f: impl FnOnce(&mut StreamOptions)) {
        f(&mut lock(&self.inner.settings).stream_options);
    }

    /// Sets the batch scheduling.
    pub fn set_tool_execution(&self, mode: ToolExecution) {
        lock(&self.inner.settings).tool_execution = mode;
    }

    /// Steering drain mode.
    #[must_use]
    pub fn steering_mode(&self) -> QueueMode {
        lock(&self.inner.steering).mode
    }

    /// Sets the steering drain mode.
    pub fn set_steering_mode(&self, mode: QueueMode) {
        lock(&self.inner.steering).mode = mode;
    }

    /// Follow-up drain mode.
    #[must_use]
    pub fn follow_up_mode(&self) -> QueueMode {
        lock(&self.inner.follow_up).mode
    }

    /// Sets the follow-up drain mode.
    pub fn set_follow_up_mode(&self, mode: QueueMode) {
        lock(&self.inner.follow_up).mode = mode;
    }

    /// Queues a message injected after the current turn.
    pub fn steer(&self, message: AgentMessage) {
        lock(&self.inner.steering).messages.push(message);
    }

    /// Queues a message delivered when the agent would otherwise stop.
    pub fn follow_up(&self, message: AgentMessage) {
        lock(&self.inner.follow_up).messages.push(message);
    }

    /// Clears the steering queue.
    pub fn clear_steering_queue(&self) {
        lock(&self.inner.steering).messages.clear();
    }

    /// Clears the follow-up queue.
    pub fn clear_follow_up_queue(&self) {
        lock(&self.inner.follow_up).messages.clear();
    }

    /// Clears both queues.
    pub fn clear_all_queues(&self) {
        self.clear_steering_queue();
        self.clear_follow_up_queue();
    }

    /// Whether either queue has messages.
    #[must_use]
    pub fn has_queued_messages(&self) -> bool {
        !lock(&self.inner.steering).messages.is_empty()
            || !lock(&self.inner.follow_up).messages.is_empty()
    }

    /// Messages the next turn would take, without consuming them.
    #[must_use]
    pub fn peek_queued_messages(&self) -> Vec<AgentMessage> {
        let steering = lock(&self.inner.steering).peek();
        if steering.is_empty() {
            lock(&self.inner.follow_up).peek()
        } else {
            steering
        }
    }

    /// Every queued steering message (in order).
    #[must_use]
    pub fn steering_queue(&self) -> Vec<AgentMessage> {
        lock(&self.inner.steering).messages.clone()
    }

    /// Every queued follow-up message (in order).
    #[must_use]
    pub fn follow_up_queue(&self) -> Vec<AgentMessage> {
        lock(&self.inner.follow_up).messages.clone()
    }

    /// Abort signal of the active run.
    #[must_use]
    pub fn signal(&self) -> Option<CancellationToken> {
        lock(&self.inner.active)
            .as_ref()
            .map(|run| run.cancel.clone())
    }

    /// Aborts the active run, if any.
    pub fn abort(&self) {
        if let Some(run) = lock(&self.inner.active).as_ref() {
            run.cancel.cancel();
        }
    }

    /// Resolves once no run is active and every `agent_end` listener has returned.
    pub async fn wait_for_idle(&self) {
        let mut idle = self.inner.idle.subscribe();
        // `wait_for` only fails when the sender is dropped, which cannot happen while `self`
        // keeps `inner` alive.
        let _ = idle.wait_for(|idle| *idle).await;
    }

    /// Clears the transcript (keeping the replayed system baseline) and both queues.
    ///
    /// # Errors
    ///
    /// [`AgentError::ResetWhileBusy`] while a run is active.
    pub fn reset(&self) -> Result<(), AgentError> {
        if lock(&self.inner.active).is_some() {
            return Err(AgentError::ResetWhileBusy);
        }
        {
            let mut state = lock(&self.inner.state);
            let baseline = SystemState::fold(state.messages.iter().filter_map(|m| match m {
                AgentMessage::System(system) => Some(system),
                _ => None,
            }))
            .into_message();
            state.messages = baseline.map(AgentMessage::System).into_iter().collect();
            state.is_streaming = false;
            state.streaming_message = None;
            state.pending_tool_calls.clear();
            state.error_message = None;
        }
        self.clear_all_queues();
        Ok(())
    }

    /// pi `prompt(text, images)`: a user message with text and images.
    ///
    /// # Errors
    ///
    /// [`AgentError::PromptWhileBusy`] while a run is active.
    pub async fn prompt_text(
        &self,
        text: impl Into<String>,
        images: Vec<ImageContent>,
    ) -> Result<(), AgentError> {
        let mut content = vec![UserContent::text(text)];
        content.extend(images.into_iter().map(UserContent::Image));
        self.prompt(vec![AgentMessage::User(UserMessage {
            content: UserContentField::Blocks(content),
            timestamp: now_ms(),
        })])
        .await
    }

    /// pi `prompt(messages)`: starts a run with `messages`; resolves when the run is idle.
    ///
    /// # Errors
    ///
    /// [`AgentError::PromptWhileBusy`] while a run is active.
    pub async fn prompt(&self, messages: Vec<AgentMessage>) -> Result<(), AgentError> {
        let cancel = self.begin_run().ok_or(AgentError::PromptWhileBusy)?;
        self.run(RunKind::Prompt(messages, false), cancel).await;
        Ok(())
    }

    /// pi `continue()`: continues from the transcript; after an assistant message the queues
    /// are drained instead.
    ///
    /// # Errors
    ///
    /// [`AgentError::ContinueWhileBusy`], [`AgentError::NoMessages`],
    /// [`AgentError::CannotContinue`].
    pub async fn continue_run(&self) -> Result<(), AgentError> {
        if lock(&self.inner.active).is_some() {
            return Err(AgentError::ContinueWhileBusy);
        }
        let (last_is_assistant, empty) = {
            let state = lock(&self.inner.state);
            (
                matches!(state.messages.last(), Some(AgentMessage::Assistant(_))),
                state
                    .messages
                    .iter()
                    .all(|m| matches!(m, AgentMessage::System(_))),
            )
        };
        if empty {
            return Err(AgentError::NoMessages);
        }
        let kind = if last_is_assistant {
            let steering = lock(&self.inner.steering).drain();
            if steering.is_empty() {
                let follow_ups = lock(&self.inner.follow_up).drain();
                if follow_ups.is_empty() {
                    return Err(AgentError::CannotContinue);
                }
                RunKind::Prompt(follow_ups, false)
            } else {
                RunKind::Prompt(steering, true)
            }
        } else {
            RunKind::Continue
        };
        let cancel = self.begin_run().ok_or(AgentError::ContinueWhileBusy)?;
        self.run(kind, cancel).await;
        Ok(())
    }

    fn begin_run(&self) -> Option<CancellationToken> {
        let mut active = lock(&self.inner.active);
        if active.is_some() {
            return None;
        }
        let cancel = CancellationToken::new();
        *active = Some(ActiveRun {
            cancel: cancel.clone(),
        });
        drop(active);
        self.inner.idle.send_replace(false);
        let mut state = lock(&self.inner.state);
        state.is_streaming = true;
        state.streaming_message = None;
        state.error_message = None;
        Some(cancel)
    }

    async fn run(&self, kind: RunKind, cancel: CancellationToken) {
        let (context, config) = self.loop_inputs(matches!(kind, RunKind::Prompt(_, true)));
        let sink = AgentSink {
            inner: Arc::clone(&self.inner),
            cancel: cancel.clone(),
        };
        let outcome = match kind {
            RunKind::Prompt(messages, _) => {
                agent_loop(messages, context, &config, &sink, cancel.clone()).await
            }
            RunKind::Continue => agent_loop_continue(context, &config, &sink, cancel.clone()).await,
        };
        if let Err(error) = outcome {
            self.handle_run_failure(&error, cancel.is_cancelled(), &sink)
                .await;
        }
        self.finish_run();
    }

    fn loop_inputs(&self, skip_initial_steering_poll: bool) -> (AgentContext, AgentLoopConfig) {
        let (context, model, thinking_level) = {
            let state = lock(&self.inner.state);
            (
                AgentContext {
                    messages: state.messages.clone(),
                    tools: state.tools.clone(),
                },
                state.model.clone(),
                state.thinking_level,
            )
        };
        let settings = lock(&self.inner.settings);
        let hooks = settings.hooks.clone();
        let config = AgentLoopConfig {
            model,
            thinking_level,
            stream_options: settings.stream_options.clone(),
            stream: Arc::clone(&settings.stream),
            convert_to_llm: Arc::clone(&settings.convert_to_llm),
            transform_context: hooks.transform_context,
            get_api_key: hooks.get_api_key,
            api_key: None,
            before_tool_call: hooks.before_tool_call,
            after_tool_call: hooks.after_tool_call,
            finish_turn: hooks.finish_turn,
            prepare_request: hooks.prepare_request,
            prepare_next_turn: hooks.prepare_next_turn,
            get_steering_messages: Some(Arc::new(QueueDrain {
                inner: Arc::downgrade(&self.inner),
                steering: true,
                skip_next: AtomicBool::new(skip_initial_steering_poll),
            })),
            get_follow_up_messages: Some(Arc::new(QueueDrain {
                inner: Arc::downgrade(&self.inner),
                steering: false,
                skip_next: AtomicBool::new(false),
            })),
            tool_execution: settings.tool_execution,
        };
        (context, config)
    }

    async fn handle_run_failure(&self, error: &AgentLoopError, aborted: bool, sink: &AgentSink) {
        let model = self.model();
        let mut failure = AssistantMessage::empty(&model.api, &model.provider, &model.id, now_ms());
        failure.content = vec![AssistantContent::text("")];
        failure.stop_reason = if aborted {
            StopReason::Aborted
        } else {
            StopReason::Error
        };
        failure.error_message = Some(error.to_string());
        let message = AgentMessage::Assistant(failure);
        sink.emit(AgentEvent::MessageStart {
            message: message.clone(),
        })
        .await;
        sink.emit(AgentEvent::MessageEnd {
            message: message.clone(),
        })
        .await;
        sink.emit(AgentEvent::TurnEnd {
            message: message.clone(),
            tool_results: Vec::new(),
        })
        .await;
        sink.emit(AgentEvent::AgentEnd {
            messages: vec![message],
        })
        .await;
    }

    fn finish_run(&self) {
        {
            let mut state = lock(&self.inner.state);
            state.is_streaming = false;
            state.streaming_message = None;
            state.pending_tool_calls.clear();
        }
        *lock(&self.inner.active) = None;
        self.inner.idle.send_replace(true);
    }
}

enum RunKind {
    /// Prompt messages; `true` skips the initial steering poll (they came from steering).
    Prompt(Vec<AgentMessage>, bool),
    Continue,
}

struct QueueDrain {
    inner: Weak<Inner>,
    steering: bool,
    skip_next: AtomicBool,
}

impl QueueSource for QueueDrain {
    fn drain(&self) -> BoxFuture<'_, Vec<AgentMessage>> {
        Box::pin(async move {
            if self.skip_next.swap(false, Ordering::AcqRel) {
                return Vec::new();
            }
            let Some(inner) = self.inner.upgrade() else {
                return Vec::new();
            };
            if self.steering {
                lock(&inner.steering).drain()
            } else {
                lock(&inner.follow_up).drain()
            }
        })
    }
}

struct AgentSink {
    inner: Arc<Inner>,
    cancel: CancellationToken,
}

impl AgentSink {
    fn reduce(&self, event: &AgentEvent) {
        let mut state = lock(&self.inner.state);
        match event {
            AgentEvent::MessageStart { message } | AgentEvent::MessageUpdate { message, .. } => {
                state.streaming_message = Some(message.clone());
            }
            AgentEvent::MessageEnd { message } => {
                state.streaming_message = None;
                state.messages.push(message.clone());
            }
            AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                state.pending_tool_calls.insert(tool_call_id.clone());
            }
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => {
                state.pending_tool_calls.shift_remove(tool_call_id);
            }
            AgentEvent::TurnEnd {
                message: AgentMessage::Assistant(message),
                ..
            } => {
                if let Some(error) = &message.error_message {
                    state.error_message = Some(error.clone());
                }
            }
            AgentEvent::AgentEnd { .. } => state.streaming_message = None,
            _ => {}
        }
    }
}

impl EventSink for AgentSink {
    fn emit(&self, event: AgentEvent) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            self.reduce(&event);
            let listeners: Vec<Arc<dyn AgentListener>> = lock(&self.inner.listeners)
                .iter()
                .map(|(_, listener)| Arc::clone(listener))
                .collect();
            for listener in listeners {
                listener.on_event(&event, &self.cancel).await;
            }
        })
    }
}
