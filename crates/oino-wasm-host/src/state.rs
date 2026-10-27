//! Store data of one component instance and per-extension shared state.

use std::collections::{HashMap, VecDeque};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use oino_extension_api::manifest::Permissions;
use oino_extension_api::{CallContext, ExtensionId, HostError, HostServices, Manifest};
use wasmtime::component::{Access, Accessor, GuestTaskId, HasData, ResourceTable};
use wasmtime::{AsContextMut, StoreLimits};
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
    pub(crate) bundled: bool,
    pub(crate) host: Arc<dyn HostServices>,
    pub(crate) config: HostConfig,
    pub(crate) paths: ExtPaths,
    pub(crate) kv: KvStore,
    pub(crate) timers: Timers,
    pub(crate) http: HttpService,
    pub(crate) processes: ProcessService,
    pub(crate) preopen_list: Vec<Preopen>,
    /// Hosts granted implicitly (registered provider base URLs, §5.6).
    pub(crate) http_grants: HttpGrants,
    /// Context of the most recent call (fallback for imports made outside a known task).
    pub(crate) last_cx: Mutex<CallContext>,
    /// `ui.dialog` calls waiting for the user; handler deadlines pause meanwhile (GA-42).
    pub(crate) dialogs_open: std::sync::atomic::AtomicUsize,
}

impl ExtShared {
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
}

const CALL_TABLE_CAP: usize = 512;

impl CallTable {
    pub(crate) fn insert(&mut self, task: GuestTaskId, cx: CallContext) {
        if self.map.insert(task, cx).is_none() {
            self.order.push_back(task);
        }
        while self.order.len() > CALL_TABLE_CAP {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }

    pub(crate) fn get(&self, task: GuestTaskId) -> Option<&CallContext> {
        self.map.get(&task)
    }
}

/// Store data.
pub(crate) struct HostState {
    pub(crate) wasi: WasiCtx,
    pub(crate) table: ResourceTable,
    pub(crate) limits: StoreLimits,
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
