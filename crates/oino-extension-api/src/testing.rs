//! Test doubles (feature `testing`): [`RecordingHostServices`] records every host call and
//! keeps a small in-memory session, bus and secret store; [`FakeExtensionRuntime`] produces
//! [`FakeInstance`]s scripted with Rust closures.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::channel::oneshot;
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use crate::guest::{CompletionItem, InitInput, ToolInvocation};
use crate::json::RawJson;
use crate::manifest::Manifest;
use crate::ops::{
    ControlOp, ControlReply, DeviceOp, EditorOp, HostContextInfo, PanelOp, SecretOp, SessionQuery,
    SessionWrite, SettingsLayer, SettingsScope,
};
use crate::registry::{
    ArbiterRoles, CommandSpec, EventKind, FlagSpec, Registration, RendererSpec, ShortcutSpec,
    ToolSpec,
};
use crate::runtime::{
    BoxFut, ExtensionInstance, ExtensionRuntime, HostServices, LoadedPackage, ProviderInstance,
};
use crate::types::{CallContext, ExtensionError, ExtensionId, HostError};

fn guard<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One recorded host call.
#[derive(Clone, Debug, PartialEq)]
pub struct HostCall {
    /// Method (`register_tool`, `session_write`, `deliver_event`, …).
    pub method: String,
    /// Calling extension, when known.
    pub ext: Option<String>,
    /// Arguments as JSON.
    pub detail: Value,
}

/// Records calls; answers with simple in-memory behaviour.
#[derive(Default)]
pub struct RecordingHostServices {
    calls: Mutex<Vec<HostCall>>,
    entries: Mutex<Vec<Value>>,
    sticky: Mutex<BTreeMap<String, RawJson>>,
    secrets: Mutex<HashMap<String, String>>,
    dialog_answers: Mutex<VecDeque<RawJson>>,
    dialog_waiters: Mutex<VecDeque<oneshot::Sender<RawJson>>>,
    flags: Mutex<HashMap<String, RawJson>>,
    editor: Mutex<String>,
    panels: Mutex<u32>,
    /// Extension settings layers: `(scope, namespace)` → values object.
    ext_settings: Mutex<BTreeMap<(String, String), Value>>,
}

/// Sets dot path `key` in `target` (`None` removes it).
fn set_dotted(target: &mut Value, key: &str, value: Option<Value>) {
    if !target.is_object() {
        *target = json!({});
    }
    let segments: Vec<&str> = key.split('.').collect();
    let Some((last, parents)) = segments.split_last() else {
        return;
    };
    let mut cursor = target;
    for segment in parents {
        let Value::Object(map) = cursor else { return };
        let next = map
            .entry((*segment).to_owned())
            .or_insert_with(|| json!({}));
        if !next.is_object() {
            *next = json!({});
        }
        cursor = next;
    }
    if let Value::Object(map) = cursor {
        match value {
            Some(v) => {
                map.insert((*last).to_owned(), v);
            }
            None => {
                map.remove(*last);
            }
        }
    }
}

fn merge_values(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                merge_values(b.entry(k.clone()).or_insert(Value::Null), v);
            }
        }
        (b, o) => *b = o.clone(),
    }
}

impl std::fmt::Debug for RecordingHostServices {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RecordingHostServices")
            .field("calls", &guard(&self.calls).len())
            .finish_non_exhaustive()
    }
}

