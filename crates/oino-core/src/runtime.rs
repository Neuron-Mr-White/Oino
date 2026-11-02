//! `AgentSessionRuntime` (`agent-session-runtime` + `createAgentSession`):
//! owns the current [`AgentSession`], replaces it on new/switch/fork/clone/import, and keeps
//! one client event stream across replacements.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures::StreamExt;
use oino_ai::{Model, ThinkingLevel};
use oino_models::resolver::{InitialModelOptions, ScopedModel, find_initial_model};
use oino_models::virtual_models::branch_selection;
use oino_session::{
    CreateOptions, EntryId, EntryKind, LeaseInfo, SessionEntry, SessionError, SessionLease,
    SessionLockError, SessionManager, clear_takeover_request, read_takeover_request,
    write_takeover_request,
};
use serde_json::json;
use tokio::task::JoinHandle;

use crate::events::{ModelChangeSource, OinoEvent, ReplaceReason};
use crate::extensions::api::ExtensionMode;
use crate::hub::{EventHub, EventStream};
use crate::services::{Services, ServicesError};
use crate::session::{AgentSession, SessionConfig, SessionOpError, ToolEntry, lock};

/// Where the session comes from (runtime §4).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionSource {
    /// A new persisted session.
    New,
    /// A new persisted session with an explicit header id (`--session-id`; the id is created
    /// when no session has it yet).
    NewWithId(String),
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

/// Tool selection (`--tools`, `-nt`, `-nbt`).
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
    /// The session's own working directory when it differs from the services' project
    /// (`services.cwd`): an embedding hub may run several per-session folders (the
    /// desktop's "No workspace" scratch dirs) on one shared services set. New sessions
    /// record it as their header cwd and the system prompt names it. `None` =
    /// `services.cwd`.
    pub cwd: Option<PathBuf>,
}

impl SessionOptions {
    /// The session's cwd: [`SessionOptions::cwd`], else the services' project.
    fn session_cwd<'a>(&'a self, services: &'a Services) -> &'a Path {
        self.cwd.as_deref().unwrap_or(&services.cwd)
    }
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
            cwd: None,
        }
    }
}

/// The `{cancelled}` result of replacements.
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

/// Owns the current session. Extension command contexts reach it
/// through a weak link for `newSession` / `fork` / `switchSession` / `shutdown`.
pub struct AgentSessionRuntime {
    core: Arc<RuntimeCore>,
}

fn old_session(current: &Mutex<Current>) -> AgentSession {
    lock(current).session.clone()
}

