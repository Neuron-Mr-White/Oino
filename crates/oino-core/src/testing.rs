//! Test doubles (feature `testing`): a scripted faux provider component, a closure-driven
//! fake extension, and a one-call in-memory session builder. No wasmtime involved
//! (runtime.md §17).

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use futures::future::BoxFuture;
use futures::stream::{self, BoxStream};
use indexmap::IndexMap;
use oino_agent::{AgentToolResult, ToolInvocation, UpdateSink};
use oino_ai::{
    AssistantContent, AssistantMessage, DoneReason, ErrorReason, JsonObject, JsonValue, Model,
    ProviderEvent, StopReason, ThinkingLevel, ToolCall, Usage,
};
use oino_models::{ModelRegistry, ProviderRegistration};
use oino_resources::{ResourceLoader, ResourceLoaderOptions};
use oino_session::{CreateOptions, SessionManager};
use oino_settings::SettingsManager;
use serde_json::json;
use tokio_util::sync::CancellationToken;

use crate::extensions::api::{
    CallContext, ExtensionCallError, ExtensionInstance, ExtensionKind, ExtensionMode,
    ProviderInstance, Registration,
};
use crate::extensions::runner::{ExtensionSet, LoadedExtension};
use crate::provider::ProviderStream;
use crate::session::{AgentSession, SessionConfig, ToolEntry};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// API implemented by [`FauxProvider`].
pub const FAUX_API: &str = "faux";

/// One scripted assistant response.
#[derive(Clone, Debug, PartialEq)]
pub enum FauxResponse {
    /// Text answer (`stop`).
    Text(String),
    /// Tool calls (`toolUse`): `(id, name, arguments)`.
    ToolCalls(Vec<(String, String, JsonObject)>),
    /// Provider error with this message.
    Error(String),
    /// Full control over content, stop reason and usage.
    Message {
        /// Content blocks.
        content: Vec<AssistantContent>,
        /// Stop reason.
        stop_reason: StopReason,
        /// Usage.
        usage: Usage,
        /// Error text.
        error_message: Option<String>,
    },
}

/// Picks a response for a request before the queue is consulted (`None` = use the queue).
pub type FauxRouter = Arc<dyn Fn(&JsonValue) -> Option<FauxResponse> + Send + Sync>;

/// Scripted provider component (`provides.apis = ["faux"]`).
#[derive(Default)]
pub struct FauxProvider {
    responses: Mutex<VecDeque<FauxResponse>>,
    requests: Mutex<Vec<JsonValue>>,
    router: Mutex<Option<FauxRouter>>,
}

impl std::fmt::Debug for FauxProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FauxProvider").finish_non_exhaustive()
    }
}

impl FauxProvider {
    /// A provider answering with `responses` in order (then `"(no more responses)"`).
    #[must_use]
    pub fn new(responses: Vec<FauxResponse>) -> Arc<Self> {
        Arc::new(Self {
            responses: Mutex::new(responses.into()),
            requests: Mutex::new(Vec::new()),
            router: Mutex::new(None),
        })
    }

    /// Answers requests `router` recognises (e.g. nested agents by their system prompt)
    /// without consuming the queue.
    pub fn route(
        &self,
        router: impl Fn(&JsonValue) -> Option<FauxResponse> + Send + Sync + 'static,
    ) {
        *lock(&self.router) = Some(Arc::new(router));
    }

    /// Queues more responses.
    pub fn push(&self, response: FauxResponse) {
        lock(&self.responses).push_back(response);
    }

    /// Every request body received.
    #[must_use]
    pub fn requests(&self) -> Vec<JsonValue> {
        lock(&self.requests).clone()
    }

    /// The faux model.
    #[must_use]
    pub fn model() -> Model {
        serde_json::from_value(json!({
            "id": "faux-1", "name": "Faux 1", "api": FAUX_API, "provider": "faux", "baseUrl": "",
            "reasoning": true, "input": ["text", "image"],
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
            "contextWindow": 128_000, "maxTokens": 16_384
        }))
        .unwrap_or_else(|_| oino_agent::agent::unknown_model())
    }

