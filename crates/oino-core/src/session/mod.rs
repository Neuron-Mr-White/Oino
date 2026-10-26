//! `AgentSession` (pi `agent-session.ts`): agent lifecycle, persistence, prompts, queues,
//! models, tools, compaction, retry, bash, tree navigation and the extension bindings of one
//! session.
//!
//! The session is cheap to clone (`Arc` inside). State is split into small mutexes that are
//! never held across an `.await` on provider, tool or extension calls (architecture §7).

mod bash;
mod boundary;
mod compaction_flow;
mod hooks;
mod host;
mod listener;
mod model;
mod prompt;
mod retry;
mod stats;
mod tools;
mod tree;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use oino_agent::{Agent, AgentOptions, StreamFn, Subscription};
use oino_ai::transcript::SystemState;
use oino_ai::{
    AgentMessage, AssistantMessage, BashExecutionMessage, CustomMessage, Model, SystemMessage,
    ThinkingLevel,
};
use oino_models::ModelRegistry;
use oino_resources::{BuildSystemPromptOptions, ResourceLoader};
use oino_session::{EntryId, SessionEntry, SessionManager};
use oino_settings::SettingsManager;
use serde::{Deserialize, Serialize};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;

use crate::events::{AgentSessionEvent, ClientEvent, ExtensionErrorEvent, OinoEvent, RunState};
use crate::extensions::api::{CallContext, CallKind, ExtensionMode};
use crate::extensions::runner::{ErrorSink, ExtensionRunner, ExtensionSet};
use crate::hub::{EventHub, EventStream};
use crate::ui::{UiBridge, UiOutput, UiSink};

pub use bash::BashResult;
pub use host::SlashCommandInfo;
pub use model::{CycleDirection, CycleModelResult};
pub use prompt::{InputSource, PromptDisposition, PromptOptions, StreamingBehavior};
pub use stats::{ContextUsage, SessionStats, TokenTotals};
pub use tools::{BUILTIN_PATH_PREFIX, ToolEntry, ToolInfo, ToolSource};
pub use tree::{ForkMessage, NavigateTreeOptions, NavigateTreeResult};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// pi `getCurrentSystemMessage` over agent messages.
#[must_use]
pub fn current_system_message(messages: &[AgentMessage]) -> Option<SystemMessage> {
    SystemState::fold(messages.iter().filter_map(|message| match message {
        AgentMessage::System(system) => Some(system),
        _ => None,
    }))
    .into_message()
}

/// Failure of a session operation (pi error texts).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SessionOpError {
    /// `prompt` while streaming without `streaming_behavior`.
    #[error(
        "Agent is already processing. Specify streamingBehavior ('steer' or 'followUp') to queue the message."
    )]
    StreamingWithoutBehavior,
    /// No model selected.
    #[error("{0}")]
    NoModel(String),
    /// No credentials for the provider.
    #[error("{message}")]
    NoApiKey {
        /// Provider id.
        provider: String,
        /// pi message.
        message: String,
    },
    /// An extension command cannot be queued.
    #[error(
        "Extension command \"/{0}\" cannot be queued. Use prompt() or execute the command when not streaming."
    )]
    CommandNotAllowed(String),
    /// Prompt during manual compaction.
    #[error(
        "Cannot submit a prompt while compaction is in progress. Wait for compaction to finish and retry."
    )]
    CompactionInProgress,
    /// Session-replacing operations need a command context.
    #[error("This operation is only available in command and shortcut contexts")]
    CommandContextRequired,
    /// Any other failure with pi's text.
    #[error("{0}")]
    Failed(String),
}

/// What an `agent_before_settle` / `turn_end` run ended with (pi `AgentActivityOutcome`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActivityOutcome {
    /// Normal end.
    #[default]
    Completed,
    /// Aborted.
    Aborted,
    /// Error.
    Error,
}

