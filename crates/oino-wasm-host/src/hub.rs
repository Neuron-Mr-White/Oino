//! [`LocalServices`]: cross-extension features the WASM host can serve on its own while
//! `oino-core` does not implement them (core-runtime deviations "Deferred"): the event bus,
//! services, secrets, the background-task service and delivery of host-produced `oino_*`
//! events. Every other [`HostServices`] method passes through to the wrapped host.
//!
//! One hub is shared process-wide per agent dir ([`crate::EngineRegistry`], E-ENGINE), so
//! the task store, secrets and sticky bus topics stay coherent across every service of one
//! host. Instances and wrapped hosts are held weakly: when a service is disposed its
//! entries go dead instead of keeping it alive, and a later service registering the same
//! extension id replaces the entry (last one wins).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use oino_extension_api::host_service::HostService;
use oino_extension_api::ops::{
    ControlOp, ControlReply, Delivery, DeviceOp, EditorOp, HostContextInfo, ModelsOp, ModelsReply,
    PanelOp, ProviderHook, ProviderHookReply, SecretOp, SendOptions, SessionQuery, SessionWrite,
    SessionsIndexQuery, SettingsLayer, SettingsScope, TaskOp,
};
use oino_extension_api::registry::{
    ArbiterRoles, CommandSpec, FlagSpec, RendererSpec, ShortcutSpec, ToolSpec,
};
use oino_extension_api::{
    AgentHandle, BoxFut, CallContext, CallKind, CancellationToken, EventKind, ExtensionId,
    ExtensionInstance, HostError, HostServices, RawJson, Registration,
};
use serde_json::json;

use crate::instance::WasmInstance;
use crate::secrets::SecretStore;
use crate::state::lock;
use crate::tasks::{TaskInfo, TaskService, TaskSpec};

/// Custom message type of task completion notifications.
pub const TASK_NOTIFICATION_TYPE: &str = "background-task-notification";

/// Identity of one services set: the address of the host every extension of one load
/// pass is wrapped around (core builds one `CoreHost` per `Services`). Held only while a
/// load of that set is alive, so the address is not reused under a live entry.
pub(crate) type Group = usize;

/// The [`Group`] of `inner`.
pub(crate) fn group_of(inner: &Arc<dyn HostServices>) -> Group {
    Arc::as_ptr(inner).cast::<()>().addr()
}

/// One load of an extension id: the instance plus the host it was wrapped around.
#[derive(Clone)]
struct Load {
    group: Group,
    instance: Weak<WasmInstance>,
    inner: Weak<dyn HostServices>,
}

#[derive(Default)]
struct ExtEntry {
    /// Every live load of this extension id, oldest first (one per services set: several
    /// cwds load the same extension). Calls, events, bus deliveries and timers route to
    /// the caller's OWN services set first; only a caller without one (the embedding host)
    /// or a set that lacks the extension falls back to the newest awake load. The old
    /// "last load wins" routing sent a session's `subagents.profiles` to another cwd's
    /// instance — or to a disposed set's hibernated one, which woke with no session state
    /// ("subagents are not loaded or are disabled"). Dead loads are pruned on every new
    /// load, so the list is bounded by the live services sets. A task started by an older
    /// load still finds its own route after a newer load died (G-04).
    loads: Vec<Load>,
    events: HashSet<EventKind>,
    services: HashSet<String>,
    bus: HashSet<String>,
}

/// Process-wide local services shared by all instances of one [`crate::WasmRuntime`] (one
/// hub per agent dir per process, [`crate::EngineRegistry`]).
pub struct LocalServices {
    exts: Mutex<HashMap<ExtensionId, ExtEntry>>,
    sticky: Mutex<HashMap<String, RawJson>>,
    secrets: SecretStore,
    tasks: Arc<TaskService>,
    /// Host-provided services (`host-service.wit`): name → service (v2 ext-abi §3).
    host_services: std::sync::RwLock<BTreeMap<String, Arc<dyn HostService>>>,
    /// Nested agent session → its parent session (browser scope = root conversation).
    agent_parents: Mutex<HashMap<String, String>>,
}

impl std::fmt::Debug for LocalServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalServices").finish_non_exhaustive()
    }
}

/// `pattern` matches `topic`: exact, or `prefix:*` / `*`.
#[must_use]
pub fn topic_matches(pattern: &str, topic: &str) -> bool {
    pattern == "*"
        || pattern == topic
        || pattern
            .strip_suffix('*')
            .is_some_and(|prefix| topic.starts_with(prefix))
}

/// Most loads one extension id keeps. A disposed services set can stay reachable (core's
/// host router and extension set hold each other), so its hibernated loads would never be
/// pruned as dead: past the cap the oldest hibernated load is forgotten first.
const MAX_LOADS: usize = 16;

