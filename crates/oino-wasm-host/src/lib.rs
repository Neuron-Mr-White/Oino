#![doc = r#"WASM component-model host for Oino extensions (`oino:extension@0.1.0`).

A shared [`WasmHostEngine`] owns a wasmtime `Engine` (epoch interruption, on-disk
compiled-component cache). [`WasmExtension`] is one lazily-instantiated
component per extension: calls are serialized through a mutex, capped by
per-call epoch deadlines and a store memory limit, and capability-gated through
the manifest [`ExtensionPermissions`]. [`WasmStreamProvider`] adapts a
`provider-extension` component to [`oino_agent_loop::StreamProvider`] by pulling
`response.next()` batches onto a dedicated task.

This crate stays free of `oino-app`/`oino-tui` deps; app-owned surfaces are
injected through [`SecretResolver`], [`HostUi`], and [`HostSession`].
"#]
#![forbid(unsafe_code)]

use oino_agent_loop::{AbortSignal, BoxFuture, LoopResult, StreamProvider};
use oino_extension_core::ExtensionPermissions;
use oino_types::{AssistantStreamEvent, OinoId, StopReason};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
    time::Duration,
};
use thiserror::Error;
use tokio::sync::Mutex;
use wasmtime::{
    component::{Component, Linker, ResourceTable},
    Config, Engine, Store, StoreLimits, StoreLimitsBuilder,
};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView};
use wasmtime_wasi_io::IoView;

mod bindings {
    wasmtime::component::bindgen!({
        world: "extension",
        path: "../../wit",
        exports: { default: async },
    });
}

mod provider_bindings {
    wasmtime::component::bindgen!({
        world: "provider-extension",
        path: "../../wit",
        exports: { default: async },
    });
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(Debug, Error)]
pub enum WasmHostError {
    #[error("engine setup failed: {0}")]
    Engine(#[from] anyhow::Error),
    #[error("component failed to instantiate: {0}")]
    Instantiate(String),
    #[error("guest returned error: {0}")]
    Guest(String),
    #[error("capability denied: {0}")]
    Capability(String),
    #[error("call deadline exceeded")]
    Deadline,
    #[error("extension is unhealthy after {0} failed reinstantiations")]
    Unhealthy(u32),
    #[error("serialization failed: {0}")]
    Serialization(String),
    #[error("io failed: {0}")]
    Io(#[from] std::io::Error),
}

pub type WasmResult<T> = Result<T, WasmHostError>;

impl Drop for WasmHostEngine {
    fn drop(&mut self) {
        self.ticker_stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
}

impl From<WasmHostError> for oino_agent_loop::LoopError {
    fn from(value: WasmHostError) -> Self {
        oino_agent_loop::LoopError::Stream(value.to_string())
    }
}

// ---------------------------------------------------------------------------
// Host-injected surfaces
// ---------------------------------------------------------------------------

/// Resolves declared secret names to values. Never falls back to env.
pub trait SecretResolver: Send + Sync {
    fn secret(&self, name: &str) -> Option<String>;
}

/// UI surface for `host.notify` / `host.confirm`.
pub trait HostUi: Send + Sync {
    fn notify(&self, level: u8, text: &str);
    fn confirm(&self, title: &str, body: &str) -> bool;
}

/// Session surface for `host.session-append` / `host.session-entries`.
pub trait HostSession: Send + Sync {
    fn append(&self, kind: &str, payload: &str) -> Result<(), String>;
    /// Serialized `SessionEntry` list (active branch, or all entries when
    /// `all_branches`).
    fn entries(&self, all_branches: bool) -> Result<String, String> {
        let _ = all_branches;
        Err("session entries unavailable".into())
    }
}

/// Non-streaming model completion for `host.complete`. Implemented by the
/// app against its `ProviderRouter` (runs on its own runtime; the guest call
/// stays synchronous WIT-side).
pub trait HostModel: Send + Sync {
    /// request/response are the JSON contract documented in `extension.wit`.
    fn complete(&self, request: &str) -> Result<String, String>;
}

/// Never-granted `HostModel` (capability not declared or app did not wire a
/// router).
pub struct NoopModel;
impl HostModel for NoopModel {
    fn complete(&self, _request: &str) -> Result<String, String> {
        Err("model completion unavailable".into())
    }
}

/// `host.tool-progress` sink.
pub type ProgressSink = Arc<dyn Fn(&str, &str, Option<String>) + Send + Sync>;

/// Default no-op implementations usable in tests/headless runs.
pub struct NoopSecrets;
impl SecretResolver for NoopSecrets {
    fn secret(&self, _name: &str) -> Option<String> {
        None
    }
}
pub struct NoopUi;
impl HostUi for NoopUi {
    fn notify(&self, _level: u8, _text: &str) {}
    fn confirm(&self, _title: &str, _body: &str) -> bool {
        false
    }
}
pub struct NoopSession;
impl HostSession for NoopSession {
    fn append(&self, _kind: &str, _payload: &str) -> Result<(), String> {
        Ok(())
    }
    fn entries(&self, _all_branches: bool) -> Result<String, String> {
        Ok("[]".into())
    }
}

/// Capability context built from a manifest plus injected surfaces.
#[derive(Clone)]
pub struct HostCaps {
    pub permissions: ExtensionPermissions,
    pub secrets: Arc<dyn SecretResolver>,
    pub ui: Arc<dyn HostUi>,
    pub session: Arc<dyn HostSession>,
    pub progress: ProgressSink,
    pub kv_dir: PathBuf,
    /// `host.complete` backing (router); gated by `host_capabilities`
    /// containing `"model.complete"`.
    pub model: Arc<dyn HostModel>,
}

impl Default for HostCaps {
    fn default() -> Self {
        Self {
            permissions: ExtensionPermissions::default(),
            secrets: Arc::new(NoopSecrets),
            ui: Arc::new(NoopUi),
            session: Arc::new(NoopSession),
            progress: Arc::new(|_, _, _| {}),
            kv_dir: PathBuf::new(),
            model: Arc::new(NoopModel),
        }
    }
}

// ---------------------------------------------------------------------------
// Engine + compiled-component cache
// ---------------------------------------------------------------------------

/// Shared wasmtime engine. Epoch interruption ticks every `tick` (default
/// 10 ms) on a background task; compiled components are cached on disk under
/// `cache_dir` keyed by component sha256 plus wasmtime version.
pub struct WasmHostEngine {
    engine: Engine,
    cache_dir: PathBuf,
    cache_dir_marker: std::marker::PhantomData<()>,
    tick: Duration,
    ticker_stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    _ticker: std::thread::JoinHandle<()>,
}

impl WasmHostEngine {
    pub fn new(cache_dir: impl Into<PathBuf>) -> WasmResult<Arc<Self>> {
        Self::with_tick(cache_dir, Duration::from_millis(10))
    }

    pub fn with_tick(cache_dir: impl Into<PathBuf>, tick: Duration) -> WasmResult<Arc<Self>> {
        let cache_dir = cache_dir.into();
        std::fs::create_dir_all(&cache_dir)?;
        let mut config = Config::new();
        config.wasm_component_model(true).epoch_interruption(true);
        let mut cache_config = wasmtime::CacheConfig::new();
        cache_config.with_directory(&cache_dir);
        let cache = wasmtime::Cache::new(cache_config)
            .map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))?;
        config.cache(Some(cache));
        let engine =
            Engine::new(&config).map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))?;
        // A dedicated OS thread (not a tokio task): during a blocking guest
        // call the executor may be starved, which would also starve the
        // ticker and prevent deadline interruption.
        let ticker_engine = engine.clone();
        let ticker_stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let ticker_stop_thread = std::sync::Arc::clone(&ticker_stop);
        let ticker = std::thread::spawn(move || {
            while !ticker_stop_thread.load(std::sync::atomic::Ordering::Relaxed) {
                std::thread::sleep(tick);
                ticker_engine.increment_epoch();
            }
        });
        Ok(Arc::new(Self {
            engine,
            cache_dir,
            cache_dir_marker: std::marker::PhantomData,
            tick,
            ticker_stop,
            _ticker: ticker,
        }))
    }

    /// Directory where wasmtime's built-in cache persists compiled artifacts.
    #[must_use]
    pub fn cache_dir(&self) -> &std::path::Path {
        &self.cache_dir
    }

    /// Load or compile a component. Compilation results are cached by the
    /// engine's built-in `cache` feature (keyed internally by content +
    /// wasmtime version) under `cache_dir`.
    pub fn component(&self, bytes: &[u8]) -> WasmResult<Component> {
        Component::new(&self.engine, bytes)
            .map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))
    }

    /// Epoch ticks per wall-clock duration, rounded up.
    fn deadline_ticks(&self, duration: Duration) -> u64 {
        let tick_ms = self.tick.as_millis().max(1) as u64;
        (duration.as_millis() as u64).div_ceil(tick_ms).max(1)
    }
}

