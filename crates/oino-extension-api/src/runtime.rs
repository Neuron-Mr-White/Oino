//! Runtime traits: [`ExtensionRuntime`] loads packages into [`ExtensionInstance`]s, which call
//! back into [`HostServices`] (implemented by `oino-core`).

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_ai::{Model, ProviderEvent};
use tokio_util::sync::CancellationToken;

use crate::guest::{CompletionItem, InitInput, ToolInvocation};
use crate::json::RawJson;
use crate::manifest::Manifest;
use crate::ops::{
    ControlOp, ControlReply, DeviceOp, EditorOp, HostContextInfo, ModelsOp, ModelsReply, PanelOp,
    ProviderHook, ProviderHookReply, SecretOp, SessionQuery, SessionWrite, SessionsIndexQuery,
    SettingsLayer, SettingsScope, TaskOp,
};
use crate::registry::{
    ArbiterRoles, CommandSpec, EventKind, FlagSpec, Registration, RendererSpec, ShortcutSpec,
    ToolSpec,
};
use crate::types::{CallContext, ExtensionError, ExtensionId, HostError};

/// Boxed future alias used across the contract.
pub type BoxFut<'a, T> = BoxFuture<'a, T>;

/// A process taken over from a detached `bash` tool call ([`HostServices::take_bash`]).
pub struct DetachedBash {
    /// The command line.
    pub command: String,
    /// OS process id, when known.
    pub pid: Option<u32>,
    /// Output the tool call returned before the detach marker.
    pub output_so_far: String,
    /// Temp file with the complete output so far (when the call truncated it).
    pub full_output_path: Option<String>,
    /// Further output chunks (stdout and stderr interleaved); ends with the process.
    pub output: BoxStream<'static, Vec<u8>>,
    /// Resolves with the exit code once the process ended (`None` when killed by a signal).
    pub exit: BoxFuture<'static, Option<i32>>,
    /// Cancel to kill the process tree.
    pub kill: CancellationToken,
}

impl std::fmt::Debug for DetachedBash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DetachedBash")
            .field("command", &self.command)
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

/// Component bytes or a path to them.
#[derive(Clone, Debug)]
pub enum ComponentSource {
    /// In-memory component (bundled).
    Bytes(Arc<[u8]>),
    /// Component file on disk.
    Path(PathBuf),
}

/// A validated package ready to load.
#[derive(Clone, Debug)]
pub struct LoadedPackage {
    /// Manifest.
    pub manifest: Manifest,
    /// Component.
    pub component: ComponentSource,
    /// Package root on disk (resources), when any.
    pub root: Option<PathBuf>,
    /// Bundled with Oino (grants such as `"*"` hosts are trusted).
    pub bundled: bool,
}

/// Loads packages into instances.
pub trait ExtensionRuntime: Send + Sync {
    /// Instantiates `package`; `host` serves its imports.
    fn load(
        &self,
        package: LoadedPackage,
        host: Arc<dyn HostServices>,
    ) -> BoxFut<'_, Result<Arc<dyn ExtensionInstance>, ExtensionError>>;
}

