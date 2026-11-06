//! `AgentSession`: agent lifecycle, persistence, prompts, queues,
//! models, tools, compaction, retry, bash, tree navigation and the extension bindings of one
//! session.
//!
//! The session is cheap to clone (`Arc` inside). State is split into small mutexes that are
//! never held across an `.await` on provider, tool or extension calls (architecture §7).

mod bash;
mod boundary;
pub mod checkpoints;
mod compaction_flow;
mod cwd;
mod feeds;
mod frontend;
mod hooks;
mod host;
mod listener;
mod model;
mod nested;
pub mod permission;
mod prompt;
mod queue;
mod retry;
mod stats;
mod tools;
mod tree;

use std::any::Any;
use std::future::Future;
use std::panic::AssertUnwindSafe;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, RwLock};

use futures::FutureExt;
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
use crate::ui::{RegistrySink, UiBridge, UiOutput, UiRegistry, UiSink, is_dialog};

pub use bash::BashResult;
pub use checkpoints::{Checkpoint, CheckpointFile, RewindFailure, RewindResult};
pub use cwd::cwd_change_reminder;
pub use feeds::{DetachedBash, ExtensionInfo, RunningBash};
pub use frontend::{
    AuthProviderInfo, CompletionEntry, CompletionGroup, EditorEvent, EditorIntent, FRONTEND_CALLER,
    FrontendError, RenderDensity, RenderRequest, RenderTarget, RenderWidth,
};
pub use host::SlashCommandInfo;
pub(crate) use host::check_render_override;
pub(crate) use model::branch_refs as model_branch_refs;
pub use model::{CycleDirection, CycleModelResult};
pub use prompt::{
    InputSource, PromptDisposition, PromptOptions, ReminderAppend, StreamingBehavior,
};
pub use queue::{QueueEdit, QueueKind};
pub use stats::{ContextUsage, ModelStats, SessionStats, TokenTotals};
pub use tools::{BUILTIN_PATH_PREFIX, ToolEntry, ToolInfo, ToolSource};
pub use tree::{ForkMessage, NavigateTreeOptions, NavigateTreeResult};

pub(crate) fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Panic payload → message (`Box<dyn Any + Send>`).
#[must_use]
pub(crate) fn panic_message(panic: &(dyn Any + Send)) -> String {
    if let Some(text) = panic.downcast_ref::<&str>() {
        (*text).to_owned()
    } else if let Some(text) = panic.downcast_ref::<String>() {
        text.clone()
    } else {
        "unknown panic".to_owned()
    }
}

/// Kinds of session-spawned tasks covered by panic isolation
/// ([`AgentSession::spawn_isolated`], X-PANIC).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IsolatedTask {
    /// The agent drive task (`drive_run`) and its deferred follow-up starts: a panic
    /// resets the run book so the session accepts a new prompt.
    Drive,
    /// Child event forwarding (`forward_child_events`).
    ChildEvents,
    /// The arbiter wait-source poll timer.
    ArbiterPoll,
}

impl IsolatedTask {
    /// Task name used in logs.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Drive => "drive_run",
            Self::ChildEvents => "forward_child_events",
            Self::ArbiterPoll => "arbiter_poll",
        }
    }
}

/// `getCurrentSystemMessage` over agent messages.
#[must_use]
pub fn current_system_message(messages: &[AgentMessage]) -> Option<SystemMessage> {
    SystemState::fold(messages.iter().filter_map(|message| match message {
        AgentMessage::System(system) => Some(system),
        _ => None,
    }))
    .into_message()
}

/// Failure of a session operation (fixed error texts).
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
        /// Message.
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
    /// Another runner holds the single-writer lease and did not respond to the takeover
    /// request (D-LOCK / D-TAKEOVER); the text names the holder.
    #[error("{0}")]
    SessionLocked(String),
    /// Session-replacing operations need a command context.
    #[error("This operation is only available in command and shortcut contexts")]
    CommandContextRequired,
    /// Any other failure.
    #[error("{0}")]
    Failed(String),
}

/// What an `agent_before_settle` / `turn_end` run ended with.
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

