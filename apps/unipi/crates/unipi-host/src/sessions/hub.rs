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
use unipi_proto::session_bulk::{
    query_words, SessionsBatchData, SessionsBatchFailure, SessionsQueryAutomation,
    SessionsQueryFilters, SessionsQueryScope, SessionsQuerySort, SessionsQueryStatus,
    SESSIONS_EVENT_CHUNK, SESSIONS_QUERY_LIMIT_DEFAULT, SESSIONS_QUERY_LIMIT_MAX,
};

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
    /// Owner fix 2026-11-03 (`[ux-sessions]`): one chunk of a batch archive — these
    /// sessions left the list together (`archived`: their files went to `trash/`).
    SessionsRemoved {
        /// Session ids (at most `SESSIONS_EVENT_CHUNK`).
        session_ids: Vec<String>,
        /// The files moved to the trash.
        archived: bool,
    },
    /// Owner fix 2026-11-03 (`[ux-sessions]`): one chunk of a batch restore.
    SessionsChanged {
        /// The cards (at most `SESSIONS_EVENT_CHUNK`).
        cards: Vec<SessionCard>,
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
    /// The user's home — older "No workspace" chats (cloned from UniPi) ran there; they may
    /// "Move to a workspace…" too, but the home is never copied from or deleted. Hermetic
    /// hubs (tests) treat their primary cwd as the home.
    home: Option<PathBuf>,
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
    /// What supervised sessions wait on between runs (the cards' `pending_work` /
    /// `background_work`). One entry per supervised session at most: removed when its
    /// supervisor ends.
    activity: HashMap<String, Activity>,
}

/// The between-runs facts of one supervised session (core `get_state` `waitingOn` /
/// `backgroundOn`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Activity {
    /// Work that will wake the session (it stays `Working` meanwhile).
    pending: Vec<String>,
    /// Quiet background work (no wake).
    background: Vec<String>,
}

/// Longest `pending_work` / `background_work` text on a card.
const ACTIVITY_TEXT_MAX: usize = 120;

impl Activity {
    fn text(list: &[String]) -> Option<String> {
        if list.is_empty() {
            return None;
        }
        let joined = list.join(" · ");
        Some(if joined.chars().count() > ACTIVITY_TEXT_MAX {
            let mut cut: String = joined.chars().take(ACTIVITY_TEXT_MAX - 1).collect();
            cut.push('…');
            cut
        } else {
            joined
        })
    }

    fn pending_text(&self) -> Option<String> {
        Self::text(&self.pending)
    }

    fn background_text(&self) -> Option<String> {
        Self::text(&self.background)
    }
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
        let home = if options.host_data_dir.is_some() {
            crate::fs::home_dir_opt()
        } else {
            Some(cwd.clone())
        };
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
            home,
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

    /// Drops the cached core services sets: the next command discovers the
    /// extensions again, so a freshly installed (or removed) extension package
    /// is live without restarting the host (`host_cmd`'s `reload_extensions`).
    pub async fn reset_core_services(&self) {
        self.inner.core.reset_services().await;
    }

    /// Every cached core services set (one per project): a hub-level global settings
    /// write is replayed into the others (`host_cmd`).
    pub async fn core_services_sets(&self) -> Vec<Arc<oino_core::services::Services>> {
        self.inner.core.services_sets().await
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

    /// [`Self::resolve`] that never waits for another session's wake (the core hub lock
    /// is held through a wake's whole-file parse): a busy hub answers from its live-id
    /// mirror (perf-open's file-first open).
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target, listing I/O failures.
    pub async fn resolve_quick(&self, target: &str) -> Result<SessionCard, HubError> {
        let record = self
            .inner
            .core
            .find_quick(target)
            .await
            .map_err(HubError::from_find)?;
        let state = lock(&self.inner.state);
        Ok(self.inner.card_from(&state, &record))
    }

    /// How many sessions run or wait on work right now (`Working`, background work included):
    /// the live-update swap waits for 0 (owner 2026-11-06).
    #[must_use]
    pub fn working_count(&self) -> usize {
        let state = lock(&self.inner.state);
        let working = state
            .states
            .values()
            .filter(|s| matches!(s, HubState::Working))
            .count();
        working
            + state
                .activity
                .values()
                .filter(|a| !a.background.is_empty())
                .count()
    }

    /// Is the core hub busy (a wake parsing a session file under its lock)?
    #[must_use]
    pub fn core_busy(&self) -> bool {
        self.inner.core.is_busy()
    }

    /// Whether the session's runtime is live in this hub (an open needs no wake; never
    /// waits for the hub lock).
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

    /// The session's composer draft (owner 2026-11-06), when one is stored.
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target.
    pub async fn draft_get(&self, target: &str) -> Result<Option<serde_json::Value>, HubError> {
        let id = self.inner.resolve(target).await?.info.id;
        Ok(self.inner.meta.draft(&id))
    }

    /// Stores (or clears) the session's composer draft; its card's `draft_at` follows and
    /// every client gets the card (`SessionChanged`). Returns the card.
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target, a draft too large to keep, a failed write.
    pub async fn draft_set(
        &self,
        target: &str,
        draft: Option<serde_json::Value>,
    ) -> Result<SessionCard, HubError> {
        let id = self.inner.resolve(target).await?.info.id;
        if let Some(value) = &draft {
            let size = serde_json::to_vec(value).map_or(usize::MAX, |b| b.len());
            if size > super::meta::DRAFT_MAX_BYTES {
                return Err(HubError::failed("The draft is too large to keep"));
            }
        }
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX));
        self.inner
            .meta
            .set_draft(&id, draft.as_ref(), now)
            .map_err(|error| HubError::failed(format!("Could not save the draft: {error}")))?;
        self.inner
            .card(&id)
            .await?
            .inspect(|card| {
                self.inner
                    .emit(HubEvent::SessionChanged { card: card.clone() })
            })
            .ok_or_else(|| HubError::failed(format!("No session found matching '{target}'")))
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

