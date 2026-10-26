//! `AgentSessionRuntime` (pi `agent-session-runtime.ts` + `sdk.ts` `createAgentSession`):
//! owns the current [`AgentSession`], replaces it on new/switch/fork/clone/import, and keeps
//! one client event stream across replacements.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, Weak};

use futures::StreamExt;
use oino_ai::{Model, ThinkingLevel};
use oino_models::resolver::{InitialModelOptions, ScopedModel, find_initial_model};
use oino_models::virtual_models::branch_selection;
use oino_session::{CreateOptions, EntryId, EntryKind, SessionEntry, SessionManager};
use serde_json::json;
use tokio::task::JoinHandle;

use crate::events::{ModelChangeSource, OinoEvent, ReplaceReason};
use crate::hub::{EventHub, EventStream};
use crate::services::{Services, ServicesError};
use crate::session::{AgentSession, SessionConfig, SessionOpError, ToolEntry, lock};

/// Where the session comes from (runtime §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionSource {
    /// A new persisted session.
    New,
    /// Open a session file.
    Open(PathBuf),
    /// Continue the most recent session of the cwd.
    ContinueRecent,
    /// Resume by id (exact, then unique prefix).
    Resume(String),
    /// Fork a session file into a new one.
    Fork(PathBuf),
    /// Not persisted.
    InMemory,
}

/// Tool selection (pi `--tools`, `-nt`, `-nbt`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum ToolSelection {
    /// `defaultTools` setting.
    #[default]
    Default,
    /// Explicit list (`+name`/`-name` entries modify the defaults).
    List(Vec<String>),
    /// `--no-tools`.
    None,
    /// `--no-builtin-tools`.
    NoBuiltin,
}

/// Inputs of [`AgentSessionRuntime::create`].
#[derive(Clone)]
pub struct SessionOptions {
    /// Session source.
    pub source: SessionSource,
    /// `--session-dir`.
    pub session_dir: Option<PathBuf>,
    /// `--name`.
    pub name: Option<String>,
    /// Explicit model (`--model`, resolved by the caller).
    pub model: Option<Model>,
    /// Explicit thinking level.
    pub thinking_level: Option<ThinkingLevel>,
    /// `--models` scope.
    pub scoped_models: Vec<ScopedModel>,
    /// Tool selection.
    pub tools: ToolSelection,
    /// `--exclude-tools`.
    pub exclude_tools: Vec<String>,
    /// Built-in tools available to the session.
    pub builtin_tools: Vec<ToolEntry>,
    /// SDK custom tools.
    pub custom_tools: Vec<ToolEntry>,
}

impl std::fmt::Debug for SessionOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionOptions")
            .field("source", &self.source)
            .field("tools", &self.tools)
            .finish_non_exhaustive()
    }
}

impl Default for SessionOptions {
    fn default() -> Self {
        Self {
            source: SessionSource::New,
            session_dir: None,
            name: None,
            model: None,
            thinking_level: None,
            scoped_models: Vec::new(),
            tools: ToolSelection::Default,
            exclude_tools: Vec::new(),
            builtin_tools: Vec::new(),
            custom_tools: Vec::new(),
        }
    }
}

/// pi `{cancelled}` result of replacements.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Replaced {
    /// An extension cancelled the replacement.
    pub cancelled: bool,
    /// Text of the forked-from user message (fork `Before`).
    pub selected_text: Option<String>,
}

/// Fork position.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ForkPosition {
    /// Before the user message (its text is returned for editing).
    #[default]
    Before,
    /// At the entry (included).
    At,
}

struct Current {
    session: AgentSession,
    forward: JoinHandle<()>,
}

/// Owns the current session (pi `AgentSessionRuntime`). Extension command contexts reach it
/// through a weak link for `newSession` / `fork` / `switchSession` / `shutdown`.
pub struct AgentSessionRuntime {
    core: Arc<RuntimeCore>,
}