    #[allow(
        clippy::too_many_lines,
        reason = "one arm per scripted response and block kind"
    )]
    fn events(response: FauxResponse) -> Vec<ProviderEvent> {
        let model = Self::model();
        let mut message =
            AssistantMessage::empty(FAUX_API, &model.provider, &model.id, oino_agent::now_ms());
        let mut events = vec![ProviderEvent::Start {}];
        let (content, stop_reason, usage, error) = match response {
            FauxResponse::Text(text) => (
                vec![AssistantContent::text(text)],
                StopReason::Stop,
                Usage::default(),
                None,
            ),
            FauxResponse::ToolCalls(calls) => (
                calls
                    .into_iter()
                    .map(|(id, name, arguments)| {
                        AssistantContent::ToolCall(ToolCall {
                            id,
                            name,
                            arguments,
                            ..ToolCall::default()
                        })
                    })
                    .collect(),
                StopReason::ToolUse,
                Usage::default(),
                None,
            ),
            FauxResponse::Error(error) => {
                (Vec::new(), StopReason::Error, Usage::default(), Some(error))
            }
            FauxResponse::Message {
                content,
                stop_reason,
                usage,
                error_message,
            } => (content, stop_reason, usage, error_message),
        };
        for (index, block) in content.iter().enumerate() {
            match block {
                AssistantContent::Text(text) => {
                    events.push(ProviderEvent::TextStart {
                        content_index: index,
                    });
                    events.push(ProviderEvent::TextDelta {
                        content_index: index,
                        delta: text.text.clone(),
                    });
                    events.push(ProviderEvent::TextEnd {
                        content_index: index,
                        content: text.text.clone(),
                    });
                }
                AssistantContent::Thinking(thinking) => {
                    events.push(ProviderEvent::ThinkingStart {
                        content_index: index,
                    });
                    events.push(ProviderEvent::ThinkingDelta {
                        content_index: index,
                        delta: thinking.thinking.clone(),
                    });
                    events.push(ProviderEvent::ThinkingEnd {
                        content_index: index,
                        content: thinking.thinking.clone(),
                    });
                }
                AssistantContent::ToolCall(call) => {
                    events.push(ProviderEvent::ToolcallStart {
                        content_index: index,
                        id: Some(call.id.clone()),
                        tool_name: Some(call.name.clone()),
                    });
                    events.push(ProviderEvent::ToolcallDelta {
                        content_index: index,
                        delta: serde_json::to_string(&call.arguments).unwrap_or_default(),
                    });
                    events.push(ProviderEvent::ToolcallEnd {
                        content_index: index,
                        tool_call: call.clone(),
                    });
                }
            }
        }
        message.content = content;
        message.stop_reason = stop_reason;
        message.usage = usage;
        message.error_message = error;
        events.push(match stop_reason {
            StopReason::Error => ProviderEvent::Error {
                reason: ErrorReason::Error,
                error: message,
            },
            StopReason::Aborted => ProviderEvent::Error {
                reason: ErrorReason::Aborted,
                error: message,
            },
            StopReason::ToolUse => ProviderEvent::Done {
                reason: DoneReason::ToolUse,
                message,
            },
            StopReason::Length => ProviderEvent::Done {
                reason: DoneReason::Length,
                message,
            },
            _ => ProviderEvent::Done {
                reason: DoneReason::Stop,
                message,
            },
        });
        events
    }
}

