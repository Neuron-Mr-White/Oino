//! E-LAZY lazy session hub (core spec §2.2, WP-C7): a session is a record on disk until a
//! message arrives (infinity's "spawn driver on demand" as a core type).
//!
//! [`SessionHub`] lists session records with no services, engines or leases; `open`/`attach`
//! wake one (acquire lease → repair → `AgentSessionRuntime` → [`InProcessBackend`]); a live
//! session with no run, compaction, arbiter wait, open tools, live children or subscribers
//! for `extensionSettings.hub.sessionIdleSec` (default 300 s) is evicted — disposed
//! (`session_shutdown{reason:"idle"}`, lease released, guests drop their per-session state)
//! and returns to a record that re-opens and repairs on demand. A services set with zero
//! live sessions and idle for `servicesIdleSec` (default 300 s, measured from last
//! session activity) is disposed (extension `shutdown`), unless running work pins it;
//! at most `maxServices` (default 4) are kept, least-recently-used first — an idle set is
//! evicted (its idle sessions return to records) to make room, and when every set has a
//! busy session the hub rather exceeds the cap by one than refuses the open.
//!
//! Settings (`extensionSettings.hub.*`, §2.2): `sessionIdleSec`/`servicesIdleSec` `0`
//! disables eviction — the TUI's single-session runtime does not use the hub at all.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::Duration;
use tokio::time::Instant;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::{JsonObject, JsonValue};
use oino_core::agents::AgentRunState;
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource, ToolSelection};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::session::{SessionOpError, ToolEntry};
use oino_core::ui::UiResponse;
use oino_models::ModelRegistry;
use oino_session::{SessionInfo, SessionManager, lock_metadata_path};
use oino_settings::SettingsManager;

use crate::backend::{ClientError, SessionBackend};
use crate::in_process::InProcessBackend;
use crate::protocol::{AgentCommand, RpcCommand, RpcResponseData};

/// `extensionSettings.hub.sessionIdleSec` default (ruling, core spec §7).
pub const DEFAULT_SESSION_IDLE_SECS: u64 = 300;
/// `extensionSettings.hub.servicesIdleSec` default (core spec §2.2).
pub const DEFAULT_SERVICES_IDLE_SECS: u64 = 300;
/// `extensionSettings.hub.maxServices` default (core spec §2.2).
pub const DEFAULT_MAX_SERVICES: usize = 4;

/// Lower bound of the idle monitor's poll interval (tests use sub-second idles).
const MIN_POLL: Duration = Duration::from_millis(100);
/// Upper bound of the idle monitor's poll interval.
const MAX_POLL: Duration = Duration::from_secs(2);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Pure services routing: preserve path spelling (including symlinks and case),
/// matching the in-process cache. Relative/empty record cwds use the primary cwd.
#[must_use]
pub fn services_key(session_cwd: &Path, shared_root: Option<&Path>, default_cwd: &Path) -> PathBuf {
    let cwd = if session_cwd.is_absolute() {
        session_cwd
    } else {
        default_cwd
    };
    match shared_root {
        Some(root) if cwd.parent() == Some(root) => root.to_path_buf(),
        _ => cwd.to_path_buf(),
    }
}

/// Snapshot of lifecycle ownership. Zero busy work alone is not enough to exit:
/// idle sessions and services must first retire via the normal monitor.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BusyReport {
    /// Session runtimes still owned by this hub.
    pub live_sessions: usize,
    /// Shared project service sets still owned by this hub.
    pub services_sets: usize,
    /// Sessions currently executing work.
    pub busy_sessions: usize,
    /// Service sets with background keepalive work.
    pub keepalive_services: usize,
}

impl BusyReport {
    #[must_use]
    /// Whether all sessions and service sets have retired.
    pub fn is_empty(&self) -> bool {
        self.live_sessions == 0 && self.services_sets == 0
    }
}

/// One session as the hub sees it (E-LAZY).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HubSession {
    /// The record (path, id, name, counts) — read from disk, never woken.
    pub info: SessionInfo,
    /// Live-ness.
    pub state: HubSessionState,
}

/// Live-ness of one hub session (E-LAZY): a record until a message arrives.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HubSessionState {
    /// On disk only: no runtime, engine or lease in this process.
    #[default]
    Record,
    /// Open in this process (runtime + lease held).
    Live,
    /// A session task crashed and was contained (X-PANIC): listed `crashed` until
    /// re-opened from the record.
    Crashed,
    /// Another runner holds the lease and did not respond to the takeover request
    /// (D-TAKEOVER deadline): the last `open` failed with `SessionLocked`.
    Locked,
}

/// Inputs of [`SessionHub::create`].
pub struct HubOptions {
    /// Primary working directory (settings project file, default services cwd).
    pub cwd: PathBuf,
    /// Agent dir (`<agent>/sessions`, settings, state).
    pub agent_dir: PathBuf,
    /// Bundled docs root; defaults to `<agent_dir>/docs`.
    pub docs_root: Option<PathBuf>,
    /// Sessions root override (`--session-dir`); the default chain mirrors
    /// [`oino_core::services::Services::sessions_root`].
    pub session_dir: Option<PathBuf>,
    /// Run mode (lease holder label, extension contexts).
    pub mode: ExtensionMode,
    /// Dialog UI available.
    pub has_ui: bool,
    /// Trust prompt override (`-a`/`-na`); `None` decides from the trust store.
    pub approve: Option<bool>,
    /// CLI resource flags.
    pub flags: ResourceFlags,
    /// Extension loader (CLI discovery today, core `sources` after WP-C10).
    pub loader: Arc<dyn ExtensionLoader>,
    /// Model registry override (tests); default reads `<agent>/models.json` + `auth.json`.
    pub models: Option<ModelRegistry>,
    /// Tool selection applied to every woken session.
    pub tools: ToolSelection,
    /// `--exclude-tools` applied to every woken session.
    pub exclude_tools: Vec<String>,
}

impl HubOptions {
    /// The sessions root of these options: `session_dir` > `OINO_SESSION_DIR` >
    /// `sessionDir` setting > `<agent_dir>/sessions` (the `Services` chain, resolved once
    /// so hub listing and the runtimes it spawns agree).
    #[must_use]
    pub fn sessions_root(&self, settings: &SettingsManager) -> PathBuf {
        if let Some(dir) = &self.session_dir {
            return dir.clone();
        }
        if let Some(dir) = std::env::var_os("OINO_SESSION_DIR").filter(|d| !d.is_empty()) {
            return PathBuf::from(dir);
        }
        settings.settings().session_dir().map_or_else(
            || self.agent_dir.join("sessions"),
            |dir| {
                if dir.is_absolute() {
                    dir
                } else {
                    self.cwd.join(dir)
                }
            },
        )
    }
}

/// Host-level limits of one hub, layered over the `extensionSettings.hub.*` values;
/// [`HubLimits::default`] keeps every setting. Kept separate from [`HubOptions`] (a
/// plain struct literal, frozen at 1.0) so new knobs here stay additive:
/// `#[non_exhaustive]`, grown by builder methods only.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct HubLimits {
    /// Overrides `extensionSettings.hub.maxServices` (`None` keeps the setting, default 4).
    /// Every cached services set holds one wasm instance per extension — preopen dirs and
    /// log descriptors — so a host with a low `RLIMIT_NOFILE` cannot afford the default.
    max_services: Option<usize>,
    /// Sessions whose cwd is a direct child of this dir share the services set of the dir
    /// itself (see [`HubLimits::with_shared_services_root`]).
    shared_services_root: Option<PathBuf>,
    /// See [`HubLimits::with_session_tools`].
    session_tools: Option<SessionTools>,
}

