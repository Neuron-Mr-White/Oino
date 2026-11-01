//! E-LAZY lazy session hub (core spec §2.2, WP-C7): a session is a record on disk until a
//! message arrives (infinity's "spawn driver on demand" as a core type).
//!
//! [`SessionHub`] lists session records with no services, engines or leases; `open`/`attach`
//! wake one (acquire lease → repair → `AgentSessionRuntime` → [`InProcessBackend`]); a live
//! session with no run, compaction, arbiter wait, open tools, live children or subscribers
//! for `extensionSettings.hub.sessionIdleSec` (default 900 s) is evicted — disposed
//! (`session_shutdown{reason:"idle"}`, lease released, guests drop their per-session state)
//! and returns to a record that re-opens and repairs on demand. A services set with zero
//! live sessions for `servicesIdleSec` (default 900 s) is disposed (extension `shutdown`);
//! at most `maxServices` (default 4) are kept, least-recently-used first — an idle set is
//! evicted (its idle sessions return to records) to make room, and when every set has a
//! busy session the hub rather exceeds the cap by one than refuses the open.
//!
//! Settings (`extensionSettings.hub.*`, §2.2): `sessionIdleSec`/`servicesIdleSec` `0`
//! disables eviction — the TUI's single-session runtime does not use the hub at all.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::{JsonObject, JsonValue};
use oino_core::agents::AgentRunState;
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource, ToolSelection};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::session::SessionOpError;
use oino_core::ui::UiResponse;
use oino_models::ModelRegistry;
use oino_session::{SessionInfo, SessionManager, lock_metadata_path};
use oino_settings::SettingsManager;

use crate::backend::{ClientError, SessionBackend};
use crate::in_process::InProcessBackend;
use crate::protocol::{RpcCommand, RpcResponseData};

/// `extensionSettings.hub.sessionIdleSec` default (ruling, core spec §7).
pub const DEFAULT_SESSION_IDLE_SECS: u64 = 900;
/// `extensionSettings.hub.servicesIdleSec` default (core spec §2.2).
pub const DEFAULT_SERVICES_IDLE_SECS: u64 = 900;
/// `extensionSettings.hub.maxServices` default (core spec §2.2).
pub const DEFAULT_MAX_SERVICES: usize = 4;

/// Lower bound of the idle monitor's poll interval (tests use sub-second idles).
const MIN_POLL: Duration = Duration::from_millis(100);
/// Upper bound of the idle monitor's poll interval.
const MAX_POLL: Duration = Duration::from_secs(2);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
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
#[derive(Clone, Copy, Debug, Default)]
#[non_exhaustive]
pub struct HubLimits {
    /// Overrides `extensionSettings.hub.maxServices` (`None` keeps the setting, default 4).
    /// Every cached services set holds one wasm instance per extension — preopen dirs and
    /// log descriptors — so a host with a low `RLIMIT_NOFILE` cannot afford the default.
    max_services: Option<usize>,
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
}

/// `extensionSettings.hub.*` (§2.2): idle windows (`None` = disabled) and the services LRU
/// cap. Defaults 900/900/4 (owner ruling).
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
    services: ServicesKey,
    /// Current session id (re-keyed on `session_replaced`).
    id: Mutex<String>,
    /// Last activity: any client event, command or busy predicate.
    last_active: Mutex<Instant>,
    /// Set when the runtime's streams ended (dispose/evict/takeover).
    gone: AtomicBool,
}

