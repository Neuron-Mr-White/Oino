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
//! - **Cards and project grouping** ([`super::card`]) — the Sessions list is headers only
//!   (core `list_all`), merged with the host's live state; nothing here loads a runtime.
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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use futures_util::FutureExt;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use oino_client::backend::{ClientError, SessionBackend};
use oino_client::hub::{
    HubOptions as CoreHubOptions, HubSession as CoreHubSession, HubSessionState as CoreHubState,
    SessionHub as CoreHub,
};
use oino_client::protocol::{PiCommand, RpcCommand, RpcResponseData, RpcSessionState};
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ExtensionLoader, ResourceFlags};
use oino_models::ModelRegistry;
use oino_settings::SettingsManager;

use super::card::{HubState, ProjectGroup, SessionCard};
use super::crash;

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

/// Failure of one hub operation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum HubError {
    /// A user-facing failure (unknown session, archive I/O, …).
    #[error("{0}")]
    Failed(String),
    /// The core hub rejected the call.
    #[error(transparent)]
    Client(#[from] ClientError),
}

impl HubError {
    fn failed(message: impl Into<String>) -> Self {
        Self::Failed(message.into())
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
}

/// One live session as the host hands it to a frontend (H4's stream, the tests).
pub struct HubSession {
    backend: Arc<dyn SessionBackend>,
    hub: Weak<Inner>,
    card: SessionCard,
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
    fn new(hub: Weak<Inner>, backend: Arc<dyn SessionBackend>, card: SessionCard) -> Self {
        Self { backend, hub, card }
    }

    /// Current session id (changes after a fork/switch: the runtime keeps running).
    #[must_use]
    pub fn session_id(&self) -> String {
        self.backend.session_id()
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

    /// Sends one command, making sure the session has a run supervisor (§4.4): a crash
    /// during the run marks this session and leaves the others alone.
    ///
    /// # Errors
    ///
    /// Any backend failure (the session was evicted, taken over or disposed).
    pub async fn send(&self, command: RpcCommand) -> Result<RpcResponseData, HubError> {
        if let Some(hub) = self.hub.upgrade() {
            hub.ensure_supervisor(Arc::clone(&self.backend));
        }
        self.backend.send(command).await.map_err(HubError::from)
    }
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
    started: SystemTime,
}

#[derive(Default)]
struct State {
    /// Host-side state overrides: `Working`, `Crashed{error}`, `TakenOver{from}`. Absent is
    /// idle (the record on disk is the truth).
    states: HashMap<String, HubState>,
    /// Sessions with a supervisor task.
    supervising: HashSet<String>,
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
        let core = CoreHub::create(core_options)?;
        let inner = Arc::new(Inner {
            core,
            cwd,
            agent_dir,
            sessions_root,
            events,
            notices: Mutex::new(Vec::new()),
            state: Mutex::new(State::default()),
            started: SystemTime::now(),
        });
        Ok(Arc::new(Self { inner }))
    }

