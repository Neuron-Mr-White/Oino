#![doc = r#"High-level Oino runtime harness.

The harness is the headless runtime boundary between the pure agent loop and the
application shell. It wires a stateful agent to sessions, tools, resources, auth
resolution, execution environments, and deterministic typed hooks without owning a
terminal UI, provider-specific HTTP code, extension package management, memory DBs,
or project filesystem discovery.

## Boundary map

- [`HarnessConfig`] supplies the model, stream provider, event sink, tools,
  session tree, execution environment, resource labels, and optional auth resolver.
- [`Harness`] owns the live agent plus synchronized session/tool/resource state.
  It records prompts, model changes, thinking-level changes, titles, compaction
  summaries, and session replacements; callers choose when and where to persist
  with [`Harness::save_session_jsonl`].
- [`HookRegistry`] exposes deterministic notification and mutation hooks. Use
  [`NotificationHook`] for event fan-out and [`MutatingHook`] families for context,
  provider-boundary, tool, compaction, or tree-navigation changes.
- [`Harness::sync_system`] records the desired prompt (base plus ordered
  [`PromptSection`]s); the next prompt flushes one delta `System` message into the
  agent context and session, which is the prefix-stability mechanism behind
  `/reload`, resume, and tool changes.
- [`FullPromptInspect`] is a debugging snapshot for `/inspect`: model, thinking
  level, replayed system prompt and prompt deltas, transformed messages, tools, and
  an approximate token count.

## Contributor rules

Keep this crate provider- and UI-agnostic. Add user-facing commands, key handling,
settings pages, and rendering in `oino-tui`/`oino-app`; add provider request JSON
inside provider crates; add resource path discovery in `oino-resource`; and add
raw tool filesystem/process behavior behind `oino-env`/`oino-tools`. When adding
new mutable harness state, keep lock scopes small and avoid holding async mutex
guards while calling providers or tools.
"#]
#![forbid(unsafe_code)]

use oino_agent::Agent;
use oino_agent_loop::{
    AbortSignal, AfterToolCall, AgentEvent, AgentLoopConfig, BeforeToolCall, BeforeToolCallResult,
    BoxFuture, EventSink, FinishTurn, LoopError, LoopResult, StreamEventSink, StreamProvider,
    StreamRequest, Tool, ToolCall, ToolResult, TransformContext, TurnContext, TurnDecision,
    TurnUpdate,
};
mod cache;

pub use cache::{
    CacheMissNotice, CacheMissNotifier, CacheStatus, CacheTracker, CacheWarmingConfig,
    CacheWarmingMode, CacheWarmingVeto, ModelCacheInfo, ModelCacheInfoLookup,
};
use oino_env::{ExecutionEnv, LocalExecutionEnv};
use oino_session::{SessionEntry, SessionEntryKind, SessionManager};
use oino_types::{
    transcript::{current_system, current_tools, render_system_prompt, tool_declaration_canonical},
    AssistantStreamEvent, ContentBlock, Message, Model, PromptSection, ThinkingLevel,
};
use std::{collections::BTreeMap, sync::Arc};
use thiserror::Error;
use tokio::sync::Mutex;

