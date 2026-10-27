//! Host calls made by extensions (≈ `HostServices`, extension-api.md §6), as one typed
//! operation enum. `oino-core` implements [`HostApi`]; a WASM host forwards guest imports to it.

use futures::future::BoxFuture;
use oino_ai::{JsonValue, ThinkingLevel, UserContentField};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::api::{ArbiterRoles, CallContext, CommandSpec, FlagSpec, ShortcutSpec, ToolSpec};
use crate::agents::ChildAgent;

/// Delivery of extension messages (pi `deliverAs`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Delivery {
    /// Steering queue.
    Steer,
    /// Follow-up queue.
    FollowUp,
    /// With the next user prompt.
    NextTurn,
}

/// `sendMessage` draft (pi `Pick<CustomMessage, "customType"|"content"|"display"|"details">`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomMessageDraft {
    /// Extension-defined type.
    pub custom_type: String,
    /// Content.
    #[serde(default)]
    pub content: UserContentField,
    /// Render in frontends.
    #[serde(default)]
    pub display: bool,
    /// Metadata.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
}

/// Read-only session queries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "query", rename_all = "kebab-case")]
pub enum SessionQuery {
    /// Header.
    Header,
    /// All entries.
    Entries,
    /// Root → leaf path.
    Branch,
    /// One entry.
    Entry {
        /// Entry id.
        id: String,
    },
    /// Tree.
    Tree,
    /// Projection at the leaf.
    Projection,
    /// Context-selected entries.
    ContextEntries,
    /// Label of an entry.
    Label {
        /// Entry id.
        id: String,
    },
    /// Session name.
    Name,
    /// Leaf id.
    Leaf,
    /// Session id.
    Id,
    /// Session file.
    File,
}