/// One loaded extension (one component instance per process, D-principle 5).
pub trait ExtensionInstance: Send + Sync {
    /// Manifest id.
    fn id(&self) -> &ExtensionId;
    /// Manifest.
    fn manifest(&self) -> &Manifest;
    /// `init`.
    fn init(
        &self,
        input: InitInput,
        cx: CallContext,
    ) -> BoxFut<'_, Result<Registration, ExtensionError>>;
    /// `on-event`; `Ok(None)` = no result (pi `undefined`).
    fn on_event(
        &self,
        kind: EventKind,
        payload: RawJson,
        cx: CallContext,
    ) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>>;
    /// `run-tool` → `AgentToolResult` JSON.
    fn run_tool(
        &self,
        inv: ToolInvocation,
        cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    /// `prepare-arguments`.
    ///
    /// # Errors
    ///
    /// Guest trap.
    fn prepare_arguments(&self, tool: &str, raw: RawJson) -> Result<RawJson, ExtensionError>;
    /// `prepare-loadout`.
    ///
    /// # Errors
    ///
    /// Guest trap.
    fn prepare_loadout(
        &self,
        tool: &str,
        loadout: RawJson,
    ) -> Result<Option<RawJson>, ExtensionError>;
    /// `reaches-tools` → `"all"` | `["name", …]`.
    ///
    /// # Errors
    ///
    /// Guest trap.
    fn reaches_tools(&self, tool: &str, input: RawJson) -> Result<RawJson, ExtensionError>;
    /// `run-command`.
    fn run_command(
        &self,
        name: &str,
        args: &str,
        cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFut<'_, Result<Option<RawJson>, ExtensionError>>;
    /// `complete-command`.
    fn complete_command(
        &self,
        name: &str,
        prefix: &str,
        cx: CallContext,
    ) -> BoxFut<'_, Vec<CompletionItem>>;
    /// `run-shortcut`.
    fn run_shortcut(&self, id: &str, cx: CallContext) -> BoxFut<'_, Result<(), ExtensionError>>;
    /// `handle-service`.
    fn handle_service(
        &self,
        name: &str,
        payload: RawJson,
        caller: &ExtensionId,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    /// `propose-nudge`.
    fn propose_nudge(&self, cx: CallContext) -> BoxFut<'_, Option<RawJson>>;
    /// `nudge-delivered`.
    fn nudge_delivered(&self, id: &str, cx: CallContext);
    /// `wait-reason`.
    fn wait_reason(&self, cx: CallContext) -> BoxFut<'_, Option<String>>;
    /// `render` → OUS node JSON.
    fn render(&self, request: RawJson, cx: CallContext) -> Option<RawJson>;
    /// `route-model`.
    fn route_model(
        &self,
        request: RawJson,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    /// `complete-input`.
    fn complete_input(&self, request: RawJson, cx: CallContext) -> BoxFut<'_, Option<RawJson>>;
    /// Provider exports, for components of world `provider-extension`.
    fn provider(&self) -> Option<&dyn ProviderInstance>;
    /// `shutdown`.
    fn shutdown(&self, reason: &str) -> BoxFut<'_, ()>;
}

/// Stream of provider events.
pub type ProviderEventStream = BoxStream<'static, Result<ProviderEvent, ExtensionError>>;

/// Provider exports (`provider.wit`).
pub trait ProviderInstance: Send + Sync {
    /// `stream`: `ProviderStreamRequest` JSON in, events out. Dropping the stream cancels.
    fn stream(&self, request: RawJson) -> BoxFut<'_, Result<ProviderEventStream, ExtensionError>>;
    /// `list-models`.
    fn list_models(
        &self,
        provider_id: &str,
        credential: Option<RawJson>,
    ) -> BoxFut<'_, Result<Vec<Model>, ExtensionError>>;
    /// `oauth-login`; the flow's `ui` dialogs run in `cx` (the session that asked).
    fn oauth_login(
        &self,
        provider_id: &str,
        cx: CallContext,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
    /// `oauth-refresh`.
    fn oauth_refresh(
        &self,
        provider_id: &str,
        credential: RawJson,
    ) -> BoxFut<'_, Result<RawJson, ExtensionError>>;
}

/// A nested agent session (`agents.agent` resource).
pub trait AgentHandle: Send + Sync {
    /// Agent id.
    fn id(&self) -> String;
    /// Child session file.
    fn session_file(&self) -> Option<String>;
    /// `prompt` → disposition.
    fn prompt(
        &self,
        text: String,
        delivery: Option<String>,
    ) -> BoxFut<'_, Result<String, HostError>>;
    /// `steer`.
    fn steer(&self, text: String) -> BoxFut<'_, Result<(), HostError>>;
    /// `follow-up`.
    fn follow_up(&self, text: String) -> BoxFut<'_, Result<(), HostError>>;
    /// `abort`.
    fn abort(&self);
    /// `set-thinking` → clamped level.
    ///
    /// # Errors
    ///
    /// Invalid level.
    fn set_thinking(&self, level: &str) -> Result<String, HostError>;
    /// `last-assistant-text`.
    fn last_assistant_text(&self) -> Option<String>;
    /// `wait-idle`.
    fn wait_idle(&self) -> BoxFut<'_, Result<(), HostError>>;
    /// `usage` (`SessionStats` subset).
    fn usage(&self) -> RawJson;
    /// `events`: child `ClientEvent` JSON stream.
    fn events(&self) -> BoxStream<'static, String>;
    /// `close`.
    fn close(&self, grace: Duration) -> BoxFut<'_, ()>;
}

macro_rules! unsupported {
    ($name:literal) => {
        Err(HostError::unsupported($name))
    };
}

/// Implemented by `oino-core`: one method per WIT import that needs the runtime (grouped by
/// interface). Every method has a default that reports the capability as unsupported, so
/// hosts can grow coverage additively; `testing::RecordingHostServices` (feature `testing`) records all.
#[allow(unused_variables, reason = "default bodies ignore their arguments")]
pub trait HostServices: Send + Sync {
    // ── registry ──
    /// `registry.subscribe`.
    fn subscribe(&self, ext: &ExtensionId, events: &[EventKind]) {}
    /// `registry.unsubscribe`.
    fn unsubscribe(&self, ext: &ExtensionId, events: &[EventKind]) {}
    /// `registry.register-tool`.
    ///
    /// # Errors
    ///
    /// Conflicts, unsupported.
    fn register_tool(&self, ext: &ExtensionId, spec: ToolSpec) -> Result<(), HostError> {
        unsupported!("registry.register-tool")
    }
    /// `registry.unregister-tool`.
    fn unregister_tool(&self, ext: &ExtensionId, name: &str) {}
    /// `registry.register-command` → invocation name.
    ///
    /// # Errors
    ///
    /// Unsupported.
    fn register_command(&self, ext: &ExtensionId, spec: CommandSpec) -> Result<String, HostError> {
        unsupported!("registry.register-command")
    }
    /// `registry.unregister-command`.
    fn unregister_command(&self, ext: &ExtensionId, name: &str) {}
    /// `registry.register-shortcut`.
    ///
    /// # Errors
    ///
    /// Unsupported.
    fn register_shortcut(&self, ext: &ExtensionId, spec: ShortcutSpec) -> Result<(), HostError> {
        unsupported!("registry.register-shortcut")
    }
    /// `registry.unregister-shortcut`.
    fn unregister_shortcut(&self, ext: &ExtensionId, id: &str) {}
    /// `registry.register-flag`.
    ///
    /// # Errors
    ///
    /// Unsupported.
    fn register_flag(&self, ext: &ExtensionId, spec: FlagSpec) -> Result<(), HostError> {
        unsupported!("registry.register-flag")
    }
    /// `registry.get-flag`.
    fn get_flag(&self, ext: &ExtensionId, name: &str) -> Option<RawJson> {
        None
    }
    /// `registry.register-renderer`.
    fn register_renderer(&self, ext: &ExtensionId, spec: RendererSpec) {}
    /// `registry.register-service`.
    ///
    /// # Errors
    ///
    /// Unsupported.
    fn register_service(&self, ext: &ExtensionId, name: &str) -> Result<(), HostError> {
        unsupported!("registry.register-service")
    }
    /// `registry.set-arbiter-roles`.
    fn set_arbiter_roles(&self, ext: &ExtensionId, roles: ArbiterRoles) {}
    /// `registry.register-settings`.
    ///
    /// # Errors
    ///
    /// Unsupported, invalid namespace.
    fn register_settings(&self, ext: &ExtensionId, ns: RawJson) -> Result<(), HostError> {
        unsupported!("registry.register-settings")
    }
    /// `registry.extension-settings`: one namespace's values as `ext` sees them.
    ///
    /// # Errors
    ///
    /// Unsupported, invalid namespace.
    fn extension_settings(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        layer: SettingsLayer,
    ) -> Result<RawJson, HostError> {
        unsupported!("registry.extension-settings")
    }
    /// `registry.set-extension-setting`: writes one key of a namespace `ext` registered.
    ///
    /// # Errors
    ///
    /// Unsupported, not the owner, invalid key or value, untrusted project, I/O.
    fn set_extension_setting(
        &self,
        ext: &ExtensionId,
        namespace: &str,
        key: &str,
        value: Option<RawJson>,
        scope: SettingsScope,
    ) -> Result<(), HostError> {
        unsupported!("registry.set-extension-setting")
    }
    /// `registry.register-provider`.
    ///
    /// # Errors
    ///
    /// Unsupported, invalid config.
    fn register_provider(&self, ext: &ExtensionId, cfg: RawJson) -> Result<(), HostError> {
        unsupported!("registry.register-provider")
    }
    /// `registry.unregister-provider`.
    fn unregister_provider(&self, ext: &ExtensionId, id: &str) {}
    /// `registry.register-virtual-model`.
    ///
    /// # Errors
    ///
    /// Unsupported, invalid definition.
    fn register_virtual_model(&self, ext: &ExtensionId, def: RawJson) -> Result<(), HostError> {
        unsupported!("registry.register-virtual-model")
    }
    /// `registry.unregister-virtual-model`.
    fn unregister_virtual_model(&self, ext: &ExtensionId, provider: &str, id: &str) {}

    // ── session ──
    /// Session reads.
    ///
    /// # Errors
    ///
    /// Stale context, unsupported.
    fn session_read(&self, cx: &CallContext, q: SessionQuery) -> Result<RawJson, HostError> {
        unsupported!("session reads")
    }
    /// Session writes → entry id for `append-entry`.
    fn session_write(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        w: SessionWrite,
    ) -> BoxFut<'_, Result<Option<String>, HostError>> {
        Box::pin(async { unsupported!("session writes") })
    }
    /// Sessions index.
    fn sessions_index(
        &self,
        ext: &ExtensionId,
        q: SessionsIndexQuery,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        Box::pin(async { unsupported!("session.sessions-list") })
    }

    // ── control ──
    /// `control.*`.
    fn control(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: ControlOp,
    ) -> BoxFut<'_, Result<ControlReply, HostError>> {
        let name = op.name();
        Box::pin(async move { Err(HostError::unsupported(&format!("control.{name}"))) })
    }
    /// `control.tool-progress`.
    fn tool_progress(&self, cx: &CallContext, tool_call_id: &str, partial: RawJson) {}

    // ── models ──
    /// `models.*`.
    fn models(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: ModelsOp,
    ) -> BoxFut<'_, Result<ModelsReply, HostError>> {
        Box::pin(async { unsupported!("models") })
    }

    // ── ui ──
    /// `ui.dialog`.
    fn ui_dialog(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        req: RawJson,
        cancel: Option<CancellationToken>,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        Box::pin(async { unsupported!("ui.dialog") })
    }
    /// `ui.fire`.
    ///
    /// # Errors
    ///
    /// Unsupported, invalid request.
    fn ui_fire(&self, ext: &ExtensionId, cx: &CallContext, req: RawJson) -> Result<(), HostError> {
        unsupported!("ui.fire")
    }
    /// Panels → panel id for `Open`.
    ///
    /// # Errors
    ///
    /// Unsupported.
    fn ui_panel(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: PanelOp,
    ) -> Result<Option<String>, HostError> {
        unsupported!("ui panels")
    }
    /// Editor → text for `Text`.
    ///
    /// # Errors
    ///
    /// Unsupported.
    fn ui_editor(&self, cx: &CallContext, op: EditorOp) -> Result<Option<String>, HostError> {
        unsupported!("ui editor")
    }
    /// Device integration.
    fn ui_device(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: DeviceOp,
    ) -> BoxFut<'_, Result<Option<RawJson>, HostError>> {
        Box::pin(async { unsupported!("ui device") })
    }

    // ── agents ──
    /// `agents.agent.spawn` (`AgentSpec` JSON).
    fn agent_spawn(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        spec: RawJson,
    ) -> BoxFut<'_, Result<Arc<dyn AgentHandle>, HostError>> {
        Box::pin(async { unsupported!("agents") })
    }
    /// `agents.list`.
    fn agents_list(&self, cx: &CallContext) -> RawJson {
        RawJson::from_json_value(&serde_json::Value::Array(Vec::new()))
    }