    /// Whether `card`'s chat belongs to a cron run still in flight (`status: running`):
    /// its `oino -p` subprocess owns the session, so an open must follow the file rather
    /// than take it over (owner-fix-6 P1-2).
    #[must_use]
    pub fn cron_run_in_flight(&self, card: &SessionCard) -> bool {
        self.inner
            .cron
            .owner_of(&card.session_id, &card.file)
            .is_some_and(|owner| owner.running)
    }

    /// Whether another live runner holds `session_file`'s single-writer lease (D-LOCK's
    /// advisory metadata; a certainly-dead holder does not count).
    #[must_use]
    pub fn lease_held_elsewhere(session_file: &Path) -> bool {
        lease_holder(session_file).is_some()
    }

    /// A cron run just started (or finished): the run's session file is written by the
    /// `oino` subprocess directly, so nothing else emits a `SessionChanged` for it and the
    /// sidebar would only show the chat after a restart. Drops the cron cache (a just
    /// started run must not wait out the 5 s read window), then emits the card when one
    /// exists; `false` = no card yet (the subprocess has not written the file — poll).
    pub async fn announce_cron_run(&self, run_id: &str) -> bool {
        self.inner.cron.invalidate();
        match self.inner.card(run_id).await {
            Ok(Some(card)) => {
                self.inner.emit(HubEvent::SessionChanged { card });
                true
            }
            _ => false,
        }
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

    /// Workspace-free restricted orchestration chat, sharing the host's stream protocol.
    ///
    /// # Errors
    /// Isolated services or session creation failed.
    pub async fn create_superagent(self: &Arc<Self>) -> Result<HubSession, HubError> {
        let runtime = crate::superagent_session::create(Arc::clone(self)).await.map_err(HubError::failed)?;
        self.open(&runtime.session().session_id()).await
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

    /// Owner fix 2026-11-03 (`sessions_archive`, `[ux-sessions]`): archive many sessions in
    /// one op — one listing resolves every target, running sessions are skipped, removals
    /// are announced in [`SESSIONS_EVENT_CHUNK`]-sized `SessionsRemoved` events.
    ///
    /// # Errors
    ///
    /// Listing I/O or creating the trash directory; per-session failures are reported in
    /// the answer instead.
    pub async fn archive_many(&self, targets: &[String]) -> Result<SessionsBatchData, HubError> {
        self.inner.archive_many(targets).await
    }

    /// Owner fix 2026-11-03 (`sessions_restore`, `[ux-sessions]`): restore many archived
    /// sessions in one op (one trash listing), announced as `SessionsChanged` chunks.
    ///
    /// # Errors
    ///
    /// Listing the trash; per-session failures are reported in the answer.
    pub async fn restore_many(&self, targets: &[String]) -> Result<SessionsBatchData, HubError> {
        self.inner.restore_many(targets).await
    }

    /// Owner fix 2026-11-03 (`sessions_query`, `[ux-sessions]`): one page of the sidebar's
    /// session search over the card indexes (live and/or trash) — title + first user
    /// message, the list's filters, newest first by default. Never wakes a runtime.
    ///
    /// # Errors
    ///
    /// Listing I/O.
    pub async fn query(&self, query: &SessionsQuery) -> Result<SessionsQueryPage, HubError> {
        debug_list_delay().await;
        self.inner.query(query).await
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
        let activity = state.activity.get(&info.id);
        self.meta.onto(SessionCard {
            session_id: info.id.clone(),
            file: info.path.clone(),
            name: info.name.clone(),
            // A chat that moved to a workspace (`oino.cwd_change`) is listed there.
            cwd: info.moved_cwd.clone().unwrap_or_else(|| info.cwd.clone()),
            updated_at: epoch_ms(info.modified),
            message_count: info.message_count,
            // owner-fix-6 P2-1: a cron run in flight is run by its `oino -p` subprocess,
            // not here — the run record is what says the chat is Working.
            state: match Self::state_of(state, &info.id, record.state) {
                HubState::Idle if cron.as_ref().is_some_and(|c| c.running) => HubState::Working,
                other => other,
            },
            // Extension dialogs (needs-you) ride H4's `UiRequest` stream; the card's field is
            // filled from there.
            needs_you: state.needs_you.contains(&info.id),
            model: None,
            pinned: false,
            unread: false,
            title_locked: false,
            draft_at: 0,
            // App spec §3.14 (WP-A23, Z-112): a run-owned session says who runs it.
            source: cron.as_ref().map(|_| "cron".to_owned()),
            cron_job_id: cron.as_ref().map(|c| c.job_id.clone()),
            cron_job_name: cron.and_then(|c| c.job_name),
            changes: None,
            last_activity: epoch_ms(info.modified),
            // Live facts of a supervised session (a record's runtime is gone: none).
            pending_work: activity.and_then(|a| a.pending_text()),
            background_work: activity.and_then(|a| a.background_text()),
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
            draft_at: 0,
            source: None,
            cron_job_id: None,
            cron_job_name: None,
            changes: None,
            last_activity: now,
            pending_work: None,
            background_work: None,
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
        {
            let state = lock(&self.state);
            card.state = Self::state_of(&state, &card.session_id, CoreHubState::Live);
            let activity = state.activity.get(&card.session_id);
            card.pending_work = activity.and_then(Activity::pending_text);
            card.background_work = activity.and_then(Activity::background_text);
        }
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
        let home = self.home.as_deref().map(canon);
        let scratch_root = canon(&self.scratch_root);
        let source =
            crate::scratch::check_move(&scratch_root, home.as_deref(), &current, &target_dir)
                .map_err(HubError::failed)?;
        // The home is never copied from (it is everything the user has).
        if copy_files && source == crate::scratch::MoveSource::Home {
            return Err(HubError::failed(
                "This chat ran in your home folder — its files can't be copied; move it without copying",
            ));
        }
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
        // The scratch folder has served its purpose (its files were copied when asked):
        // delete it — only this chat's own `<root>/<id>`, never the home. A failure here
        // leaves a stray folder, not a failed move.
        if source == crate::scratch::MoveSource::Scratch {
            self.remove_scratch(&id, &current).await;
        }
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
        if let Some(runtime) = crate::superagent_session::runtime(target) {
            let cwd = runtime.session().cwd().to_path_buf();
            let backend: Arc<dyn SessionBackend> = Arc::new(oino_client::in_process::InProcessBackend::new(runtime));
            let card = self.card_for(&backend, &cwd).await;
            return Ok(HubSession::new(Arc::downgrade(self), backend, card, None));
        }
        if let Ok(Some(card)) = oino_session::SessionManager::find_card(&self.sessions_root, target) {
            if crate::superagent_session::is_superagent_file(&card.path) {
                let runtime = crate::superagent_session::create_from(
                    // The public wrapper owns the Arc required by the restricted factory.
                    Arc::new(SessionHub { inner: Arc::clone(self) }), Some(card.path)
                ).await.map_err(HubError::failed)?;
                let cwd = runtime.session().cwd().to_path_buf();
                let backend: Arc<dyn SessionBackend> = Arc::new(oino_client::in_process::InProcessBackend::new(runtime));
                let card = self.card_for(&backend, &cwd).await;
                return Ok(HubSession::new(Arc::downgrade(self), backend, card, None));
            }
        }
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
        // A session that (re)opens waiting on work that will wake it — graph nodes resuming
        // after a restart, background subagents, a waking bg task (core asks its wait
        // sources when it binds) — is supervised and `Working` at once, not only after its
        // next run.
        if !lock(&self.state).supervising.contains(&id) && probe(&backend).await.busy() {
            self.ensure_supervisor(Arc::clone(&backend));
        }
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
        let mut cwd = record_cwd(&record, &self.cwd);
        // A scratch chat's fork gets its own scratch folder, a copy of the original's —
        // never a shared one (deleting either chat deletes only its own folder).
        let mut fork_id = None;
        if self.is_scratch_cwd(&cwd) {
            let id = uuid::Uuid::now_v7().hyphenated().to_string();
            let dir = crate::scratch::scratch_dir(&self.scratch_root, &id);
            let (from, to) = (cwd.clone(), dir.clone());
            let copied = tokio::task::spawn_blocking(move || {
                std::fs::create_dir_all(&to)?;
                crate::scratch::copy_missing(&from, &to)
            })
            .await
            .map_err(|error| HubError::failed(format!("copying the scratch folder: {error}")))?;
            if let Err(error) = copied {
                self.remove_scratch(&id, &dir).await;
                return Err(HubError::failed(format!(
                    "Copying the scratch folder for the fork failed: {error}"
                )));
            }
            cwd = dir;
            fork_id = Some(id);
        }
        // 1. Write the fork's file from the target's file (no runtime touched: the target
        //    keeps its lease, its live state and its events).
        let path = match fork_file(
            &record.info.path,
            &cwd,
            &self.sessions_root,
            entry_id,
            fork_id.clone(),
        ) {
            Ok(path) => path,
            Err(error) => {
                if let Some(id) = &fork_id {
                    self.remove_scratch(id, &cwd).await;
                }
                return Err(error);
            }
        };
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
                    draft_at: 0,
                    source: None,
                    cron_job_id: None,
                    cron_job_name: None,
                    changes: None,
                    last_activity: updated_at,
                    pending_work: None,
                    background_work: None,
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
            draft_at: 0,
            source: None,
            cron_job_id: None,
            cron_job_name: None,
            changes: None,
            last_activity: epoch_ms(entry.modified),
            pending_work: None,
            background_work: None,
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
        self.remove_scratch(&entry.id, Path::new(&entry.cwd)).await;
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
                    self.remove_scratch(&entry.id, Path::new(&entry.cwd)).await;
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

    /// Whether `cwd` is a chat's scratch folder (lexically, or once symlinks resolve).
    fn is_scratch_cwd(&self, cwd: &Path) -> bool {
        crate::scratch::is_scratch(&self.scratch_root, cwd)
            || dunce::canonicalize(cwd).is_ok_and(|cwd| {
                dunce::canonicalize(&self.scratch_root)
                    .is_ok_and(|root| crate::scratch::is_scratch(&root, &cwd))
            })
    }

    /// Deletes chat `session_id`'s own scratch folder when `cwd` is it
    /// ([`crate::scratch::remove_own`]: never a workspace, the home or another chat's
    /// folder); failures are logged, not raised (the session itself is already gone/moved).
    async fn remove_scratch(&self, session_id: &str, cwd: &Path) {
        let root = self.scratch_root.clone();
        let (id, cwd) = (session_id.to_owned(), cwd.to_path_buf());
        let removed = tokio::task::spawn_blocking(move || {
            // Compared on canonical roots too (a symlinked data dir).
            let canon_root = dunce::canonicalize(&root).unwrap_or_else(|_| root.clone());
            match crate::scratch::remove_own(&root, &id, &cwd) {
                Ok(false) if canon_root != root => {
                    crate::scratch::remove_own(&canon_root, &id, &cwd)
                }
                other => other,
            }
        })
        .await;
        match removed {
            Ok(Ok(true)) => tracing::info!(session_id, "scratch folder deleted"),
            Ok(Ok(false)) => {}
            Ok(Err(error)) => tracing::warn!(session_id, %error, "scratch folder not deleted"),
            Err(error) => tracing::warn!(session_id, %error, "scratch folder cleanup panicked"),
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
    fork_id: Option<String>,
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
            .branch_into_new_session_with_id(&leaf, fork_id)
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
    /// The supervisor subscribes only while a run is in flight or the session waits on work
    /// that will wake it (the session is busy then anyway), so it never pins an idle session
    /// against core's eviction. Quiet background work alone is followed without a
    /// subscription ([`watch_background`]).
    fn ensure_supervisor(self: &Arc<Self>, backend: Arc<dyn SessionBackend>) {
        let id = backend.session_id();
        let newly = {
            let mut state = lock(&self.state);
            let newly = state.supervising.insert(id.clone());
            if !newly
                && matches!(
                    state.states.get(&id),
                    Some(HubState::Crashed { .. } | HubState::TakenOver { .. })
                )
            {
                return;
            }
            // A supervisor following quiet background work picks the run up on its next
            // probe; the card says `Working` at once.
            let was = state.states.insert(id.clone(), HubState::Working);
            if !newly {
                if was.as_ref().is_some_and(HubState::is_working) {
                    return;
                }
                if let Some(activity) = state.activity.get_mut(&id) {
                    activity.background.clear();
                }
            }
            newly
        };
        if !newly {
            let weak = Arc::downgrade(self);
            tokio::spawn(async move {
                if let Some(inner) = weak.upgrade() {
                    inner.changed(&id).await;
                }
            });
            return;
        }
        // Subscribe before the caller's command is sent, so the first events of the run (a
        // crash included) can never be missed between spawn and subscription.
        let events = backend.events();
        let weak = Arc::downgrade(self);
        tokio::spawn(async move {
            if let Some(inner) = weak.upgrade() {
                inner.changed(&id).await;
            }
            let mut events = Some(events);
            let mut id = id;
            loop {
                let stream = events.take().unwrap_or_else(|| backend.events());
                let caught = std::panic::AssertUnwindSafe(supervise(&weak, &backend, stream))
                    .catch_unwind()
                    .await;
                let Some(inner) = weak.upgrade() else {
                    return;
                };
                match caught {
                    // Idle, but quiet background work runs on: follow it unsubscribed, and
                    // supervise again when a run starts.
                    Ok(end) if end.background => {
                        id = end.session_id;
                        inner.enter_background(&id).await;
                        drop(inner);
                        if watch_background(&weak, &backend, &id).await {
                            if let Some(inner) = weak.upgrade() {
                                inner.resume_working(&id).await;
                            }
                            continue;
                        }
                        if let Some(inner) = weak.upgrade() {
                            inner.finish_supervisor(&id, None).await;
                        }
                    }
                    // The stream ended (evicted, taken over, closed) or the run finished.
                    Ok(end) => inner.finish_supervisor(&end.session_id, None).await,
                    // A panic in the host's own supervision is contained to that session.
                    Err(payload) => {
                        let error = crash::panic_message(&*payload);
                        tracing::warn!(session_id = %id, %error, "session supervisor panicked");
                        inner.finish_supervisor(&id, Some(error)).await;
                    }
                }
                return;
            }
        });
    }

    /// The supervisor saw new between-runs facts: update the card when they changed.
    async fn set_activity(&self, session_id: &str, activity: Activity) {
        let changed = {
            let mut state = lock(&self.state);
            if !state.supervising.contains(session_id) {
                return;
            }
            let old = state.activity.get(session_id);
            if old.map_or(activity == Activity::default(), |old| *old == activity) {
                false
            } else {
                state.activity.insert(session_id.to_owned(), activity);
                true
            }
        };
        if changed {
            self.changed(session_id).await;
        }
    }

    /// A run ended with only quiet background work left: the card goes idle (with its
    /// `background_work`), and an unfocused session is `unread` (Z-86).
    async fn enter_background(&self, session_id: &str) {
        let ended_run = {
            let mut state = lock(&self.state);
            let working = matches!(state.states.get(session_id), Some(HubState::Working));
            if working {
                state.states.remove(session_id);
            }
            if let Some(activity) = state.activity.get_mut(session_id) {
                activity.pending.clear();
            }
            working
        };
        if ended_run {
            self.mark_unread_unfocused(session_id);
        }
        self.changed(session_id).await;
    }

    /// A run started on a session followed for its background work: `Working` again.
    async fn resume_working(&self, session_id: &str) {
        {
            let mut state = lock(&self.state);
            if !matches!(
                state.states.get(session_id),
                Some(HubState::Crashed { .. } | HubState::TakenOver { .. })
            ) {
                state
                    .states
                    .insert(session_id.to_owned(), HubState::Working);
            }
            if let Some(activity) = state.activity.get_mut(session_id) {
                activity.background.clear();
            }
        }
        self.changed(session_id).await;
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
        if let Some(activity) = state.activity.remove(old) {
            state.activity.insert(new.to_owned(), activity);
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
            state.activity.remove(session_id);
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

/// How [`supervise`] ended.
struct SuperviseEnd {
    /// The session's id at that point (it may have been re-keyed by a fork).
    session_id: String,
    /// Idle, with quiet background work still running ([`watch_background`] follows it).
    background: bool,
}

/// Follows one running session until it is idle (nothing it waits on left), crashes, is
/// evicted or is taken over. While the session waits on work that will wake it (core
/// `waiting`: background tasks with a wake, background subagents, graph nodes) it stays
/// supervised and its card `Working`, with the reasons as `pending_work`.
async fn supervise(
    hub: &Weak<Inner>,
    backend: &Arc<dyn SessionBackend>,
    mut events: futures_util::stream::BoxStream<'static, ClientEvent>,
) -> SuperviseEnd {
    let mut session_id = backend.session_id();
    let mut tick = tokio::time::interval(SUPERVISE_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut idle_polls = 0;
    let end = |session_id: String, background: bool| SuperviseEnd {
        session_id,
        background,
    };
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
                        return end(session_id, false);
                    }
                    // The waiting reasons changed (core publishes every change): the card
                    // follows at once instead of on the next probe.
                    Some(ClientEvent::Oino(OinoEvent::RunState {
                        waiting_on,
                        background_on,
                        ..
                    })) => {
                        if let Some(hub) = hub.upgrade() {
                            let activity = Activity {
                                pending: waiting_on.unwrap_or_default(),
                                background: background_on.unwrap_or_default(),
                            };
                            hub.set_activity(&session_id, activity).await;
                        }
                    }
                    Some(ClientEvent::Oino(OinoEvent::SessionEvicted { .. })) | None => {
                        return end(session_id, false);
                    }
                    Some(_) => {}
                }
            }
            _ = tick.tick() => {
                let probe = probe(backend).await;
                if let Some(hub) = hub.upgrade() {
                    hub.set_activity(&session_id, probe.activity()).await;
                }
                if probe.busy() {
                    idle_polls = 0;
                } else {
                    idle_polls += 1;
                    if idle_polls >= SUPERVISE_IDLE_POLLS {
                        return end(session_id, !probe.background.is_empty());
                    }
                }
            }
        }
    }
}

/// How often [`watch_background`] re-asks a session with only quiet background work.
const BACKGROUND_POLL: Duration = Duration::from_secs(5);

/// Follows an idle session's quiet background work without subscribing (the session stays
/// evictable; `get_state` does not touch its idle clock). `true` when a run started (or
/// the session waits again): supervise it; `false` when the work ended or the session
/// went away.
async fn watch_background(
    hub: &Weak<Inner>,
    backend: &Arc<dyn SessionBackend>,
    session_id: &str,
) -> bool {
    loop {
        tokio::time::sleep(BACKGROUND_POLL).await;
        let probe = probe(backend).await;
        if probe.gone {
            return false;
        }
        if probe.busy() {
            return true;
        }
        if probe.background.is_empty() {
            return false;
        }
        let Some(inner) = hub.upgrade() else {
            return false;
        };
        inner.set_activity(session_id, probe.activity()).await;
    }
}

/// One `get_state` probe of a supervised session.
#[derive(Default)]
struct Probe {
    /// A run (or compaction) is in flight.
    running: bool,
    /// Work that will wake the session (core `waiting`).
    waiting: Vec<String>,
    /// Quiet background work.
    background: Vec<String>,
    /// The backend is closed (evicted, disposed) or answered garbage.
    gone: bool,
}

impl Probe {
    /// Running or waiting on work that will wake it: keep supervising.
    fn busy(&self) -> bool {
        self.running || !self.waiting.is_empty()
    }

    /// The card facts: a running session reports neither (`Working` says it all).
    fn activity(&self) -> Activity {
        if self.running {
            return Activity::default();
        }
        Activity {
            pending: self.waiting.clone(),
            background: if self.waiting.is_empty() {
                self.background.clone()
            } else {
                Vec::new()
            },
        }
    }
}

/// Asks `backend` for its run facts. A disposed/evicted backend is gone (not busy).
async fn probe(backend: &Arc<dyn SessionBackend>) -> Probe {
    match backend
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await
    {
        Ok(Some(data)) => match serde_json::from_value::<RpcSessionState>(data) {
            Ok(state) => Probe {
                running: state.is_streaming || state.is_compacting,
                waiting: state.waiting_on,
                background: state.background_on,
                gone: false,
            },
            Err(_) => Probe {
                gone: true,
                ..Probe::default()
            },
        },
        _ => Probe {
            gone: true,
            ..Probe::default()
        },
    }
}

/// A `sessions_query` request as the hub sees it (`[ux-sessions]`).
#[derive(Clone, Debug, Default)]
pub struct SessionsQuery {
    /// Raw query text (cut and split by [`query_words`]).
    pub text: String,
    /// Live, archived or both.
    pub scope: SessionsQueryScope,
    /// The list's filters.
    pub filters: SessionsQueryFilters,
    /// Row order.
    pub sort: SessionsQuerySort,
    /// Page size (clamped to `1..=SESSIONS_QUERY_LIMIT_MAX`).
    pub limit: Option<u32>,
    /// Offset cursor of the previous page (`next_cursor`); a bad one restarts at 0.
    pub cursor: Option<String>,
}

/// One `sessions_query` hit in host terms.
#[derive(Clone, Debug)]
pub struct SessionsQueryHostHit {
    /// The card.
    pub card: SessionCard,
    /// In the trash.
    pub archived: bool,
    /// First-message excerpt when only the message matched.
    pub snippet: Option<String>,
}

/// One `sessions_query` page in host terms.
#[derive(Clone, Debug, Default)]
pub struct SessionsQueryPage {
    /// The page.
    pub hits: Vec<SessionsQueryHostHit>,
    /// Every match.
    pub total: usize,
    /// Next offset, when there is one.
    pub next_cursor: Option<String>,
}

/// A query candidate: the card plus the facts only the index knows.
struct QueryRow {
    card: SessionCard,
    archived: bool,
    first_message: String,
    created_ms: i64,
}

/// Characters of first-message context around a match in a snippet.
const SNIPPET_CONTEXT: usize = 40;

/// `true` for a folder the list files under "No workspace" (the app's `isHomeCwd`):
/// empty, `/`, a bare home directory, or a chat's scratch folder.
fn is_no_workspace(cwd: &str, scratch_root: &Path) -> bool {
    let clean = cwd.trim_end_matches(['/', '\\']);
    if clean.is_empty() || clean == "~" || clean == "/root" {
        return true;
    }
    let parts: Vec<&str> = clean.split(['/', '\\']).collect();
    let bare_home = matches!(parts.as_slice(), ["", "home", user] | ["", "Users", user] if !user.is_empty())
        || matches!(parts.as_slice(), [drive, "Users", user] if drive.ends_with(':') && !user.is_empty());
    bare_home || crate::scratch::is_scratch(scratch_root, Path::new(clean))
}

/// The list's title of a card (name, else the folder, else "New chat") — `sessionTitle`.
fn query_title(card: &SessionCard, no_workspace: bool) -> String {
    if let Some(name) = card
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty())
    {
        return name.to_owned();
    }
    if !no_workspace {
        if let Some(folder) = Path::new(card.cwd.trim_end_matches(['/', '\\'])).file_name() {
            return folder.to_string_lossy().into_owned();
        }
    }
    "New chat".to_owned()
}

/// An excerpt of `text` around the first occurrence of `word` (lowercase), one line.
fn snippet_around(text: &str, word: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let at = lower.find(word)?;
    // Byte offsets of `lower` map onto `text` only for same-length case folding; fall back
    // to a char walk over the lowercase copy (what is shown is lowercase-safe text).
    let source = if lower.len() == text.len() {
        text
    } else {
        lower.as_str()
    };
    let chars: Vec<(usize, char)> = source.char_indices().collect();
    let pos = chars.iter().position(|(i, _)| *i >= at).unwrap_or(0);
    let start = pos.saturating_sub(SNIPPET_CONTEXT);
    let end = (pos + word.chars().count() + SNIPPET_CONTEXT).min(chars.len());
    let mut out: String = chars[start..end].iter().map(|(_, c)| *c).collect();
    out = out.split_whitespace().collect::<Vec<_>>().join(" ");
    if start > 0 {
        out.insert(0, '…');
    }
    if end < chars.len() {
        out.push('…');
    }
    Some(out)
}

impl Inner {
    /// [`SessionHub::archive_many`].
    async fn archive_many(&self, targets: &[String]) -> Result<SessionsBatchData, HubError> {
        let started = Instant::now();
        let records = self.core.list().await?;
        let by_id: HashMap<&str, &CoreHubSession> =
            records.iter().map(|r| (r.info.id.as_str(), r)).collect();
        let trash = self.agent_dir.join("trash");
        tokio::fs::create_dir_all(&trash)
            .await
            .map_err(|error| HubError::failed(format!("creating {}: {error}", trash.display())))?;
        let mut out = SessionsBatchData::default();
        let mut chunk: Vec<String> = Vec::with_capacity(SESSIONS_EVENT_CHUNK);
        let mut seen: HashSet<String> = HashSet::with_capacity(targets.len());
        for target in targets {
            let fail = |out: &mut SessionsBatchData, message: String| {
                out.failed.push(SessionsBatchFailure {
                    session: target.clone(),
                    message,
                });
            };
            // Exact ids come from the one listing; anything else resolves like `session_archive`.
            let resolved;
            let record = if let Some(record) = by_id.get(target.as_str()) {
                *record
            } else {
                match self.resolve(target).await {
                    Ok(record) => {
                        resolved = record;
                        &resolved
                    }
                    Err(error) => {
                        fail(&mut out, error.to_string());
                        continue;
                    }
                }
            };
            let id = record.info.id.clone();
            if !seen.insert(id.clone()) {
                continue;
            }
            let working = {
                let state = lock(&self.state);
                matches!(state.states.get(&id), Some(HubState::Working))
                    || state.needs_you.contains(&id)
            };
            if working {
                fail(&mut out, "the session is running".to_owned());
                continue;
            }
            if let Err(error) = self.core.close(&id).await {
                fail(&mut out, error.to_string());
                continue;
            }
            lock(&self.state).states.remove(&id);
            let name = record.info.path.file_name().map_or_else(
                || format!("{id}.jsonl"),
                |name| name.to_string_lossy().into_owned(),
            );
            let dest = unique_destination(&trash, &name);
            if let Err(error) = tokio::fs::rename(&record.info.path, &dest).await {
                fail(
                    &mut out,
                    format!("archiving {}: {error}", record.info.path.display()),
                );
                continue;
            }
            out.done.push(id.clone());
            chunk.push(id);
            if chunk.len() >= SESSIONS_EVENT_CHUNK {
                self.emit(HubEvent::SessionsRemoved {
                    session_ids: std::mem::take(&mut chunk),
                    archived: true,
                });
                // Let the event pumps and other requests run between chunks.
                tokio::task::yield_now().await;
            }
        }
        if !chunk.is_empty() {
            self.emit(HubEvent::SessionsRemoved {
                session_ids: chunk,
                archived: true,
            });
        }
        tracing::info!(
            archived = out.done.len(),
            failed = out.failed.len(),
            took_ms = started.elapsed().as_millis() as u64,
            "sessions archived (batch)"
        );
        Ok(out)
    }

    /// [`SessionHub::restore_many`].
    async fn restore_many(&self, targets: &[String]) -> Result<SessionsBatchData, HubError> {
        let started = Instant::now();
        let archived = self.archived_sessions().await?;
        let by_id: HashMap<&str, &oino_session::manager::SessionInfo> = archived
            .iter()
            .map(|info| (info.id.as_str(), info))
            .collect();
        let mut out = SessionsBatchData::default();
        let mut chunk: Vec<SessionCard> = Vec::new();
        let mut seen: HashSet<String> = HashSet::with_capacity(targets.len());
        for target in targets {
            let Some(entry) = by_id.get(target.as_str()) else {
                // Not an exact id: the single op's resolution (prefix / path) and its event.
                match self.restore(target).await {
                    Ok(card) => out.done.push(card.session_id),
                    Err(error) => out.failed.push(SessionsBatchFailure {
                        session: target.clone(),
                        message: error.to_string(),
                    }),
                }
                continue;
            };
            if !seen.insert(entry.id.clone()) {
                continue;
            }
            match self.restore_entry(entry).await {
                Ok(card) => {
                    out.done.push(card.session_id.clone());
                    chunk.push(card);
                    if chunk.len() >= SESSIONS_EVENT_CHUNK {
                        self.emit(HubEvent::SessionsChanged {
                            cards: std::mem::take(&mut chunk),
                        });
                        tokio::task::yield_now().await;
                    }
                }
                Err(error) => out.failed.push(SessionsBatchFailure {
                    session: target.clone(),
                    message: error.to_string(),
                }),
            }
        }
        if !chunk.is_empty() {
            self.emit(HubEvent::SessionsChanged { cards: chunk });
        }
        tracing::info!(
            restored = out.done.len(),
            failed = out.failed.len(),
            took_ms = started.elapsed().as_millis() as u64,
            "sessions restored (batch)"
        );
        Ok(out)
    }

    /// Moves one archived file back (the body of `restore`, without its event): a cheap
    /// card from the archived entry — the listing re-reads it from the index anyway.
    async fn restore_entry(
        &self,
        entry: &oino_session::manager::SessionInfo,
    ) -> Result<SessionCard, HubError> {
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
        let modified = epoch_ms(entry.modified);
        let cron = self.cron.owner_of(&entry.id, &dest);
        Ok(self.meta.onto(SessionCard {
            session_id: entry.id.clone(),
            file: dest,
            name: entry.name.clone(),
            cwd: entry.moved_cwd.clone().unwrap_or_else(|| entry.cwd.clone()),
            updated_at: modified,
            message_count: entry.message_count,
            state: HubState::Idle,
            needs_you: false,
            model: None,
            pinned: false,
            unread: false,
            title_locked: false,
            draft_at: 0,
            source: cron.as_ref().map(|_| "cron".to_owned()),
            cron_job_id: cron.as_ref().map(|c| c.job_id.clone()),
            cron_job_name: cron.and_then(|c| c.job_name),
            changes: None,
            last_activity: modified,
            pending_work: None,
            background_work: None,
        }))
    }

    /// [`SessionHub::query`].
    async fn query(&self, query: &SessionsQuery) -> Result<SessionsQueryPage, HubError> {
        let started = Instant::now();
        let mut rows: Vec<QueryRow> = Vec::new();
        if query.scope != SessionsQueryScope::Archived {
            let records = self.core.list().await?;
            let state = lock(&self.state);
            rows.extend(records.iter().map(|record| QueryRow {
                card: self.card_from(&state, record),
                archived: false,
                first_message: record.info.first_message.clone(),
                created_ms: record.info.created.to_ms().unwrap_or_else(|| {
                    i64::try_from(epoch_ms(record.info.modified)).unwrap_or(i64::MAX)
                }),
            }));
        }
        if query.scope != SessionsQueryScope::Live {
            for info in self.archived_sessions().await? {
                let modified = epoch_ms(info.modified);
                let created_ms = info
                    .created
                    .to_ms()
                    .unwrap_or_else(|| i64::try_from(modified).unwrap_or(i64::MAX));
                let card = self.meta.onto(SessionCard {
                    session_id: info.id.clone(),
                    file: info.path.clone(),
                    name: info.name.clone(),
                    cwd: info.moved_cwd.clone().unwrap_or_else(|| info.cwd.clone()),
                    updated_at: modified,
                    message_count: info.message_count,
                    state: HubState::Idle,
                    needs_you: false,
                    model: None,
                    pinned: false,
                    unread: false,
                    title_locked: false,
                    draft_at: 0,
                    source: None,
                    cron_job_id: None,
                    cron_job_name: None,
                    changes: None,
                    last_activity: modified,
                    pending_work: None,
                    background_work: None,
                });
                rows.push(QueryRow {
                    card,
                    archived: true,
                    first_message: info.first_message,
                    created_ms,
                });
            }
        }
        let words = query_words(&query.text);
        let filters = &query.filters;
        let workspaces: HashSet<&str> = filters.workspaces.iter().map(String::as_str).collect();
        let mut hits: Vec<(QueryRow, String, Option<String>)> = Vec::new();
        for row in rows {
            let no_workspace = is_no_workspace(&row.card.cwd, &self.scratch_root);
            if !workspaces.is_empty() {
                let key = if no_workspace {
                    ""
                } else {
                    row.card.cwd.trim_end_matches(['/', '\\'])
                };
                if !workspaces.contains(key) {
                    continue;
                }
            }
            if !filters.automation.is_empty() {
                let automation = if row.card.source.as_deref() == Some("cron") {
                    SessionsQueryAutomation::Cron
                } else {
                    SessionsQueryAutomation::Manual
                };
                if !filters.automation.contains(&automation) {
                    continue;
                }
            }
            if !filters.status.is_empty() {
                let status = if row.card.needs_you {
                    SessionsQueryStatus::Waiting
                } else {
                    match row.card.state {
                        HubState::Working => SessionsQueryStatus::Running,
                        HubState::Crashed { .. } => SessionsQueryStatus::Failed,
                        HubState::Idle | HubState::TakenOver { .. } => SessionsQueryStatus::Ready,
                    }
                };
                if !filters.status.contains(&status) {
                    continue;
                }
            }
            if !filters.models.is_empty()
                && !filters
                    .models
                    .iter()
                    .any(|m| m == row.card.model.as_deref().unwrap_or(""))
            {
                continue;
            }
            if filters
                .created
                .is_some_and(|range| !range.contains(row.created_ms))
            {
                continue;
            }
            let activity = i64::try_from(row.card.last_activity).unwrap_or(i64::MAX);
            if filters
                .updated
                .is_some_and(|range| !range.contains(activity))
            {
                continue;
            }
            let title = query_title(&row.card, no_workspace);
            let mut snippet = None;
            if !words.is_empty() {
                let title_lower = title.to_lowercase();
                // The index's placeholder for an empty chat is not searchable text.
                let first = if row.first_message == "(no messages)" {
                    ""
                } else {
                    row.first_message.as_str()
                };
                let first_lower = first.to_lowercase();
                if !words
                    .iter()
                    .all(|w| title_lower.contains(w.as_str()) || first_lower.contains(w.as_str()))
                {
                    continue;
                }
                if let Some(word) = words.iter().find(|w| !title_lower.contains(w.as_str())) {
                    snippet = snippet_around(first, word);
                }
            }
            hits.push((row, title, snippet));
        }
        match query.sort {
            SessionsQuerySort::Updated => hits.sort_by(|a, b| {
                b.0.card
                    .last_activity
                    .cmp(&a.0.card.last_activity)
                    .then_with(|| a.0.card.session_id.cmp(&b.0.card.session_id))
            }),
            SessionsQuerySort::Created => hits.sort_by(|a, b| {
                b.0.created_ms
                    .cmp(&a.0.created_ms)
                    .then_with(|| a.0.card.session_id.cmp(&b.0.card.session_id))
            }),
            SessionsQuerySort::Name => {
                hits.sort_by_cached_key(|h| (h.1.to_lowercase(), h.0.card.session_id.clone()))
            }
        }
        let limit = query
            .limit
            .unwrap_or(SESSIONS_QUERY_LIMIT_DEFAULT)
            .clamp(1, SESSIONS_QUERY_LIMIT_MAX) as usize;
        let total = hits.len();
        let start = query
            .cursor
            .as_deref()
            .and_then(|c| c.parse::<usize>().ok())
            .unwrap_or(0)
            .min(total);
        let page: Vec<SessionsQueryHostHit> = hits
            .into_iter()
            .skip(start)
            .take(limit)
            .map(|(row, _, snippet)| SessionsQueryHostHit {
                card: row.card,
                archived: row.archived,
                snippet,
            })
            .collect();
        let next = start + page.len();
        tracing::info!(
            words = words.len(),
            total,
            took_ms = started.elapsed().as_millis() as u64,
            "sessions query"
        );
        Ok(SessionsQueryPage {
            hits: page,
            total,
            next_cursor: (next < total).then(|| next.to_string()),
        })
    }
}

#[cfg(test)]
mod query_helper_tests {
    use super::*;

    #[test]
    fn no_workspace_matches_the_app() {
        let scratch = Path::new("/home/u/.local/share/oino/scratch");
        assert!(is_no_workspace("", scratch));
        assert!(is_no_workspace("/", scratch));
        assert!(is_no_workspace("/home/u/", scratch));
        assert!(is_no_workspace("/Users/u", scratch));
        assert!(is_no_workspace("C:\\Users\\u", scratch));
        assert!(!is_no_workspace("/home/u/Projects/x", scratch));
    }

    #[test]
    fn snippets_are_short_and_one_line() {
        let text = format!(
            "{}\nfind the Needle here{}",
            "a ".repeat(100),
            " b".repeat(100)
        );
        let s = snippet_around(&text, "needle").unwrap();
        assert!(
            s.contains("Needle") && s.starts_with('…') && s.ends_with('…') && !s.contains('\n')
        );
        assert!(s.chars().count() < 2 * SNIPPET_CONTEXT + 12);
        assert!(snippet_around("abc", "zz").is_none());
    }
}
