#![doc = r#"High-level Oino runtime harness.

The harness binds the loop, stateful agent, sessions, execution environment, resources,
provider/auth boundaries, and deterministic typed hooks. It is intentionally headless: TUI,
real providers, MCP, memory databases, and dynamic plugin ABIs belong in later layers.
"#]
#![forbid(unsafe_code)]

use oino_agent::Agent;
use oino_agent_loop::{
    AbortSignal, AfterToolCall, AgentEvent, AgentLoopConfig, BeforeToolCall, BeforeToolCallResult,
    BoxFuture, EventSink, LoopResult, StreamEventSink, StreamProvider, StreamRequest, Tool,
    ToolCall, ToolResult, TransformContext,
};
use oino_env::{ExecutionEnv, LocalExecutionEnv};
use oino_session::{SessionEntryKind, SessionManager};
use oino_types::{AssistantStreamEvent, Message, Model, ThinkingLevel};
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
pub type BeforeAgentStartHandler =
    Arc<dyn Fn() -> BoxFuture<'static, LoopResult<()>> + Send + Sync>;
pub type ContextHandler =
    Arc<dyn Fn(Vec<Message>) -> BoxFuture<'static, LoopResult<Vec<Message>>> + Send + Sync>;
pub type BeforeToolCallHandler =
    Arc<dyn Fn(ToolCall) -> BoxFuture<'static, LoopResult<BeforeToolCallResult>> + Send + Sync>;
pub type AfterToolCallHandler =
    Arc<dyn Fn(ToolResult) -> BoxFuture<'static, LoopResult<ToolResult>> + Send + Sync>;
pub type StringMutationHandler =
    Arc<dyn Fn(String) -> BoxFuture<'static, LoopResult<String>> + Send + Sync>;
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

    async fn run_before_agent_start(&self) -> LoopResult<()> {
        for handler in self.before_agent_start.lock().await.clone() {
            handler().await?;
        }
        Ok(())
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
                blocked @ BeforeToolCallResult::Block(_) => return Ok(blocked),
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

fn hook_for_event(event: &AgentEvent) -> Option<NotificationHook> {
    match event {
        AgentEvent::AgentStart { .. } => Some(NotificationHook::AgentStart),
        AgentEvent::TurnStart { .. } => Some(NotificationHook::TurnStart),
        AgentEvent::MessageStart { .. } => Some(NotificationHook::MessageStart),
        AgentEvent::MessageUpdate { .. } => Some(NotificationHook::MessageUpdate),
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
        self.inner.stream_events(request, signal, sink).await?;
        let _response_marker = self
            .hooks
            .mutate_after_provider_response("response".into())
            .await?;
        Ok(())
    }
}

struct HookEventSink {
    registry: HookRegistry,
    inner: Arc<dyn EventSink>,
}
#[async_trait::async_trait]
impl EventSink for HookEventSink {
    async fn emit(&self, event: AgentEvent) -> LoopResult<()> {
        self.inner.emit(event.clone()).await?;
        self.registry.notify(event).await;
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
        }
    }
}

pub struct Harness {
    agent: Agent,
    session: Arc<Mutex<SessionManager>>,
    hooks: HookRegistry,
    env: Arc<dyn ExecutionEnv>,
    resources: Arc<Mutex<Vec<String>>>,
    tools: Arc<Mutex<BTreeMap<String, Arc<dyn Tool>>>>,
    auth_resolver: Option<AuthResolver>,
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
        let stream = Arc::new(HookedStreamProvider {
            inner: Arc::clone(&config.stream),
            hooks: hooks.clone(),
        }) as Arc<dyn StreamProvider>;
        let mut loop_config = AgentLoopConfig::new(initial_model, stream);
        loop_config.thinking_level = initial_thinking_level;
        loop_config.system_prompt = config.system_prompt;
        loop_config.tools = config.tools.clone();
        loop_config.event_sink = Arc::new(HookEventSink {
            registry: hooks.clone(),
            inner: config.event_sink,
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
        Self {
            agent: Agent::new_with_messages(loop_config, initial_messages),
            session: Arc::new(Mutex::new(config.session)),
            hooks,
            env: config.env,
            resources: Arc::new(Mutex::new(config.resources)),
            tools: Arc::new(Mutex::new(config.tools)),
            auth_resolver: config.auth_resolver,
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
        self.hooks
            .run_before_agent_start()
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
        let summary = self
            .hooks
            .mutate_before_compaction(summary)
            .await
            .map_err(oino_agent::AgentError::Loop)?;
        self.session
            .lock()
            .await
            .append_compaction(summary, Vec::new());
        Ok(())
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
        self.agent.reset().await;
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
    pub async fn get_system_prompt(&self) -> Option<String> {
        self.agent.state().await.system_prompt
    }

    async fn persist_messages(&self, messages: &[Message]) {
        let mut session = self.session.lock().await;
        let existing = session
            .get_entries()
            .into_iter()
            .filter(|entry| matches!(entry.kind, SessionEntryKind::Message { .. }))
            .count();
        for message in messages.iter().skip(existing) {
            session.append_message(message.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::{FauxStream, VecEventSink};
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
    async fn replace_session_clears_agent_and_session_context() {
        let h = harness();
        let result = h.prompt(Message::user_text("hi")).await;
        assert!(result.is_ok());
        assert!(!h.build_context().await.unwrap_or_default().is_empty());

        let replacement = SessionManager::new(SessionHeader::new("new", PathBuf::from("/tmp")));
        h.replace_session(replacement).await;

        assert!(h.build_context().await.unwrap_or_default().is_empty());
        assert!(h.agent.messages().await.is_empty());
    }
}