/// Shared state of an [`AgentSessionRuntime`].
pub(crate) struct RuntimeCore {
    services: Arc<Services>,
    hub: EventHub,
    current: Mutex<Current>,
    options: SessionOptions,
    model_fallback_message: std::sync::OnceLock<String>,
    shutdown: tokio::sync::watch::Sender<bool>,
    me: Weak<RuntimeCore>,
}

impl std::fmt::Debug for AgentSessionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentSessionRuntime")
            .finish_non_exhaustive()
    }
}

fn session_error(error: impl std::fmt::Display) -> ServicesError {
    ServicesError::Session(error.to_string())
}

fn op_error(error: impl std::fmt::Display) -> SessionOpError {
    SessionOpError::Failed(error.to_string())
}

/// The model and thinking level a session starts with (pi `createAgentSession`).
fn initial_selection(
    services: &Services,
    manager: &SessionManager,
    options: &SessionOptions,
) -> (Option<Model>, ThinkingLevel, Option<String>) {
    let context = manager.build_session_context();
    let has_existing = !context.messages.is_empty();
    let branch: Vec<SessionEntry> = manager.branch(None).into_iter().cloned().collect();
    let has_thinking_entry = branch
        .iter()
        .any(|e| matches!(e.kind, EntryKind::ThinkingLevelChange { .. }));
    let models = &services.models;
    let settings = lock(&services.settings).snapshot();
    let mut model = options.model.clone();
    let mut fallback = None;
    if model.is_none() && has_existing {
        let refs = crate::session::model_branch_refs(&branch);
        if let Some(selection) =
            branch_selection(&refs, |p, id| models.virtual_model(p, id).is_some())
        {
            match models.find(&selection.provider, &selection.model_id) {
                Some(found) if models.has_configured_auth(&found.provider) => model = Some(found),
                _ => {
                    fallback = Some(format!(
                        "Could not restore model {}/{}",
                        selection.provider, selection.model_id
                    ));
                }
            }
        }
    }
    if model.is_none() {
        let levels = settings.model_thinking_levels();
        let initial = find_initial_model(
            &InitialModelOptions {
                scoped_models: &options.scoped_models,
                is_continuing: has_existing,
                default_provider: settings.default_provider(),
                default_model_id: settings.default_model(),
                default_thinking_level: settings.default_thinking_level(),
                model_thinking_levels: Some(&levels),
            },
            models,
        );
        model = initial.model;
        match (&model, fallback.take()) {
            (None, _) => {
                fallback = Some(oino_models::auth::guidance::no_models_available_message(
                    &services.docs_root.join("docs"),
                ));
            }
            (Some(m), Some(message)) => {
                fallback = Some(format!("{message}. Using {}/{}", m.provider, m.id));
            }
            (Some(_), None) => {}
        }
    }
    let default_level = settings
        .default_thinking_level()
        .unwrap_or(ThinkingLevel::Medium);
    let mut thinking = options.thinking_level;
    if thinking.is_none() && has_existing {
        thinking = Some(if has_thinking_entry {
            context.thinking_level.parse().unwrap_or(default_level)
        } else {
            default_level
        });
    }
    if thinking.is_none() {
        thinking = model
            .as_ref()
            .and_then(|m| settings.model_thinking_level(&m.provider, &m.id));
    }
    let thinking = thinking.unwrap_or(default_level);
    let thinking = model.as_ref().map_or(ThinkingLevel::Off, |m| {
        thinking.clamp(&m.available_thinking_levels())
    });
    (model, thinking, fallback)
}

fn tool_names(
    options: &SessionOptions,
    settings: &oino_settings::Settings,
) -> (Vec<String>, Option<Vec<String>>) {
    let defaults = settings.default_tools().unwrap_or_else(|| {
        oino_settings::tools::DEFAULT_TOOL_NAMES
            .iter()
            .map(|s| (*s).to_owned())
            .collect()
    });
    match &options.tools {
        ToolSelection::Default => (defaults, None),
        ToolSelection::None => (Vec::new(), Some(Vec::new())),
        ToolSelection::NoBuiltin => (Vec::new(), None),
        ToolSelection::List(entries) => {
            if entries
                .iter()
                .any(|e| oino_settings::tools::is_tool_modifier(e))
            {
                (
                    oino_settings::tools::apply_tool_modifiers(&defaults, entries),
                    None,
                )
            } else {
                (entries.clone(), Some(entries.clone()))
            }
        }
    }
}

