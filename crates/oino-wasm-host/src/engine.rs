//! [`WasmRuntime`]: the wasmtime engine, epoch ticker, compile cache and linker, and the
//! [`ExtensionRuntime`] implementation. Engines are process-shared through [`EngineRegistry`]
//! (E-ENGINE: one engine, one ticker thread and one compiled-component cache per engine
//! configuration, no matter how many runtimes/services exist).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{
    Arc, Mutex, MutexGuard, OnceLock, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard,
};
use std::time::Duration;

use oino_extension_api::manifest::{ExtensionKind, Manifest};
use oino_extension_api::{
    BoxFut, CallContext, ComponentSource, ExtensionError, ExtensionInstance, ExtensionRuntime,
    HostServices, LoadedPackage,
};
use sha2::{Digest, Sha256};
use wasmtime::component::{Component, InstancePre, Linker};
use wasmtime::{Config, Engine, StoreLimits, StoreLimitsBuilder};

use crate::http::HttpService;
use crate::hub::LocalServices;
use crate::instance::WasmInstance;
use crate::paths::{self, ExtPaths};
use crate::paths::{is_agent_file, refresh_agent_view};
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

/// SHA-256 of a component source; the in-memory compile-cache key.
type ComponentKey = [u8; 32];

/// The wasmtime engine, linker, epoch ticker and compile caches shared by every
/// [`WasmRuntime`] of one engine configuration (E-ENGINE). Obtain one through
/// [`EngineRegistry`] (or [`WasmRuntime::engine`]); the registry keeps it alive for the
/// process so services created later reuse it instead of paying engine setup, a ticker
/// thread and recompilation again.
pub struct EngineShared {
    pub(crate) engine: Engine,
    pub(crate) linker: Linker<HostState>,
    stop: Arc<AtomicBool>,
    /// Compiled components by source digest (compiled once per process).
    components: RwLock<HashMap<ComponentKey, Arc<Component>>>,
    /// `instantiate_pre` results by source digest (linked once per process).
    pres: RwLock<HashMap<ComponentKey, InstancePre<HostState>>>,
    /// Components actually compiled (cache misses; diagnostic/test counter).
    compiles: AtomicU64,
    /// `instantiate_pre` calls (cache misses; diagnostic/test counter).
    pres_linked: AtomicU64,
}

impl std::fmt::Debug for EngineShared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EngineShared")
            .field("compiles", &self.compiles.load(Ordering::Relaxed))
            .finish_non_exhaustive()
    }
}

impl Drop for EngineShared {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(PoisonError::into_inner)
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write().unwrap_or_else(PoisonError::into_inner)
}

fn mutex_lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// SHA-256 of the component bytes (bundled bytes or the file on disk).
fn source_digest(source: &ComponentSource) -> Result<ComponentKey, String> {
    let mut hasher = Sha256::new();
    match source {
        ComponentSource::Bytes(bytes) => hasher.update(bytes),
        ComponentSource::Path(path) => {
            let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
            hasher.update(bytes);
        }
    }
    Ok(hasher.finalize().into())
}

/// Compiles `source` with `engine` (the uncached path).
fn compile(engine: &Engine, source: &ComponentSource) -> Result<Component, String> {
    match source {
        ComponentSource::Bytes(bytes) => Component::new(engine, bytes),
        ComponentSource::Path(path) => Component::from_file(engine, path),
    }
    .map_err(|e| format!("{e:#}"))
}

/// Cached value for `key`, cloned out so the read guard is dropped before any write
/// lock (a match on `read_lock(..).get(..)` keeps its guard through the arms and would
/// deadlock on the miss path).
fn cached<T: Clone>(map: &RwLock<HashMap<ComponentKey, T>>, key: &ComponentKey) -> Option<T> {
    read_lock(map).get(key).cloned()
}

