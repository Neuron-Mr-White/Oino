//! Bridge to `oino-core`'s extension runner (feature `core`).
//!
//! `oino-core` currently carries its own copy of the extension contract
//! (`oino_core::extensions::{api, host}`, core-runtime deviation 11). Until it switches to
//! `oino-extension-api`, this module adapts both directions:
//!
//! - [`CoreLoader`] implements `oino_core::services::ExtensionLoader`: it loads packages into the
//!   [`WasmRuntime`] and hands core [`CoreInstance`] wrappers.
//! - [`CoreHost`] implements [`HostServices`] over core's `HostApi`, mapping each capability
//!   core supports onto a `HostOp`; capabilities core lacks report `unsupported` (bus, services,
//!   secrets and tasks are served by [`crate::LocalServices`]).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_agent::{AgentToolResult, ToolInvocation as CoreInvocation, UpdateSink};
use oino_ai::{JsonObject, JsonValue, Model, ProviderEvent};
use oino_core::agents::ChildAgent;
use oino_core::extensions::api as ca;
use oino_core::extensions::host::{self as ch, HostApi, HostOp};
use oino_core::extensions::settings as cs;
use oino_core::services::ExtensionLoader;
use oino_extension_api::manifest::ExtensionKind;
use oino_extension_api::ops::{
    ControlOp, ControlReply, DeviceOp, PanelOp, SessionQuery, SessionWrite, SettingsLayer,
    SettingsScope,
};
use oino_extension_api::registry as ar;
use oino_extension_api::{
    AgentHandle, BoxFut, CallContext, CallKind, CancellationToken, EventKind, ExtensionId,
    ExtensionInstance, ExtensionRuntime, HostError, HostServices, InitInput, LoadedPackage, Mode,
    RawJson, ToolInvocation,
};
use serde_json::{Value, json};

use crate::WasmRuntime;
use crate::state::lock;

// ── context / error conversion ───────────────────────────────────────────

fn api_cx(cx: &ca::CallContext) -> CallContext {
    CallContext {
        session_id: cx.session_id.clone(),
        mode: match cx.mode {
            ca::ExtensionMode::Tui => Mode::Tui,
            ca::ExtensionMode::Rpc => Mode::Rpc,
            ca::ExtensionMode::Json => Mode::Json,
            ca::ExtensionMode::Print => Mode::Print,
            ca::ExtensionMode::Acp => Mode::Acp,
        },
        has_ui: cx.has_ui,
        cwd: cx.cwd.clone(),
        is_child: cx.is_child,
        child_role: cx.child_role.clone(),
        kind: match cx.call_kind {
            ca::CallKind::Event => CallKind::Event,
            ca::CallKind::Tool => CallKind::Tool,
            ca::CallKind::Command => CallKind::Command,
            ca::CallKind::Shortcut => CallKind::Shortcut,
            ca::CallKind::Service => CallKind::Service,
        },
        tool_call_id: None,
    }
}

fn core_cx(cx: &CallContext) -> ca::CallContext {
    ca::CallContext {
        session_id: cx.session_id.clone(),
        mode: match cx.mode {
            Mode::Tui | Mode::Desktop | Mode::Mobile => ca::ExtensionMode::Tui,
            Mode::Rpc => ca::ExtensionMode::Rpc,
            Mode::Json => ca::ExtensionMode::Json,
            Mode::Print => ca::ExtensionMode::Print,
            Mode::Acp => ca::ExtensionMode::Acp,
        },
        has_ui: cx.has_ui,
        cwd: cx.cwd.clone(),
        is_child: cx.is_child,
        child_role: cx.child_role.clone(),
        call_kind: match cx.kind {
            CallKind::Tool => ca::CallKind::Tool,
            CallKind::Command => ca::CallKind::Command,
            CallKind::Shortcut => ca::CallKind::Shortcut,
            CallKind::Service => ca::CallKind::Service,
            _ => ca::CallKind::Event,
        },
    }
}

fn host_error(e: ch::HostError) -> HostError {
    match e {
        ch::HostError::StaleContext(_) => HostError::StaleContext,
        ch::HostError::CommandContextRequired => HostError::CommandContextRequired,
        ch::HostError::Invalid(m) => HostError::InvalidArgument(m),
        other => HostError::Failed(other.to_string()),
    }
}

fn call_error(e: &oino_extension_api::ExtensionError) -> ca::ExtensionCallError {
    ca::ExtensionCallError {
        message: e.message.clone(),
        trap: e.trap,
    }
}

/// Runs `fut` to completion from synchronous host-import code.
fn block<T>(fut: impl Future<Output = T>) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| handle.block_on(fut))
        }
        _ => futures::executor::block_on(fut),
    }
}

// ── spec conversion ──────────────────────────────────────────────────────

fn tool_spec(s: &ar::ToolSpec) -> ca::ToolSpec {
    ca::ToolSpec {
        name: s.name.clone(),
        label: s.label.clone(),
        description: s.description.clone(),
        parameters: s.parameters.to_value(),
        prompt_snippet: s.prompt_snippet.clone(),
        prompt_guidelines: s.prompt_guidelines.clone(),
        exposure: match s.exposure {
            ar::Exposure::Direct => ca::ToolExposure::Direct,
            ar::Exposure::ModelOnly => ca::ToolExposure::ModelOnly,
            ar::Exposure::Deferred => ca::ToolExposure::Deferred,
            ar::Exposure::Hidden => ca::ToolExposure::Hidden,
        },
        default_active: Some(s.default_active),
        execution_mode: Some(match s.execution_mode {
            ar::ExecutionMode::Parallel => oino_agent::ExecutionMode::Parallel,
            ar::ExecutionMode::Sequential => oino_agent::ExecutionMode::Sequential,
        }),
        namespace: s
            .namespace
            .as_ref()
            .and_then(|n| serde_json::to_value(n).ok()),
        annotations: Some(
            json!({"kind": s.kind, "hints": s.annotations, "render": s.render.as_ref().map(RawJson::to_value)}),
        ),
        reaches_tools: match &s.reaches_tools {
            ar::ReachesTools::None => None,
            ar::ReachesTools::All => Some(json!("all")),
            ar::ReachesTools::Names(n) => Some(json!(n)),
            ar::ReachesTools::Dynamic => Some(json!("dynamic")),
        },
        output_schema: s.output_schema.as_ref().map(RawJson::to_value),
        prepare_arguments: s.prepare_arguments,
    }
}