impl RecordingHostServices {
    /// A new recorder.
    #[must_use]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    fn record(&self, method: &str, ext: Option<&ExtensionId>, detail: Value) {
        guard(&self.calls).push(HostCall {
            method: method.to_owned(),
            ext: ext.map(ToString::to_string),
            detail,
        });
    }

    /// Every call so far.
    #[must_use]
    pub fn calls(&self) -> Vec<HostCall> {
        guard(&self.calls).clone()
    }

    /// Calls of one method.
    #[must_use]
    pub fn calls_named(&self, method: &str) -> Vec<HostCall> {
        guard(&self.calls)
            .iter()
            .filter(|c| c.method == method)
            .cloned()
            .collect()
    }

    /// Custom entries appended so far (`{type:"custom", customType, data, id}`).
    #[must_use]
    pub fn entries(&self) -> Vec<Value> {
        guard(&self.entries).clone()
    }

    /// Queues an answer for the next `ui.dialog`.
    pub fn queue_dialog_answer(&self, answer: Value) {
        guard(&self.dialog_answers).push_back(RawJson::from(answer));
    }

    /// Answers the oldest pending dialog; returns false when none is waiting.
    pub fn answer_pending_dialog(&self, answer: Value) -> bool {
        let waiter = guard(&self.dialog_waiters).pop_front();
        waiter.is_some_and(|tx| tx.send(RawJson::from(answer)).is_ok())
    }

    /// Number of dialogs currently waiting for an answer.
    #[must_use]
    pub fn pending_dialogs(&self) -> usize {
        guard(&self.dialog_waiters).len()
    }

    /// Values stored by `set-extension-setting` in one scope (`{}` when none).
    #[must_use]
    pub fn extension_setting_layer(&self, scope: SettingsScope, namespace: &str) -> Value {
        guard(&self.ext_settings)
            .get(&(scope_name(scope).to_owned(), namespace.to_owned()))
            .cloned()
            .unwrap_or_else(|| json!({}))
    }

    /// Presets one key of an extension settings layer (as if a user had edited it).
    pub fn preset_extension_setting(
        &self,
        scope: SettingsScope,
        namespace: &str,
        key: &str,
        value: Value,
    ) {
        let mut all = guard(&self.ext_settings);
        let layer = all
            .entry((scope_name(scope).to_owned(), namespace.to_owned()))
            .or_insert_with(|| json!({}));
        set_dotted(layer, key, Some(value));
    }

    /// Sets a CLI flag value.
    pub fn set_flag(&self, name: &str, value: Value) {
        guard(&self.flags).insert(name.to_owned(), RawJson::from(value));
    }
}

