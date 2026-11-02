//! The host's session hub (spec-host §4.2, WP-H3): the owner of every live session.
//!
//! The hub is the host's layer over the wave-1 lazy core hub
//! ([`oino_client::hub::SessionHub`], E-LAZY): a session is a record on disk until a message
//! arrives, waking acquires the single-writer lease, repairs a torn tail and builds the
//! runtime from the shared [`oino_core::Services`]; idle sessions are evicted back to
//! records (core's `extensionSettings.hub.*`). On top of that this layer adds what belongs
//! to the host process:
//!
//! - **The host's operations** — `create_session` (a new runtime, "session_new"), `open`
//!   (takeover + repair, disposing a crashed session first), `fork` (before an entry, or at
//!   the leaf), `archive` (dispose, then move the file to `<agent dir>/trash/`, §14.2) and
//!   `close` (dispose, keep the file).
//! - **Takeover** (§8, D-TAKEOVER is core's) — every open goes through core's lease with
//!   takeover, so a session held by the TUI, a `oino -p` job or another host is closed there
//!   and reloaded here; the runner it came from is carried on the [`HubSession`] for the
//!   stream's `{kind:"taken_over", from}` notice (§8.2). Symmetrically, when another runner
//!   takes one of this host's sessions, the run supervisor or the stream tap marks the card
//!   [`HubState::TakenOver`] (§8.4); opening it again takes it back.
//! - **Cards and project grouping** ([`super::card`]) — the Sessions list comes from core's
//!   card index (`list_all_cards`: bounded reads cached by path + size + mtime, on the
//!   blocking pool; big files refined in the background), merged with the host's live
//!   state; lookups by id go by file name, never a listing. Nothing here loads a runtime.
//! - **Per-session panic isolation** (§4.4) — a supervisor follows a session while it runs
//!   (it is only subscribed while the session is busy, so core's idle eviction keeps
//!   working); a `SessionCrashed` / panicking supervisor marks *that* session
//!   [`HubState::Crashed`], emits a `SessionChanged` card and leaves the other sessions
//!   running. Re-opening disposes the crashed runtime and repairs the file.
//! - **Hub events** (§7.4) — `SessionChanged` / `SessionRemoved` / `HostNotice` for
//!   `events_open` (H4) and the safe-mode banner.
//!
//! Host services (`cron`, `browser`, `kanban`, `host`) are registered separately, see
//! [`super::services`]; the hub itself never interprets an extension call.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use futures_util::FutureExt;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use oino_client::backend::{ClientError, SessionBackend};
use oino_client::hub::{
    HubLimits as CoreHubLimits, HubOptions as CoreHubOptions, HubSession as CoreHubSession,
    HubSessionState as CoreHubState, SessionHub as CoreHub,
};
use oino_client::protocol::{
    AgentCommand, OinoCommand, RpcCommand, RpcResponseData, RpcSessionState,
};
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ExtensionLoader, ResourceFlags};
use oino_models::ModelRegistry;
use oino_session::{EntryId, LeaseInfo, SessionManager};
use oino_settings::SettingsManager;

use super::card::{HubState, ProjectGroup, SessionCard};
use super::crash;
use super::meta::{FocusRegistry, MetaStore};
use unipi_proto::session::SessionMetaPatch;

/// Event queue of one [`SessionHub::subscribe`] receiver.
const EVENTS_CAPACITY: usize = 256;

/// How often the run supervisor checks whether its session is still busy.
const SUPERVISE_POLL: Duration = Duration::from_secs(1);

/// Consecutive idle polls after which the supervisor drops its subscription (the session is
/// then evictable again; a queued follow-up keeps it `is_streaming`).
const SUPERVISE_IDLE_POLLS: u32 = 3;

/// Host notices kept for clients that connect after startup (the safe-mode banner is emitted
/// before the app's `events_open` exists).
const NOTICE_HISTORY: usize = 16;

/// Minimum spacing between background project-service pre-warms (each is a no-op while
/// the services stay cached).
const WARM_INTERVAL: Duration = Duration::from_secs(60);

/// Failure of one hub operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HubError {
    /// A user-facing failure (archive I/O, an ambiguous id, …).
    #[error("{0}")]
    Failed(String),
    /// No session (or archived session) matches the target: the app forgets it rather
    /// than retrying (`E_NOT_FOUND`).
    #[error("{0}")]
    NotFound(String),
    /// The core hub rejected the call.
    #[error(transparent)]
    Client(#[from] ClientError),
}

impl HubError {
    fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
    }

    /// Core's "unknown target" text becomes [`HubError::NotFound`].
    fn from_find(error: ClientError) -> Self {
        match error {
            ClientError::Command(text) if text.starts_with("No session found matching") => {
                Self::NotFound(text)
            }
            other => Self::Client(other),
        }
    }
}

/// Test hook (debug builds only): `UNIPI_DEBUG_SESSIONS_LIST_DELAY_MS` makes every
/// session listing sleep that long **where the listing runs** (the blocking pool), so a
/// test can prove a slow listing never stalls `host_info` or other requests.
async fn debug_list_delay() {
    #[cfg(debug_assertions)]
    if let Some(ms) = std::env::var("UNIPI_DEBUG_SESSIONS_LIST_DELAY_MS")
        .ok()
        .and_then(|ms| ms.parse::<u64>().ok())
    {
        let _ = tokio::task::spawn_blocking(move || {
            std::thread::sleep(Duration::from_millis(ms));
        })
        .await;
    }
}

/// Severity of a [`HubEvent::HostNotice`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoticeLevel {
    /// Informational (updater prompt, "session moved").
    Info,
    /// Something the user should act on (safe-mode banner, crash notice).
    Warning,
}

/// What `events_open` streams (§7.4): the Sessions list and host notices.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum HubEvent {
    /// A card was created, renamed, changed state or became idle again.
    SessionChanged {
        /// The card.
        card: SessionCard,
    },
    /// A session left the list (closed and archived; the file moved to `trash/`).
    SessionRemoved {
        /// Session id.
        session_id: String,
    },
    /// A host-level notice (safe mode, updater, crash).
    HostNotice {
        /// Severity.
        level: NoticeLevel,
        /// Text to show.
        text: String,
    },
    /// The **host-scope** ui registry changed (ui-protocol §9.2): the first event after
    /// `events_open` is a snapshot, patches follow with `rev = previous + 1` (§7.4).
    Ui {
        /// Full registry snapshot (`UiSnapshot` JSON, forwarded verbatim).
        snapshot: Option<serde_json::Value>,
        /// Incremental update (`UiPatch` JSON, forwarded verbatim).
        patch: Option<serde_json::Value>,
    },
    /// A board project's files changed (app.md §3.14 `board_watch`, WP-A22): the Board
    /// re-reads the project.
    BoardChanged {
        /// Project slug.
        project: String,
    },
}

/// Inputs of [`SessionHub::create`] — the host's view of the core [`CoreHubOptions`].
pub struct HostHubOptions {
    /// Primary working directory (settings project file, default session cwd).
    pub cwd: PathBuf,
    /// Agent dir (`~/.oino`: sessions, settings, extensions, state).
    pub agent_dir: PathBuf,
    /// Bundled docs root; defaults to `<agent_dir>/docs`.
    pub docs_root: Option<PathBuf>,
    /// Sessions root override (the same chain as core: `--session-dir` > env > setting).
    pub session_dir: Option<PathBuf>,
    /// Run mode (lease holder label, extension contexts) — `desktop` for the host.
    pub mode: ExtensionMode,
    /// Dialog UI available (the app is attached; the host answers through it).
    pub has_ui: bool,
    /// Trust prompt override (`-a`/`-na`); `None` decides from the trust store.
    pub approve: Option<bool>,
    /// Resource flags (`--no-extensions`, safe mode, `--no-skills`, …).
    pub flags: ResourceFlags,
    /// Extension loader (the host's discovery: `~/.oino/extensions` + project dir).
    pub loader: Arc<dyn ExtensionLoader>,
    /// Model registry override (tests); default reads `<agent>/models.json` + `auth.json`.
    pub models: Option<ModelRegistry>,
    /// Tool selection applied to every woken session.
    pub tools: ToolSelection,
    /// `--exclude-tools` applied to every woken session.
    pub exclude_tools: Vec<String>,
    /// Overrides the hub's `maxServices` (descriptor budget; see [`crate::fd_budget`]).
    /// `None` keeps the `extensionSettings.hub.maxServices` setting (default 4).
    pub services_cap: Option<usize>,
    /// Host data dir (`~/.oino/host`): where `session-meta.json` lives (app spec
    /// §3.14). `None` falls back to `<agent_dir>/host` (hermetic tests).
    pub host_data_dir: Option<PathBuf>,
}