fn const_hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write;
        let _ = write!(out, "{byte:02x}");
    }
    out
}

// ---------------------------------------------------------------------------
// Store state + capability gating
// ---------------------------------------------------------------------------

/// Per-call wall-clock deadlines.
#[derive(Debug, Clone)]
pub struct CallLimits {
    pub hook: Duration,
    pub command: Duration,
    pub tool: Duration,
    pub provider_next: Duration,
    pub memory_bytes: usize,
    pub max_restarts: u32,
}

impl Default for CallLimits {
    fn default() -> Self {
        Self {
            hook: Duration::from_secs(2),
            command: Duration::from_secs(30),
            tool: Duration::from_secs(600),
            provider_next: Duration::from_secs(120),
            memory_bytes: 256 * 1024 * 1024,
            max_restarts: 3,
        }
    }
}

/// HTTP gating hooks: `send_request` allows only exact-host matches from
/// `permissions.network.hosts` (or any host when `raw_network` is true).
struct GatedHooks {
    hosts: BTreeSet<String>,
}

impl WasiHttpHooks for GatedHooks {
    fn send_request(
        &mut self,
        request: http::Request<wasmtime_wasi_http::WasiBody>,
        options: Option<wasmtime_wasi_http::RequestOptions>,
        _fut: Box<dyn std::future::Future<Output = Result<(), wasmtime_wasi_http::Error>> + Send>,
    ) -> Box<
        dyn std::future::Future<
                Output = Result<
                    (
                        http::Response<wasmtime_wasi_http::WasiBody>,
                        Box<
                            dyn std::future::Future<Output = Result<(), wasmtime_wasi_http::Error>>
                                + Send,
                        >,
                    ),
                    wasmtime_wasi_http::Error,
                >,
            > + Send,
    > {
        let host = request
            .uri()
            .host()
            .map(str::to_ascii_lowercase)
            .unwrap_or_default();
        // `raw_network` never grants wasi:http; only an exact declared host
        // match does. Raw sockets are denied entirely in `build_wasi_ctx`.
        if !self.hosts.contains(&host) {
            return Box::new(async move { Err(wasmtime_wasi_http::Error::HttpRequestDenied) });
        }
        Box::new(async move {
            use http_body_util::BodyExt;
            let (res, io) = wasmtime_wasi_http::default_send_request(request, options).await?;
            Ok((
                res.map(BodyExt::boxed_unsync),
                Box::new(io)
                    as Box<
                        dyn std::future::Future<Output = Result<(), wasmtime_wasi_http::Error>>
                            + Send,
                    >,
            ))
        })
    }
}