/// Mutable run bookkeeping (pi private fields).
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors pi's private AgentSession flags one-to-one"
)]
#[derive(Default)]
pub(crate) struct RunBook {
    pub(crate) steering: Vec<String>,
    pub(crate) follow_up: Vec<String>,
    pub(crate) pending_next_turn: Vec<CustomMessage>,
    pub(crate) pending_custom: Vec<CustomMessage>,
    pub(crate) pending_bash: Vec<BashExecutionMessage>,
    pub(crate) run_active: bool,
    pub(crate) abort_requested: bool,
    pub(crate) overflow_recovery_attempted: bool,
    pub(crate) retry_attempt: u32,
    pub(crate) failed_response: Option<AssistantMessage>,
    pub(crate) last_assistant: Option<AssistantMessage>,
    pub(crate) last_outcome: ActivityOutcome,
    pub(crate) turn_index: u64,
    pub(crate) is_before_settle: bool,
    pub(crate) abort_during_before_settle: bool,
    pub(crate) emitting_settled: bool,
    pub(crate) boundary_dispatched: bool,
    pub(crate) run_prompt_options: Option<BuildSystemPromptOptions>,
    /// Entry id of the latest persisted assistant message.
    pub(crate) last_assistant_entry: Option<EntryId>,
    /// Entry ids of persisted tool results by tool-call id (current run).
    pub(crate) tool_result_entries: Vec<(String, EntryId)>,
}

/// Cancellation handles of long operations.
#[derive(Default)]
pub(crate) struct Cancels {
    pub(crate) compaction: Option<CancellationToken>,
    pub(crate) auto_compaction: Option<CancellationToken>,
    pub(crate) branch_summary: Option<CancellationToken>,
    pub(crate) retry: Option<CancellationToken>,
    pub(crate) bash: Vec<CancellationToken>,
}

/// Inputs of [`AgentSession::new`].
pub struct SessionConfig {
    /// Working directory.
    pub cwd: PathBuf,
    /// Agent dir (`~/.oino`).
    pub agent_dir: PathBuf,
    /// Root of the bundled Oino docs (system prompt docs section).
    pub docs_root: PathBuf,
    /// Session tree.
    pub manager: SessionManager,
    /// Settings (shared per cwd).
    pub settings: Arc<Mutex<SettingsManager>>,
    /// Resources (shared per cwd).
    pub resources: Arc<RwLock<ResourceLoader>>,
    /// Models and auth.
    pub models: ModelRegistry,
    /// Loaded extensions.
    pub extensions: Arc<ExtensionSet>,
    /// Provider dispatch.
    pub stream: Arc<dyn StreamFn>,
    /// Run mode.
    pub mode: ExtensionMode,
    /// Dialog-capable UI attached.
    pub has_ui: bool,
    /// Built-in tools (with prompt metadata).
    pub builtin_tools: Vec<ToolEntry>,
    /// SDK custom tools.
    pub custom_tools: Vec<ToolEntry>,
    /// Initially active tool names.
    pub initial_active_tools: Vec<String>,
    /// `--tools` allowlist patterns.
    pub allowed_tools: Option<Vec<String>>,
    /// `--exclude-tools` patterns.
    pub excluded_tools: Vec<String>,
    /// Initial model.
    pub model: Option<Model>,
    /// Initial thinking level.
    pub thinking_level: ThinkingLevel,
    /// `--models` scope.
    pub scoped_models: Vec<oino_models::resolver::ScopedModel>,
    /// CLI flag values for extension flags.
    pub flag_values: indexmap::IndexMap<String, oino_ai::JsonValue>,
    /// Nested (child) session.
    pub is_child: bool,
}

pub(crate) struct Inner {
    pub(crate) agent: Agent,
    pub(crate) stream: Arc<dyn StreamFn>,
    pub(crate) manager: Mutex<SessionManager>,
    pub(crate) settings: Arc<Mutex<SettingsManager>>,
    pub(crate) resources: Arc<RwLock<ResourceLoader>>,
    pub(crate) models: ModelRegistry,
    pub(crate) runner: RwLock<ExtensionRunner>,
    pub(crate) hub: EventHub,
    pub(crate) ui: UiBridge,
    pub(crate) book: Mutex<RunBook>,
    pub(crate) cancels: Mutex<Cancels>,
    pub(crate) tools: Mutex<tools::ToolState>,
    pub(crate) scoped_models: Mutex<Vec<oino_models::resolver::ScopedModel>>,
    pub(crate) idle: watch::Sender<bool>,
    pub(crate) cwd: PathBuf,
    pub(crate) agent_dir: PathBuf,
    pub(crate) docs_root: PathBuf,
    pub(crate) mode: ExtensionMode,
    pub(crate) subscription: Mutex<Option<Subscription>>,
}