/// What [`SessionHub::move_to_workspace`] did.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct MoveOutcome {
    /// The moved chat's card (its `cwd` is the workspace now).
    pub card: SessionCard,
    /// Scratch files copied into the workspace.
    pub copied: usize,
    /// Scratch files left out because the workspace already had them.
    pub skipped: usize,
}

/// One live session as the host hands it to a frontend (H4's stream, the tests).
pub struct HubSession {
    backend: Arc<dyn SessionBackend>,
    hub: Weak<Inner>,
    card: SessionCard,
    /// The runner this open took the session over from (§8.2), if any: the stream emits it
    /// as the first event after the snapshot.
    taken_over: Option<String>,
}

impl std::fmt::Debug for HubSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HubSession")
            .field("session_id", &self.session_id())
            .field("card", &self.card)
            .finish_non_exhaustive()
    }
}

impl HubSession {
    fn new(
        hub: Weak<Inner>,
        backend: Arc<dyn SessionBackend>,
        card: SessionCard,
        taken_over: Option<String>,
    ) -> Self {
        Self {
            backend,
            hub,
            card,
            taken_over,
        }
    }

    /// Current session id (changes after a fork/switch: the runtime keeps running).
    #[must_use]
    pub fn session_id(&self) -> String {
        self.backend.session_id()
    }

    /// Who this open took the session over from (`tui`, `host:<name>`, `cli`), when another
    /// runner held it (§8.1–8.2). The stream sends it right after the snapshot.
    #[must_use]
    pub fn taken_over(&self) -> Option<&str> {
        self.taken_over.as_deref()
    }

    /// The card at open time (state included).
    #[must_use]
    pub fn card(&self) -> &SessionCard {
        &self.card
    }

    /// The backend any oino RPC command goes through (H4 forwards `SessionCmd` here).
    #[must_use]
    pub fn backend(&self) -> &Arc<dyn SessionBackend> {
        &self.backend
    }

    /// A fresh ordered event stream (one per subscriber; H4's per-client tap).
    pub fn events(&self) -> futures_util::stream::BoxStream<'static, ClientEvent> {
        self.backend.events()
    }

    /// Sends one command, supervising it when it starts agent work (§4.4): a crash
    /// during the run marks this session and leaves the others alone. Read-only
    /// commands — the snapshot's `get_state`/`get_entries`/`ui`/`topics`, `_oino/render`,
    /// an accordion-expand fetch — never start supervision, so they cannot flip the card
    /// to `Working` or mark an unfocused session `unread`.
    ///
    /// # Errors
    ///
    /// Any backend failure (the session was evicted, taken over or disposed).
    pub async fn send(&self, command: RpcCommand) -> Result<RpcResponseData, HubError> {
        self.dispatch(command, None).await
    }

    /// [`HubSession::send`] with the RPC request id (`SessionCmd{id}`): `bash` streams
    /// `bash_execution_update{id}` with it, exactly as a pi client would (§7.3).
    ///
    /// # Errors
    ///
    /// As [`HubSession::send`].
    pub async fn send_with_id(
        &self,
        request_id: Option<String>,
        command: RpcCommand,
    ) -> Result<RpcResponseData, HubError> {
        self.dispatch(command, request_id).await
    }

    /// One command through the backend, with supervision started exactly when the
    /// command starts agent work (or its reply says it did).
    async fn dispatch(
        &self,
        command: RpcCommand,
        request_id: Option<String>,
    ) -> Result<RpcResponseData, HubError> {
        let supervised = starts_run(&command);
        if supervised {
            if let Some(hub) = self.hub.upgrade() {
                hub.ensure_supervisor(Arc::clone(&self.backend));
            }
        }
        let reply = if let Some(id) = request_id {
            self.backend.send_with_id(Some(id), command).await
        } else {
            self.backend.send(command).await
        };
        // A command we did not recognise as run-starting may still have started one (a
        // queued steer, an extension dispatch): supervise from its reply on.
        if !supervised && reply_started_run(reply.as_ref()) {
            if let Some(hub) = self.hub.upgrade() {
                hub.ensure_supervisor(Arc::clone(&self.backend));
            }
        }
        reply.map_err(HubError::from)
    }
}

/// Commands that start agent work (a run, a `!` command, compaction) or queue into one
/// — the only ones that may switch a session's card to `Working` (§4.4). Everything
/// else (`get_state`, `get_entries`, `get_commands`, `get_available_models`, model and
/// thinking selectors, `ui`/`topics`/`_oino/render`/`_oino/get_shortcuts`, tree
/// navigation, renames, forks) is read-only as far as the card is concerned.
fn starts_run(command: &RpcCommand) -> bool {
    matches!(
        command,
        RpcCommand::Agent(
            AgentCommand::Prompt { .. }
                | AgentCommand::Steer { .. }
                | AgentCommand::FollowUp { .. }
                | AgentCommand::Bash { .. }
                | AgentCommand::Compact { .. }
        )
    )
}

/// Whether a command's reply says agent work started or was queued into a running
/// turn (a `prompt`'s `{disposition: "started" | "queued"}`), so a command outside
/// [`starts_run`] still gets supervision when it actually began work.
fn reply_started_run(reply: Result<&RpcResponseData, &ClientError>) -> bool {
    reply
        .ok()
        .and_then(|data| data.as_ref())
        .and_then(|value| value.get("disposition"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|disposition| disposition == "started" || disposition == "queued")
}

/// The host hub: the core lazy hub plus host state, cards and hub events.
pub struct SessionHub {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for SessionHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHub")
            .field("cwd", &self.inner.cwd)
            .field("agent_dir", &self.inner.agent_dir)
            .field("sessions_root", &self.inner.sessions_root)
            .finish_non_exhaustive()
    }
}

struct Inner {
    core: Arc<CoreHub>,
    cwd: PathBuf,
    agent_dir: PathBuf,
    sessions_root: PathBuf,
    events: broadcast::Sender<HubEvent>,
    /// Notices emitted before any client was listening (replayed by `events_open`).
    notices: Mutex<Vec<HubEvent>>,
    state: Mutex<State>,
    /// Per-user session flags (`session-meta.json`, app spec §3.14).
    meta: MetaStore,
    /// Which session each live `events_open` connection focuses (Z-86).
    focus: FocusRegistry,
    /// Which sessions the cron scheduler owns (app spec §3.14 "Cron source", WP-A23):
    /// fills the cards' `source: "cron"` from the cron runs store.
    cron: crate::cron_serve::CronSessions,
    /// When the last background project pre-warm ran (throttle).
    warmed_at: Mutex<Option<Instant>>,
    /// Per-provider credential validation (D-01/D-37): cached, lazily probed.
    readiness: crate::models_readiness::ReadinessCache,
    /// The host data dir (`~/.oino/host`; the fallback hermetic tests use) — where the
    /// blob store media references point.
    host_data: PathBuf,
    /// Where "No workspace" chats run: `<scratch root>/<session id>` (scratch.rs). All of
    /// them share the root's services set.
    scratch_root: PathBuf,
    started: SystemTime,
}

#[derive(Default)]
struct State {
    /// Host-side state overrides: `Working`, `Crashed{error}`, `TakenOver{from}`. Absent is
    /// idle (the record on disk is the truth).
    states: HashMap<String, HubState>,
    /// Sessions with a supervisor task.
    supervising: HashSet<String>,
    /// Sessions with a dialog waiting for the user (the card's `needs_you`).
    needs_you: HashSet<String>,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn epoch_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH)
        .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
}

