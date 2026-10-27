//! Native contract between the runner and a loaded extension instance.
//!
//! This mirrors `oino-extension-api` (extension-api.md §6) closely so the WASM host can
//! implement it directly; once `oino-extension-api` lands these definitions are replaced by
//! re-exports. Event payloads and results are pi-shaped JSON (D5).

use std::path::PathBuf;

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use oino_agent::{AgentToolResult, ToolInvocation, UpdateSink};
use oino_ai::{JsonObject, JsonValue, Model, ProviderEvent};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

/// Stable extension id (manifest `id`).
pub type ExtensionId = String;

/// Manifest `kind` (D15): `-ne` disables only `Extension`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionKind {
    /// A provider component.
    Provider,
    /// Any other extension.
    #[default]
    Extension,
}

/// pi `ExtensionMode` plus ACP.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtensionMode {
    /// Interactive terminal.
    Tui,
    /// RPC.
    Rpc,
    /// JSON event stream.
    Json,
    /// Print mode.
    #[default]
    Print,
    /// ACP adapter.
    Acp,
    /// The GPUI desktop app (in-process backend).
    Desktop,
    /// The GPUI mobile app (in-process backend).
    Mobile,
}

/// What kind of guest call is being served (runtime §7.3).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CallKind {
    /// Event handler.
    #[default]
    Event,
    /// Tool execution.
    Tool,
    /// Slash command.
    Command,
    /// Shortcut.
    Shortcut,
    /// Service request.
    Service,
}

/// Context carried by every guest call.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CallContext {
    /// Session the call belongs to.
    pub session_id: String,
    /// Run mode.
    pub mode: ExtensionMode,
    /// Dialog-capable UI attached.
    pub has_ui: bool,
    /// Session cwd.
    pub cwd: PathBuf,
    /// Nested (child) session.
    pub is_child: bool,
    /// Child role, for nested sessions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub child_role: Option<String>,
    /// Call kind.
    pub call_kind: CallKind,
}

impl CallContext {
    /// The same context for another call kind.
    #[must_use]
    pub fn with_kind(&self, call_kind: CallKind) -> Self {
        Self {
            call_kind,
            ..self.clone()
        }
    }
}

/// Failure of a guest call.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ExtensionCallError {
    /// Error text (pi `error.message`).
    pub message: String,
    /// The guest trapped (instance restarted lazily).
    pub trap: bool,
}

impl ExtensionCallError {
    /// A non-trap error.
    #[must_use]
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            trap: false,
        }
    }
}

/// pi `ToolExposure`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolExposure {
    /// Declared to the model and callable by tools.
    #[default]
    Direct,
    /// Declared to the model only.
    ModelOnly,
    /// Callable from code mode only.
    Codemode,
    /// Declared through tool search.
    Deferred,
    /// Not declared.
    Hidden,
}

/// Tool registered by an extension (pi `ToolDefinition` data part).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    /// Model-facing name.
    pub name: String,
    /// Frontend label.
    #[serde(default)]
    pub label: String,
    /// Model-facing description.
    pub description: String,
    /// JSON schema.
    pub parameters: JsonValue,
    /// One-line prompt snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snippet: Option<String>,
    /// Prompt guidelines.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub prompt_guidelines: Vec<String>,
    /// Exposure.
    #[serde(default)]
    pub exposure: ToolExposure,
    /// Activated on registration (default `true`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_active: Option<bool>,
    /// `sequential` forces sequential batches.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub execution_mode: Option<oino_agent::ExecutionMode>,
    /// Tool namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<JsonValue>,
    /// Annotations (ACP kind, read-only hints, …).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<JsonValue>,
    /// Tools this tool may call (`ctx.executeTool`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reaches_tools: Option<JsonValue>,
    /// Output schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<JsonValue>,
    /// The guest implements `prepare-arguments` for this tool.
    #[serde(default)]
    pub prepare_arguments: bool,
}

/// Slash command (pi `RegisteredCommand` data part).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    /// Command name without `/`.
    pub name: String,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Hidden commands are invokable but not listed.
    #[serde(default)]
    pub hidden: bool,
    /// The guest completes arguments.
    #[serde(default)]
    pub completions: bool,
}