async fn install(
    services: &Arc<Services>,
    options: &SessionOptions,
    hub: &EventHub,
    runtime: Option<&Weak<RuntimeCore>>,
    mut manager: SessionManager,
    reason: &str,
    previous: Option<PathBuf>,
) -> Result<(Current, Option<String>), ServicesError> {
    let (model, thinking, fallback) = initial_selection(services, &manager, options);
    let has_existing = !manager.build_session_context().messages.is_empty();
    let has_thinking_entry = manager
        .branch(None)
        .iter()
        .any(|e| matches!(e.kind, EntryKind::ThinkingLevelChange { .. }));
    if has_existing {
        if !has_thinking_entry {
            manager
                .append_thinking_level_change(&thinking.to_string())
                .map_err(session_error)?;
        }
    } else {
        if let Some(model) = &model {
            manager
                .append_model_change(&model.provider, &model.id)
                .map_err(session_error)?;
        }
        manager
            .append_thinking_level_change(&thinking.to_string())
            .map_err(session_error)?;
    }
    let settings = lock(&services.settings).snapshot();
    let (initial, allowed) = tool_names(options, &settings);
    let excluded = &options.exclude_tools;
    let initial: Vec<String> = initial
        .into_iter()
        .filter(|n| !excluded.contains(n))
        .collect();
    let builtin = if matches!(
        options.tools,
        ToolSelection::NoBuiltin | ToolSelection::None
    ) {
        Vec::new()
    } else {
        options.builtin_tools.clone()
    };
    let session = AgentSession::new(SessionConfig {
        cwd: services.cwd.clone(),
        agent_dir: services.agent_dir.clone(),
        docs_root: services.docs_root.clone(),
        manager,
        settings: Arc::clone(&services.settings),
        resources: Arc::clone(&services.resources),
        models: services.models.clone(),
        extensions: Arc::clone(&services.extensions),
        stream: Arc::clone(&services.stream) as Arc<dyn oino_agent::StreamFn>,
        mode: services.mode,
        has_ui: services.has_ui,
        builtin_tools: builtin,
        custom_tools: options.custom_tools.clone(),
        initial_active_tools: initial,
        allowed_tools: allowed,
        excluded_tools: excluded.clone(),
        model,
        thinking_level: thinking,
        scoped_models: options.scoped_models.clone(),
        flag_values: services.flag_values.clone(),
        child: None,
    });
    if matches!(options.tools, ToolSelection::Default) {
        session.restore_tools_from_transcript(&[]);
    }
    services.host.attach(&session);
    session.link_router(&services.host);
    if let Some(runtime) = runtime {
        session.link_runtime(runtime.clone());
    }
    let mut stream = session.subscribe();
    let hub = hub.clone();
    let forward = tokio::spawn(async move {
        while let Some(event) = stream.next().await {
            hub.publish(event);
        }
    });
    session.bind_extensions(reason, previous).await;
    if reason == "startup"
        && let Some(model) = session.model()
    {
        let runner = session.runner();
        if runner.has_handlers("model_select") {
            runner
                .emit(json!({"type": "model_select", "model": model, "source": "restore"}))
                .await;
        }
        session.emit_model_changed(ModelChangeSource::Restore);
    }
    Ok((Current { session, forward }, fallback))
}