/// Extra custom tools for a session the hub wakes from its record: called with the
/// session file and id; an empty list adds nothing.
#[derive(Clone)]
pub struct SessionTools(pub Arc<SessionToolsFn>);

/// The [`SessionTools`] callback: session file and id → extra tools.
pub type SessionToolsFn = dyn Fn(&Path, &str) -> Vec<ToolEntry> + Send + Sync;

impl std::fmt::Debug for SessionTools {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SessionTools(..)")
    }
}

impl HubLimits {
    /// Caps this hub at `max` services sets, overriding `extensionSettings.hub.maxServices`
    /// (clamped to `1..=1024` at create). An embedding host derives this from its own
    /// file-descriptor budget, not from user settings.
    #[must_use]
    pub fn with_max_services(mut self, max: usize) -> Self {
        self.max_services = Some(max);
        self
    }

    /// Sessions whose cwd is a direct child of `root` run on the one services set of `root`
    /// (settings, resources, extensions) while each keeps its own cwd — the desktop's
    /// per-chat "No workspace" scratch folders, which would otherwise cost one services
    /// set (and its descriptors) per chat.
    #[must_use]
    pub fn with_shared_services_root(mut self, root: PathBuf) -> Self {
        self.shared_services_root = Some(root);
        self
    }

    /// Custom tools the embedding host attaches when the hub wakes a session from its
    /// file, e.g. the desktop's board tool for a dispatcher lead the user reopens. The
    /// session's recorded tool declarations survive a reopen, so a tool that existed only
    /// in the original runtime would otherwise be declared but unavailable.
    #[must_use]
    pub fn with_session_tools(mut self, tools: SessionTools) -> Self {
        self.session_tools = Some(tools);
        self
    }
}

/// `extensionSettings.hub.*` (§2.2): idle windows (`None` = disabled) and the services LRU
/// cap. Defaults 300/300/4 (owner ruling).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct HubConfig {
    /// `sessionIdleSec` (`0` disables session eviction).
    session_idle: Option<Duration>,
    /// `servicesIdleSec` (`0` disables services disposal).
    services_idle: Option<Duration>,
    /// `maxServices`.
    max_services: usize,
}

impl HubConfig {
    fn read(merged: &JsonObject) -> Self {
        let hub = merged
            .get("extensionSettings")
            .and_then(|e| e.get("hub"))
            .and_then(JsonValue::as_object);
        let secs = |key: &str, default: u64| {
            let raw = hub
                .and_then(|h| h.get(key))
                .and_then(JsonValue::as_u64)
                .unwrap_or(default);
            (raw > 0).then(|| Duration::from_secs(raw.min(u64::from(u32::MAX))))
        };
        let max_services = hub
            .and_then(|h| h.get("maxServices"))
            .and_then(JsonValue::as_u64)
            .unwrap_or(DEFAULT_MAX_SERVICES as u64);
        Self {
            session_idle: secs("sessionIdleSec", DEFAULT_SESSION_IDLE_SECS),
            services_idle: secs("servicesIdleSec", DEFAULT_SERVICES_IDLE_SECS),
            max_services: usize::try_from(max_services.clamp(1, 1024)).unwrap_or(1),
        }
    }

    /// The monitor's poll interval: a quarter of the shortest idle window.
    fn poll(&self) -> Option<Duration> {
        let shortest = [self.session_idle, self.services_idle]
            .into_iter()
            .flatten()
            .min()?;
        Some((shortest / 4).clamp(MIN_POLL, MAX_POLL))
    }
}

/// Services cache key: one `Services` per `(cwd, agent_dir)` (E-LAZY; the engine itself is
/// process-shared, E-ENGINE).
type ServicesKey = (PathBuf, PathBuf);

/// One cached services set.
struct ServicesEntry {
    /// Cache key.
    key: ServicesKey,
    /// The services (dropped on disposal).
    services: Arc<Services>,
    /// LRU clock.
    last_used: Instant,
    /// When the last live session of this group left; `None` while any is live.
    idle_since: Option<Instant>,
}

/// One live session: the runtime plus the shared bookkeeping the watcher and idle monitor
/// update (the session id changes on replacement, so it lives behind a mutex).
struct Live {
    runtime: Arc<AgentSessionRuntime>,
    backend: Arc<InProcessBackend>,
    services: ServicesKey,
    /// Current session id (re-keyed on `session_replaced`).
    id: Arc<Mutex<String>>,
    /// Last activity: any client event, command or busy predicate.
    last_active: Mutex<Instant>,
    /// Live operation count, or RETIRING/GONE. CAS from zero commits idle eviction.
    admission: AtomicUsize,
    retired: tokio::sync::Notify,
}

/// The live runtimes by session id, mirrored into a plain id set that is readable while
/// a wake holds the hub lock ([`SessionHub::is_live`], [`SessionHub::find_quick`]): a wake
/// parses the whole session file under the lock, and asking "is it live?" must not wait
/// for somebody else's wake.
struct LiveMap {
    map: HashMap<String, Arc<Live>>,
    ids: Arc<Mutex<HashSet<String>>>,
}

impl std::ops::Deref for LiveMap {
    type Target = HashMap<String, Arc<Live>>;

    fn deref(&self) -> &Self::Target {
        &self.map
    }
}

impl LiveMap {
    fn insert(&mut self, id: String, live: Arc<Live>) -> Option<Arc<Live>> {
        lock(&self.ids).insert(id.clone());
        self.map.insert(id, live)
    }

    fn remove(&mut self, id: &str) -> Option<Arc<Live>> {
        lock(&self.ids).remove(id);
        self.map.remove(id)
    }
}

/// Hub state guarded by one async mutex: open/close/evict transitions are serialised, so a
/// wake never races an eviction of the same session.
struct State {
    services: Vec<ServicesEntry>,
    live: LiveMap,
    crashed: HashSet<String>,
    /// Session id → (file, holder text) of a failed takeover (D-TAKEOVER deadline).
    locked: HashMap<String, (PathBuf, String)>,
}

struct Inner {
    cwd: PathBuf,
    agent_dir: PathBuf,
    docs_root: PathBuf,
    sessions_root: PathBuf,
    mode: ExtensionMode,
    has_ui: bool,
    approve: Option<bool>,
    flags: ResourceFlags,
    loader: Arc<dyn ExtensionLoader>,
    models: Option<ModelRegistry>,
    tools: ToolSelection,
    exclude_tools: Vec<String>,
    config: HubConfig,
    /// [`HubLimits::with_shared_services_root`].
    shared_services_root: Option<PathBuf>,
    /// [`HubLimits::with_session_tools`].
    session_tools: Option<SessionTools>,
    /// Ids of the live runtimes ([`LiveMap`]'s lock-free mirror).
    live_ids: Arc<Mutex<HashSet<String>>>,
    state: tokio::sync::Mutex<State>,
}

/// The lazy session hub (E-LAZY): records on disk until a message arrives.
///
/// Listing never creates services, engines or leases; `open`/`attach` wake a record
/// (lease + repair + runtime); idle sessions are evicted back to records. Cheap to hold:
/// the idle monitor exits when the hub is dropped.
pub struct SessionHub {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for SessionHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionHub")
            .field("sessions_root", &self.inner.sessions_root)
            .field("config", &self.inner.config)
            .finish_non_exhaustive()
    }
}