struct HostState {
    table: ResourceTable,
    wasi_ctx: WasiCtx,
    http_ctx: WasiHttpCtx,
    hooks: GatedHooks,
    caps: HostCaps,
    limits: StoreLimits,
}

impl IoView for HostState {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

impl WasiView for HostState {
    fn ctx(&mut self) -> wasmtime_wasi::WasiCtxView<'_> {
        wasmtime_wasi::WasiCtxView {
            ctx: &mut self.wasi_ctx,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for HostState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http_ctx,
            table: &mut self.table,
            hooks: &mut self.hooks,
        }
    }
}

fn host_capability_error(what: &str) -> String {
    format!("capability denied: {what}")
}

impl bindings::oino::extension::host::Host for HostState {
    fn log(&mut self, level: u8, msg: String) {
        let _ = level;
        tracing::debug!(target: "oino-wasm", "{msg}");
    }

    fn secret(&mut self, name: String) -> Result<String, String> {
        if !self.caps.permissions.secrets.contains(&name) {
            return Err(host_capability_error(&format!(
                "secret `{name}` not declared"
            )));
        }
        self.caps
            .secrets
            .secret(&name)
            .ok_or_else(|| format!("secret `{name}` unavailable"))
    }

    fn kv_get(&mut self, key: String) -> Option<Vec<u8>> {
        if self.caps.permissions.session_persistence.is_empty() {
            return None;
        }
        let path = self.kv_path(&key);
        std::fs::read(path).ok()
    }

    fn kv_set(&mut self, key: String, value: Vec<u8>) -> Result<(), String> {
        if self.caps.permissions.session_persistence.is_empty() {
            return Err(host_capability_error("session_persistence not declared"));
        }
        let path = self.kv_path(&key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        std::fs::write(path, value).map_err(|err| err.to_string())
    }

    fn session_append(&mut self, kind: String, payload: String) -> Result<(), String> {
        self.caps.session.append(&kind, &payload)
    }

    fn session_entries(&mut self, all_branches: bool) -> Result<String, String> {
        if self.caps.permissions.session_persistence.is_empty() {
            return Err(host_capability_error("session_persistence not declared"));
        }
        self.caps.session.entries(all_branches)
    }

    fn complete(&mut self, request: String) -> Result<String, String> {
        if !self
            .caps
            .permissions
            .host_capabilities
            .iter()
            .any(|cap| cap == "model.complete")
        {
            return Err(host_capability_error(
                "host capability `model.complete` not declared",
            ));
        }
        self.caps.model.complete(&request)
    }

    fn notify(&mut self, level: u8, text: String) {
        self.caps.ui.notify(level, &text);
    }

    fn confirm(&mut self, title: String, body: String) -> bool {
        self.caps.ui.confirm(&title, &body)
    }

    fn exec(
        &mut self,
        cmd: String,
        args: Vec<String>,
        cwd: Option<String>,
    ) -> Result<bindings::oino::extension::host::ExecOutput, String> {
        let permission = &self.caps.permissions.shell_process;
        if !permission.allowed || !permission.commands.iter().any(|allowed| allowed == &cmd) {
            return Err(host_capability_error(&format!("exec `{cmd}` not allowed")));
        }
        if let Some(cwd) = &cwd {
            // cwd must resolve inside a declared preopen (paths are absolute).
            let cwd_path = std::path::Path::new(cwd);
            let allowed = self.caps.permissions.filesystem.iter().any(|perm| {
                perm.paths.iter().any(|base| {
                    let base = std::path::Path::new(base);
                    cwd_path == base || cwd_path.starts_with(base)
                })
            });
            if !allowed {
                return Err(host_capability_error(&format!(
                    "exec cwd `{cwd}` outside declared filesystem permissions"
                )));
            }
        }
        let mut command = std::process::Command::new(&cmd);
        command.args(&args);
        // Never inherit the host environment: it may hold provider API keys.
        command.env_clear();
        for name in ["PATH", "HOME"] {
            if let Ok(value) = std::env::var(name) {
                command.env(name, value);
            }
        }
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        let output = command.output().map_err(|err| err.to_string())?;
        Ok(bindings::oino::extension::host::ExecOutput {
            status: output.status.code().unwrap_or(-1),
            stdout: output.stdout,
            stderr: output.stderr,
        })
    }

    fn tool_progress(&mut self, call_id: String, message: String, details: Option<String>) {
        (self.caps.progress)(&call_id, &message, details);
    }
}

impl HostState {
    fn kv_path(&self, key: &str) -> PathBuf {
        use sha2::Digest;
        let hash = sha2::Sha256::digest(key.as_bytes());
        self.caps.kv_dir.join(const_hex(hash.as_slice()))
    }
}

fn build_wasi_ctx(caps: &HostCaps) -> WasmResult<WasiCtx> {
    let mut builder = WasiCtxBuilder::new();
    builder.inherit_stderr();
    // Raw sockets and DNS are reserved for a later version; `raw_network` in
    // the manifest is currently not granted to anything.
    builder
        .allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false);
    for permission in &caps.permissions.filesystem {
        use oino_extension_core::FilesystemAccess;
        let perms = match permission.access {
            FilesystemAccess::Read => wasmtime_wasi::FsPerms::ReadOnly,
            FilesystemAccess::Write | FilesystemAccess::ReadWrite => {
                wasmtime_wasi::FsPerms::ReadWrite
            }
        };
        for path in &permission.paths {
            if matches!(perms, wasmtime_wasi::FsPerms::ReadWrite) {
                // Read-write preopens must exist; extension-writable roots are
                // created on demand (e.g. `.oino/ralph`).
                let _ = std::fs::create_dir_all(path);
            }
            builder
                .preopened_dir(path, path, perms)
                .map_err(|err| WasmHostError::Instantiate(format!("preopen {path}: {err}")))?;
        }
    }
    Ok(builder.build())
}