impl AgentSessionRuntime {
    /// Creates the runtime and its first session; emits `session_start{reason:"startup"}`,
    /// `resources_discover` and the model restore.
    ///
    /// # Errors
    ///
    /// Session file failures.
    pub async fn create(
        services: Arc<Services>,
        options: SessionOptions,
    ) -> Result<Self, ServicesError> {
        let root = services.sessions_root(options.session_dir.as_deref());
        let manager = match &options.source {
            SessionSource::New => {
                SessionManager::create(&services.cwd, &root, CreateOptions::default())
            }
            SessionSource::Open(path) => SessionManager::open(path, &root, None),
            SessionSource::ContinueRecent => SessionManager::continue_recent(&services.cwd, &root),
            SessionSource::Resume(id) => {
                match SessionManager::find_by_id(&services.cwd, id, &root).map_err(session_error)? {
                    Some(path) => SessionManager::open(&path, &root, None),
                    None => {
                        return Err(ServicesError::Session(format!(
                            "No session found matching '{id}'"
                        )));
                    }
                }
            }
            SessionSource::Fork(path) => {
                SessionManager::fork_from(path, &services.cwd, &root, CreateOptions::default())
            }
            SessionSource::InMemory => Ok(SessionManager::in_memory(
                &services.cwd,
                CreateOptions::default(),
                Vec::new(),
            )),
        }
        .map_err(session_error)?;
        let hub = EventHub::new();
        let (current, fallback) =
            install(&services, &options, &hub, None, manager, "startup", None).await?;
        let core = Arc::new_cyclic(|me: &Weak<RuntimeCore>| RuntimeCore {
            services: Arc::clone(&services),
            hub: hub.clone(),
            current: Mutex::new(current),
            options,
            model_fallback_message: std::sync::OnceLock::new(),
            shutdown: tokio::sync::watch::channel(false).0,
            me: me.clone(),
        });
        if let Some(fallback) = fallback {
            let _ = core.model_fallback_message.set(fallback);
        }
        core.session().link_runtime(core.me.clone());
        let runtime = Self { core };
        if let Some(name) = runtime.core.options.name.clone() {
            runtime
                .session()
                .set_session_name(Some(name))
                .await
                .map_err(session_error)?;
        }
        Ok(runtime)
    }

    /// pi `modelFallbackMessage`.
    #[must_use]
    pub fn model_fallback_message(&self) -> Option<&str> {
        self.core.model_fallback_message.get().map(String::as_str)
    }

    /// The services.
    #[must_use]
    pub fn services(&self) -> &Arc<Services> {
        &self.core.services
    }

    /// The current session.
    #[must_use]
    pub fn session(&self) -> AgentSession {
        self.core.session()
    }

    /// Client events across session replacements.
    #[must_use]
    pub fn subscribe(&self) -> EventStream {
        self.core.hub.subscribe()
    }

    /// Becomes `true` when an extension called `ctx.shutdown()` (pi `shutdownHandler`);
    /// frontends exit once the session is idle.
    #[must_use]
    pub fn shutdown_requested(&self) -> tokio::sync::watch::Receiver<bool> {
        self.core.shutdown.subscribe()
    }

    /// pi `newSession`.
    ///
    /// # Errors
    ///
    /// Session file failures.
    pub async fn new_session(&self, parent: Option<PathBuf>) -> Result<Replaced, SessionOpError> {
        self.core.new_session(parent).await
    }

    /// pi `switchSession`.
    ///
    /// # Errors
    ///
    /// The file cannot be opened.
    pub async fn switch_session(
        &self,
        path: PathBuf,
        cwd_override: Option<PathBuf>,
    ) -> Result<Replaced, SessionOpError> {
        self.core.switch_session(path, cwd_override).await
    }

    /// pi `fork`.
    ///
    /// # Errors
    ///
    /// `Invalid entry ID for forking` and session file failures.
    pub async fn fork(
        &self,
        entry: &EntryId,
        position: ForkPosition,
    ) -> Result<Replaced, SessionOpError> {
        self.core
            .fork_with_reason(entry, position, ReplaceReason::Fork)
            .await
    }