impl SessionHub {
    /// Creates the hub: resolves the sessions root, reads `extensionSettings.hub.*` and
    /// starts the idle monitor (unless both idles are disabled). See [`Self::create_with`]
    /// for host-level limits over those settings.
    ///
    /// # Errors
    ///
    /// Never today; `Result` keeps room for settings failures.
    pub fn create(opts: HubOptions) -> Result<Arc<Self>, ClientError> {
        Self::create_with(opts, HubLimits::default())
    }

    /// [`Self::create`] with host-level `limits` layered over `extensionSettings.hub.*`:
    /// the embedding host's process budgets (fd limits), which the user did not set.
    ///
    /// # Errors
    ///
    /// Never today; `Result` keeps room for settings failures.
    pub fn create_with(opts: HubOptions, limits: HubLimits) -> Result<Arc<Self>, ClientError> {
        let settings = SettingsManager::create(&opts.cwd, &opts.agent_dir, false);
        let mut config = HubConfig::read(settings.settings().merged());
        if let Some(max) = limits.max_services {
            config.max_services = max.clamp(1, 1024);
        }
        let sessions_root = opts.sessions_root(&settings);
        let docs_root = opts
            .docs_root
            .clone()
            .unwrap_or_else(|| opts.agent_dir.join("docs"));
        let live_ids = Arc::new(Mutex::new(HashSet::new()));
        let inner = Arc::new(Inner {
            live_ids: Arc::clone(&live_ids),
            cwd: opts.cwd,
            agent_dir: opts.agent_dir,
            docs_root,
            sessions_root,
            mode: opts.mode,
            has_ui: opts.has_ui,
            approve: opts.approve,
            flags: opts.flags,
            loader: opts.loader,
            models: opts.models,
            tools: opts.tools,
            exclude_tools: opts.exclude_tools,
            config,
            shared_services_root: limits.shared_services_root,
            session_tools: limits.session_tools,
            state: tokio::sync::Mutex::new(State {
                services: Vec::new(),
                live: LiveMap {
                    map: HashMap::new(),
                    ids: live_ids,
                },
                crashed: HashSet::new(),
                locked: HashMap::new(),
            }),
        });
        if let Some(poll) = config.poll() {
            let weak = Arc::downgrade(&inner);
            tokio::spawn(async move { monitor(weak, poll).await });
        }
        Ok(Arc::new(Self { inner }))
    }

    /// Wakes `target` (exact session id, unique id prefix ≥ 4 chars, or a session file
    /// path) and returns its backend: lease + repair + `AgentSessionRuntime`
    /// (D-TAKEOVER/D-REPAIR through the runtime) → [`InProcessBackend`]. A live session is
    /// returned as-is. A session locked by an unresponsive holder fails with the precise
    /// D-TAKEOVER text and is listed [`HubSessionState::Locked`].
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target; services or session-open failures (with
    /// [`ClientError::Command`] texts, `SessionLocked` preserved).
    pub async fn open(&self, target: &str) -> Result<Arc<dyn SessionBackend>, ClientError> {
        self.wake(target).await
    }

    /// Creates a fresh session in `cwd` (the hub's primary cwd when
    /// `None`) and returns its backend. Services come from the hub's per-cwd cache; the
    /// runtime starts over `SessionSource::New`, so core writes the session file at the
    /// first conversation entry (durability rule), and `name` is recorded as the session's
    /// `session_info` name. The live entry is keyed by the new session's id, so later
    /// [`SessionHub::open`]/[`SessionHub::attach`] calls reuse this runtime.
    ///
    /// # Errors
    ///
    /// Services creation or session-open failures.
    pub async fn create_session(
        &self,
        cwd: Option<&Path>,
        name: Option<&str>,
    ) -> Result<Arc<dyn SessionBackend>, ClientError> {
        self.create_session_with_model(cwd, name, None).await
    }

    /// [`SessionHub::create_session`] with an explicit initial model (the host's
    /// `session_new` picker, D-01): the model is the session's starting selection —
    /// no `model_change` entry, nothing persisted — exactly like the CLI's resolved
    /// `--model`. `None` keeps [`SessionOptions`]' own initial selection.
    ///
    /// # Errors
    ///
    /// Services creation or session-open failures.
    pub async fn create_session_with_model(
        &self,
        cwd: Option<&Path>,
        name: Option<&str>,
        model: Option<oino_ai::Model>,
    ) -> Result<Arc<dyn SessionBackend>, ClientError> {
        self.create_session_with_id(cwd, name, model, None).await
    }

    /// [`SessionHub::create_session_with_model`] with an explicit session id (`None` = a
    /// fresh `UUIDv7`): the embedding host can name a per-session folder (the desktop's
    /// "No workspace" scratch dir) after the id before the runtime exists.
    ///
    /// # Errors
    ///
    /// Services creation or session-open failures (an invalid id included).
    pub async fn create_session_with_id(
        &self,
        cwd: Option<&Path>,
        name: Option<&str>,
        model: Option<oino_ai::Model>,
        id: Option<String>,
    ) -> Result<Arc<dyn SessionBackend>, ClientError> {
        let session_cwd = cwd.map_or_else(|| self.inner.cwd.clone(), Path::to_path_buf);
        let cwd = self.services_cwd(&session_cwd);
        self.refresh_cached_project(&cwd).await?;
        let mut state = self.inner.state.lock().await;
        let services = self.services_for(&mut state, &cwd).await?;
        let options = SessionOptions {
            source: id.map_or(SessionSource::New, SessionSource::NewWithId),
            cwd: (session_cwd != cwd).then_some(session_cwd),
            session_dir: Some(self.inner.sessions_root.clone()),
            name: name.map(str::to_owned),
            model,
            tools: self.inner.tools.clone(),
            exclude_tools: self.inner.exclude_tools.clone(),
            ..SessionOptions::default()
        };
        let runtime = Arc::new(
            AgentSessionRuntime::create_for_hub(Arc::clone(&services), options)
                .await
                .map_err(|error| ClientError::Command(error.to_string()))?,
        );
        let session_id = runtime.session().session_id();
        tracing::info!(session_id = %session_id, cwd = %cwd.display(), "hub created a session");
        let live = live_entry(&runtime, (cwd, self.inner.agent_dir.clone()));
        state.crashed.remove(&session_id);
        state.locked.remove(&session_id);
        state.live.insert(session_id, Arc::clone(&live));
        drop(state);
        tokio::spawn(watch(Arc::downgrade(&self.inner), Arc::clone(&live)));
        Ok(lazy_backend(&live, Arc::downgrade(&self.inner)))
    }

    /// Attaches to `session_id` by exact id, waking the record if needed (E-LAZY "a record
    /// becomes live on open/attach").
    ///
    /// # Errors
    ///
    /// As [`SessionHub::open`].
    pub async fn attach(&self, session_id: &str) -> Result<Arc<dyn SessionBackend>, ClientError> {
        self.wake(session_id).await
    }

    /// Every session record, newest modified first, merged with live state — no wake, no
    /// services, no leases (the lazy listing).
    ///
    /// # Errors
    ///
    /// Listing I/O failures.
    pub async fn list(&self) -> Result<Vec<HubSession>, ClientError> {
        // Card index (bounded reads, cached by path + size + mtime), on the blocking pool:
        // a cold listing of a big store never stalls the async workers.
        let root = self.inner.sessions_root.clone();
        let records = tokio::task::spawn_blocking(move || SessionManager::list_all_cards(&root))
            .await
            .map_err(|error| ClientError::Command(format!("listing sessions: {error}")))?;
        // Never wait for the hub lock: a wake holds it through services creation (cold
        // extension loading, model probes) and the whole-file parse - seconds - and the
        // app's first `sessions_list` races the restored chat's open at every start
        // (sidebar-data: the Task list showed rows ~12 s late behind a 13.8 s wake).
        // While the lock is busy the live-id mirror answers `Live`/`Record` (crashed and
        // locked marks read as `Record` until the next listing; the embedding host keeps
        // its own crash/takeover marks).
        let Ok(mut state) = self.inner.state.try_lock() else {
            let live = lock(&self.inner.live_ids).clone();
            return Ok(records
                .into_iter()
                .map(|info| HubSession {
                    state: if live.contains(&info.id) {
                        HubSessionState::Live
                    } else {
                        HubSessionState::Record
                    },
                    info,
                })
                .collect());
        };
        // A locked marker only stands while the holder's lease metadata does.
        state
            .locked
            .retain(|_, (path, _)| lock_metadata_path(path).exists());
        Ok(records
            .into_iter()
            .map(|info| HubSession {
                state: if state.live.contains_key(&info.id) {
                    if state.crashed.contains(&info.id) {
                        HubSessionState::Crashed
                    } else {
                        HubSessionState::Live
                    }
                } else if state.crashed.contains(&info.id) {
                    HubSessionState::Crashed
                } else if state.locked.contains_key(&info.id) {
                    HubSessionState::Locked
                } else {
                    HubSessionState::Record
                },
                info,
            })
            .collect())
    }

    /// One record by session file path, exact id or unique id prefix (≥ 4 chars), merged
    /// with live state — from file names and the card index, never a content scan, so an
    /// unknown id answers at once.
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target (the [`SessionHub::open`] texts).
    pub async fn find(&self, target: &str) -> Result<HubSession, ClientError> {
        let info = find_record(&self.inner.sessions_root, target).await?;
        let state = self.inner.state.lock().await;
        let session_state = record_state(&state, &info);
        Ok(HubSession {
            info,
            state: session_state,
        })
    }
}

/// A record's state under the hub lock.
fn record_state(state: &State, info: &SessionInfo) -> HubSessionState {
    if state.live.contains_key(&info.id) {
        if state.crashed.contains(&info.id) {
            HubSessionState::Crashed
        } else {
            HubSessionState::Live
        }
    } else if state.crashed.contains(&info.id) {
        HubSessionState::Crashed
    } else if state
        .locked
        .get(&info.id)
        .is_some_and(|(path, _)| lock_metadata_path(path).exists())
    {
        HubSessionState::Locked
    } else {
        HubSessionState::Record
    }
}

impl SessionHub {
    /// Closes `session_id` if it is live (`session_shutdown{reason:"quit"}`, lease
    /// released); idempotent — closing a record or unknown id is `Ok`.
    ///
    /// # Errors
    ///
    /// Never today; `Result` keeps room for disposal failures.
    pub async fn close(&self, session_id: &str) -> Result<(), ClientError> {
        let live = self.inner.state.lock().await.live.get(session_id).cloned();
        if let Some(live) = live {
            live.admission.store(RETIRING, Ordering::Release);
            live.runtime.dispose().await;
            let mut state = self.inner.state.lock().await;
            if state
                .live
                .get(session_id)
                .is_some_and(|entry| Arc::ptr_eq(entry, &live))
            {
                state.live.remove(session_id);
            }
            live.admission.store(GONE, Ordering::Release);
            live.retired.notify_waiters();
        }
        Ok(())
    }