fn new_store(engine: &Engine, caps: HostCaps, limits: &CallLimits) -> Store<HostState> {
    let state = HostState {
        table: ResourceTable::new(),
        wasi_ctx: build_wasi_ctx(&caps).unwrap_or_else(|_| WasiCtx::builder().build()),
        http_ctx: WasiHttpCtx::new(),
        hooks: GatedHooks {
            hosts: caps
                .permissions
                .network
                .hosts
                .iter()
                .map(|host| host.to_ascii_lowercase())
                .collect(),
        },
        caps,
        limits: StoreLimitsBuilder::new()
            .memory_size(limits.memory_bytes)
            .build(),
    };
    let mut store = Store::new(engine, state);
    store.limiter(|state| &mut state.limits);
    // No deadline until a scoped call installs one; instantiation itself must
    // not be interruptible or every first call traps immediately.
    store.set_epoch_deadline(u64::MAX / 4);
    store
}

// ---------------------------------------------------------------------------
// WasmExtension — one lazily-instantiated component per extension
// ---------------------------------------------------------------------------

struct GuestInstance {
    instance: bindings::Extension,
    store: Store<HostState>,
}

struct ProviderInstance {
    instance: provider_bindings::ProviderExtension,
    store: Store<HostState>,
}

enum ExtensionKind {
    Guest(GuestInstance),
    Provider(ProviderInstance),
}

struct Slot {
    instance: Option<ExtensionKind>,
    restarts: u32,
    unhealthy: bool,
}

/// One extension component: serialized calls, epoch deadlines, health
/// tracking, capability gating.
pub struct WasmExtension {
    host: Arc<WasmHostEngine>,
    component: Component,
    caps: HostCaps,
    limits: CallLimits,
    provider: bool,
    slot: Mutex<Slot>,
}

impl WasmExtension {
    pub fn new(
        host: Arc<WasmHostEngine>,
        bytes: &[u8],
        caps: HostCaps,
        limits: CallLimits,
        provider: bool,
    ) -> WasmResult<Self> {
        let component = host.component(bytes)?;
        Ok(Self {
            host,
            component,
            caps,
            limits,
            provider,
            slot: Mutex::new(Slot {
                instance: None,
                restarts: 0,
                unhealthy: false,
            }),
        })
    }

    pub async fn is_healthy(&self) -> bool {
        !self.slot.lock().await.unhealthy
    }

    async fn ensure(&self, slot: &mut Slot) -> WasmResult<()> {
        if slot.unhealthy {
            return Err(WasmHostError::Unhealthy(slot.restarts));
        }
        if slot.instance.is_none() {
            slot.instance = Some(self.instantiate().await?);
        }
        Ok(())
    }