/// Shared state of an [`AgentSessionRuntime`].
pub(crate) struct RuntimeCore {
    services: Arc<Services>,
    hub: EventHub,
    current: Mutex<Current>,
    options: SessionOptions,
    model_fallback_message: std::sync::OnceLock<String>,
    shutdown: tokio::sync::watch::Sender<bool>,
    /// Single-writer lease of the current session (D-LOCK); `None` for in-memory sessions.
    lease: Mutex<Option<SessionLease>>,
    /// Bumped whenever `lease` changes; stale takeover watchers exit on a mismatch.
    lease_epoch: AtomicU64,
    /// Embedding override of the default takeover behaviour (D-TAKEOVER).
    takeover_handler: Mutex<Option<TakeoverHandler>>,
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

/// D-LOCK / D-TAKEOVER timings (spec core.md §2.1): retry window after a certainly-dead
/// holder (kernel release lag), takeover poll interval, takeover deadline and the holder-side
/// watcher poll.
const DEAD_HOLDER_RETRY: Duration = Duration::from_secs(2);
const TAKEOVER_POLL: Duration = Duration::from_millis(200);
const TAKEOVER_DEADLINE: Duration = Duration::from_secs(10);
const TAKEOVER_WATCH_POLL: Duration = Duration::from_millis(500);

/// Overrides the default D-TAKEOVER holder behaviour (`on_takeover`): called instead of
/// "close this session and keep the runtime usable" — e.g. WP-C7's hub closes its backend
/// and notifies its client.
pub type TakeoverHandler =
    Arc<dyn Fn(LeaseInfo) -> futures::future::BoxFuture<'static, ()> + Send + Sync>;

/// Process identity for lease metadata (D-LOCK): `oino <mode>` holding a pid/host/boot id.
pub(crate) fn process_lease_info(agent_dir: &Path, mode: ExtensionMode) -> LeaseInfo {
    let mode = match mode {
        ExtensionMode::Tui => "tui",
        ExtensionMode::Rpc => "rpc",
        ExtensionMode::Json => "json",
        ExtensionMode::Print => "print",
        ExtensionMode::Acp => "acp",
        ExtensionMode::Desktop => "desktop",
        ExtensionMode::Mobile => "mobile",
    };
    LeaseInfo::for_process(&format!("oino {mode}"), &agent_dir.join("state"))
}

/// The D-TAKEOVER text when a holder does not respond: precise, names the holder.
fn locked_text(session_id: &str, holder: &LeaseInfo) -> String {
    format!(
        "Session {session_id} is open in {} (pid {} on {}). It was asked to close and did not \
         respond; close it there and try again.",
        holder.client, holder.pid, holder.host
    )
}

/// Wraps a session-manager failure as [`SessionOpError::Failed`].
fn session_op(error: impl std::fmt::Display) -> SessionOpError {
    SessionOpError::Failed(error.to_string())
}

/// Acquires the single-writer lease of `session_file`, taking an unresponsive holder over
/// (D-LOCK / D-TAKEOVER): (1) a certainly-dead holder gets a ≤2 s retry window (kernel
/// release lag), (2) a live one is asked through `<file>.takeover` to close — its watcher
/// disposes the session and releases within the 10 s deadline —, (3) past the deadline the
/// precise [`SessionOpError::SessionLocked`] names the holder. The kernel lock remains the
/// arbiter throughout; there is no lease expiry.
pub(crate) async fn acquire_lease(
    session_file: &Path,
    holder: &LeaseInfo,
    session_id: &str,
) -> Result<SessionLease, SessionOpError> {
    let attempt = || SessionLease::acquire(session_file, holder);
    let mut held_by = match attempt() {
        Ok(lease) => return Ok(lease),
        Err(SessionLockError::HeldBy(info)) => info,
        Err(SessionLockError::Io { path, source }) => {
            return Err(SessionOpError::Failed(format!(
                "session lock I/O error on {}: {source}",
                path.display()
            )));
        }
    };
    // (1) certainly-dead holder: the kernel releases on death, retry out its lag. A live
    // holder appearing mid-window falls through to the takeover phase.
    if held_by.dead() {
        let deadline = Instant::now() + DEAD_HOLDER_RETRY;
        while Instant::now() < deadline {
            tokio::time::sleep(TAKEOVER_POLL).await;
            match attempt() {
                Ok(lease) => return Ok(lease),
                Err(SessionLockError::HeldBy(info)) => {
                    held_by = info;
                    if !held_by.dead() {
                        break;
                    }
                }
                Err(SessionLockError::Io { .. }) => break,
            }
        }
    }
    // (2) ask the holder to close and reload here (D-TAKEOVER), polling the kernel lock.
    if write_takeover_request(session_file, holder).is_err() {
        tracing::warn!(
            session_file = %session_file.display(),
            "cannot write the takeover request; polling the lock only"
        );
    }
    let deadline = Instant::now() + TAKEOVER_DEADLINE;
    loop {
        if Instant::now() >= deadline {
            clear_takeover_request(session_file);
            return Err(SessionOpError::SessionLocked(locked_text(
                session_id, &held_by,
            )));
        }
        tokio::time::sleep(TAKEOVER_POLL).await;
        match attempt() {
            Ok(lease) => {
                clear_takeover_request(session_file);
                return Ok(lease);
            }
            Err(SessionLockError::HeldBy(info)) => held_by = info,
            Err(SessionLockError::Io { path, source }) => {
                clear_takeover_request(session_file);
                return Err(SessionOpError::Failed(format!(
                    "session lock I/O error on {}: {source}",
                    path.display()
                )));
            }
        }
    }
}

/// Binds the lease of `manager`'s session file (D-LOCK ownership rule: the runtime holds the
/// lease of the current session, children hold their own). New-session paths have unique
/// planned names, so acquisition is uncontended there.
async fn bind_lease(
    services: &Services,
    manager: &SessionManager,
) -> Result<Option<SessionLease>, SessionOpError> {
    let Some(file) = manager.session_file().filter(|_| manager.is_persisted()) else {
        return Ok(None);
    };
    let holder = process_lease_info(&services.agent_dir, services.mode);
    acquire_lease(file, &holder, manager.session_id())
        .await
        .map(Some)
}

/// Opens a file-backed session under its lease (D-TAKEOVER step 4: acquire, open, repair):
/// the first open resolves migrations and the canonical path, the lease is acquired with
/// takeover, and the file is re-read + repaired under the lease so nothing the previous
/// holder appended after the first read is missed.
async fn open_under_lease(
    services: &Services,
    path: &Path,
    root: &Path,
    cwd_override: Option<&Path>,
) -> Result<(SessionManager, Option<SessionLease>), SessionOpError> {
    let first = SessionManager::open(path, root, cwd_override).map_err(session_op)?;
    let lease = bind_lease(services, &first).await?;
    if lease.is_none() {
        return Ok((first, None));
    }
    let file = first.session_file().map(Path::to_path_buf);
    let manager = match file {
        Some(file) => SessionManager::open(&file, root, cwd_override).map_err(session_op)?,
        None => first,
    };
    let manager = repaired(manager).map_err(session_op)?;
    Ok((manager, lease))
}

/// Wraps a freshly created manager (new/fork sources) with its lease; no repair (empty or
/// copy-on-write content).
async fn fresh_under_lease(
    services: &Services,
    manager: SessionManager,
) -> Result<(SessionManager, Option<SessionLease>), SessionOpError> {
    let lease = bind_lease(services, &manager).await?;
    Ok((manager, lease))
}

/// Repairs the interrupted turn of a just-opened session (D-REPAIR, WP-C2): every write
/// open of an existing session file repairs before the session is used — a torn tail
/// line is terminated and dangling tool calls are closed with unknown-outcome
/// toolResults, so `prompt`/`continue_run` always see a balanced transcript. Read-only
/// paths (listing, fork sources, `export`, `session-read`) never repair.
fn repaired(mut manager: SessionManager) -> Result<SessionManager, SessionError> {
    let report = manager.repair_interrupted_turn()?;
    if report.torn_tail || !report.closed_tool_calls.is_empty() {
        tracing::info!(
            session = manager.session_id(),
            closed_tool_calls = ?report.closed_tool_calls,
            torn_tail = report.torn_tail,
            "resume repair appended closers (D-REPAIR)"
        );
    }
    Ok(manager)
}

/// The model and thinking level a session starts with.
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

/// Holder-side takeover watcher (D-TAKEOVER): polls `<session file>.takeover` while the
/// lease identified by `epoch` is the current one; on a request the session is handed to
/// the requester through [`RuntimeCore::on_takeover`] (close here, reload there).
async fn lease_watcher(core: Weak<RuntimeCore>, epoch: u64, session_file: PathBuf) {
    loop {
        tokio::time::sleep(TAKEOVER_WATCH_POLL).await;
        let Some(core) = core.upgrade() else {
            return; // runtime gone
        };
        if core.lease_epoch.load(Ordering::SeqCst) != epoch {
            return; // lease replaced or released
        }
        if let Some(request) = read_takeover_request(&session_file) {
            core.on_takeover(epoch, &session_file, request.requester)
                .await;
            return; // teardown released this lease; the epoch moved on
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
) -> Result<(Current, Option<String>), SessionOpError> {
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
                .map_err(op_error)?;
        }
    } else {
        if let Some(model) = &model {
            manager
                .append_model_change(&model.provider, &model.id)
                .map_err(op_error)?;
        }
        manager
            .append_thinking_level_change(&thinking.to_string())
            .map_err(op_error)?;
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
        cwd: options.session_cwd(services).to_path_buf(),
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

/// Resolves the first session manager of `source` together with its lease: write opens of
/// existing files go through [`open_under_lease`] (acquire → open → repair), fresh sources
/// through [`fresh_under_lease`].
async fn source_under_lease(
    services: &Services,
    source: &SessionSource,
    root: &Path,
    cwd: &Path,
) -> Result<(SessionManager, Option<SessionLease>), SessionOpError> {
    Ok(match source {
        SessionSource::New => {
            fresh_under_lease(
                services,
                SessionManager::create(cwd, root, CreateOptions::default()).map_err(op_error)?,
            )
            .await?
        }
        SessionSource::NewWithId(id) => {
            fresh_under_lease(
                services,
                SessionManager::create(
                    cwd,
                    root,
                    CreateOptions {
                        id: Some(id.clone()),
                        parent_session: None,
                    },
                )
                .map_err(op_error)?,
            )
            .await?
        }
        SessionSource::Open(path) => open_under_lease(services, path, root, None).await?,
        SessionSource::ContinueRecent => {
            match SessionManager::find_most_recent(cwd, root).map_err(op_error)? {
                Some(path) => open_under_lease(services, &path, root, Some(cwd)).await?,
                None => {
                    fresh_under_lease(
                        services,
                        SessionManager::create(cwd, root, CreateOptions::default())
                            .map_err(op_error)?,
                    )
                    .await?
                }
            }
        }
        SessionSource::Resume(id) => {
            match SessionManager::find_by_id(cwd, id, root).map_err(op_error)? {
                Some(path) => open_under_lease(services, &path, root, None).await?,
                None => {
                    return Err(SessionOpError::Failed(format!(
                        "No session found matching '{id}'"
                    )));
                }
            }
        }
        SessionSource::Fork(path) => {
            fresh_under_lease(
                services,
                SessionManager::fork_from(path, cwd, root, CreateOptions::default())
                    .map_err(op_error)?,
            )
            .await?
        }
        SessionSource::InMemory => (
            SessionManager::in_memory(cwd, CreateOptions::default(), Vec::new()),
            None,
        ),
    })
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
        Self::open_session(services, options)
            .await
            .map_err(session_error)
    }

    /// Creates the runtime and its first session keeping [`SessionOpError`] variants
    /// intact (E-LAZY, core spec §2.2): the hub maps `SessionLocked` (D-TAKEOVER deadline)
    /// to its `locked` state instead of a generic failure text.
    ///
    /// # Errors
    ///
    /// Session file and lease failures, with variants preserved.
    pub async fn create_for_hub(
        services: Arc<Services>,
        options: SessionOptions,
    ) -> Result<Self, SessionOpError> {
        Self::open_session(services, options).await
    }

    /// The body of both constructors ([`Self::create`], [`Self::create_for_hub`]).
    async fn open_session(
        services: Arc<Services>,
        options: SessionOptions,
    ) -> Result<Self, SessionOpError> {
        let root = services.sessions_root(options.session_dir.as_deref());
        let (manager, lease) = source_under_lease(
            &services,
            &options.source,
            &root,
            options.session_cwd(&services),
        )
        .await?;
        let hub = EventHub::new();
        let (current, fallback) =
            install(&services, &options, &hub, None, manager, "startup", None).await?;
        let lease_file = lease
            .as_ref()
            .map(|lease| lease.session_file().to_path_buf());
        let core = Arc::new_cyclic(|me: &Weak<RuntimeCore>| RuntimeCore {
            services: Arc::clone(&services),
            hub: hub.clone(),
            current: Mutex::new(current),
            options,
            model_fallback_message: std::sync::OnceLock::new(),
            shutdown: tokio::sync::watch::channel(false).0,
            lease: Mutex::new(lease),
            lease_epoch: AtomicU64::new(0),
            takeover_handler: Mutex::new(None),
            me: me.clone(),
        });
        if let Some(file) = lease_file {
            let epoch = core.lease_epoch.fetch_add(1, Ordering::SeqCst) + 1;
            tokio::spawn(lease_watcher(core.me.clone(), epoch, file));
        }
        if let Some(fallback) = fallback {
            let _ = core.model_fallback_message.set(fallback);
        }
        core.session().link_runtime(core.me.clone());
        let runtime = Self { core };
        if let Some(name) = runtime.core.options.name.clone() {
            runtime.session().set_session_name(Some(name)).await?;
        }
        Ok(runtime)
    }

    /// Message shown when the model falls back.
    #[must_use]
    pub fn model_fallback_message(&self) -> Option<&str> {
        self.core.model_fallback_message.get().map(String::as_str)
    }

    /// The services.
    #[must_use]
    pub fn services(&self) -> &Arc<Services> {
        &self.core.services
    }

    /// Extension settings for frontends (settings hub, modes §4 `_oino/get_settings_schema`,
    /// `get_extension_settings`, `set_extension_setting`): schema, layered values with masked
    /// secrets, scoped writes and change notifications.
    #[must_use]
    pub fn extension_settings(&self) -> &Arc<crate::extensions::settings::ExtensionSettings> {
        &self.core.services.extension_settings
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

    /// Becomes `true` when an extension called `ctx.shutdown()`;
    /// frontends exit once the session is idle.
    #[must_use]
    pub fn shutdown_requested(&self) -> tokio::sync::watch::Receiver<bool> {
        self.core.shutdown.subscribe()
    }

    /// Starts a new session.
    ///
    /// # Errors
    ///
    /// Session file failures.
    pub async fn new_session(&self, parent: Option<PathBuf>) -> Result<Replaced, SessionOpError> {
        self.core.new_session(parent).await
    }

    /// Switches to another session.
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

    /// Forks the session.
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

    /// Copies the file into the sessions dir and switches to it.
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

    /// Installs the embedding's takeover behaviour override (D-TAKEOVER `on_takeover`);
    /// `None` restores the default (dispose the session, keep the runtime usable with a
    /// fresh one, publish [`OinoEvent::SessionTakenOver`], ask frontends to exit).
    pub fn set_takeover_handler(&self, handler: Option<TakeoverHandler>) {
        *lock(&self.core.takeover_handler) = handler;
    }

    /// Shuts the current session down (`session_shutdown{reason:"quit"}`).
    pub async fn dispose(&self) {
        self.core.teardown("quit", None).await;
        self.core.hub.close();
    }

    /// Idle eviction (E-LAZY, core spec §2.2): disposes the session so it returns to a
    /// record — `session_shutdown{reason:"idle"}` reaches the extensions (guests drop
    /// their `SessionMap` entry), the lease is released, and
    /// [`OinoEvent::SessionEvicted`] is published before the client streams end. Unlike
    /// [`AgentSessionRuntime::dispose`], no fresh session replaces the current one: the
    /// hub re-opens the file on demand.
    pub async fn evict(&self) {
        let session = self.core.session();
        let session_id = session.session_id();
        tracing::info!(session_id = %session_id, "idle eviction (E-LAZY): disposing");
        session
            .hub()
            .publish(OinoEvent::SessionEvicted { session_id });
        self.core.teardown("idle", None).await;
        self.core.hub.close();
    }

    /// Hub variant of the D-TAKEOVER holder behaviour: another runner asked this process
    /// to give the session up. Disposes the session (releasing the lease) without the
    /// default handler's fresh replacement session, publishes
    /// [`OinoEvent::SessionTakenOver`] on the session stream and asks frontends to return
    /// to their session list (`shutdown` watch).
    pub async fn evict_taken_over(&self, by: LeaseInfo) {
        let session = self.core.session();
        tracing::info!(
            session_id = %session.session_id(),
            by = %by.describe(),
            "session taken over (D-TAKEOVER, hub variant)"
        );
        session.hub().publish(OinoEvent::SessionTakenOver { by });
        self.core.teardown("takeover", None).await;
        self.core.hub.close();
        self.core.shutdown.send_replace(true);
    }
}

impl RuntimeCore {
    pub(crate) fn session(&self) -> AgentSession {
        lock(&self.current).session.clone()
    }

    /// `ctx.shutdown()`.
    pub(crate) fn request_shutdown(&self) {
        if !self.shutdown.send_replace(true) {
            self.session().hub().publish(OinoEvent::ShutdownRequested);
        }
    }

    /// D-TAKEOVER holder side: this session was taken over by another runner. Aborts the
    /// run, emits `session_shutdown{reason:"takeover"}`, disposes the session (releasing
    /// the lease), publishes [`OinoEvent::SessionTakenOver`], keeps the runtime usable with
    /// a fresh session and asks frontends to return to their session list.
    ///
    /// `epoch` + `session_file` guard against a stale watcher (one tick behind a lease
    /// swap) taking over the session that replaced its own.
    pub(crate) async fn on_takeover(
        self: &Arc<Self>,
        epoch: u64,
        session_file: &Path,
        by: LeaseInfo,
    ) {
        {
            let held = lock(&self.lease);
            let still_current = self.lease_epoch.load(Ordering::SeqCst) == epoch
                && held
                    .as_ref()
                    .is_some_and(|lease| lease.session_file() == session_file);
            if !still_current {
                return; // stale watcher: this lease is no longer ours
            }
        }
        let handler = lock(&self.takeover_handler).clone();
        if let Some(handler) = handler {
            handler(by).await;
            return;
        }
        tracing::info!(session_id = %self.session().session_id(), by = %by.describe(), "session taken over (D-TAKEOVER)");
        self.teardown("takeover", None).await;
        self.hub.publish(OinoEvent::SessionTakenOver { by });
        // "Next session": keep the runtime coherent — the taken-over file is gone from here.
        let (persisted, root) = self.persisted();
        let manager = if persisted {
            SessionManager::create(
                self.options.session_cwd(&self.services),
                &root,
                CreateOptions::default(),
            )
        } else {
            Ok(SessionManager::in_memory(
                self.options.session_cwd(&self.services),
                CreateOptions::default(),
                Vec::new(),
            ))
        };
        if let Ok(manager) = manager {
            match bind_lease(&self.services, &manager).await {
                Ok(lease) => {
                    if self.swap(manager, lease, "new", None).await.is_ok() {
                        self.replaced(ReplaceReason::New);
                    }
                }
                Err(error) => {
                    tracing::warn!(%error, "cannot bind a fresh session after takeover");
                }
            }
        }
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
        self.release_lease();
    }

    /// Stores the lease of the (about-to-be) current session (D-LOCK): bumps the epoch,
    /// retiring any previous watcher, and starts this lease's takeover watcher.
    fn set_lease(&self, lease: Option<SessionLease>) {
        let epoch = self.lease_epoch.fetch_add(1, Ordering::SeqCst) + 1;
        let file = lease
            .as_ref()
            .map(|lease| lease.session_file().to_path_buf());
        if let Some(file) = file {
            tokio::spawn(lease_watcher(self.me.clone(), epoch, file));
        }
        *lock(&self.lease) = lease; // replaces (and releases) any leftover
    }

    /// Releases the current session's lease (kernel lock, metadata, registry).
    fn release_lease(&self) {
        self.lease_epoch.fetch_add(1, Ordering::SeqCst);
        drop(lock(&self.lease).take());
    }

    async fn swap(
        &self,
        manager: SessionManager,
        lease: Option<SessionLease>,
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
        .await?;
        next.session
            .inherit_frontend_state(&old_session(&self.current));
        if reason == "fork" {
            next.session
                .inherit_checkpoints(&old_session(&self.current));
        }
        let old = std::mem::replace(&mut *lock(&self.current), next);
        old.forward.abort();
        self.set_lease(lease);
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

    /// Starts a new session.
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
        let (manager, lease) = if persisted {
            fresh_under_lease(
                &self.services,
                SessionManager::create(self.options.session_cwd(&self.services), &root, options)
                    .map_err(op_error)?,
            )
            .await?
        } else {
            (
                SessionManager::in_memory(
                    self.options.session_cwd(&self.services),
                    options,
                    Vec::new(),
                ),
                None,
            )
        };
        let target = manager.session_file().map(Path::to_path_buf);
        self.teardown("new", target.as_deref()).await;
        self.swap(manager, lease, "new", previous).await?;
        self.replaced(ReplaceReason::New);
        Ok(Replaced::default())
    }

    /// Switches to another session.
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
        let (manager, lease) =
            open_under_lease(&self.services, &path, &root, cwd_override.as_deref()).await?;
        let target = manager.session_file().map(Path::to_path_buf);
        self.teardown("resume", target.as_deref()).await;
        self.swap(manager, lease, "resume", previous).await?;
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
        let (manager, lease) = fresh_under_lease(&self.services, manager).await?;
        let target = manager.session_file().map(Path::to_path_buf);
        self.teardown("fork", target.as_deref()).await;
        self.swap(manager, lease, "fork", previous).await?;
        self.replaced(reason);
        Ok(Replaced {
            cancelled: false,
            selected_text,
        })
    }

    /// Copies the file into the sessions dir and switches to it.
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
        let dir =
            oino_session::manager::cwd_session_dir(&root, self.options.session_cwd(&self.services));
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