impl SessionHub {
    /// Builds the host hub over a fresh core hub (one services cache per `(cwd, agent dir)`,
    /// one wasm engine for the process).
    ///
    /// # Errors
    ///
    /// The core hub's creation failures (none today; `Result` keeps room).
    pub fn create(options: HostHubOptions) -> Result<Arc<Self>, HubError> {
        let cwd = options.cwd.clone();
        let agent_dir = options.agent_dir.clone();
        let mut core_options = CoreHubOptions {
            cwd: options.cwd,
            agent_dir: options.agent_dir,
            docs_root: options.docs_root,
            session_dir: options.session_dir,
            mode: options.mode,
            has_ui: options.has_ui,
            approve: options.approve,
            flags: options.flags,
            loader: options.loader,
            models: options.models,
            tools: options.tools,
            exclude_tools: options.exclude_tools,
        };
        // The host writes session files (create/fork/archive) too: resolve the sessions root
        // once, with core's own chain, and pin it so both sides always agree.
        let settings = SettingsManager::create(&core_options.cwd, &core_options.agent_dir, false);
        let sessions_root = core_options.sessions_root(&settings);
        core_options.session_dir = Some(sessions_root.clone());
        let (events, _) = broadcast::channel(EVENTS_CAPACITY);
        // The host's fd budget caps the services LRU (fd_budget.rs) — a host-level
        // limit layered over user settings, not a user knob.
        let mut limits = CoreHubLimits::default();
        if let Some(cap) = options.services_cap {
            limits = limits.with_max_services(cap);
        }
        // "No workspace" chats run in per-chat scratch folders (never `$HOME`), all on the
        // root's one services set. Hermetic hubs (no host data dir: tests) keep them under
        // their own agent dir; a real host uses the OS data dir (scratch.rs).
        let scratch_root = if options.host_data_dir.is_some() {
            crate::scratch::default_scratch_root()
        } else {
            agent_dir.join("scratch")
        };
        if let Err(error) = std::fs::create_dir_all(&scratch_root) {
            tracing::warn!(root = %scratch_root.display(), "scratch root not created: {error}");
        }
        limits = limits.with_shared_services_root(scratch_root.clone());
        let core = CoreHub::create_with(core_options, limits)?;
        let host_data = options
            .host_data_dir
            .clone()
            .unwrap_or_else(|| agent_dir.join("host"));
        let meta = MetaStore::open(&host_data);
        let inner = Arc::new(Inner {
            core,
            cwd,
            agent_dir: agent_dir.clone(),
            sessions_root,
            events,
            notices: Mutex::new(Vec::new()),
            state: Mutex::new(State::default()),
            meta,
            focus: FocusRegistry::default(),
            cron: crate::cron_serve::CronSessions::new(&agent_dir),
            warmed_at: Mutex::new(None),
            readiness: crate::models_readiness::ReadinessCache::new(),
            host_data,
            scratch_root,
            started: SystemTime::now(),
        });
        Ok(Arc::new(Self { inner }))
    }

