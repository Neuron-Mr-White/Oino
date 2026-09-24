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
use std::{collections::BTreeSet, path::PathBuf, sync::Arc, time::Duration};
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

/// Session surface for `host.session-append`.
pub trait HostSession: Send + Sync {
    fn append(&self, kind: &str, payload: &str) -> Result<(), String>;
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
            .map_err(|err| WasmHostError::Instantiate(err.to_string()))?;
        config.cache(Some(cache));
        let engine =
            Engine::new(&config).map_err(|err| WasmHostError::Instantiate(err.to_string()))?;
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
            .map_err(|err| WasmHostError::Instantiate(err.to_string()))
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
    raw_network: bool,
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
        if !self.raw_network && !self.hosts.contains(&host) {
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
        let mut command = std::process::Command::new(&cmd);
        command.args(&args);
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
    for permission in &caps.permissions.filesystem {
        use oino_extension_core::FilesystemAccess;
        let perms = match permission.access {
            FilesystemAccess::Read => wasmtime_wasi::FsPerms::ReadOnly,
            FilesystemAccess::Write | FilesystemAccess::ReadWrite => {
                wasmtime_wasi::FsPerms::ReadWrite
            }
        };
        for path in &permission.paths {
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
            raw_network: caps.permissions.network.raw_network,
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
            .map_err(|err| WasmHostError::Instantiate(err.to_string()))?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)
            .map_err(|err| WasmHostError::Instantiate(err.to_string()))?;
        bindings::oino::extension::host::add_to_linker::<_, wasmtime::component::HasSelf<HostState>>(
            &mut linker,
            |state: &mut HostState| state,
        )
        .map_err(|err| WasmHostError::Instantiate(err.to_string()))?;
        if self.provider {
            let instance = provider_bindings::ProviderExtension::instantiate_async(
                &mut store,
                &self.component,
                &linker,
            )
            .await
            .map_err(|err| WasmHostError::Instantiate(err.to_string()))?;
            Ok(ExtensionKind::Provider(ProviderInstance {
                instance,
                store,
            }))
        } else {
            let instance =
                bindings::Extension::instantiate_async(&mut store, &self.component, &linker)
                    .await
                    .map_err(|err| WasmHostError::Instantiate(err.to_string()))?;
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
                Wit::Text(text) => oino_types::ContentBlock::Text { text },
                Wit::Thinking(text) => oino_types::ContentBlock::Thinking {
                    text,
                    signature: None,
                    provider_data: None,
                    redacted: false,
                },
                Wit::ToolCall(json) | Wit::Image(json) | Wit::Custom(json) => {
                    serde_json::from_str(&json)
                        .unwrap_or(oino_types::ContentBlock::Text { text: json })
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
        Wit::Text(text) => oino_types::ContentBlock::Text { text },
        Wit::Thinking(text) => oino_types::ContentBlock::Thinking {
            text,
            signature: None,
            provider_data: None,
            redacted: false,
        },
        Wit::ToolCall(json) | Wit::Image(json) | Wit::Custom(json) => {
            serde_json::from_str(&json).unwrap_or(oino_types::ContentBlock::Text { text: json })
        }
    }
}

fn classify_error(err: wasmtime::Error) -> WasmHostError {
    if err.is::<wasmtime::Trap>() {
        let trap = err.downcast_ref::<wasmtime::Trap>();
        if matches!(trap, Some(wasmtime::Trap::Interrupt)) {
            return WasmHostError::Deadline;
        }
    }
    WasmHostError::Instantiate(err.to_string())
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
            instance
                .oino_extension_provider()
                .call_stream(&mut *store, &request_str)
                .await
                .map_err(classify_loop_error)?
                .map_err(oino_agent_loop::LoopError::Stream)?
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
            let batch = instance
                .oino_extension_provider()
                .response()
                .call_next(&mut *store, response)
                .await
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
                arguments: value["arguments"].clone(),
            })
        }
        Wit::Usage(json) => serde_json::from_str::<oino_types::Usage>(&json)
            .ok()
            .map(|usage| AssistantStreamEvent::Usage { usage }),
        Wit::Done(json) => {
            let value: Value = serde_json::from_str(&json).ok()?;
            Some(AssistantStreamEvent::Done {
                stop_reason: stop_reason_from_str(value["stop_reason"].as_str()),
                provider: None,
            })
        }
        Wit::Error(message) => Some(AssistantStreamEvent::Error { message }),
        Wit::Aborted => Some(AssistantStreamEvent::Aborted),
    }
}

fn stop_reason_from_str(value: Option<&str>) -> StopReason {
    match value {
        Some("end_turn") | Some("end-turn") => StopReason::EndTurn,
        Some("tool_use") | Some("tool-use") => StopReason::ToolUse,
        Some("length") => StopReason::Length,
        Some("error") => StopReason::Error,
        Some("aborted") => StopReason::Aborted,
        _ => StopReason::Unknown,
    }
}