fn command_spec(s: &ar::CommandSpec) -> ca::CommandSpec {
    ca::CommandSpec {
        name: s.name.clone(),
        description: (!s.description.is_empty()).then(|| s.description.clone()),
        hidden: s.hidden,
        completions: s.completions,
    }
}

fn shortcut_spec(s: &ar::ShortcutSpec) -> ca::ShortcutSpec {
    ca::ShortcutSpec {
        id: s.id.clone(),
        default_keys: s.default_keys.clone(),
        description: s.description.clone(),
    }
}

fn flag_spec(s: &ar::FlagSpec) -> ca::FlagSpec {
    ca::FlagSpec {
        name: s.name.clone(),
        kind: match s.kind {
            ar::FlagType::Boolean => ca::FlagType::Boolean,
            ar::FlagType::String => ca::FlagType::String,
        },
        default: s.default_value.as_ref().map(RawJson::to_value),
        description: s.description.clone(),
    }
}

// ── HostServices over core HostApi ───────────────────────────────────────

/// Routes `control.tool-progress` to the sink of the running tool call.
#[derive(Default)]
struct ProgressRoutes(Mutex<HashMap<String, UpdateSink>>);

/// [`HostServices`] implemented over `oino_core`'s `HostApi`.
pub struct CoreHost {
    api: Arc<dyn HostApi>,
    progress: Arc<ProgressRoutes>,
    /// Counter for generated panel ids.
    panels: std::sync::atomic::AtomicU64,
}

impl std::fmt::Debug for CoreHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CoreHost")
    }
}

impl CoreHost {
    /// Wraps core's host router.
    #[must_use]
    pub fn new(api: Arc<dyn HostApi>) -> Self {
        Self {
            api,
            progress: Arc::default(),
            panels: std::sync::atomic::AtomicU64::new(0),
        }
    }

    fn call_sync(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: HostOp,
    ) -> Result<JsonValue, HostError> {
        let cx = core_cx(cx);
        block(self.api.call(ext.as_str(), &cx, op)).map_err(host_error)
    }

    fn registry(&self, ext: &ExtensionId, op: HostOp) -> Result<JsonValue, HostError> {
        let cx = CallContext::default();
        self.call_sync(ext, &cx, op)
    }

    async fn call(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: HostOp,
    ) -> Result<JsonValue, HostError> {
        let cx = core_cx(cx);
        self.api
            .call(ext.as_str(), &cx, op)
            .await
            .map_err(host_error)
    }
}

fn wires(events: &[EventKind]) -> Vec<String> {
    events.iter().map(|k| k.wire().to_owned()).collect()
}

fn split_method(req: &RawJson) -> Result<(String, JsonObject), HostError> {
    let Value::Object(mut obj) = req.to_value() else {
        return Err(HostError::InvalidArgument(
            "UI request must be a JSON object".into(),
        ));
    };
    let method = obj
        .remove("method")
        .and_then(|m| m.as_str().map(ToOwned::to_owned))
        .ok_or_else(|| HostError::InvalidArgument("UI request needs `method`".into()))?;
    Ok((method, obj))
}

fn json_reply(v: JsonValue) -> ControlReply {
    ControlReply::Json(RawJson::from(v))
}

/// Converts a core reply into the WIT reply shape of one `control.*` function.
type ReplyShape = fn(JsonValue) -> ControlReply;

