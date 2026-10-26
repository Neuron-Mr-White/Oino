//! [`WasmRuntime`]: the shared wasmtime engine (epoch ticker, compile cache, linker) and the
//! [`ExtensionRuntime`] implementation.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use oino_extension_api::manifest::{ExtensionKind, Manifest};
use oino_extension_api::{
    BoxFut, CallContext, ComponentSource, ExtensionError, ExtensionInstance, ExtensionRuntime,
    HostServices, LoadedPackage,
};
use wasmtime::component::{Component, Linker};
use wasmtime::{Config, Engine, StoreLimits, StoreLimitsBuilder};

use crate::http::HttpService;
use crate::hub::LocalServices;
use crate::instance::WasmInstance;
use crate::paths::{self, ExtPaths};
use crate::state::{ExtShared, HostConfig, HostState, HttpGrants};

/// Epoch tick (deadline granularity).
pub const EPOCH_TICK: Duration = Duration::from_millis(10);

/// Engine-level failure.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum WasmHostError {
    /// Engine or linker setup.
    #[error("wasm engine setup failed: {0}")]
    Engine(String),
}

pub(crate) struct EngineShared {
    pub(crate) engine: Engine,
    pub(crate) linker: Linker<HostState>,
    stop: Arc<AtomicBool>,
}

impl Drop for EngineShared {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

impl EngineShared {
    pub(crate) fn limits(manifest: &Manifest) -> StoreLimits {
        let bytes = usize::try_from(manifest.limits.memory_mib).unwrap_or(512) * 1024 * 1024;
        StoreLimitsBuilder::new()
            .memory_size(bytes)
            .table_elements(100_000)
            .instances(10)
            .build()
    }
}

/// Options of [`WasmRuntime`].
#[derive(Clone, Debug)]
pub struct WasmRuntimeOptions {
    /// Host config (agent dir, cwd, proxy, versions).
    pub config: HostConfig,
    /// Compiled-component cache dir (`~/.oino/cache/wasmtime`); `None` disables it.
    pub cache_dir: Option<PathBuf>,
    /// Serve `bus`, `services`, `secrets`, `tasks` and host-produced event delivery locally
    /// (until core implements them) — see [`LocalServices`].
    pub local_services: bool,
}

impl WasmRuntimeOptions {
    /// Defaults for `config` (cache in `<agent>/cache/wasmtime`, local services on).
    #[must_use]
    pub fn new(config: HostConfig) -> Self {
        Self {
            cache_dir: Some(config.agent_dir.join("cache").join("wasmtime")),
            config,
            local_services: true,
        }
    }
}

/// The wasmtime component runtime.
pub struct WasmRuntime {
    shared: Arc<EngineShared>,
    options: WasmRuntimeOptions,
    hub: Option<Arc<LocalServices>>,
}

impl std::fmt::Debug for WasmRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmRuntime")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

impl WasmRuntime {
    /// Creates the engine (component model + async, epoch interruption, cache) and linker.
    ///
    /// # Errors
    ///
    /// Engine configuration or linker setup failed.
    pub fn new(options: WasmRuntimeOptions) -> Result<Self, WasmHostError> {
        let err = |e: wasmtime::Error| WasmHostError::Engine(format!("{e:#}"));
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .wasm_component_model_async(true)
            .epoch_interruption(true);
        if let Some(dir) = &options.cache_dir {
            let mut cache = wasmtime::CacheConfig::new();
            cache.with_directory(dir);
            if let Ok(cache) = wasmtime::Cache::new(cache) {
                config.cache(Some(cache));
            }
        }
        let engine = Engine::new(&config).map_err(err)?;
        let mut linker = Linker::<HostState>::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker).map_err(err)?;
        crate::bindings::add_oino_to_linker(&mut linker).map_err(err)?;
        let stop = Arc::new(AtomicBool::new(false));
        let ticker_engine = engine.clone();
        let ticker_stop = Arc::clone(&stop);
        std::thread::Builder::new()
            .name("oino-wasm-epoch".into())
            .spawn(move || {
                while !ticker_stop.load(Ordering::Relaxed) {
                    std::thread::sleep(EPOCH_TICK);
                    ticker_engine.increment_epoch();
                }
            })
            .map_err(|e| WasmHostError::Engine(e.to_string()))?;
        let hub = options
            .local_services
            .then(|| Arc::new(LocalServices::new(&options.config.agent_dir)));
        Ok(Self {
            shared: Arc::new(EngineShared {
                engine,
                linker,
                stop,
            }),
            options,
            hub,
        })
    }

