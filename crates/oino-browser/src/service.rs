//! [`BrowserService`]: the process-wide [`BrowserHost`] (browser-control.md §3): profiles,
//! launch, scope → tab resolution, residency, idle shutdown.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

use futures::stream::BoxStream;
use oino_browser_protocol::{
    BrowserResult, BrowserScope, BrowserStatus, BrowserViewEvent, FrontendRequest, ProfileInfo,
    TabOwner, UnavailableReason, Viewport,
};
use oino_extension_api::BoxFut;
use oino_extension_api::browser::{BrowserHost, BrowserRequest, ViewStream};
use tokio_util::sync::CancellationToken;

use crate::config::{BrowserSettings, DisplayChoice, TransportChoice, valid_profile_name};
use crate::display::{self, Displays};
use crate::engine::{self, EngineKind, EngineMissing};
use crate::hub::Hub;
use crate::launch::{BrowserProcess, LaunchSpec, Placement};
use crate::profile::ProfileRt;
use crate::tab::{Candidate, Tab, lock, residency_victim};

/// Overall browser launch budget (QA-2): engine start plus the CDP handshake. A wedged
/// launch becomes `launch_failed` (browser-control.md §5.5) instead of a tool call that
/// never ends.
const LAUNCH_TIMEOUT: Duration = Duration::from_secs(90);

/// Construction options.
#[derive(Clone, Debug)]
pub struct BrowserOptions {
    /// `~/.oino/browser` (profiles, shots, downloads, site permissions).
    pub root: PathBuf,
    /// Settings.
    pub settings: BrowserSettings,
    /// `--browser=ephemeral`: temporary profiles deleted at exit.
    pub ephemeral: bool,
}

impl BrowserOptions {
    /// Options for agent dir `agent_dir` (`~/.oino`), reading `settings.json`.
    #[must_use]
    pub fn for_agent_dir(agent_dir: &std::path::Path) -> Self {
        Self {
            root: agent_dir.join("browser"),
            settings: BrowserSettings::load(agent_dir),
            ephemeral: false,
        }
    }
}

pub(crate) struct Inner {
    pub(crate) settings: RwLock<BrowserSettings>,
    pub(crate) hub: Arc<Hub>,
    displays: Displays,
    launching: tokio::sync::Mutex<()>,
    pub(crate) live: Mutex<HashMap<String, Arc<ProfileRt>>>,
    generation: AtomicU64,
    reaper: AtomicBool,
    /// Keeps the `--browser=ephemeral` root alive (deleted on drop).
    _ephemeral: Option<tempfile::TempDir>,
    pub(crate) site_allow: Mutex<HashSet<(String, String)>>,
    pub(crate) views: Mutex<HashMap<u64, (String, String)>>,
}

/// The browser service. Cheap to clone.
#[derive(Clone)]
pub struct BrowserService {
    pub(crate) inner: Arc<Inner>,
}

impl std::fmt::Debug for BrowserService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserService").finish_non_exhaustive()
    }
}

impl BrowserService {
    /// A service (nothing starts until the first command or viewer).
    #[must_use]
    pub fn new(options: BrowserOptions) -> Self {
        let ephemeral = options
            .ephemeral
            .then(|| {
                tempfile::Builder::new()
                    .prefix("oino-browser-")
                    .tempdir()
                    .ok()
            })
            .flatten();
        let root = ephemeral
            .as_ref()
            .map_or(options.root, |t| t.path().to_path_buf());
        Self {
            inner: Arc::new(Inner {
                settings: RwLock::new(options.settings),
                hub: Arc::new(Hub::new(root)),
                displays: Displays::default(),
                launching: tokio::sync::Mutex::new(()),
                live: Mutex::new(HashMap::new()),
                generation: AtomicU64::new(0),
                reaper: AtomicBool::new(false),
                _ephemeral: ephemeral,
                site_allow: Mutex::new(HashSet::new()),
                views: Mutex::new(HashMap::new()),
            }),
        }
    }