/// Maps a `control.*` call to a core [`HostOp`] and the shape of its reply.
#[allow(clippy::too_many_lines, reason = "one arm per control function")]
fn control_op(op: ControlOp, cx: &CallContext) -> Result<(HostOp, ReplyShape), HostError> {
    let name = op.name();
    Ok(match op {
        ControlOp::Model => (HostOp::GetModel, |v| {
            ControlReply::MaybeJson((!v.is_null()).then(|| RawJson::from(v)))
        }),
        ControlOp::SetModel { provider, id } => (HostOp::SetModel { provider, id }, |v| {
            ControlReply::Bool(v.as_bool().unwrap_or(false))
        }),
        ControlOp::ThinkingLevel => (HostOp::GetThinkingLevel, |v| {
            ControlReply::String(v.as_str().unwrap_or("off").to_owned())
        }),
        ControlOp::SetThinkingLevel(level) => {
            let level = level
                .parse()
                .map_err(|_| HostError::InvalidArgument(format!("thinking level `{level}`")))?;
            (HostOp::SetThinkingLevel(level), |v| {
                ControlReply::String(v.as_str().unwrap_or_default().to_owned())
            })
        }
        ControlOp::IsIdle => (HostOp::IsIdle, |v| {
            ControlReply::Bool(v.as_bool().unwrap_or(true))
        }),
        ControlOp::HasPendingMessages => (HostOp::HasPendingMessages, |v| {
            ControlReply::Bool(v.as_bool().unwrap_or(false))
        }),
        ControlOp::Abort => (HostOp::Abort, |_| ControlReply::Unit),
        ControlOp::ContextUsage => (HostOp::GetContextUsage, json_reply),
        ControlOp::SystemPrompt => (HostOp::GetSystemPrompt, |v| {
            ControlReply::String(v.as_str().unwrap_or_default().to_owned())
        }),
        ControlOp::Compact {
            custom_instructions,
        } => (
            HostOp::Compact {
                custom_instructions,
            },
            json_reply,
        ),
        ControlOp::WaitForIdle => (HostOp::WaitForIdle, |_| ControlReply::Unit),
        ControlOp::NavigateTree { target_id, options } => {
            let o = options.to_value();
            let s = |k: &str| o.get(k).and_then(Value::as_str).map(ToOwned::to_owned);
            let b = |k: &str| o.get(k).and_then(Value::as_bool).unwrap_or(false);
            (
                HostOp::NavigateTree {
                    target_id,
                    summarize: b("summarize"),
                    custom_instructions: s("customInstructions"),
                    replace_instructions: b("replaceInstructions"),
                    label: s("label"),
                },
                json_reply,
            )
        }
        ControlOp::ActiveTools => (HostOp::GetActiveTools, |v| {
            ControlReply::Strings(serde_json::from_value(v).unwrap_or_default())
        }),
        ControlOp::SetActiveTools(names) => (HostOp::SetActiveTools(names), |_| ControlReply::Unit),
        ControlOp::AllTools => (HostOp::GetAllTools, json_reply),
        ControlOp::GetSettings => (HostOp::GetSettings, json_reply),
        ControlOp::ModelsList => (HostOp::GetAvailableModels, json_reply),
        ControlOp::ScopedModels => (HostOp::GetScopedModels, json_reply),
        ControlOp::SessionStats => (HostOp::GetSessionStats, json_reply),
        ControlOp::RoutedModel => (HostOp::GetRoutedModel, |v| {
            ControlReply::MaybeJson((!v.is_null()).then(|| RawJson::from(v)))
        }),
        ControlOp::IsProjectTrusted => (HostOp::IsProjectTrusted, |v| {
            ControlReply::Bool(v.as_bool().unwrap_or(false))
        }),
        ControlOp::CommandsList => (HostOp::GetCommands, json_reply),
        ControlOp::CommandInvoke { name, args } => (HostOp::InvokeCommand { name, args }, |v| {
            ControlReply::MaybeJson((!v.is_null()).then(|| RawJson::from(v)))
        }),
        ControlOp::ExecuteTool { name, args, .. } => {
            let parent = cx.tool_call_id.clone().ok_or_else(|| {
                HostError::InvalidArgument("execute-tool needs a tool call".into())
            })?;
            let Value::Object(args) = args.to_value() else {
                return Err(HostError::InvalidArgument(
                    "tool arguments must be an object".into(),
                ));
            };
            (
                HostOp::ExecuteTool {
                    parent_tool_call_id: parent,
                    name,
                    args,
                },
                json_reply,
            )
        }
        ControlOp::RunState => (HostOp::GetRunState, json_reply),
        ControlOp::Shutdown => (HostOp::Shutdown, |_| ControlReply::Unit),
        ControlOp::SystemPromptOptions => (HostOp::GetSystemPromptOptions, json_reply),
        ControlOp::NewSession { parent_session } => (
            HostOp::NewSession {
                parent_session: parent_session.map(PathBuf::from),
            },
            cancelled_reply,
        ),
        ControlOp::Fork { entry_id, at } => (HostOp::Fork { entry_id, at }, json_reply),
        ControlOp::SwitchSession { path } => {
            (HostOp::SwitchSession(PathBuf::from(path)), cancelled_reply)
        }
        ControlOp::Reload => (HostOp::Reload, |_| ControlReply::Unit),
        _ => return Err(HostError::unsupported(&format!("control.{name}"))),
    })
}

/// `{cancelled}` → the WIT `bool` of `new-session` / `switch-session`.
#[allow(
    clippy::needless_pass_by_value,
    reason = "matches the ReplyShape signature"
)]
fn cancelled_reply(v: JsonValue) -> ControlReply {
    ControlReply::Bool(v.get("cancelled").and_then(Value::as_bool).unwrap_or(false))
}