/// Drops loads whose instance and host have both gone, then trims to [`MAX_LOADS`] - 1
/// (room for the load about to be added), oldest hibernated first.
fn prune_loads(entry: &mut ExtEntry) {
    entry
        .loads
        .retain(|r| r.instance.strong_count() > 0 || r.inner.strong_count() > 0);
    while entry.loads.len() >= MAX_LOADS {
        let victim = entry
            .loads
            .iter()
            .position(|l| l.instance.upgrade().is_none_or(|i| i.hibernated()))
            .unwrap_or(0);
        entry.loads.remove(victim);
    }
}

/// The load to route to: `group`'s own, else the newest awake one, else the newest live.
fn pick<T>(
    loads: &[Load],
    group: Option<Group>,
    get: impl Fn(&Load) -> Option<T>,
    awake: impl Fn(&Load) -> bool,
) -> Option<T> {
    if let Some(group) = group
        && let Some(found) = loads
            .iter()
            .rev()
            .filter(|l| l.group == group)
            .find_map(&get)
    {
        return Some(found);
    }
    loads
        .iter()
        .rev()
        .filter(|l| awake(l))
        .find_map(&get)
        .or_else(|| loads.iter().rev().find_map(&get))
}

impl LocalServices {
    /// Hub rooted at `agent_dir` (secrets file, task output).
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            exts: Mutex::new(HashMap::new()),
            sticky: Mutex::new(HashMap::new()),
            secrets: SecretStore::new(agent_dir),
            tasks: Arc::new(TaskService::new(agent_dir)),
            host_services: std::sync::RwLock::new(BTreeMap::new()),
            agent_parents: Mutex::new(HashMap::new()),
        }
    }

    /// Registers (replaces) one host-provided service served to extensions with a
    /// matching `hostServices` permission (`host-service.wit`, v2 ext-abi §3).
    pub fn register_host_service(&self, name: &str, service: Arc<dyn HostService>) {
        self.host_services
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(name.to_owned(), service);
    }

    /// The registered host service `name`, if any (the hub registry an embedding host
    /// installs; `inner.host_service` is the fallback below it).
    #[must_use]
    pub fn host_service(&self, name: &str) -> Option<Arc<dyn HostService>> {
        self.host_services
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(name)
            .cloned()
    }

    /// Names of the registered host services (sorted).
    #[must_use]
    pub fn host_service_names(&self) -> Vec<String> {
        self.host_services
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .keys()
            .cloned()
            .collect()
    }

    /// Root conversation session of `session` (walks nested-agent parents, §6.1).
    #[must_use]
    pub fn root_session(&self, session: &str) -> String {
        let parents = lock(&self.agent_parents);
        let mut cur = session.to_owned();
        for _ in 0..16 {
            match parents.get(&cur) {
                Some(p) if *p != cur => cur.clone_from(p),
                _ => break,
            }
        }
        cur
    }

    /// Service names `id` registered (`registry.register-service` and `init`).
    #[must_use]
    pub fn services_of(&self, id: &ExtensionId) -> Vec<String> {
        let mut names: Vec<String> = lock(&self.exts)
            .get(id)
            .map(|e| e.services.iter().cloned().collect())
            .unwrap_or_default();
        names.sort();
        names
    }

    /// The task service.
    #[must_use]
    pub fn tasks(&self) -> &Arc<TaskService> {
        &self.tasks
    }

    /// Wraps the host given to extension `id` (held weakly: a disposed service must not be
    /// kept alive by the shared hub). The entry's previous complete registration moves to
    /// `past` so it stays reachable while either half is alive (G-04).
    pub(crate) fn wrap(
        self: &Arc<Self>,
        id: &ExtensionId,
        inner: Arc<dyn HostServices>,
    ) -> Arc<dyn HostServices> {
        let group = group_of(&inner);
        let mut exts = lock(&self.exts);
        let entry = exts.entry(id.clone()).or_default();
        prune_loads(entry);
        entry.loads.push(Load {
            group,
            instance: Weak::new(),
            inner: Arc::downgrade(&inner),
        });
        drop(exts);
        Arc::new(Composite {
            inner,
            group,
            hub: Arc::downgrade(self),
        })
    }

    /// [`LocalServices::wrap`] for harness tests (feature `testing`): the component
    /// harnesses exercise the real wake path, which runs through the wrapped host (G-04).
    #[cfg(feature = "testing")]
    pub fn wrap_for_test(
        self: &Arc<Self>,
        id: &ExtensionId,
        inner: Arc<dyn HostServices>,
    ) -> Arc<dyn HostServices> {
        self.wrap(id, inner)
    }

    pub(crate) fn register_instance(self: &Arc<Self>, instance: &Arc<WasmInstance>, group: Group) {
        let _ = instance.hub.set(Arc::downgrade(self));
        let mut exts = lock(&self.exts);
        let entry = exts.entry(instance.id().clone()).or_default();
        // The newest load of this set still waiting for its instance (`wrap` ran first).
        match entry
            .loads
            .iter_mut()
            .rev()
            .find(|l| l.group == group && l.instance.strong_count() == 0)
        {
            Some(load) => load.instance = Arc::downgrade(instance),
            None => entry.loads.push(Load {
                group,
                instance: Arc::downgrade(instance),
                inner: Weak::<Composite>::new(),
            }),
        }
    }

    /// Records the `init` registration (events and services the hub routes).
    pub(crate) fn note_registration(&self, id: &ExtensionId, reg: &Registration) {
        let mut exts = lock(&self.exts);
        let entry = exts.entry(id.clone()).or_default();
        entry.events.extend(reg.events.iter().copied());
        entry.services.extend(reg.services.iter().cloned());
    }

    /// The instance of `id` serving `group` (the caller's services set), else the newest
    /// awake load, else any live one (G-04 — the newest loader may have died while an
    /// older services set still runs the extension).
    fn instance(&self, id: &ExtensionId, group: Option<Group>) -> Option<Arc<WasmInstance>> {
        let exts = lock(&self.exts);
        let entry = exts.get(id)?;
        pick(
            &entry.loads,
            group,
            |l| l.instance.upgrade(),
            |l| l.instance.upgrade().is_some_and(|i| !i.hibernated()),
        )
    }

    /// A live host route of `id` (`group`'s own first, as [`Self::instance`]; G-04).
    fn inner(&self, id: &ExtensionId, group: Option<Group>) -> Option<Arc<dyn HostServices>> {
        let mut exts = lock(&self.exts);
        let entry = exts.get_mut(id)?;
        prune_loads(entry);
        pick(
            &entry.loads,
            group,
            |l| l.inner.upgrade(),
            |l| l.instance.upgrade().is_some_and(|i| !i.hibernated()),
        )
    }

    /// Delivers a host-produced event to `id` when it subscribed to `kind` (the newest
    /// awake load; [`Self::deliver_in`] targets one services set).
    pub fn deliver(&self, id: &ExtensionId, session_id: &str, kind: EventKind, payload: RawJson) {
        self.deliver_in(None, id, session_id, kind, payload);
    }

    /// [`Self::deliver`] preferring `group`'s own load of `id`.
    fn deliver_in(
        &self,
        group: Option<Group>,
        id: &ExtensionId,
        session_id: &str,
        kind: EventKind,
        payload: RawJson,
    ) {
        let subscribed = lock(&self.exts)
            .get(id)
            .is_some_and(|e| e.events.contains(&kind));
        if !subscribed {
            return;
        }
        let Some(instance) = self.instance(id, group) else {
            return;
        };
        let mut cx = lock(&instance.shared.last_cx)
            .clone()
            .with_kind(CallKind::Event);
        if !session_id.is_empty() {
            session_id.clone_into(&mut cx.session_id);
        }
        tokio::spawn(async move {
            if let Err(e) = instance.on_event(kind, payload, cx).await {
                tracing::warn!(extension = %e.extension, call = %e.call, "{}", e.message);
            }
        });
    }

    /// Publishes `topic` on the bus on behalf of the embedding host (`publisher` = the
    /// extension identity the event speaks for): a sticky value is kept for late
    /// subscribers, and every extension subscribed to a matching pattern receives the
    /// `oino_bus` event. Host-side counterpart of the `bus.publish` WIT import (an embedded
    /// cron scheduler's `cron:run-finished` uses it, ext-abi §6.3).
    pub fn publish_host(
        &self,
        publisher: &ExtensionId,
        session_id: &str,
        topic: &str,
        data: &RawJson,
        sticky: bool,
    ) {
        self.publish(None, publisher, session_id, topic, data, sticky);
    }

    fn publish(
        &self,
        group: Option<Group>,
        publisher: &ExtensionId,
        session_id: &str,
        topic: &str,
        data: &RawJson,
        sticky: bool,
    ) {
        if sticky {
            lock(&self.sticky).insert(topic.to_owned(), data.clone());
        }
        let targets: Vec<ExtensionId> = lock(&self.exts)
            .iter()
            .filter(|(id, e)| *id != publisher && e.bus.iter().any(|p| topic_matches(p, topic)))
            .map(|(id, _)| id.clone())
            .collect();
        let payload = RawJson::from(json!({
            "type": "oino_bus", "topic": topic, "data": data.to_value(), "sticky": sticky, "publisher": publisher.as_str(),
        }));
        for id in targets {
            lock(&self.exts)
                .entry(id.clone())
                .or_default()
                .events
                .insert(EventKind::OinoBus);
            self.deliver_in(group, &id, session_id, EventKind::OinoBus, payload.clone());
        }
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "the caller's services set routes the call (ext-restart)"
    )]
    async fn call_service(
        &self,
        group: Option<Group>,
        caller: &ExtensionId,
        cx: &CallContext,
        target: Option<&ExtensionId>,
        name: &str,
        payload: RawJson,
        timeout: Duration,
    ) -> Result<RawJson, HostError> {
        let target = match target {
            Some(t) => t.clone(),
            None => lock(&self.exts)
                .iter()
                .find(|(_, e)| e.services.contains(name))
                .map(|(id, _)| id.clone())
                .ok_or_else(|| HostError::NotFound(format!("service {name}")))?,
        };
        let registered = lock(&self.exts)
            .get(&target)
            .is_some_and(|e| e.services.contains(name));
        if !registered {
            return Err(HostError::NotFound(format!("service {target}/{name}")));
        }
        let instance = self
            .instance(&target, group)
            .ok_or_else(|| HostError::NotFound(format!("extension {target}")))?;
        let call = instance.handle_service(name, payload, caller, cx.with_kind(CallKind::Service));
        match tokio::time::timeout(timeout, call).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(HostError::Failed(e.message)),
            Err(_) => Err(HostError::Timeout),
        }
    }

    /// Serves the `tasks.*` operations. `via` is the calling session's own host route,
    /// captured so the completion wake survives later re-loads of the extension (G-04).
    fn tasks_op(
        self: &Arc<Self>,
        ext: &ExtensionId,
        cx: &CallContext,
        op: TaskOp,
        via: Option<Weak<dyn HostServices>>,
    ) -> Result<RawJson, HostError> {
        let to_json = |v: serde_json::Value| RawJson::from(v);
        let id = match &op {
            TaskOp::Get(id) | TaskOp::Kill(id) | TaskOp::Logs { id, .. } => Some(id),
            _ => None,
        };
        if let Some(id) = id
            && self
                .tasks
                .get(id)
                .is_some_and(|t| t.session_id != cx.session_id)
        {
            return Err(HostError::NotFound(format!("task {id}")));
        }
        match op {
            TaskOp::Start(spec) => {
                let spec: TaskSpec = spec
                    .decode()
                    .map_err(|e| HostError::InvalidArgument(format!("TaskSpec: {e}")))?;
                let hub = Arc::downgrade(self);
                let id = self.tasks.start(
                    ext,
                    &cx.session_id,
                    &cx.cwd,
                    spec,
                    Arc::new(move |info| {
                        if let Some(hub) = hub.upgrade() {
                            hub.task_finished(info, via.clone());
                        }
                    }),
                )?;
                Ok(to_json(json!(id)))
            }
            TaskOp::AdoptBash { .. } => Err(HostError::unsupported(
                "tasks.adopt-bash without a core host",
            )),
            TaskOp::List => Ok(to_json(
                serde_json::to_value(self.tasks.list(&cx.session_id)).unwrap_or_default(),
            )),
            TaskOp::Get(id) => Ok(to_json(
                self.tasks
                    .get(&id)
                    .map(|t| serde_json::to_value(t).unwrap_or_default())
                    .unwrap_or_default(),
            )),
            TaskOp::Logs {
                id,
                max_bytes,
                tail,
            } => self
                .tasks
                .logs(&id, max_bytes, tail)
                .map(|s| to_json(json!(s))),
            TaskOp::Kill(id) => self.tasks.kill(&id).map(|()| RawJson::null()),
            TaskOp::ClearFinished => {
                self.tasks.clear_finished(&cx.session_id);
                Ok(RawJson::null())
            }
        }
    }

    /// Detaches the running `bash` call `tool_call_id` (through the core host `inner`) and
    /// adopts the process as a background task of `ext` (`control.bash-detach`,
    /// `tasks.adopt-bash`). Returns the task id.
    async fn adopt_bash(
        self: &Arc<Self>,
        inner: &Arc<dyn HostServices>,
        ext: &ExtensionId,
        cx: &CallContext,
        tool_call_id: &str,
        reason: &str,
    ) -> Result<String, HostError> {
        let task_id = self.tasks.next_id();
        let detached = inner
            .take_bash(ext, cx, tool_call_id, &task_id, reason)
            .await?;
        let hub = Arc::downgrade(self);
        let via = Arc::downgrade(inner);
        self.tasks.adopt(
            &task_id,
            ext,
            &cx.session_id,
            detached,
            Arc::new(move |info| {
                if let Some(hub) = hub.upgrade() {
                    hub.task_finished(info, Some(via.clone()));
                }
            }),
        )?;
        Ok(task_id)
    }

    /// A task ended: deliver the `oino_task` event, render the notification (any live
    /// registration of the owner) and wake the starting session through the route the
    /// task was started from (G-04) — the process-global registration of the extension
    /// may point at a newer load that has since died.
    fn task_finished(self: Arc<Self>, info: TaskInfo, via: Option<Weak<dyn HostServices>>) {
        let Ok(owner) = ExtensionId::new(info.owner.clone()) else {
            return;
        };
        let state = serde_json::to_value(info.state).unwrap_or_default();
        let payload = RawJson::from(
            json!({"type": "oino_task", "id": info.id, "state": state, "exitCode": info.exit_code}),
        );
        // The starting load's own set (G-04 route), else the newest awake load.
        let group = via
            .as_ref()
            .and_then(Weak::upgrade)
            .map(|inner| group_of(&inner));
        self.deliver_in(
            group,
            &owner,
            &info.session_id,
            EventKind::OinoTask,
            payload,
        );
        if !info.spec.notify_on_completion {
            return;
        }
        let hub = Arc::clone(&self);
        tokio::spawn(async move {
            // Rendering is best-effort: without a live instance the fallback text goes out
            // and the notification still wakes the agent.
            let mut cx = CallContext {
                session_id: info.session_id.clone(),
                kind: CallKind::Service,
                ..CallContext::default()
            };
            let rendered = match hub.instance(&owner, group) {
                Some(instance) => {
                    cx = lock(&instance.shared.last_cx)
                        .clone()
                        .with_kind(CallKind::Service);
                    cx.session_id.clone_from(&info.session_id);
                    let info_json = RawJson::from(serde_json::to_value(&info).unwrap_or_default());
                    let has_renderer = lock(&hub.exts)
                        .get(&owner)
                        .is_some_and(|e| e.services.contains("tasks.notification"));
                    if has_renderer {
                        instance
                            .handle_service("tasks.notification", info_json, &owner, cx.clone())
                            .await
                            .ok()
                            .and_then(|v| v.to_value().as_str().map(ToOwned::to_owned))
                    } else {
                        None
                    }
                }
                None => None,
            };
            let text = rendered.unwrap_or_else(|| {
                format!(
                    "Background task {} finished with exit code {}",
                    info.spec.name,
                    info.exit_code
                        .map_or_else(|| "none".to_owned(), |c| c.to_string())
                )
            });
            let message = RawJson::from(
                json!({"customType": TASK_NOTIFICATION_TYPE, "content": text, "display": true, "details": {"taskId": info.id}}),
            );
            // The host delivers the notification as a follow-up so it lands after the running
            // turn instead of steering it (`notifyCompletion` → `deliverAs: 'followUp'`).
            let options = SendOptions {
                trigger_turn: info.spec.trigger_on_completion,
                deliver_as: Some(Delivery::FollowUp),
                provenance: None,
            };
            // G-04: the wake goes through the route the task was started from (held weakly
            // by the finish closure), falling back to any live registration of the owner.
            // The old code only knew the process-global "last one wins" entry — when the
            // newest load of the extension died (its services set was disposed), every
            // completion of tasks started by older loads was silently dropped and the
            // agent was never woken.
            let route = via
                .as_ref()
                .and_then(Weak::upgrade)
                .or_else(|| hub.inner(&owner, group));
            if let Some(inner) = route {
                if let Err(error) = inner
                    .session_write(&owner, &cx, SessionWrite::SendMessage { message, options })
                    .await
                {
                    tracing::warn!(
                        task = %info.id,
                        session = %info.session_id,
                        "background-task completion could not be delivered: {error}"
                    );
                }
            } else {
                tracing::warn!(
                    task = %info.id,
                    session = %info.session_id,
                    extension = %owner,
                    "background-task completion lost: no live host route for the owning extension"
                );
            }
        });
    }
}