    /// The sessions root this hub lists and wakes from (its offset/card indexes live
    /// beside it).
    #[must_use]
    pub fn sessions_root(&self) -> &Path {
        &self.inner.sessions_root
    }

    /// Whether `session_id` is a live runtime of this hub (created or woken here).
    /// A session created by [`SessionHub::create_session`] is live before its first
    /// conversation entry writes the file — it has no record yet, so id-based lookups
    /// that go through [`SessionHub::list`] miss it (E-03).
    pub fn is_live(&self, session_id: &str) -> impl Future<Output = bool> {
        std::future::ready(lock(&self.inner.live_ids).contains(session_id))
    }

    /// Is a wake/close/eviction holding the hub lock right now? (A caller that must not
    /// wait — the host's file-first open — takes another path.)
    #[must_use]
    pub fn is_busy(&self) -> bool {
        self.inner.state.try_lock().is_err()
    }

    /// Lifecycle snapshot for worker supervision. Takes no guest calls.
    pub async fn busy_report(&self) -> BusyReport {
        let state = self.inner.state.lock().await;
        BusyReport {
            live_sessions: state.live.len(),
            services_sets: state.services.len(),
            busy_sessions: state
                .live
                .values()
                .filter(|live| {
                    live.admission.load(Ordering::Acquire) != 0
                        || session_busy(&live.runtime)
                        || live
                            .runtime
                            .session()
                            .ui()
                            .snapshot()
                            .iter()
                            .any(|request| oino_core::ui::is_dialog(&request.method))
                })
                .count(),
            keepalive_services: state
                .services
                .iter()
                .filter(|entry| services_background_work(&entry.services))
                .count(),
        }
    }

    /// True only after all session and services ownership has retired.
    pub async fn is_empty(&self) -> bool {
        self.busy_report().await.is_empty()
    }

    /// [`Self::find`] without waiting for the hub lock: while another session wakes (its
    /// whole file is parsed under the lock) the state comes from the live-id mirror —
    /// `Live` or `Record` (crashed/locked marks need the lock and read as `Record`).
    ///
    /// # Errors
    ///
    /// Unknown or ambiguous target, listing I/O failures.
    pub async fn find_quick(&self, target: &str) -> Result<HubSession, ClientError> {
        let info = find_record(&self.inner.sessions_root, target).await?;
        let state = match self.inner.state.try_lock() {
            Ok(state) => record_state(&state, &info),
            Err(_) if lock(&self.inner.live_ids).contains(&info.id) => HubSessionState::Live,
            Err(_) => HubSessionState::Record,
        };
        Ok(HubSession { info, state })
    }