impl HostServices for CoreHost {
    fn subscribe(&self, ext: &ExtensionId, events: &[EventKind]) {
        let _ = self.registry(ext, HostOp::Subscribe(wires(events)));
    }
    fn unsubscribe(&self, ext: &ExtensionId, events: &[EventKind]) {
        let _ = self.registry(ext, HostOp::Unsubscribe(wires(events)));
    }
    fn register_tool(&self, ext: &ExtensionId, spec: ar::ToolSpec) -> Result<(), HostError> {
        self.registry(ext, HostOp::RegisterTool(tool_spec(&spec)))
            .map(|_| ())
    }
    fn unregister_tool(&self, ext: &ExtensionId, name: &str) {
        let _ = self.registry(ext, HostOp::UnregisterTool(name.to_owned()));
    }
    fn register_command(
        &self,
        ext: &ExtensionId,
        spec: ar::CommandSpec,
    ) -> Result<String, HostError> {
        let name = spec.name.clone();
        self.registry(ext, HostOp::RegisterCommand(command_spec(&spec)))
            .map(|v| v.as_str().map_or(name, ToOwned::to_owned))
    }
    fn unregister_command(&self, ext: &ExtensionId, name: &str) {
        let _ = self.registry(ext, HostOp::UnregisterCommand(name.to_owned()));
    }
    fn register_shortcut(
        &self,
        ext: &ExtensionId,
        spec: ar::ShortcutSpec,
    ) -> Result<(), HostError> {
        self.registry(ext, HostOp::RegisterShortcut(shortcut_spec(&spec)))
            .map(|_| ())
    }
    fn unregister_shortcut(&self, ext: &ExtensionId, id: &str) {
        let _ = self.registry(ext, HostOp::UnregisterShortcut(id.to_owned()));
    }
    fn register_flag(&self, ext: &ExtensionId, spec: ar::FlagSpec) -> Result<(), HostError> {
        self.registry(ext, HostOp::RegisterFlag(flag_spec(&spec)))
            .map(|_| ())
    }
    fn get_flag(&self, ext: &ExtensionId, name: &str) -> Option<RawJson> {
        self.registry(ext, HostOp::GetFlag(name.to_owned()))
            .ok()
            .filter(|v| !v.is_null())
            .map(RawJson::from)
    }
    fn register_renderer(&self, ext: &ExtensionId, spec: ar::RendererSpec) {
        let _ = self.registry(
            ext,
            HostOp::RegisterRenderer(serde_json::to_value(spec).unwrap_or_default()),
        );
    }
    fn set_arbiter_roles(&self, ext: &ExtensionId, roles: ar::ArbiterRoles) {
        let _ = self.registry(ext, HostOp::SetArbiterRoles(arbiter_roles(roles)));
    }
    fn register_provider(&self, ext: &ExtensionId, cfg: RawJson) -> Result<(), HostError> {
        self.registry(ext, HostOp::RegisterProvider(cfg.to_value()))
            .map(|_| ())
    }
    fn register_settings(&self, ext: &ExtensionId, ns: RawJson) -> Result<(), HostError> {
        self.registry(ext, HostOp::RegisterSettings(ns.to_value()))
            .map(|_| ())
    }
    fn extension_settings(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        layer: SettingsLayer,
    ) -> Result<RawJson, HostError> {
        let layer = match layer {
            SettingsLayer::Effective => cs::SettingsLayer::Effective,
            SettingsLayer::Global => cs::SettingsLayer::Global,
            SettingsLayer::Project => cs::SettingsLayer::Project,
        };
        self.registry(
            ext,
            HostOp::GetExtensionSettings {
                namespace: namespace.to_owned(),
                layer,
            },
        )
        .map(RawJson::from)
    }
    fn set_extension_setting(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        key: &str,
        value: Option<RawJson>,
        scope: SettingsScope,
    ) -> Result<(), HostError> {
        let scope = match scope {
            SettingsScope::Global => oino_settings::SettingsScope::Global,
            SettingsScope::Project => oino_settings::SettingsScope::Project,
        };
        self.registry(
            ext,
            HostOp::SetExtensionSetting {
                namespace: namespace.to_owned(),
                key: key.to_owned(),
                value: value.map(|v| v.to_value()),
                scope,
            },
        )
        .map(|_| ())
    }
    fn unregister_provider(&self, ext: &ExtensionId, id: &str) {
        let _ = self.registry(ext, HostOp::UnregisterProvider(id.to_owned()));
    }
    fn register_virtual_model(&self, ext: &ExtensionId, def: RawJson) -> Result<(), HostError> {
        self.registry(ext, HostOp::RegisterVirtualModel(def.to_value()))
            .map(|_| ())
    }
    fn unregister_virtual_model(&self, ext: &ExtensionId, provider: &str, id: &str) {
        let _ = self.registry(
            ext,
            HostOp::UnregisterVirtualModel {
                provider: provider.to_owned(),
                id: id.to_owned(),
            },
        );
    }

    fn session_read(&self, cx: &CallContext, q: SessionQuery) -> Result<RawJson, HostError> {
        let q = match q {
            SessionQuery::Header => ch::SessionQuery::Header,
            SessionQuery::SessionId => ch::SessionQuery::Id,
            SessionQuery::SessionFile => ch::SessionQuery::File,
            SessionQuery::LeafId => ch::SessionQuery::Leaf,
            SessionQuery::Entry { id } => ch::SessionQuery::Entry { id },
            SessionQuery::Entries => ch::SessionQuery::Entries,
            SessionQuery::Branch { from: None } => ch::SessionQuery::Branch,
            SessionQuery::Branch { from: Some(_) } => {
                return Err(HostError::unsupported("session.branch(from)"));
            }
            SessionQuery::ContextEntries => ch::SessionQuery::ContextEntries,
            SessionQuery::Projection => ch::SessionQuery::Projection,
            SessionQuery::Tree => ch::SessionQuery::Tree,
            SessionQuery::Label { id } => ch::SessionQuery::Label { id },
            SessionQuery::SessionName => ch::SessionQuery::Name,
        };
        let ext = ExtensionId::new("oino.host").map_err(|e| HostError::Failed(e.to_string()))?;
        self.call_sync(&ext, cx, HostOp::SessionRead(q))
            .map(RawJson::from)
    }