impl EngineShared {
    /// Creates a fresh engine (component model + async, epoch interruption, optional cache),
    /// linker and epoch-ticker thread. Production code resolves the process-shared engine
    /// through [`EngineRegistry`] instead; this constructor exists for
    /// [`WasmRuntimeOptions::engine`] (test isolation).
    ///
    /// # Errors
    ///
    /// Engine configuration or linker setup failed.
    pub fn new(cache_dir: Option<&Path>) -> Result<Self, WasmHostError> {
        let err = |e: wasmtime::Error| WasmHostError::Engine(format!("{e:#}"));
        let mut config = Config::new();
        config
            .wasm_component_model(true)
            .wasm_component_model_async(true)
            .epoch_interruption(true);
        if let Some(dir) = cache_dir {
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
        REGISTRY_TICKERS_STARTED.fetch_add(1, Ordering::Relaxed);
        REGISTRY_ENGINES_CREATED.fetch_add(1, Ordering::Relaxed);
        Ok(Self {
            engine,
            linker,
            stop,
            components: RwLock::new(HashMap::new()),
            pres: RwLock::new(HashMap::new()),
            compiles: AtomicU64::new(0),
            pres_linked: AtomicU64::new(0),
        })
    }

    pub(crate) fn limits(manifest: &Manifest) -> StoreLimits {
        let bytes = usize::try_from(manifest.limits.memory_mib).unwrap_or(512) * 1024 * 1024;
        StoreLimitsBuilder::new()
            .memory_size(bytes)
            .table_elements(100_000)
            .instances(10)
            .build()
    }

    /// The wasmtime engine (shared per process for one engine configuration).
    #[must_use]
    pub fn engine(&self) -> &Engine {
        &self.engine
    }

    /// Components this engine actually compiled (compile-cache misses). N services loading
    /// the same packages leave this at one compile per distinct component.
    #[must_use]
    pub fn compiles(&self) -> u64 {
        self.compiles.load(Ordering::Relaxed)
    }

    /// `instantiate_pre` executions (pre-cache misses).
    #[must_use]
    pub fn pre_instantiations(&self) -> u64 {
        self.pres_linked.load(Ordering::Relaxed)
    }

    /// The compiled component and its `InstancePre` for `source`, compiling/linking at most
    /// once per distinct component per process (blocking; call from `spawn_blocking`).
    pub(crate) fn compiled(
        &self,
        source: &ComponentSource,
    ) -> Result<(Component, InstancePre<HostState>), String> {
        let key = source_digest(source)?;
        // Concurrent misses compile twice and the last insert wins (benign).
        let component = match cached(&self.components, &key) {
            Some(hit) => (*hit).clone(),
            None => self.compile_uncached(source, key)?,
        };
        let pre = match cached(&self.pres, &key) {
            Some(hit) => hit,
            None => self.link_uncached(&component, key)?,
        };
        Ok((component, pre))
    }

    /// The compile path of a [`EngineShared::compiled`] cache miss (under the write lock,
    /// after a second-chance lookup that may lose a benign race).
    fn compile_uncached(
        &self,
        source: &ComponentSource,
        key: ComponentKey,
    ) -> Result<Component, String> {
        let component = compile(&self.engine, source)?;
        self.compiles.fetch_add(1, Ordering::Relaxed);
        let mut map = write_lock(&self.components);
        if let Some(hit) = map.get(&key) {
            return Ok((**hit).clone());
        }
        map.insert(key, Arc::new(component.clone()));
        Ok(component)
    }

    /// The `instantiate_pre` path of a [`EngineShared::compiled`] cache miss.
    fn link_uncached(
        &self,
        component: &Component,
        key: ComponentKey,
    ) -> Result<InstancePre<HostState>, String> {
        let pre = self
            .linker
            .instantiate_pre(component)
            .map_err(|e| format!("{e:#}"))?;
        self.pres_linked.fetch_add(1, Ordering::Relaxed);
        let mut map = write_lock(&self.pres);
        if let Some(hit) = map.get(&key) {
            return Ok(hit.clone());
        }
        map.insert(key, pre.clone());
        Ok(pre)
    }
}

/// Identity of one engine configuration: everything that goes into the wasmtime `Config`
/// today is the cache dir plus a hash of the remaining engine-level settings (E-ENGINE;
/// risk: config drift silently splitting the registry — grow the hash whenever the
/// engine-relevant settings grow).
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
struct EngineKey {
    cache_dir: Option<PathBuf>,
    config_hash: u64,
}

/// Hash of every engine-relevant setting besides the cache dir.
fn engine_config_hash() -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    EPOCH_TICK.hash(&mut hasher);
    hasher.finish()
}