    /// Clones the session at its leaf (fork `At` the leaf).
    ///
    /// # Errors
    ///
    /// As [`AgentSessionRuntime::fork`]; `Nothing to clone` for an empty session.
    pub async fn clone_session(&self) -> Result<Replaced, SessionOpError> {
        let leaf = self
            .session()
            .with_manager(|m| oino_session::ReadonlySession::leaf_id(m).cloned())
            .ok_or_else(|| SessionOpError::Failed("Nothing to clone".into()))?;
        self.core
            .fork_with_reason(&leaf, ForkPosition::At, ReplaceReason::Clone)
            .await
    }

    /// pi `importFromJsonl`: copies the file into the sessions dir and switches to it.
    ///
    /// # Errors
    ///
    /// Missing file or copy failure.
    pub async fn import_jsonl(
        &self,
        path: PathBuf,
        cwd_override: Option<PathBuf>,
    ) -> Result<Replaced, SessionOpError> {
        self.core.import_jsonl(path, cwd_override).await
    }

    /// Shuts the current session down (`session_shutdown{reason:"quit"}`).
    pub async fn dispose(&self) {
        self.core.teardown("quit", None).await;
        self.core.hub.close();
    }
}

impl RuntimeCore {
    pub(crate) fn session(&self) -> AgentSession {
        lock(&self.current).session.clone()
    }

    /// `ctx.shutdown()`.
    pub(crate) fn request_shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    async fn before_switch(&self, reason: &str, target: Option<&Path>) -> bool {
        let runner = self.session().runner();
        if !runner.has_handlers("session_before_switch") {
            return false;
        }
        let mut payload = json!({"type": "session_before_switch", "reason": reason});
        if let Some(target) = target {
            payload["targetSessionFile"] = json!(target.display().to_string());
        }
        runner
            .emit(payload)
            .await
            .is_some_and(|r| r.get("cancel") == Some(&serde_json::Value::Bool(true)))
    }

    async fn teardown(&self, reason: &str, target: Option<&Path>) {
        let session = self.session();
        session.abort().await;
        let runner = session.runner();
        if runner.has_handlers("session_shutdown") {
            let mut payload = json!({"type": "session_shutdown", "reason": reason});
            if let Some(target) = target {
                payload["targetSessionFile"] = json!(target.display().to_string());
            }
            runner.emit(payload).await;
        }
        self.services.host.detach(&session.session_id());
        session.dispose().await;
    }

    async fn swap(
        &self,
        manager: SessionManager,
        reason: &str,
        previous: Option<PathBuf>,
    ) -> Result<(), SessionOpError> {
        let (next, _) = install(
            &self.services,
            &self.options,
            &self.hub,
            Some(&self.me),
            manager,
            reason,
            previous,
        )
        .await
        .map_err(op_error)?;
        let old = std::mem::replace(&mut *lock(&self.current), next);
        old.forward.abort();
        Ok(())
    }

    fn replaced(&self, reason: ReplaceReason) {
        let session = self.session();
        self.hub.publish(OinoEvent::SessionReplaced {
            session_file: session.session_file().map(|p| p.display().to_string()),
            session_id: session.session_id(),
            reason,
        });
    }

    fn persisted(&self) -> (bool, PathBuf) {
        self.session()
            .with_manager(|m| (m.is_persisted(), m.session_dir().to_path_buf()))
    }

    /// pi `newSession`.
    ///
    /// # Errors
    ///
    /// Session file failures.
    pub(crate) async fn new_session(
        &self,
        parent: Option<PathBuf>,
    ) -> Result<Replaced, SessionOpError> {
        if self.before_switch("new", None).await {
            return Ok(Replaced {
                cancelled: true,
                selected_text: None,
            });
        }
        let previous = self.session().session_file();
        let (persisted, root) = self.persisted();
        let options = CreateOptions {
            id: None,
            parent_session: parent,
        };
        let manager = if persisted {
            SessionManager::create(&self.services.cwd, &root, options).map_err(op_error)?
        } else {
            SessionManager::in_memory(&self.services.cwd, options, Vec::new())
        };
        let target = manager.session_file().map(Path::to_path_buf);
        self.teardown("new", target.as_deref()).await;
        self.swap(manager, "new", previous).await?;
        self.replaced(ReplaceReason::New);
        Ok(Replaced::default())
    }