    fn session_write(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        w: SessionWrite,
    ) -> BoxFut<'_, Result<Option<String>, HostError>> {
        let (ext, cx) = (ext.clone(), cx.clone());
        Box::pin(async move {
            let op = match w {
                SessionWrite::SetSessionName(name) => {
                    HostOp::SetSessionName(name.unwrap_or_default())
                }
                SessionWrite::SetLabel { id, label } => HostOp::SetLabel {
                    entry_id: id,
                    label,
                },
                SessionWrite::AppendEntry { custom_type, data } => HostOp::AppendEntry {
                    custom_type,
                    data: data.map(|d| d.to_value()),
                },
                SessionWrite::SendMessage { message, options } => HostOp::SendMessage {
                    message: message
                        .decode()
                        .map_err(|e| HostError::InvalidArgument(format!("message: {e}")))?,
                    trigger_turn: options.trigger_turn,
                    deliver_as: options.deliver_as.map(delivery),
                },
                SessionWrite::SendUserMessage { content, options } => HostOp::SendUserMessage {
                    content: content
                        .decode()
                        .map_err(|e| HostError::InvalidArgument(format!("content: {e}")))?,
                    deliver_as: options.deliver_as.map(delivery),
                },
            };
            let v = self.call(&ext, &cx, op).await?;
            Ok(v.as_str().map(ToOwned::to_owned))
        })
    }

    fn control(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: ControlOp,
    ) -> BoxFut<'_, Result<ControlReply, HostError>> {
        let (ext, cx) = (ext.clone(), cx.clone());
        Box::pin(async move {
            let (core_op, shape) = control_op(op, &cx)?;
            self.call(&ext, &cx, core_op).await.map(shape)
        })
    }

    fn tool_progress(&self, _cx: &CallContext, tool_call_id: &str, partial: RawJson) {
        let sink = lock(&self.progress.0).get(tool_call_id).cloned();
        if let (Some(sink), Ok(result)) = (sink, partial.decode::<AgentToolResult>()) {
            sink.send(result);
        }
    }

    fn ui_dialog(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        req: RawJson,
        cancel: Option<CancellationToken>,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        let (ext, cx) = (ext.clone(), cx.clone());
        Box::pin(async move {
            let (method, params) = split_method(&req)?;
            self.call(
                &ext,
                &cx,
                HostOp::UiDialog {
                    method,
                    params,
                    cancel,
                },
            )
            .await
            .map(RawJson::from)
        })
    }

    fn ui_fire(&self, ext: &ExtensionId, cx: &CallContext, req: RawJson) -> Result<(), HostError> {
        let (method, params) = split_method(&req)?;
        self.call_sync(ext, cx, HostOp::UiFire { method, params })
            .map(|_| ())
    }

    /// Nested agents run as child sessions of the calling session (runtime §12).
    fn agent_spawn(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        spec: RawJson,
    ) -> BoxFut<'_, Result<Arc<dyn AgentHandle>, HostError>> {
        let (ext, cx) = (ext.clone(), core_cx(cx));
        Box::pin(async move {
            let child = self
                .api
                .spawn_agent(ext.as_str(), &cx, spec.to_value())
                .await
                .map_err(host_error)?;
            Ok(Arc::new(CoreAgent(child)) as Arc<dyn AgentHandle>)
        })
    }

    fn agents_list(&self, cx: &CallContext) -> RawJson {
        let ext = ExtensionId::new("oino.host").ok();
        ext.and_then(|ext| self.call_sync(&ext, cx, HostOp::ListAgents).ok())
            .map_or_else(|| RawJson::from(json!([])), RawJson::from)
    }

    /// `models.complete` and `models.summarize-default` run in the calling session
    /// (credentials, virtual-model routing); the other model operations are unsupported.
    fn models(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: oino_extension_api::ops::ModelsOp,
    ) -> BoxFut<'_, Result<oino_extension_api::ops::ModelsReply, HostError>> {
        use oino_extension_api::ops::{ModelsOp, ModelsReply};
        let (ext, cx) = (ext.clone(), cx.clone());
        Box::pin(async move {
            let op = match op {
                ModelsOp::Complete { request, cancel } => {
                    let request = request.to_value();
                    HostOp::ModelComplete {
                        model: request
                            .get("model")
                            .and_then(Value::as_str)
                            .map(ToOwned::to_owned),
                        context: request.get("context").cloned().unwrap_or(Value::Null),
                        options: request.get("options").cloned().unwrap_or(Value::Null),
                        cancel,
                    }
                }
                ModelsOp::SummarizeDefault {
                    messages,
                    custom_instructions,
                } => HostOp::SummarizeDefault {
                    messages: messages.to_value(),
                    custom_instructions,
                },
                _ => return Err(HostError::unsupported("this models operation")),
            };
            self.call(&ext, &cx, op)
                .await
                .map(|v| ModelsReply::Json(RawJson::from(v)))
        })
    }

    /// Panels are retained fire-and-forget requests (`openPanel` / `updatePanel` /
    /// `closePanel`, ui-protocol §5); the panel id defaults to `<ext>/panel-<n>`.
    fn ui_panel(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: PanelOp,
    ) -> Result<Option<String>, HostError> {
        let object = |raw: &RawJson, what: &str| match raw.to_value() {
            Value::Object(obj) => Ok(obj),
            _ => Err(HostError::InvalidArgument(format!(
                "{what} must be a JSON object"
            ))),
        };
        let (method, params, id) = match op {
            PanelOp::Open(spec) => {
                let mut params = object(&spec, "panel")?;
                let id = match params.get("panelId").and_then(Value::as_str) {
                    Some(id) if !id.is_empty() => id.to_owned(),
                    _ => {
                        let n = self
                            .panels
                            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        format!("{ext}/panel-{n}")
                    }
                };
                params.insert("panelId".into(), json!(id));
                ("openPanel", params, Some(id))
            }
            PanelOp::Update { panel_id, spec } => {
                let mut params = object(&spec, "panel update")?;
                params.insert("panelId".into(), json!(panel_id));
                ("updatePanel", params, None)
            }
            PanelOp::Close(panel_id) => {
                let mut params = JsonObject::new();
                params.insert("panelId".into(), json!(panel_id));
                ("closePanel", params, None)
            }
        };
        self.call_sync(
            ext,
            cx,
            HostOp::UiFire {
                method: method.to_owned(),
                params,
            },
        )?;
        Ok(id)
    }

    /// Fire-and-forget device requests (`osNotify`, `openUrl`, `clipboardWrite`) go to the
    /// frontend; clipboard reads and external editing need a round trip (unsupported).
    fn ui_device(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: DeviceOp,
    ) -> BoxFut<'_, Result<Option<RawJson>, HostError>> {
        let request = match op {
            DeviceOp::OsNotify {
                title,
                body,
                urgency,
                action_url,
            } => Ok((
                "osNotify",
                json!({"title": title, "body": body, "urgency": urgency, "actionUrl": action_url}),
            )),
            DeviceOp::OpenUrl(url) => Ok(("openUrl", json!({"url": url}))),
            DeviceOp::ClipboardWriteText(text) => Ok(("clipboardWrite", json!({"text": text}))),
            DeviceOp::ClipboardReadImage => Err(HostError::unsupported("ui.clipboard-read-image")),
            DeviceOp::EditExternally { .. } => Err(HostError::unsupported("ui.edit-externally")),
        };
        let result = request.and_then(|(method, params)| {
            let Value::Object(params) = params else {
                return Ok(None);
            };
            self.call_sync(
                ext,
                cx,
                HostOp::UiFire {
                    method: method.to_owned(),
                    params,
                },
            )
            .map(|_| None)
        });
        Box::pin(async move { result })
    }
}

