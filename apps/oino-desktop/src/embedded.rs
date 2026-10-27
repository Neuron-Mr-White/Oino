//! [`EmbeddedHost`]: `oino-core` in-process with every bundled component (gpui-app §3.3, D14).
//!
//! One `Services` (settings, resources, models, extension runner + wasmtime runtime) per
//! working directory, shared by every session opened there. Bundled packages come from
//! `oino-bundle` (resources extracted once to `~/.oino/bundled/`); installed packages from
//! `~/.oino/extensions` and, trust-gated, `<cwd>/.oino/extensions`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::time::UNIX_EPOCH;

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oino_client::InProcessBackend;
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions};
use oino_session::SessionManager;
use oino_ui::host::{
    AppSettings, ExtensionInfo, HostError, HostInfo, OpenRequest, OpenedSession, SessionHost,
};
use oino_ui::model::sessions::SessionSummary;
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};

use crate::prefs;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Bundled packages, extracted once per process.
#[derive(Clone, Default)]
struct Bundle {
    packages: Vec<SourcedPackage>,
    infos: Vec<ExtensionInfo>,
}

/// A short permission summary for the Extensions page (design.md D12).
fn permission_summary(p: &oino_extension_api::manifest::Permissions) -> Vec<String> {
    let mut out = Vec::new();
    if p.session_control {
        out.push("session control".to_owned());
    }
    if p.process.spawn {
        out.push("process".to_owned());
    }
    if !p.network.hosts.is_empty() {
        out.push(if p.network.hosts.iter().any(|h| h == "*") {
            "network (any host)".to_owned()
        } else {
            format!("network ({})", p.network.hosts.len())
        });
    }
    if !p.credentials.is_empty() {
        out.push("credentials".to_owned());
    }
    if p.secrets {
        out.push("secrets".to_owned());
    }
    if !p.fs.paths.is_empty() {
        out.push("fs paths".to_owned());
    }
    if p.os_notify {
        out.push("notifications".to_owned());
    }
    if p.open_url {
        out.push("open url".to_owned());
    }
    if p.edit_externally {
        out.push("editor".to_owned());
    }
    out
}

fn info_of(
    manifest: &oino_extension_api::manifest::Manifest,
    bundled: bool,
    root: Option<PathBuf>,
    disabled: &[String],
) -> ExtensionInfo {
    let id = manifest.id.to_string();
    let mut info = ExtensionInfo::basic(
        &id,
        &manifest.version,
        &format!("{:?}", manifest.kind).to_lowercase(),
        bundled,
        None,
    );
    info.name.clone_from(&manifest.name);
    info.description.clone_from(&manifest.description);
    info.permissions = permission_summary(&manifest.permissions);
    info.enabled = !disabled.contains(&id);
    info.root = root;
    info
}

/// `disabledExtensions` from the global settings.
fn disabled_ids(agent_dir: &Path) -> Vec<String> {
    let cwd = oino_settings::paths::home_dir().unwrap_or_else(|| agent_dir.to_path_buf());
    oino_settings::SettingsManager::create(&cwd, agent_dir, false)
        .settings()
        .disabled_extensions()
}

fn copy_tree(from: &Path, to: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), target)?;
        }
    }
    Ok(())
}

fn load_bundle(agent_dir: &Path) -> Bundle {
    let disabled = disabled_ids(agent_dir);
    let mut bundle = Bundle::default();
    for ext in oino_bundle::BUNDLED {
        let root = ext.extract(agent_dir).ok();
        let package = ext
            .component()
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                oino_wasm_host::packages::from_bytes(ext.manifest, Arc::from(bytes), root.clone())
                    .map_err(|e| e.to_string())
            });
        match package {
            Ok(package) => {
                bundle
                    .infos
                    .push(info_of(&package.manifest, true, root.clone(), &disabled));
                bundle.packages.push(SourcedPackage {
                    package,
                    project: false,
                });
            }
            Err(error) => bundle.infos.push(ExtensionInfo::basic(
                ext.id,
                ext.version,
                "unknown",
                true,
                Some(error),
            )),
        }
    }
    bundle
}

struct Workspace {
    services: Arc<Services>,
}