fn scope_name(scope: SettingsScope) -> &'static str {
    match scope {
        SettingsScope::Global => "global",
        SettingsScope::Project => "project",
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

impl HostServices for RecordingHostServices {
    fn subscribe(&self, ext: &ExtensionId, events: &[EventKind]) {
        self.record("subscribe", Some(ext), to_json(&events));
    }
    fn unsubscribe(&self, ext: &ExtensionId, events: &[EventKind]) {
        self.record("unsubscribe", Some(ext), to_json(&events));
    }
    fn register_tool(&self, ext: &ExtensionId, spec: ToolSpec) -> Result<(), HostError> {
        self.record("register_tool", Some(ext), to_json(&spec));
        Ok(())
    }
    fn unregister_tool(&self, ext: &ExtensionId, name: &str) {
        self.record("unregister_tool", Some(ext), json!(name));
    }
    fn register_command(&self, ext: &ExtensionId, spec: CommandSpec) -> Result<String, HostError> {
        let name = spec.name.clone();
        self.record("register_command", Some(ext), to_json(&spec));
        Ok(name)
    }
    fn unregister_command(&self, ext: &ExtensionId, name: &str) {
        self.record("unregister_command", Some(ext), json!(name));
    }
    fn register_shortcut(&self, ext: &ExtensionId, spec: ShortcutSpec) -> Result<(), HostError> {
        self.record("register_shortcut", Some(ext), to_json(&spec));
        Ok(())
    }
    fn unregister_shortcut(&self, ext: &ExtensionId, id: &str) {
        self.record("unregister_shortcut", Some(ext), json!(id));
    }
    fn register_flag(&self, ext: &ExtensionId, spec: FlagSpec) -> Result<(), HostError> {
        self.record("register_flag", Some(ext), to_json(&spec));
        Ok(())
    }
    fn get_flag(&self, ext: &ExtensionId, name: &str) -> Option<RawJson> {
        self.record("get_flag", Some(ext), json!(name));
        guard(&self.flags).get(name).cloned()
    }
    fn register_renderer(&self, ext: &ExtensionId, spec: RendererSpec) {
        self.record("register_renderer", Some(ext), to_json(&spec));
    }
    fn register_service(&self, ext: &ExtensionId, name: &str) -> Result<(), HostError> {
        self.record("register_service", Some(ext), json!(name));
        Ok(())
    }
    fn set_arbiter_roles(&self, ext: &ExtensionId, roles: ArbiterRoles) {
        self.record("set_arbiter_roles", Some(ext), to_json(&roles));
    }
    fn register_settings(&self, ext: &ExtensionId, ns: RawJson) -> Result<(), HostError> {
        self.record("register_settings", Some(ext), ns.to_value());
        Ok(())
    }
    fn extension_settings(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        layer: SettingsLayer,
    ) -> Result<RawJson, HostError> {
        self.record(
            "extension_settings",
            Some(ext),
            json!({"namespace": namespace, "layer": layer}),
        );
        let all = guard(&self.ext_settings);
        let get = |scope: SettingsScope| {
            all.get(&(scope_name(scope).to_owned(), namespace.to_owned()))
                .cloned()
                .unwrap_or_else(|| json!({}))
        };
        let value = match layer {
            SettingsLayer::Effective => {
                let mut v = get(SettingsScope::Global);
                merge_values(&mut v, &get(SettingsScope::Project));
                v
            }
            SettingsLayer::Global => get(SettingsScope::Global),
            SettingsLayer::Project => get(SettingsScope::Project),
        };
        Ok(RawJson::from(value))
    }
    fn set_extension_setting(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        key: &str,
        value: Option<RawJson>,
        scope: SettingsScope,
    ) -> Result<(), HostError> {
        let value = value.map(|v| v.to_value());
        self.record(
            "set_extension_setting",
            Some(ext),
            json!({"namespace": namespace, "key": key, "value": value, "scope": scope}),
        );
        let mut all = guard(&self.ext_settings);
        let layer = all
            .entry((scope_name(scope).to_owned(), namespace.to_owned()))
            .or_insert_with(|| json!({}));
        set_dotted(layer, key, value.filter(|v| !v.is_null()));
        Ok(())
    }
    fn register_provider(&self, ext: &ExtensionId, cfg: RawJson) -> Result<(), HostError> {
        self.record("register_provider", Some(ext), cfg.to_value());
        Ok(())
    }
    fn unregister_provider(&self, ext: &ExtensionId, id: &str) {
        self.record("unregister_provider", Some(ext), json!(id));
    }
    fn register_virtual_model(&self, ext: &ExtensionId, def: RawJson) -> Result<(), HostError> {
        self.record("register_virtual_model", Some(ext), def.to_value());
        Ok(())
    }
    fn unregister_virtual_model(&self, ext: &ExtensionId, provider: &str, id: &str) {
        self.record("unregister_virtual_model", Some(ext), json!([provider, id]));
    }

    fn session_read(&self, cx: &CallContext, q: SessionQuery) -> Result<RawJson, HostError> {
        self.record("session_read", None, to_json(&q));
        let value = match q {
            SessionQuery::SessionId => json!(cx.session_id),
            SessionQuery::Entries | SessionQuery::Branch { .. } | SessionQuery::ContextEntries => {
                Value::Array(self.entries())
            }
            SessionQuery::Entry { id } => self
                .entries()
                .into_iter()
                .find(|e| e["id"] == id.as_str())
                .unwrap_or(Value::Null),
            SessionQuery::Header => json!({"type": "session", "version": 3, "id": cx.session_id}),
            _ => Value::Null,
        };
        Ok(RawJson::from(value))
    }

    fn session_write(
        &self,
        ext: &ExtensionId,
        _cx: &CallContext,
        w: SessionWrite,
    ) -> BoxFut<'_, Result<Option<String>, HostError>> {
        let detail = match &w {
            SessionWrite::SetSessionName(n) => json!({"setSessionName": n}),
            SessionWrite::SetLabel { id, label } => json!({"setLabel": [id, label]}),
            SessionWrite::AppendEntry { custom_type, data } => {
                json!({"appendEntry": custom_type, "data": data.as_ref().map(RawJson::to_value)})
            }
            SessionWrite::SendMessage { message, options } => {
                json!({"sendMessage": message.to_value(), "options": to_json(options)})
            }
            SessionWrite::SendUserMessage { content, options } => {
                json!({"sendUserMessage": content.to_value(), "options": to_json(options)})
            }
        };
        self.record("session_write", Some(ext), detail);
        let id = if let SessionWrite::AppendEntry { custom_type, data } = w {
            let mut entries = guard(&self.entries);
            let id = format!("{:08x}", entries.len() + 1);
            entries.push(json!({
                "type": "custom", "id": id, "customType": custom_type,
                "data": data.as_ref().map_or(Value::Null, RawJson::to_value),
            }));
            Some(id)
        } else {
            None
        };
        Box::pin(async move { Ok(id) })
    }

    fn control(
        &self,
        ext: &ExtensionId,
        _cx: &CallContext,
        op: ControlOp,
    ) -> BoxFut<'_, Result<ControlReply, HostError>> {
        self.record("control", Some(ext), json!(op.name()));
        let reply = match op {
            ControlOp::IsIdle | ControlOp::IsProjectTrusted => ControlReply::Bool(true),
            ControlOp::HasPendingMessages => ControlReply::Bool(false),
            ControlOp::ThinkingLevel => ControlReply::String("off".into()),
            ControlOp::SystemPrompt => ControlReply::String(String::new()),
            ControlOp::ActiveTools => ControlReply::Strings(Vec::new()),
            ControlOp::Model | ControlOp::RoutedModel => ControlReply::MaybeJson(None),
            ControlOp::Abort | ControlOp::Shutdown | ControlOp::SetActiveTools(_) => {
                ControlReply::Unit
            }
            _ => ControlReply::Json(RawJson::null()),
        };
        Box::pin(async move { Ok(reply) })
    }

    fn tool_progress(&self, _cx: &CallContext, tool_call_id: &str, partial: RawJson) {
        self.record(
            "tool_progress",
            None,
            json!({"toolCallId": tool_call_id, "partial": partial.to_value()}),
        );
    }

    fn ui_dialog(
        &self,
        ext: &ExtensionId,
        _cx: &CallContext,
        req: RawJson,
        cancel: Option<CancellationToken>,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        self.record("ui_dialog", Some(ext), req.to_value());
        let queued = guard(&self.dialog_answers).pop_front();
        let waiter = if queued.is_none() {
            let (tx, rx) = oneshot::channel();
            guard(&self.dialog_waiters).push_back(tx);
            Some(rx)
        } else {
            None
        };
        Box::pin(async move {
            if let Some(answer) = queued {
                return Ok(answer);
            }
            let Some(rx) = waiter else {
                return Err(HostError::Cancelled);
            };
            let cancelled = async {
                match cancel {
                    Some(token) => token.cancelled().await,
                    None => std::future::pending().await,
                }
            };
            futures::pin_mut!(cancelled);
            match futures::future::select(rx, cancelled).await {
                futures::future::Either::Left((Ok(answer), _)) => Ok(answer),
                futures::future::Either::Left((Err(_), _)) | futures::future::Either::Right(_) => {
                    Err(HostError::Cancelled)
                }
            }
        })
    }

    fn ui_fire(&self, ext: &ExtensionId, _cx: &CallContext, req: RawJson) -> Result<(), HostError> {
        self.record("ui_fire", Some(ext), req.to_value());
        Ok(())
    }

    fn ui_panel(
        &self,
        ext: &ExtensionId,
        _cx: &CallContext,
        op: PanelOp,
    ) -> Result<Option<String>, HostError> {
        let (detail, id) = match op {
            PanelOp::Open(spec) => {
                let mut n = guard(&self.panels);
                *n += 1;
                (json!({"open": spec.to_value()}), Some(format!("panel-{n}")))
            }
            PanelOp::Update { panel_id, spec } => {
                (json!({"update": panel_id, "spec": spec.to_value()}), None)
            }
            PanelOp::Close(id) => (json!({"close": id}), None),
        };
        self.record("ui_panel", Some(ext), detail);
        Ok(id)
    }

    fn ui_editor(&self, _cx: &CallContext, op: EditorOp) -> Result<Option<String>, HostError> {
        self.record("ui_editor", None, json!(format!("{op:?}")));
        let mut editor = guard(&self.editor);
        Ok(match op {
            EditorOp::Text => Some(editor.clone()),
            EditorOp::Set(t) => {
                *editor = t;
                None
            }
            EditorOp::Insert(t) => {
                editor.push_str(&t);
                None
            }
        })
    }

    fn ui_device(
        &self,
        ext: &ExtensionId,
        _cx: &CallContext,
        op: DeviceOp,
    ) -> BoxFut<'_, Result<Option<RawJson>, HostError>> {
        self.record("ui_device", Some(ext), json!(format!("{op:?}")));
        Box::pin(async { Ok(None) })
    }

    fn bus_publish(
        &self,
        ext: &ExtensionId,
        _cx: &CallContext,
        topic: &str,
        data: RawJson,
        sticky: bool,
    ) {
        self.record(
            "bus_publish",
            Some(ext),
            json!({"topic": topic, "data": data.to_value(), "sticky": sticky}),
        );
        if sticky {
            guard(&self.sticky).insert(topic.to_owned(), data);
        }
    }
    fn bus_subscribe(&self, ext: &ExtensionId, _cx: &CallContext, pattern: &str, on: bool) {
        self.record(
            "bus_subscribe",
            Some(ext),
            json!({"pattern": pattern, "on": on}),
        );
    }
    fn bus_get(&self, _cx: &CallContext, topic: &str) -> Option<RawJson> {
        guard(&self.sticky).get(topic).cloned()
    }

    fn secret(&self, ext: &ExtensionId, op: SecretOp) -> Result<Option<String>, HostError> {
        let mut secrets = guard(&self.secrets);
        let key = |n: &str| format!("{ext}/{n}");
        self.record(
            "secret",
            Some(ext),
            json!(format!("{op:?}").split('(').next()),
        );
        Ok(match op {
            SecretOp::Get(name) => secrets.get(&key(&name)).cloned(),
            SecretOp::Set { name, value } => {
                secrets.insert(key(&name), value);
                None
            }
            SecretOp::Delete(name) => secrets.remove(&key(&name)),
        })
    }

    fn host_context(&self, _cx: &CallContext) -> HostContextInfo {
        HostContextInfo {
            frontends: vec!["rpc".into()],
            frontend_focused: true,
            workspace: None,
        }
    }

    fn deliver_event(
        &self,
        ext: &ExtensionId,
        session_id: &str,
        kind: EventKind,
        payload: RawJson,
    ) {
        self.record(
            "deliver_event",
            Some(ext),
            json!({"sessionId": session_id, "kind": kind.wire(), "payload": payload.to_value()}),
        );
    }

    fn service_call(
        &self,
        ext: &ExtensionId,
        _cx: &CallContext,
        target: Option<&ExtensionId>,
        name: &str,
        payload: RawJson,
        _timeout: Duration,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        self.record("service_call", Some(ext), json!({"target": target.map(ToString::to_string), "name": name, "payload": payload.to_value()}));
        Box::pin(async { Err(HostError::NotFound("service".into())) })
    }
}