/// One live session. Cheap to clone.
#[derive(Clone)]
pub struct AgentSession {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for AgentSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSession")
            .field("session_id", &self.session_id())
            .finish_non_exhaustive()
    }
}

struct HubErrors(EventHub);

impl ErrorSink for HubErrors {
    fn report(&self, error: ExtensionErrorEvent) {
        tracing::warn!(extension = %error.extension_path, event = %error.event, "{}", error.error);
        self.0.publish(error);
    }
}

struct HubUi(EventHub);

impl UiSink for HubUi {
    fn publish(&self, output: UiOutput) {
        match output {
            UiOutput::Request(request) => self.0.publish(ClientEvent::Ui(request)),
            UiOutput::Dismiss { id, reason } => {
                self.0.publish(OinoEvent::ExtensionUiDismiss { id, reason });
            }
        }
    }
}

impl AgentSession {
    /// Creates a session (pi `new AgentSession` + `createAgentSession` agent wiring). Call
    /// [`AgentSession::bind_extensions`] afterwards to emit `session_start`.
    #[must_use]
    pub fn new(config: SessionConfig) -> Self {
        let hub = EventHub::new();
        let session_id = config.manager.session_id().to_owned();
        let has_ui = config.has_ui;
        let cx = CallContext {
            session_id: session_id.clone(),
            mode: config.mode,
            has_ui,
            cwd: config.cwd.clone(),
            is_child: config.is_child,
            child_role: None,
            call_kind: CallKind::Event,
        };
        let runner = ExtensionRunner::new(
            Arc::clone(&config.extensions),
            cx,
            Arc::new(HubErrors(hub.clone())),
            config.flag_values.clone(),
        );
        let ui = UiBridge::new(has_ui, session_id.clone(), Arc::new(HubUi(hub.clone())));
        let context = config.manager.build_session_context();
        let settings = lock(&config.settings).snapshot();
        let session_stream = Arc::new(hooks::SessionStream {
            inner: std::sync::OnceLock::new(),
            base: Arc::clone(&config.stream),
        });
        let mut options = AgentOptions::new(Arc::clone(&session_stream) as Arc<dyn StreamFn>);
        options.model = config
            .model
            .clone()
            .unwrap_or_else(oino_agent::agent::unknown_model);
        options.thinking_level = config.thinking_level;
        options.messages = context.messages;
        options.steering_mode = map_queue_mode(settings.steering_mode());
        options.follow_up_mode = map_queue_mode(settings.follow_up_mode());
        options.stream_options.session_id = Some(session_id);
        options.stream_options.transport = Some(settings.transport());
        options.stream_options.thinking_budgets = settings.thinking_budgets();
        options.stream_options.max_retry_delay_ms =
            Some(settings.provider_retry_settings().max_retry_delay_ms);
        options.convert_to_llm = convert_with_block_images(Arc::clone(&config.settings));
        let agent = Agent::new(options);
        let (idle, _) = watch::channel(true);
        let tool_state = tools::ToolState::new(
            config.builtin_tools,
            config.custom_tools,
            config.allowed_tools.as_deref(),
            &config.excluded_tools,
        );
        let inner = Arc::new(Inner {
            agent,
            stream: Arc::clone(&session_stream) as Arc<dyn StreamFn>,
            manager: Mutex::new(config.manager),
            settings: config.settings,
            resources: config.resources,
            models: config.models,
            runner: RwLock::new(runner),
            hub,
            ui,
            book: Mutex::new(RunBook::default()),
            cancels: Mutex::new(Cancels::default()),
            tools: Mutex::new(tool_state),
            scoped_models: Mutex::new(config.scoped_models),
            idle,
            cwd: config.cwd,
            agent_dir: config.agent_dir,
            docs_root: config.docs_root,
            mode: config.mode,
            subscription: Mutex::new(None),
        });
        let _ = session_stream.inner.set(Arc::downgrade(&inner));
        let session = Self { inner };
        let subscription = session
            .inner
            .agent
            .subscribe(Arc::new(listener::Listener::new(&session.inner)));
        *lock(&session.inner.subscription) = Some(subscription);
        session
            .inner
            .agent
            .set_hooks(hooks::install(&session.inner));
        session.refresh_tool_registry(Some(config.initial_active_tools), true);
        session
    }