struct Inner {
    agent_dir: PathBuf,
    default_cwd: PathBuf,
    bundle: Bundle,
    workspaces: tokio::sync::Mutex<HashMap<PathBuf, Arc<Workspace>>>,
    infos: Mutex<Vec<ExtensionInfo>>,
    /// Live session runtimes (weak: a released pane drops its runtime; strong references
    /// here kept every session ever opened alive, with its extension state).
    runtimes: Mutex<Vec<Weak<AgentSessionRuntime>>>,
    /// Process-wide browser service served to `oino.browser` (browser-control.md §3).
    browser: Option<oino_browser::BrowserService>,
    /// The cron scheduler this process attaches to: the single-owner embedded scheduler
    /// when nobody else ticks, else the daemon's client (unipi-port §3.24).
    cron: CronAttach,
}

/// How this process reaches the cron scheduler.
enum CronAttach {
    /// We own the tick loop (embedded scheduler + its tick task).
    Embedded(oino_cron::serve::Embedded),
    /// The daemon (`oino cron serve` / the user service) owns it; we are a client.
    Daemon,
    /// No cron paths at all.
    None,
}

impl CronAttach {
    /// The host-facing handle.
    fn host(&self) -> Option<Arc<dyn oino_extension_api::cron::CronHost>> {
        match self {
            Self::Embedded(embedded) => Some(Arc::new(oino_cron::host::EmbeddedCronHost {
                scheduler: embedded.scheduler.clone(),
            })),
            Self::Daemon => oino_cron::CronPaths::from_agent_dir().ok().map(|paths| {
                Arc::new(oino_cron::host::DaemonCronHost { paths })
                    as Arc<dyn oino_extension_api::cron::CronHost>
            }),
            Self::None => None,
        }
    }

    /// Stops the embedded tick loop, if we own it (app quit).
    fn shutdown(&self) {
        if let Self::Embedded(embedded) = self {
            embedded.shutdown();
        }
    }
}

/// The in-process host.
#[derive(Clone)]
pub(crate) struct EmbeddedHost {
    inner: Arc<Inner>,
}

impl EmbeddedHost {
    /// A host over `agent_dir`; new sessions start in `default_cwd`.
    #[must_use]
    #[cfg(test)]
    pub(crate) fn new(agent_dir: PathBuf, default_cwd: PathBuf) -> Self {
        Self::with_browser(agent_dir, default_cwd, None)
    }

    /// A host whose extension runtimes get `browser` (the in-app browser service).
    pub(crate) fn with_browser(
        agent_dir: PathBuf,
        default_cwd: PathBuf,
        browser: Option<oino_browser::BrowserService>,
    ) -> Self {
        let bundle = load_bundle(&agent_dir);
        tracing::info!(count = bundle.packages.len(), "bundled extensions");
        // Attach to the cron scheduler: become the single ticking owner when nobody else
        // holds the lock, else talk to the daemon. Single-owner lock: only one scheduler
        // fires jobs (unipi-port §3.24).
        let cron = {
            let paths = oino_cron::CronPaths::new(&agent_dir);
            match oino_cron::serve::Embedded::start(paths) {
                Ok(embedded) => {
                    tracing::info!("cron: embedded scheduler owns the tick loop");
                    CronAttach::Embedded(embedded)
                }
                Err(oino_cron::CronError::AlreadyOwned(holder)) => {
                    tracing::info!(%holder, "cron: daemon owns the tick loop; attaching as client");
                    CronAttach::Daemon
                }
                Err(error) => {
                    tracing::warn!(%error, "cron: scheduler unavailable");
                    CronAttach::None
                }
            }
        };
        Self {
            inner: Arc::new(Inner {
                infos: Mutex::new(bundle.infos.clone()),
                agent_dir,
                default_cwd,
                bundle,
                workspaces: tokio::sync::Mutex::new(HashMap::new()),
                runtimes: Mutex::new(Vec::new()),
                browser,
                cron,
            }),
        }
    }
}

