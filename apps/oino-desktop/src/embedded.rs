//! [`EmbeddedHost`]: `oino-core` in-process with every bundled component (gpui-app §3.3, D14).
//!
//! One `Services` (settings, resources, models, extension runner + wasmtime runtime) per
//! working directory, shared by every session opened there. Bundled packages come from
//! `oino-bundle` (resources extracted once to `~/.oino/bundled/`); installed packages from
//! `~/.oino/extensions` and, trust-gated, `<cwd>/.oino/extensions`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
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

fn load_bundle(agent_dir: &Path) -> Bundle {
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
                bundle.infos.push(ExtensionInfo {
                    id: package.manifest.id.to_string(),
                    version: package.manifest.version.clone(),
                    kind: format!("{:?}", package.manifest.kind).to_lowercase(),
                    bundled: true,
                    error: None,
                });
                bundle.packages.push(SourcedPackage {
                    package,
                    project: false,
                });
            }
            Err(error) => bundle.infos.push(ExtensionInfo {
                id: ext.id.to_owned(),
                version: ext.version.to_owned(),
                kind: "unknown".to_owned(),
                bundled: true,
                error: Some(error),
            }),
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
    runtimes: Mutex<Vec<Arc<AgentSessionRuntime>>>,
}

/// The in-process host.
#[derive(Clone)]
pub(crate) struct EmbeddedHost {
    inner: Arc<Inner>,
}

impl EmbeddedHost {
    /// A host over `agent_dir`; new sessions start in `default_cwd`.
    #[must_use]
    pub(crate) fn new(agent_dir: PathBuf, default_cwd: PathBuf) -> Self {
        let bundle = load_bundle(&agent_dir);
        tracing::info!(count = bundle.packages.len(), "bundled extensions");
        Self {
            inner: Arc::new(Inner {
                infos: Mutex::new(bundle.infos.clone()),
                agent_dir,
                default_cwd,
                bundle,
                workspaces: tokio::sync::Mutex::new(HashMap::new()),
                runtimes: Mutex::new(Vec::new()),
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
        let runtime = Arc::new(runtime);
        let (global, _) =
            oino_wasm_host::packages::discover(&[self.agent_dir.join("extensions")], false);
        let (project, _) =
            oino_wasm_host::packages::discover(&[cwd.join(".oino").join("extensions")], false);
        let mut packages = self.bundle.packages.clone();
        let bundled_ids: Vec<String> = packages
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
            extra_infos.push(ExtensionInfo {
                id,
                version: package.manifest.version.clone(),
                kind: format!("{:?}", package.manifest.kind).to_lowercase(),
                bundled: false,
                error: None,
            });
            packages.push(SourcedPackage {
                package,
                project: is_project,
            });
        }
        let loader = Arc::new(CoreLoader::new(runtime, packages));
        let services = Services::create(ServicesOptions {
            cwd: cwd.to_path_buf(),
            agent_dir: self.agent_dir.clone(),
            docs_root: self.agent_dir.clone(),
            mode: ExtensionMode::Rpc,
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
        lock(&self.runtimes).push(Arc::clone(&runtime));
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