    async fn instantiate(&self) -> WasmResult<ExtensionKind> {
        let mut store = new_store(&self.host.engine, self.caps.clone(), &self.limits);
        let mut linker = Linker::new(&self.host.engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)
            .map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
            .map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))?;
        bindings::oino::extension::host::add_to_linker::<_, wasmtime::component::HasSelf<HostState>>(
            &mut linker,
            |state: &mut HostState| state,
        )
        .map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))?;
        if self.provider {
            let instance = provider_bindings::ProviderExtension::instantiate_async(
                &mut store,
                &self.component,
                &linker,
            )
            .await
            .map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))?;
            Ok(ExtensionKind::Provider(ProviderInstance {
                instance,
                store,
            }))
        } else {
            let instance =
                bindings::Extension::instantiate_async(&mut store, &self.component, &linker)
                    .await
                    .map_err(|err| WasmHostError::Instantiate(format!("{err:#}")))?;
            Ok(ExtensionKind::Guest(GuestInstance { instance, store }))
        }
    }

    async fn failed(&self, slot: &mut Slot) {
        slot.instance = None;
        slot.restarts += 1;
        if slot.restarts > self.limits.max_restarts {
            slot.unhealthy = true;
        }
    }

    fn deadline(&self, duration: Duration) -> u64 {
        self.host.deadline_ticks(duration)
    }

    /// `init(config)` → registration.
    pub async fn init(&self, config: Value) -> WasmResult<Value> {
        let mut slot = self.slot.lock().await;
        self.ensure(&mut slot).await?;
        let config = serde_json::to_string(&config)
            .map_err(|err| WasmHostError::Serialization(err.to_string()))?;
        let result = match slot.instance.as_mut() {
            Some(ExtensionKind::Guest(GuestInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.hook));
                instance
                    .oino_extension_guest()
                    .call_init(store, &config)
                    .await
                    .map(|inner| {
                        inner.map(|r| {
                            (
                                r.tools.len(),
                                r.commands.len(),
                                r.hooks.clone(),
                                r.providers.clone(),
                                tools_json(&r),
                                commands_json(&r),
                            )
                        })
                    })
            }
            Some(ExtensionKind::Provider(ProviderInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.hook));
                instance
                    .oino_extension_guest()
                    .call_init(store, &config)
                    .await
                    .map(|inner| {
                        inner.map(|r| {
                            (
                                r.tools.len(),
                                r.commands.len(),
                                r.hooks.clone(),
                                r.providers.clone(),
                                prov_tools_json(&r),
                                prov_commands_json(&r),
                            )
                        })
                    })
            }
            None => unreachable!("instance ensured"),
        };
        match result {
            Ok(Ok((_tools, _commands, hooks, providers, tools, commands))) => {
                Ok(serde_json::json!({
                    "tools": tools,
                    "commands": commands,
                    "hooks": hooks,
                    "providers": providers,
                }))
            }
            Ok(Err(err)) => Err(WasmHostError::Guest(err)),
            Err(err) => {
                self.failed(&mut slot).await;
                Err(classify_error(err))
            }
        }
    }

    /// `handle-event(kind, payload)` → JSON result.
    pub async fn handle_event(&self, kind: &str, payload: Value) -> WasmResult<Value> {
        let payload_str = serde_json::to_string(&payload)
            .map_err(|err| WasmHostError::Serialization(err.to_string()))?;
        let mut slot = self.slot.lock().await;
        self.ensure(&mut slot).await?;
        let result = match slot.instance.as_mut() {
            Some(ExtensionKind::Guest(GuestInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.hook));
                instance
                    .oino_extension_guest()
                    .call_handle_event(store, kind, &payload_str)
                    .await
            }
            Some(ExtensionKind::Provider(ProviderInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.hook));
                instance
                    .oino_extension_guest()
                    .call_handle_event(store, kind, &payload_str)
                    .await
            }
            None => unreachable!("instance ensured"),
        };
        match result {
            Ok(Ok(json)) => serde_json::from_str(&json)
                .map_err(|err| WasmHostError::Serialization(err.to_string())),
            Ok(Err(err)) => Err(WasmHostError::Guest(err)),
            Err(err) => {
                self.failed(&mut slot).await;
                Err(classify_error(err))
            }
        }
    }

    /// `run-command(name, args)` → JSON result.
    pub async fn run_command(&self, name: &str, args: &str) -> WasmResult<Value> {
        let mut slot = self.slot.lock().await;
        self.ensure(&mut slot).await?;
        let result = match slot.instance.as_mut() {
            Some(ExtensionKind::Guest(GuestInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.command));
                instance
                    .oino_extension_guest()
                    .call_run_command(store, name, args)
                    .await
            }
            Some(ExtensionKind::Provider(ProviderInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.command));
                instance
                    .oino_extension_guest()
                    .call_run_command(store, name, args)
                    .await
            }
            None => unreachable!("instance ensured"),
        };
        match result {
            Ok(Ok(json)) => serde_json::from_str(&json)
                .map_err(|err| WasmHostError::Serialization(err.to_string())),
            Ok(Err(err)) => Err(WasmHostError::Guest(err)),
            Err(err) => {
                self.failed(&mut slot).await;
                Err(classify_error(err))
            }
        }
    }

    /// `run-tool(name, call-id, args)` → mapped to a loop [`ToolResult`]-shaped value.
    pub async fn run_tool(
        &self,
        name: &str,
        call_id: OinoId,
        args: Value,
    ) -> WasmResult<GuestToolOutput> {
        let args_str = serde_json::to_string(&args)
            .map_err(|err| WasmHostError::Serialization(err.to_string()))?;
        let mut slot = self.slot.lock().await;
        self.ensure(&mut slot).await?;
        let result = match slot.instance.as_mut() {
            Some(ExtensionKind::Guest(GuestInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.tool));
                instance
                    .oino_extension_guest()
                    .call_run_tool(store, name, &call_id.to_string(), &args_str)
                    .await
                    .map(|inner| inner.map(GuestToolOutput::from_wit))
            }
            Some(ExtensionKind::Provider(ProviderInstance { instance, store })) => {
                store.set_epoch_deadline(self.deadline(self.limits.tool));
                instance
                    .oino_extension_guest()
                    .call_run_tool(store, name, &call_id.to_string(), &args_str)
                    .await
                    .map(|inner| inner.map(GuestToolOutput::from_wit_prov))
            }
            None => unreachable!("instance ensured"),
        };
        match result {
            Ok(Ok(output)) => Ok(output),
            Ok(Err(err)) => Err(WasmHostError::Guest(err)),
            Err(err) => {
                self.failed(&mut slot).await;
                Err(classify_error(err))
            }
        }
    }

    /// Provider path: `stream(request)` returns a pull handle resolved through
    /// [`WasmStreamProvider`]. Internal — use `stream_events` for the loop path.
    pub(crate) fn provider(&self) -> bool {
        self.provider
    }
}

fn tools_json(
    registration: &bindings::exports::oino::extension::guest::Registration,
) -> Vec<Value> {
    registration
        .tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()),
            })
        })
        .collect()
}

fn commands_json(
    registration: &bindings::exports::oino::extension::guest::Registration,
) -> Vec<Value> {
    registration
        .commands
        .iter()
        .map(|decl| serde_json::json!({"name": decl.name, "description": decl.description}))
        .collect()
}

fn prov_tools_json(
    registration: &provider_bindings::exports::oino::extension::guest::Registration,
) -> Vec<Value> {
    registration
        .tools
        .iter()
        .map(|tool| {
            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "input_schema": tool.input_schema.as_deref().and_then(|s| serde_json::from_str::<Value>(s).ok()),
            })
        })
        .collect()
}

fn prov_commands_json(
    registration: &provider_bindings::exports::oino::extension::guest::Registration,
) -> Vec<Value> {
    registration
        .commands
        .iter()
        .map(|decl| serde_json::json!({"name": decl.name, "description": decl.description}))
        .collect()
}

