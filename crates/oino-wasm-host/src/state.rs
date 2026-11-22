//! Store data of one component instance and per-extension shared state.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use oino_extension_api::manifest::Permissions;
use oino_extension_api::{CallContext, ExtensionId, HostError, HostServices, Manifest};
use wasmtime::component::{Access, Accessor, GuestTaskId, HasData, ResourceTable};
use wasmtime::{AsContextMut, ResourceLimiter, StoreLimits};
use wasmtime_wasi::{WasiCtx, WasiCtxView, WasiView};

use crate::http::HttpService;
use crate::kv::KvStore;
use crate::paths::{ExtPaths, Preopen};
use crate::process::ProcessService;
use crate::timers::Timers;

/// Locks a std mutex, recovering from poisoning (state stays usable after a panic elsewhere).
pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Host-wide configuration shared by every instance.
#[derive(Clone, Debug)]
pub struct HostConfig {
    /// Agent dir (`~/.oino`).
    pub agent_dir: PathBuf,
    /// Process cwd for instances (Services are per cwd).
    pub cwd: PathBuf,
    /// Oino version reported by `host-info.get`.
    pub oino_version: String,
    /// HTTP proxy (settings `httpProxy`).
    pub http_proxy: Option<String>,
    /// Idle HTTP timeout.
    pub http_idle_timeout: std::time::Duration,
}

impl HostConfig {
    /// Config for `agent_dir` and `cwd` with defaults.
    #[must_use]
    pub fn new(agent_dir: impl Into<PathBuf>, cwd: impl Into<PathBuf>) -> Self {
        Self {
            agent_dir: agent_dir.into(),
            cwd: cwd.into(),
            oino_version: env!("CARGO_PKG_VERSION").to_owned(),
            http_proxy: None,
            http_idle_timeout: std::time::Duration::from_secs(300),
        }
    }
}

/// State shared by all calls of one extension.
pub(crate) struct ExtShared {
    pub(crate) id: ExtensionId,
    pub(crate) manifest: Manifest,
    /// Package root on disk (resources + `bin/`), when any — the process
    /// service resolves declared `binaries` against it (ext-abi §5.2).
    pub(crate) package_root: Option<PathBuf>,
    pub(crate) bundled: bool,
    pub(crate) host: Arc<dyn HostServices>,
    pub(crate) config: HostConfig,
    pub(crate) paths: ExtPaths,
    pub(crate) kv: KvStore,
    pub(crate) timers: Timers,
    pub(crate) timer_target: std::sync::OnceLock<std::sync::Weak<crate::instance::WasmInstance>>,
    pub(crate) http: HttpService,
    pub(crate) processes: ProcessService,
    pub(crate) preopen_list: Vec<Preopen>,
    /// Hosts granted implicitly (registered provider base URLs, §5.6).
    pub(crate) http_grants: HttpGrants,
    /// Context of the most recent call (fallback for imports made outside a known task).
    pub(crate) last_cx: Mutex<CallContext>,
    /// `ui.dialog` calls waiting for the user; handler deadlines pause meanwhile (GA-42).
    pub(crate) dialogs_open: std::sync::atomic::AtomicUsize,
    /// Milliseconds spent (so far) blocked in a process-approval check: the user answering an approval card is not
    /// handler time (N6 — a `tool_call` hook that spawned a process timed out at 30 s behind the approval queue).
    pub(crate) approval_wait_ms: std::sync::atomic::AtomicU64,
}

struct Waited<'a>(&'a std::sync::atomic::AtomicU64, std::time::Instant);
impl Drop for Waited<'_> {
    fn drop(&mut self) {
        let ms = u64::try_from(self.1.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.0
            .fetch_add(ms.max(1), std::sync::atomic::Ordering::SeqCst);
    }
}