/// Every host operation an extension can perform.
#[allow(
    clippy::large_enum_variant,
    reason = "operations are built once per call and moved, never stored"
)]
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum HostOp {
    /// Subscribe to event kinds.
    Subscribe(Vec<String>),
    /// Unsubscribe from event kinds.
    Unsubscribe(Vec<String>),
    /// Register (or replace own) tool.
    RegisterTool(ToolSpec),
    /// Remove own tool.
    UnregisterTool(String),
    /// Register a command; returns its invocation name.
    RegisterCommand(CommandSpec),
    /// Remove own command.
    UnregisterCommand(String),
    /// Register a shortcut.
    RegisterShortcut(ShortcutSpec),
    /// Remove a shortcut.
    UnregisterShortcut(String),
    /// Register a CLI flag.
    RegisterFlag(FlagSpec),
    /// Read a flag value.
    GetFlag(String),
    /// Register a render spec.
    RegisterRenderer(JsonValue),
    /// Register a provider (extension-api §4.4 JSON).
    RegisterProvider(JsonValue),
    /// Remove a provider.
    UnregisterProvider(String),
    /// Register a virtual model.
    RegisterVirtualModel(JsonValue),
    /// Remove a virtual model.
    UnregisterVirtualModel {
        /// Provider id.
        provider: String,
        /// Model id.
        id: String,
    },
    /// Read session data.
    SessionRead(SessionQuery),
    /// pi `appendEntry(customType, data)`; returns the entry id.
    AppendEntry {
        /// Custom type.
        custom_type: String,
        /// Payload.
        data: Option<JsonValue>,
    },
    /// pi `sendMessage`.
    SendMessage {
        /// Draft.
        message: CustomMessageDraft,
        /// Trigger a turn.
        trigger_turn: bool,
        /// Delivery mode.
        deliver_as: Option<Delivery>,
    },
    /// pi `sendUserMessage`.
    SendUserMessage {
        /// Content.
        content: UserContentField,
        /// Delivery mode while streaming.
        deliver_as: Option<Delivery>,
    },
    /// pi `setSessionName`.
    SetSessionName(String),
    /// pi `setLabel`.
    SetLabel {
        /// Entry id.
        entry_id: String,
        /// Label (`None` clears).
        label: Option<String>,
    },
    /// pi `getActiveTools`.
    GetActiveTools,
    /// pi `getAllTools`.
    GetAllTools,
    /// pi `setActiveTools`.
    SetActiveTools(Vec<String>),
    /// pi `getCommands`.
    GetCommands,
    /// pi `setModel`; returns `true` when auth is configured and the model was set.
    SetModel {
        /// Provider id.
        provider: String,
        /// Model id.
        id: String,
    },
    /// Current model (`null` when none).
    GetModel,
    /// pi `getThinkingLevel`.
    GetThinkingLevel,
    /// pi `setThinkingLevel`.
    SetThinkingLevel(ThinkingLevel),
    /// pi `ctx.isIdle`.
    IsIdle,
    /// pi `ctx.abort`.
    Abort,
    /// pi `ctx.hasPendingMessages`.
    HasPendingMessages,
    /// pi `ctx.getContextUsage`.
    GetContextUsage,
    /// pi `ctx.compact`.
    Compact {
        /// Custom instructions.
        custom_instructions: Option<String>,
    },
    /// pi `ctx.getSystemPrompt`.
    GetSystemPrompt,
    /// pi `getSettings()`: the merged settings object.
    GetSettings,
    /// Models with configured credentials (`control.models-list`).
    GetAvailableModels,
    /// The `--models` / `enabledModels` scope (`control.scoped-models`).
    GetScopedModels,
    /// The model a virtual selection currently routes to (`control.routed-model`).
    GetRoutedModel,
    /// pi `SessionStats` (`control.session-stats`).
    GetSessionStats,
    /// Project trust decision (`control.is-project-trusted`).
    IsProjectTrusted,
    /// `models.complete`: one non-session completion → `AssistantMessage`.
    ModelComplete {
        /// `provider/id` (default: the session model).
        model: Option<String>,
        /// `Context` JSON (`{messages}`).
        context: JsonValue,
        /// `StreamOptions` JSON.
        options: JsonValue,
        /// Caller cancellation.
        cancel: Option<CancellationToken>,
    },
    /// `models.summarize-default`: core's compaction summariser → `{summary, usage}`.
    SummarizeDefault {
        /// `Vec<AgentMessage>` JSON.
        messages: JsonValue,
        /// Extra focus.
        custom_instructions: Option<String>,
    },
    /// pi command `ctx.waitForIdle` (command context only).
    WaitForIdle,
    /// pi command `ctx.navigateTree` (command context only).
    NavigateTree {
        /// Target entry.
        target_id: String,
        /// Summarise the abandoned branch.
        summarize: bool,
        /// Custom instructions.
        custom_instructions: Option<String>,
        /// Replace the default prompt.
        replace_instructions: bool,
        /// Label for the summary or target.
        label: Option<String>,
    },
    /// Raise a dialog (`select`, `confirm`, …) and wait for the answer.
    UiDialog {
        /// Method.
        method: String,
        /// Fields.
        params: oino_ai::JsonObject,
        /// Caller cancel token.
        cancel: Option<CancellationToken>,
    },
    /// Fire-and-forget UI request.
    UiFire {
        /// Method.
        method: String,
        /// Fields.
        params: oino_ai::JsonObject,
    },
    /// pi 1.4 `ctx.executeTool`: runs a callable tool nested under `parent_tool_call_id`.
    ExecuteTool {
        /// Running tool call that makes the call.
        parent_tool_call_id: String,
        /// Tool name.
        name: String,
        /// Arguments.
        args: oino_ai::JsonObject,
    },
    /// Invoke another extension's command (`command-invoke`).
    InvokeCommand {
        /// Invocation name.
        name: String,
        /// Arguments.
        args: String,
    },
    /// Declare turn arbiter roles (`registry.set-arbiter-roles`).
    SetArbiterRoles(ArbiterRoles),
    /// Nested agents of the calling session (`agents.list`).
    ListAgents,
    /// `control.run-state`: `{state, waitingOn?}`.
    GetRunState,
    /// `control.shutdown` (pi `ctx.shutdown()`): asks the frontend to exit when idle.
    Shutdown,
    /// `control.system-prompt-options` (command context only).
    GetSystemPromptOptions,
    /// `control.new-session` (command context only) → `{cancelled}`.
    NewSession {
        /// `parentSession` of the new header.
        parent_session: Option<std::path::PathBuf>,
    },
    /// `control.fork` (command context only) → `{cancelled, selectedText?}`.
    Fork {
        /// Entry to fork from.
        entry_id: String,
        /// Fork at the entry (`true`) or before it.
        at: bool,
    },
    /// `control.switch-session` (command context only) → `{cancelled}`.
    SwitchSession(std::path::PathBuf),
    /// `control.reload` (command context only).
    Reload,
    /// `registry.register-settings`: a ui-protocol §6.6 namespace declaration.
    RegisterSettings(JsonValue),
    /// `registry.extension-settings`: one namespace's values as the caller sees them.
    GetExtensionSettings {
        /// Namespace.
        namespace: String,
        /// Layer.
        layer: crate::extensions::settings::SettingsLayer,
    },
    /// `registry.set-extension-setting`: writes one key of an owned namespace.
    SetExtensionSetting {
        /// Namespace.
        namespace: String,
        /// Dot-path key.
        key: String,
        /// Value (`None` deletes: "use default").
        value: Option<JsonValue>,
        /// Target file.
        scope: oino_settings::SettingsScope,
    },
    /// `{namespace: values}` of every known namespace as the caller sees them (init input).
    GetAllExtensionSettings,
    /// `ui.editor-text`: the composer text last reported by the frontend
    /// (`editor_event`, ui-protocol §7.1).
    EditorText,
    /// `models.credential-for`: `{provider, headers, baseUrl?}` for a provider, or `null`.
    CredentialFor(String),
    /// `bus.publish` mirror: records sticky topics and feeds subscribed frontends
    /// (`_oino/topics`). Served by the host router, independent of any session.
    BusPublish {
        /// Topic.
        topic: String,
        /// Payload.
        data: JsonValue,
        /// Sticky.
        sticky: bool,
    },
    /// `bus.get` (hosts without their own bus): last sticky value or `null`.
    BusGet(String),
    /// `control.running-bash`: `[{toolCallId, command, startedAt, pid?}]` of the session.
    RunningBash,
    /// `host-info.get` frontends: `["desktop", "rpc", …]` attached to the session.
    GetFrontends,
}