/// Tool output converted from the WIT `tool-output` record.
#[derive(Debug)]
pub struct GuestToolOutput {
    pub content: Vec<oino_types::ContentBlock>,
    pub is_error: bool,
    pub details: Option<Value>,
    pub added_tool_names: Vec<String>,
    pub terminate: bool,
}

impl GuestToolOutput {
    /// Map a guest tool output to the agent-loop [`ToolResult`].
    pub fn into_tool_result(self, call: &oino_agent_loop::ToolCall) -> oino_agent_loop::ToolResult {
        oino_agent_loop::ToolResult {
            call_id: call.id,
            tool_name: call.name.clone(),
            content: self.content,
            is_error: self.is_error,
            terminate: self.terminate,
            details: self.details,
            added_tool_names: self.added_tool_names,
        }
    }

    fn from_wit(output: bindings::exports::oino::extension::guest::ToolOutput) -> Self {
        let content = output
            .content
            .into_iter()
            .map(content_block_from_wit)
            .collect();
        Self {
            content,
            is_error: output.is_error,
            details: output
                .details
                .and_then(|json| serde_json::from_str(&json).ok()),
            added_tool_names: output.added_tool_names,
            terminate: output.terminate,
        }
    }
}

impl GuestToolOutput {
    fn from_wit_prov(
        output: provider_bindings::exports::oino::extension::guest::ToolOutput,
    ) -> Self {
        use provider_bindings::exports::oino::extension::guest::ContentBlock as Wit;
        let content = output
            .content
            .into_iter()
            .map(|block| match block {
                Wit::Text(text) => oino_types::ContentBlock::Text {
                    text,
                    signature: None,
                },
                Wit::Thinking(text) => oino_types::ContentBlock::Thinking {
                    text,
                    signature: None,
                    provider_data: None,
                    redacted: false,
                },
                Wit::ToolCall(json) | Wit::Image(json) | Wit::Custom(json) => {
                    serde_json::from_str(&json).unwrap_or(oino_types::ContentBlock::Text {
                        text: json,
                        signature: None,
                    })
                }
            })
            .collect();
        Self {
            content,
            is_error: output.is_error,
            details: output
                .details
                .and_then(|json| serde_json::from_str(&json).ok()),
            added_tool_names: output.added_tool_names,
            terminate: output.terminate,
        }
    }
}

fn content_block_from_wit(
    block: bindings::exports::oino::extension::guest::ContentBlock,
) -> oino_types::ContentBlock {
    use bindings::exports::oino::extension::guest::ContentBlock as Wit;
    match block {
        Wit::Text(text) => oino_types::ContentBlock::Text {
            text,
            signature: None,
        },
        Wit::Thinking(text) => oino_types::ContentBlock::Thinking {
            text,
            signature: None,
            provider_data: None,
            redacted: false,
        },
        Wit::ToolCall(json) | Wit::Image(json) | Wit::Custom(json) => serde_json::from_str(&json)
            .unwrap_or(oino_types::ContentBlock::Text {
                text: json,
                signature: None,
            }),
    }
}

fn classify_error(err: wasmtime::Error) -> WasmHostError {
    if err.is::<wasmtime::Trap>() {
        let trap = err.downcast_ref::<wasmtime::Trap>();
        if matches!(trap, Some(wasmtime::Trap::Interrupt)) {
            return WasmHostError::Deadline;
        }
    }
    WasmHostError::Instantiate(format!("{err:#}"))
}

// ---------------------------------------------------------------------------
// WasmStreamProvider
// ---------------------------------------------------------------------------

/// Adapts a `provider-extension` component to [`StreamProvider`]: serializes
/// the request to JSON, calls `provider.stream`, then pulls `next()` batches
/// on a dedicated task until `none`, forwarding events to the loop sink.
pub struct WasmStreamProvider {
    extension: Arc<WasmExtension>,
}

impl WasmStreamProvider {
    pub fn new(extension: Arc<WasmExtension>) -> WasmResult<Self> {
        if !extension.provider() {
            return Err(WasmHostError::Instantiate(
                "component does not export provider interface".into(),
            ));
        }
        Ok(Self { extension })
    }

    /// `provider.list-models()` → JSON (extension-defined schema).
    pub async fn list_models(&self) -> WasmResult<Value> {
        let mut slot = self.extension.slot.lock().await;
        self.extension.ensure(&mut slot).await?;
        let Some(ExtensionKind::Provider(ProviderInstance { instance, store })) =
            slot.instance.as_mut()
        else {
            return Err(WasmHostError::Instantiate(
                "component is not a provider".into(),
            ));
        };
        store.set_epoch_deadline(self.extension.deadline(self.extension.limits.hook));
        match instance
            .oino_extension_provider()
            .call_list_models(&mut *store)
            .await
        {
            Ok(Ok(json)) => serde_json::from_str(&json)
                .map_err(|err| WasmHostError::Serialization(err.to_string())),
            Ok(Err(err)) => Err(WasmHostError::Guest(err)),
            Err(err) => {
                self.extension.failed(&mut slot).await;
                Err(classify_error(err))
            }
        }
    }
}