    /// The local services hub, when enabled.
    #[must_use]
    pub fn local_services(&self) -> Option<&Arc<LocalServices>> {
        self.hub.as_ref()
    }

    /// Compiles `source` (cached).
    fn compile(&self, source: &ComponentSource) -> Result<Component, String> {
        match source {
            ComponentSource::Bytes(bytes) => Component::new(&self.shared.engine, bytes),
            ComponentSource::Path(path) => Component::from_file(&self.shared.engine, path),
        }
        .map_err(|e| format!("{e:#}"))
    }

    async fn load_inner(
        &self,
        package: LoadedPackage,
        host: Arc<dyn HostServices>,
    ) -> Result<Arc<WasmInstance>, ExtensionError> {
        let id = package.manifest.id.clone();
        let fail = |m: String| ExtensionError::new(id.as_str(), "load", m);
        let engine = Arc::clone(&self.shared);
        let source = package.component.clone();
        let component = {
            let this_engine = engine.engine.clone();
            tokio::task::spawn_blocking(move || match &source {
                ComponentSource::Bytes(bytes) => Component::new(&this_engine, bytes),
                ComponentSource::Path(path) => Component::from_file(&this_engine, path),
            })
            .await
            .map_err(|e| fail(e.to_string()))?
            .map_err(|e| fail(format!("{e:#}")))?
        };
        let exports_provider = component
            .get_export_index(None, "oino:extension/provider@1.0.0")
            .is_some();
        if package.manifest.kind == ExtensionKind::Provider && !exports_provider {
            return Err(fail(
                "kind `provider` requires the component to export oino:extension/provider".into(),
            ));
        }
        if component
            .get_export_index(None, "oino:extension/guest@1.0.0")
            .is_none()
        {
            return Err(fail(
                "component does not export oino:extension/guest@1.0.0".into(),
            ));
        }
        let pre = engine
            .linker
            .instantiate_pre(&component)
            .map_err(|e| fail(format!("{e:#}")))?;
        let config = self.options.config.clone();
        let ext_paths = ExtPaths::new(&config.agent_dir, &id);
        ext_paths
            .create()
            .map_err(|e| fail(format!("state dirs: {e}")))?;
        let preopen_list = paths::preopens(&ext_paths, &package.manifest.permissions, &config.cwd);
        let host: Arc<dyn HostServices> = match &self.hub {
            Some(hub) => hub.wrap(&id, host),
            None => host,
        };
        let shared = Arc::new(ExtShared {
            http: HttpService::new(config.http_proxy.clone(), config.http_idle_timeout),
            kv: crate::kv::KvStore::default(),
            timers: crate::timers::Timers::default(),
            processes: crate::process::ProcessService::default(),
            preopen_list,
            http_grants: HttpGrants::default(),
            last_cx: std::sync::Mutex::new(CallContext {
                cwd: config.cwd.clone(),
                ..CallContext::default()
            }),
            id: id.clone(),
            bundled: package.bundled,
            host,
            paths: ext_paths,
            config,
            manifest: package.manifest,
        });
        let instance = Arc::new(WasmInstance::new(shared, engine, pre, exports_provider));
        if let Some(hub) = &self.hub {
            hub.register_instance(&instance);
        }
        Ok(instance)
    }
}

impl ExtensionRuntime for WasmRuntime {
    fn load(
        &self,
        package: LoadedPackage,
        host: Arc<dyn HostServices>,
    ) -> BoxFut<'_, Result<Arc<dyn ExtensionInstance>, ExtensionError>> {
        Box::pin(async move {
            let instance = self.load_inner(package, host).await?;
            Ok(instance as Arc<dyn ExtensionInstance>)
        })
    }
}

#[allow(dead_code, reason = "used by benches and future cache warmers")]
impl WasmRuntime {
    /// Pre-compiles a component into the cache.
    pub(crate) fn precompile(&self, source: &ComponentSource) -> Result<(), String> {
        self.compile(source).map(|_| ())
    }
}