impl ExtShared {
    /// `host.check_process`, with the time it blocks (an approval card may be waiting for the user) excluded
    /// from the handler deadline.
    pub(crate) fn check_process(
        &self,
        cx: &CallContext,
        command: &str,
        args: &[String],
    ) -> Result<(), HostError> {
        if self.is_package_action(cx, command, args) {
            return Ok(());
        }
        let _open = crate::imports::runtime::DialogOpen::new(&self.dialogs_open);
        let _waited = Waited(&self.approval_wait_ms, std::time::Instant::now());
        self.host.check_process(&self.id, cx, command, args)
    }

    /// Package-owned helper calls are extension implementation, not agent actions.
    /// Exact declared read-only helpers work in every call context. An explicit
    /// user command may also execute its own declared binaries (including writes).
    /// Fail closed for PATH/dev overrides, undeclared binaries and symlink escapes;
    /// agent tools retain consent for anything except declared read-only helpers.
    fn is_package_action(&self, cx: &CallContext, command: &str, args: &[String]) -> bool {
        let Some(root) = self
            .package_root
            .as_ref()
            .and_then(|p| p.canonicalize().ok())
        else {
            return false;
        };
        let path = std::path::Path::new(command);
        if !path.is_absolute() {
            return false;
        }
        self.manifest.binaries.iter().any(|name| {
            let allowed = self.manifest.helpers.get(name);
            if std::path::Path::new(name).components().count() != 1
                || !(cx.kind == oino_extension_api::CallKind::Command
                    || allowed.is_some_and(|calls| calls.iter().any(|argv| argv == args)))
            {
                return false;
            }
            let declared = root.join("bin").join(name);
            let (Ok(actual), Ok(expected)) = (path.canonicalize(), declared.canonicalize()) else {
                return false;
            };
            actual == expected && actual.starts_with(&root) && actual.is_file()
        })
    }

    pub(crate) fn perms(&self) -> &Permissions {
        &self.manifest.permissions
    }

    /// Preopens visible to calls (fixed at load).
    pub(crate) fn preopens(&self, _cx: &CallContext) -> Vec<Preopen> {
        self.preopen_list.clone()
    }

    /// Records a detached daemon in `state/<id>/global/daemons.json` (§5.4).
    pub(crate) fn record_daemon(&self, pid: u32) {
        let path = self.paths.global.join("daemons.json");
        let mut list: Vec<serde_json::Value> = std::fs::read(&path)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        list.push(serde_json::json!({"pid": pid, "startedAt": crate::timers::now_ms()}));
        if let Ok(bytes) = serde_json::to_vec_pretty(&list) {
            let _ = crate::kv::atomic_write(&path, &bytes, None);
        }
    }

    /// `Ok` when `granted`, else `denied(name)`.
    pub(crate) fn require(granted: bool, name: &str) -> Result<(), HostError> {
        if granted {
            Ok(())
        } else {
            Err(HostError::Denied(name.to_owned()))
        }
    }
}

/// Maps root guest tasks to the call context they serve. Bounded so detached guest tasks
/// (spawned work that outlives its export call) keep their context for a while.
#[derive(Default)]
pub(crate) struct CallTable {
    map: HashMap<GuestTaskId, CallContext>,
    order: VecDeque<GuestTaskId>,
    #[cfg(feature = "core")]
    hooks: HashMap<GuestTaskId, oino_core::extensions::runner::HookContext>,
}

const CALL_TABLE_CAP: usize = 512;

impl CallTable {
    pub(crate) fn insert(&mut self, task: GuestTaskId, cx: CallContext) {
        #[cfg(feature = "core")]
        self.hooks
            .insert(task, oino_core::extensions::runner::HookContext::capture());
        if self.map.insert(task, cx).is_none() {
            self.order.push_back(task);
        }
        while self.order.len() > CALL_TABLE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
                #[cfg(feature = "core")]
                self.hooks.remove(&old);
            }
        }
    }

    pub(crate) fn get(&self, task: GuestTaskId) -> Option<&CallContext> {
        self.map.get(&task)
    }
}