impl ProviderInstance for FauxProvider {
    fn stream(
        &self,
        request: JsonValue,
        _cancel: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<BoxStream<'static, Result<ProviderEvent, ExtensionCallError>>, ExtensionCallError>,
    > {
        let routed = lock(&self.router).clone().and_then(|route| route(&request));
        lock(&self.requests).push(request);
        let response = routed.unwrap_or_else(|| {
            lock(&self.responses)
                .pop_front()
                .unwrap_or_else(|| FauxResponse::Text("(no more responses)".into()))
        });
        let events = Self::events(response);
        Box::pin(async move {
            Ok(Box::pin(stream::iter(events.into_iter().map(Ok))) as BoxStream<'static, _>)
        })
    }

    fn list_models(
        &self,
        _provider_id: &str,
        _credential: Option<JsonValue>,
    ) -> BoxFuture<'_, Result<Vec<Model>, ExtensionCallError>> {
        Box::pin(async { Ok(vec![Self::model()]) })
    }
}

/// The faux provider as an extension instance.
#[derive(Debug)]
pub struct FauxComponent(pub Arc<FauxProvider>);

impl ExtensionInstance for FauxComponent {
    fn id(&self) -> &'static str {
        "faux-provider"
    }

    fn kind(&self) -> ExtensionKind {
        ExtensionKind::Provider
    }