    /// Effective settings.
    #[must_use]
    pub fn settings(&self) -> BrowserSettings {
        self.inner
            .settings
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    /// Replaces the settings (display changes apply on the next launch).
    pub fn update_settings(&self, settings: BrowserSettings) {
        *self
            .inner
            .settings
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = settings;
        let status = self.status(None);
        let _ = self.inner.hub.global.send(BrowserViewEvent::Status {
            status: Box::new(status),
        });
    }

    /// Storage root.
    #[must_use]
    pub fn root(&self) -> PathBuf {
        self.inner.hub.paths.root.clone()
    }

    /// OS pids of running browsers (tests, cleanup checks).
    #[must_use]
    pub fn pids(&self) -> Vec<u32> {
        lock(&self.inner.live)
            .values()
            .filter_map(|p| p.process.pid())
            .collect()
    }

    /// Events of tabs without a session plus status changes.
    pub fn subscribe_global(&self) -> BoxStream<'static, BrowserViewEvent> {
        stream_of(self.inner.hub.global.subscribe())
    }

    /// Stops every browser and the hidden display.
    pub async fn shutdown(&self) {
        let all: Vec<Arc<ProfileRt>> = lock(&self.inner.live).drain().map(|(_, p)| p).collect();
        for p in all {
            p.shutdown("exit").await;
        }
        self.inner.displays.stop().await;
    }

    /// Stops one profile's browser.
    pub async fn stop_profile(&self, name: &str) {
        let p = lock(&self.inner.live).remove(name);
        if let Some(p) = p {
            p.shutdown("exit").await;
        }
    }

    pub(crate) fn profile_of(&self, scope: &BrowserScope) -> String {
        if let Some(p) = scope.profile.clone().filter(|p| valid_profile_name(p)) {
            return p;
        }
        lock(&self.inner.hub.sessions)
            .get(&scope.session_id)
            .and_then(|s| s.profile.clone())
            .unwrap_or_else(|| self.settings().profile)
    }

    /// Profiles on disk (+ running state).
    pub(crate) fn profiles(&self) -> Vec<ProfileInfo> {
        let dir = self.inner.hub.paths.root.join("profiles");
        let live = lock(&self.inner.live);
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .map(|d| {
                d.filter_map(Result::ok)
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .filter(|n| valid_profile_name(n))
                    .collect()
            })
            .unwrap_or_default();
        for n in live.keys() {
            if !names.contains(n) {
                names.push(n.clone());
            }
        }
        if !names.iter().any(|n| n == "default") {
            names.push("default".into());
        }
        names.sort();
        names
            .into_iter()
            .map(|name| ProfileInfo {
                running: live
                    .get(&name)
                    .is_some_and(|p| !p.dead.load(Ordering::SeqCst)),
                cookie_domains: crate::profiles::cookie_domains(
                    &self.inner.hub.paths.profile(&name),
                ),
                name,
            })
            .collect()
    }

    /// Finds a live tab by id.
    pub(crate) fn find_tab(&self, id: &str) -> Option<(Arc<ProfileRt>, Arc<Tab>)> {
        let live = lock(&self.inner.live);
        live.values()
            .find_map(|p| p.tab(id).map(|t| (Arc::clone(p), t)))
    }

    fn placement_choice(settings: &BrowserSettings, kind: EngineKind) -> DisplayChoice {
        match settings.display {
            DisplayChoice::Auto => {
                if display::xvfb().is_some() && kind == EngineKind::Cloak {
                    DisplayChoice::Hidden
                } else {
                    DisplayChoice::Headless
                }
            }
            other => other,
        }
    }