/// Shortcut (ui-protocol §8).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ShortcutSpec {
    /// Stable action id.
    pub id: String,
    /// Default key ids.
    #[serde(default)]
    pub default_keys: Vec<String>,
    /// Description.
    #[serde(default)]
    pub description: String,
}

/// Flag type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagType {
    /// `--name` / `--no-name`.
    Boolean,
    /// `--name <value>`.
    String,
}

/// CLI flag registered by an extension.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FlagSpec {
    /// Long name without `--`.
    pub name: String,
    /// Type.
    #[serde(rename = "type")]
    pub kind: FlagType,
    /// Default value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<JsonValue>,
    /// Help text.
    #[serde(default)]
    pub description: String,
}

/// Everything an extension registers at `init` (dynamic registration uses [`super::host::HostApi`]).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    /// Tools.
    #[serde(default)]
    pub tools: Vec<ToolSpec>,
    /// Commands.
    #[serde(default)]
    pub commands: Vec<CommandSpec>,
    /// Shortcuts.
    #[serde(default)]
    pub shortcuts: Vec<ShortcutSpec>,
    /// Flags.
    #[serde(default)]
    pub flags: Vec<FlagSpec>,
    /// Subscribed event kinds (pi `type` strings or `oino_*`).
    #[serde(default)]
    pub events: Vec<String>,
    /// Provider registrations (extension-api §4.4 JSON).
    #[serde(default)]
    pub providers: Vec<JsonValue>,
    /// Virtual model definitions.
    #[serde(default)]
    pub virtual_models: Vec<JsonValue>,
    /// Render specs (ui-protocol §6.4).
    #[serde(default)]
    pub renderers: Vec<JsonValue>,
    /// APIs this provider component implements (`provides.apis`).
    #[serde(default)]
    pub apis: Vec<String>,
    /// Turn arbiter roles (runtime §7.6).
    #[serde(default)]
    pub arbiter: ArbiterRoles,
    /// Settings namespaces (ui-protocol §6.6 declarations, runtime §10.3).
    #[serde(default)]
    pub settings: Vec<JsonValue>,
}

/// Turn arbiter roles of an extension (runtime §7.6, D9).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArbiterRoles {
    /// Implements `propose-nudge` / `nudge-delivered`.
    #[serde(default)]
    pub nudge_provider: bool,
    /// Implements `wait-reason`.
    #[serde(default)]
    pub wait_source: bool,
}

/// Argument completion item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionItem {
    /// Inserted text.
    pub value: String,
    /// Label shown.
    pub label: String,
    /// Extra description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// Streaming provider implementation of a provider component.
pub trait ProviderInstance: Send + Sync {
    /// Starts a request (`{model, context, options, apiKey?, headers?}` JSON) and streams
    /// provider events.
    fn stream(
        &self,
        request: JsonValue,
        cancel: CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<BoxStream<'static, Result<ProviderEvent, ExtensionCallError>>, ExtensionCallError>,
    >;

    /// Lists models of `provider_id`.
    fn list_models(
        &self,
        provider_id: &str,
        credential: Option<JsonValue>,
    ) -> BoxFuture<'_, Result<Vec<Model>, ExtensionCallError>>;

    /// Runs the provider's OAuth sign-in (`oauth-login`) in `cx` (its dialogs reach that
    /// session's frontend); returns the credential JSON to store.
    fn oauth_login(
        &self,
        provider_id: &str,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<JsonValue, ExtensionCallError>> {
        let _ = (provider_id, cx);
        Box::pin(async { Err(ExtensionCallError::new("OAuth sign-in is not supported")) })
    }

    /// Exchanges the refresh token of a stored OAuth credential (`oauth-refresh`).
    fn oauth_refresh(
        &self,
        provider_id: &str,
        credential: JsonValue,
    ) -> BoxFuture<'_, Result<JsonValue, ExtensionCallError>> {
        let _ = (provider_id, credential);
        Box::pin(async { Err(ExtensionCallError::new("OAuth refresh is not supported")) })
    }
}

/// Skill, prompt and theme directories of an extension package (manifest `resources`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PackageResourceDirs {
    /// Skill directories.
    pub skills: Vec<PathBuf>,
    /// Prompt template directories.
    pub prompts: Vec<PathBuf>,
    /// Theme directories.
    pub themes: Vec<PathBuf>,
}