    /// pi `bindExtensions` tail: emits `session_start` and merges `resources_discover`
    /// results into the resources.
    pub async fn bind_extensions(&self, reason: &str, previous_session_file: Option<PathBuf>) {
        let runner = self.runner();
        if runner.has_handlers("session_start") {
            let mut payload = serde_json::json!({"type": "session_start", "reason": reason});
            if let Some(previous) = previous_session_file {
                payload["previousSessionFile"] = serde_json::json!(previous.display().to_string());
            }
            runner.emit(payload).await;
        }
        let discover_reason = if reason == "reload" {
            "reload"
        } else {
            "startup"
        };
        self.extend_resources_from_extensions(discover_reason).await;
    }

    async fn extend_resources_from_extensions(&self, reason: &str) {
        let runner = self.runner();
        if !runner.has_handlers("resources_discover") {
            return;
        }
        let found = runner
            .emit_resources_discover(&self.inner.cwd.display().to_string(), reason)
            .await;
        if found.skill_paths.is_empty()
            && found.prompt_paths.is_empty()
            && found.theme_paths.is_empty()
        {
            return;
        }
        let convert = |paths: Vec<(String, String)>| -> Vec<oino_resources::ExtraResourcePath> {
            paths
                .into_iter()
                .map(|(path, extension)| oino_resources::ExtraResourcePath {
                    path: PathBuf::from(path),
                    metadata: oino_resources::source::PathMetadata {
                        source: format!("extension:{extension}"),
                        scope: oino_resources::source::SourceScope::Temporary,
                        origin: oino_resources::source::SourceOrigin::TopLevel,
                        base_dir: None,
                    },
                })
                .collect()
        };
        let paths = oino_resources::ResourceExtensionPaths {
            skill_paths: convert(found.skill_paths),
            prompt_paths: convert(found.prompt_paths),
            theme_paths: convert(found.theme_paths),
        };
        self.inner
            .resources
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(&paths);
        let active = self.active_tool_names();
        self.rebuild_system_prompt(&active);
    }

    pub(crate) fn from_inner(inner: Arc<Inner>) -> Self {
        Self { inner }
    }

    /// The extension runner of this session.
    #[must_use]
    pub fn runner(&self) -> ExtensionRunner {
        self.inner
            .runner
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Ordered client event stream (bounded, coalescing).
    #[must_use]
    pub fn subscribe(&self) -> EventStream {
        self.inner.hub.subscribe()
    }

    /// The event hub (runtime rebinding).
    #[must_use]
    pub fn hub(&self) -> &EventHub {
        &self.inner.hub
    }

    pub(crate) fn emit(&self, event: AgentSessionEvent) {
        self.inner.hub.publish(event);
    }

    /// Frontend → extension UI plumbing.
    #[must_use]
    pub fn ui(&self) -> UiBridge {
        self.inner.ui.clone()
    }

    /// The underlying agent.
    #[must_use]
    pub fn agent(&self) -> &Agent {
        &self.inner.agent
    }

    /// Runs `f` with the session manager locked.
    pub fn with_manager<R>(&self, f: impl FnOnce(&mut SessionManager) -> R) -> R {
        f(&mut lock(&self.inner.manager))
    }

    /// Runs `f` with the settings manager locked.
    pub fn with_settings<R>(&self, f: impl FnOnce(&mut SettingsManager) -> R) -> R {
        f(&mut lock(&self.inner.settings))
    }

    /// Model registry.
    #[must_use]
    pub fn models(&self) -> &ModelRegistry {
        &self.inner.models
    }

    /// Session cwd.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.inner.cwd
    }

    /// Agent dir.
    #[must_use]
    pub fn agent_dir(&self) -> &Path {
        &self.inner.agent_dir
    }

    /// Run mode.
    #[must_use]
    pub fn mode(&self) -> ExtensionMode {
        self.inner.mode
    }

    /// Current model (`None` until one is selected).
    #[must_use]
    pub fn model(&self) -> Option<Model> {
        let model = self.inner.agent.model();
        (model.provider != "unknown" || model.id != "unknown").then_some(model)
    }