    /// The primary working directory of the host.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.inner.cwd
    }

    /// Where "No workspace" chats run: one `<root>/<session id>` folder per chat.
    #[must_use]
    pub fn scratch_root(&self) -> &Path {
        &self.inner.scratch_root
    }

    /// The agent dir (`~/.oino`).
    #[must_use]
    pub fn agent_dir(&self) -> &Path {
        &self.inner.agent_dir
    }

    /// Where archived session files go (`<agent dir>/trash/`).
    #[must_use]
    pub fn trash_dir(&self) -> PathBuf {
        self.inner.agent_dir.join("trash")
    }

    /// The host's blob store (`<host data dir>/blobs`, `crate::blob`): where a bounded
    /// snapshot's image `mediaRef`s point and the session stream's `media` command reads.
    #[must_use]
    pub fn blobs_dir(&self) -> PathBuf {
        self.inner.host_data.join("blobs")
    }

    /// How long this hub has been alive (the safe-mode success threshold reads it).
    #[must_use]
    pub fn uptime(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.inner.started)
            .unwrap_or_default()
    }

    /// App spec §3.14 (WP-A24): the services of the primary cwd — the same
    /// cached, idle-evicted set a session runs on, but with **no session
    /// runtime** — for the hub-level `_oino/*` subset (`host_cmd`).
    ///
    /// # Errors
    ///
    /// Services creation failures (core hub texts).
    pub async fn core_services(&self) -> Result<Arc<oino_core::services::Services>, HubError> {
        self.inner.core.services().await.map_err(HubError::Client)
    }

    /// Warms (or returns) the cached services of `cwd` with no session runtime — the
    /// background pre-warm of the recent projects, so a `session_open` there does not
    /// pay extension loading on the click path.
    ///
    /// # Errors
    ///
    /// Services creation failures (core hub texts).
    pub async fn core_services_for(
        &self,
        cwd: &Path,
    ) -> Result<Arc<oino_core::services::Services>, HubError> {
        self.inner
            .core
            .warm_services(cwd)
            .await
            .map_err(HubError::Client)
    }

    /// The hub event stream (§7.4). A new `events_open` stream first sends
    /// [`SessionHub::notices`], then this stream.
    #[must_use]
    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.inner.events.subscribe()
    }

    /// Host notices emitted so far (§7.4): the startup safe-mode banner would otherwise be
    /// lost for a client that connects after the host started. Oldest first, at most
    /// [`NOTICE_HISTORY`].
    #[must_use]
    pub fn notices(&self) -> Vec<HubEvent> {
        lock(&self.inner.notices).clone()
    }

    /// Emits a host notice (the safe-mode banner, an updater prompt, …).
    pub fn notice(&self, level: NoticeLevel, text: impl Into<String>) {
        let event = HubEvent::HostNotice {
            level,
            text: text.into(),
        };
        {
            let mut notices = lock(&self.inner.notices);
            notices.push(event.clone());
            let excess = notices.len().saturating_sub(NOTICE_HISTORY);
            if excess > 0 {
                notices.drain(..excess);
            }
        }
        self.inner.emit(event);
    }

    /// The lazy Sessions list (§4.2): headers only, merged with the host's live state.
    /// Also refreshes the background pre-warm of the most recent projects' services
    /// (throttled; see [`SessionHub::warm_recent_projects`]).
    ///
    /// # Errors
    ///
    /// Listing I/O failures.
    pub async fn list(&self) -> Result<Vec<SessionCard>, HubError> {
        debug_list_delay().await;
        let cards = self.inner.list().await?;
        self.warm_recent_projects(false);
        self.refine_cards();
        Ok(cards)
    }

    /// Background pass over the sessions the card index only sampled (big files: head +
    /// tail): reads them whole once, niced on the blocking pool, and re-announces every
    /// card whose name, folder or count changed (`SessionChanged`). One pass at a time;
    /// listings and opens never wait for it.
    fn refine_cards(&self) {
        let weak = Arc::downgrade(&self.inner);
        let root = self.inner.sessions_root.clone();
        tokio::spawn(async move {
            let changed = tokio::task::spawn_blocking(move || {
                let mut ids = Vec::new();
                oino_session::manager::refine_all_cards(&root, |info| ids.push(info.id));
                ids
            })
            .await
            .unwrap_or_default();
            let Some(inner) = weak.upgrade() else { return };
            if !changed.is_empty() {
                tracing::info!(count = changed.len(), "session cards refined");
            }
            for id in changed {
                inner.changed(&id).await;
            }
        });
    }

    /// Pre-warms the session hub's cached services for the most recent projects'
    /// cwds, in the background: opening a session there then answers with a live
    /// runtime rebuild only (`session_open` never waits for extension loading or
    /// provider model probes). At most one service set per distinct cwd is kept —
    /// bounded by the hub's `maxServices` — and passes run at most once per
    /// [`WARM_INTERVAL`] unless `force`.
    pub fn warm_recent_projects(&self, force: bool) {
        if !force && lock(&self.inner.warmed_at).is_some_and(|at| at.elapsed() < WARM_INTERVAL) {
            return;
        }
        *lock(&self.inner.warmed_at) = Some(Instant::now());
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            let Some(inner) = weak.upgrade() else { return };
            let Ok(records) = inner.core.list().await else {
                return;
            };
            let mut seen: HashSet<PathBuf> = HashSet::new();
            let mut cwds = 0;
            for record in records {
                if cwds >= inner.core.max_services() {
                    break;
                }
                let mut cwd = PathBuf::from(&record.info.cwd);
                // Every scratch chat runs on the scratch root's one services set.
                if crate::scratch::is_scratch(&inner.scratch_root, &cwd) {
                    cwd = inner.scratch_root.clone();
                }
                if record.info.cwd.is_empty()
                    || !cwd.is_absolute()
                    || !cwd.is_dir()
                    || !seen.insert(cwd.clone())
                {
                    continue;
                }
                cwds += 1;
                // A miss here is fine: the first open of that project builds its
                // services on the click path, off the provider network.
                let _ = inner.core.warm_services(&cwd).await;
            }
        });
    }

    /// Resolves a `session` target — exact id, unique prefix (≥ 4 chars) or session file
    /// path — to its card, without waking anything (§4.2, §7.2 `session_close` /
    /// `session_archive`).
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target, listing I/O failures.
    pub async fn resolve(&self, target: &str) -> Result<SessionCard, HubError> {
        let record = self.inner.resolve(target).await?;
        let state = lock(&self.inner.state);
        Ok(self.inner.card_from(&state, &record))
    }

    /// Whether the session's runtime is live in this hub (an open needs no wake).
    pub async fn is_live(&self, session_id: &str) -> bool {
        self.inner.core.is_live(session_id).await
    }

    /// The sessions root (session files live under it; the card and offset indexes beside it).
    #[must_use]
    pub fn sessions_root(&self) -> &Path {
        self.inner.core.sessions_root()
    }

    /// Emits the current card of one session again (a rename, or a state the protocol layer
    /// changed): `SessionChanged` keeps the Sessions list live (§7.4).
    pub async fn refresh(&self, session_id: &str) {
        self.inner.changed(session_id).await;
    }

    /// `session_meta_set` (app spec §3.14, WP-A4): pin / unread / title-lock flags,
    /// persisted in `session-meta.json`, carried on every card. Emits the updated
    /// `SessionChanged` and returns its card.
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target, listing I/O failures.
    pub async fn meta_set(
        &self,
        target: &str,
        patch: SessionMetaPatch,
    ) -> Result<SessionCard, HubError> {
        self.inner.meta_set(target, patch).await
    }

    /// A dialog arrived for (or left) `session_id`: the card's `needs_you`. A session
    /// that starts needing the user while no connection focuses it is marked `unread`
    /// (§3.14, Z-86). Called by the dialog wiring (U10/A9); idempotent.
    pub async fn set_needs_you(&self, session_id: &str, needs_you: bool) {
        self.inner.set_needs_you(session_id, needs_you).await;
    }

    /// Registers a new `events_open` connection for focus reporting (§3.14); its
    /// `Control::Focus` frames update the entry, [`SessionHub::release_focus`] ends it.
    #[must_use]
    pub fn focus_connection(&self) -> u64 {
        self.inner.focus.acquire()
    }

    /// What `token`'s connection focuses now (`None` = nothing). Focusing a session
    /// clears its `unread` (the user is looking at it, Z-82 "cleared on open").
    pub async fn set_focus(&self, token: u64, session: Option<&str>) {
        self.inner.set_focus(token, session).await;
    }

    /// The connection behind `token` ended: its focus stops counting (Z-86).
    pub async fn release_focus(&self, token: u64) {
        self.inner.release_focus(token).await;
    }

    /// Emits a host-scope ui registry event on `events_open` (ui-protocol §9.2: the host
    /// forwards the registry's snapshot and patches verbatim).
    pub fn emit_ui(&self, snapshot: Option<serde_json::Value>, patch: Option<serde_json::Value>) {
        self.inner.emit(HubEvent::Ui { snapshot, patch });
    }

    /// The hub's per-provider credential validation cache (D-01/D-37).
    #[must_use]
    pub fn readiness(&self) -> &crate::models_readiness::ReadinessCache {
        &self.inner.readiness
    }

    /// The model a new session on this hub will start on (D-01): the configured default
    /// when its provider is available, else the first validated fallback. `session_new`
    /// starts the runtime on it and `host_cmd › default_model` reports it.
    ///
    /// # Errors
    ///
    /// Services creation failures (core hub texts).
    pub async fn default_model(
        &self,
    ) -> Result<crate::models_readiness::DefaultModelChoice, HubError> {
        let services = self.core_services().await?;
        let settings = services
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .snapshot();
        Ok(crate::models_readiness::choose_default_model(
            &services.models,
            &settings,
            &self.inner.readiness,
            false,
        )
        .await)
    }

    /// Emits a board project change on `events_open` (app.md §3.14 `board_watch`, WP-A22):
    /// the CLI/agent wrote the project's files; the Board re-reads it.
    pub fn emit_board_changed(&self, project: &str) {
        self.inner.emit(HubEvent::BoardChanged {
            project: project.to_owned(),
        });
    }

    /// The same list grouped by project cwd (the Sessions list's project sections).
    ///
    /// # Errors
    ///
    /// As [`SessionHub::list`].
    pub async fn projects(&self) -> Result<Vec<ProjectGroup>, HubError> {
        self.inner.projects().await
    }

    /// One card by id, or `None` when no such record exists (a new session that has not
    /// written its file yet has no record).
    ///
    /// # Errors
    ///
    /// Listing I/O failures.
    pub async fn card(&self, session_id: &str) -> Result<Option<SessionCard>, HubError> {
        self.inner.card(session_id).await
    }

    /// §8.4: another runner took one of this host's sessions over — record it on the card
    /// and emit the new `SessionChanged` (once: the run supervisor and the stream tap can
    /// both see the core event). The runtime is already gone (core's holder side disposed
    /// it); opening the record again takes it back.
    pub async fn mark_taken_over(&self, session_id: &str, from: &str) {
        self.inner.mark_taken_over(session_id, from).await;
    }

    /// `session_new{cwd, name?}` (§4.2): a new runtime over a fresh session file (core
    /// writes the header when the first message arrives).
    ///
    /// # Errors
    ///
    /// Services creation or session-open failures.
    pub async fn create_session(
        &self,
        cwd: Option<&Path>,
        name: Option<&str>,
    ) -> Result<HubSession, HubError> {
        self.inner.create_session(cwd, name, None).await
    }

    /// `session_new{name?}` without a `cwd` — a "No workspace" chat: a new session whose
    /// cwd is its own private scratch folder `<scratch root>/<session id>`, created here.
    /// Never the user's home (the agent could read and write all of it).
    ///
    /// # Errors
    ///
    /// The folder cannot be created; services creation or session-open failures.
    pub async fn create_scratch_session(&self, name: Option<&str>) -> Result<HubSession, HubError> {
        self.inner.create_scratch_session(name).await
    }

    /// "Move to a workspace…": a scratch chat moves to `cwd` from its next turn on — one
    /// way only ([`crate::scratch::check_move`]). Append-only: the core appends one
    /// `oino.cwd_change` reminder; the header cwd, the system prompt and earlier entries
    /// stay byte-identical (prompt cache). The session file stays where it is (under the
    /// scratch cwd's sessions dir — listing scans every dir, and the reopen finds the move
    /// in the file). With `copy_files`, the scratch folder's files are copied into `cwd`
    /// first, never overwriting. Emits the moved card (the Sessions list regroups it).
    ///
    /// # Errors
    ///
    /// Unknown session, the one-way rule, a busy agent, copy failures.
    pub async fn move_to_workspace(
        &self,
        target: &str,
        cwd: &Path,
        copy_files: bool,
    ) -> Result<MoveOutcome, HubError> {
        self.inner.move_to_workspace(target, cwd, copy_files).await
    }

    /// `session_open{session}`: wake (or reuse) the session, taking the lease over from any
    /// other runner (core D-TAKEOVER). A crashed session is disposed first, so the resume
    /// layer repairs the torn tail (§4.2, §4.4).
    ///
    /// # Errors
    ///
    /// Unknown/ambiguous target, services or session failures (a holder that refuses to
    /// yield keeps core's `SessionLocked` text).
    pub async fn open(&self, target: &str) -> Result<HubSession, HubError> {
        self.inner.open(target).await
    }

    /// `session_fork{session, entry_id?, name?}`: fork before `entry_id` (pi semantics), or
    /// at the leaf when no entry is given; the stream keeps the fork, the original stays.
    ///
    /// # Errors
    ///
    /// As [`SessionHub::open`], plus the fork command's own failures.
    pub async fn fork(
        &self,
        target: &str,
        entry_id: Option<&str>,
        name: Option<&str>,
    ) -> Result<HubSession, HubError> {
        self.inner.fork(target, entry_id, name).await
    }

    /// `session_archive{session}`: dispose the runtime and move the file to
    /// `<agent dir>/trash/` (restorable from there, §14.2).
    ///
    /// # Errors
    ///
    /// Unknown target, dispose failures, trash I/O.
    pub async fn archive(&self, target: &str) -> Result<SessionCard, HubError> {
        self.inner.archive(target).await
    }

    /// App spec §3.14 (`sessions_list{archived:true}`, WP-A5): the archived sessions — the
    /// session files in `<agent dir>/trash/` — newest activity first, headers only.
    ///
    /// # Errors
    ///
    /// Trash directory I/O.
    pub async fn archived(&self) -> Result<Vec<SessionCard>, HubError> {
        debug_list_delay().await;
        let cards = self.inner.archived().await?;
        // The trash's big files are refined too (no events: the list reloads on open).
        let trash = self.inner.agent_dir.join("trash");
        tokio::task::spawn_blocking(move || {
            oino_session::manager::refine_dir_cards(&trash, |_| {});
        });
        Ok(cards)
    }

    /// App spec §3.14 (`session_restore`, WP-A5, Z-13): move an archived session file back
    /// into its project's folder under the sessions root and return its card.
    ///
    /// # Errors
    ///
    /// Unknown/ambiguous archived target, trash or sessions I/O.
    pub async fn restore(&self, target: &str) -> Result<SessionCard, HubError> {
        self.inner.restore(target).await
    }

    /// App spec §3.14 (`session_delete`, WP-A5, Z-13/Z-20): delete one archived session
    /// file. Live sessions are archived first (the app does that), so a target outside the
    /// trash is refused rather than silently skipped.
    ///
    /// # Errors
    ///
    /// Unknown/ambiguous archived target, trash I/O.
    pub async fn delete_archived(&self, target: &str) -> Result<(), HubError> {
        self.inner.delete_archived(target).await
    }

    /// App spec §3.14 (`trash_empty`, WP-A5, Z-13): delete every archived session file;
    /// answers how many went.
    ///
    /// # Errors
    ///
    /// Reading the trash directory.
    pub async fn empty_trash(&self) -> Result<usize, HubError> {
        self.inner.empty_trash().await
    }

    /// `session_close{session}`: dispose the runtime when idle and no other subscriber; the
    /// session file stays a record.
    ///
    /// # Errors
    ///
    /// Never today (closing a record is a no-op); `Result` keeps room.
    pub async fn close(&self, session_id: &str) -> Result<(), HubError> {
        self.inner.close(session_id).await
    }
}