impl Inner {
    async fn workspace(&self, cwd: &Path) -> Result<Arc<Workspace>, HostError> {
        let mut workspaces = self.workspaces.lock().await;
        if let Some(ws) = workspaces.get(cwd) {
            return Ok(Arc::clone(ws));
        }
        let runtime = WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
            self.agent_dir.clone(),
            cwd,
        )))
        .map_err(|e| HostError::Session(format!("wasm runtime: {e}")))?;
        if let (Some(hub), Some(browser)) = (runtime.local_services(), &self.browser) {
            hub.set_browser(Some(
                Arc::new(browser.clone()) as Arc<dyn oino_extension_api::browser::BrowserHost>
            ));
        }
        if let Some(hub) = runtime.local_services() {
            hub.set_cron(self.cron.host());
        }
        let runtime = Arc::new(runtime);
        let (global, _) =
            oino_wasm_host::packages::discover(&[self.agent_dir.join("extensions")], false);
        let (project, _) =
            oino_wasm_host::packages::discover(&[cwd.join(".oino").join("extensions")], false);
        // `disabledExtensions` (Extensions page → Disable) applies to new workspaces.
        let disabled = disabled_ids(&self.agent_dir);
        let mut packages: Vec<SourcedPackage> = self
            .bundle
            .packages
            .iter()
            .filter(|p| !disabled.contains(&p.package.manifest.id.to_string()))
            .cloned()
            .collect();
        let bundled_ids: Vec<String> = self
            .bundle
            .packages
            .iter()
            .map(|p| p.package.manifest.id.to_string())
            .collect();
        let mut extra_infos = Vec::new();
        for (package, is_project) in global
            .into_iter()
            .map(|p| (p, false))
            .chain(project.into_iter().map(|p| (p, true)))
        {
            let id = package.manifest.id.to_string();
            if bundled_ids.contains(&id) {
                continue;
            }
            extra_infos.push(info_of(
                &package.manifest,
                false,
                package.root.clone(),
                &disabled,
            ));
            if !disabled.contains(&id) {
                packages.push(SourcedPackage {
                    package,
                    project: is_project,
                });
            }
        }
        let loader = Arc::new(CoreLoader::new(runtime, packages));
        let services = Services::create(ServicesOptions {
            cwd: cwd.to_path_buf(),
            agent_dir: self.agent_dir.clone(),
            docs_root: oino_bundle::DOCS
                .extract(&self.agent_dir)
                .unwrap_or_else(|_| self.agent_dir.clone()),
            mode: ExtensionMode::Desktop,
            has_ui: true,
            flags: ResourceFlags::default(),
            extension_flag_values: IndexMap::new(),
            loader: Arc::clone(&loader) as Arc<dyn oino_core::services::ExtensionLoader>,
            approve: None,
            models: None,
            settings: None,
        })
        .await
        .map_err(|e| HostError::Session(e.to_string()))?;
        {
            let errors = lock(&loader.errors);
            let mut infos = lock(&self.infos);
            for info in infos.iter_mut().chain(extra_infos.iter_mut()) {
                if let Some(error) = errors.iter().find(|e| e.extension == info.id) {
                    info.error = Some(error.message.clone());
                }
            }
            for extra in extra_infos {
                if !infos.iter().any(|i| i.id == extra.id) {
                    infos.push(extra);
                }
            }
        }
        for diagnostic in &services.diagnostics {
            tracing::warn!(%diagnostic, "services");
        }
        let ws = Arc::new(Workspace { services });
        workspaces.insert(cwd.to_path_buf(), Arc::clone(&ws));
        Ok(ws)
    }

    async fn open(&self, request: OpenRequest) -> Result<OpenedSession, HostError> {
        let (cwd, source) = match request {
            OpenRequest::New { cwd } => (cwd, SessionSource::New),
            OpenRequest::Resume { path, cwd } => (cwd, SessionSource::Open(path)),
        };
        let cwd = std::fs::canonicalize(&cwd).unwrap_or(cwd);
        let ws = self.workspace(&cwd).await?;
        let services = Arc::clone(&ws.services);
        let context = Arc::new(SessionToolContext::default());
        let settings = services
            .settings
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .snapshot();
        let builtin_tools = builtin_tool_entries(
            &services.cwd,
            &services.agent_dir,
            &settings,
            Arc::clone(&context),
        );
        let options = SessionOptions {
            source,
            builtin_tools,
            ..SessionOptions::default()
        };
        let runtime = AgentSessionRuntime::create(services, options)
            .await
            .map_err(|e| HostError::Session(e.to_string()))?;
        let session = runtime.session();
        context.bind(&session);
        let file = session.session_file();
        let runtime = Arc::new(runtime);
        {
            let mut runtimes = lock(&self.runtimes);
            runtimes.retain(|r| r.strong_count() > 0);
            runtimes.push(Arc::downgrade(&runtime));
        }
        // GA-45 / frontend-api §10: the desktop registers as a frontend so `host-info` and
        // `/doctor` report `desktop` plus this client name.
        session.register_frontend("oino-desktop");
        Ok(OpenedSession {
            backend: Arc::new(InProcessBackend::new(runtime)),
            cwd: cwd.display().to_string(),
            file,
        })
    }
}