type EventFn =
    dyn Fn(EventKind, &RawJson, &CallContext) -> Result<Option<RawJson>, String> + Send + Sync;
type ToolFn = dyn Fn(&ToolInvocation, &CallContext) -> Result<RawJson, String> + Send + Sync;
type CommandFn = dyn Fn(&str, &str) -> Result<Option<RawJson>, String> + Send + Sync;

/// A scripted instance.
pub struct FakeInstance {
    manifest: Manifest,
    registration: Registration,
    on_event: Option<Box<EventFn>>,
    run_tool: Option<Box<ToolFn>>,
    run_command: Option<Box<CommandFn>>,
}

impl std::fmt::Debug for FakeInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeInstance")
            .field("id", &self.manifest.id)
            .finish_non_exhaustive()
    }
}

impl FakeInstance {
    /// An instance with `manifest` that registers `registration` at init.
    #[must_use]
    pub fn new(manifest: Manifest, registration: Registration) -> Self {
        Self {
            manifest,
            registration,
            on_event: None,
            run_tool: None,
            run_command: None,
        }
    }

    /// Event handler.
    #[must_use]
    pub fn on_event(
        mut self,
        f: impl Fn(EventKind, &RawJson, &CallContext) -> Result<Option<RawJson>, String>
        + Send
        + Sync
        + 'static,
    ) -> Self {
        self.on_event = Some(Box::new(f));
        self
    }