    // ── bash detach ──
    /// Takes the running foreground `bash` call `tool_call_id` of the calling session away
    /// from its tool call (which returns at once with a detach marker naming `task_id`) and
    /// hands the process over; the background-task service adopts it
    /// (`control.bash-detach`, `tasks.adopt-bash`).
    fn take_bash(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        tool_call_id: &str,
        task_id: &str,
        reason: &str,
    ) -> BoxFut<'_, Result<DetachedBash, HostError>> {
        Box::pin(async { unsupported!("bash detach") })
    }

    // ── tasks ──
    /// `tasks.*`.
    ///
    /// # Errors
    ///
    /// Unsupported, not found.
    fn tasks(&self, ext: &ExtensionId, cx: &CallContext, op: TaskOp) -> Result<RawJson, HostError> {
        unsupported!("tasks")
    }

    // ── bus / services / secrets ──
    /// `bus.publish`.
    fn bus_publish(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        topic: &str,
        data: RawJson,
        sticky: bool,
    ) {
    }
    /// `bus.subscribe` / `unsubscribe`.
    fn bus_subscribe(&self, ext: &ExtensionId, cx: &CallContext, pattern: &str, on: bool) {}
    /// `bus.get`.
    fn bus_get(&self, cx: &CallContext, topic: &str) -> Option<RawJson> {
        None
    }
    /// `services.call`.
    fn service_call(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        target: Option<&ExtensionId>,
        name: &str,
        payload: RawJson,
        timeout: Duration,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        Box::pin(async { unsupported!("services.call") })
    }
    /// `services.list`.
    fn services_list(&self) -> Vec<(ExtensionId, String)> {
        Vec::new()
    }
    /// `secrets.*`.
    ///
    /// # Errors
    ///
    /// Unsupported, I/O.
    fn secret(&self, ext: &ExtensionId, op: SecretOp) -> Result<Option<String>, HostError> {
        unsupported!("secrets")
    }

    // ── provider-host ──
    /// Provider-level extension events.
    fn provider_hook(
        &self,
        ext: &ExtensionId,
        hook: ProviderHook,
    ) -> BoxFut<'_, ProviderHookReply> {
        let reply = hook.passthrough();
        Box::pin(async move { reply })
    }

    // ── host-info ──
    /// Runtime parts of host info.
    fn host_context(&self, cx: &CallContext) -> HostContextInfo {
        HostContextInfo::default()
    }

    // ── browser ──
    /// `browser.execute`: one browser command (JSON of
    /// [`crate::browser::BrowserRequest`]) in the caller's scope. Browser failures are
    /// results with `isError`; `Err` is only for permission / transport problems.
    fn browser_execute(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        request: RawJson,
        cancel: CancellationToken,
    ) -> BoxFut<'_, Result<oino_browser_protocol::BrowserResult, HostError>> {
        Box::pin(async { unsupported!("browser") })
    }
    /// `browser.status` (JSON of `BrowserStatus`; disabled when no browser host).
    fn browser_status(&self, ext: &ExtensionId, cx: &CallContext) -> RawJson {
        RawJson::from_json_value(&serde_json::json!({
            "enabled": false, "available": false, "reason": "disabled", "stealth": false, "display": "headless"
        }))
    }

    // ── cron ──
    /// `cron.call`: one scheduler op (unipi-port §3.24) in the caller's scope. The hub
    /// overrides this with the installed [`crate::cron::CronHost`] (or the daemon client)
    /// before it reaches the inner host.
    fn cron_call(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: &str,
        payload: RawJson,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        Box::pin(async { unsupported!("cron") })
    }

    // ── host-produced events ──
    /// Delivers an event produced by the runtime host itself (timers, process exits, tasks).
    fn deliver_event(
        &self,
        ext: &ExtensionId,
        session_id: &str,
        kind: EventKind,
        payload: RawJson,
    ) {
    }
}