fn delivery(d: oino_extension_api::ops::Delivery) -> ch::Delivery {
    match d {
        oino_extension_api::ops::Delivery::Steer => ch::Delivery::Steer,
        oino_extension_api::ops::Delivery::FollowUp => ch::Delivery::FollowUp,
        oino_extension_api::ops::Delivery::NextTurn => ch::Delivery::NextTurn,
    }
}

fn arbiter_roles(roles: ar::ArbiterRoles) -> ca::ArbiterRoles {
    ca::ArbiterRoles {
        nudge_provider: roles.nudge_provider,
        wait_source: roles.wait_source,
    }
}

// ── agents ───────────────────────────────────────────────────────────────

/// `agents.agent` over a core [`ChildAgent`]; dropping the last handle closes the agent.
struct CoreAgent(Arc<ChildAgent>);

impl Drop for CoreAgent {
    fn drop(&mut self) {
        if self.0.is_closed() {
            return;
        }
        let child = Arc::clone(&self.0);
        if let Ok(handle) = tokio::runtime::Handle::try_current() {
            handle.spawn(async move { child.close(std::time::Duration::ZERO).await });
        }
    }
}

impl AgentHandle for CoreAgent {
    fn id(&self) -> String {
        self.0.id().to_owned()
    }
    fn session_file(&self) -> Option<String> {
        self.0.session_file().map(|p| p.display().to_string())
    }
    fn prompt(
        &self,
        text: String,
        delivery: Option<String>,
    ) -> BoxFut<'_, Result<String, HostError>> {
        Box::pin(async move {
            self.0
                .prompt(text, delivery.as_deref())
                .await
                .map_err(host_error)
        })
    }
    fn steer(&self, text: String) -> BoxFut<'_, Result<(), HostError>> {
        Box::pin(async move { self.0.steer(text).await.map_err(host_error) })
    }
    fn follow_up(&self, text: String) -> BoxFut<'_, Result<(), HostError>> {
        Box::pin(async move { self.0.follow_up(text).await.map_err(host_error) })
    }
    fn abort(&self) {
        self.0.abort();
    }
    fn set_thinking(&self, level: &str) -> Result<String, HostError> {
        self.0.set_thinking(level).map_err(host_error)
    }
    fn last_assistant_text(&self) -> Option<String> {
        self.0.last_assistant_text()
    }
    fn wait_idle(&self) -> BoxFut<'_, Result<(), HostError>> {
        Box::pin(async move {
            self.0.wait_idle().await;
            Ok(())
        })
    }
    fn usage(&self) -> RawJson {
        RawJson::from(self.0.usage())
    }
    fn events(&self) -> BoxStream<'static, String> {
        Box::pin(self.0.events().map(|event| event.to_string()))
    }
    fn close(&self, grace: std::time::Duration) -> BoxFut<'_, ()> {
        Box::pin(self.0.close(grace))
    }
}

// ── core ExtensionInstance over an api instance ──────────────────────────

/// `oino_core` instance wrapper around a loaded component.
pub struct CoreInstance {
    inner: Arc<dyn ExtensionInstance>,
    api: Arc<dyn HostApi>,
    progress: Arc<ProgressRoutes>,
    prepare: Mutex<HashSet<String>>,
    path: String,
}

impl std::fmt::Debug for CoreInstance {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreInstance")
            .field("id", self.inner.id())
            .finish_non_exhaustive()
    }
}

impl CoreInstance {
    /// Manifest plus `{namespace: values}` of every namespace known so far, as this extension
    /// sees them (its own namespaces register during `init`, so their secrets resolve on the
    /// first `extension-settings` read).
    async fn init_input(&self, cx: &ca::CallContext) -> InitInput {
        let settings = self
            .api
            .call(
                self.inner.id().as_str(),
                cx,
                HostOp::GetAllExtensionSettings,
            )
            .await
            .ok()
            .and_then(|v| RawJson::from_value(&v).ok())
            .unwrap_or_else(|| RawJson::from(json!({})));
        InitInput {
            manifest: RawJson::from_value(self.inner.manifest())
                .unwrap_or_else(|_| RawJson::null()),
            settings,
            ..InitInput::default()
        }
    }
}

impl ca::ExtensionInstance for CoreInstance {
    fn id(&self) -> &str {
        self.inner.id().as_str()
    }