    /// The running profile `name`, launching it if needed.
    #[allow(
        clippy::too_many_lines,
        clippy::result_large_err,
        reason = "launch steps in order; the error is the ready-made tool result"
    )]
    pub(crate) async fn ensure(&self, name: &str) -> Result<Arc<ProfileRt>, BrowserResult> {
        let settings = self.settings();
        if !settings.enabled {
            return Err(BrowserResult::unavailable(
                "disabled",
                "browser control is off (Settings → Browser, `browser.enabled`)",
            ));
        }
        // QA-2: a launch that wedges must not wedge every later browser call waiting on this
        // mutex; fail as `launch_failed` once the wait exceeds the launch budget.
        let acquire = self.inner.launching.lock();
        let _guard = tokio::time::timeout(LAUNCH_TIMEOUT, acquire)
            .await
            .map_err(|_| {
                BrowserResult::unavailable(
                    "launch_failed",
                    format!(
                        "another browser launch has been pending for over {} s; try again",
                        LAUNCH_TIMEOUT.as_secs()
                    ),
                )
            })?;
        if let Some(p) = lock(&self.inner.live).get(name).cloned()
            && !p.dead.load(Ordering::SeqCst)
        {
            p.touch();
            return Ok(p);
        }
        lock(&self.inner.live).remove(name);
        let engine = engine::resolve(&settings).map_err(|m| match m {
            EngineMissing::NoEngine => BrowserResult::unavailable(
                "no_engine",
                "no Chrome-family browser found; install Google Chrome or Chromium, or set browser.executablePath",
            ),
            EngineMissing::CloakNotInstalled => BrowserResult::unavailable(
                "cloak_not_installed",
                "CloakBrowser is not installed; install it from Settings → Browser or set browser.engine to auto",
            ),
        })?;
        let running = lock(&self.inner.live).len();
        if running >= settings.max_running {
            let victim = lock(&self.inner.live)
                .iter()
                .map(|(n, p)| (n.clone(), p.idle_for()))
                .filter(|(_, idle)| *idle > Duration::from_secs(60))
                .max_by_key(|(_, idle)| *idle)
                .map(|(n, _)| n);
            match victim {
                Some(v) => {
                    let p = lock(&self.inner.live).remove(&v);
                    if let Some(p) = p {
                        p.shutdown("idle").await;
                    }
                }
                None => {
                    return Err(BrowserResult::unavailable(
                        "too_many",
                        format!(
                            "{running} browser profiles are busy (browser.maxRunning); close tabs of another profile or stay on this session's profile"
                        ),
                    ));
                }
            }
        }
        let placement = match Self::placement_choice(&settings, engine.kind) {
            DisplayChoice::Hidden => match self.inner.displays.ensure().await {
                Ok(d) => Placement::Hidden(d),
                Err(e) => {
                    tracing::info!(%e, "browser: hidden display unavailable, using headless");
                    Placement::Headless
                }
            },
            DisplayChoice::Window => {
                let graphical = !cfg!(target_os = "linux")
                    || std::env::var_os("DISPLAY").is_some()
                    || std::env::var_os("WAYLAND_DISPLAY").is_some();
                if !graphical {
                    return Err(BrowserResult::unavailable(
                        "no_display",
                        "no graphical session for browser.display = window",
                    ));
                }
                Placement::Window
            }
            DisplayChoice::Headless | DisplayChoice::Auto => Placement::Headless,
        };
        let pipe = match settings.transport {
            TransportChoice::Auto => cfg!(unix),
            TransportChoice::Pipe => true,
            TransportChoice::Port => false,
        };
        let user_data_dir =
            crate::profiles::user_data_dir(&self.inner.hub.paths, name, &engine.executable);
        let spec = LaunchSpec {
            engine: engine.clone(),
            user_data_dir: user_data_dir.clone(),
            placement,
            pipe,
        };
        let process = {
            // QA-2: browser start is bounded; a wedged launch surfaces as `launch_failed`
            // (browser-control.md §5.5) instead of a tool call that never ends.
            let launch = BrowserProcess::launch(&spec);
            match tokio::time::timeout(LAUNCH_TIMEOUT, launch).await {
                Ok(Ok(p)) => p,
                Ok(Err(e)) => {
                    return Err(BrowserResult::unavailable(
                        "launch_failed",
                        format!("{} did not start: {e}", engine.product),
                    ));
                }
                Err(_) => {
                    return Err(BrowserResult::unavailable(
                        "launch_failed",
                        format!(
                            "{} did not start within {} s",
                            engine.product,
                            LAUNCH_TIMEOUT.as_secs()
                        ),
                    ));
                }
            }
        };
        let generation = self.inner.generation.fetch_add(1, Ordering::SeqCst) + 1;
        let grants = crate::permissions::load(&self.inner.hub.paths.site_permissions());
        let rt = {
            let start = ProfileRt::start(
                name.to_owned(),
                generation,
                engine.kind.as_str().to_owned(),
                process,
                Arc::clone(&self.inner.hub),
                settings.downloads_dir.clone(),
                &grants,
            );
            // QA-2: the CDP handshake is bounded too; a wedged engine becomes
            // `launch_failed` instead of a tool call that never ends. Dropping the future
            // drops `BrowserProcess`, which kills the process group.
            match tokio::time::timeout(LAUNCH_TIMEOUT, start).await {
                Ok(Ok(rt)) => rt,
                Ok(Err(e)) => {
                    return Err(BrowserResult::unavailable("launch_failed", e.to_string()));
                }
                Err(_) => {
                    return Err(BrowserResult::unavailable(
                        "launch_failed",
                        format!(
                            "the browser did not answer CDP within {} s",
                            LAUNCH_TIMEOUT.as_secs()
                        ),
                    ));
                }
            }
        };
        crate::profiles::touch_meta(&user_data_dir);
        lock(&self.inner.live).insert(name.to_owned(), Arc::clone(&rt));
        self.start_reaper();
        let status = self.status(None);
        let _ = self.inner.hub.global.send(BrowserViewEvent::Status {
            status: Box::new(status),
        });
        Ok(rt)
    }

    fn start_reaper(&self) {
        if self.inner.reaper.swap(true, Ordering::SeqCst) {
            return;
        }
        let weak = Arc::downgrade(&self.inner);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let Some(inner) = weak.upgrade() else { return };
                let svc = BrowserService { inner };
                svc.reap().await;
            }
        });
    }

    /// Idle shutdown (§6.3) and take-over expiry (§7.4).
    async fn reap(&self) {
        let idle = self.settings().idle_minutes;
        let profiles: Vec<Arc<ProfileRt>> = lock(&self.inner.live).values().cloned().collect();
        for p in profiles {
            for tab in p.all_tabs() {
                let expired = {
                    let s = lock(&tab.state);
                    s.controlled_by == oino_browser_protocol::ControlledBy::User
                        && s.handoff.is_none()
                        && s.last_user_input
                            .is_none_or(|t| t.elapsed() > Duration::from_secs(600))
                };
                if expired {
                    crate::frontend::set_takeover(&p, &tab, false);
                }
            }
            if idle > 0 && p.idle_for() > Duration::from_secs(idle * 60) {
                lock(&self.inner.live).remove(&p.name);
                p.shutdown("idle").await;
            }
        }
    }

    /// Opens a tab for `owner` in `rt`, closing the oldest safe tab when over the limit.
    pub(crate) async fn open_tab(
        &self,
        rt: &Arc<ProfileRt>,
        owner: TabOwner,
        url: &str,
    ) -> Result<Arc<Tab>, crate::page::ActionError> {
        let cands: Vec<Candidate> = rt.all_tabs().iter().map(|t| Candidate::of(t)).collect();
        if let Some(victim) = residency_victim(&cands, self.settings().max_tabs)
            && let Some(t) = rt.tab(&victim)
        {
            rt.close_tab(&t, "limit").await;
        }
        Ok(rt.open_tab(owner, url).await?)
    }

    fn status_for(&self, scope: Option<&BrowserScope>) -> BrowserStatus {
        let settings = self.settings();
        let engine = engine::resolve(&settings);
        let live = lock(&self.inner.live).values().next().cloned();
        let mut st = BrowserStatus {
            enabled: settings.enabled,
            available: settings.enabled && engine.is_ok(),
            reason: if settings.enabled {
                match engine {
                    Err(EngineMissing::NoEngine) => Some(UnavailableReason::NoEngine),
                    Err(EngineMissing::CloakNotInstalled) => {
                        Some(UnavailableReason::CloakNotInstalled)
                    }
                    Ok(_) => None,
                }
            } else {
                Some(UnavailableReason::Disabled)
            },
            engine: engine.as_ref().ok().map(|e| e.kind.as_str().to_owned()),
            product: engine.as_ref().ok().map(|e| e.product.clone()),
            version: live.as_ref().map(|p| p.version.clone()),
            executable: engine
                .as_ref()
                .ok()
                .map(|e| e.executable.display().to_string()),
            stealth: engine.as_ref().is_ok_and(|e| e.kind == EngineKind::Cloak),
            stealth_note: None,
            display: live.as_ref().map_or_else(
                || {
                    let kind = engine.as_ref().map_or(EngineKind::Chrome, |e| e.kind);
                    match Self::placement_choice(&settings, kind) {
                        DisplayChoice::Hidden => "hidden",
                        DisplayChoice::Window => "window",
                        _ => "headless",
                    }
                    .to_owned()
                },
                |p| p.process.placement.as_str().to_owned(),
            ),
            profiles: self.profiles(),
            session_profile: None,
            tabs: Vec::new(),
        };
        if st.stealth && st.display == "headless" {
            st.stealth_note = Some("Less stealthy: no hidden display (install Xvfb)".into());
        } else if !st.stealth && st.available {
            st.stealth_note = Some("Not stealthy: a regular Chrome-family browser".into());
        }
        if let Some(scope) = scope {
            st.session_profile = Some(self.profile_of(scope));
            st.tabs = lock(&self.inner.live)
                .values()
                .flat_map(|p| p.session_tabs(Some(&scope.session_id)))
                .collect();
        }
        st
    }

    pub(crate) fn hub(&self) -> &Arc<Hub> {
        &self.inner.hub
    }
}