    /// Tool handler.
    #[must_use]
    pub fn run_tool(
        mut self,
        f: impl Fn(&ToolInvocation, &CallContext) -> Result<RawJson, String> + Send + Sync + 'static,
    ) -> Self {
        self.run_tool = Some(Box::new(f));
        self
    }

    /// Command handler.
    #[must_use]
    pub fn run_command(
        mut self,
        f: impl Fn(&str, &str) -> Result<Option<RawJson>, String> + Send + Sync + 'static,
    ) -> Self {
        self.run_command = Some(Box::new(f));
        self
    }

    fn err(&self, call: &str, message: String) -> ExtensionError {
        ExtensionError::new(self.manifest.id.as_str(), call, message)
    }
}

impl ExtensionInstance for FakeInstance {
    fn id(&self) -> &ExtensionId {
        &self.manifest.id
    }
    fn manifest(&self) -> &Manifest {
        &self.manifest
    }
    fn init(
        &self,
        _input: InitInput,
        _cx: CallContext,
    ) -> BoxFut<'_, Result<Registration, ExtensionError>> {
        let reg = self.registration.clone();
        Box::pin(async move { Ok(reg) })
    }
    fn on_event(
        &self,
        kind: EventKind,
        payload: RawJson,
        cx: CallContext,
    ) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>> {
        let result = self
            .on_event
            .as_ref()
            .map_or(Ok(None), |f| f(kind, &payload, &cx));
        Box::pin(async move { result.map_err(|m| self.err("on-event", m)) })
    }
    fn run_tool(
        &self,
        inv: ToolInvocation,
        _cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        let result = self
            .run_tool
            .as_ref()
            .map_or_else(|| Err("no tools".to_owned()), |f| f(&inv, &cx));
        Box::pin(async move { result.map_err(|m| self.err("run-tool", m)) })
    }
    fn prepare_arguments(&self, _tool: &str, raw: RawJson) -> Result<RawJson, ExtensionError> {
        Ok(raw)
    }
    fn prepare_loadout(
        &self,
        _tool: &str,
        _loadout: RawJson,
    ) -> Result<Option<RawJson>, ExtensionError> {
        Ok(None)
    }
    fn reaches_tools(&self, _tool: &str, _input: RawJson) -> Result<RawJson, ExtensionError> {
        Ok(RawJson::from_json_value(&json!([])))
    }
    fn run_command(
        &self,
        name: &str,
        args: &str,
        _cancel: CancellationToken,
        _cx: CallContext,
    ) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>> {
        let result = self
            .run_command
            .as_ref()
            .map_or(Ok(None), |f| f(name, args));
        Box::pin(async move { result.map_err(|m| self.err("run-command", m)) })
    }
    fn complete_command(
        &self,
        _name: &str,
        _prefix: &str,
        _cx: CallContext,
    ) -> BoxFut<'_, Vec<CompletionItem>> {
        Box::pin(async { Vec::new() })
    }
    fn run_shortcut(&self, _id: &str, _cx: CallContext) -> BoxFut<'_, Result<(), ExtensionError>> {
        Box::pin(async { Ok(()) })
    }
    fn handle_service(
        &self,
        name: &str,
        _payload: RawJson,
        _caller: &ExtensionId,
        _cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        let err = self.err("handle-service", format!("unknown service {name}"));
        Box::pin(async move { Err(err) })
    }
    fn propose_nudge(&self, _cx: CallContext) -> BoxFut<'_, Option<RawJson>> {
        Box::pin(async { None })
    }
    fn nudge_delivered(&self, _id: &str, _cx: CallContext) {}
    fn wait_reason(&self, _cx: CallContext) -> BoxFut<'_, Option<String>> {
        Box::pin(async { None })
    }
    fn render(&self, _request: RawJson, _cx: CallContext) -> Option<RawJson> {
        None
    }
    fn route_model(
        &self,
        _request: RawJson,
        _cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>> {
        let err = self.err("route-model", "not a router".into());
        Box::pin(async move { Err(err) })
    }
    fn complete_input(&self, _request: RawJson, _cx: CallContext) -> BoxFut<'_, Option<RawJson>> {
        Box::pin(async { None })
    }
    fn provider(&self) -> Option<&dyn ProviderInstance> {
        None
    }
    fn shutdown(&self, _reason: &str) -> BoxFut<'_, ()> {
        Box::pin(async {})
    }
}

type Factory = dyn Fn(&LoadedPackage) -> FakeInstance + Send + Sync;

/// Runtime producing [`FakeInstance`]s from a factory.
pub struct FakeExtensionRuntime {
    factory: Box<Factory>,
}

impl std::fmt::Debug for FakeExtensionRuntime {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("FakeExtensionRuntime")
    }
}

impl FakeExtensionRuntime {
    /// A runtime calling `factory` per loaded package.
    #[must_use]
    pub fn new(factory: impl Fn(&LoadedPackage) -> FakeInstance + Send + Sync + 'static) -> Self {
        Self {
            factory: Box::new(factory),
        }
    }
}

impl ExtensionRuntime for FakeExtensionRuntime {
    fn load(
        &self,
        package: LoadedPackage,
        _host: Arc<dyn HostServices>,
    ) -> BoxFut<'_, Result<Arc<dyn ExtensionInstance>, ExtensionError>> {
        let instance: Arc<dyn ExtensionInstance> = Arc::new((self.factory)(&package));
        Box::pin(async move { Ok(instance) })
    }
}