#[async_trait::async_trait]
impl StreamProvider for WasmStreamProvider {
    async fn stream(
        &self,
        request: oino_agent_loop::StreamRequest,
        signal: AbortSignal,
    ) -> LoopResult<Vec<AssistantStreamEvent>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = Arc::clone(&events);
        let sink = Arc::new(move |event| {
            let sink_events = Arc::clone(&sink_events);
            let fut: BoxFuture<'static, LoopResult<()>> = Box::pin(async move {
                sink_events.lock().await.push(event);
                Ok(())
            });
            fut
        });
        self.stream_events(request, signal, sink).await?;
        let events = events.lock().await.clone();
        Ok(events)
    }

    async fn stream_events(
        &self,
        request: oino_agent_loop::StreamRequest,
        signal: AbortSignal,
        sink: oino_agent_loop::StreamEventSink,
    ) -> LoopResult<()> {
        let request_json = stream_request_json(&request);
        let mut slot = self.extension.slot.lock().await;
        self.extension
            .ensure(&mut slot)
            .await
            .map_err(|err| oino_agent_loop::LoopError::Stream(err.to_string()))?;
        let Some(ExtensionKind::Provider(ProviderInstance { instance, store })) =
            slot.instance.as_mut()
        else {
            return Err(oino_agent_loop::LoopError::Stream(
                "component is not a provider".into(),
            ));
        };
        let request_str = serde_json::to_string(&request_json)
            .map_err(|err| oino_agent_loop::LoopError::Stream(err.to_string()))?;
        let response = {
            store.set_epoch_deadline(self.extension.deadline(self.extension.limits.provider_next));
            let out = instance
                .oino_extension_provider()
                .call_stream(&mut *store, &request_str)
                .await
                .map_err(classify_loop_error)?
                .map_err(oino_agent_loop::LoopError::Stream)?;
            out
        };
        loop {
            if signal.is_aborted() {
                let _ = instance
                    .oino_extension_provider()
                    .response()
                    .call_cancel(&mut *store, response)
                    .await;
                let _ = sink(AssistantStreamEvent::Aborted).await;
                return Ok(());
            }
            store.set_epoch_deadline(self.extension.deadline(self.extension.limits.provider_next));
            let batch = {
                instance
                    .oino_extension_provider()
                    .response()
                    .call_next(&mut *store, response)
                    .await
            }
            .map_err(classify_loop_error)?
            .map_err(oino_agent_loop::LoopError::Stream)?;
            let Some(events) = batch else {
                break;
            };
            for event in events {
                if let Some(event) = assistant_event_from_wit(event) {
                    sink(event).await?;
                }
            }
        }
        Ok(())
    }
}

use oino_agent_loop::LoopError;

fn classify_loop_error(err: wasmtime::Error) -> LoopError {
    let err = classify_error(err);
    LoopError::Stream(err.to_string())
}

fn stream_request_json(request: &oino_agent_loop::StreamRequest) -> Value {
    serde_json::json!({
        "model": request.model,
        "thinking_level": request.thinking_level,
        "transcript": { "messages": request.transcript.messages() },
        "session_id": request.session_id,
        "cache_retention": request.cache_retention,
        "max_output_tokens": request.max_output_tokens,
    })
}

fn assistant_event_from_wit(
    event: provider_bindings::exports::oino::extension::provider::AssistantEvent,
) -> Option<AssistantStreamEvent> {
    use provider_bindings::exports::oino::extension::provider::AssistantEvent as Wit;
    match event {
        Wit::Start(json) => {
            let value: Value = serde_json::from_str(&json).ok()?;
            Some(AssistantStreamEvent::Start {
                response_id: value["response_id"].as_str().map(Into::into),
                response_model: value["response_model"].as_str().map(Into::into),
            })
        }
        Wit::TextStart(_) => None,
        Wit::TextDelta(delta) => Some(AssistantStreamEvent::TextDelta { delta }),
        Wit::ThinkingStart(_) => None,
        Wit::ThinkingDelta(delta) => Some(AssistantStreamEvent::ThinkingDelta { delta }),
        Wit::ThinkingSignature(signature) => {
            Some(AssistantStreamEvent::ThinkingSignature { signature })
        }
        Wit::ToolCallStart(json) | Wit::ToolCallDelta(json) => {
            let value: Value = serde_json::from_str(&json).ok()?;
            Some(AssistantStreamEvent::ToolCallDelta {
                id: value["id"]
                    .as_str()
                    .and_then(|s| s.parse::<oino_types::OinoId>().ok())
                    .unwrap_or_else(uuid::Uuid::new_v4),
                name: value["name"].as_str().map(Into::into),
                native_id: value["native_id"].as_str().map(Into::into),
                arguments_delta: value["arguments_delta"].as_str().unwrap_or_default().into(),
            })
        }
        Wit::ToolCallDone(json) => {
            let value: Value = serde_json::from_str(&json).ok()?;
            Some(AssistantStreamEvent::ToolCallDone {
                id: value["id"]
                    .as_str()
                    .and_then(|s| s.parse::<oino_types::OinoId>().ok())
                    .unwrap_or_else(uuid::Uuid::new_v4),
                name: value["name"].as_str().unwrap_or_default().into(),
                native_id: value["native_id"].as_str().map(Into::into),
                arguments: value["arguments"].clone(),
            })
        }
        Wit::Usage(json) => serde_json::from_str::<oino_types::Usage>(&json)
            .ok()
            .map(|usage| AssistantStreamEvent::Usage { usage }),
        Wit::Done(json) => {
            let value: Value = serde_json::from_str(&json).ok()?;
            Some(AssistantStreamEvent::Done {
                stop_reason: value["stop_reason"]
                    .as_str()
                    .map(stop_reason_from_str)
                    .unwrap_or_else(|| {
                        serde_json::from_value::<StopReason>(value["stop_reason"].clone())
                            .unwrap_or(StopReason::Unknown)
                    }),
                provider: serde_json::from_value::<oino_types::ProviderMetadata>(
                    value["provider"].clone(),
                )
                .ok(),
            })
        }
        Wit::Error(message) => Some(AssistantStreamEvent::Error { message }),
        Wit::Aborted => Some(AssistantStreamEvent::Aborted),
    }
}