impl HostOp {
    /// Bus operations, served by the host router independent of any session.
    #[must_use]
    pub fn is_bus_op(&self) -> bool {
        matches!(self, Self::BusPublish { .. } | Self::BusGet(_))
    }

    /// Operations served by the settings registry, independent of any session.
    #[must_use]
    pub fn is_settings_op(&self) -> bool {
        matches!(
            self,
            Self::RegisterSettings(_)
                | Self::GetExtensionSettings { .. }
                | Self::SetExtensionSetting { .. }
                | Self::GetAllExtensionSettings
                | Self::GetSettings
        )
    }
}

/// A `models.complete` / `models.stream` request.
#[derive(Clone, Debug, Default)]
pub struct ModelRequest {
    /// `provider/id`; the session model when absent.
    pub model: Option<String>,
    /// pi `Context` JSON.
    pub context: JsonValue,
    /// pi `StreamOptions` JSON (`null` = defaults).
    pub options: JsonValue,
    /// Caller cancellation.
    pub cancel: Option<CancellationToken>,
}

/// Failure of a host operation.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum HostError {
    /// The caller's session was replaced or disposed (pi `assertActive`).
    #[error("{0}")]
    StaleContext(String),
    /// Session-replacing operations need a command or shortcut context.
    #[error("This operation is only available in command and shortcut contexts")]
    CommandContextRequired,
    /// Invalid input.
    #[error("{0}")]
    Invalid(String),
    /// Name already registered by another extension.
    #[error("{0}")]
    Conflict(String),
    /// The operation failed.
    #[error("{0}")]
    Failed(String),
}

/// Implemented by `oino-core`; extensions call it with their id and call context.
pub trait HostApi: Send + Sync {
    /// Performs `op` for extension `extension` in the session of `cx`.
    fn call<'a>(
        &'a self,
        extension: &'a str,
        cx: &'a CallContext,
        op: HostOp,
    ) -> BoxFuture<'a, Result<JsonValue, HostError>>;

    /// `models.stream`: streams one completion (`AssistantMessageEvent` JSON items) in the
    /// session of `cx`; same model/credential resolution as [`HostOp::ModelComplete`].
    fn model_stream<'a>(
        &'a self,
        _extension: &'a str,
        _cx: &'a CallContext,
        _request: ModelRequest,
    ) -> BoxFuture<'a, Result<futures::stream::BoxStream<'static, JsonValue>, HostError>> {
        Box::pin(async {
            Err(HostError::Failed(
                "models.stream is not supported by this host".into(),
            ))
        })
    }

    /// `control.bash-detach` / `tasks.adopt-bash`: detaches the running `bash` call
    /// `tool_call_id` of the session of `cx` into background task `task_id`.
    fn detach_bash<'a>(
        &'a self,
        _extension: &'a str,
        _cx: &'a CallContext,
        _tool_call_id: &'a str,
        _task_id: &'a str,
        _reason: &'a str,
    ) -> BoxFuture<'a, Result<crate::session::DetachedBash, HostError>> {
        Box::pin(async {
            Err(HostError::Failed(
                "bash detach is not supported by this host".into(),
            ))
        })
    }

    /// `agents.spawn`: creates a nested `AgentSession` under the session of `cx`
    /// (runtime §12). `spec` is the `AgentSpec` JSON.
    fn spawn_agent<'a>(
        &'a self,
        _extension: &'a str,
        _cx: &'a CallContext,
        _spec: JsonValue,
    ) -> BoxFuture<'a, Result<std::sync::Arc<ChildAgent>, HostError>> {
        Box::pin(async {
            Err(HostError::Failed(
                "agents is not supported by this host".into(),
            ))
        })
    }
}
