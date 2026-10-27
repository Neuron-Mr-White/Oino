//! [`LocalServices`]: cross-extension features the WASM host can serve on its own while
//! `oino-core` does not implement them (core-runtime deviations "Deferred"): the event bus,
//! services, secrets, the background-task service and delivery of host-produced `oino_*`
//! events. Every other [`HostServices`] method passes through to the wrapped host.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;

use oino_browser_protocol::{BrowserResult, BrowserScope};
use oino_extension_api::browser::{BrowserHost, BrowserRequest};
use oino_extension_api::ops::{
    ControlOp, ControlReply, DeviceOp, EditorOp, HostContextInfo, ModelsOp, ModelsReply, PanelOp,
    ProviderHook, ProviderHookReply, SecretOp, SendOptions, SessionQuery, SessionWrite,
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

#[derive(Default)]
struct ExtEntry {
    instance: Option<Weak<WasmInstance>>,
    inner: Option<Arc<dyn HostServices>>,
    events: HashSet<EventKind>,
    services: HashSet<String>,
    bus: HashSet<String>,
}

/// Process-wide local services shared by all instances of one [`crate::WasmRuntime`].
pub struct LocalServices {
    exts: Mutex<HashMap<ExtensionId, ExtEntry>>,
    sticky: Mutex<HashMap<String, RawJson>>,
    secrets: SecretStore,
    tasks: Arc<TaskService>,
    browser: std::sync::RwLock<Option<Arc<dyn BrowserHost>>>,
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

impl LocalServices {
    /// Hub rooted at `agent_dir` (secrets file, task output).
    #[must_use]
    pub fn new(agent_dir: &Path) -> Self {
        Self {
            exts: Mutex::new(HashMap::new()),
            sticky: Mutex::new(HashMap::new()),
            secrets: SecretStore::new(agent_dir),
            tasks: Arc::new(TaskService::new(agent_dir)),
            browser: std::sync::RwLock::new(None),
            agent_parents: Mutex::new(HashMap::new()),
        }
    }

    /// Installs the process-wide browser service served to extensions with the `browser`
    /// permission (browser-control.md §8.2); `None` disables it.
    pub fn set_browser(&self, browser: Option<Arc<dyn BrowserHost>>) {
        *self
            .browser
            .write()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = browser;
    }

    /// The browser service, if installed.
    #[must_use]
    pub fn browser(&self) -> Option<Arc<dyn BrowserHost>> {
        self.browser
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
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

    fn browser_scope(&self, cx: &CallContext) -> BrowserScope {
        BrowserScope::session(
            self.root_session(&cx.session_id),
            cx.cwd.display().to_string(),
        )
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

    /// Wraps the host given to extension `id`.
    pub(crate) fn wrap(
        self: &Arc<Self>,
        id: &ExtensionId,
        inner: Arc<dyn HostServices>,
    ) -> Arc<dyn HostServices> {
        lock(&self.exts).entry(id.clone()).or_default().inner = Some(Arc::clone(&inner));
        Arc::new(Composite {
            inner,
            hub: Arc::downgrade(self),
        })
    }

    pub(crate) fn register_instance(self: &Arc<Self>, instance: &Arc<WasmInstance>) {
        let _ = instance.hub.set(Arc::downgrade(self));
        lock(&self.exts)
            .entry(instance.id().clone())
            .or_default()
            .instance = Some(Arc::downgrade(instance));
    }

    /// Records the `init` registration (events and services the hub routes).
    pub(crate) fn note_registration(&self, id: &ExtensionId, reg: &Registration) {
        let mut exts = lock(&self.exts);
        let entry = exts.entry(id.clone()).or_default();
        entry.events.extend(reg.events.iter().copied());
        entry.services.extend(reg.services.iter().cloned());
    }

    fn instance(&self, id: &ExtensionId) -> Option<Arc<WasmInstance>> {
        lock(&self.exts)
            .get(id)
            .and_then(|e| e.instance.as_ref())
            .and_then(Weak::upgrade)
    }

    fn inner(&self, id: &ExtensionId) -> Option<Arc<dyn HostServices>> {
        lock(&self.exts).get(id).and_then(|e| e.inner.clone())
    }

    /// Delivers a host-produced event to `id` when it subscribed to `kind`.
    pub fn deliver(&self, id: &ExtensionId, session_id: &str, kind: EventKind, payload: RawJson) {
        let subscribed = lock(&self.exts)
            .get(id)
            .is_some_and(|e| e.events.contains(&kind));
        if !subscribed {
            return;
        }
        let Some(instance) = self.instance(id) else {
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

    fn publish(
        &self,
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
            self.deliver(&id, session_id, EventKind::OinoBus, payload.clone());
        }
    }

    async fn call_service(
        &self,
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
            .instance(&target)
            .ok_or_else(|| HostError::NotFound(format!("extension {target}")))?;
        let call = instance.handle_service(name, payload, caller, cx.with_kind(CallKind::Service));
        match tokio::time::timeout(timeout, call).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => Err(HostError::Failed(e.message)),
            Err(_) => Err(HostError::Timeout),
        }
    }

    fn tasks_op(
        self: &Arc<Self>,
        ext: &ExtensionId,
        cx: &CallContext,
        op: TaskOp,
    ) -> Result<RawJson, HostError> {
        let to_json = |v: serde_json::Value| RawJson::from(v);
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
                            hub.task_finished(info);
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
        self.tasks.adopt(
            &task_id,
            ext,
            &cx.session_id,
            detached,
            Arc::new(move |info| {
                if let Some(hub) = hub.upgrade() {
                    hub.task_finished(info);
                }
            }),
        )?;
        Ok(task_id)
    }

    fn task_finished(self: Arc<Self>, info: TaskInfo) {
        let Ok(owner) = ExtensionId::new(info.owner.clone()) else {
            return;
        };
        let state = serde_json::to_value(info.state).unwrap_or_default();
        let payload = RawJson::from(
            json!({"type": "oino_task", "id": info.id, "state": state, "exitCode": info.exit_code}),
        );
        self.deliver(&owner, &info.session_id, EventKind::OinoTask, payload);
        if !info.spec.notify_on_completion {
            return;
        }
        let hub = Arc::clone(&self);
        tokio::spawn(async move {
            let Some(instance) = hub.instance(&owner) else {
                return;
            };
            let mut cx = lock(&instance.shared.last_cx)
                .clone()
                .with_kind(CallKind::Service);
            cx.session_id.clone_from(&info.session_id);
            let info_json = RawJson::from(serde_json::to_value(&info).unwrap_or_default());
            let has_renderer = lock(&hub.exts)
                .get(&owner)
                .is_some_and(|e| e.services.contains("tasks.notification"));
            let rendered = if has_renderer {
                instance
                    .handle_service("tasks.notification", info_json, &owner, cx.clone())
                    .await
                    .ok()
                    .and_then(|v| v.to_value().as_str().map(ToOwned::to_owned))
            } else {
                None
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
            let options = SendOptions {
                trigger_turn: info.spec.trigger_on_completion,
                deliver_as: None,
                provenance: None,
            };
            if let Some(inner) = hub.inner(&owner) {
                let _ = inner
                    .session_write(&owner, &cx, SessionWrite::SendMessage { message, options })
                    .await;
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
    hub: Weak<LocalServices>,
}

impl Composite {
    fn hub(&self) -> Option<Arc<LocalServices>> {
        self.hub.upgrade()
    }
}

impl HostServices for Composite {
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
    fn browser_execute(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        request: RawJson,
        cancel: CancellationToken,
    ) -> BoxFut<'_, Result<BrowserResult, HostError>> {
        let Some(hub) = self.hub() else {
            return self.inner.browser_execute(ext, cx, request, cancel);
        };
        let Some(browser) = hub.browser() else {
            return Box::pin(async {
                Ok(BrowserResult::unavailable(
                    "disabled",
                    "the in-app browser is not available in this host",
                ))
            });
        };
        let scope = hub.browser_scope(cx);
        Box::pin(async move {
            let request: BrowserRequest = request
                .decode()
                .map_err(|e| HostError::InvalidArgument(format!("browser command: {e}")))?;
            Ok(browser.execute(scope, request, cancel).await)
        })
    }
    fn browser_status(&self, ext: &ExtensionId, cx: &CallContext) -> RawJson {
        match self.hub().and_then(|h| h.browser().map(|b| (h, b))) {
            Some((hub, browser)) => {
                let scope = hub.browser_scope(cx);
                RawJson::from(
                    serde_json::to_value(browser.status(Some(&scope))).unwrap_or_default(),
                )
            }
            None => self.inner.browser_status(ext, cx),
        }
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
            (_, Some(hub)) => hub.tasks_op(ext, cx, op),
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
            hub.publish(ext, &cx.session_id, topic, &data, sticky);
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
                    hub.call_service(&ext, &cx, target.as_ref(), &name, payload, timeout)
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
            Some(hub) => hub.deliver(ext, session_id, kind, payload),
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
        let ext = ExtensionId::new("unipi.watchdog").unwrap();
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
        assert_eq!(info.owner, "unipi.watchdog");
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

    #[test]
    fn topics() {
        assert!(topic_matches("a:b", "a:b"));
        assert!(topic_matches("a:*", "a:b"));
        assert!(topic_matches("*", "x"));
        assert!(!topic_matches("a:*", "b:a"));
        assert!(!topic_matches("a", "ab"));
    }
}