/// Runs `fut` from synchronous host code (multi-thread runtime: in place; otherwise on a
/// helper executor).
fn block_on_runtime<T>(fut: impl Future<Output = T>) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(fut))
        }
        _ => futures::executor::block_on(fut),
    }
}

/// Per-extension host: local services first, everything else to `inner`.
struct Composite {
    inner: Arc<dyn HostServices>,
    /// The services set this load belongs to ([`group_of`] `inner`).
    group: Group,
    hub: Weak<LocalServices>,
}

impl Composite {
    fn hub(&self) -> Option<Arc<LocalServices>> {
        self.hub.upgrade()
    }
}

impl HostServices for Composite {
    fn check_process(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        command: &str,
        args: &[String],
    ) -> Result<(), HostError> {
        self.inner.check_process(ext, cx, command, args)
    }
    fn subscribe(&self, ext: &ExtensionId, events: &[EventKind]) {
        if let Some(hub) = self.hub() {
            lock(&hub.exts)
                .entry(ext.clone())
                .or_default()
                .events
                .extend(events.iter().copied());
        }
        self.inner.subscribe(ext, events);
    }
    fn unsubscribe(&self, ext: &ExtensionId, events: &[EventKind]) {
        if let Some(hub) = self.hub()
            && let Some(e) = lock(&hub.exts).get_mut(ext)
        {
            for k in events {
                e.events.remove(k);
            }
        }
        self.inner.unsubscribe(ext, events);
    }
    fn register_tool(&self, ext: &ExtensionId, spec: ToolSpec) -> Result<(), HostError> {
        self.inner.register_tool(ext, spec)
    }
    fn unregister_tool(&self, ext: &ExtensionId, name: &str) {
        self.inner.unregister_tool(ext, name);
    }
    fn register_command(&self, ext: &ExtensionId, spec: CommandSpec) -> Result<String, HostError> {
        self.inner.register_command(ext, spec)
    }
    fn unregister_command(&self, ext: &ExtensionId, name: &str) {
        self.inner.unregister_command(ext, name);
    }
    fn register_shortcut(&self, ext: &ExtensionId, spec: ShortcutSpec) -> Result<(), HostError> {
        self.inner.register_shortcut(ext, spec)
    }
    fn unregister_shortcut(&self, ext: &ExtensionId, id: &str) {
        self.inner.unregister_shortcut(ext, id);
    }
    fn register_flag(&self, ext: &ExtensionId, spec: FlagSpec) -> Result<(), HostError> {
        self.inner.register_flag(ext, spec)
    }
    fn get_flag(&self, ext: &ExtensionId, name: &str) -> Option<RawJson> {
        self.inner.get_flag(ext, name)
    }
    fn register_renderer(&self, ext: &ExtensionId, spec: RendererSpec) {
        self.inner.register_renderer(ext, spec);
    }
    fn register_service(&self, ext: &ExtensionId, name: &str) -> Result<(), HostError> {
        if let Some(hub) = self.hub() {
            lock(&hub.exts)
                .entry(ext.clone())
                .or_default()
                .services
                .insert(name.to_owned());
        }
        match self.inner.register_service(ext, name) {
            Err(HostError::Failed(m)) if m.ends_with("not supported by this host") => Ok(()),
            other => other,
        }
    }
    fn set_arbiter_roles(&self, ext: &ExtensionId, roles: ArbiterRoles) {
        self.inner.set_arbiter_roles(ext, roles);
    }
    fn register_settings(&self, ext: &ExtensionId, ns: RawJson) -> Result<(), HostError> {
        self.inner.register_settings(ext, ns)
    }
    fn extension_settings(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        layer: SettingsLayer,
    ) -> Result<RawJson, HostError> {
        self.inner.extension_settings(ext, namespace, layer)
    }
    fn set_extension_setting(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        key: &str,
        value: Option<RawJson>,
        scope: SettingsScope,
    ) -> Result<(), HostError> {
        self.inner
            .set_extension_setting(ext, namespace, key, value, scope)
    }
    fn register_provider(&self, ext: &ExtensionId, cfg: RawJson) -> Result<(), HostError> {
        self.inner.register_provider(ext, cfg)
    }
    fn unregister_provider(&self, ext: &ExtensionId, id: &str) {
        self.inner.unregister_provider(ext, id);
    }
    fn register_virtual_model(&self, ext: &ExtensionId, def: RawJson) -> Result<(), HostError> {
        self.inner.register_virtual_model(ext, def)
    }
    fn unregister_virtual_model(&self, ext: &ExtensionId, provider: &str, id: &str) {
        self.inner.unregister_virtual_model(ext, provider, id);
    }
    fn session_read(&self, cx: &CallContext, q: SessionQuery) -> Result<RawJson, HostError> {
        self.inner.session_read(cx, q)
    }
    fn session_query(
        &self,
        cx: &CallContext,
        query: &oino_extension_api::ops::EntryQuery,
    ) -> Result<RawJson, HostError> {
        self.inner.session_query(cx, query)
    }
    fn session_write(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        w: SessionWrite,
    ) -> BoxFut<'_, Result<Option<String>, HostError>> {
        self.inner.session_write(ext, cx, w)
    }
    fn sessions_index(
        &self,
        ext: &ExtensionId,
        q: SessionsIndexQuery,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        self.inner.sessions_index(ext, q)
    }
    fn control(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: ControlOp,
    ) -> BoxFut<'_, Result<ControlReply, HostError>> {
        match (op, self.hub()) {
            (
                ControlOp::BashDetach {
                    tool_call_id,
                    reason,
                },
                Some(hub),
            ) => {
                let (ext, cx) = (ext.clone(), cx.clone());
                Box::pin(async move {
                    hub.adopt_bash(&self.inner, &ext, &cx, &tool_call_id, &reason)
                        .await
                        .map(ControlReply::String)
                })
            }
            (op, _) => self.inner.control(ext, cx, op),
        }
    }
    fn tool_progress(&self, cx: &CallContext, tool_call_id: &str, partial: RawJson) {
        self.inner.tool_progress(cx, tool_call_id, partial);
    }
    fn models(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: ModelsOp,
    ) -> BoxFut<'_, Result<ModelsReply, HostError>> {
        self.inner.models(ext, cx, op)
    }
    fn ui_dialog(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        req: RawJson,
        cancel: Option<CancellationToken>,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        self.inner.ui_dialog(ext, cx, req, cancel)
    }
    fn ui_fire(&self, ext: &ExtensionId, cx: &CallContext, req: RawJson) -> Result<(), HostError> {
        self.inner.ui_fire(ext, cx, req)
    }
    fn ui_panel(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: PanelOp,
    ) -> Result<Option<String>, HostError> {
        self.inner.ui_panel(ext, cx, op)
    }
    fn ui_editor(&self, cx: &CallContext, op: EditorOp) -> Result<Option<String>, HostError> {
        self.inner.ui_editor(cx, op)
    }
    fn ui_device(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: DeviceOp,
    ) -> BoxFut<'_, Result<Option<RawJson>, HostError>> {
        self.inner.ui_device(ext, cx, op)
    }
    fn agent_spawn(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        spec: RawJson,
    ) -> BoxFut<'_, Result<Arc<dyn AgentHandle>, HostError>> {
        let parent = cx.session_id.clone();
        let spawn = self.inner.agent_spawn(ext, cx, spec);
        Box::pin(async move {
            let handle = spawn.await?;
            if let Some(hub) = self.hub() {
                lock(&hub.agent_parents).insert(handle.id(), parent);
            }
            Ok(handle)
        })
    }

    fn host_service(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        name: &str,
        op: &str,
        payload: RawJson,
        cancel: CancellationToken,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        // Registry first (the hub an embedding host installed services into), then the
        // inner host; a name neither provides is `not-found` (v2 ext-abi §3.2).
        if let Some(service) = self.hub().and_then(|hub| hub.host_service(name)) {
            let op = op.to_owned();
            let cx = cx.clone();
            return Box::pin(async move { service.call_in(&cx, &op, payload, cancel).await });
        }
        if !self
            .inner
            .host_services()
            .iter()
            .any(|provided| provided == name)
        {
            let err = HostError::NotFound(format!("host service {name}"));
            return Box::pin(async { Err(err) });
        }
        self.inner.host_service(ext, cx, name, op, payload, cancel)
    }

    fn host_services(&self) -> Vec<String> {
        let mut names = self.inner.host_services();
        if let Some(hub) = self.hub() {
            names.extend(hub.host_service_names());
        }
        names.sort();
        names.dedup();
        names
    }
    fn agents_list(&self, cx: &CallContext) -> RawJson {
        self.inner.agents_list(cx)
    }
    fn tasks(&self, ext: &ExtensionId, cx: &CallContext, op: TaskOp) -> Result<RawJson, HostError> {
        match (&op, self.hub()) {
            (
                TaskOp::AdoptBash {
                    tool_call_id,
                    reason,
                },
                Some(hub),
            ) => {
                let adopt = hub.adopt_bash(&self.inner, ext, cx, tool_call_id, reason);
                block_on_runtime(adopt).map(|id| RawJson::from(json!(id)))
            }
            // G-04: capture the calling session's own host route so the completion wake
            // does not depend on the process-global registration of the extension.
            (_, Some(hub)) => hub.tasks_op(ext, cx, op, Some(Arc::downgrade(&self.inner))),
            (_, None) => self.inner.tasks(ext, cx, op),
        }
    }
    fn bus_publish(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        topic: &str,
        data: RawJson,
        sticky: bool,
    ) {
        if let Some(hub) = self.hub() {
            hub.publish(Some(self.group), ext, &cx.session_id, topic, &data, sticky);
        }
        // Core mirrors publishes into the frontend topic feed (`_oino/topics`).
        self.inner.bus_publish(ext, cx, topic, data, sticky);
    }
    fn bus_subscribe(&self, ext: &ExtensionId, cx: &CallContext, pattern: &str, on: bool) {
        match self.hub() {
            Some(hub) => {
                let mut exts = lock(&hub.exts);
                let entry = exts.entry(ext.clone()).or_default();
                if on {
                    entry.bus.insert(pattern.to_owned());
                } else {
                    entry.bus.remove(pattern);
                }
            }
            None => self.inner.bus_subscribe(ext, cx, pattern, on),
        }
    }
    fn bus_get(&self, cx: &CallContext, topic: &str) -> Option<RawJson> {
        match self.hub() {
            Some(hub) => lock(&hub.sticky).get(topic).cloned(),
            None => self.inner.bus_get(cx, topic),
        }
    }
    fn service_call(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        target: Option<&ExtensionId>,
        name: &str,
        payload: RawJson,
        timeout: Duration,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        let (ext, cx, target, name) = (ext.clone(), cx.clone(), target.cloned(), name.to_owned());
        Box::pin(async move {
            match self.hub() {
                Some(hub) => {
                    hub.call_service(
                        Some(self.group),
                        &ext,
                        &cx,
                        target.as_ref(),
                        &name,
                        payload,
                        timeout,
                    )
                    .await
                }
                None => {
                    self.inner
                        .service_call(&ext, &cx, target.as_ref(), &name, payload, timeout)
                        .await
                }
            }
        })
    }
    fn services_list(&self) -> Vec<(ExtensionId, String)> {
        match self.hub() {
            Some(hub) => {
                let mut out: Vec<(ExtensionId, String)> = lock(&hub.exts)
                    .iter()
                    .flat_map(|(id, e)| e.services.iter().map(move |s| (id.clone(), s.clone())))
                    .collect();
                out.sort();
                out
            }
            None => self.inner.services_list(),
        }
    }
    fn secret(&self, ext: &ExtensionId, op: SecretOp) -> Result<Option<String>, HostError> {
        match self.hub() {
            Some(hub) => hub.secrets.apply(ext, op),
            None => self.inner.secret(ext, op),
        }
    }
    fn provider_hook(
        &self,
        ext: &ExtensionId,
        hook: ProviderHook,
    ) -> BoxFut<'_, ProviderHookReply> {
        self.inner.provider_hook(ext, hook)
    }
    fn host_context(&self, cx: &CallContext) -> HostContextInfo {
        self.inner.host_context(cx)
    }
    fn deliver_event(
        &self,
        ext: &ExtensionId,
        session_id: &str,
        kind: EventKind,
        payload: RawJson,
    ) {
        match self.hub() {
            // Timers and self-deliveries stay in this load's own services set.
            Some(hub) => hub.deliver_in(Some(self.group), ext, session_id, kind, payload),
            None => self.inner.deliver_event(ext, session_id, kind, payload),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A core stand-in that hands over one detached `bash` call.
    struct DetachingCore;

    impl HostServices for DetachingCore {
        fn take_bash(
            &self,
            _ext: &ExtensionId,
            _cx: &CallContext,
            tool_call_id: &str,
            _task_id: &str,
            reason: &str,
        ) -> BoxFut<'_, Result<oino_extension_api::DetachedBash, HostError>> {
            let known = tool_call_id == "b1";
            let reason = reason.to_owned();
            Box::pin(async move {
                if !known {
                    return Err(HostError::Failed("No running bash call".into()));
                }
                let kill = CancellationToken::new();
                let killed = kill.clone();
                Ok(oino_extension_api::DetachedBash {
                    command: format!("sleep 30 # {reason}"),
                    pid: Some(7),
                    output_so_far: "so far".into(),
                    full_output_path: None,
                    output: Box::pin(futures::stream::empty()),
                    exit: Box::pin(async move {
                        killed.cancelled().await;
                        None
                    }),
                    kill,
                })
            })
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn bash_detach_and_adopt_become_host_tasks() {
        let dir = tempfile::tempdir().unwrap();
        let hub = Arc::new(LocalServices::new(dir.path()));
        let ext = ExtensionId::new("acme.watchdog").unwrap();
        let host = hub.wrap(&ext, Arc::new(DetachingCore));
        let cx = CallContext {
            session_id: "s1".into(),
            ..CallContext::default()
        };
        let reply = host
            .control(
                &ext,
                &cx,
                ControlOp::BashDetach {
                    tool_call_id: "b1".into(),
                    reason: "stuck".into(),
                },
            )
            .await
            .unwrap();
        let ControlReply::String(task) = reply else {
            panic!("task id expected");
        };
        let info = hub.tasks().get(&task).unwrap();
        assert_eq!(info.pid, 7);
        assert_eq!(info.owner, "acme.watchdog");
        assert!(info.spec.command.contains("stuck"));
        assert_eq!(hub.tasks().running("s1"), 1);
        hub.tasks().kill(&task).unwrap();
        let missing = host
            .control(
                &ext,
                &cx,
                ControlOp::BashDetach {
                    tool_call_id: "zz".into(),
                    reason: "x".into(),
                },
            )
            .await;
        assert!(missing.is_err());
        let adopted = host
            .tasks(
                &ext,
                &cx,
                TaskOp::AdoptBash {
                    tool_call_id: "b1".into(),
                    reason: "bg".into(),
                },
            )
            .unwrap();
        let id = adopted.to_value().as_str().unwrap().to_owned();
        assert!(hub.tasks().get(&id).is_some());
    }

    /// ext-restart: a call routes to the caller's own services set, never "the newest
    /// load" of another cwd; without a set of its own it prefers an awake load.
    #[test]
    fn routing_prefers_the_callers_own_set() {
        let a: Arc<dyn HostServices> = Arc::new(DetachingCore);
        let b: Arc<dyn HostServices> = Arc::new(DetachingCore);
        let load = |inner: &Arc<dyn HostServices>| Load {
            group: group_of(inner),
            instance: Weak::new(),
            inner: Arc::downgrade(inner),
        };
        // `a` loaded first, `b` (another cwd) is the newest load.
        let loads = vec![load(&a), load(&b)];
        let get = |l: &Load| l.inner.upgrade();
        let is = |found: Option<Arc<dyn HostServices>>, want: &Arc<dyn HostServices>| {
            found.is_some_and(|f| Arc::ptr_eq(&f, want))
        };
        assert!(is(pick(&loads, Some(group_of(&a)), get, |_| true), &a));
        assert!(is(pick(&loads, Some(group_of(&b)), get, |_| true), &b));
        // No own load: the newest awake one (here `a`, `b` hibernated), else the newest.
        let a_group = group_of(&a);
        assert!(is(pick(&loads, None, get, |l| l.group == a_group), &a));
        assert!(is(pick(&loads, None, get, |_| false), &b));
    }

    #[test]
    fn topics() {
        assert!(topic_matches("a:b", "a:b"));
        assert!(topic_matches("a:*", "a:b"));
        assert!(topic_matches("*", "x"));
        assert!(!topic_matches("a:*", "b:a"));
        assert!(!topic_matches("a", "ab"));
    }
}