/// Broadcast receiver → stream (lag skipped).
pub(crate) fn stream_of(
    rx: tokio::sync::broadcast::Receiver<BrowserViewEvent>,
) -> BoxStream<'static, BrowserViewEvent> {
    Box::pin(futures::stream::unfold(rx, |mut rx| async move {
        loop {
            match rx.recv().await {
                Ok(e) => return Some((e, rx)),
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
            }
        }
    }))
}

impl BrowserHost for BrowserService {
    fn execute(
        &self,
        scope: BrowserScope,
        request: BrowserRequest,
        cancel: CancellationToken,
    ) -> BoxFut<'_, BrowserResult> {
        Box::pin(async move { crate::commands::execute(self, scope, request, cancel).await })
    }

    fn status(&self, scope: Option<&BrowserScope>) -> BrowserStatus {
        self.status_for(scope)
    }

    fn subscribe(&self, session_id: &str) -> BoxStream<'static, BrowserViewEvent> {
        stream_of(self.inner.hub.session_tx(session_id, "").subscribe())
    }

    fn attach_view(
        &self,
        session_id: Option<&str>,
        tab: &str,
        viewport: Option<Viewport>,
    ) -> Result<ViewStream, String> {
        crate::frontend::attach_view(self, session_id, tab, viewport)
    }

    fn frontend(
        &self,
        session_id: Option<&str>,
        workspace: &str,
        request: FrontendRequest,
    ) -> BoxFut<'_, Result<serde_json::Value, String>> {
        let session = session_id.map(str::to_owned);
        let workspace = workspace.to_owned();
        Box::pin(async move {
            crate::frontend::handle(self, session.as_deref(), &workspace, request).await
        })
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        // Browser processes are killed by `BrowserProcess::drop` (process group).
        lock(&self.live).clear();
    }
}