impl Inner {
    fn emit(&self, event: HubEvent) {
        // No receivers yet (no `events_open` client) is fine: hub events are a live channel.
        let _ = self.events.send(event);
    }

    async fn list(&self) -> Result<Vec<SessionCard>, HubError> {
        let records = self.core.list().await?;
        let state = lock(&self.state);
        Ok(records
            .into_iter()
            .map(|record| self.card_from(&state, &record))
            .collect())
    }

    async fn projects(&self) -> Result<Vec<ProjectGroup>, HubError> {
        Ok(ProjectGroup::group(self.list().await?))
    }

    /// One card by exact id (file names + card index — no listing).
    async fn card(&self, session_id: &str) -> Result<Option<SessionCard>, HubError> {
        match self.core.find(session_id).await {
            Ok(record) if record.info.id == session_id => {
                Ok(Some(self.card_from(&lock(&self.state), &record)))
            }
            Ok(_) => Ok(None),
            Err(error) => match HubError::from_find(error) {
                HubError::NotFound(_) => Ok(None),
                other => Err(other),
            },
        }
    }

    /// The card of one record, with the host's live state merged in.
    fn card_from(&self, state: &State, record: &CoreHubSession) -> SessionCard {
        let info = &record.info;
        let cron = self.cron.owner_of(&info.id, &info.path);
        self.meta.onto(SessionCard {
            session_id: info.id.clone(),
            file: info.path.clone(),
            name: info.name.clone(),
            // A chat that moved to a workspace (`oino.cwd_change`) is listed there.
            cwd: info.moved_cwd.clone().unwrap_or_else(|| info.cwd.clone()),
            updated_at: epoch_ms(info.modified),
            message_count: info.message_count,
            state: Self::state_of(state, &info.id, record.state),
            // Extension dialogs (needs-you) ride H4's `UiRequest` stream; the card's field is
            // filled from there.
            needs_you: state.needs_you.contains(&info.id),
            model: None,
            pinned: false,
            unread: false,
            title_locked: false,
            // App spec §3.14 (WP-A23, Z-112): a run-owned session says who runs it.
            source: cron.as_ref().map(|_| "cron".to_owned()),
            cron_job_id: cron.as_ref().map(|c| c.job_id.clone()),
            cron_job_name: cron.and_then(|c| c.job_name),
            changes: None,
            last_activity: epoch_ms(info.modified),
        })
    }

    /// The card state: the host's override when it has one, else the record's live-ness.
    fn state_of(state: &State, id: &str, core: CoreHubState) -> HubState {
        if let Some(host) = state.states.get(id) {
            match host {
                HubState::Crashed { .. } | HubState::TakenOver { .. } => return host.clone(),
                HubState::Working if core == CoreHubState::Live => return HubState::Working,
                _ => {}
            }
        }
        match core {
            // Core contained a crash (X-PANIC) and the runtime is still listed: the panic
            // text is unknown here (a live supervisor records it when it saw the event).
            CoreHubState::Crashed => HubState::Crashed {
                error: HubState::UNKNOWN_CRASH.to_owned(),
            },
            // A record, an idle live session, or one locked by another runner that refused:
            // all of them are "idle on disk" from the list's point of view (`open` reports
            // the precise `SessionLocked` text).
            _ => HubState::Idle,
        }
    }

    /// The card for `backend` (§4.2): the record when it exists (a new session has no file
    /// until its first message), else a card built from the runtime state.
    async fn card_for(&self, backend: &Arc<dyn SessionBackend>, cwd: &Path) -> SessionCard {
        let id = backend.session_id();
        if let Ok(record) = self.core.find(&id).await {
            if record.info.id == id {
                return self.card_from(&lock(&self.state), &record);
            }
        }
        let now = epoch_ms(SystemTime::now());
        let mut card = SessionCard {
            session_id: id,
            file: PathBuf::new(),
            name: None,
            cwd: cwd.display().to_string(),
            updated_at: now,
            message_count: 0,
            state: HubState::Idle,
            needs_you: false,
            model: None,
            pinned: false,
            unread: false,
            title_locked: false,
            source: None,
            cron_job_id: None,
            cron_job_name: None,
            changes: None,
            last_activity: now,
        };
        if let Ok(Some(data)) = backend
            .send(RpcCommand::Agent(AgentCommand::GetState))
            .await
        {
            if let Ok(session) = serde_json::from_value::<RpcSessionState>(data) {
                card.session_id = session.session_id;
                card.file = session.session_file.map(PathBuf::from).unwrap_or_default();
                card.name = session.session_name;
                card.message_count = session.message_count;
                card.model = session.model.map(|model| model.id);
                // The live runtime knows its cwd even before its file exists (E-03).
                if let Some(cwd) = session.cwd {
                    card.cwd = cwd;
                }
            }
        }
        card.state = Self::state_of(&lock(&self.state), &card.session_id, CoreHubState::Live);
        self.meta.onto(card)
    }

    /// Emits the current card of `session_id` (state/name changed).
    async fn changed(&self, session_id: &str) {
        if let Ok(Some(card)) = self.card(session_id).await {
            self.emit(HubEvent::SessionChanged { card });
        }
    }

    /// `session_meta_set` (§3.14): patch, persist, tell the clients.
    async fn meta_set(
        &self,
        target: &str,
        patch: SessionMetaPatch,
    ) -> Result<SessionCard, HubError> {
        let id = self.resolve(target).await?.info.id;
        self.meta.patch(&id, patch);
        self.card(&id)
            .await?
            .inspect(|card| self.emit(HubEvent::SessionChanged { card: card.clone() }))
            .ok_or_else(|| HubError::failed(format!("No session found matching '{target}'")))
    }

    /// A dialog arrived/left: `needs_you` on the card; a session that starts needing the
    /// user unfocused is `unread` (Z-86).
    async fn set_needs_you(&self, session_id: &str, needs_you: bool) {
        let became = {
            let mut state = lock(&self.state);
            if needs_you {
                state.needs_you.insert(session_id.to_owned())
            } else {
                state.needs_you.remove(session_id)
            }
        };
        if !became {
            return;
        }
        if needs_you {
            self.mark_unread_unfocused(session_id);
        }
        self.changed(session_id).await;
    }

    /// `Control::Focus` on an `events_open` connection (§3.14): the connection's focus
    /// changes; a newly focused session drops its `unread` dot (Z-82 "cleared on open").
    async fn set_focus(&self, token: u64, session: Option<&str>) {
        self.focus.set(token, session);
        if let Some(session) = session.filter(|s| self.meta.flags(s).unread) {
            self.meta.patch(
                session,
                SessionMetaPatch {
                    unread: Some(false),
                    ..SessionMetaPatch::default()
                },
            );
            self.changed(session).await;
        }
    }

    /// The `events_open` connection behind `token` ended (its focus stops counting).
    async fn release_focus(&self, token: u64) {
        self.focus.release(token);
    }

    /// Z-86: `unread = true` while no connection focuses `session_id`; the caller
    /// re-emits the card.
    fn mark_unread_unfocused(&self, session_id: &str) {
        if self.focus.is_focused(session_id) {
            return;
        }
        self.meta.patch(
            session_id,
            SessionMetaPatch {
                unread: Some(true),
                ..SessionMetaPatch::default()
            },
        );
    }

    /// `session_new{cwd, name?}` (§4.2): a new runtime over a fresh session file (core
    /// writes the header when the first message arrives). The runtime starts on
    /// [`SessionHub::default_model`]'s choice (D-01), so the draft chip, the snapshot and
    /// the first send all agree on one model that actually exists on this host.
    async fn create_session(
        self: &Arc<Self>,
        cwd: Option<&Path>,
        name: Option<&str>,
        id: Option<String>,
    ) -> Result<HubSession, HubError> {
        let cwd = cwd.map_or_else(|| self.cwd.clone(), Path::to_path_buf);
        let model = self.default_model_for(&cwd).await?;
        let backend = self
            .core
            .create_session_with_id(Some(&cwd), name, model, id)
            .await?;
        let card = self.card_for(&backend, &cwd).await;
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        Ok(HubSession::new(Arc::downgrade(self), backend, card, None))
    }