    async fn wake(&self, target: &str) -> Result<Arc<dyn SessionBackend>, ClientError> {
        loop {
            let live = self.inner.state.lock().await.live.get(target).cloned();
            let Some(live) = live else { break };
            let notified = live.retired.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(_guard) = live.admit() {
                *lock(&live.last_active) = Instant::now();
                live.runtime
                    .services()
                    .refresh_project()
                    .await
                    .map_err(|e| ClientError::Command(e.to_string()))?;
                return Ok(lazy_backend(&live, Arc::downgrade(&self.inner)));
            }
            notified.await;
        }
        // Resolve the record outside the hub lock (file names + card index, off the async
        // workers): an unknown id fails fast and never holds up other opens or listings.
        let info = &find_record(&self.inner.sessions_root, target).await?;
        let session_cwd = self.record_cwd(info);
        let cwd = self.services_cwd(&session_cwd);
        self.refresh_cached_project(&cwd).await?;
        // The wake transition itself runs under the hub lock: opens, closes and evictions
        // serialise, so two opens of one record never race a lease against each other.
        let mut state = self.inner.state.lock().await;
        // Live under the resolved id (opened by prefix or path, or by a concurrent open).
        if state.live.contains_key(&info.id) {
            drop(state);
            return Box::pin(self.wake(&info.id)).await;
        }
        let services = self.services_for(&mut state, &cwd).await?;
        let options = SessionOptions {
            source: SessionSource::Open(info.path.clone()),
            cwd: (session_cwd != cwd).then_some(session_cwd),
            session_dir: Some(self.inner.sessions_root.clone()),
            tools: self.inner.tools.clone(),
            exclude_tools: self.inner.exclude_tools.clone(),
            custom_tools: self
                .inner
                .session_tools
                .as_ref()
                .map_or_else(Vec::new, |tools| (tools.0)(&info.path, &info.id)),
            ..SessionOptions::default()
        };
        let runtime =
            match AgentSessionRuntime::create_for_hub(Arc::clone(&services), options).await {
                Ok(runtime) => Arc::new(runtime),
                Err(SessionOpError::SessionLocked(text)) => {
                    state
                        .locked
                        .insert(info.id.clone(), (info.path.clone(), text.clone()));
                    tracing::warn!(session_id = %info.id, "{text}");
                    return Err(ClientError::Command(text));
                }
                Err(error) => return Err(ClientError::Command(error.to_string())),
            };
        let session_id = runtime.session().session_id();
        tracing::info!(session_id = %session_id, "hub woke a session (E-LAZY)");
        let key = (cwd, self.inner.agent_dir.clone());
        let live = live_entry(&runtime, key);
        state.crashed.remove(&session_id);
        state.locked.remove(&session_id);
        state.live.insert(session_id, Arc::clone(&live));
        drop(state);
        tokio::spawn(watch(Arc::downgrade(&self.inner), Arc::clone(&live)));
        Ok(lazy_backend(&live, Arc::downgrade(&self.inner)))
    }

    /// The services of the hub's primary cwd — cached and LRU-bounded like a session's —
    /// with **no session runtime**: the hub-level `_oino/*` subset (app spec §3.14,
    /// `host_cmd`, WP-A24) runs settings/auth/extension commands against it.
    ///
    /// # Errors
    ///
    /// Services creation failures.
    pub async fn services(&self) -> Result<Arc<Services>, ClientError> {
        let cwd = self.inner.cwd.clone();
        let mut state = self.inner.state.lock().await;
        let services = self.services_for(&mut state, &cwd).await?;
        drop(state);
        services
            .refresh_project()
            .await
            .map_err(|e| ClientError::Command(e.to_string()))?;
        Ok(services)
    }

    /// Warms (or returns) the cached services of `cwd`, with no session runtime — the
    /// embedding host's background pre-warm of its recent projects' cwds, so a
    /// `session_open` there answers without paying services creation (extension load,
    /// wasm instances) on the click path.
    ///
    /// # Errors
    ///
    /// Services creation failures.
    pub async fn warm_services(&self, cwd: &Path) -> Result<Arc<Services>, ClientError> {
        let cwd = self.services_cwd(cwd);
        let mut state = self.inner.state.lock().await;
        let services = self.services_for(&mut state, &cwd).await?;
        drop(state);
        services
            .refresh_project()
            .await
            .map_err(|e| ClientError::Command(e.to_string()))?;
        Ok(services)
    }

    /// Drops every cached services set: the next [`Self::services`] (or
    /// [`Self::warm_services`]) discovers the extensions again — the way a
    /// freshly installed (or removed) extension package is picked up without
    /// restarting the host.
    ///
    /// Live sessions keep the services they hold; only the hub's cache is
    /// dropped, so the next command builds a fresh set.
    pub async fn reset_services(&self) {
        let mut state = self.inner.state.lock().await;
        state.services.clear();
    }

    /// Every cached services set (one per project cwd, busy or idle-warm) — a hub-level
    /// global settings write reaches the other sets through these.
    pub async fn services_sets(&self) -> Vec<Arc<Services>> {
        let state = self.inner.state.lock().await;
        let mut services: Vec<_> = state
            .services
            .iter()
            .map(|entry| Arc::clone(&entry.services))
            .collect();
        // reset_services drops the cache, not services still held by live chats.
        // Global reload and trust changes must reach those older instances too.
        for live in state.live.values() {
            let held = live.runtime.services();
            if !services.iter().any(|entry| Arc::ptr_eq(entry, held)) {
                services.push(Arc::clone(held));
            }
        }
        services
    }

    /// The effective `maxServices` of this hub: the setting, or the [`HubLimits`] override.
    #[must_use]
    pub fn max_services(&self) -> usize {
        self.inner.config.max_services
    }

    // Never hold the hub map lock across an MCP network handshake. Existing project
    // instances can reload independently while unrelated chats list/open/dispatch.
    async fn refresh_cached_project(&self, cwd: &Path) -> Result<(), ClientError> {
        let services = {
            let state = self.inner.state.lock().await;
            state
                .services
                .iter()
                .find(|entry| entry.key.0 == cwd)
                .map(|entry| Arc::clone(&entry.services))
        };
        if let Some(services) = services {
            services
                .refresh_project()
                .await
                .map_err(|e| ClientError::Command(e.to_string()))?;
        }
        Ok(())
    }