    /// Current thinking level.
    #[must_use]
    pub fn thinking_level(&self) -> ThinkingLevel {
        self.inner.agent.thinking_level()
    }

    /// Whether an agent run (or post-run continuation) is active.
    #[must_use]
    pub fn is_streaming(&self) -> bool {
        lock(&self.inner.book).run_active
    }

    /// Whether a compaction or branch summary is running.
    #[must_use]
    pub fn is_compacting(&self) -> bool {
        let cancels = lock(&self.inner.cancels);
        cancels.compaction.is_some()
            || cancels.auto_compaction.is_some()
            || cancels.branch_summary.is_some()
    }

    /// pi `isIdle`.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        !self.is_streaming() && !self.is_compacting()
    }

    /// Whether an automatic retry is waiting.
    #[must_use]
    pub fn is_retrying(&self) -> bool {
        lock(&self.inner.cancels).retry.is_some()
    }

    /// Whether a `!` command runs.
    #[must_use]
    pub fn is_bash_running(&self) -> bool {
        !lock(&self.inner.cancels).bash.is_empty()
    }

    /// Current retry attempt (0 when not retrying).
    #[must_use]
    pub fn retry_attempt(&self) -> u32 {
        lock(&self.inner.book).retry_attempt
    }

    /// The finalized transcript (session projection).
    #[must_use]
    pub fn messages(&self) -> Vec<AgentMessage> {
        self.inner.agent.messages()
    }

    /// Session file, when persisted.
    #[must_use]
    pub fn session_file(&self) -> Option<PathBuf> {
        lock(&self.inner.manager)
            .session_file()
            .map(Path::to_path_buf)
    }

    /// Session id.
    #[must_use]
    pub fn session_id(&self) -> String {
        lock(&self.inner.manager).session_id().to_owned()
    }

    /// Session display name.
    #[must_use]
    pub fn session_name(&self) -> Option<String> {
        lock(&self.inner.manager).session_name()
    }

    /// Steering texts queued.
    #[must_use]
    pub fn steering_messages(&self) -> Vec<String> {
        lock(&self.inner.book).steering.clone()
    }

    /// Follow-up texts queued.
    #[must_use]
    pub fn follow_up_messages(&self) -> Vec<String> {
        lock(&self.inner.book).follow_up.clone()
    }

    /// Number of queued texts.
    #[must_use]
    pub fn pending_message_count(&self) -> usize {
        let book = lock(&self.inner.book);
        book.steering.len() + book.follow_up.len()
    }

    /// Clears both queues and returns their texts (pi `clearQueue`).
    #[must_use]
    pub fn clear_queue(&self) -> (Vec<String>, Vec<String>) {
        let (steering, follow_up) = {
            let mut book = lock(&self.inner.book);
            (
                std::mem::take(&mut book.steering),
                std::mem::take(&mut book.follow_up),
            )
        };
        self.inner.agent.clear_all_queues();
        self.emit_queue_update();
        (steering, follow_up)
    }

    pub(crate) fn emit_queue_update(&self) {
        let (steering, follow_up) = {
            let book = lock(&self.inner.book);
            (book.steering.clone(), book.follow_up.clone())
        };
        self.emit(AgentSessionEvent::QueueUpdate {
            steering,
            follow_up,
        });
    }

    /// Text of the last assistant message (pi `getLastAssistantText`).
    #[must_use]
    pub fn last_assistant_text(&self) -> Option<String> {
        let messages = self.messages();
        let last = messages.iter().rev().find_map(|message| match message {
            AgentMessage::Assistant(assistant)
                if !(assistant.stop_reason == oino_ai::StopReason::Aborted
                    && assistant.content.is_empty()) =>
            {
                Some(assistant)
            }
            _ => None,
        })?;
        let text: String = last
            .content
            .iter()
            .filter_map(|block| match block {
                oino_ai::AssistantContent::Text(text) => Some(text.text.as_str()),
                _ => None,
            })
            .collect();
        let trimmed = text.trim();
        (!trimmed.is_empty()).then(|| trimmed.to_owned())
    }

    /// Steering drain mode.
    #[must_use]
    pub fn steering_mode(&self) -> oino_agent::QueueMode {
        self.inner.agent.steering_mode()
    }

    /// Follow-up drain mode.
    #[must_use]
    pub fn follow_up_mode(&self) -> oino_agent::QueueMode {
        self.inner.agent.follow_up_mode()
    }

    /// Sets the steering mode and saves it to settings.
    pub fn set_steering_mode(&self, mode: oino_agent::QueueMode) {
        self.inner.agent.set_steering_mode(mode);
        let result = self.with_settings(|s| s.set_steering_mode(unmap_queue_mode(mode)));
        if let Err(error) = result {
            tracing::warn!("saving steeringMode failed: {error}");
        }
    }

    /// Sets the follow-up mode and saves it to settings.
    pub fn set_follow_up_mode(&self, mode: oino_agent::QueueMode) {
        self.inner.agent.set_follow_up_mode(mode);
        let result = self.with_settings(|s| s.set_follow_up_mode(unmap_queue_mode(mode)));
        if let Err(error) = result {
            tracing::warn!("saving followUpMode failed: {error}");
        }
    }

    /// Replaces the `--models` scope.
    pub fn set_scoped_models(&self, scope: Vec<oino_models::resolver::ScopedModel>) {
        *lock(&self.inner.scoped_models) = scope;
    }

    /// The `--models` scope.
    #[must_use]
    pub fn scoped_models(&self) -> Vec<oino_models::resolver::ScopedModel> {
        lock(&self.inner.scoped_models).clone()
    }

    /// Rebuilds the finalized transcript from the session projection (pi
    /// `_refreshFinalizedContext`).
    pub fn refresh_context(&self) {
        let messages = lock(&self.inner.manager)
            .build_session_projection()
            .messages;
        self.inner.agent.set_messages(messages);
    }

    /// Copy of the session entries (`get_entries`).
    #[must_use]
    pub fn entries(&self) -> Vec<SessionEntry> {
        lock(&self.inner.manager).entries().to_vec()
    }

    /// Coarse run state.
    #[must_use]
    pub fn run_state(&self) -> RunState {
        if self.is_compacting() {
            RunState::Compacting
        } else if self.is_retrying() {
            RunState::Retrying
        } else if self.is_streaming() {
            RunState::Running
        } else {
            RunState::Idle
        }
    }

    pub(crate) fn set_idle_if_idle(&self) {
        if self.is_idle() {
            self.inner.idle.send_replace(true);
        }
    }

    pub(crate) fn mark_busy(&self) {
        self.inner.idle.send_replace(false);
    }

    /// Resolves once the session is idle (pi `waitForIdle`).
    pub async fn wait_for_idle(&self) {
        if self.is_idle() {
            return;
        }
        let mut idle = self.inner.idle.subscribe();
        loop {
            if self.is_idle() && *idle.borrow() {
                return;
            }
            if idle.changed().await.is_err() {
                return;
            }
        }
    }

    /// Aborts the current operation and waits for idle (pi `abort`).
    pub async fn abort(&self) {
        {
            let mut book = lock(&self.inner.book);
            if book.run_active {
                book.abort_requested = true;
            }
            if book.is_before_settle {
                book.abort_during_before_settle = true;
            }
        }
        self.abort_retry();
        self.abort_compaction();
        self.abort_branch_summary();
        self.inner.agent.abort();
        self.wait_for_idle().await;
    }

    /// Cancels the retry wait.
    pub fn abort_retry(&self) {
        if let Some(token) = &lock(&self.inner.cancels).retry {
            token.cancel();
        }
    }

    /// Cancels manual or automatic compaction.
    pub fn abort_compaction(&self) {
        let cancels = lock(&self.inner.cancels);
        for token in [&cancels.compaction, &cancels.auto_compaction]
            .into_iter()
            .flatten()
        {
            token.cancel();
        }
    }

    /// Cancels branch summarisation.
    pub fn abort_branch_summary(&self) {
        if let Some(token) = &lock(&self.inner.cancels).branch_summary {
            token.cancel();
        }
    }

    /// Cancels running `!` commands.
    pub fn abort_bash(&self) {
        for token in &lock(&self.inner.cancels).bash {
            token.cancel();
        }
    }

    /// Disposes the session: aborts everything and invalidates extension contexts.
    #[allow(
        clippy::unused_async,
        reason = "async per the runtime.md AgentSession contract"
    )]
    pub async fn dispose(&self) {
        self.abort_retry();
        self.abort_compaction();
        self.abort_branch_summary();
        self.abort_bash();
        self.inner.agent.abort();
        self.runner().invalidate(None);
        self.inner.ui.cancel_all();
        *lock(&self.inner.subscription) = None;
    }

    /// Sets the session name (pi `setSessionName`).
    ///
    /// # Errors
    ///
    /// Persisting the `session_info` entry failed.
    pub async fn set_session_name(&self, name: Option<String>) -> Result<(), SessionOpError> {
        let current = {
            let mut manager = lock(&self.inner.manager);
            manager
                .append_session_info(name.as_deref())
                .map_err(|e| SessionOpError::Failed(e.to_string()))?;
            manager.session_name()
        };
        self.emit(AgentSessionEvent::SessionInfoChanged {
            name: current.clone(),
        });
        let mut payload = serde_json::json!({"type": "session_info_changed"});
        if let Some(name) = current {
            payload["name"] = serde_json::json!(name);
        }
        self.runner().emit(payload).await;
        Ok(())
    }

    /// Sets or clears a label.
    ///
    /// # Errors
    ///
    /// Unknown entry or persistence failure.
    #[allow(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "async per the runtime.md AgentSession contract"
    )]
    pub async fn set_label(
        &self,
        entry: &EntryId,
        label: Option<String>,
    ) -> Result<(), SessionOpError> {
        lock(&self.inner.manager)
            .append_label_change(entry, label.as_deref())
            .map(|_| ())
            .map_err(|e| SessionOpError::Failed(e.to_string()))
    }

    pub(crate) fn publish_entry(&self, entry_id: &EntryId) {
        let entry = lock(&self.inner.manager).entry(entry_id).cloned();
        if let Some(entry) = entry {
            self.emit(AgentSessionEvent::EntryAppended { entry });
        }
    }

    pub(crate) fn emit_model_changed(&self, source: crate::events::ModelChangeSource) {
        if let Some(model) = self.model() {
            self.inner.hub.publish(OinoEvent::ModelChanged {
                model,
                routed_model: self.routed_model(),
                source,
            });
        }
    }
}