    /// [`SessionHub::create_scratch_session`]: the id is chosen first so the folder can be
    /// named after it before the runtime (and its header cwd) exists.
    async fn create_scratch_session(
        self: &Arc<Self>,
        name: Option<&str>,
    ) -> Result<HubSession, HubError> {
        let id = uuid::Uuid::now_v7().hyphenated().to_string();
        let dir = crate::scratch::scratch_dir(&self.scratch_root, &id);
        std::fs::create_dir_all(&dir).map_err(|error| {
            HubError::failed(format!(
                "Could not create the chat's scratch folder {}: {error}",
                dir.display()
            ))
        })?;
        tracing::info!(session_id = %id, cwd = %dir.display(), "scratch chat (No workspace)");
        self.create_session(Some(&dir), name, Some(id)).await
    }

    /// [`SessionHub::move_to_workspace`].
    async fn move_to_workspace(
        self: &Arc<Self>,
        target: &str,
        cwd: &Path,
        copy_files: bool,
    ) -> Result<MoveOutcome, HubError> {
        let opened = self.open(target).await?;
        let id = opened.session_id();
        let backend = Arc::clone(&opened.backend);
        // The live runtime is the truth for the current folder (a moved file-less chat has
        // no record yet).
        let current = match backend
            .send(RpcCommand::Agent(AgentCommand::GetState))
            .await
        {
            Ok(Some(data)) => serde_json::from_value::<RpcSessionState>(data)
                .ok()
                .and_then(|state| state.cwd)
                .unwrap_or_else(|| opened.card.cwd.clone()),
            _ => opened.card.cwd.clone(),
        };
        // Judged on canonical paths (a symlinked temp/data dir must not hide that the
        // target sits inside the scratch area).
        let canon = |path: &Path| dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        let current = canon(Path::new(&current));
        let target_dir = canon(cwd);
        crate::scratch::check_move(&canon(&self.scratch_root), &current, &target_dir)
            .map_err(HubError::failed)?;
        let copied = if copy_files {
            crate::scratch::copy_missing(&current, &target_dir).map_err(|error| {
                HubError::failed(format!(
                    "Copying the scratch files into {} failed: {error}",
                    target_dir.display()
                ))
            })?
        } else {
            crate::scratch::CopyReport::default()
        };
        backend
            .send(RpcCommand::Oino(OinoCommand::SetCwd {
                cwd: target_dir.display().to_string(),
            }))
            .await?;
        tracing::info!(
            session_id = %id,
            from = %current.display(),
            to = %target_dir.display(),
            copied = copied.copied,
            skipped = copied.skipped,
            "chat moved to a workspace"
        );
        let card = match self.card(&id).await? {
            Some(card) => card,
            None => self.card_for(&backend, &target_dir).await,
        };
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        Ok(MoveOutcome {
            card,
            copied: copied.copied,
            skipped: copied.skipped,
        })
    }

    /// The chooser behind `session_new`: the same services the runtime will run on, so the
    /// choice reflects that project's provider implementations; a chooser failure never
    /// blocks the session (the engine's own selection takes over).
    async fn default_model_for(&self, cwd: &Path) -> Result<Option<oino_ai::Model>, HubError> {
        let services = match self.core.warm_services(cwd).await {
            Ok(services) => services,
            Err(_) => self.core.services().await?,
        };
        let settings = services
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .snapshot();
        let choice = crate::models_readiness::choose_default_model(
            &services.models,
            &settings,
            &self.readiness,
            true,
        )
        .await;
        if let Some(reason) = &choice.reason {
            tracing::info!(
                source = choice.source,
                model = choice.model.as_ref().map(|model| model.id.as_str()).unwrap_or("none"),
                reason = %reason,
                "session_new model choice"
            );
        }
        Ok(choice.model)
    }