    /// The services of `cwd`: cached, LRU-bounded (`maxServices`) and reset to "in use".
    /// Called with the hub lock held (the caller owns it through the whole wake).
    #[allow(
        clippy::too_many_lines,
        reason = "services LRU selection and atomic retirement are one transition"
    )]
    async fn services_for(
        &self,
        state: &mut tokio::sync::MutexGuard<'_, State>,
        cwd: &Path,
    ) -> Result<Arc<Services>, ClientError> {
        let key: ServicesKey = (cwd.to_path_buf(), self.inner.agent_dir.clone());
        if let Some(entry) = state.services.iter_mut().find(|entry| entry.key == key) {
            entry.last_used = Instant::now();
            entry.idle_since = None;
            return Ok(Arc::clone(&entry.services));
        }
        'pressure: while state.services.len() >= self.inner.config.max_services {
            // owner-fix-6 P1-3: a session woken (or used) moments ago has no stream
            // attached yet and read as idle, so the next services request (a hub-level
            // command 11 ms later) retired the chat the user had just opened. Victims go
            // in tiers: sets without live sessions, then idle sets whose sessions are past
            // the grace; at the cap the cache grows by one instead; only past it does an
            // idle set inside the grace go (least recently used: never the newest wake).
            let now = Instant::now();
            let pick = |filter: &dyn Fn(&ServicesEntry) -> bool| {
                state
                    .services
                    .iter()
                    .enumerate()
                    .filter(|(_, entry)| filter(entry) && !services_keepalive_work(&entry.services))
                    .min_by_key(|(_, entry)| entry.last_used)
                    .map(|(index, _)| index)
            };
            let victim = pick(&|entry| !state.live.values().any(|live| live.services == entry.key))
                .or_else(|| {
                    pick(&|entry| {
                        services_set_idle(state, &entry.key)
                            && services_set_rested(state, &entry.key, now)
                    })
                })
                .or_else(|| {
                    (state.services.len() > self.inner.config.max_services)
                        .then(|| pick(&|entry| services_set_idle(state, &entry.key)))
                        .flatten()
                });
            if let Some(index) = victim {
                let key = state.services[index].key.clone();
                let drained: Vec<Arc<Live>> = state
                    .live
                    .values()
                    .filter(|live| live.services == key)
                    .cloned()
                    .collect();
                let mut reserved: Vec<Arc<Live>> = Vec::new();
                for live in drained {
                    if live
                        .admission
                        .compare_exchange(0, RETIRING, Ordering::AcqRel, Ordering::Acquire)
                        .is_err()
                    {
                        for held in reserved {
                            held.admission.store(0, Ordering::Release);
                            held.retired.notify_waiters();
                        }
                        // A command raced selection. Preserve it and admit the
                        // new project using temporary overflow, not an internal error.
                        break 'pressure;
                    }
                    reserved.push(live);
                }
                if reserved.iter().any(|live| {
                    session_working(&live.runtime)
                        || live
                            .runtime
                            .session()
                            .ui()
                            .snapshot()
                            .iter()
                            .any(|request| oino_core::ui::is_dialog(&request.method))
                }) {
                    for held in reserved {
                        held.admission.store(0, Ordering::Release);
                        held.retired.notify_waiters();
                    }
                    break 'pressure;
                }
                let entry = state.services.remove(index);
                tracing::info!(cwd = %entry.key.0.display(), "hub hibernating idle services under pressure");
                let inner = Arc::clone(&self.inner);
                // The caller still owns the map lock. Shutdown runs in its own task;
                // retiring entries stay visible until their leases are released.
                tokio::spawn(async move {
                    for live in reserved {
                        let id = lock(&live.id).clone();
                        live.runtime.evict().await;
                        let mut state = inner.state.lock().await;
                        if state
                            .live
                            .get(&id)
                            .is_some_and(|entry| Arc::ptr_eq(entry, &live))
                        {
                            state.live.remove(&id);
                        }
                        drop(state);
                        live.admission.store(GONE, Ordering::Release);
                        live.retired.notify_waiters();
                    }
                    dispose_services(&entry.services).await;
                });
            } else {
                // maxServices is a cache target, not a limit on user work. Never
                // evict running turns or reject a chat just because all sets are
                // busy. The next allocation reclaims idle LRU sets first.
                tracing::warn!(
                    cwd = %cwd.display(),
                    sets = state.services.len(),
                    cap = self.inner.config.max_services,
                    "no idle services victim; temporarily exceeding maxServices"
                );
                break;
            }
        }
        let services = Services::create(ServicesOptions {
            cwd: key.0.clone(),
            agent_dir: key.1.clone(),
            docs_root: self.inner.docs_root.clone(),
            mode: self.inner.mode,
            has_ui: self.inner.has_ui,
            flags: self.inner.flags.clone(),
            extension_flag_values: indexmap::IndexMap::new(),
            loader: Arc::clone(&self.inner.loader),
            approve: self.inner.approve,
            models: self.inner.models.clone(),
            settings: None,
        })
        .await
        .map_err(|error| {
            ClientError::Command(format!("services for {}: {error}", key.0.display()))
        })?;
        state.services.push(ServicesEntry {
            key: key.clone(),
            services: Arc::clone(&services),
            last_used: Instant::now(),
            idle_since: None,
        });
        Ok(services)
    }

    /// The services cwd of a record: its header cwd when absolute, else the hub's primary.
    /// The services cwd of a session cwd: the shared root for its direct children
    /// ([`HubLimits::with_shared_services_root`]), else the cwd itself.
    fn services_cwd(&self, session_cwd: &Path) -> PathBuf {
        services_key(
            session_cwd,
            self.inner.shared_services_root.as_deref(),
            &self.inner.cwd,
        )
    }

    fn record_cwd(&self, info: &SessionInfo) -> PathBuf {
        let cwd = Path::new(&info.cwd);
        if info.cwd.is_empty() || !cwd.is_absolute() {
            self.inner.cwd.clone()
        } else {
            cwd.to_path_buf()
        }
    }
}

/// One live session's entry: the runtime plus the bookkeeping the watcher and idle monitor
/// update, with the hub variant of the holder side of D-TAKEOVER installed (close our side,
/// no fresh replacement session).
fn live_entry(runtime: &Arc<AgentSessionRuntime>, services: ServicesKey) -> Arc<Live> {
    let live = Arc::new(Live {
        runtime: Arc::clone(runtime),
        backend: Arc::new(InProcessBackend::new(Arc::clone(runtime))),
        services,
        id: Arc::new(Mutex::new(runtime.session().session_id())),
        last_active: Mutex::new(Instant::now()),
        admission: AtomicUsize::new(0),
        retired: tokio::sync::Notify::new(),
    });
    let weak_runtime = Arc::downgrade(runtime);
    runtime.set_takeover_handler(Some(Arc::new(move |by| {
        let weak_runtime = weak_runtime.clone();
        Box::pin(async move {
            if let Some(runtime) = weak_runtime.upgrade() {
                runtime.evict_taken_over(by).await;
            }
        })
    })));
    live
}

/// The backend of one woken session: an [`InProcessBackend`] wrapped with idle-clock
/// touches and transparent post-eviction reopens.
fn lazy_backend(live: &Arc<Live>, inner: Weak<Inner>) -> Arc<dyn SessionBackend> {
    let id = Arc::clone(&live.id);
    Arc::new(LazyBackend {
        live: Mutex::new(Arc::downgrade(live)),
        inner,
        id,
    })
}

// Low bits count admitted operations; retirement can commit only from zero.
const RETIRING: usize = 1 << (usize::BITS - 1);
const GONE: usize = RETIRING | 1;

struct Admission(Arc<Live>);

impl Drop for Admission {
    fn drop(&mut self) {
        let _ = self
            .0
            .admission
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state > 0 && state < RETIRING).then(|| state - 1)
            });
    }
}

impl Live {
    fn admit(self: &Arc<Self>) -> Option<Admission> {
        self.admission
            .try_update(Ordering::AcqRel, Ordering::Acquire, |state| {
                (state < RETIRING).then_some(state + 1)
            })
            .ok()
            .map(|_| Admission(Arc::clone(self)))
    }
}

async fn log_memory_lifecycle(inner: &Inner) {
    let state = inner.state.lock().await;
    tracing::info!(
        live_sessions = state.live.len(),
        services = state.services.len(),
        "memory lifecycle"
    );
    for live in state.live.values() {
        let session = live.runtime.session();
        tracing::info!(session_id = %lock(&live.id), run_state = ?session.run_state(),
                pending = session.pending_message_count(), subscribers = session.hub().subscriber_count(),
                agents = ?session.agent_tree(), admission = live.admission.load(Ordering::Acquire),
                idle_secs = lock(&live.last_active).elapsed().as_secs(), "memory session");
    }
    for entry in &state.services {
        let background: Vec<_> = entry
            .services
            .extensions
            .extensions()
            .iter()
            .filter(|extension| extension.instance.has_keepalive_work())
            .map(|extension| extension.instance.id().to_owned())
            .collect();
        tracing::info!(cwd = %entry.key.0.display(), ?background, "memory services");
    }
}