    /// pi `switchSession`.
    ///
    /// # Errors
    ///
    /// The file cannot be opened.
    pub(crate) async fn switch_session(
        &self,
        path: PathBuf,
        cwd_override: Option<PathBuf>,
    ) -> Result<Replaced, SessionOpError> {
        if self.before_switch("resume", Some(&path)).await {
            return Ok(Replaced {
                cancelled: true,
                selected_text: None,
            });
        }
        let previous = self.session().session_file();
        let (_, root) = self.persisted();
        let manager =
            SessionManager::open(&path, &root, cwd_override.as_deref()).map_err(op_error)?;
        let target = manager.session_file().map(Path::to_path_buf);
        self.teardown("resume", target.as_deref()).await;
        self.swap(manager, "resume", previous).await?;
        self.replaced(ReplaceReason::Resume);
        Ok(Replaced::default())
    }

    pub(crate) async fn fork_with_reason(
        &self,
        entry: &EntryId,
        position: ForkPosition,
        reason: ReplaceReason,
    ) -> Result<Replaced, SessionOpError> {
        let session = self.session();
        let runner = session.runner();
        let position_name = match position {
            ForkPosition::Before => "before",
            ForkPosition::At => "at",
        };
        if runner.has_handlers("session_before_fork") {
            let result = runner
                .emit(json!({"type": "session_before_fork", "entryId": entry.as_str(), "position": position_name}))
                .await;
            if result.is_some_and(|r| r.get("cancel") == Some(&serde_json::Value::Bool(true))) {
                return Ok(Replaced {
                    cancelled: true,
                    selected_text: None,
                });
            }
        }
        let selected = session
            .with_manager(|m| m.entry(entry).cloned())
            .ok_or_else(|| SessionOpError::Failed("Invalid entry ID for forking".into()))?;
        let (leaf, selected_text) = match position {
            ForkPosition::At => (Some(selected.id.clone()), None),
            ForkPosition::Before => match selected.message() {
                Some(oino_ai::AgentMessage::User(user)) => {
                    (selected.parent_id.clone(), Some(user.content.text("\n")))
                }
                _ => {
                    return Err(SessionOpError::Failed(
                        "Invalid entry ID for forking".into(),
                    ));
                }
            },
        };
        let previous = session.session_file();
        let mut manager = session.with_manager(|m| m.clone());
        match &leaf {
            None => manager
                .new_session(CreateOptions {
                    id: None,
                    parent_session: previous.clone(),
                })
                .map_err(op_error)?,
            Some(leaf) => {
                manager.branch_into_new_session(leaf).map_err(op_error)?;
            }
        }
        let target = manager.session_file().map(Path::to_path_buf);
        self.teardown("fork", target.as_deref()).await;
        self.swap(manager, "fork", previous).await?;
        self.replaced(reason);
        Ok(Replaced {
            cancelled: false,
            selected_text,
        })
    }

    /// pi `importFromJsonl`: copies the file into the sessions dir and switches to it.
    ///
    /// # Errors
    ///
    /// Missing file or copy failure.
    pub(crate) async fn import_jsonl(
        &self,
        path: PathBuf,
        cwd_override: Option<PathBuf>,
    ) -> Result<Replaced, SessionOpError> {
        if !path.exists() {
            return Err(SessionOpError::Failed(format!(
                "File not found: {}",
                path.display()
            )));
        }
        let (_, root) = self.persisted();
        let dir = oino_session::manager::cwd_session_dir(&root, &self.services.cwd);
        tokio::fs::create_dir_all(&dir).await.map_err(op_error)?;
        let destination = dir.join(path.file_name().unwrap_or_default());
        if destination != path {
            tokio::fs::copy(&path, &destination)
                .await
                .map_err(op_error)?;
        }
        let result = self.switch_session(destination, cwd_override).await?;
        if !result.cancelled {
            self.replaced(ReplaceReason::Import);
        }
        Ok(result)
    }
}