/// Mutable run bookkeeping (private fields).
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors the private AgentSession flags one-to-one"
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
    pub(crate) reminder_model: Option<String>,
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
    pub(crate) bash: Vec<(u64, CancellationToken)>,
    pub(crate) next_bash: u64,
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
    /// Nested-agent inputs of a child session (`None` for root sessions).
    pub child: Option<crate::agents::ChildConfig>,
}

pub(crate) struct Inner {
    pub(crate) agent: Agent,
    pub(crate) stream: Arc<dyn StreamFn>,
    pub(crate) manager: Mutex<SessionManager>,
    pub(crate) settings: Arc<Mutex<SettingsManager>>,
    pub(crate) image_policy: Arc<Mutex<ImagePolicy>>,
    pub(crate) resources: Arc<RwLock<ResourceLoader>>,
    pub(crate) models: ModelRegistry,
    pub(crate) runner: RwLock<ExtensionRunner>,
    pub(crate) hub: EventHub,
    pub(crate) ui: UiBridge,
    /// Retained UI state (§6): entries, meta, dialogs, patches.
    pub(crate) registry: Arc<Mutex<UiRegistry>>,
    pub(crate) book: Mutex<RunBook>,
    pub(crate) cancels: Mutex<Cancels>,
    pub(crate) tools: Mutex<tools::ToolState>,
    pub(crate) nested: Mutex<nested::NestedState>,
    pub(crate) scoped_models: Mutex<Vec<oino_models::resolver::ScopedModel>>,
    pub(crate) idle: watch::Sender<bool>,
    /// The cwd the session started in (header cwd): the system prompt and the services'
    /// project. Never changes, so a workspace move keeps the prompt prefix byte-identical.
    pub(crate) cwd: PathBuf,
    /// Where tools, `!bash`, `@` completions and extension calls run: [`Inner::cwd`] until
    /// the session moves ([`AgentSession::change_cwd`], `oino.cwd_change` entries).
    pub(crate) work_cwd: RwLock<PathBuf>,
    pub(crate) agent_dir: PathBuf,
    pub(crate) docs_root: PathBuf,
    pub(crate) mode: ExtensionMode,
    pub(crate) subscription: Mutex<Option<Subscription>>,
    pub(crate) agents: crate::agents::AgentsState,
    pub(crate) arbiter: Mutex<boundary::ArbiterState>,
    pub(crate) runtime: std::sync::OnceLock<std::sync::Weak<crate::runtime::RuntimeCore>>,
    /// Composer text last reported through `editor_event` (`ui.editor-text`).
    pub(crate) editor_text: Mutex<String>,
    /// Topics, frontends, running bash calls, checkpoints, runtime handle.
    pub(crate) frontend: feeds::FrontendState,
    pub(crate) permission: Mutex<permission::PermissionState>,
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

struct HubErrors {
    hub: EventHub,
    registry: Arc<Mutex<UiRegistry>>,
}

impl ErrorSink for HubErrors {
    fn report(&self, error: ExtensionErrorEvent) {
        tracing::warn!(extension = %error.extension_path, event = %error.event, "{}", error.error);
        self.hub.publish(error);
    }