/// Select under the map lock, but never run guest shutdown under that lock.
async fn tick(inner: &Inner) {
    let config = inner.config;
    if std::env::var_os("OINO_MEMORY_DIAGNOSTICS").is_some() {
        log_memory_lifecycle(inner).await;
    }
    let victims: Vec<Arc<Live>> = inner.state.lock().await.live.values().cloned().collect();
    if let Some(idle) = config.session_idle {
        for live in victims {
            if live
                .admission
                .compare_exchange(0, RETIRING, Ordering::AcqRel, Ordering::Acquire)
                .is_err()
            {
                continue;
            }
            let now = Instant::now();
            let busy = session_busy(&live.runtime)
                || live
                    .runtime
                    .session()
                    .ui()
                    .snapshot()
                    .iter()
                    .any(|request| oino_core::ui::is_dialog(&request.method));
            if busy || now.duration_since(*lock(&live.last_active)) < idle {
                if busy {
                    *lock(&live.last_active) = now;
                }
                live.admission.store(0, Ordering::Release);
                live.retired.notify_waiters();
                continue;
            }
            // Teardown publishes events; the watcher may treat those as activity.
            // Retirement itself must not restart the services idle clock.
            let last_active = *lock(&live.last_active);
            let id = lock(&live.id).clone();
            tracing::info!(session_id = %id, "hub evicted an idle session (E-LAZY)");
            live.runtime.evict().await;
            let mut state = inner.state.lock().await;
            if state
                .live
                .get(&id)
                .is_some_and(|entry| Arc::ptr_eq(entry, &live))
            {
                state.live.remove(&id);
                if let Some(entry) = state
                    .services
                    .iter_mut()
                    .find(|entry| entry.key == live.services)
                {
                    entry.last_used = entry.last_used.max(last_active);
                }
            }
            live.admission.store(GONE, Ordering::Release);
            live.retired.notify_waiters();
        }
    }
    retire_services(inner, config).await;
}

async fn retire_services(inner: &Inner, config: HubConfig) {
    let disposed = {
        let mut state = inner.state.lock().await;
        let now = Instant::now();
        let live_keys: HashSet<ServicesKey> = state
            .live
            .values()
            .map(|live| live.services.clone())
            .collect();
        let last_active = state
            .live
            .values()
            .fold(HashMap::new(), |mut clocks, live| {
                let active = *lock(&live.last_active);
                clocks
                    .entry(live.services.clone())
                    .and_modify(|clock| *clock = std::cmp::max(*clock, active))
                    .or_insert(active);
                clocks
            });
        let mut disposed = Vec::new();
        if let Some(idle) = config.services_idle {
            let mut index = 0;
            while index < state.services.len() {
                let entry = &mut state.services[index];
                if live_keys.contains(&entry.key) {
                    entry.last_used = last_active[&entry.key];
                    entry.idle_since = None;
                    index += 1;
                    continue;
                }
                // Share the session inactivity window: do not add another five
                // minutes after the last idle session has already been retired.
                let since = *entry.idle_since.get_or_insert(entry.last_used);
                if now.duration_since(since) >= idle && services_background_work(&entry.services) {
                    index += 1;
                    continue;
                }
                if now.duration_since(since) >= idle {
                    disposed.push(state.services.remove(index));
                } else {
                    index += 1;
                }
            }
        }
        disposed
    };
    for entry in disposed {
        tracing::info!(cwd = %entry.key.0.display(), "hub disposed idle services (E-LAZY)");
        dispose_services(&entry.services).await;
    }
}

/// Whether a session must stay live (E-LAZY conditions): a run, compaction, retry/arbiter
/// wait or open `!` command, a live (non-hibernated) child, or any external subscriber
/// (the runtime's own forward task is the single allowed one). An idle child stops pinning
/// its parent once it hibernates (perf-fix #4 made that happen; perf-baseline #5).
fn session_busy(runtime: &Arc<AgentSessionRuntime>) -> bool {
    session_working(runtime) || runtime.session().hub().subscriber_count() > 1
}

// An attached idle view is not executing work. Under cache pressure it may
// hibernate (SessionEvicted tells the client to wake on demand).
fn session_working(runtime: &Arc<AgentSessionRuntime>) -> bool {
    let session = runtime.session();
    session.run_state() != oino_core::events::RunState::Idle
        || session.is_bash_running()
        || session.pending_message_count() > 0
        || session
            .agent_tree()
            .iter()
            .any(|agent| matches!(agent.state, AgentRunState::Running | AgentRunState::Idle))
}

/// Whether every live session running on `key`'s services is at rest: no run in flight,
/// no pending work and no open dialog. An idle attached stream may hibernate under
/// pressure; the normal idle timer still preserves attached views. Such a set is a safe `maxServices` eviction
/// victim — its sessions return to records, exactly like the idle monitor's eviction.
fn services_set_idle(state: &State, key: &ServicesKey) -> bool {
    state
        .live
        .values()
        .filter(|live| &live.services == key)
        .all(|live| {
            live.admission.load(Ordering::Acquire) == 0 && !session_working(&live.runtime) && {
                let session = live.runtime.session();
                !session
                    .ui()
                    .snapshot()
                    .iter()
                    .any(|request| oino_core::ui::is_dialog(&request.method))
            }
        })
}

/// How long a woken or used session is never a `maxServices` victim while the cache may
/// still grow by one (owner-fix-6 P1-3: the open's stream attaches moments after the wake).
const EVICT_GRACE: Duration = Duration::from_secs(30);

/// Whether every live session on `key`'s services was last active at least
/// [`EVICT_GRACE`] ago (woken, a command, an event).
fn services_set_rested(state: &State, key: &ServicesKey, now: Instant) -> bool {
    state
        .live
        .values()
        .filter(|live| &live.services == key)
        .all(|live| now.duration_since(*lock(&live.last_active)) >= EVICT_GRACE)
}

/// Pressure may hibernate timers, but never executing extension work.
fn services_keepalive_work(services: &Services) -> bool {
    services
        .extensions
        .extensions()
        .iter()
        .any(|extension| extension.instance.has_keepalive_work())
}

/// Preserve timer keepalive for normal idle expiry unless workers are enabled.
fn services_background_work(services: &Services) -> bool {
    services.extensions.extensions().iter().any(|extension| {
        if std::env::var("OINO_WORKERS").as_deref() == Ok("1") {
            extension.instance.has_keepalive_work()
        } else {
            extension.instance.has_background_work()
        }
    })
}

/// Shuts a disposed services set's extensions down (E-LAZY: "extension shutdown, instances
/// dropped").
async fn dispose_services(services: &Arc<Services>) {
    for extension in services.extensions.extensions() {
        extension.instance.shutdown("idle").await;
    }
}

/// The text of an unknown session target (hosts map it to "not found").
#[must_use]
pub fn not_found_text(target: &str) -> String {
    format!("No session found matching '{target}'")
}

/// Resolves `target` under `sessions_root`: a session file path, an exact id, or a unique
/// id prefix of at least 4 characters (the `find_by_id` rules) — from file names and the
/// card index ([`SessionManager::find_card`]), on the blocking pool.
async fn find_record(sessions_root: &Path, target: &str) -> Result<SessionInfo, ClientError> {
    let root = sessions_root.to_path_buf();
    let owned = target.to_owned();
    let found = tokio::task::spawn_blocking(move || SessionManager::find_card(&root, &owned))
        .await
        .map_err(|error| ClientError::Command(format!("finding a session: {error}")))?;
    match found {
        Ok(Some(info)) => Ok(info),
        Ok(None) if Path::new(target).exists() => Err(ClientError::Command(format!(
            "Not an Oino session in the hub: {target}"
        ))),
        Ok(None) => Err(ClientError::Command(not_found_text(target))),
        Err(oino_session::SessionError::AmbiguousId(paths)) => Err(ClientError::Command(format!(
            "Ambiguous session id {target}: it matches {} sessions",
            paths.len()
        ))),
        Err(error) => Err(ClientError::Command(error.to_string())),
    }
}