    fn kind(&self) -> ca::ExtensionKind {
        match self.inner.manifest().kind {
            ExtensionKind::Provider => ca::ExtensionKind::Provider,
            ExtensionKind::Extension => ca::ExtensionKind::Extension,
        }
    }

    fn path(&self) -> String {
        self.path.clone()
    }

    fn init(
        &self,
        cx: ca::CallContext,
    ) -> BoxFuture<'_, Result<ca::Registration, ca::ExtensionCallError>> {
        Box::pin(async move {
            let input = self.init_input(&cx).await;
            let reg = self
                .inner
                .init(input, api_cx(&cx).with_kind(CallKind::Init))
                .await
                .map_err(|e| call_error(&e))?;
            lock(&self.prepare).extend(
                reg.tools
                    .iter()
                    .filter(|t| t.prepare_arguments)
                    .map(|t| t.name.clone()),
            );
            Ok(ca::Registration {
                tools: reg.tools.iter().map(tool_spec).collect(),
                commands: reg.commands.iter().map(command_spec).collect(),
                shortcuts: reg.shortcuts.iter().map(shortcut_spec).collect(),
                flags: reg.flags.iter().map(flag_spec).collect(),
                events: wires(&reg.events),
                providers: Vec::new(),
                virtual_models: Vec::new(),
                renderers: reg
                    .renderers
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect(),
                apis: self.inner.manifest().provides.apis.clone(),
                arbiter: arbiter_roles(reg.arbiter),
                settings: reg.settings.iter().map(RawJson::to_value).collect(),
            })
        })
    }

    fn on_event(
        &self,
        kind: &str,
        payload: JsonValue,
        cx: ca::CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ca::ExtensionCallError>> {
        let kind = kind.parse::<EventKind>();
        Box::pin(async move {
            let Ok(kind) = kind else { return Ok(None) };
            let r = self
                .inner
                .on_event(kind, RawJson::from(payload), api_cx(&cx))
                .await
                .map_err(|e| call_error(&e))?;
            Ok(r.map(|v| v.to_value()))
        })
    }

    fn run_tool(
        &self,
        tool: &str,
        invocation: CoreInvocation,
        updates: UpdateSink,
        cancel: CancellationToken,
        cx: ca::CallContext,
    ) -> BoxFuture<'_, Result<AgentToolResult, ca::ExtensionCallError>> {
        let tool = tool.to_owned();
        Box::pin(async move {
            let id = invocation.tool_call_id.clone();
            lock(&self.progress.0).insert(id.clone(), updates);
            let inv = ToolInvocation {
                tool_call_id: id.clone(),
                name: tool,
                args: RawJson::from(Value::Object(invocation.args)),
                parent_tool_call_id: invocation.parent_tool_call_id,
            };
            let mut cx = api_cx(&cx).with_kind(CallKind::Tool);
            cx.tool_call_id = Some(id.clone());
            let result = self.inner.run_tool(inv, cancel, cx).await;
            lock(&self.progress.0).remove(&id);
            let raw = result.map_err(|e| call_error(&e))?;
            raw.decode::<AgentToolResult>()
                .map_err(|e| ca::ExtensionCallError::new(format!("invalid tool result: {e}")))
        })
    }

    fn prepare_arguments(&self, tool: &str, raw: JsonObject) -> JsonObject {
        if !lock(&self.prepare).contains(tool) {
            return raw;
        }
        match self
            .inner
            .prepare_arguments(tool, RawJson::from(Value::Object(raw.clone())))
        {
            Ok(v) => match v.to_value() {
                Value::Object(o) => o,
                _ => raw,
            },
            Err(_) => raw,
        }
    }

    fn run_command(
        &self,
        name: &str,
        args: &str,
        cancel: CancellationToken,
        cx: ca::CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ca::ExtensionCallError>> {
        let (name, args) = (name.to_owned(), args.to_owned());
        Box::pin(async move {
            let r = self
                .inner
                .run_command(
                    &name,
                    &args,
                    cancel,
                    api_cx(&cx).with_kind(CallKind::Command),
                )
                .await
                .map_err(|e| call_error(&e))?;
            Ok(r.map(|v| v.to_value()))
        })
    }

    fn complete_command(
        &self,
        name: &str,
        prefix: &str,
        cx: ca::CallContext,
    ) -> BoxFuture<'_, Vec<ca::CompletionItem>> {
        let (name, prefix) = (name.to_owned(), prefix.to_owned());
        Box::pin(async move {
            self.inner
                .complete_command(&name, &prefix, api_cx(&cx))
                .await
                .into_iter()
                .map(|i| ca::CompletionItem {
                    label: i.label.unwrap_or_else(|| i.value.clone()),
                    value: i.value,
                    description: i.description,
                })
                .collect()
        })
    }

    fn run_shortcut(
        &self,
        id: &str,
        cx: ca::CallContext,
    ) -> BoxFuture<'_, Result<(), ca::ExtensionCallError>> {
        let id = id.to_owned();
        Box::pin(async move {
            self.inner
                .run_shortcut(&id, api_cx(&cx).with_kind(CallKind::Shortcut))
                .await
                .map_err(|e| call_error(&e))
        })
    }

    fn route_model(
        &self,
        request: JsonValue,
        cx: ca::CallContext,
    ) -> BoxFuture<'_, Result<JsonValue, ca::ExtensionCallError>> {
        Box::pin(async move {
            self.inner
                .route_model(
                    RawJson::from(request),
                    api_cx(&cx).with_kind(CallKind::Route),
                )
                .await
                .map(|v| v.to_value())
                .map_err(|e| call_error(&e))
        })
    }

    fn propose_nudge(&self, cx: ca::CallContext) -> BoxFuture<'_, Option<JsonValue>> {
        Box::pin(async move {
            self.inner
                .propose_nudge(api_cx(&cx))
                .await
                .map(|v| v.to_value())
        })
    }

    fn nudge_delivered(&self, id: &str, cx: ca::CallContext) {
        self.inner.nudge_delivered(id, api_cx(&cx));
    }

    fn wait_reason(&self, cx: ca::CallContext) -> BoxFuture<'_, Option<String>> {
        Box::pin(async move { self.inner.wait_reason(api_cx(&cx)).await })
    }

    fn hook_deadline(&self, kind: &str) -> Option<std::time::Duration> {
        let limits = &self.inner.manifest().limits;
        let ms = if kind == "tool_call" {
            limits.tool_call_hook_timeout_ms
        } else {
            limits.hook_timeout_ms
        };
        Some(std::time::Duration::from_millis(ms))
    }

    fn provider(&self) -> Option<&dyn ca::ProviderInstance> {
        self.inner
            .provider()
            .map(|_| self as &dyn ca::ProviderInstance)
    }

    fn shutdown(&self, reason: &str) -> BoxFuture<'_, ()> {
        let reason = reason.to_owned();
        Box::pin(async move { self.inner.shutdown(&reason).await })
    }
}