static REGISTRY_ENGINES: OnceLock<Mutex<HashMap<EngineKey, Arc<EngineShared>>>> = OnceLock::new();
static REGISTRY_HUBS: OnceLock<Mutex<HashMap<PathBuf, Arc<LocalServices>>>> = OnceLock::new();
static REGISTRY_ENGINES_CREATED: AtomicU64 = AtomicU64::new(0);
static REGISTRY_TICKERS_STARTED: AtomicU64 = AtomicU64::new(0);

/// Process-wide engine and hub registry (E-ENGINE): one wasmtime engine (with one epoch
/// ticker thread and the in-memory component/`InstancePre` caches) per engine
/// configuration, and one [`LocalServices`] hub per agent dir, shared by every
/// [`WasmRuntime`] of the process — so every `CliLoader`, the ACP factory and a multi-cwd
/// host share one engine instead of multiplying them per service.
pub struct EngineRegistry;

impl EngineRegistry {
    /// Resolves the shared engine for `options`, creating it on first use. An explicit
    /// [`WasmRuntimeOptions::engine`] bypasses this lookup (test isolation).
    pub(crate) fn resolve(
        options: &WasmRuntimeOptions,
    ) -> Result<Arc<EngineShared>, WasmHostError> {
        let key = EngineKey {
            cache_dir: options.cache_dir.clone(),
            config_hash: engine_config_hash(),
        };
        let mut engines = mutex_lock(REGISTRY_ENGINES.get_or_init(Default::default));
        if let Some(shared) = engines.get(&key) {
            return Ok(Arc::clone(shared));
        }
        let shared = Arc::new(EngineShared::new(key.cache_dir.as_deref())?);
        engines.insert(key, Arc::clone(&shared));
        tracing::debug!(
            engines = engines.len(),
            "oino-wasm-host: created engine #{} (registry splits only on cache dir or engine config)",
            engines.len()
        );
        Ok(shared)
    }

    /// The process-wide [`LocalServices`] hub of `agent_dir` (bus, secrets, tasks shared
    /// across every service of one agent dir).
    pub(crate) fn hub(agent_dir: &Path) -> Arc<LocalServices> {
        let mut hubs = mutex_lock(REGISTRY_HUBS.get_or_init(Default::default));
        if let Some(hub) = hubs.get(agent_dir) {
            return Arc::clone(hub);
        }
        let hub = Arc::new(LocalServices::new(agent_dir));
        hubs.insert(agent_dir.to_path_buf(), Arc::clone(&hub));
        hub
    }

    /// Engines created by this process (diagnostics; regression counter for E-ENGINE).
    #[must_use]
    pub fn engines_created() -> u64 {
        REGISTRY_ENGINES_CREATED.load(Ordering::Relaxed)
    }

    /// Epoch-ticker threads started by this process (one per engine; E-ENGINE regression
    /// counter — a process must not start one ticker per service).
    #[must_use]
    pub fn ticker_threads_started() -> u64 {
        REGISTRY_TICKERS_STARTED.load(Ordering::Relaxed)
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
    /// Use this engine instead of the process-shared one (test isolation). The hub lookup
    /// is unaffected; pass `local_services: false` to isolate that too.
    pub engine: Option<Arc<EngineShared>>,
}

impl WasmRuntimeOptions {
    /// Defaults for `config` (cache in `<agent>/cache/wasmtime`, local services on).
    #[must_use]
    pub fn new(config: HostConfig) -> Self {
        Self {
            cache_dir: Some(config.agent_dir.join("cache").join("wasmtime")),
            config,
            local_services: true,
            engine: None,
        }
    }
}

/// The wasmtime component runtime.
pub struct WasmRuntime {
    shared: Arc<EngineShared>,
    options: WasmRuntimeOptions,
    hub: Option<Arc<LocalServices>>,
    /// Holds the agent-dir watcher that refreshes the `/agent` preopen view (dropped with the
    /// runtime, which stops it).
    _agent_watch: Option<notify::RecommendedWatcher>,
}

impl std::fmt::Debug for WasmRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WasmRuntime")
            .field("options", &self.options)
            .finish_non_exhaustive()
    }
}