    /// `session_open{session}` — with takeover (§8): a session held by the TUI, a CLI run or
    /// another host is asked to close (core D-TAKEOVER), reloaded here, and the runner it
    /// came from is attached to the result for the stream's `taken_over` notice.
    async fn open(self: &Arc<Self>, target: &str) -> Result<HubSession, HubError> {
        // E-03: a session created in this hub (`session_new`) has no record until its
        // first conversation entry writes the file, so `resolve` fails while the live
        // runtime is still wakeable by exact id. Wake it instead of failing the open —
        // that first send then lands and the file is written. No takeover and no record
        // state applies: a file-less session has no lease and was never taken over.
        let record = match self.resolve(target).await {
            Ok(record) => Some(record),
            // The resolve error is only reported when the target is not live either.
            Err(_error) if self.core.is_live(target).await => None,
            Err(error) => return Err(error),
        };
        let id = record
            .as_ref()
            .map_or_else(|| target.to_owned(), |record| record.info.id.clone());
        let prior = Self::state_of(
            &lock(&self.state),
            &id,
            record.as_ref().map_or(CoreHubState::Live, |r| r.state),
        );
        // Read the previous holder before `core.open` overwrites the advisory metadata with
        // ours: a live holder means this open is a takeover (§8.1); a dead one is a stale
        // lock core reclaims silently (spec-core D-TAKEOVER step 1). A session live in this
        // hub is already ours — nothing to take over.
        let taken_over = match record.as_ref().map(|record| record.state) {
            Some(CoreHubState::Live) | None => None,
            Some(_) => {
                let path = &record.as_ref().expect("record state checked").info.path;
                lease_holder(path).map(|holder| takeover_from(&holder))
            }
        };
        if matches!(prior, HubState::Crashed { .. }) {
            // §4.2: "crashed sessions are disposed on next open" — the resume layer repairs
            // the torn tail and the fresh runtime continues from the record.
            self.core.close(&id).await?;
            lock(&self.state).states.remove(&id);
            // The crashed runtime's supervisor ends when its stream closes; wait (bounded)
            // so the fresh runtime gets its own supervisor on the next command.
            for _ in 0..40 {
                if !lock(&self.state).supervising.contains(&id) {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        let backend = self.core.open(&id).await?;
        // The runtime is ours now: a crash we repaired or a takeover we took back is
        // history (§4.2, §8.4). `Working` still describes a running session and stays.
        {
            let mut state = lock(&self.state);
            if matches!(
                state.states.get(&id),
                Some(HubState::Crashed { .. } | HubState::TakenOver { .. })
            ) {
                state.states.remove(&id);
            }
        }
        let cwd = match &record {
            Some(record) => record_cwd(record, &self.cwd),
            // No record: the live runtime's own cwd (its header is not written yet).
            None => self.cwd.clone(),
        };
        let card = self.card_for(&backend, &cwd).await;
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        Ok(HubSession::new(
            Arc::downgrade(self),
            backend,
            card,
            taken_over,
        ))
    }

    /// `session_fork{session, entry_id?, name?}` — a **file-level** fork (v2 QA D-09):
    /// the fork is written from the target's session file (branch before `entry_id`, or at
    /// the leaf) and woken as **its own runtime with its own id**; the original keeps its
    /// runtime, feed and stream untouched. Core's in-place `fork`/`clone` RPCs stay as they
    /// are for in-stream use, where the issuing stream *follows* the new session — a
    /// host-level fork must never rewire a session somebody else is looking at.
    async fn fork(
        self: &Arc<Self>,
        target: &str,
        entry_id: Option<&str>,
        name: Option<&str>,
    ) -> Result<HubSession, HubError> {
        let record = self.resolve(target).await?;
        let cwd = record_cwd(&record, &self.cwd);
        // 1. Write the fork's file from the target's file (no runtime touched: the target
        //    keeps its lease, its live state and its events).
        let path = fork_file(&record.info.path, &cwd, &self.sessions_root, entry_id)?;
        // 2. Wake the fork's fresh record as its own runtime (a new file: no other holder).
        let backend = self.core.open(&path.to_string_lossy()).await?;
        if let Some(name) = name {
            backend
                .send(RpcCommand::Agent(AgentCommand::SetSessionName {
                    name: name.to_owned(),
                }))
                .await?;
        }
        let card = self.card_for(&backend, &cwd).await;
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        tracing::info!(
            fork = %card.session_id,
            from = %record.info.id,
            path = %path.display(),
            "session forked (file-level)"
        );
        Ok(HubSession::new(Arc::downgrade(self), backend, card, None))
    }

    /// `session_archive{session}`: dispose, then move the file to `trash/` (§14.2).
    async fn archive(&self, target: &str) -> Result<SessionCard, HubError> {
        let record = self.resolve(target).await?;
        let id = record.info.id.clone();
        let card = self.card_from(&lock(&self.state), &record);
        self.core.close(&id).await?;
        lock(&self.state).states.remove(&id);
        let trash = self.agent_dir.join("trash");
        tokio::fs::create_dir_all(&trash)
            .await
            .map_err(|error| HubError::failed(format!("creating {}: {error}", trash.display())))?;
        let name = record.info.path.file_name().map_or_else(
            || format!("{id}.jsonl"),
            |name| name.to_string_lossy().into_owned(),
        );
        let dest = unique_destination(&trash, &name);
        tokio::fs::rename(&record.info.path, &dest)
            .await
            .map_err(|error| {
                HubError::failed(format!("archiving {}: {error}", record.info.path.display()))
            })?;
        self.emit(HubEvent::SessionRemoved {
            session_id: id.clone(),
        });
        tracing::info!(session_id = %id, dest = %dest.display(), "session archived");
        Ok(card)
    }

    /// `session_close{session}`: dispose, keep the file.
    async fn close(&self, session_id: &str) -> Result<(), HubError> {
        self.core.close(session_id).await?;
        lock(&self.state).states.remove(session_id);
        self.changed(session_id).await;
        Ok(())
    }

    /// `sessions_list{archived:true}` (§3.14, WP-A5): the trashed session files, newest
    /// activity first, as idle cards. Each file is read header-only
    /// ([`oino_session::manager::session_info`]); nothing wakes a runtime.
    async fn archived(&self) -> Result<Vec<SessionCard>, HubError> {
        let mut cards: Vec<SessionCard> = self
            .archived_sessions()
            .await?
            .iter()
            .map(|info| {
                let updated_at = info
                    .modified
                    .duration_since(UNIX_EPOCH)
                    .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
                let card = SessionCard {
                    session_id: info.id.clone(),
                    file: info.path.clone(),
                    name: info.name.clone(),
                    cwd: info.cwd.clone(),
                    updated_at,
                    message_count: info.message_count,
                    state: HubState::Idle,
                    needs_you: false,
                    model: None,
                    pinned: false,
                    unread: false,
                    title_locked: false,
                    source: None,
                    cron_job_id: None,
                    cron_job_name: None,
                    changes: None,
                    last_activity: updated_at,
                };
                self.meta.onto(card)
            })
            .collect();
        cards.sort_by_key(|card| std::cmp::Reverse(card.updated_at));
        Ok(cards)
    }

    /// `session_restore{session}` (§3.14, WP-A5, Z-13): move the archived file back to
    /// `<sessions root>/<encoded cwd>/` (core's own layout) and re-list it. The restored
    /// card is announced on `events_open` like any other change.
    async fn restore(&self, target: &str) -> Result<SessionCard, HubError> {
        let entry = self.resolve_archived(target).await?;
        let cwd = if entry.cwd.is_empty() {
            self.cwd.clone()
        } else {
            PathBuf::from(&entry.cwd)
        };
        let dir = oino_session::manager::cwd_session_dir(&self.sessions_root, &cwd);
        tokio::fs::create_dir_all(&dir)
            .await
            .map_err(|error| HubError::failed(format!("creating {}: {error}", dir.display())))?;
        let name = entry.path.file_name().map_or_else(
            || format!("{}.jsonl", entry.id),
            |name| name.to_string_lossy().into_owned(),
        );
        let dest = unique_destination(&dir, &name);
        tokio::fs::rename(&entry.path, &dest)
            .await
            .map_err(|error| {
                HubError::failed(format!("restoring {}: {error}", entry.path.display()))
            })?;
        let card = match self.core.find(&dest.to_string_lossy()).await {
            Ok(record) => Some(self.card_from(&lock(&self.state), &record)),
            Err(error) => {
                tracing::warn!(%error, session_id = %entry.id, "reading a restored session");
                None
            }
        }
        .unwrap_or_else(|| SessionCard {
            session_id: entry.id.clone(),
            file: dest.clone(),
            name: entry.name.clone(),
            cwd: entry.cwd.clone(),
            updated_at: epoch_ms(entry.modified),
            message_count: entry.message_count,
            state: HubState::Idle,
            needs_you: false,
            model: None,
            pinned: false,
            unread: false,
            title_locked: false,
            source: None,
            cron_job_id: None,
            cron_job_name: None,
            changes: None,
            last_activity: epoch_ms(entry.modified),
        });
        let card = self.meta.onto(card);
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        tracing::info!(session_id = %entry.id, dest = %dest.display(), "session restored");
        Ok(card)
    }

    /// `session_delete{session}` (§3.14, WP-A5): unlink one archived session file. The
    /// op is trash-only: a live session has to be archived first (Z-13/Z-20).
    async fn delete_archived(&self, target: &str) -> Result<(), HubError> {
        let entry = self.resolve_archived(target).await?;
        tokio::fs::remove_file(&entry.path).await.map_err(|error| {
            HubError::failed(format!("deleting {}: {error}", entry.path.display()))
        })?;
        self.emit(HubEvent::SessionRemoved {
            session_id: entry.id.clone(),
        });
        tracing::info!(session_id = %entry.id, "archived session deleted");
        Ok(())
    }

    /// `trash_empty` (§3.14, WP-A5): delete every archived session file (ZCode's
    /// "Delete all archived").
    async fn empty_trash(&self) -> Result<usize, HubError> {
        let archived = self.archived_sessions().await?;
        let mut deleted = 0;
        for entry in archived {
            match tokio::fs::remove_file(&entry.path).await {
                Ok(()) => {
                    deleted += 1;
                    self.emit(HubEvent::SessionRemoved {
                        session_id: entry.id,
                    });
                }
                // Another runner already deleted it: the trash ends up empty either way.
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => {
                    tracing::warn!(%error, path = %entry.path.display(), "emptying the trash")
                }
            }
        }
        tracing::info!(deleted, "trash emptied");
        Ok(deleted)
    }

    /// Every session file in `trash/` that parses as a session, newest first — from the
    /// trash's card index (bounded reads), on the blocking pool.
    async fn archived_sessions(&self) -> Result<Vec<oino_session::manager::SessionInfo>, HubError> {
        let trash = self.agent_dir.join("trash");
        tokio::task::spawn_blocking(move || oino_session::manager::list_dir_cards(&trash))
            .await
            .map_err(|error| HubError::failed(format!("listing the trash: {error}")))
    }

    /// Resolves an archived `session` target — exact id, unique prefix (≥ 4 chars) or the
    /// archived file's path — against `trash/`, by file name (no listing).
    async fn resolve_archived(
        &self,
        target: &str,
    ) -> Result<oino_session::manager::SessionInfo, HubError> {
        let trash = self.agent_dir.join("trash");
        let owned = target.to_owned();
        let found = tokio::task::spawn_blocking(move || {
            oino_session::manager::find_card_in_dir(&trash, &owned)
        })
        .await
        .map_err(|error| HubError::failed(format!("reading the trash: {error}")))?;
        match found {
            Ok(Some(entry)) => Ok(entry),
            Ok(None) if Path::new(target).is_absolute() => Err(HubError::NotFound(format!(
                "no archived session at {target}"
            ))),
            Ok(None) => Err(HubError::NotFound(format!(
                "no archived session matches {target:?}"
            ))),
            Err(_) => Err(HubError::failed(format!(
                "{target:?} matches more than one archived session"
            ))),
        }
    }

    /// Resolves a `session` target (exact id, unique prefix ≥ 4 chars, or file path) against
    /// the records — core's own rules, without waking anything.
    async fn resolve(&self, target: &str) -> Result<CoreHubSession, HubError> {
        self.core.find(target).await.map_err(HubError::from_find)
    }
}

/// The `from` label of a takeover (§8.2: `tui`, `host:<name>` or `cli`) from the previous
/// runner's lease identity (D-LOCK advisory metadata). Core labels every lease `oino <mode>`;
/// `desktop` and `mobile` are only ever held by a host process, so those name the host.
#[must_use]
pub fn takeover_from(holder: &LeaseInfo) -> String {
    let client = holder.client.trim();
    let host = holder.host.trim();
    if client.eq_ignore_ascii_case("oino tui") || client.eq_ignore_ascii_case("tui") {
        return "tui".to_owned();
    }
    let from_host = client.eq_ignore_ascii_case("oino desktop")
        || client.eq_ignore_ascii_case("oino mobile")
        || client.to_ascii_lowercase().starts_with("unipi-host");
    if from_host {
        return if host.is_empty() {
            "host".to_owned()
        } else {
            format!("host:{host}")
        };
    }
    "cli".to_owned()
}

/// The runner holding `session_file`'s lease right now, when it is a live one (D-LOCK's
/// advisory `<file>.lock.json`; core replaces it with ours when the open acquires). A
/// certainly-dead holder is not a takeover — core reclaims the stale lock silently.
fn lease_holder(session_file: &Path) -> Option<LeaseInfo> {
    let text = std::fs::read_to_string(oino_session::lock_metadata_path(session_file)).ok()?;
    let holder: LeaseInfo = serde_json::from_str(&text).ok()?;
    (!holder.dead()).then_some(holder)
}

/// The cwd a session record runs in: its header cwd when absolute, else the hub's primary.
fn record_cwd(record: &CoreHubSession, default: &Path) -> PathBuf {
    let cwd = Path::new(&record.info.cwd);
    if record.info.cwd.is_empty() || !cwd.is_absolute() {
        default.to_path_buf()
    } else {
        cwd.to_path_buf()
    }
}

/// Writes a fork of `source` under `sessions_root` and returns the new session file
/// (D-09): the new file's header gets a fresh id and `parentSession = source`, and its
/// entries are the source's **current branch** — up to and including `leaf_fork`'s entry,
/// or before the given `entry_id` (pi `fork` semantics: the entry must be a user message,
/// and the branch restarts at its parent). Nothing is woken and the source is only read.
///
/// # Errors
///
/// Reading/parsing the source, an unknown or non-user `entry_id` (core's own text), or a
/// source without conversation entries (nothing to fork).
fn fork_file(
    source: &Path,
    cwd: &Path,
    sessions_root: &Path,
    entry_id: Option<&str>,
) -> Result<PathBuf, HubError> {
    let mut manager = SessionManager::open(source, sessions_root, Some(cwd))
        .map_err(|error| HubError::failed(format!("reading {}: {error}", source.display())))?;
    let leaf = match entry_id {
        // pi `fork` before an entry: the entry must be a user message; the fork branches
        // at its parent (core `fork_with_reason`'s own rule and text).
        Some(entry) => {
            let selected = manager
                .entry(&EntryId::from(entry))
                .ok_or_else(|| HubError::failed("Invalid entry ID for forking"))?;
            if selected
                .message()
                .is_none_or(|message| message.role() != "user")
            {
                return Err(HubError::failed("Invalid entry ID for forking"));
            }
            selected.parent_id.clone()
        }
        // pi `clone`: the whole current branch, at its leaf.
        None => manager.leaf_id().cloned(),
    };
    let branched = match leaf {
        Some(leaf) => manager
            .branch_into_new_session(&leaf)
            .map_err(|error| HubError::failed(format!("forking {}: {error}", source.display())))?,
        // A fork before the first entry (or of a session without conversation entries)
        // would be an empty session; core refuses the clone with the same idea.
        None => return Err(HubError::failed("Nothing to fork yet")),
    };
    branched.ok_or_else(|| HubError::failed("Nothing to fork yet"))
}

/// `<dir>/<name>`, or `<dir>/<stem>.<n><ext>` when an archived file of that name exists.
fn unique_destination(dir: &Path, name: &str) -> PathBuf {
    let first = dir.join(name);
    if !first.exists() {
        return first;
    }
    let path = Path::new(name);
    let stem = path.file_stem().map_or_else(
        || name.to_owned(),
        |stem| stem.to_string_lossy().into_owned(),
    );
    let ext = path
        .extension()
        .map_or_else(String::new, |ext| format!(".{}", ext.to_string_lossy()));
    let candidate = (1..1000)
        .map(|n| dir.join(format!("{stem}.{n}{ext}")))
        .find(|candidate| !candidate.exists());
    candidate.unwrap_or(first)
}

impl Inner {
    /// Starts the run supervisor for `backend` unless one is already running (§4.4).
    ///
    /// The supervisor subscribes only while a run is in flight (the run already keeps the
    /// session busy), so it never pins an idle session against core's eviction.
    fn ensure_supervisor(self: &Arc<Self>, backend: Arc<dyn SessionBackend>) {
        let id = backend.session_id();
        {
            let mut state = lock(&self.state);
            if !state.supervising.insert(id.clone()) {
                return;
            }
            state.states.insert(id.clone(), HubState::Working);
        }
        // Subscribe before the caller's command is sent, so the first events of the run (a
        // crash included) can never be missed between spawn and subscription.
        let events = backend.events();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            if let Some(inner) = weak.upgrade() {
                inner.changed(&id).await;
            }
            let caught = std::panic::AssertUnwindSafe(supervise(&weak, &backend, events))
                .catch_unwind()
                .await;
            let Some(inner) = weak.upgrade() else {
                return;
            };
            match caught {
                // The stream ended (evicted, taken over, closed) or the run finished.
                Ok(final_id) => inner.finish_supervisor(&final_id, None).await,
                // A panic in the host's own supervision is contained to that session.
                Err(payload) => {
                    let error = crash::panic_message(&*payload);
                    tracing::warn!(session_id = %id, %error, "session supervisor panicked");
                    inner.finish_supervisor(&id, Some(error)).await;
                }
            }
        });
    }

    /// Records a host state and updates the card (§4.4: `crashed`/`taken_over` notice).
    async fn mark(&self, session_id: &str, state: HubState) {
        lock(&self.state)
            .states
            .insert(session_id.to_owned(), state);
        self.changed(session_id).await;
    }

    /// §8.4: the session was taken over by another runner — mark the card once and tell the
    /// Sessions list (the supervisor and the stream tap may both observe the same event).
    async fn mark_taken_over(&self, session_id: &str, from: &str) {
        {
            let mut state = lock(&self.state);
            if matches!(
                state.states.get(session_id),
                Some(HubState::TakenOver { from: existing }) if existing == from
            ) {
                return;
            }
            state.states.insert(
                session_id.to_owned(),
                HubState::TakenOver {
                    from: from.to_owned(),
                },
            );
        }
        self.changed(session_id).await;
    }

    /// Moves a supervised session's bookkeeping when the runtime replaces its session (fork,
    /// switch, new session through a backend).
    fn rekey(&self, old: &str, new: &str) {
        let mut state = lock(&self.state);
        if let Some(entry) = state.states.remove(old) {
            state.states.insert(new.to_owned(), entry);
        }
        if state.supervising.remove(old) {
            state.supervising.insert(new.to_owned());
        }
    }

    /// Ends the supervision of `session_id`: idle again, or `crashed` when the supervisor
    /// itself panicked or the session crashed. A run that ends while no connection focuses
    /// the session marks it `unread` (Z-86).
    async fn finish_supervisor(&self, session_id: &str, panic: Option<String>) {
        let ended_run;
        let had_panic = panic.is_some();
        {
            let mut state = lock(&self.state);
            ended_run = matches!(state.states.get(session_id), Some(HubState::Working));
            state.supervising.remove(session_id);
            if let Some(error) = panic {
                state
                    .states
                    .insert(session_id.to_owned(), HubState::Crashed { error });
            } else if !matches!(
                state.states.get(session_id),
                Some(HubState::Crashed { .. } | HubState::TakenOver { .. })
            ) {
                state.states.remove(session_id);
            }
        }
        if ended_run && !had_panic {
            self.mark_unread_unfocused(session_id);
        }
        self.changed(session_id).await;
    }
}

/// Follows one running session until it is idle, crashes, is evicted or is taken over; the
/// returned id is the session's id at that point (it may have been re-keyed by a fork).
async fn supervise(
    hub: &Weak<Inner>,
    backend: &Arc<dyn SessionBackend>,
    mut events: futures_util::stream::BoxStream<'static, ClientEvent>,
) -> String {
    let mut session_id = backend.session_id();
    let mut tick = tokio::time::interval(SUPERVISE_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut idle_polls = 0;
    loop {
        tokio::select! {
            event = events.next() => {
                match event {
                    Some(ClientEvent::Oino(OinoEvent::SessionReplaced {
                        session_id: new_id,
                        ..
                    })) => {
                        if let Some(hub) = hub.upgrade() {
                            hub.rekey(&session_id, &new_id);
                        }
                        session_id = new_id;
                    }
                    Some(ClientEvent::Oino(OinoEvent::SessionCrashed { error })) => {
                        if let Some(hub) = hub.upgrade() {
                            hub.mark(&session_id, HubState::Crashed { error }).await;
                        }
                    }
                    Some(ClientEvent::Oino(OinoEvent::SessionTakenOver { by })) => {
                        if let Some(hub) = hub.upgrade() {
                            hub.mark_taken_over(&session_id, &takeover_from(&by))
                                .await;
                        }
                        return session_id;
                    }
                    Some(ClientEvent::Oino(OinoEvent::SessionEvicted { .. })) | None => {
                        return session_id;
                    }
                    Some(_) => {}
                }
            }
            _ = tick.tick() => {
                if is_busy(backend).await {
                    idle_polls = 0;
                } else {
                    idle_polls += 1;
                    if idle_polls >= SUPERVISE_IDLE_POLLS {
                        return session_id;
                    }
                }
            }
        }
    }
}

/// Is a run (or compaction) in flight in `backend`? A disposed/evicted backend is not busy.
async fn is_busy(backend: &Arc<dyn SessionBackend>) -> bool {
    match backend
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await
    {
        Ok(Some(data)) => serde_json::from_value::<RpcSessionState>(data)
            .map(|state| state.is_streaming || state.is_compacting)
            .unwrap_or(false),
        _ => false,
    }
}