/// One loaded extension instance (≈ `oino_extension_api::ExtensionInstance`).
pub trait ExtensionInstance: Send + Sync {
    /// Manifest id.
    fn id(&self) -> &str;

    /// Manifest kind.
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::Extension
    }

    /// Path or label used in `extension_error.extensionPath`.
    fn path(&self) -> String {
        self.id().to_owned()
    }

    /// Resource directories the package declares (manifest `resources`, absolute paths under
    /// the package root); loaded like pi package resources (runtime §9.6).
    fn resource_dirs(&self) -> PackageResourceDirs {
        PackageResourceDirs::default()
    }

    /// Initialises the instance and returns its static registrations.
    fn init(&self, cx: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>>;

    /// Handles one subscribed event; `Ok(None)` means "no result".
    fn on_event(
        &self,
        kind: &str,
        payload: JsonValue,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>>;

    /// Runs a registered tool.
    fn run_tool(
        &self,
        tool: &str,
        invocation: ToolInvocation,
        updates: UpdateSink,
        cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<AgentToolResult, ExtensionCallError>>;

    /// pi `prepareArguments` for a registered tool.
    fn prepare_arguments(&self, _tool: &str, raw: JsonObject) -> JsonObject {
        raw
    }

    /// Runs a registered command.
    fn run_command(
        &self,
        name: &str,
        args: &str,
        cancel: CancellationToken,
        cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>>;

    /// Completes command arguments.
    fn complete_command(
        &self,
        _name: &str,
        _prefix: &str,
        _cx: CallContext,
    ) -> BoxFuture<'_, Vec<CompletionItem>> {
        Box::pin(async { Vec::new() })
    }

    /// Runs a registered shortcut.
    fn run_shortcut(
        &self,
        _id: &str,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<(), ExtensionCallError>> {
        Box::pin(async { Ok(()) })
    }

    /// Autocomplete contributor (`complete-input`, ui-protocol §7.3): `{text, cursor}` in,
    /// `[CompletionGroup]` JSON out.
    fn complete_input(
        &self,
        _request: JsonValue,
        _cx: CallContext,
    ) -> BoxFuture<'_, Option<JsonValue>> {
        Box::pin(async { None })
    }

    /// Service names this instance serves (`registry.register-service`).
    fn services(&self) -> Vec<String> {
        Vec::new()
    }

    /// Serves a service request (`handle-service`) from `caller`.
    fn handle_service(
        &self,
        name: &str,
        _payload: JsonValue,
        _caller: &str,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<JsonValue, ExtensionCallError>> {
        let message = format!("service {name} is not implemented");
        Box::pin(async move { Err(ExtensionCallError::new(message)) })
    }

    /// Routes a virtual model (`route-model` export): a pi route request in, a route result
    /// out.
    fn route_model(
        &self,
        _request: JsonValue,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<JsonValue, ExtensionCallError>> {
        Box::pin(async { Err(ExtensionCallError::new("route-model is not implemented")) })
    }

    /// Arbiter `propose-nudge`: `{id?, priority, customType, content, display, details?}`.
    fn propose_nudge(&self, _cx: CallContext) -> BoxFuture<'_, Option<JsonValue>> {
        Box::pin(async { None })
    }

    /// Arbiter `nudge-delivered` for the winning proposal.
    fn nudge_delivered(&self, _id: &str, _cx: CallContext) {}

    /// Arbiter `wait-reason`: why the session should wait instead of settling.
    fn wait_reason(&self, _cx: CallContext) -> BoxFuture<'_, Option<String>> {
        Box::pin(async { None })
    }

    /// Manifest deadline for handlers of `kind` (`limits.hookTimeoutMs`,
    /// `toolCallHookTimeoutMs`); the runner never goes below its own default.
    fn hook_deadline(&self, _kind: &str) -> Option<std::time::Duration> {
        None
    }

    /// The provider implementation, for provider components.
    fn provider(&self) -> Option<&dyn ProviderInstance> {
        None
    }

    /// Shuts the instance down.
    fn shutdown(&self, _reason: &str) -> BoxFuture<'_, ()> {
        Box::pin(async {})
    }
}