/// Watches `<agent>` so the `/agent` preopen view follows edits to `mcp.json` (the copies are
/// refreshed when an instance is created too, so a watcher failure only costs freshness).
fn watch_agent_dir(agent_dir: &Path) -> Option<notify::RecommendedWatcher> {
    use notify::Watcher as _;
    let dir = agent_dir.to_path_buf();
    let mut watcher =
        notify::recommended_watcher(move |event: Result<notify::Event, notify::Error>| {
            let Ok(event) = event else { return };
            if event.paths.iter().any(|path| is_agent_file(path)) {
                let _ = refresh_agent_view(&dir);
            }
        })
        .ok()?;
    watcher
        .watch(agent_dir, notify::RecursiveMode::NonRecursive)
        .ok()?;
    Some(watcher)
}

impl WasmRuntime {
    /// Creates the runtime around the process-shared engine (E-ENGINE): resolves — or
    /// creates — the [`EngineShared`] of this engine configuration (engine, linker, epoch
    /// ticker, compile caches) and the [`LocalServices`] hub of this agent dir, so several
    /// runtimes (the CLI loader, the ACP factory, a multi-cwd host) share one engine and one
    /// ticker thread per process.
    ///
    /// # Errors
    ///
    /// Engine configuration or linker setup failed.
    pub fn new(options: WasmRuntimeOptions) -> Result<Self, WasmHostError> {
        let shared = match &options.engine {
            Some(shared) => Arc::clone(shared),
            None => EngineRegistry::resolve(&options)?,
        };
        let hub = options
            .local_services
            .then(|| EngineRegistry::hub(&options.config.agent_dir));
        let agent_watch = watch_agent_dir(&options.config.agent_dir);
        Ok(Self {
            shared,
            options,
            hub,
            _agent_watch: agent_watch,
        })
    }

    /// The shared engine (with its compile caches and counters) this runtime runs on.
    #[must_use]
    pub fn engine(&self) -> &Arc<EngineShared> {
        &self.shared
    }

    /// The local services hub, when enabled.
    #[must_use]
    pub fn local_services(&self) -> Option<&Arc<LocalServices>> {
        self.hub.as_ref()
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
        let (component, pre) = {
            let blocking_engine = Arc::clone(&engine);
            tokio::task::spawn_blocking(move || blocking_engine.compiled(&source))
                .await
                .map_err(|e| fail(e.to_string()))?
                .map_err(|e| ExtensionError::new(id.as_str(), "load", e))?
        };
        let wit = oino_extension_api::manifest::WIT_VERSION;
        let exports_provider = component
            .get_export_index(None, &format!("oino:extension/provider@{wit}"))
            .is_some();
        if package.manifest.kind == ExtensionKind::Provider && !exports_provider {
            return Err(fail(
                "kind `provider` requires the component to export oino:extension/provider".into(),
            ));
        }
        if component
            .get_export_index(None, &format!("oino:extension/guest@{wit}"))
            .is_none()
        {
            return Err(fail(format!(
                "component does not export oino:extension/guest@{wit}: a 1.x extension must be \
                 rebuilt against oino:extension@{wit}"
            )));
        }
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
            last_cx: Mutex::new(CallContext {
                cwd: config.cwd.clone(),
                ..CallContext::default()
            }),
            dialogs_open: std::sync::atomic::AtomicUsize::new(0),
            id: id.clone(),
            bundled: package.bundled,
            package_root: package.root,
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
    /// Pre-compiles a component into the engine caches (on disk and in memory).
    pub(crate) fn precompile(&self, source: &ComponentSource) -> Result<(), String> {
        self.shared.compiled(source).map(|_| ())
    }
}