fn map_queue_mode(mode: oino_settings::QueueMode) -> oino_agent::QueueMode {
    match mode {
        oino_settings::QueueMode::All => oino_agent::QueueMode::All,
        oino_settings::QueueMode::OneAtATime => oino_agent::QueueMode::OneAtATime,
    }
}

fn unmap_queue_mode(mode: oino_agent::QueueMode) -> oino_settings::QueueMode {
    match mode {
        oino_agent::QueueMode::All => oino_settings::QueueMode::All,
        oino_agent::QueueMode::OneAtATime => oino_settings::QueueMode::OneAtATime,
    }
}

/// pi `convertToLlmWithBlockImages`.
fn convert_with_block_images(settings: Arc<Mutex<SettingsManager>>) -> oino_agent::ConvertToLlm {
    Arc::new(move |messages: &[AgentMessage]| {
        let converted = oino_session::convert_to_llm(messages);
        if !lock(&settings).settings().image_settings().block_images {
            return converted;
        }
        converted.into_iter().map(block_images).collect()
    })
}

const IMAGE_DISABLED: &str = "Image reading is disabled.";

fn block_images(message: oino_ai::Message) -> oino_ai::Message {
    use oino_ai::{Message, UserContent, UserContentField};
    let strip = |content: Vec<UserContent>| -> Vec<UserContent> {
        let mut out: Vec<UserContent> = Vec::new();
        for block in content {
            let block = match block {
                UserContent::Image(_) => UserContent::text(IMAGE_DISABLED),
                text @ UserContent::Text(_) => text,
            };
            let duplicate = matches!((&block, out.last()), (UserContent::Text(a), Some(UserContent::Text(b))) if a.text == IMAGE_DISABLED && b.text == IMAGE_DISABLED);
            if !duplicate {
                out.push(block);
            }
        }
        out
    };
    match message {
        Message::User(mut user) => {
            if let UserContentField::Blocks(blocks) = user.content {
                user.content = UserContentField::Blocks(strip(blocks));
            }
            Message::User(user)
        }
        Message::ToolResult(mut result) => {
            result.content = strip(result.content);
            Message::ToolResult(result)
        }
        other => other,
    }
}