impl ca::ProviderInstance for CoreInstance {
    fn stream(
        &self,
        request: JsonValue,
        cancel: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<
            BoxStream<'static, Result<ProviderEvent, ca::ExtensionCallError>>,
            ca::ExtensionCallError,
        >,
    > {
        Box::pin(async move {
            let provider = self
                .inner
                .provider()
                .ok_or_else(|| ca::ExtensionCallError::new("not a provider"))?;
            let stream = provider
                .stream(RawJson::from(request))
                .await
                .map_err(|e| call_error(&e))?;
            let stream = stream
                .map(|r| r.map_err(|e| call_error(&e)))
                .take_until(cancel.cancelled_owned());
            Ok(Box::pin(stream) as BoxStream<'static, _>)
        })
    }

    fn list_models(
        &self,
        provider_id: &str,
        credential: Option<JsonValue>,
    ) -> BoxFuture<'_, Result<Vec<Model>, ca::ExtensionCallError>> {
        let provider_id = provider_id.to_owned();
        Box::pin(async move {
            let provider = self
                .inner
                .provider()
                .ok_or_else(|| ca::ExtensionCallError::new("not a provider"))?;
            provider
                .list_models(&provider_id, credential.map(RawJson::from))
                .await
                .map_err(|e| call_error(&e))
        })
    }
}

// ── loader ───────────────────────────────────────────────────────────────

/// A package plus where it was found (project packages are trust-gated).
#[derive(Clone, Debug)]
pub struct SourcedPackage {
    /// The package.
    pub package: LoadedPackage,
    /// Found under the project `.oino/extensions` (skipped when the project is untrusted).
    pub project: bool,
}

/// `oino_core::services::ExtensionLoader` backed by a [`WasmRuntime`].
pub struct CoreLoader {
    runtime: Arc<WasmRuntime>,
    packages: Vec<SourcedPackage>,
    /// Load errors from the last `load` (`extension_error` material for the caller).
    pub errors: Mutex<Vec<oino_extension_api::ExtensionError>>,
}

impl std::fmt::Debug for CoreLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoreLoader")
            .field("packages", &self.packages.len())
            .finish_non_exhaustive()
    }
}

impl CoreLoader {
    /// Loader for `packages` in load order.
    #[must_use]
    pub fn new(runtime: Arc<WasmRuntime>, packages: Vec<SourcedPackage>) -> Self {
        Self {
            runtime,
            packages,
            errors: Mutex::new(Vec::new()),
        }
    }

    /// Packages from `global_dirs` (always) and `project_dirs` (trust-gated), discovered with
    /// [`crate::packages::discover`].
    #[must_use]
    pub fn discover(
        runtime: Arc<WasmRuntime>,
        global_dirs: &[PathBuf],
        project_dirs: &[PathBuf],
    ) -> Self {
        let (global, _) = crate::packages::discover(global_dirs, false);
        let (project, _) = crate::packages::discover(project_dirs, false);
        let packages = global
            .into_iter()
            .map(|package| SourcedPackage {
                package,
                project: false,
            })
            .chain(project.into_iter().map(|package| SourcedPackage {
                package,
                project: true,
            }))
            .collect();
        Self::new(runtime, packages)
    }
}

impl ExtensionLoader for CoreLoader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ca::ExtensionInstance>>> {
        Box::pin(async move {
            let core_host = Arc::new(CoreHost::new(host));
            let mut out: Vec<Arc<dyn ca::ExtensionInstance>> = Vec::new();
            for sourced in &self.packages {
                if sourced.project && !project_trusted {
                    continue;
                }
                let path = sourced.package.root.as_ref().map_or_else(
                    || sourced.package.manifest.id.to_string(),
                    |r| r.to_string_lossy().into_owned(),
                );
                let services: Arc<dyn HostServices> =
                    Arc::clone(&core_host) as Arc<dyn HostServices>;
                match self.runtime.load(sourced.package.clone(), services).await {
                    Ok(inner) => out.push(Arc::new(CoreInstance {
                        inner,
                        api: Arc::clone(&core_host.api),
                        progress: Arc::clone(&core_host.progress),
                        prepare: Mutex::new(HashSet::new()),
                        path,
                    })),
                    Err(e) => lock(&self.errors).push(e),
                }
            }
            out
        })
    }
}