/// The store's resource limits plus a linear-memory growth trail (perf-fix #1): a guest's
/// memory never shrinks, so each doubling past [`MEMORY_LOG_FLOOR`] is logged with the
/// extension id — the attribution the perf baseline lacked ("4 instances of ≈ 480 MB").
pub(crate) struct GuestLimits {
    inner: StoreLimits,
    extension: String,
    logged: usize,
}

/// Linear memory below this is not worth a log line.
const MEMORY_LOG_FLOOR: usize = 64 << 20;

impl GuestLimits {
    pub(crate) fn new(inner: StoreLimits, extension: &ExtensionId) -> Self {
        Self {
            inner,
            extension: extension.to_string(),
            logged: MEMORY_LOG_FLOOR / 2,
        }
    }
}

impl ResourceLimiter for GuestLimits {
    fn memory_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        let allowed = self.inner.memory_growing(current, desired, maximum)?;
        if allowed && desired >= self.logged.saturating_mul(2) {
            self.logged = desired;
            tracing::info!(
                extension = %self.extension,
                mib = desired >> 20,
                "extension linear memory grew (it never shrinks)"
            );
        }
        Ok(allowed)
    }
    fn memory_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.memory_grow_failed(error)
    }
    fn table_growing(
        &mut self,
        current: usize,
        desired: usize,
        maximum: Option<usize>,
    ) -> wasmtime::Result<bool> {
        self.inner.table_growing(current, desired, maximum)
    }
    fn table_grow_failed(&mut self, error: wasmtime::Error) -> wasmtime::Result<()> {
        self.inner.table_grow_failed(error)
    }
    fn instances(&self) -> usize {
        self.inner.instances()
    }
    fn tables(&self) -> usize {
        self.inner.tables()
    }
    fn memories(&self) -> usize {
        self.inner.memories()
    }
}

/// Store data.
pub(crate) struct HostState {
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: GuestLimits,
    pub(crate) ext: Arc<ExtShared>,
    pub(crate) calls: CallTable,
}

impl WasiView for HostState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl HasData for HostState {
    type Data<'a> = &'a mut HostState;
}

/// Resolves the call context of the guest task making the current import call.
pub(crate) fn current<T>(access: &mut Access<'_, T, HostState>) -> (Arc<ExtShared>, CallContext) {
    let root = access
        .as_context_mut()
        .async_call_stack()
        .ok()
        .and_then(Iterator::last);
    let state = access.get();
    let ext = Arc::clone(&state.ext);
    let cx = root
        .and_then(|task| state.calls.get(task).cloned())
        .unwrap_or_else(|| lock(&ext.last_cx).clone());
    (ext, cx)
}

/// [`current`] through an [`Accessor`].
pub(crate) fn current_async<T>(accessor: &Accessor<T, HostState>) -> (Arc<ExtShared>, CallContext) {
    accessor.with(|mut access| current(&mut access))
}

/// Hosts allowed without a manifest grant (provider base URLs registered by the extension).
#[derive(Default)]
pub(crate) struct HttpGrants(Mutex<std::collections::HashSet<String>>);

impl HttpGrants {
    pub(crate) fn add_url(&self, url: &str) {
        if let Some(host) = reqwest::Url::parse(url)
            .ok()
            .as_ref()
            .and_then(crate::http::host_of)
        {
            lock(&self.0).insert(host);
        }
    }

    pub(crate) fn allows(&self, host: &str) -> bool {
        lock(&self.0).contains(host)
    }
}

/// Restore command context from the actual guest root task: imports may be
/// polled by Wasmtime on a different executor task from the actor job.
#[cfg(feature = "core")]
pub(crate) fn current_hook<T>(
    accessor: &Accessor<T, HostState>,
) -> oino_core::extensions::runner::HookContext {
    accessor.with(|mut access| {
        let root = access
            .as_context_mut()
            .async_call_stack()
            .ok()
            .and_then(Iterator::last);
        root.and_then(|task| access.get().calls.hooks.get(&task).cloned())
            .unwrap_or_default()
    })
}