fn stop_reason_from_str(value: &str) -> StopReason {
    match value {
        "end_turn" | "end-turn" => StopReason::EndTurn,
        "tool_use" | "tool-use" => StopReason::ToolUse,
        "length" => StopReason::Length,
        "error" => StopReason::Error,
        "aborted" => StopReason::Aborted,
        _ => StopReason::Unknown,
    }
}

/// Routes per-tool-call progress from guests to the right
/// `ToolUpdateCallback`, keyed by call id.
#[derive(Clone, Default)]
pub struct ProgressRouter {
    callbacks: Arc<std::sync::Mutex<BTreeMap<String, oino_agent_loop::ToolUpdateCallback>>>,
}

impl ProgressRouter {
    /// A `ProgressSink` that forwards updates to the callback registered for
    /// the given call id. Unregistered ids are dropped.
    pub fn sink(&self) -> ProgressSink {
        let callbacks = Arc::clone(&self.callbacks);
        Arc::new(
            move |call_id: &str, message: &str, details: Option<String>| {
                let callback = callbacks
                    .lock()
                    .ok()
                    .and_then(|map| map.get(call_id).cloned());
                if let Some(callback) = callback {
                    let details = details.and_then(|json| serde_json::from_str(&json).ok());
                    let update = oino_agent_loop::ToolUpdate {
                        message: message.to_string(),
                        details,
                    };
                    tokio::spawn(async move {
                        let _ = callback.update(update).await;
                    });
                }
            },
        )
    }

    pub fn register(&self, call_id: uuid::Uuid, callback: oino_agent_loop::ToolUpdateCallback) {
        if let Ok(mut map) = self.callbacks.lock() {
            map.insert(call_id.to_string(), callback);
        }
    }

    pub fn unregister(&self, call_id: uuid::Uuid) {
        if let Ok(mut map) = self.callbacks.lock() {
            map.remove(&call_id.to_string());
        }
    }
}

/// An agent-loop `Tool` backed by a wasm extension's `run_tool` export.
pub struct WasmTool {
    extension: Arc<WasmExtension>,
    progress: ProgressRouter,
    /// Name passed to the guest's `run_tool` (the registration name).
    guest_name: String,
    definition: oino_agent_loop::ToolDefinition,
    execution_mode: oino_agent_loop::ToolExecutionMode,
}

impl WasmTool {
    #[must_use]
    pub fn new(
        extension: Arc<WasmExtension>,
        progress: ProgressRouter,
        guest_name: impl Into<String>,
        definition: oino_agent_loop::ToolDefinition,
        execution_mode: oino_agent_loop::ToolExecutionMode,
    ) -> Self {
        Self {
            extension,
            progress,
            guest_name: guest_name.into(),
            definition,
            execution_mode,
        }
    }
}

#[async_trait::async_trait]
impl oino_agent_loop::Tool for WasmTool {
    fn definition(&self) -> oino_agent_loop::ToolDefinition {
        self.definition.clone()
    }
    fn execution_mode(&self) -> oino_agent_loop::ToolExecutionMode {
        self.execution_mode
    }
    async fn execute(
        &self,
        call: oino_agent_loop::ToolCall,
        updates: oino_agent_loop::ToolUpdateCallback,
        signal: AbortSignal,
    ) -> oino_agent_loop::LoopResult<oino_agent_loop::ToolResult> {
        if signal.is_aborted() {
            return Ok(oino_agent_loop::ToolResult::error(&call, "aborted"));
        }
        self.progress.register(call.id, updates);
        let output = self
            .extension
            .run_tool(&self.guest_name, call.id, call.arguments.clone())
            .await;
        self.progress.unregister(call.id);
        match output {
            Ok(output) => Ok(output.into_tool_result(&call)),
            Err(err) => Err(oino_agent_loop::LoopError::Tool(err.to_string())),
        }
    }
}

#[cfg(test)]
mod stream_request_mirror_tests {
    use super::stream_request_json;
    use oino_agent_loop::CacheRetention;
    use oino_types::{transcript::Transcript, Message, Model, ThinkingLevel};

    /// The `stream(request)` JSON the host produces must keep exactly the
    /// shape `oino_provider_kit::StreamRequest` deserializes (the guest
    /// workspace is separate, so compatibility is asserted on the wire
    /// shape here).
    #[test]
    fn stream_request_json_matches_guest_mirror() {
        let request = oino_agent_loop::StreamRequest {
            model: Model::new("prov", "m"),
            thinking_level: ThinkingLevel::Low,
            transcript: Transcript::new(vec![Message::user_text("hi")]),
            session_id: Some("s".into()),
            cache_retention: CacheRetention::Long,
            max_output_tokens: None,
        };
        let value = stream_request_json(&request);
        // mirror field-for-field
        assert_eq!(
            value["model"],
            serde_json::to_value(Model::new("prov", "m")).unwrap_or_default()
        );
        assert_eq!(
            value["thinking_level"],
            serde_json::to_value(ThinkingLevel::Low).unwrap_or_default()
        );
        assert!(value["transcript"]["messages"].is_array());
        assert_eq!(value["session_id"], serde_json::json!("s"));
        assert_eq!(value["cache_retention"], serde_json::json!("long"));
        let mut keys: Vec<String> = value
            .as_object()
            .map(|o| o.keys().cloned().collect())
            .unwrap_or_default();
        keys.sort();
        assert_eq!(
            keys,
            [
                "cache_retention",
                "max_output_tokens",
                "model",
                "session_id",
                "thinking_level",
                "transcript"
            ]
        );
    }
}