/// Whether a client event counts as session activity for the idle clock. Fire-and-forget
/// UI updates (status, widget, title, notify; registry patches) do not: an extension
/// refreshing its footer on a timer kept an idle session from ever being evicted (perf-fix:
/// the 200 MB bench chat stayed resident forever on the watchdog's 15 s status).
fn is_activity(event: &ClientEvent) -> bool {
    match event {
        ClientEvent::Ui(request) => oino_core::ui::is_dialog(&request.method),
        ClientEvent::Oino(OinoEvent::UiPatch { .. }) => false,
        _ => true,
    }
}

/// Follows one live session's client events (on the runtime stream, which survives session
/// replacements): refreshes its activity clock, re-keys it on replacement, records
/// contained crashes (X-PANIC), and removes it when the stream ends (dispose/evict/
/// takeover all close the runtime hub).
async fn watch(inner: Weak<Inner>, live: Arc<Live>) {
    let mut stream = live.runtime.subscribe();
    while let Some(event) = stream.next().await {
        if is_activity(&event) {
            *lock(&live.last_active) = Instant::now();
        }
        match &event {
            ClientEvent::Oino(OinoEvent::SessionReplaced { session_id, .. }) => {
                if let Some(inner) = inner.upgrade() {
                    rekey(&inner, &live, session_id.clone()).await;
                }
            }
            ClientEvent::Oino(OinoEvent::SessionCrashed { .. }) => {
                if let Some(inner) = inner.upgrade() {
                    let id = lock(&live.id).clone();
                    inner.state.lock().await.crashed.insert(id);
                }
            }
            _ => {}
        }
    }
    // Stream end: the runtime was disposed, evicted or taken over.
    if let Some(inner) = inner.upgrade() {
        let id = lock(&live.id).clone();
        let mut state = inner.state.lock().await;
        if state
            .live
            .get(&id)
            .is_some_and(|entry| Arc::ptr_eq(entry, &live))
        {
            state.live.remove(&id);
        }
    }
    live.admission.store(GONE, Ordering::Release);
    live.retired.notify_waiters();
}

/// Moves a live entry to its replacement session's id (new/fork/switch through a backend).
async fn rekey(inner: &Inner, live: &Arc<Live>, new_id: String) {
    let mut state = inner.state.lock().await;
    let old = std::mem::replace(&mut *lock(&live.id), new_id.clone());
    if state
        .live
        .get(&old)
        .is_some_and(|entry| Arc::ptr_eq(entry, live))
    {
        state.live.remove(&old);
        state.crashed.remove(&old);
        state.locked.remove(&old);
    }
    state.live.insert(new_id, Arc::clone(live));
}

/// The idle monitor: polls until the hub is dropped (both idles disabled ⇒ never started).
async fn monitor(inner: Weak<Inner>, poll: Duration) {
    loop {
        tokio::time::sleep(poll).await;
        let Some(hub) = inner.upgrade() else {
            return;
        };
        tick(&hub).await;
    }
}

/// A hub backend: the [`InProcessBackend`] of a woken session, with every command, event
/// subscription or dialog answer refreshing the session's idle clock. Retained handles
/// hold only a weak entry and reopen after idle retirement.
struct LazyBackend {
    live: Mutex<Weak<Live>>,
    inner: Weak<Inner>,
    id: Arc<Mutex<String>>,
}

impl LazyBackend {
    fn admitted(&self) -> Option<(Admission, Arc<InProcessBackend>)> {
        let live = lock(&self.live).upgrade()?;
        let guard = live.admit()?;
        let backend = Arc::clone(&live.backend);
        Some((guard, backend))
    }

    async fn reopen(&self) -> Result<Arc<dyn SessionBackend>, ClientError> {
        let inner = self.inner.upgrade().ok_or(ClientError::Closed)?;
        let hub = SessionHub { inner };
        let id = lock(&self.id).clone();
        let backend = hub.open(&id).await?;
        if let Some(live) = hub.inner.state.lock().await.live.get(&backend.session_id()) {
            *lock(&self.live) = Arc::downgrade(live);
        }
        Ok(backend)
    }
}

impl SessionBackend for LazyBackend {
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        self.send_with_id(None, command)
    }

    fn send_with_id(
        &self,
        request_id: Option<String>,
        command: RpcCommand,
    ) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        Box::pin(async move {
            if let Some((guard, backend)) = self.admitted() {
                if !matches!(
                    command,
                    RpcCommand::Agent(AgentCommand::GetState | AgentCommand::GetSessionStats)
                ) {
                    *lock(&guard.0.last_active) = Instant::now();
                }
                backend.send_with_id(request_id, command).await
            } else {
                self.reopen().await?.send_with_id(request_id, command).await
            }
        })
    }

    fn restore_durable_queue(&self) -> BoxFuture<'_, Result<(), ClientError>> {
        Box::pin(async move {
            if let Some((_guard, backend)) = self.admitted() {
                backend.restore_durable_queue().await
            } else {
                self.reopen().await?.restore_durable_queue().await
            }
        })
    }

    fn runtime_identity(&self) -> Option<usize> {
        lock(&self.live)
            .upgrade()
            .map(|live| Arc::as_ptr(&live.runtime) as usize)
    }

    fn events(&self) -> BoxStream<'static, ClientEvent> {
        if let Some((guard, backend)) = self.admitted() {
            *lock(&guard.0.last_active) = Instant::now();
            let stream = backend.events();
            // Admission protects creating the subscription, not its lifetime.
            // The subscription pins normal idle eviction via subscriber_count;
            // pressure eviction can close an idle stream and re-wake on demand.
            drop(guard);
            return stream;
        }
        let inner = self.inner.clone();
        let id = lock(&self.id).clone();
        futures::stream::once(async move {
            if let Some(inner) = inner.upgrade()
                && let Ok(backend) = (SessionHub { inner }).open(&id).await
            {
                return backend.events();
            }
            futures::stream::empty().boxed()
        })
        .flatten()
        .boxed()
    }

    fn ui_respond(&self, response: UiResponse) {
        if let Some((guard, backend)) = self.admitted() {
            *lock(&guard.0.last_active) = Instant::now();
            backend.ui_respond(response);
        }
    }

    fn session_id(&self) -> String {
        lock(&self.live)
            .upgrade()
            .map_or_else(|| lock(&self.id).clone(), |live| lock(&live.id).clone())
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        Box::pin(async move {
            if let Some((_guard, backend)) = self.admitted() {
                backend.shutdown().await;
            }
        })
    }

    fn shutdown_requested(&self) -> BoxFuture<'static, ()> {
        let Some(live) = lock(&self.live).upgrade() else {
            return Box::pin(futures::future::pending());
        };
        let mut requested = live.runtime.shutdown_requested();
        let weak = Arc::downgrade(&live);
        Box::pin(async move {
            if requested.wait_for(|on| *on).await.is_err() {
                return futures::future::pending().await;
            }
            if let Some(live) = weak.upgrade() {
                live.runtime.session().wait_for_idle().await;
            }
        })
    }
}
