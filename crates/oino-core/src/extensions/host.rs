//! Host calls made by extensions (≈ `HostServices`, extension-api.md §6), as one typed
//! operation enum. `oino-core` implements [`HostApi`]; a WASM host forwards guest imports to it.

use futures::future::BoxFuture;
use oino_ai::{JsonValue, ThinkingLevel, UserContentField};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::api::{CallContext, CommandSpec, FlagSpec, ShortcutSpec, ToolSpec};

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
    /// Invoke another extension's command (`command-invoke`).
    InvokeCommand {
        /// Invocation name.
        name: String,
        /// Arguments.
        args: String,
    },
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
}