/// Hub state guarded by one async mutex: open/close/evict transitions are serialised, so a
/// wake never races an eviction of the same session.
struct State {
    services: Vec<ServicesEntry>,
    live: HashMap<String, Arc<Live>>,
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
        let inner = Arc::new(Inner {
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
            state: tokio::sync::Mutex::new(State {
                services: Vec::new(),
                live: HashMap::new(),
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
        let mut state = self.inner.state.lock().await;
        let cwd = cwd.map_or_else(|| self.inner.cwd.clone(), Path::to_path_buf);
        let services = self.services_for(&mut state, &cwd).await?;
        let options = SessionOptions {
            source: SessionSource::New,
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
        Ok(lazy_backend(live))
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
        let records = SessionManager::list_all(&self.inner.sessions_root, None, None)
            .map_err(|error| ClientError::Command(error.to_string()))?;
        let mut state = self.inner.state.lock().await;
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

    /// Closes `session_id` if it is live (`session_shutdown{reason:"quit"}`, lease
    /// released); idempotent — closing a record or unknown id is `Ok`.
    ///
    /// # Errors
    ///
    /// Never today; `Result` keeps room for disposal failures.
    pub async fn close(&self, session_id: &str) -> Result<(), ClientError> {
        let live = self.inner.state.lock().await.live.remove(session_id);
        if let Some(live) = live {
            live.gone.store(true, Ordering::Release);
            live.runtime.dispose().await;
        }
        Ok(())
    }

    async fn wake(&self, target: &str) -> Result<Arc<dyn SessionBackend>, ClientError> {
        // The whole wake transition runs under the hub lock: opens, closes and evictions
        // serialise, so two opens of one record never race a lease against each other.
        let mut state = self.inner.state.lock().await;
        // Live by exact id: hand out a fresh backend over the shared runtime.
        if let Some(live) = state.live.get(target) {
            let live = Arc::clone(live);
            drop(state);
            *lock(&live.last_active) = Instant::now();
            return Ok(lazy_backend(live));
        }
        let records = SessionManager::list_all(&self.inner.sessions_root, None, None)
            .map_err(|error| ClientError::Command(error.to_string()))?;
        let info = resolve_target(target, &records)?;
        // Live under the resolved id (opened by prefix or path).
        if let Some(live) = state.live.get(&info.id) {
            let live = Arc::clone(live);
            drop(state);
            *lock(&live.last_active) = Instant::now();
            return Ok(lazy_backend(live));
        }
        let cwd = self.record_cwd(info);
        let services = self.services_for(&mut state, &cwd).await?;
        let options = SessionOptions {
            source: SessionSource::Open(info.path.clone()),
            session_dir: Some(self.inner.sessions_root.clone()),
            tools: self.inner.tools.clone(),
            exclude_tools: self.inner.exclude_tools.clone(),
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
        Ok(lazy_backend(live))
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
        self.services_for(&mut state, &cwd).await
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
        let mut state = self.inner.state.lock().await;
        self.services_for(&mut state, cwd).await
    }

    /// The effective `maxServices` of this hub: the setting, or the [`HubLimits`] override.
    #[must_use]
    pub fn max_services(&self) -> usize {
        self.inner.config.max_services
    }

    /// The services of `cwd`: cached, LRU-bounded (`maxServices`) and reset to "in use".
    /// Called with the hub lock held (the caller owns it through the whole wake).
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
        if state.services.len() >= self.inner.config.max_services {
            let victim = state
                .services
                .iter()
                .enumerate()
                .filter(|(_, entry)| services_set_idle(state, &entry.key))
                .min_by_key(|(_, entry)| entry.last_used)
                .map(|(index, _)| index);
            match victim {
                Some(index) => {
                    let entry = state.services.remove(index);
                    // Idle live sessions on the victim keep its `Services` alive inside
                    // their runtimes, so they go back to records first (exactly what the
                    // idle monitor's eviction does; the next command re-wakes them).
                    let drained: Vec<Arc<Live>> = state
                        .live
                        .values()
                        .filter(|live| live.services == entry.key)
                        .cloned()
                        .collect();
                    for live in drained {
                        let id = lock(&live.id).clone();
                        state.live.remove(&id);
                        live.gone.store(true, Ordering::Release);
                        live.runtime.dispose().await;
                        tracing::info!(
                            session_id = %id, cwd = %entry.key.0.display(),
                            "hub retired an idle session to free its services (maxServices)"
                        );
                    }
                    tracing::info!(cwd = %entry.key.0.display(), "hub evicted services (maxServices)");
                    dispose_services(&entry.services).await;
                }
                None if state.services.len() == self.inner.config.max_services => {
                    // Every cached set has a busy session (a running turn, an attached
                    // stream or an open dialog). Serving the user outranks the descriptor
                    // budget: grow one set past the cap rather than refusing the open —
                    // the next idle set trims back as soon as one frees up.
                    tracing::warn!(
                        cwd = %cwd.display(),
                        sets = state.services.len(),
                        cap = self.inner.config.max_services,
                        "all services sets are busy; exceeding maxServices by one"
                    );
                }
                None => {
                    return Err(ClientError::Command(format!(
                        "hub services cache is over budget ({} of max {}) and every set has a \
                         running turn; wait for one to finish or close a session",
                        state.services.len(),
                        self.inner.config.max_services
                    )));
                }
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
        services,
        id: Mutex::new(runtime.session().session_id()),
        last_active: Mutex::new(Instant::now()),
        gone: AtomicBool::new(false),
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
/// touches and post-eviction `Closed` failures.
fn lazy_backend(live: Arc<Live>) -> Arc<dyn SessionBackend> {
    let backend = Arc::new(InProcessBackend::new(Arc::clone(&live.runtime)));
    Arc::new(LazyBackend { live, backend })
}

/// One idle-monitor pass: evict idle live sessions, dispose idle services groups.
async fn tick(inner: &Inner) {
    let config = inner.config;
    let mut state = inner.state.lock().await;
    let now = Instant::now();
    if let Some(idle) = config.session_idle {
        let victims: Vec<Arc<Live>> = state
            .live
            .iter()
            .filter(|(_, live)| {
                if session_busy(&live.runtime) {
                    *lock(&live.last_active) = now;
                    return false;
                }
                now.duration_since(*lock(&live.last_active)) >= idle
            })
            .map(|(_, live)| Arc::clone(live))
            .collect();
        for live in victims {
            let id = lock(&live.id).clone();
            state.live.remove(&id);
            live.runtime.evict().await;
            // The record stays (re-listed from disk); a crash marker survives eviction.
        }
    }
    if config.services_idle.is_some() {
        let live_keys: HashSet<ServicesKey> = state
            .live
            .values()
            .map(|live| live.services.clone())
            .collect();
        let mut index = 0;
        while index < state.services.len() {
            let entry = &mut state.services[index];
            if live_keys.contains(&entry.key) {
                entry.last_used = now;
                entry.idle_since = None;
                index += 1;
                continue;
            }
            let since = *entry.idle_since.get_or_insert(now);
            if now.duration_since(since) >= config.services_idle.unwrap_or(Duration::MAX) {
                let entry = state.services.remove(index);
                tracing::info!(cwd = %entry.key.0.display(), "hub disposed idle services (E-LAZY)");
                dispose_services(&entry.services).await;
            } else {
                index += 1;
            }
        }
    }
}

/// Whether a session must stay live (E-LAZY conditions): a run, compaction, retry/arbiter
/// wait or open `!` command, a live (non-hibernated) child, or any external subscriber
/// (the runtime's own forward task is the single allowed one).
fn session_busy(runtime: &Arc<AgentSessionRuntime>) -> bool {
    let session = runtime.session();
    session.run_state() != oino_core::events::RunState::Idle
        || session.is_bash_running()
        || session
            .agent_tree()
            .iter()
            .any(|agent| matches!(agent.state, AgentRunState::Running | AgentRunState::Idle))
        || session.hub().subscriber_count() > 1
}

/// Whether every live session running on `key`'s services is at rest: no run in flight,
/// no attached stream and no open dialog. Such a set is a safe `maxServices` eviction
/// victim — its sessions return to records, exactly like the idle monitor's eviction.
fn services_set_idle(state: &State, key: &ServicesKey) -> bool {
    state
        .live
        .values()
        .filter(|live| &live.services == key)
        .all(|live| {
            !session_busy(&live.runtime) && {
                let session = live.runtime.session();
                !session
                    .ui()
                    .snapshot()
                    .iter()
                    .any(|request| oino_core::ui::is_dialog(&request.method))
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

/// Resolves `target` against the records: a session file path, an exact id, or a unique
/// id prefix of at least 4 characters (the `find_by_id` rules).
fn resolve_target<'a>(
    target: &str,
    records: &'a [SessionInfo],
) -> Result<&'a SessionInfo, ClientError> {
    let path = Path::new(target);
    if path.exists() {
        return records
            .iter()
            .find(|info| info.path == path)
            .ok_or_else(|| {
                ClientError::Command(format!("Not an Oino session in the hub: {target}"))
            });
    }
    let exact: Vec<&SessionInfo> = records.iter().filter(|info| info.id == target).collect();
    if exact.len() == 1 {
        return Ok(exact[0]);
    }
    if exact.len() > 1 {
        return Err(ClientError::Command(format!(
            "Ambiguous session id {target}: it matches {} sessions",
            exact.len()
        )));
    }
    if target.len() >= 4 {
        let prefixed: Vec<&SessionInfo> = records
            .iter()
            .filter(|info| info.id.starts_with(target))
            .collect();
        match prefixed.len() {
            1 => return Ok(prefixed[0]),
            0 => {}
            n => {
                return Err(ClientError::Command(format!(
                    "Ambiguous session id {target}: it matches {n} sessions"
                )));
            }
        }
    }
    Err(ClientError::Command(format!(
        "No session found matching '{target}'"
    )))
}

/// Follows one live session's client events (on the runtime stream, which survives session
/// replacements): refreshes its activity clock, re-keys it on replacement, records
/// contained crashes (X-PANIC), and removes it when the stream ends (dispose/evict/
/// takeover all close the runtime hub).
async fn watch(inner: Weak<Inner>, live: Arc<Live>) {
    let mut stream = live.runtime.subscribe();
    while let Some(event) = stream.next().await {
        *lock(&live.last_active) = Instant::now();
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
    live.gone.store(true, Ordering::Release);
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
/// subscription or dialog answer refreshing the session's idle clock ("a record becomes
/// live … on any inbound message"), and post-eviction commands failing
/// [`ClientError::Closed`] instead of reaching a disposed session.
struct LazyBackend {
    live: Arc<Live>,
    backend: Arc<InProcessBackend>,
}

impl LazyBackend {
    fn touch(&self) {
        *lock(&self.live.last_active) = Instant::now();
    }
}

impl SessionBackend for LazyBackend {
    fn send(&self, command: RpcCommand) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        self.touch();
        if self.live.gone.load(Ordering::Acquire) {
            return Box::pin(futures::future::ready(Err(ClientError::Closed)));
        }
        self.backend.send(command)
    }

    fn send_with_id(
        &self,
        request_id: Option<String>,
        command: RpcCommand,
    ) -> BoxFuture<'_, Result<RpcResponseData, ClientError>> {
        self.touch();
        if self.live.gone.load(Ordering::Acquire) {
            return Box::pin(futures::future::ready(Err(ClientError::Closed)));
        }
        self.backend.send_with_id(request_id, command)
    }

    fn events(&self) -> BoxStream<'static, ClientEvent> {
        self.touch();
        self.backend.events()
    }

    fn ui_respond(&self, response: UiResponse) {
        self.touch();
        self.backend.ui_respond(response);
    }

    fn session_id(&self) -> String {
        self.backend.session_id()
    }

    fn shutdown(&self) -> BoxFuture<'_, ()> {
        self.backend.shutdown()
    }

    fn shutdown_requested(&self) -> BoxFuture<'static, ()> {
        self.backend.shutdown_requested()
    }
}