#[derive(Debug, Error)]
pub enum HarnessError {
    #[error(transparent)]
    Agent(#[from] oino_agent::AgentError),
    #[error(transparent)]
    Session(#[from] oino_session::SessionError),
}

pub type HarnessResult<T> = Result<T, HarnessError>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NotificationHook {
    AgentStart,
    TurnStart,
    MessageStart,
    MessageUpdate,
    MessageEnd,
    ToolExecutionStart,
    ToolExecutionUpdate,
    ToolExecutionEnd,
    TurnEnd,
    AgentEnd,
    QueueUpdate,
    SavePoint,
    Settled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum MutatingHook {
    BeforeAgentStart,
    Context,
    BeforeProviderRequest,
    BeforeProviderPayload,
    AfterProviderResponse,
    BeforeToolCall,
    AfterToolCall,
    BeforeCompaction,
    BeforeTreeNavigation,
}

pub type NotificationHandler = Arc<dyn Fn(AgentEvent) -> BoxFuture<'static, ()> + Send + Sync>;
/// `before_agent_start` handlers return prompt-section edits merged into the
/// pending system state; never a whole replacement prompt.
pub type BeforeAgentStartHandler =
    Arc<dyn Fn() -> BoxFuture<'static, LoopResult<Vec<PromptSection>>> + Send + Sync>;
/// `turn_end` handlers see the finished turn and may append entries and/or
/// request one more turn.
pub type TurnEndHandler =
    Arc<dyn Fn(TurnContext) -> BoxFuture<'static, LoopResult<Option<TurnEndResult>>> + Send + Sync>;

/// Result of an actionable `turn_end` hook: extra entries appended (in
/// registration order) and whether the run should continue.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TurnEndResult {
    pub entries: Vec<Message>,
    pub continue_run: bool,
}
pub type ContextHandler =
    Arc<dyn Fn(Vec<Message>) -> BoxFuture<'static, LoopResult<Vec<Message>>> + Send + Sync>;
pub type BeforeToolCallHandler =
    Arc<dyn Fn(ToolCall) -> BoxFuture<'static, LoopResult<BeforeToolCallResult>> + Send + Sync>;
pub type AfterToolCallHandler =
    Arc<dyn Fn(ToolResult) -> BoxFuture<'static, LoopResult<ToolResult>> + Send + Sync>;
pub type StringMutationHandler =
    Arc<dyn Fn(String) -> BoxFuture<'static, LoopResult<String>> + Send + Sync>;
/// `input` handlers may rewrite the user's prompt message before it is
/// appended to the transcript and sent to the provider.
pub type InputHandler =
    Arc<dyn Fn(Message) -> BoxFuture<'static, LoopResult<Message>> + Send + Sync>;
pub type AuthResolver =
    Arc<dyn Fn(String) -> BoxFuture<'static, LoopResult<Option<String>>> + Send + Sync>;

#[derive(Clone, Default)]
pub struct HookRegistry {
    notifications: Arc<Mutex<BTreeMap<NotificationHook, Vec<NotificationHandler>>>>,
    before_agent_start: Arc<Mutex<Vec<BeforeAgentStartHandler>>>,
    context: Arc<Mutex<Vec<ContextHandler>>>,
    before_tool_call: Arc<Mutex<Vec<BeforeToolCallHandler>>>,
    after_tool_call: Arc<Mutex<Vec<AfterToolCallHandler>>>,
    before_provider_request: Arc<Mutex<Vec<StringMutationHandler>>>,
    before_provider_payload: Arc<Mutex<Vec<StringMutationHandler>>>,
    after_provider_response: Arc<Mutex<Vec<StringMutationHandler>>>,
    before_compaction: Arc<Mutex<Vec<StringMutationHandler>>>,
    before_tree_navigation: Arc<Mutex<Vec<StringMutationHandler>>>,
    turn_end: Arc<Mutex<Vec<TurnEndHandler>>>,
    input: Arc<Mutex<Vec<InputHandler>>>,
}

impl HookRegistry {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub async fn on_notification(&self, hook: NotificationHook, handler: NotificationHandler) {
        self.notifications
            .lock()
            .await
            .entry(hook)
            .or_default()
            .push(handler);
    }
    pub async fn on_before_agent_start(&self, handler: BeforeAgentStartHandler) {
        self.before_agent_start.lock().await.push(handler);
    }
    pub async fn on_context(&self, handler: ContextHandler) {
        self.context.lock().await.push(handler);
    }
    pub async fn on_before_tool_call(&self, handler: BeforeToolCallHandler) {
        self.before_tool_call.lock().await.push(handler);
    }
    pub async fn on_after_tool_call(&self, handler: AfterToolCallHandler) {
        self.after_tool_call.lock().await.push(handler);
    }
    pub async fn on_before_provider_request(&self, handler: StringMutationHandler) {
        self.before_provider_request.lock().await.push(handler);
    }
    pub async fn on_before_provider_payload(&self, handler: StringMutationHandler) {
        self.before_provider_payload.lock().await.push(handler);
    }
    pub async fn on_after_provider_response(&self, handler: StringMutationHandler) {
        self.after_provider_response.lock().await.push(handler);
    }
    pub async fn on_before_compaction(&self, handler: StringMutationHandler) {
        self.before_compaction.lock().await.push(handler);
    }
    pub async fn on_before_tree_navigation(&self, handler: StringMutationHandler) {
        self.before_tree_navigation.lock().await.push(handler);
    }
    /// Register an actionable turn-end hook. Handlers run in registration
    /// order; returned entries are appended (agent + session) and any
    /// `continue_run` maps to [`TurnDecision::Continue`].
    pub async fn on_turn_end(&self, handler: TurnEndHandler) {
        self.turn_end.lock().await.push(handler);
    }
    /// Register an `input` hook: handlers may rewrite the user message before
    /// it is persisted and sent to the provider. Extension `input` events map
    /// here.
    pub async fn on_input(&self, handler: InputHandler) {
        self.input.lock().await.push(handler);
    }

    async fn notify(&self, event: AgentEvent) {
        let Some(hook) = hook_for_event(&event) else {
            return;
        };
        let handlers = self
            .notifications
            .lock()
            .await
            .get(&hook)
            .cloned()
            .unwrap_or_default();
        for handler in handlers {
            handler(event.clone()).await;
        }
    }

    async fn run_before_agent_start(&self) -> LoopResult<Vec<PromptSection>> {
        let mut edits = Vec::new();
        for handler in self.before_agent_start.lock().await.clone() {
            edits.extend(handler().await?);
        }
        Ok(edits)
    }

    async fn run_input(&self, mut message: Message) -> LoopResult<Message> {
        for handler in self.input.lock().await.clone() {
            message = handler(message).await?;
        }
        Ok(message)
    }

    async fn run_turn_end(&self, context: TurnContext) -> LoopResult<TurnEndResult> {
        let mut merged = TurnEndResult::default();
        for handler in self.turn_end.lock().await.clone() {
            if let Some(result) = handler(context.clone()).await? {
                merged.entries.extend(result.entries);
                merged.continue_run |= result.continue_run;
            }
        }
        Ok(merged)
    }

    async fn run_context(&self, mut messages: Vec<Message>) -> LoopResult<Vec<Message>> {
        for handler in self.context.lock().await.clone() {
            messages = handler(messages).await?;
        }
        Ok(messages)
    }

    async fn run_before_tool_call(&self, mut call: ToolCall) -> LoopResult<BeforeToolCallResult> {
        for handler in self.before_tool_call.lock().await.clone() {
            match handler(call.clone()).await? {
                BeforeToolCallResult::Allow(rewritten) => call = rewritten,
                blocked @ BeforeToolCallResult::Block { .. } => return Ok(blocked),
            }
        }
        Ok(BeforeToolCallResult::Allow(call))
    }

    async fn run_after_tool_call(&self, mut result: ToolResult) -> LoopResult<ToolResult> {
        for handler in self.after_tool_call.lock().await.clone() {
            result = handler(result).await?;
        }
        Ok(result)
    }

    pub async fn mutate_before_provider_request(&self, value: String) -> LoopResult<String> {
        run_string_hooks(Arc::clone(&self.before_provider_request), value).await
    }
    pub async fn mutate_before_provider_payload(&self, value: String) -> LoopResult<String> {
        run_string_hooks(Arc::clone(&self.before_provider_payload), value).await
    }
    pub async fn mutate_after_provider_response(&self, value: String) -> LoopResult<String> {
        run_string_hooks(Arc::clone(&self.after_provider_response), value).await
    }
    pub async fn mutate_before_compaction(&self, value: String) -> LoopResult<String> {
        run_string_hooks(Arc::clone(&self.before_compaction), value).await
    }
    pub async fn mutate_before_tree_navigation(&self, value: String) -> LoopResult<String> {
        run_string_hooks(Arc::clone(&self.before_tree_navigation), value).await
    }
}

async fn run_string_hooks(
    hooks: Arc<Mutex<Vec<StringMutationHandler>>>,
    mut value: String,
) -> LoopResult<String> {
    for handler in hooks.lock().await.clone() {
        value = handler(value).await?;
    }
    Ok(value)
}

async fn append_session_title(session: &Arc<Mutex<SessionManager>>, title: String) {
    session.lock().await.append(SessionEntryKind::SessionInfo {
        name: Some(title),
        cwd: None,
    });
}

fn has_explicit_session_title(session: &SessionManager) -> bool {
    session
        .get_branch(session.get_leaf_id())
        .ok()
        .is_some_and(|branch| {
            branch.iter().any(|entry| {
                matches!(
                    &entry.kind,
                    SessionEntryKind::SessionInfo {
                        name: Some(name),
                        ..
                    } if !name.trim().is_empty()
                )
            })
        })
}

fn hook_for_event(event: &AgentEvent) -> Option<NotificationHook> {
    match event {
        AgentEvent::AgentStart { .. } => Some(NotificationHook::AgentStart),
        AgentEvent::TurnStart { .. } => Some(NotificationHook::TurnStart),
        AgentEvent::MessageStart { .. } => Some(NotificationHook::MessageStart),
        AgentEvent::MessageDelta { .. } => Some(NotificationHook::MessageUpdate),
        AgentEvent::MessageEnd { .. } => Some(NotificationHook::MessageEnd),
        AgentEvent::ToolExecutionStart { .. } => Some(NotificationHook::ToolExecutionStart),
        AgentEvent::ToolExecutionUpdate { .. } => Some(NotificationHook::ToolExecutionUpdate),
        AgentEvent::ToolExecutionEnd { .. } => Some(NotificationHook::ToolExecutionEnd),
        AgentEvent::TurnEnd { .. } => Some(NotificationHook::TurnEnd),
        AgentEvent::AgentEnd { .. } => Some(NotificationHook::AgentEnd),
        AgentEvent::QueueUpdate { .. } => Some(NotificationHook::QueueUpdate),
        AgentEvent::SavePoint { .. } => Some(NotificationHook::SavePoint),
        AgentEvent::Settled => Some(NotificationHook::Settled),
    }
}

struct HookedStreamProvider {
    inner: Arc<dyn StreamProvider>,
    hooks: HookRegistry,
    cache: Arc<CacheTracker>,
}

#[async_trait::async_trait]
impl StreamProvider for HookedStreamProvider {
    async fn stream(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
    ) -> LoopResult<Vec<AssistantStreamEvent>> {
        self.inner.stream(request, signal).await
    }

    async fn stream_events(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
        sink: StreamEventSink,
    ) -> LoopResult<()> {
        let _request_marker = self
            .hooks
            .mutate_before_provider_request("request".into())
            .await?;
        let _payload_marker = self
            .hooks
            .mutate_before_provider_payload("payload".into())
            .await?;
        let previous = self.cache.request_fingerprints(&request);
        self.cache.begin_request(&request);
        let usage = Arc::new(std::sync::Mutex::new(None::<oino_types::Usage>));
        let usage_sink = usage.clone();
        let wrapped: StreamEventSink = Arc::new(move |event| {
            if let AssistantStreamEvent::Usage { usage: reported } = &event {
                *usage_sink.lock().unwrap_or_else(|e| e.into_inner()) = Some(reported.clone());
            }
            sink(event)
        });
        self.inner.stream_events(request, signal, wrapped).await?;
        let _response_marker = self
            .hooks
            .mutate_after_provider_response("response".into())
            .await?;
        if let Some(usage) = usage.lock().unwrap_or_else(|e| e.into_inner()).clone() {
            self.cache.complete_request(previous, &usage);
        }
        Ok(())
    }
}

struct HookEventSink {
    registry: HookRegistry,
    inner: Arc<dyn EventSink>,
    cache: Arc<CacheTracker>,
}
#[async_trait::async_trait]
impl EventSink for HookEventSink {
    async fn emit(&self, event: AgentEvent) -> LoopResult<()> {
        self.inner.emit(event.clone()).await?;
        self.registry.notify(event.clone()).await;
        self.cache.on_agent_event(&event).await;
        Ok(())
    }
}

pub struct HarnessConfig {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub system_prompt: Option<String>,
    pub stream: Arc<dyn StreamProvider>,
    pub event_sink: Arc<dyn EventSink>,
    pub tools: BTreeMap<String, Arc<dyn Tool>>,
    pub session: SessionManager,
    pub env: Arc<dyn ExecutionEnv>,
    pub resources: Vec<String>,
    pub auth_resolver: Option<AuthResolver>,
    /// Agent-loop turn cap override (`None` keeps the default).
    pub max_turns: Option<u32>,
}

impl HarnessConfig {
    #[must_use]
    pub fn new(model: Model, stream: Arc<dyn StreamProvider>, session: SessionManager) -> Self {
        Self {
            model,
            thinking_level: ThinkingLevel::Off,
            system_prompt: None,
            stream,
            event_sink: Arc::new(oino_agent_loop::NoopEventSink),
            tools: BTreeMap::new(),
            session,
            env: Arc::new(LocalExecutionEnv),
            resources: Vec::new(),
            auth_resolver: None,
            max_turns: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FullPromptInspect {
    pub content: String,
    pub token_count: usize,
}

/// Desired prompt state: base text plus ordered sections.
type PendingSystemState = (String, Vec<PromptSection>);

pub struct Harness {
    agent: Agent,
    session: Arc<Mutex<SessionManager>>,
    hooks: HookRegistry,
    env: Arc<dyn ExecutionEnv>,
    resources: Arc<Mutex<Vec<String>>>,
    tools: Arc<Mutex<BTreeMap<String, Arc<dyn Tool>>>>,
    auth_resolver: Option<AuthResolver>,
    cache_tracker: Arc<CacheTracker>,
    /// Desired prompt state set by [`Harness::sync_system`]; flushed into the
    /// transcript as one delta `System` message before the next prompt.
    pending_system: Arc<Mutex<Option<PendingSystemState>>>,
    /// Bridge fallback prompt used when no `System` message exists and no
    /// pending state was synced.
    system_prompt_fallback: Option<String>,
}

impl Harness {
    #[must_use]
    pub fn new(config: HarnessConfig) -> Self {
        let session_context = config.session.build_session_context().ok();
        let initial_messages = session_context
            .as_ref()
            .map_or_else(Vec::new, |context| context.messages.clone());
        let initial_model = session_context
            .as_ref()
            .and_then(|context| context.model.clone())
            .unwrap_or(config.model);
        let initial_thinking_level = session_context
            .as_ref()
            .and_then(|context| context.thinking_level)
            .unwrap_or(config.thinking_level);
        let hooks = HookRegistry::new();
        let context_hooks = hooks.clone();
        let before_hooks = hooks.clone();
        let after_hooks = hooks.clone();
        let turn_end_hooks = hooks.clone();
        let cache_tracker = Arc::new(CacheTracker::new(Arc::clone(&config.stream)));
        let stream = Arc::new(HookedStreamProvider {
            inner: Arc::clone(&config.stream),
            hooks: hooks.clone(),
            cache: Arc::clone(&cache_tracker),
        }) as Arc<dyn StreamProvider>;
        let mut loop_config = AgentLoopConfig::new(initial_model, stream);
        loop_config.thinking_level = initial_thinking_level;
        loop_config.system_prompt = config.system_prompt.clone();
        loop_config.session_id = Some(config.session.header().session_id.to_string());
        loop_config.tools = config.tools.clone();
        if let Some(max_turns) = config.max_turns {
            loop_config.max_turns = max_turns;
        }
        loop_config.event_sink = Arc::new(HookEventSink {
            registry: hooks.clone(),
            inner: config.event_sink,
            cache: Arc::clone(&cache_tracker),
        });
        loop_config.transform_context = Some(Arc::new(move |messages| {
            let hooks = context_hooks.clone();
            let fut: BoxFuture<'static, LoopResult<Vec<Message>>> =
                Box::pin(async move { hooks.run_context(messages).await });
            fut
        }) as TransformContext);
        loop_config.before_tool_call = Some(Arc::new(move |call| {
            let hooks = before_hooks.clone();
            let fut: BoxFuture<'static, LoopResult<BeforeToolCallResult>> =
                Box::pin(async move { hooks.run_before_tool_call(call).await });
            fut
        }) as BeforeToolCall);
        loop_config.after_tool_call = Some(Arc::new(move |result| {
            let hooks = after_hooks.clone();
            let fut: BoxFuture<'static, LoopResult<ToolResult>> =
                Box::pin(async move { hooks.run_after_tool_call(result).await });
            fut
        }) as AfterToolCall);
        loop_config.finish_turn = Some(Arc::new(move |context| {
            let hooks = turn_end_hooks.clone();
            let fut: BoxFuture<'static, LoopResult<Option<TurnUpdate>>> = Box::pin(async move {
                let result = hooks.run_turn_end(context).await?;
                Ok(Some(TurnUpdate {
                    decision: result.continue_run.then_some(TurnDecision::Continue),
                    messages: result.entries,
                }))
            });
            fut
        }) as FinishTurn);
        Self {
            agent: Agent::new_with_messages(loop_config, initial_messages),
            session: Arc::new(Mutex::new(config.session)),
            hooks,
            env: config.env,
            resources: Arc::new(Mutex::new(config.resources)),
            tools: Arc::new(Mutex::new(config.tools)),
            auth_resolver: config.auth_resolver,
            cache_tracker,
            pending_system: Arc::new(Mutex::new(None)),
            system_prompt_fallback: config.system_prompt,
        }
    }

    #[must_use]
    pub fn hooks(&self) -> HookRegistry {
        self.hooks.clone()
    }
    #[must_use]
    pub fn env(&self) -> Arc<dyn ExecutionEnv> {
        Arc::clone(&self.env)
    }

    pub async fn prompt(&self, message: Message) -> HarnessResult<Vec<Message>> {
        let edits = self
            .hooks
            .run_before_agent_start()
            .await
            .map_err(oino_agent::AgentError::Loop)?;
        self.merge_section_edits(edits).await;
        self.flush_system().await?;
        let message = self
            .hooks
            .run_input(message)
            .await
            .map_err(oino_agent::AgentError::Loop)?;
        let output = self.agent.prompt(message).await?;
        self.persist_messages(&output.messages).await;
        self.hooks
            .notify(AgentEvent::SavePoint {
                label: "prompt".into(),
            })
            .await;
        self.hooks.notify(AgentEvent::Settled).await;
        Ok(output.messages)
    }

    pub async fn steer(&self, message: Message) -> HarnessResult<()> {
        self.agent
            .steer(message)
            .await
            .map_err(oino_agent::AgentError::Loop)?;
        Ok(())
    }

    pub async fn abort(&self) {
        self.agent.abort().await;
    }

    pub async fn skill(&self, name: &str, input: &str) -> HarnessResult<Vec<Message>> {
        self.prompt(Message::user_text(format!("Run skill `{name}`:\n{input}")))
            .await
    }

    pub async fn prompt_template(
        &self,
        template: &str,
        input: &str,
    ) -> HarnessResult<Vec<Message>> {
        self.prompt(Message::user_text(template.replace("{{input}}", input)))
            .await
    }

    pub async fn compact(&self, summary: String) -> HarnessResult<()> {
        self.append_compaction(summary, None, 0).await.map(|_| ())
    }

    #[must_use]
    pub fn session_handle(&self) -> Arc<Mutex<SessionManager>> {
        Arc::clone(&self.session)
    }

    pub async fn active_branch_entries(&self) -> HarnessResult<Vec<SessionEntry>> {
        let session = self.session.lock().await;
        Ok(session.get_branch(session.get_leaf_id())?)
    }

    pub async fn all_session_entries(&self) -> Vec<SessionEntry> {
        self.session.lock().await.get_entries()
    }

    /// Append a compaction entry. `first_kept` is the id of the first entry on
    /// the active branch to keep (`None` retains nothing). `tokens_before` is
    /// the estimated token count of the compacted region.
    pub async fn append_compaction(
        &self,
        summary: String,
        first_kept: Option<oino_types::OinoId>,
        tokens_before: u64,
    ) -> HarnessResult<Vec<Message>> {
        let summary = self
            .hooks
            .mutate_before_compaction(summary)
            .await
            .map_err(oino_agent::AgentError::Loop)?;
        let messages = {
            let mut session = self.session.lock().await;
            session.append_compaction(summary, first_kept, tokens_before);
            session.build_session_context()?.messages
        };
        self.agent.replace_messages(messages.clone()).await;
        Ok(messages)
    }

    pub async fn append_branch_summary(&self, summary: String) -> HarnessResult<Vec<Message>> {
        let messages = {
            let mut session = self.session.lock().await;
            session.append(SessionEntryKind::BranchSummary { summary });
            session.build_session_context()?.messages
        };
        self.agent.replace_messages(messages.clone()).await;
        Ok(messages)
    }

    pub async fn navigate_tree(&self, leaf: Option<oino_types::OinoId>) -> HarnessResult<()> {
        let label = leaf
            .map(|id| id.to_string())
            .unwrap_or_else(|| "root".into());
        let _label = self
            .hooks
            .mutate_before_tree_navigation(label)
            .await
            .map_err(oino_agent::AgentError::Loop)?;
        self.session.lock().await.reset_leaf(leaf)?;
        Ok(())
    }

    pub async fn replace_session(&self, session: SessionManager) {
        let context = session.build_session_context().ok();
        let messages = context
            .as_ref()
            .map_or_else(Vec::new, |context| context.messages.clone());
        self.agent.replace_messages(messages).await;
        if let Some(model) = context.as_ref().and_then(|context| context.model.clone()) {
            self.agent.set_model(model).await;
        }
        if let Some(thinking_level) = context.as_ref().and_then(|context| context.thinking_level) {
            self.agent.set_thinking_level(thinking_level).await;
        }
        self.agent
            .set_session_id(Some(session.header().session_id.to_string()))
            .await;
        *self.session.lock().await = session;
    }

    pub async fn set_model(&self, model: Model) -> HarnessResult<()> {
        self.agent.set_model(model.clone()).await;
        self.session
            .lock()
            .await
            .append(SessionEntryKind::ModelChange { model });
        Ok(())
    }

    pub async fn set_thinking_level(&self, thinking_level: ThinkingLevel) -> HarnessResult<()> {
        self.agent.set_thinking_level(thinking_level).await;
        self.session
            .lock()
            .await
            .append(SessionEntryKind::ThinkingLevelChange { thinking_level });
        Ok(())
    }

    /// Record the desired prompt state (base text plus ordered sections).
    /// Repeated calls coalesce; the next `prompt` flushes one delta `System`
    /// message carrying only what changed.
    pub async fn sync_system(&self, base: String, sections: Vec<PromptSection>) {
        *self.pending_system.lock().await = Some((base, sections));
    }

    /// Merge `before_agent_start` section edits into the pending prompt state:
    /// same-name sections are replaced, new names appended in order.
    async fn merge_section_edits(&self, edits: Vec<PromptSection>) {
        if edits.is_empty() {
            return;
        }
        let mut pending = self.pending_system.lock().await;
        let (_, sections) = pending.get_or_insert_with(|| {
            (
                self.system_prompt_fallback.clone().unwrap_or_default(),
                Vec::new(),
            )
        });
        for edit in edits {
            if let Some(existing) = sections
                .iter_mut()
                .find(|section| section.name == edit.name)
            {
                *existing = edit;
            } else {
                sections.push(edit);
            }
        }
    }

    /// Flush pending prompt/tool state into the transcript. Without a `System`
    /// message a full `System` is appended; otherwise a single delta `System`
    /// carrying changed sections and tool declarations is appended. The
    /// message goes to both the agent context and the session, before the next
    /// user message.
    async fn flush_system(&self) -> HarnessResult<()> {
        let pending = self.pending_system.lock().await.clone();
        let desired_tools: Vec<oino_types::ToolDeclaration> = self
            .tools
            .lock()
            .await
            .values()
            .map(|tool| tool.definition())
            .collect();
        let messages = self.agent.messages().await;
        let has_system = messages
            .iter()
            .any(|message| matches!(message, Message::System { .. }));
        if !has_system {
            let (base, sections) = pending.unwrap_or_else(|| {
                (
                    self.system_prompt_fallback.clone().unwrap_or_default(),
                    Vec::new(),
                )
            });
            let system = Message::System {
                id: oino_types::OinoId::new_v4(),
                content: base,
                sections,
                tools_added: desired_tools,
                tools_removed: Vec::new(),
            };
            self.agent.append_transcript_message(system.clone()).await;
            self.session.lock().await.append_message(system);
            return Ok(());
        }
        let Some(replayed) = current_system(&messages) else {
            return Ok(());
        };
        let Message::System {
            content: replayed_content,
            sections: replayed_sections,
            ..
        } = replayed
        else {
            return Ok(());
        };
        let (base, desired_sections) =
            pending.unwrap_or_else(|| (replayed_content.clone(), replayed_sections.clone()));
        // `content` is append-only in pi semantics, so a changed base is
        // carried as the pseudo-section `oino/base` with the new base text.
        let mut effective: Vec<PromptSection> = Vec::new();
        if base != replayed_content {
            effective.push(PromptSection {
                name: "oino/base".into(),
                text: Some(base),
            });
        }
        effective.extend(
            desired_sections
                .into_iter()
                .filter(|section| section.text.is_some()),
        );
        let replayed_by_name: BTreeMap<&str, Option<&str>> = replayed_sections
            .iter()
            .map(|section| (section.name.as_str(), section.text.as_deref()))
            .collect();
        let mut delta_sections: Vec<PromptSection> = effective
            .iter()
            .filter(|section| {
                replayed_by_name
                    .get(section.name.as_str())
                    .copied()
                    .flatten()
                    != section.text.as_deref()
                    || !replayed_by_name.contains_key(section.name.as_str())
            })
            .cloned()
            .collect();
        let effective_names: std::collections::BTreeSet<&str> = effective
            .iter()
            .map(|section| section.name.as_str())
            .collect();
        for section in &replayed_sections {
            if !effective_names.contains(section.name.as_str()) {
                delta_sections.push(PromptSection {
                    name: section.name.clone(),
                    text: None,
                });
            }
        }
        let replayed_tools = current_tools(&messages);
        let replayed_canonical: BTreeMap<String, String> = replayed_tools
            .iter()
            .map(|tool| (tool.name.clone(), tool_declaration_canonical(tool)))
            .collect();
        let desired_canonical: BTreeMap<String, String> = desired_tools
            .iter()
            .map(|tool| (tool.name.clone(), tool_declaration_canonical(tool)))
            .collect();
        let tools_added: Vec<oino_types::ToolDeclaration> = desired_tools
            .iter()
            .filter(|tool| replayed_canonical.get(&tool.name) != desired_canonical.get(&tool.name))
            .cloned()
            .collect();
        let tools_removed: Vec<String> = replayed_tools
            .iter()
            .filter(|tool| desired_canonical.get(&tool.name) != replayed_canonical.get(&tool.name))
            .map(|tool| tool.name.clone())
            .collect();
        if delta_sections.is_empty() && tools_added.is_empty() && tools_removed.is_empty() {
            return Ok(());
        }
        let delta = Message::System {
            id: oino_types::OinoId::new_v4(),
            content: String::new(),
            sections: delta_sections,
            tools_added,
            tools_removed,
        };
        self.agent.append_transcript_message(delta.clone()).await;
        self.session.lock().await.append_message(delta);
        Ok(())
    }

    pub async fn set_session_title(&self, title: impl Into<String>) -> HarnessResult<()> {
        append_session_title(&self.session, title.into()).await;
        Ok(())
    }

    pub fn session_title_setter(
        &self,
    ) -> Arc<dyn Fn(String, bool) -> BoxFuture<'static, LoopResult<()>> + Send + Sync> {
        let session = Arc::clone(&self.session);
        Arc::new(move |title, override_existing| {
            let session = Arc::clone(&session);
            Box::pin(async move {
                let mut session = session.lock().await;
                if has_explicit_session_title(&session) && !override_existing {
                    return Err(LoopError::Tool(
                        "session title has already been set; pass `override: true` to set it again"
                            .into(),
                    ));
                }
                session.append(SessionEntryKind::SessionInfo {
                    name: Some(title),
                    cwd: None,
                });
                Ok(())
            })
        })
    }

    pub async fn session_title(&self) -> String {
        self.session.lock().await.get_session_name()
    }

    pub async fn tool_names(&self) -> Vec<String> {
        self.tools.lock().await.keys().cloned().collect()
    }

    pub async fn set_tools(&self, tools: BTreeMap<String, Arc<dyn Tool>>) {
        self.agent.set_tools(tools.clone()).await;
        *self.tools.lock().await = tools;
    }
    pub async fn set_resources(&self, resources: Vec<String>) {
        *self.resources.lock().await = resources;
    }
    pub async fn resources(&self) -> Vec<String> {
        self.resources.lock().await.clone()
    }
    pub async fn resolve_auth(&self, provider: impl Into<String>) -> LoopResult<Option<String>> {
        if let Some(resolver) = &self.auth_resolver {
            resolver(provider.into()).await
        } else {
            Ok(None)
        }
    }
    pub async fn build_context(&self) -> HarnessResult<Vec<Message>> {
        Ok(self.session.lock().await.build_session_context()?.messages)
    }
    pub async fn save_session_jsonl(&self, path: impl AsRef<std::path::Path>) -> HarnessResult<()> {
        self.session.lock().await.save_jsonl(path).await?;
        Ok(())
    }
    /// The current replayed system prompt text, or the pending prompt state
    /// when no `System` message has been appended yet.
    pub async fn get_system_prompt(&self) -> Option<String> {
        let messages = self.agent.messages().await;
        if let Some(system) = current_system(&messages) {
            let prompt = render_system_prompt(&system);
            if !prompt.is_empty() {
                return Some(prompt);
            }
        }
        if let Some((base, sections)) = self.pending_system.lock().await.clone() {
            let prompt = render_system_prompt(&Message::System {
                id: oino_types::OinoId::nil(),
                content: base,
                sections,
                tools_added: Vec::new(),
                tools_removed: Vec::new(),
            });
            if !prompt.is_empty() {
                return Some(prompt);
            }
        }
        self.system_prompt_fallback.clone()
    }

    /// Cache telemetry/warming handle (fingerprints, hit rates, warm config).
    #[must_use]
    pub fn cache_tracker(&self) -> Arc<CacheTracker> {
        Arc::clone(&self.cache_tracker)
    }

    /// `/session` cache lines: last hit rate, last miss diagnosis, next
    /// warming decision.
    #[must_use]
    pub fn cache_status(&self) -> CacheStatus {
        self.cache_tracker.status()
    }

    pub async fn inspect_full_prompt(&self) -> HarnessResult<FullPromptInspect> {
        let state = self.agent.state().await;
        let mut messages = state.messages.clone();
        messages.push(Message::user_text("<next user message>"));
        let messages = self
            .hooks
            .run_context(messages)
            .await
            .map_err(oino_agent::AgentError::Loop)?;
        let mut content = String::new();
        content.push_str("# Provider Request Preview\n\n");
        content.push_str(&format!("Model: {}\n", state.model.identifier()));
        content.push_str(&format!("Thinking: {:?}\n", state.thinking_level));
        content.push_str("\n# System Prompt\n\n");
        let replayed = current_system(&state.messages);
        let prompt_text = replayed
            .as_ref()
            .map(render_system_prompt)
            .filter(|prompt| !prompt.trim().is_empty())
            .or_else(|| {
                self.system_prompt_fallback
                    .clone()
                    .filter(|prompt| !prompt.trim().is_empty())
            })
            .unwrap_or_else(|| "<none>".into());
        content.push_str(&prompt_text);
        let mut seen_system = false;
        for message in &state.messages {
            let Message::System {
                sections,
                tools_added,
                tools_removed,
                ..
            } = message
            else {
                continue;
            };
            if !seen_system {
                seen_system = true;
                continue;
            }
            let mut changed: Vec<String> = sections
                .iter()
                .map(|section| match &section.text {
                    Some(_) => section.name.clone(),
                    None => format!("-{}", section.name),
                })
                .collect();
            changed.extend(tools_added.iter().map(|tool| format!("+{}", tool.name)));
            changed.extend(tools_removed.iter().map(|name| format!("-{name}")));
            if !changed.is_empty() {
                content.push_str("\n\n# Prompt deltas\n\n");
                content.push_str(&format!("- {}", changed.join(", ")));
            }
        }
        content.push_str("\n\n# Messages\n");
        for (index, message) in messages.iter().enumerate() {
            content.push('\n');
            content.push_str(&format_message_for_inspect(index, message));
        }
        content.push_str("\n# Tools\n");
        let transcript_tools = current_tools(&state.messages);
        let tools = if transcript_tools.is_empty() {
            state.tools.clone()
        } else {
            transcript_tools
        };
        if tools.is_empty() {
            content.push_str("\n<none>\n");
        } else {
            for tool in &tools {
                content.push('\n');
                content.push_str(&format!("## {}\n\n", tool.name));
                content.push_str(&tool.description);
                content.push_str("\n\nInput schema:\n\n```json\n");
                content.push_str(
                    &serde_json::to_string_pretty(&tool.input_schema)
                        .unwrap_or_else(|_| tool.input_schema.to_string()),
                );
                content.push_str("\n```\n");
            }
        }
        let token_count = inspect_token_count(&content);
        Ok(FullPromptInspect {
            content,
            token_count,
        })
    }

    /// Append output messages not already present on the active branch,
    /// preserving order. Id-based so messages appended outside the loop (e.g.
    /// `System` deltas) never shift the offset.
    async fn persist_messages(&self, messages: &[Message]) {
        let mut session = self.session.lock().await;
        let mut existing: std::collections::BTreeSet<oino_types::OinoId> =
            std::collections::BTreeSet::new();
        for entry in session
            .get_branch(session.get_leaf_id())
            .unwrap_or_default()
            .iter()
        {
            existing.insert(entry.id);
            if let SessionEntryKind::Message { message }
            | SessionEntryKind::CustomMessage { message } = &entry.kind
            {
                existing.insert(message.id());
            }
        }
        for message in messages {
            // Summary messages are projections of session entries, not real
            // transcript messages; never persist them here.
            if matches!(
                message,
                Message::CompactionSummary { .. } | Message::BranchSummary { .. }
            ) || existing.contains(&message.id())
            {
                continue;
            }
            session.append_message(message.clone());
        }
    }
}

fn format_message_for_inspect(index: usize, message: &Message) -> String {
    match message {
        Message::System { content, .. } => format!(
            "## Message {}: system\n\n{}\n",
            index.saturating_add(1),
            content
        ),
        Message::User { content, .. } => format!(
            "## Message {}: user\n\n{}\n",
            index.saturating_add(1),
            format_content_blocks_for_inspect(content)
        ),
        Message::Assistant { content, .. } => format!(
            "## Message {}: assistant\n\n{}\n",
            index.saturating_add(1),
            format_content_blocks_for_inspect(content)
        ),
        Message::ToolResult {
            tool_name, content, ..
        } => format!(
            "## Message {}: tool:{tool_name}\n\n{}\n",
            index.saturating_add(1),
            format_content_blocks_for_inspect(content)
        ),
        Message::Custom { name, payload, .. } => format!(
            "## Message {}: custom:{name}\n\n```json\n{}\n```\n",
            index.saturating_add(1),
            serde_json::to_string_pretty(payload).unwrap_or_else(|_| payload.to_string())
        ),
        Message::CompactionSummary { summary, .. } => format!(
            "## Message {}: compaction\n\n{}\n",
            index.saturating_add(1),
            summary
        ),
        Message::BranchSummary { summary, .. } => format!(
            "## Message {}: branch\n\n{}\n",
            index.saturating_add(1),
            summary
        ),
    }
}

fn format_content_blocks_for_inspect(blocks: &[ContentBlock]) -> String {
    if blocks.is_empty() {
        return "<empty>".into();
    }
    blocks
        .iter()
        .map(|block| match block {
            ContentBlock::Text { text, .. } => text.clone(),
            ContentBlock::Image { media_type, data } => {
                format!("<image:{media_type}; {} bytes>", data.len())
            }
            ContentBlock::Thinking { text, redacted, .. } => {
                if *redacted {
                    format!("<thinking redacted; {} chars>", text.chars().count())
                } else {
                    format!("<thinking>\n{text}")
                }
            }
            ContentBlock::ToolCall {
                id,
                name,
                arguments,
                ..
            } => format!(
                "<tool-call:{name} id={id}>\n```json\n{}\n```",
                serde_json::to_string_pretty(arguments).unwrap_or_else(|_| arguments.to_string())
            ),
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn inspect_token_count(content: &str) -> usize {
    content.split_whitespace().count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::{
        AbortSignal, FauxStream, ToolCall, ToolDefinition, ToolResult, ToolUpdateCallback,
        VecEventSink,
    };
    use oino_session::SessionHeader;
    use oino_types::{AssistantStreamEvent, StopReason};
    use std::path::PathBuf;

    fn harness() -> Harness {
        let session = SessionManager::new(SessionHeader::new("h", PathBuf::from("/tmp")));
        let stream = Arc::new(FauxStream::once(vec![
            AssistantStreamEvent::TextDelta { delta: "ok".into() },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            },
        ]));
        Harness::new(HarnessConfig::new(
            Model::new("test", "faux"),
            stream,
            session,
        ))
    }

    struct InspectTool;

    #[async_trait::async_trait]
    impl Tool for InspectTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "inspect_tool".into(),
                description: "Tool visible in inspect snapshots".into(),
                input_schema: serde_json::json!({"type": "object"}),
            }
        }

        async fn execute(
            &self,
            call: ToolCall,
            _updates: ToolUpdateCallback,
            _signal: AbortSignal,
        ) -> LoopResult<ToolResult> {
            Ok(ToolResult::text(&call, "ok"))
        }
    }

    #[tokio::test]
    async fn inspect_full_prompt_includes_system_placeholder_and_tools() {
        let session = SessionManager::new(SessionHeader::new("h", PathBuf::from("/tmp")));
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }]));
        let mut cfg = HarnessConfig::new(Model::new("test", "faux"), stream, session);
        cfg.system_prompt = Some("system prelude".into());
        cfg.tools
            .insert("inspect_tool".into(), Arc::new(InspectTool));
        let h = Harness::new(cfg);

        let snapshot = match h.inspect_full_prompt().await {
            Ok(snapshot) => snapshot,
            Err(err) => panic!("inspect prompt failed: {err}"),
        };

        assert!(snapshot.content.contains("system prelude"));
        assert!(snapshot.content.contains("<next user message>"));
        assert!(snapshot.content.contains("## inspect_tool"));
        assert!(snapshot
            .content
            .contains("Tool visible in inspect snapshots"));
        assert_eq!(
            snapshot.token_count,
            snapshot.content.split_whitespace().count()
        );
    }

    #[tokio::test]
    async fn session_title_tool_setter_requires_override_after_first_title() {
        let h = harness();
        let setter = h.session_title_setter();

        let first = setter("Design Review".into(), false).await;
        assert!(first.is_ok());
        assert_eq!(h.session_title().await, "Design Review");

        let second = setter("Renamed".into(), false).await;
        assert!(matches!(second, Err(LoopError::Tool(message)) if message.contains("override")));
        assert_eq!(h.session_title().await, "Design Review");

        let third = setter("Renamed".into(), true).await;
        assert!(third.is_ok());
        assert_eq!(h.session_title().await, "Renamed");
    }

    #[tokio::test]
    async fn notification_hooks_are_ordered() {
        let h = harness();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s1 = Arc::clone(&seen);
        h.hooks()
            .on_notification(
                NotificationHook::AgentStart,
                Arc::new(move |_event| {
                    let seen = Arc::clone(&s1);
                    Box::pin(async move {
                        seen.lock().await.push(1);
                    })
                }),
            )
            .await;
        let s2 = Arc::clone(&seen);
        h.hooks()
            .on_notification(
                NotificationHook::AgentStart,
                Arc::new(move |_event| {
                    let seen = Arc::clone(&s2);
                    Box::pin(async move {
                        seen.lock().await.push(2);
                    })
                }),
            )
            .await;
        let result = h.prompt(Message::user_text("hi")).await;
        assert!(result.is_ok());
        assert_eq!(*seen.lock().await, vec![1, 2]);
    }

    #[tokio::test]
    async fn context_hook_can_mutate_messages() {
        let h = harness();
        h.hooks()
            .on_context(Arc::new(|mut messages| {
                Box::pin(async move {
                    messages.push(Message::user_text("injected"));
                    Ok(messages)
                })
            }))
            .await;
        let result = h.prompt(Message::user_text("hi")).await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn provider_string_hooks_are_deterministic() {
        let h = harness();
        h.hooks()
            .on_before_provider_request(Arc::new(|value| {
                Box::pin(async move { Ok(format!("{value}-a")) })
            }))
            .await;
        h.hooks()
            .on_before_provider_request(Arc::new(|value| {
                Box::pin(async move { Ok(format!("{value}-b")) })
            }))
            .await;
        let value = h.hooks().mutate_before_provider_request("x".into()).await;
        let value = match value {
            Ok(value) => value,
            Err(err) => panic!("hook failed: {err}"),
        };
        assert_eq!(value, "x-a-b");
    }

    #[tokio::test]
    async fn prompt_runs_provider_hooks() {
        let h = harness();
        let seen = Arc::new(Mutex::new(Vec::new()));
        let before_request = Arc::clone(&seen);
        h.hooks()
            .on_before_provider_request(Arc::new(move |value| {
                let seen = Arc::clone(&before_request);
                Box::pin(async move {
                    seen.lock().await.push(format!("request:{value}"));
                    Ok(value)
                })
            }))
            .await;
        let before_payload = Arc::clone(&seen);
        h.hooks()
            .on_before_provider_payload(Arc::new(move |value| {
                let seen = Arc::clone(&before_payload);
                Box::pin(async move {
                    seen.lock().await.push(format!("payload:{value}"));
                    Ok(value)
                })
            }))
            .await;
        let after_response = Arc::clone(&seen);
        h.hooks()
            .on_after_provider_response(Arc::new(move |value| {
                let seen = Arc::clone(&after_response);
                Box::pin(async move {
                    seen.lock().await.push(format!("response:{value}"));
                    Ok(value)
                })
            }))
            .await;
        let result = h.prompt(Message::user_text("hi")).await;
        assert!(result.is_ok());
        assert_eq!(
            *seen.lock().await,
            vec![
                "request:request".to_string(),
                "payload:payload".to_string(),
                "response:response".to_string(),
            ]
        );
    }

    #[tokio::test]
    async fn auth_resolver_boundary_is_exposed() {
        let session = SessionManager::new(SessionHeader::new("h", PathBuf::from("/tmp")));
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }]));
        let mut cfg = HarnessConfig::new(Model::new("test", "faux"), stream, session);
        cfg.auth_resolver = Some(Arc::new(|provider| {
            Box::pin(async move { Ok(Some(format!("token-for-{provider}"))) })
        }));
        let h = Harness::new(cfg);
        let token = h.resolve_auth("faux").await;
        match token {
            Ok(Some(token)) => assert_eq!(token, "token-for-faux"),
            other => panic!("unexpected auth result: {other:?}"),
        }
    }

    #[tokio::test]
    async fn prompt_persists_save_points() {
        let sink = VecEventSink::new();
        let session = SessionManager::new(SessionHeader::new("h", PathBuf::from("/tmp")));
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }]));
        let mut cfg = HarnessConfig::new(Model::new("test", "faux"), stream, session);
        cfg.event_sink = Arc::new(sink.clone());
        let h = Harness::new(cfg);
        let result = h.prompt(Message::user_text("hi")).await;
        assert!(result.is_ok());
        assert!(!h.build_context().await.unwrap_or_default().is_empty());
    }

    #[tokio::test]
    async fn replace_session_updates_agent_and_session_context() {
        let h = harness();
        let result = h.prompt(Message::user_text("hi")).await;
        assert!(result.is_ok());
        assert!(!h.build_context().await.unwrap_or_default().is_empty());

        let mut replacement = SessionManager::new(SessionHeader::new("new", PathBuf::from("/tmp")));
        replacement.append_model(Model::new("test", "replacement"));
        replacement.append_thinking_level(ThinkingLevel::High);
        let loaded_message = Message::user_text("loaded");
        replacement.append_message(loaded_message.clone());
        h.replace_session(replacement).await;

        let context = h.build_context().await.unwrap_or_default();
        assert_eq!(context, vec![loaded_message]);
        let agent_state = h.agent.state().await;
        assert_eq!(agent_state.messages, context);
        assert_eq!(agent_state.model, Model::new("test", "replacement"));
        assert_eq!(agent_state.thinking_level, ThinkingLevel::High);
    }

    fn prompt_parts() -> (String, Vec<oino_types::PromptSection>) {
        (
            "policy base".into(),
            vec![
                oino_types::PromptSection {
                    name: "project/AGENT.md".into(),
                    text: Some("rules".into()),
                },
                oino_types::PromptSection {
                    name: "oino/cwd".into(),
                    text: Some("/tmp".into()),
                },
            ],
        )
    }

    fn system_messages(messages: &[Message]) -> Vec<&Message> {
        messages
            .iter()
            .filter(|message| matches!(message, Message::System { .. }))
            .collect()
    }

    #[tokio::test]
    async fn sync_system_persists_full_then_noop_delta() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base.clone(), sections.clone()).await;
        h.sync_system(base, sections).await; // coalesces
        let first = h.prompt(Message::user_text("one")).await;
        assert!(first.is_ok());
        let second = h.prompt(Message::user_text("two")).await;
        assert!(second.is_ok());
        let entries = h.all_session_entries().await;
        let system_count = entries
            .iter()
            .filter(|entry| matches!(&entry.kind, SessionEntryKind::Message { message } if matches!(message, Message::System { .. })))
            .count();
        assert_eq!(system_count, 1);
        // The first prompt carries a full System before the user message.
        let first_system = entries
            .iter()
            .find_map(|entry| match &entry.kind {
                SessionEntryKind::Message {
                    message: system @ Message::System { .. },
                } => Some(system.clone()),
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing system message"));
        let Message::System {
            content,
            sections,
            tools_added,
            ..
        } = first_system
        else {
            panic!("expected system");
        };
        assert_eq!(content, "policy base");
        assert_eq!(sections.len(), 2);
        assert!(tools_added.is_empty());
    }

    #[tokio::test]
    async fn section_change_yields_single_section_delta() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base.clone(), sections.clone()).await;
        assert!(h.prompt(Message::user_text("one")).await.is_ok());
        let changed = vec![
            oino_types::PromptSection {
                name: "project/AGENT.md".into(),
                text: Some("rules v2".into()),
            },
            sections[1].clone(),
        ];
        h.sync_system(base, changed).await;
        assert!(h.prompt(Message::user_text("two")).await.is_ok());
        let messages = h.build_context().await.unwrap_or_default();
        let systems = system_messages(&messages);
        assert_eq!(systems.len(), 2);
        let Message::System {
            sections,
            tools_added,
            tools_removed,
            ..
        } = systems[1]
        else {
            panic!("expected system delta");
        };
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "project/AGENT.md");
        assert_eq!(sections[0].text.as_deref(), Some("rules v2"));
        assert!(tools_added.is_empty());
        assert!(tools_removed.is_empty());
    }

    #[tokio::test]
    async fn tool_change_yields_tools_only_delta() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base.clone(), sections.clone()).await;
        assert!(h.prompt(Message::user_text("one")).await.is_ok());
        let mut tools = std::collections::BTreeMap::new();
        tools.insert(
            "inspect_tool".into(),
            Arc::new(InspectTool) as Arc<dyn Tool>,
        );
        h.set_tools(tools).await;
        assert!(h.prompt(Message::user_text("two")).await.is_ok());
        let messages = h.build_context().await.unwrap_or_default();
        let systems = system_messages(&messages);
        assert_eq!(systems.len(), 2);
        let Message::System {
            sections,
            tools_added,
            tools_removed,
            ..
        } = systems[1]
        else {
            panic!("expected system delta");
        };
        assert!(sections.is_empty());
        assert_eq!(tools_added.len(), 1);
        assert_eq!(tools_added[0].name, "inspect_tool");
        assert!(tools_removed.is_empty());
    }

    #[tokio::test]
    async fn blank_session_stays_lazy_without_prompt() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base, sections).await;
        assert!(h.all_session_entries().await.is_empty());
    }

    #[tokio::test]
    async fn v1_session_resume_appends_full_system_at_leaf() {
        // v1-style session: messages but no System entry.
        let mut session = SessionManager::new(SessionHeader::new("v1", PathBuf::from("/tmp")));
        let prior = session.append_message(Message::user_text("prior"));
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }]));
        let h = Harness::new(HarnessConfig::new(
            Model::new("test", "faux"),
            stream,
            session,
        ));
        let (base, sections) = prompt_parts();
        h.sync_system(base.clone(), sections.clone()).await;
        assert!(h.prompt(Message::user_text("hi")).await.is_ok());
        let branch = h
            .active_branch_entries()
            .await
            .unwrap_or_else(|err| panic!("branch failed: {err}"));
        // The System entry is a child of the previous leaf (nothing at root).
        let system_index = branch
            .iter()
            .position(|entry| matches!(&entry.kind, SessionEntryKind::Message { message } if matches!(message, Message::System { .. })))
            .unwrap_or_else(|| panic!("no system entry appended"));
        assert_eq!(branch[system_index].parent, Some(prior));
        let Some(Message::System {
            content, sections, ..
        }) = (match &branch[system_index].kind {
            SessionEntryKind::Message { message } => Some(message.clone()),
            _ => None,
        })
        else {
            panic!("expected system");
        };
        assert_eq!(content, "policy base");
        assert_eq!(sections.len(), 2);
    }

    #[tokio::test]
    async fn prompt_text_is_byte_stable_across_builds() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base.clone(), sections.clone()).await;
        let first = h.get_system_prompt().await.unwrap_or_default();
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        let h2 = harness();
        h2.sync_system(base, sections).await;
        let second = h2.get_system_prompt().await.unwrap_or_default();
        assert_eq!(first, second);
        assert!(!first.is_empty());
    }

    #[tokio::test]
    async fn prompt_after_compaction_persists_new_messages() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base, sections).await;
        assert!(h.prompt(Message::user_text("one")).await.is_ok());
        // Retain-none compaction, then prompt again: the new messages must be
        // persisted even though message entries already exist.
        let messages = h
            .append_compaction("summary".into(), None, 10)
            .await
            .unwrap_or_else(|err| panic!("compaction failed: {err}"));
        assert!(messages
            .iter()
            .any(|message| matches!(message, Message::CompactionSummary { .. })));
        assert!(h.prompt(Message::user_text("two")).await.is_ok());
        let branch = h
            .active_branch_entries()
            .await
            .unwrap_or_else(|err| panic!("branch failed: {err}"));
        let texts: Vec<String> = branch
            .iter()
            .filter_map(|entry| match &entry.kind {
                SessionEntryKind::Message {
                    message: Message::User { content, .. },
                } => content.iter().find_map(|block| match block {
                    ContentBlock::Text { text, .. } => Some(text.clone()),
                    _ => None,
                }),
                _ => None,
            })
            .collect();
        assert!(texts.contains(&"two".to_string()));
    }

    #[tokio::test]
    async fn prompts_after_compaction_do_not_duplicate_summary_entries() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base, sections).await;
        assert!(h.prompt(Message::user_text("one")).await.is_ok());
        h.append_compaction("summary".into(), None, 10)
            .await
            .unwrap_or_else(|err| panic!("compaction failed: {err}"));
        assert!(h.prompt(Message::user_text("two")).await.is_ok());
        assert!(h.prompt(Message::user_text("three")).await.is_ok());
        let branch = h
            .active_branch_entries()
            .await
            .unwrap_or_else(|err| panic!("branch failed: {err}"));
        // No Message entry may wrap a CompactionSummary/BranchSummary.
        assert!(!branch.iter().any(|entry| matches!(
            &entry.kind,
            SessionEntryKind::Message {
                message: Message::CompactionSummary { .. } | Message::BranchSummary { .. }
            }
        )));
        let context = h
            .build_context()
            .await
            .unwrap_or_else(|err| panic!("context failed: {err}"));
        let summaries = context
            .iter()
            .filter(|message| matches!(message, Message::CompactionSummary { .. }))
            .count();
        assert_eq!(summaries, 1);
    }

    #[tokio::test]
    async fn turn_end_hook_appends_entries_and_can_continue() {
        let sink = VecEventSink::new();
        let session = SessionManager::new(SessionHeader::new("h", PathBuf::from("/tmp")));
        let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }]));
        let mut cfg = HarnessConfig::new(Model::new("test", "faux"), stream, session);
        cfg.event_sink = Arc::new(sink.clone());
        let h = Harness::new(cfg);
        let entry = Message::user_text("turn end entry");
        let entry_for_hook = entry.clone();
        let calls = Arc::new(std::sync::Mutex::new(0usize));
        let calls_hook = Arc::clone(&calls);
        h.hooks()
            .on_turn_end(Arc::new(move |_context| {
                let entry = entry_for_hook.clone();
                let calls = Arc::clone(&calls_hook);
                Box::pin(async move {
                    let mut locked = calls.lock().unwrap_or_else(|e| e.into_inner());
                    *locked += 1;
                    let continue_run = *locked == 1;
                    Ok(Some(TurnEndResult {
                        entries: if continue_run {
                            vec![entry]
                        } else {
                            Vec::new()
                        },
                        continue_run,
                    }))
                })
            }))
            .await;
        assert!(h.prompt(Message::user_text("hi")).await.is_ok());
        // Continue ran one extra turn.
        let turn_ends = sink
            .events()
            .await
            .iter()
            .filter(|event| matches!(event, AgentEvent::TurnEnd { .. }))
            .count();
        assert_eq!(turn_ends, 2);
        // The entry landed in agent context and session.
        assert!(h.build_context().await.unwrap_or_default().contains(&entry));
        let branch = h
            .active_branch_entries()
            .await
            .unwrap_or_else(|err| panic!("branch failed: {err}"));
        assert!(branch.iter().any(|e| matches!(
            &e.kind,
            SessionEntryKind::Message { message } if *message == entry
        )));
    }

    #[tokio::test]
    async fn before_agent_start_section_edits_flush_as_delta() {
        let h = harness();
        let (base, sections) = prompt_parts();
        h.sync_system(base, sections).await;
        assert!(h.prompt(Message::user_text("one")).await.is_ok());
        h.hooks()
            .on_before_agent_start(Arc::new(|| {
                Box::pin(async {
                    Ok(vec![oino_types::PromptSection {
                        name: "ext/edited".into(),
                        text: Some("from hook".into()),
                    }])
                })
            }))
            .await;
        assert!(h.prompt(Message::user_text("two")).await.is_ok());
        let messages = h.build_context().await.unwrap_or_default();
        let systems = system_messages(&messages);
        assert_eq!(systems.len(), 2);
        let Message::System { sections, .. } = systems[1] else {
            panic!("expected system delta");
        };
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].name, "ext/edited");
        assert_eq!(sections[0].text.as_deref(), Some("from hook"));
        // The replayed prompt now contains the hook section text.
        let prompt = h.get_system_prompt().await.unwrap_or_default();
        assert!(prompt.contains("from hook"));
    }

    /// Stream provider that records every request and always ends the turn.
    struct CaptureStream {
        requests: Arc<Mutex<Vec<oino_agent_loop::StreamRequest>>>,
    }

    #[async_trait::async_trait]
    impl StreamProvider for CaptureStream {
        async fn stream(
            &self,
            request: oino_agent_loop::StreamRequest,
            _signal: AbortSignal,
        ) -> LoopResult<Vec<AssistantStreamEvent>> {
            self.requests.lock().await.push(request);
            Ok(vec![AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                provider: None,
            }])
        }
    }

    #[tokio::test]
    async fn requests_are_prefix_stable_across_prompts_and_deltas() {
        let session = SessionManager::new(SessionHeader::new("h", PathBuf::from("/tmp")));
        let stream = Arc::new(CaptureStream {
            requests: Arc::new(Mutex::new(Vec::new())),
        });
        let requests = Arc::clone(&stream.requests);
        let h = Harness::new(HarnessConfig::new(
            Model::new("test", "faux"),
            stream,
            session,
        ));
        let (base, sections) = prompt_parts();
        h.sync_system(base.clone(), sections.clone()).await;
        for i in 0..3 {
            assert!(h
                .prompt(Message::user_text(format!("prompt-{i}")))
                .await
                .is_ok());
        }
        let changed = vec![
            oino_types::PromptSection {
                name: "project/AGENT.md".into(),
                text: Some("rules v2".into()),
            },
            sections[1].clone(),
        ];
        h.sync_system(base, changed).await;
        for i in 3..5 {
            assert!(h
                .prompt(Message::user_text(format!("prompt-{i}")))
                .await
                .is_ok());
        }
        let requests = requests.lock().await;
        assert_eq!(requests.len(), 5);
        // Every request's transcript is an exact prefix of the next one.
        for pair in requests.windows(2) {
            let earlier = pair[0].transcript.messages();
            let later = pair[1].transcript.messages();
            assert!(later.len() > earlier.len());
            assert_eq!(&later[..earlier.len()], earlier);
        }
        // Between prompt 3 and 4 the only new System is a one-section delta.
        let third = requests[2].transcript.messages().len();
        let inserted = &requests[3].transcript.messages()[third..];
        let deltas: Vec<&Message> = inserted
            .iter()
            .filter(|message| matches!(message, Message::System { .. }))
            .collect();
        assert_eq!(deltas.len(), 1);
        let Message::System {
            sections: delta_sections,
            tools_added,
            tools_removed,
            ..
        } = deltas[0]
        else {
            panic!("expected delta");
        };
        assert_eq!(delta_sections.len(), 1);
        assert_eq!(delta_sections[0].name, "project/AGENT.md");
        assert!(tools_added.is_empty() && tools_removed.is_empty());
        // Rendered prompt byte-identical for requests 1-3, changes only in the
        // edited section for 4-5.
        let rendered: Vec<String> = requests
            .iter()
            .map(|request| {
                oino_types::transcript::current_system(request.transcript.messages())
                    .map(|system| oino_types::transcript::render_system_prompt(&system))
                    .unwrap_or_default()
            })
            .collect();
        assert!(rendered[0] == rendered[1] && rendered[1] == rendered[2]);
        assert!(rendered[3] == rendered[4]);
        assert_ne!(rendered[2], rendered[3]);
        assert!(rendered[3].contains("rules v2"));
        let without_section: Vec<String> = rendered
            .iter()
            .map(|prompt| prompt.replace("rules v2", "rules"))
            .collect();
        assert_eq!(without_section[2], without_section[4]);
    }
}