    /// The primary working directory of the host.
    #[must_use]
    pub fn cwd(&self) -> &Path {
        &self.inner.cwd
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

    /// How long this hub has been alive (the safe-mode success threshold reads it).
    #[must_use]
    pub fn uptime(&self) -> Duration {
        SystemTime::now()
            .duration_since(self.inner.started)
            .unwrap_or_default()
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
    ///
    /// # Errors
    ///
    /// Listing I/O failures.
    pub async fn list(&self) -> Result<Vec<SessionCard>, HubError> {
        self.inner.list().await
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
        self.inner.create_session(cwd, name).await
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

    async fn card(&self, session_id: &str) -> Result<Option<SessionCard>, HubError> {
        Ok(self
            .list()
            .await?
            .into_iter()
            .find(|card| card.session_id == session_id))
    }

    /// The card of one record, with the host's live state merged in.
    fn card_from(&self, state: &State, record: &CoreHubSession) -> SessionCard {
        let info = &record.info;
        SessionCard {
            session_id: info.id.clone(),
            file: info.path.clone(),
            name: info.name.clone(),
            cwd: info.cwd.clone(),
            updated_at: epoch_ms(info.modified),
            message_count: info.message_count,
            state: Self::state_of(state, &info.id, record.state),
            // Extension dialogs (needs-you) ride H4's `UiRequest` stream; the card's field is
            // filled from there.
            needs_you: false,
            model: None,
        }
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
        if let Ok(records) = self.core.list().await {
            let state = lock(&self.state);
            if let Some(record) = records.iter().find(|record| record.info.id == id) {
                return self.card_from(&state, record);
            }
        }
        let mut card = SessionCard {
            session_id: id,
            file: PathBuf::new(),
            name: None,
            cwd: cwd.display().to_string(),
            updated_at: epoch_ms(SystemTime::now()),
            message_count: 0,
            state: HubState::Idle,
            needs_you: false,
            model: None,
        };
        if let Ok(Some(data)) = backend.send(RpcCommand::Pi(PiCommand::GetState)).await {
            if let Ok(session) = serde_json::from_value::<RpcSessionState>(data) {
                card.session_id = session.session_id;
                card.file = session.session_file.map(PathBuf::from).unwrap_or_default();
                card.name = session.session_name;
                card.message_count = session.message_count;
                card.model = session.model.map(|model| model.id);
            }
        }
        card.state = Self::state_of(&lock(&self.state), &card.session_id, CoreHubState::Live);
        card
    }

    /// Emits the current card of `session_id` (state/name changed).
    async fn changed(&self, session_id: &str) {
        if let Ok(Some(card)) = self.card(session_id).await {
            self.emit(HubEvent::SessionChanged { card });
        }
    }

    /// `session_new{cwd, name?}`.
    async fn create_session(
        self: &Arc<Self>,
        cwd: Option<&Path>,
        name: Option<&str>,
    ) -> Result<HubSession, HubError> {
        let cwd = cwd.map_or_else(|| self.cwd.clone(), Path::to_path_buf);
        let backend = self.core.create_session(Some(&cwd), name).await?;
        let card = self.card_for(&backend, &cwd).await;
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        Ok(HubSession::new(Arc::downgrade(self), backend, card))
    }

    /// `session_open{session}`.
    async fn open(self: &Arc<Self>, target: &str) -> Result<HubSession, HubError> {
        let record = self.resolve(target).await?;
        let id = record.info.id.clone();
        let crashed = matches!(
            Self::state_of(&lock(&self.state), &id, record.state),
            HubState::Crashed { .. }
        );
        if crashed {
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
        let cwd = record_cwd(&record, &self.cwd);
        let card = self.card_for(&backend, &cwd).await;
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        Ok(HubSession::new(Arc::downgrade(self), backend, card))
    }

    /// `session_fork{session, entry_id?, name?}`.
    async fn fork(
        self: &Arc<Self>,
        target: &str,
        entry_id: Option<&str>,
        name: Option<&str>,
    ) -> Result<HubSession, HubError> {
        let session = self.open(target).await?;
        let command = match entry_id {
            Some(entry) => PiCommand::Fork {
                entry_id: entry.to_owned(),
            },
            // pi `clone`: fork at the leaf (the whole conversation).
            None => PiCommand::Clone,
        };
        session.send(RpcCommand::Pi(command)).await?;
        if let Some(name) = name {
            session
                .send(RpcCommand::Pi(PiCommand::SetSessionName {
                    name: name.to_owned(),
                }))
                .await?;
        }
        let cwd = PathBuf::from(&session.card.cwd);
        let card = self.card_for(&session.backend, &cwd).await;
        self.emit(HubEvent::SessionChanged { card: card.clone() });
        Ok(HubSession::new(
            Arc::downgrade(self),
            Arc::clone(&session.backend),
            card,
        ))
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

    /// Resolves a `session` target (exact id, unique prefix ≥ 4 chars, or file path) against
    /// the records — core's own rules, without waking anything.
    async fn resolve(&self, target: &str) -> Result<CoreHubSession, HubError> {
        let records = self.core.list().await?;
        let path = Path::new(target);
        if path.exists() {
            return records
                .iter()
                .find(|record| record.info.path == path)
                .cloned()
                .ok_or_else(|| {
                    HubError::failed(format!("Not an Oino session in the hub: {target}"))
                });
        }
        let exact: Vec<&CoreHubSession> = records
            .iter()
            .filter(|record| record.info.id == target)
            .collect();
        if exact.len() > 1 {
            return Err(HubError::failed(format!(
                "Ambiguous session id {target}: it matches {} sessions",
                exact.len()
            )));
        }
        if let Some(record) = records.iter().find(|record| record.info.id == target) {
            return Ok(record.clone());
        }
        if target.len() >= 4 {
            let prefixed: Vec<&CoreHubSession> = records
                .iter()
                .filter(|record| record.info.id.starts_with(target))
                .collect();
            match prefixed.len() {
                1 => return Ok(prefixed[0].clone()),
                0 => {}
                count => {
                    return Err(HubError::failed(format!(
                        "Ambiguous session id {target}: it matches {count} sessions"
                    )));
                }
            }
        }
        Err(HubError::failed(format!(
            "No session found matching '{target}'"
        )))
    }
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
    /// itself panicked or the session crashed.
    async fn finish_supervisor(&self, session_id: &str, panic: Option<String>) {
        {
            let mut state = lock(&self.state);
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
                            hub.mark(&session_id, HubState::TakenOver { from: by.client }).await;
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
    match backend.send(RpcCommand::Pi(PiCommand::GetState)).await {
        Ok(Some(data)) => serde_json::from_value::<RpcSessionState>(data)
            .map(|state| state.is_streaming || state.is_compacting)
            .unwrap_or(false),
        _ => false,
    }
}