fn modified_ms(time: std::time::SystemTime) -> i64 {
    time.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or_default()
}

fn summary(info: oino_session::manager::SessionInfo) -> SessionSummary {
    let title = info
        .name
        .clone()
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| {
            info.first_message
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned()
        });
    SessionSummary {
        id: info.id,
        path: Some(info.path),
        cwd: info.cwd,
        title,
        modified_ms: modified_ms(info.modified),
        message_count: info.message_count,
    }
}

impl SessionHost for EmbeddedHost {
    fn info(&self) -> HostInfo {
        HostInfo {
            agent_dir: self.inner.agent_dir.clone(),
            default_cwd: self.inner.default_cwd.clone(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            backend: "in-process".to_owned(),
        }
    }

    fn list_sessions(&self) -> BoxFuture<'static, Result<Vec<SessionSummary>, HostError>> {
        let root = std::env::var_os("OINO_SESSION_DIR")
            .filter(|d| !d.is_empty())
            .map_or_else(|| self.inner.agent_dir.join("sessions"), PathBuf::from);
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                SessionManager::list_all(&root, None, None)
                    .map(|list| list.into_iter().map(summary).collect())
                    .map_err(|e| HostError::Io(e.to_string()))
            })
            .await
            .map_err(|e| HostError::Io(e.to_string()))?
        })
    }

    fn open(&self, request: OpenRequest) -> BoxFuture<'static, Result<OpenedSession, HostError>> {
        let inner = Arc::clone(&self.inner);
        Box::pin(async move { inner.open(request).await })
    }

    fn extensions(&self) -> BoxFuture<'static, Vec<ExtensionInfo>> {
        let infos = lock(&self.inner.infos).clone();
        Box::pin(async move { infos })
    }

    fn app_settings(&self) -> AppSettings {
        prefs::load(&self.inner.agent_dir)
    }

    fn save_app_settings(&self, settings: &AppSettings) {
        prefs::save(&self.inner.agent_dir, settings);
    }

    fn set_extension_enabled(&self, id: &str, enabled: bool) -> Result<(), HostError> {
        let agent_dir = &self.inner.agent_dir;
        let cwd = oino_settings::paths::home_dir().unwrap_or_else(|| agent_dir.clone());
        let mut manager = oino_settings::SettingsManager::create(&cwd, agent_dir, false);
        let mut ids = manager.settings().disabled_extensions();
        ids.retain(|d| d != id);
        if !enabled {
            ids.push(id.to_owned());
        }
        manager
            .set_disabled_extensions(&ids)
            .map_err(|e| HostError::Io(e.to_string()))?;
        for info in lock(&self.inner.infos).iter_mut() {
            if info.id == id {
                info.enabled = enabled;
            }
        }
        Ok(())
    }

    fn install_extension(&self, path: PathBuf) -> BoxFuture<'static, Result<String, HostError>> {
        let dest_root = self.inner.agent_dir.join("extensions");
        Box::pin(async move {
            tokio::task::spawn_blocking(move || {
                let dir = if path.is_file() {
                    path.parent().map(Path::to_path_buf).unwrap_or_default()
                } else {
                    path
                };
                let package = oino_wasm_host::packages::load_dir(&dir, false)
                    .map_err(|e| HostError::Io(format!("{}: {e}", dir.display())))?;
                let id = package.manifest.id.to_string();
                copy_tree(&dir, &dest_root.join(&id)).map_err(|e| HostError::Io(e.to_string()))?;
                Ok(id)
            })
            .await
            .map_err(|e| HostError::Io(e.to_string()))?
        })
    }

    fn extensions_dir(&self) -> Option<PathBuf> {
        Some(self.inner.agent_dir.join("extensions"))
    }

    fn cron_host(&self) -> Option<Arc<dyn oino_extension_api::cron::CronHost>> {
        self.inner.cron.host()
    }

    fn shutdown(&self) -> BoxFuture<'static, ()> {
        self.inner.cron.shutdown();
        let live: Vec<Arc<AgentSessionRuntime>> = lock(&self.inner.runtimes)
            .drain(..)
            .filter_map(|r| r.upgrade())
            .collect();
        Box::pin(async move {
            futures::future::join_all(live.iter().map(|r| r.dispose())).await;
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt as _;
    use oino_client::PiCommand;
    use oino_core::events::{AgentSessionEvent, ClientEvent};
    use oino_ui::model::session::SessionModel;
    use oino_ui::model::transcript::RowKind;

    fn copy_dir(from: &Path, to: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(to)?;
        for entry in std::fs::read_dir(from)? {
            let entry = entry?;
            std::fs::copy(entry.path(), to.join(entry.file_name()))?;
        }
        Ok(())
    }

    /// The real embedding path: bundled components plus an installed provider package
    /// (`extensions/dist` fixture) in `~/.oino/extensions`, one prompt end to end through the
    /// view models. Skipped when `cargo xtask wasm` has not produced the fixture.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn embedded_session_streams_through_an_installed_provider() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../extensions/dist/oino.fixture.provider-faux");
        if !fixture.join("component.wasm").is_file() {
            eprintln!("skipped: run `cargo xtask wasm` to build the provider-faux fixture");
            return;
        }
        let tmp = std::env::temp_dir().join(format!("oino-desktop-e2e-{}", std::process::id()));
        let agent = tmp.join("agent");
        let cwd = tmp.join("project");
        std::fs::create_dir_all(&cwd).unwrap();
        copy_dir(&fixture, &agent.join("extensions").join("provider-faux")).unwrap();
        std::fs::write(
            agent.join("settings.json"),
            r#"{"defaultProvider":"faux","defaultModel":"faux-1"}"#,
        )
        .unwrap();
        std::fs::write(
            agent.join("auth.json"),
            r#"{"faux":{"type":"api_key","key":"k"}}"#,
        )
        .unwrap();
        let host = EmbeddedHost::new(agent.clone(), cwd.clone());
        let opened = host
            .open(OpenRequest::New { cwd: cwd.clone() })
            .await
            .unwrap();
        let backend = opened.backend;
        let mut events = backend.events();
        backend
            .send(
                PiCommand::SetModel {
                    provider: "faux".into(),
                    model_id: "faux-1".into(),
                }
                .into(),
            )
            .await
            .unwrap();
        backend
            .send(
                PiCommand::Prompt {
                    message: "hi".into(),
                    images: Vec::new(),
                    streaming_behavior: None,
                }
                .into(),
            )
            .await
            .unwrap();
        let mut model = SessionModel::new(backend.session_id(), cwd.display().to_string());
        loop {
            let event = tokio::time::timeout(std::time::Duration::from_secs(60), events.next())
                .await
                .expect("timed out")
                .expect("stream ended");
            model.apply(&event);
            if matches!(
                event,
                ClientEvent::Session(AgentSessionEvent::AgentSettled { .. })
            ) {
                break;
            }
        }
        let text: Vec<String> = model
            .transcript
            .rows()
            .iter()
            .filter_map(|r| match &r.kind {
                RowKind::Assistant(a) => Some(a.text.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(
            text,
            vec!["Hello from faux.".to_owned()],
            "{:?}",
            model.transcript.rows()
        );
        let infos = host.extensions().await;
        assert!(
            infos
                .iter()
                .any(|i| i.id == "oino.fixture.provider-faux" && !i.bundled)
        );
        let _ = std::fs::remove_dir_all(tmp);
    }
}