    fn init(&self, _cx: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>> {
        Box::pin(async {
            Ok(Registration {
                apis: vec![FAUX_API.into()],
                providers: vec![
                    json!({"id": "faux", "name": "Faux", "api": FAUX_API, "models": [FauxProvider::model()]}),
                ],
                ..Registration::default()
            })
        })
    }

    fn on_event(
        &self,
        _kind: &str,
        _payload: JsonValue,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        Box::pin(async { Ok(None) })
    }

    fn run_tool(
        &self,
        tool: &str,
        _invocation: ToolInvocation,
        _updates: UpdateSink,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<AgentToolResult, ExtensionCallError>> {
        let tool = tool.to_owned();
        Box::pin(async move { Err(ExtensionCallError::new(format!("unknown tool {tool}"))) })
    }

    fn run_command(
        &self,
        _name: &str,
        _args: &str,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        Box::pin(async { Ok(None) })
    }

    fn provider(&self) -> Option<&dyn ProviderInstance> {
        Some(self.0.as_ref())
    }
}

/// Event handler of a [`FakeExtension`].
pub type EventHandler =
    Arc<dyn Fn(&str, JsonValue) -> Result<Option<JsonValue>, String> + Send + Sync>;
/// Tool body of a [`FakeExtension`].
pub type ToolHandler =
    Arc<dyn Fn(&str, JsonObject) -> Result<AgentToolResult, String> + Send + Sync>;
/// Command body of a [`FakeExtension`].
pub type CommandHandler =
    Arc<dyn Fn(&str, &str) -> Result<Option<JsonValue>, String> + Send + Sync>;

/// Arbiter `propose-nudge` body of a [`FakeExtension`].
pub type NudgeHandler = Arc<dyn Fn(&CallContext) -> Option<JsonValue> + Send + Sync>;
/// Arbiter `wait-reason` body of a [`FakeExtension`].
pub type WaitHandler = Arc<dyn Fn(&CallContext) -> Option<String> + Send + Sync>;

/// Closure-driven extension for runner and session tests.
pub struct FakeExtension {
    id: String,
    registration: Registration,
    on_event: EventHandler,
    on_tool: ToolHandler,
    on_command: CommandHandler,
    on_nudge: Option<NudgeHandler>,
    on_wait: Option<WaitHandler>,
    events: Mutex<Vec<(String, JsonValue)>>,
    contexts: Mutex<Vec<(String, CallContext)>>,
    delivered: Mutex<Vec<String>>,
}

impl std::fmt::Debug for FakeExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeExtension")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

impl FakeExtension {
    /// An extension with `registration` and no-op handlers.
    #[must_use]
    pub fn new(id: &str, registration: Registration) -> Self {
        Self {
            id: id.to_owned(),
            registration,
            on_event: Arc::new(|_, _| Ok(None)),
            on_tool: Arc::new(|_, _| Ok(AgentToolResult::text("ok"))),
            on_command: Arc::new(|_, _| Ok(None)),
            on_nudge: None,
            on_wait: None,
            events: Mutex::new(Vec::new()),
            contexts: Mutex::new(Vec::new()),
            delivered: Mutex::new(Vec::new()),
        }
    }

    /// Makes the extension an arbiter nudge provider.
    #[must_use]
    pub fn on_nudge(
        mut self,
        handler: impl Fn(&CallContext) -> Option<JsonValue> + Send + Sync + 'static,
    ) -> Self {
        self.registration.arbiter.nudge_provider = true;
        self.on_nudge = Some(Arc::new(handler));
        self
    }

    /// Makes the extension an arbiter wait source.
    #[must_use]
    pub fn on_wait(
        mut self,
        handler: impl Fn(&CallContext) -> Option<String> + Send + Sync + 'static,
    ) -> Self {
        self.registration.arbiter.wait_source = true;
        self.on_wait = Some(Arc::new(handler));
        self
    }

    /// Call contexts of the events received `(kind, cx)`.
    #[must_use]
    pub fn received_contexts(&self) -> Vec<(String, CallContext)> {
        lock(&self.contexts).clone()
    }

    /// Ids passed to `nudge-delivered`.
    #[must_use]
    pub fn delivered(&self) -> Vec<String> {
        lock(&self.delivered).clone()
    }

    /// Sets the event handler.
    #[must_use]
    pub fn on_event(
        mut self,
        handler: impl Fn(&str, JsonValue) -> Result<Option<JsonValue>, String> + Send + Sync + 'static,
    ) -> Self {
        self.on_event = Arc::new(handler);
        self
    }

    /// Sets the tool handler.
    #[must_use]
    pub fn on_tool(
        mut self,
        handler: impl Fn(&str, JsonObject) -> Result<AgentToolResult, String> + Send + Sync + 'static,
    ) -> Self {
        self.on_tool = Arc::new(handler);
        self
    }

    /// Sets the command handler.
    #[must_use]
    pub fn on_command(
        mut self,
        handler: impl Fn(&str, &str) -> Result<Option<JsonValue>, String> + Send + Sync + 'static,
    ) -> Self {
        self.on_command = Arc::new(handler);
        self
    }

    /// Every event received `(kind, payload)`.
    #[must_use]
    pub fn received(&self) -> Vec<(String, JsonValue)> {
        lock(&self.events).clone()
    }
}

impl ExtensionInstance for FakeExtension {
    fn id(&self) -> &str {
        &self.id
    }

    fn init(&self, _cx: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>> {
        let registration = self.registration.clone();
        Box::pin(async move { Ok(registration) })
    }

    fn on_event(
        &self,
        kind: &str,
        payload: JsonValue,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        lock(&self.events).push((kind.to_owned(), payload.clone()));
        lock(&self.contexts).push((kind.to_owned(), cx));
        let result = (self.on_event)(kind, payload).map_err(ExtensionCallError::new);
        Box::pin(async move { result })
    }

    fn run_tool(
        &self,
        tool: &str,
        invocation: ToolInvocation,
        _updates: UpdateSink,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<AgentToolResult, ExtensionCallError>> {
        let result = (self.on_tool)(tool, invocation.args).map_err(ExtensionCallError::new);
        Box::pin(async move { result })
    }

    fn run_command(
        &self,
        name: &str,
        args: &str,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        let result = (self.on_command)(name, args).map_err(ExtensionCallError::new);
        Box::pin(async move { result })
    }

    fn propose_nudge(&self, cx: CallContext) -> BoxFuture<'_, Option<JsonValue>> {
        let proposal = self.on_nudge.as_ref().and_then(|f| f(&cx));
        Box::pin(async move { proposal })
    }

    fn nudge_delivered(&self, id: &str, _cx: CallContext) {
        lock(&self.delivered).push(id.to_owned());
    }

    fn wait_reason(&self, cx: CallContext) -> BoxFuture<'_, Option<String>> {
        let reason = self.on_wait.as_ref().and_then(|f| f(&cx));
        Box::pin(async move { reason })
    }
}

/// Builder for an in-memory test session.
pub struct TestSession {
    /// Extensions in load order (the faux provider is added first).
    pub extensions: Vec<Arc<dyn ExtensionInstance>>,
    /// Built-in tools.
    pub tools: Vec<ToolEntry>,
    /// Initial settings.
    pub settings: JsonObject,
    /// Working directory.
    pub cwd: PathBuf,
    /// Dialog UI attached.
    pub has_ui: bool,
}

impl Default for TestSession {
    fn default() -> Self {
        Self {
            extensions: Vec::new(),
            tools: Vec::new(),
            settings: JsonObject::new(),
            cwd: std::env::temp_dir(),
            has_ui: false,
        }
    }
}

impl TestSession {
    /// Builds the session with `provider` answering requests.
    pub async fn build(self, provider: Arc<FauxProvider>) -> AgentSession {
        let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
        let mut instances: Vec<Arc<dyn ExtensionInstance>> =
            vec![Arc::new(FauxComponent(provider))];
        instances.extend(self.extensions);
        let cx = CallContext {
            session_id: String::new(),
            mode: ExtensionMode::Print,
            has_ui: self.has_ui,
            cwd: self.cwd.clone(),
            is_child: false,
            child_role: None,
            call_kind: crate::extensions::api::CallKind::Event,
        };
        let mut loaded = Vec::new();
        for instance in instances {
            let registration = instance.init(cx.clone()).await.unwrap_or_default();
            for api in &registration.apis {
                models.register_api_implementor(api, instance.id());
            }
            for provider in &registration.providers {
                if let Ok(registration) =
                    serde_json::from_value::<ProviderRegistration>(provider.clone())
                {
                    let _ = models.register_provider(instance.id(), registration);
                }
            }
            loaded.push(Arc::new(LoadedExtension::new(instance, registration)));
        }
        models.set_runtime_api_key("faux", "test-key");
        let set = Arc::new(ExtensionSet::new(loaded));
        let mut manager =
            SessionManager::in_memory(&self.cwd, CreateOptions::default(), Vec::new());
        // New sessions record the initial selection (pi `createAgentSession`).
        let model = FauxProvider::model();
        let _ = manager.append_model_change(&model.provider, &model.id);
        let _ = manager.append_thinking_level_change("off");
        let settings = Arc::new(Mutex::new(SettingsManager::in_memory(self.settings)));
        let resources = Arc::new(RwLock::new(ResourceLoader::new(ResourceLoaderOptions {
            cwd: self.cwd.clone(),
            agent_dir: self.cwd.join(".oino-test-agent"),
            no_skills: true,
            no_prompt_templates: true,
            no_themes: true,
            no_context_files: true,
            ..ResourceLoaderOptions::default()
        })));
        let names: Vec<String> = self.tools.iter().map(|t| t.name().to_owned()).collect();
        let session = AgentSession::new(SessionConfig {
            cwd: self.cwd.clone(),
            agent_dir: self.cwd.join(".oino-test-agent"),
            docs_root: PathBuf::from("/oino-docs"),
            manager,
            settings,
            resources,
            models,
            extensions: Arc::clone(&set),
            stream: Arc::new(ProviderStream::new(set)),
            mode: ExtensionMode::Print,
            has_ui: self.has_ui,
            builtin_tools: self.tools,
            custom_tools: Vec::new(),
            initial_active_tools: names,
            allowed_tools: None,
            excluded_tools: Vec::new(),
            model: Some(FauxProvider::model()),
            thinking_level: ThinkingLevel::Off,
            scoped_models: Vec::new(),
            flag_values: IndexMap::new(),
            child: None,
        });
        session.bind_extensions("startup", None).await;
        session
    }
}