    fn disabled(&self, event: OinoEvent) {
        if let OinoEvent::ExtensionDisabled { extension_id, .. } = &event {
            tracing::error!(extension = %extension_id, "extension disabled for this session");
            lock(&self.registry).remove_owner(extension_id);
            lock(&self.registry).flush();
        }
        self.hub.publish(event);
    }
}

/// The session's UI plumbing: legacy records go to the hub for v1 frontends,
/// registry ops go out as `oino_ui_patch` events (§9.2).
struct UiHub {
    hub: EventHub,
    registry: Arc<Mutex<UiRegistry>>,
}

impl UiHub {
    fn flush(&self) {
        lock(&self.registry).flush();
    }
}

impl UiSink for UiHub {
    fn publish(&self, output: UiOutput) {
        match output {
            UiOutput::Request(request) => {
                if is_dialog(&request.method) {
                    lock(&self.registry).dialog_opened(request.to_registry());
                    self.flush();
                }
                self.hub.publish(ClientEvent::Ui(request));
            }
            UiOutput::Dismiss { id, reason } => {
                lock(&self.registry).dialog_dismissed(&id, reason.into());
                self.flush();
                self.hub
                    .publish(OinoEvent::ExtensionUiDismiss { id, reason });
            }
        }
    }
}

impl RegistrySink for UiHub {
    fn publish(&self, patch: oino_ui::slot::UiPatch) {
        self.hub.publish(OinoEvent::from(patch));
    }
}

impl AgentSession {
    /// Creates a session (`new AgentSession` + `createAgentSession` agent wiring). Call
    /// [`AgentSession::bind_extensions`] afterwards to emit `session_start`.
    #[must_use]
    #[allow(
        clippy::too_many_lines,
        reason = "one constructor for the whole session state, mirroring createAgentSession"
    )]
    pub fn new(config: SessionConfig) -> Self {
        let hub = EventHub::new();
        let session_id = config.manager.session_id().to_owned();
        let has_ui = config.has_ui;
        // A session that moved its working directory (`oino.cwd_change`) keeps running there
        // after a reopen; the header cwd stays the prompt's.
        let work_cwd = config
            .manager
            .moved_cwd()
            .unwrap_or_else(|| config.cwd.clone());
        let cx = CallContext {
            session_id: session_id.clone(),
            mode: config.mode,
            has_ui,
            cwd: work_cwd.clone(),
            is_child: config.child.is_some(),
            child_role: config.child.as_ref().and_then(|c| c.link.role.clone()),
            call_kind: CallKind::Event,
        };
        let registry = Arc::new(Mutex::new(UiRegistry::new(
            oino_ui::slot::Scope::Session,
            Some(session_id.clone()),
        )));
        let ui_hub = Arc::new(UiHub {
            hub: hub.clone(),
            registry: Arc::clone(&registry),
        });
        lock(&registry).set_sink(ui_hub.clone());
        let ui = UiBridge::new(has_ui, session_id.clone(), ui_hub);
        let runner = ExtensionRunner::new(
            Arc::clone(&config.extensions),
            cx,
            Arc::new(HubErrors {
                hub: hub.clone(),
                registry: Arc::clone(&registry),
            }),
            config.flag_values.clone(),
        )
        .with_dialogs(ui.clone());
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
        let image_policy = Arc::new(Mutex::new(ImagePolicy::default()));
        options.convert_to_llm =
            convert_with_block_images(Arc::clone(&config.settings), Arc::clone(&image_policy));
        let agent = Agent::new(options);
        let (idle, _) = watch::channel(true);
        let agents = match config.child {
            Some(child) => crate::agents::AgentsState::child(child),
            None => crate::agents::AgentsState::root(
                Arc::clone(&config.extensions),
                Arc::clone(&config.stream),
            ),
        };
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
            image_policy,
            resources: config.resources,
            models: config.models,
            runner: RwLock::new(runner),
            hub,
            ui,
            registry,
            book: Mutex::new(RunBook::default()),
            cancels: Mutex::new(Cancels::default()),
            tools: Mutex::new(tool_state),
            nested: Mutex::new(nested::NestedState::default()),
            scoped_models: Mutex::new(config.scoped_models),
            idle,
            cwd: config.cwd,
            work_cwd: RwLock::new(work_cwd),
            agent_dir: config.agent_dir,
            docs_root: config.docs_root,
            mode: config.mode,
            subscription: Mutex::new(None),
            agents,
            arbiter: Mutex::new(boundary::ArbiterState::default()),
            runtime: std::sync::OnceLock::new(),
            editor_text: Mutex::new(String::new()),
            frontend: feeds::FrontendState::new(),
            permission: Mutex::new(permission::PermissionState::default()),
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
        lock(&session.inner.book).reminder_model = session
            .model()
            .map(|model| format!("{}/{}", model.provider, model.id));
        if let Err(error) = session.ensure_prompt_prefix() {
            session
                .runner()
                .emit_error("<runtime>", "system_snapshot", error.to_string());
        }
        session
    }

    /// `bindExtensions` tail: emits `session_start` and merges `resources_discover`
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
        // Static `register-renderer{target:"turn-footer"}` / manifest renderers get their
        // registry entry when the instance is bound (§6.7).
        for (extension, spec) in runner.renderers() {
            if spec.get("target").and_then(oino_ai::JsonValue::as_str) == Some("turn-footer")
                && let Some(key) = spec.get("key").and_then(oino_ai::JsonValue::as_str)
            {
                self.ui_register_turn_footer(&extension, key);
            }
        }
        if !runner.shortcuts().is_empty() {
            self.publish_shortcuts();
        }
        // A reopened session (app restart, eviction, reopen) whose background subagents,
        // graph nodes or waking bg tasks run on is `waiting` before any run settles.
        self.refresh_wait_sources().await;
    }

    /// Package resources (manifest `resources` of every loaded extension, package
    /// resources) followed by `resources_discover` results.
    async fn extend_resources_from_extensions(&self, reason: &str) {
        use oino_resources::source::{PathMetadata, SourceOrigin, SourceScope};
        let runner = self.runner();
        let entry =
            |path: PathBuf, extension: &str, origin: SourceOrigin, base: Option<PathBuf>| {
                oino_resources::ExtraResourcePath {
                    path,
                    metadata: PathMetadata {
                        source: format!("extension:{extension}"),
                        scope: SourceScope::Temporary,
                        origin,
                        base_dir: base,
                    },
                }
            };
        let mut paths = oino_resources::ResourceExtensionPaths::default();
        for extension in runner.set().extensions() {
            let id = extension.instance.id();
            let dirs = extension.instance.resource_dirs();
            let package = |list: Vec<PathBuf>| {
                list.into_iter()
                    .map(|dir| {
                        let base = dir.parent().map(Path::to_path_buf);
                        entry(dir, id, SourceOrigin::Package, base)
                    })
                    .collect::<Vec<_>>()
            };
            paths.skill_paths.extend(package(dirs.skills));
            paths.prompt_paths.extend(package(dirs.prompts));
            paths.theme_paths.extend(package(dirs.themes));
        }
        if runner.has_handlers("resources_discover") {
            let found = runner
                .emit_resources_discover(&self.inner.cwd.display().to_string(), reason)
                .await;
            let discovered = |list: Vec<(String, String)>| {
                list.into_iter()
                    .map(|(path, extension)| {
                        entry(
                            PathBuf::from(path),
                            &extension,
                            SourceOrigin::TopLevel,
                            None,
                        )
                    })
                    .collect::<Vec<_>>()
            };
            paths.skill_paths.extend(discovered(found.skill_paths));
            paths.prompt_paths.extend(discovered(found.prompt_paths));
            paths.theme_paths.extend(discovered(found.theme_paths));
        }
        if paths.skill_paths.is_empty()
            && paths.prompt_paths.is_empty()
            && paths.theme_paths.is_empty()
        {
            return;
        }
        self.inner
            .resources
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .extend(&paths);
        let active = self.active_tool_names();
        self.rebuild_system_prompt(&active);
    }

    /// `session_shutdown{reason:"reload"}`, settings and resources reload, tool
    /// registry rebuild, then `session_start{reason:"reload"}` and `resources_discover`.
    ///
    /// Also re-reads what lives outside the session: `models.json` and the provider
    /// credentials, so a model (or key) edited on disk reaches this session's picker
    /// without a host restart, and tells clients to refetch `get_commands` for the
    /// skills / prompts / extension commands that just changed.
    ///
    /// # Errors
    ///
    /// Never fails today; kept fallible for the package reload path.
    pub async fn reload(&self) -> Result<(), SessionOpError> {
        // Reload drops every extension instance: its session entries go with it (§6.6).
        {
            let mut registry = lock(&self.inner.registry);
            registry.clear();
            registry.flush();
        }
        let runner = self.runner();
        if runner.has_handlers("session_shutdown") {
            runner
                .emit(serde_json::json!({"type": "session_shutdown", "reason": "reload"}))
                .await;
        }
        let snapshot = {
            let mut settings = lock(&self.inner.settings);
            settings.reload();
            settings.snapshot()
        };
        self.inner
            .agent
            .set_steering_mode(map_queue_mode(snapshot.steering_mode()));
        self.inner
            .agent
            .set_follow_up_mode(map_queue_mode(snapshot.follow_up_mode()));
        let trusted = lock(&self.inner.settings).is_project_trusted();
        self.inner
            .resources
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .reload(&snapshot, trusted);
        self.refresh_tool_registry(None, true);
        self.bind_extensions("reload", None).await;
        self.inner.models.auth_storage().reload();
        self.inner.models.refresh();
        self.inner.hub.publish(OinoEvent::CommandsChanged);
        Ok(())
    }

    /// Routes host calls of nested agents through `router` (set once by the runtime).
    pub(crate) fn link_router(&self, router: &Arc<crate::services::HostRouter>) {
        let _ = self.inner.agents.router.set(Arc::downgrade(router));
    }

    /// Lets command contexts replace the session through its runtime (set once).
    pub(crate) fn link_runtime(&self, runtime: std::sync::Weak<crate::runtime::RuntimeCore>) {
        let _ = self.inner.runtime.set(runtime);
    }

    pub(crate) fn runtime_core(&self) -> Option<Arc<crate::runtime::RuntimeCore>> {
        self.inner.runtime.get().and_then(std::sync::Weak::upgrade)
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

    /// The cwd the session started in (its header cwd): the system prompt's cwd and the
    /// services' project. It never changes; tools run in [`AgentSession::work_cwd`].
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.inner.cwd
    }

    /// Where the session works now: tools, `!bash`, `@` completions, the git topic and
    /// extension calls. Equals [`AgentSession::cwd`] until the session moves
    /// ([`AgentSession::change_cwd`]).
    #[must_use]
    pub fn work_cwd(&self) -> PathBuf {
        self.inner
            .work_cwd
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
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

    /// Returns whether the session is idle.
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

    /// Transcript length, without copying the transcript (status probes poll it).
    #[must_use]
    pub fn message_count(&self) -> usize {
        self.inner.agent.message_count()
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

    /// Clears both queues and returns their texts.
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

    /// Text of the last assistant message.
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

    /// Rebuilds the finalized transcript from the session projection (the
    /// `_refreshFinalizedContext`).
    pub fn refresh_context(&self) {
        let messages = lock(&self.inner.manager)
            .build_session_projection()
            .messages;
        self.inner.agent.set_messages(self.prefix_view(messages));
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
        } else if !lock(&self.inner.arbiter).waiting_on.is_empty() {
            RunState::Waiting
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

    /// Resolves once the session is idle.
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

    /// Aborts the current operation and waits for idle.
    ///
    /// Abort is unconditional (E-04): pending dialogs are cancelled (a tool future
    /// waiting for an answer the user will never give must unwind with `Cancelled`),
    /// running `!` commands are cancelled, and the idle wait is bounded — a wedged
    /// run must not keep the turn on "Working…" forever; when the bound expires the
    /// run is forced idle so the turn always ends.
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
        self.abort_bash();
        self.cancel_approvals();
        self.inner.ui.cancel_all();
        self.inner.agent.abort();
        if tokio::time::timeout(ABORT_IDLE_BOUND, self.wait_for_idle())
            .await
            .is_err()
        {
            tracing::warn!("abort: the run did not settle in time; forcing the turn to end");
            self.inner.idle.send_replace(true);
        }
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
        for (_, token) in &lock(&self.inner.cancels).bash {
            token.cancel();
        }
    }

    /// Disposes the session: aborts everything and invalidates extension contexts.
    pub async fn dispose(&self) {
        // Boxed: closing a child disposes its session, which closes its own children.
        Box::pin(self.close_children()).await;
        self.abort_retry();
        self.abort_compaction();
        self.abort_branch_summary();
        self.abort_bash();
        self.inner.agent.abort();
        self.runner().invalidate(None);
        self.cancel_approvals();
        self.inner.ui.cancel_all();
        {
            let mut registry = lock(&self.inner.registry);
            registry.clear();
            registry.flush();
        }
        *lock(&self.inner.subscription) = None;
    }

    /// Spawns a session task with per-session panic isolation (X-PANIC, core spec §2.3):
    /// a panic is caught, logged with the session id and published as
    /// [`OinoEvent::SessionCrashed`]. A crashed [`IsolatedTask::Drive`] task additionally
    /// resets the run book and repairs dangling tool calls, so the session stays open and
    /// the next prompt works — state is rebuilt from the log. Other sessions run as
    /// separate tasks and are unaffected.
    ///
    /// Returns the wrapper task's handle; aborting it aborts `fut`.
    pub fn spawn_isolated(
        &self,
        task: IsolatedTask,
        fut: impl Future<Output = ()> + Send + 'static,
    ) -> tokio::task::JoinHandle<()> {
        let session = self.clone();
        tokio::spawn(async move {
            if let Err(panic) = AssertUnwindSafe(fut).catch_unwind().await {
                session.on_isolated_panic(task, &*panic);
            }
        })
    }

    /// Panic recovery (X-PANIC): report, reset, publish.
    fn on_isolated_panic(&self, task: IsolatedTask, panic: &(dyn Any + Send)) {
        let error = panic_message(panic);
        tracing::error!(
            session = %self.session_id(),
            task = task.name(),
            "session task panicked: {error}"
        );
        if task == IsolatedTask::Drive {
            self.recover_crashed_run();
        }
        self.inner.hub.publish(OinoEvent::SessionCrashed { error });
        // The run is over however it ended: tell clients the session is idle again.
        self.inner.hub.publish(OinoEvent::RunState {
            state: RunState::Idle,
            waiting_on: None,
            background_on: None,
        });
        self.set_idle_if_idle();
    }

    /// Resets a run crashed mid-flight (X-PANIC): closes dangling tool calls of the
    /// trailing turn (unknown outcome, D-REPAIR semantics — a no-op for in-memory sessions
    /// and balanced transcripts), clears the run book and cancellation state, and unsticks
    /// the agent so the next `prompt` is accepted.
    fn recover_crashed_run(&self) {
        match self.with_manager(SessionManager::repair_interrupted_turn) {
            Ok(report) if !report.closed_tool_calls.is_empty() || report.torn_tail => {
                tracing::warn!(
                    session = %self.session_id(),
                    "crashed run repaired: {} dangling tool call(s) closed",
                    report.closed_tool_calls.len()
                );
                self.refresh_context();
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!(session = %self.session_id(), "repair after crash failed: {error}");
            }
        }
        {
            let mut book = lock(&self.inner.book);
            book.run_active = false;
            book.abort_requested = false;
            book.overflow_recovery_attempted = false;
            book.retry_attempt = 0;
            book.failed_response = None;
            book.is_before_settle = false;
            book.abort_during_before_settle = false;
            book.emitting_settled = false;
            book.boundary_dispatched = false;
            book.run_prompt_options = None;
            book.tool_result_entries.clear();
        }
        {
            let mut cancels = lock(&self.inner.cancels);
            for token in [
                &cancels.retry,
                &cancels.compaction,
                &cancels.auto_compaction,
                &cancels.branch_summary,
            ]
            .into_iter()
            .flatten()
            {
                token.cancel();
            }
            cancels.retry = None;
            cancels.compaction = None;
            cancels.auto_compaction = None;
            cancels.branch_summary = None;
        }
        {
            let mut arbiter = lock(&self.inner.arbiter);
            arbiter.waiting_on.clear();
            arbiter.background_on.clear();
        }
        self.inner.agent.recover_orphaned_run();
    }

    /// Sets the session name.
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

/// Converts an entry to LLM records, blocking images.
#[derive(Default)]
pub(crate) struct ImagePolicy {
    /// Entry id is durable identity; original message is the conversion lookup.
    entries: Vec<(EntryId, AgentMessage, bool)>,
    current: Option<bool>,
}

impl AgentSession {
    /// Pin image serialization before first provider use. Policy records are
    /// append-only, so reopen, settings toggles and compaction cannot reserialize
    /// an already-seen entry with today's policy. Source content is never edited.
    pub(crate) fn snapshot_image_policy(
        &self,
        messages: &[AgentMessage],
    ) -> Result<(), SessionOpError> {
        let blocked = lock(&self.inner.settings)
            .settings()
            .image_settings()
            .block_images;
        let mut policy = lock(&self.inner.image_policy);
        let mut manager = lock(&self.inner.manager);
        let branch = manager.branch(None);
        let mut recorded = std::collections::HashMap::new();
        let mut previous = None;
        for entry in &branch {
            if let oino_session::EntryKind::Custom {
                custom_type,
                data: Some(data),
            } = &entry.kind
            {
                if custom_type == "oino.image-policy" {
                    if let (Some(id), Some(blocked)) =
                        (data["entryId"].as_str(), data["blocked"].as_bool())
                    {
                        recorded.insert(id.to_owned(), blocked);
                    }
                } else if custom_type == "oino.image-setting" {
                    previous = data["blocked"].as_bool();
                }
            }
        }
        let changed = policy
            .current
            .or(previous)
            .is_some_and(|old| old != blocked);
        let mut pending = Vec::new();
        let mut pending_messages = Vec::new();
        for message in messages {
            if !matches!(
                message,
                AgentMessage::User(_) | AgentMessage::ToolResult(_) | AgentMessage::Custom(_)
            ) {
                continue;
            }
            let entry = branch.iter().find(|entry| {
                oino_session::context::entry_to_context_messages(entry, None).contains(message)
            });
            let Some(entry) = entry else {
                continue;
            };
            if policy.entries.iter().any(|(id, _, _)| id == &entry.id) {
                continue;
            }
            let pinned = recorded.get(entry.id.as_str()).copied().unwrap_or(blocked);
            if !recorded.contains_key(entry.id.as_str()) {
                pending.push((entry.id.clone(), pinned));
            }
            // Commit lookup state only after all metadata appends succeed.
            pending_messages.push((entry.id.clone(), message.clone(), pinned));
        }
        for (id, pinned) in pending {
            manager
                .append_custom_entry(
                    "oino.image-policy",
                    Some(serde_json::json!({
                        "entryId":id, "blocked":pinned,
                    })),
                )
                .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        }
        if policy.current.or(previous) != Some(blocked) {
            manager
                .append_custom_entry(
                    "oino.image-setting",
                    Some(serde_json::json!({"blocked":blocked})),
                )
                .map_err(|e| SessionOpError::Failed(e.to_string()))?;
        }
        drop(manager);
        if changed {
            self.append_reminder("oino", "oino/image-policy", &format!(
                "Image reading is now {} for new content. Previously sent content retains its original image policy.",
                if blocked { "disabled" } else { "enabled" }))?;
        }
        policy.entries.extend(pending_messages);
        policy.current = Some(blocked);
        Ok(())
    }
}

fn convert_with_block_images(
    settings: Arc<Mutex<SettingsManager>>,
    policy: Arc<Mutex<ImagePolicy>>,
) -> oino_agent::ConvertToLlm {
    Arc::new(move |messages: &[AgentMessage]| {
        messages
            .iter()
            .filter_map(|message| {
                let mut converted = oino_session::messages::to_llm(message)?;
                let trusted = matches!(message, AgentMessage::Custom(c)
                if c.custom_type == prompt::REMINDER_TYPE || c.custom_type == "oino.session-start");
                if !trusted {
                    oino_ai::transcript::escape_message_reminders(&mut converted);
                }
                let blocked = lock(&policy)
                    .entries
                    .iter()
                    .find(|(_, original, _)| original == message)
                    .map_or_else(
                        || lock(&settings).settings().image_settings().block_images,
                        |(_, _, blocked)| *blocked,
                    );
                Some(if blocked {
                    block_images(converted)
                } else {
                    converted
                })
            })
            .collect::<Vec<_>>()
    })
}

use std::time::Duration;

/// How long `abort` waits for the run to settle before forcing the turn to end (E-04:
/// Stop must always win, even over a tool future that ignores cancellation).
const ABORT_IDLE_BOUND: Duration = Duration::from_secs(15);

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
