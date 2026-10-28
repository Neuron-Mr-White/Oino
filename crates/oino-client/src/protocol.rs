//! RPC records (pi `rpc-types.ts` + Oino `_oino/*`, modes.md §3–§4).
//!
//! Commands are `{id?, type, …fields}` objects; replies are
//! `{id?, type:"response", command, success, data? | error}`. The same [`RpcCommand`] enum
//! travels in-process (no serialisation) and over JSONL.

use oino_agent::QueueMode;
use oino_ai::{ImageContent, JsonObject, JsonValue, Model, ThinkingLevel};
pub use oino_core::agents::{AgentInfo, AgentRunState};
use oino_core::extensions::settings::SettingsLayer;
pub use oino_core::session::{
    AuthProviderInfo, Checkpoint, CheckpointFile, CompletionEntry, CompletionGroup, EditorEvent,
    EditorIntent, ExtensionInfo, QueueEdit, QueueKind, RewindFailure, RewindResult, RunningBash,
};
use oino_core::session::{NavigateTreeOptions, StreamingBehavior};
pub use oino_core::topics::TopicValue;
pub use oino_settings::SettingsScope;
use serde::ser::SerializeMap;
use serde::{Deserialize, Serialize, Serializer};

/// pi command types, in `rpc-types.ts` order.
pub const PI_COMMAND_TYPES: [&str; 33] = [
    "prompt",
    "steer",
    "follow_up",
    "abort",
    "new_session",
    "get_state",
    "set_model",
    "cycle_model",
    "get_available_models",
    "set_thinking_level",
    "cycle_thinking_level",
    "get_available_thinking_levels",
    "set_steering_mode",
    "set_follow_up_mode",
    "compact",
    "set_auto_compaction",
    "set_auto_retry",
    "abort_retry",
    "bash",
    "abort_bash",
    "get_session_stats",
    "export_html",
    "switch_session",
    "fork",
    "clone",
    "get_fork_messages",
    "get_entries",
    "get_tree",
    "get_last_assistant_text",
    "set_session_name",
    "get_messages",
    "get_commands",
    "clear_queue",
];

/// pi image attachments travel as `{type:"image", data, mimeType}`.
mod wire_images {
    use oino_ai::ImageContent;
    use serde::ser::SerializeSeq;
    use serde::{Deserialize, Deserializer, Serializer};
    use serde_json::json;

    pub(super) fn serialize<S: Serializer>(
        images: &[ImageContent],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let mut seq = serializer.serialize_seq(Some(images.len()))?;
        for image in images {
            seq.serialize_element(
                &json!({"type": "image", "data": image.data, "mimeType": image.mime_type}),
            )?;
        }
        seq.end()
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<ImageContent>, D::Error> {
        Ok(Option::<Vec<ImageContent>>::deserialize(deserializer)?.unwrap_or_default())
    }
}

/// A pi RPC command (pi `RpcCommand` without `id`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PiCommand {
    /// Send a prompt; replies once it was handled, queued or started.
    #[serde(rename_all = "camelCase")]
    Prompt {
        /// Text.
        message: String,
        /// Attachments.
        #[serde(default, with = "wire_images", skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
        /// Queueing while streaming.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        streaming_behavior: Option<StreamingBehavior>,
    },
    /// Queue a steering message.
    Steer {
        /// Text.
        message: String,
        /// Attachments.
        #[serde(default, with = "wire_images", skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
    },
    /// Queue a follow-up message.
    FollowUp {
        /// Text.
        message: String,
        /// Attachments.
        #[serde(default, with = "wire_images", skip_serializing_if = "Vec::is_empty")]
        images: Vec<ImageContent>,
    },
    /// Abort the current operation and wait for idle.
    Abort,
    /// Start a new session.
    #[serde(rename_all = "camelCase")]
    NewSession {
        /// Parent session file recorded in the header.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent_session: Option<String>,
    },
    /// Session state.
    GetState,
    /// Select a model.
    #[serde(rename_all = "camelCase")]
    SetModel {
        /// Provider id.
        provider: String,
        /// Model id.
        model_id: String,
    },
    /// Cycle to the next model.
    CycleModel,
    /// Models with credentials.
    GetAvailableModels,
    /// Set the thinking level.
    SetThinkingLevel {
        /// Level.
        level: ThinkingLevel,
    },
    /// Cycle the thinking level.
    CycleThinkingLevel,
    /// Levels the model supports.
    GetAvailableThinkingLevels,
    /// Steering drain mode.
    SetSteeringMode {
        /// Mode.
        mode: QueueMode,
    },
    /// Follow-up drain mode.
    SetFollowUpMode {
        /// Mode.
        mode: QueueMode,
    },
    /// Manual compaction.
    #[serde(rename_all = "camelCase")]
    Compact {
        /// Summariser instructions.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        custom_instructions: Option<String>,
    },
    /// Toggle automatic compaction.
    SetAutoCompaction {
        /// Enabled.
        enabled: bool,
    },
    /// Toggle automatic retry.
    SetAutoRetry {
        /// Enabled.
        enabled: bool,
    },
    /// Cancel a pending retry.
    AbortRetry,
    /// Run a `!` command.
    #[serde(rename_all = "camelCase")]
    Bash {
        /// Command line.
        command: String,
        /// `!!`: keep out of the model context.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        exclude_from_context: Option<bool>,
    },
    /// Cancel running `!` commands.
    AbortBash,
    /// Token and cost statistics.
    GetSessionStats,
    /// Export the branch as HTML.
    #[serde(rename_all = "camelCase")]
    ExportHtml {
        /// Output file.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_path: Option<String>,
    },
    /// Open another session file.
    #[serde(rename_all = "camelCase")]
    SwitchSession {
        /// Session file.
        session_path: String,
    },
    /// Fork before a user message.
    #[serde(rename_all = "camelCase")]
    Fork {
        /// User message entry.
        entry_id: String,
    },
    /// Clone the session at its leaf.
    Clone,
    /// User messages available for forking.
    GetForkMessages,
    /// Session entries (optionally after `since`).
    GetEntries {
        /// Entry id to start after.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        since: Option<String>,
    },
    /// Session tree.
    GetTree,
    /// Text of the last assistant message.
    GetLastAssistantText,
    /// Set the session display name.
    SetSessionName {
        /// Name.
        name: String,
    },
    /// Transcript messages.
    GetMessages,
    /// Slash commands invocable through `prompt`.
    GetCommands,
    /// Clear both queues.
    ClearQueue,
}

/// `_oino/hello` client identity.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClientInfo {
    /// Client name.
    #[serde(default)]
    pub name: String,
    /// Client version.
    #[serde(default)]
    pub version: String,
}

/// Oino RPC extensions (modes.md §4) implemented by this server.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum OinoCommand {
    /// Opt into Oino events and UI methods.
    #[serde(rename = "_oino/hello")]
    Hello {
        /// Client identity.
        #[serde(default)]
        client: ClientInfo,
        /// UI capabilities (ui-protocol §10).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        ui: Option<JsonValue>,
        /// Protocol version (1).
        #[serde(default = "protocol_version")]
        protocol: u32,
    },
    /// Retained fire-and-forget UI state plus open dialogs.
    #[serde(rename = "_oino/ui_snapshot")]
    UiSnapshot,
    /// pi `navigateTree`.
    #[serde(rename = "_oino/navigate_tree", rename_all = "camelCase")]
    NavigateTree {
        /// Target entry.
        target_id: String,
        /// Options.
        #[serde(flatten)]
        options: NavigateTreeOptions,
    },
    /// Set or clear an entry label.
    #[serde(rename = "_oino/set_label", rename_all = "camelCase")]
    SetLabel {
        /// Entry id.
        entry_id: String,
        /// Label (absent clears).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
    /// Import a JSONL session and switch to it.
    #[serde(rename = "_oino/import_jsonl")]
    ImportJsonl {
        /// File to import.
        path: String,
    },
    /// Export the branch as JSONL.
    #[serde(rename = "_oino/export_jsonl", rename_all = "camelCase")]
    ExportJsonl {
        /// Output file.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_path: Option<String>,
    },
    /// Context window usage.
    #[serde(rename = "_oino/get_context_usage")]
    GetContextUsage,
    /// Merged completions at the cursor (ui-protocol §7.3) → `{groups}`.
    #[serde(rename = "_oino/get_completions")]
    GetCompletions {
        /// Composer text.
        text: String,
        /// Cursor (character offset into `text`).
        cursor: usize,
    },
    /// Panel button / list / input action (ui-protocol §6.1) for the panel's owner.
    #[serde(rename = "_oino/ui_action", rename_all = "camelCase")]
    UiAction {
        /// Panel id.
        panel_id: String,
        /// Action id.
        action_id: String,
        /// Item id, input text, bool or selected value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<JsonValue>,
    },
    /// Composer intent (ui-protocol §7.2) → `{consumed}`.
    #[serde(rename = "_oino/editor_event")]
    EditorEvent(EditorEvent),
    /// Run an extension shortcut (ui-protocol §8). The field is `shortcutId` because `id`
    /// is the RPC correlation id.
    #[serde(rename = "_oino/invoke_shortcut", rename_all = "camelCase")]
    InvokeShortcut {
        /// Shortcut action id.
        shortcut_id: String,
    },
    /// Extension shortcuts → `{shortcuts: [{id, defaultKeys, description, extensionId}]}`.
    #[serde(rename = "_oino/get_shortcuts")]
    GetShortcuts,
    /// Extension settings schemas → `{namespaces}` (ui-protocol §6.6).
    #[serde(rename = "_oino/get_settings_schema")]
    GetSettingsSchema,
    /// One namespace's values (secrets masked) → `{values}`.
    #[serde(rename = "_oino/get_extension_settings")]
    GetExtensionSettings {
        /// Namespace.
        namespace: String,
        /// `effective` (default) | `global` | `project`.
        #[serde(default)]
        scope: SettingsLayer,
    },
    /// Write one extension setting.
    #[serde(rename = "_oino/set_extension_setting")]
    SetExtensionSetting {
        /// Namespace.
        namespace: String,
        /// Dot-path key.
        key: String,
        /// Value (absent or `null` deletes).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<JsonValue>,
        /// Target file.
        scope: SettingsScope,
    },
    /// Delete one extension setting ("use default").
    #[serde(rename = "_oino/unset_extension_setting")]
    UnsetExtensionSetting {
        /// Namespace.
        namespace: String,
        /// Dot-path key.
        key: String,
        /// Target file.
        scope: SettingsScope,
    },
    /// Core settings (merged; `key` = dot path) → `{settings}` or `{value}`.
    #[serde(rename = "_oino/get_settings")]
    GetSettings {
        /// Dot path of one value.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        key: Option<String>,
    },
    /// Write one core setting (frontend keys included).
    #[serde(rename = "_oino/set_setting")]
    SetSetting {
        /// Dot path.
        key: String,
        /// Value (absent or `null` deletes).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        value: Option<JsonValue>,
        /// Target file.
        scope: SettingsScope,
    },
    /// Call an extension service as the frontend → the service's result.
    #[serde(rename = "_oino/service_call")]
    ServiceCall {
        /// Serving extension (first server of `name` when absent).
        #[serde(default, skip_serializing_if = "Option::is_none")]
        target: Option<String>,
        /// Service name.
        name: String,
        /// Request payload.
        #[serde(default)]
        payload: JsonValue,
    },
    /// Providers with sign-in support and credential state → `{providers}`.
    #[serde(rename = "_oino/auth_providers")]
    AuthProviders,
    /// Run a provider's OAuth sign-in (dialogs through the extension UI protocol).
    #[serde(rename = "_oino/login")]
    Login {
        /// Provider id.
        provider: String,
    },
    /// Remove a provider's stored credential.
    #[serde(rename = "_oino/logout")]
    Logout {
        /// Provider id.
        provider: String,
    },
    /// Store an API key in `auth.json`.
    #[serde(rename = "_oino/set_api_key")]
    SetApiKey {
        /// Provider id.
        provider: String,
        /// Key.
        key: String,
    },
    /// Sticky topic feed: changes the subscription → `{topics: [TopicValue], patterns}`;
    /// matching publishes then arrive as `topic` events.
    #[serde(rename = "_oino/topics")]
    Topics {
        /// Patterns to add (exact, `prefix*`, `*`).
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        subscribe: Vec<String>,
        /// Patterns to remove.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        unsubscribe: Vec<String>,
    },
    /// Last sticky value of one topic → `{data | null, publisher?}`.
    #[serde(rename = "_oino/bus_get")]
    BusGet {
        /// Topic.
        topic: String,
    },
    /// Nested agents of the session tree → `{agents: [AgentInfo], follow}`; `follow` turns
    /// live `agent_event` forwarding on or off.
    #[serde(rename = "_oino/agents")]
    Agents {
        /// Forward child events.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        follow: Option<bool>,
    },
    /// Transcript of one nested agent → `{messages}`.
    #[serde(rename = "_oino/agent_messages", rename_all = "camelCase")]
    AgentMessages {
        /// Agent id.
        agent_id: String,
    },
    /// Aborts the current run of one nested agent.
    #[serde(rename = "_oino/agent_abort", rename_all = "camelCase")]
    AgentAbort {
        /// Agent id.
        agent_id: String,
    },
    /// One single-item queue edit (`{op: remove|edit|move|sendNow, queue, …}`).
    #[serde(rename = "_oino/queue_edit")]
    QueueEdit(QueueEdit),
    /// File checkpoints → `{checkpoints: [Checkpoint]}`.
    #[serde(rename = "_oino/checkpoints")]
    Checkpoints,
    /// Rewind to before a user message: restore files and/or fork the conversation →
    /// `RewindResult`.
    #[serde(rename = "_oino/rewind", rename_all = "camelCase")]
    Rewind {
        /// User message entry.
        entry_id: String,
        /// Restore the files changed since (default true).
        #[serde(default = "yes")]
        files: bool,
        /// Fork the conversation before the message (default true).
        #[serde(default = "yes")]
        conversation: bool,
    },
    /// Running foreground `bash` calls → `{calls: [RunningBash]}`.
    #[serde(rename = "_oino/running_bash")]
    RunningBash,
    /// Extensions → `{extensions: [ExtensionInfo]}`.
    #[serde(rename = "_oino/extensions")]
    Extensions,
    /// Transcript row render (ui-protocol §6.4) → `{node | null}` from the owner of the
    /// render spec for `target`/`key` (`guest.render`; built-in tool rows too).
    #[serde(rename = "_oino/render", rename_all = "camelCase")]
    Render {
        /// `customMessage` | `customEntry` | `tool`.
        target: String,
        /// Custom type or tool name.
        key: String,
        /// Entry or tool-call id.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        entry_id: Option<String>,
        /// `simple` | `regular` | `advanced`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        density: Option<String>,
        /// Expanded row.
        #[serde(default)]
        expanded: bool,
        /// `narrow` | `normal` | `wide`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        width: Option<String>,
        /// Row payload.
        #[serde(default)]
        data: JsonValue,
    },
    /// Enable or disable an extension (`disabledExtensions`; applies to new sessions).
    #[serde(rename = "_oino/set_extension_enabled")]
    SetExtensionEnabled {
        /// Extension id.
        id: String,
        /// Enabled.
        enabled: bool,
    },
}

const fn yes() -> bool {
    true
}

const fn protocol_version() -> u32 {
    1
}

/// Oino command types implemented by [`OinoCommand`].
pub const OINO_COMMAND_TYPES: [&str; 35] = [
    "_oino/hello",
    "_oino/ui_snapshot",
    "_oino/navigate_tree",
    "_oino/set_label",
    "_oino/import_jsonl",
    "_oino/export_jsonl",
    "_oino/get_context_usage",
    "_oino/get_completions",
    "_oino/ui_action",
    "_oino/editor_event",
    "_oino/invoke_shortcut",
    "_oino/get_shortcuts",
    "_oino/get_settings_schema",
    "_oino/get_extension_settings",
    "_oino/set_extension_setting",
    "_oino/unset_extension_setting",
    "_oino/get_settings",
    "_oino/set_setting",
    "_oino/service_call",
    "_oino/auth_providers",
    "_oino/login",
    "_oino/logout",
    "_oino/set_api_key",
    "_oino/topics",
    "_oino/bus_get",
    "_oino/agents",
    "_oino/agent_messages",
    "_oino/agent_abort",
    "_oino/queue_edit",
    "_oino/checkpoints",
    "_oino/rewind",
    "_oino/running_bash",
    "_oino/extensions",
    "_oino/set_extension_enabled",
    "_oino/render",
];

/// Any command a backend accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RpcCommand {
    /// pi command.
    Pi(PiCommand),
    /// Oino extension.
    Oino(OinoCommand),
}

impl PiCommand {
    /// The wire `type`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::FollowUp { .. } => "follow_up",
            Self::Abort => "abort",
            Self::NewSession { .. } => "new_session",
            Self::GetState => "get_state",
            Self::SetModel { .. } => "set_model",
            Self::CycleModel => "cycle_model",
            Self::GetAvailableModels => "get_available_models",
            Self::SetThinkingLevel { .. } => "set_thinking_level",
            Self::CycleThinkingLevel => "cycle_thinking_level",
            Self::GetAvailableThinkingLevels => "get_available_thinking_levels",
            Self::SetSteeringMode { .. } => "set_steering_mode",
            Self::SetFollowUpMode { .. } => "set_follow_up_mode",
            Self::Compact { .. } => "compact",
            Self::SetAutoCompaction { .. } => "set_auto_compaction",
            Self::SetAutoRetry { .. } => "set_auto_retry",
            Self::AbortRetry => "abort_retry",
            Self::Bash { .. } => "bash",
            Self::AbortBash => "abort_bash",
            Self::GetSessionStats => "get_session_stats",
            Self::ExportHtml { .. } => "export_html",
            Self::SwitchSession { .. } => "switch_session",
            Self::Fork { .. } => "fork",
            Self::Clone => "clone",
            Self::GetForkMessages => "get_fork_messages",
            Self::GetEntries { .. } => "get_entries",
            Self::GetTree => "get_tree",
            Self::GetLastAssistantText => "get_last_assistant_text",
            Self::SetSessionName { .. } => "set_session_name",
            Self::GetMessages => "get_messages",
            Self::GetCommands => "get_commands",
            Self::ClearQueue => "clear_queue",
        }
    }
}

impl OinoCommand {
    /// The wire `type`.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Hello { .. } => "_oino/hello",
            Self::UiSnapshot => "_oino/ui_snapshot",
            Self::NavigateTree { .. } => "_oino/navigate_tree",
            Self::SetLabel { .. } => "_oino/set_label",
            Self::ImportJsonl { .. } => "_oino/import_jsonl",
            Self::ExportJsonl { .. } => "_oino/export_jsonl",
            Self::GetContextUsage => "_oino/get_context_usage",
            Self::GetCompletions { .. } => "_oino/get_completions",
            Self::UiAction { .. } => "_oino/ui_action",
            Self::EditorEvent(_) => "_oino/editor_event",
            Self::InvokeShortcut { .. } => "_oino/invoke_shortcut",
            Self::GetShortcuts => "_oino/get_shortcuts",
            Self::GetSettingsSchema => "_oino/get_settings_schema",
            Self::GetExtensionSettings { .. } => "_oino/get_extension_settings",
            Self::SetExtensionSetting { .. } => "_oino/set_extension_setting",
            Self::UnsetExtensionSetting { .. } => "_oino/unset_extension_setting",
            Self::GetSettings { .. } => "_oino/get_settings",
            Self::SetSetting { .. } => "_oino/set_setting",
            Self::ServiceCall { .. } => "_oino/service_call",
            Self::AuthProviders => "_oino/auth_providers",
            Self::Login { .. } => "_oino/login",
            Self::Logout { .. } => "_oino/logout",
            Self::SetApiKey { .. } => "_oino/set_api_key",
            Self::Topics { .. } => "_oino/topics",
            Self::BusGet { .. } => "_oino/bus_get",
            Self::Agents { .. } => "_oino/agents",
            Self::AgentMessages { .. } => "_oino/agent_messages",
            Self::AgentAbort { .. } => "_oino/agent_abort",
            Self::QueueEdit(_) => "_oino/queue_edit",
            Self::Checkpoints => "_oino/checkpoints",
            Self::Rewind { .. } => "_oino/rewind",
            Self::RunningBash => "_oino/running_bash",
            Self::Extensions => "_oino/extensions",
            Self::SetExtensionEnabled { .. } => "_oino/set_extension_enabled",
            Self::Render { .. } => "_oino/render",
        }
    }
}

impl RpcCommand {
    /// The wire `type` of the command.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::Pi(command) => command.kind(),
            Self::Oino(command) => command.kind(),
        }
    }
}

impl From<PiCommand> for RpcCommand {
    fn from(command: PiCommand) -> Self {
        Self::Pi(command)
    }
}

impl From<OinoCommand> for RpcCommand {
    fn from(command: OinoCommand) -> Self {
        Self::Oino(command)
    }
}

/// A parsed input line that is not an `extension_ui_response`.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcRequest {
    /// Correlation id (echoed verbatim).
    pub id: Option<JsonValue>,
    /// The command.
    pub command: RpcCommand,
}

/// A command that could not be decoded; answered with an error response.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RejectedCommand {
    /// Correlation id.
    pub id: Option<JsonValue>,
    /// Command type as sent (`None` when missing).
    pub command: Option<String>,
    /// pi error text.
    pub error: String,
}

impl RpcRequest {
    /// Decodes a command object (pi `handleCommand` dispatch rules).
    ///
    /// # Errors
    ///
    /// `Unknown command: {type}` for unknown or missing types, the decoder message for bad
    /// fields.
    pub fn from_value(value: JsonValue) -> Result<Self, RejectedCommand> {
        let id = value.get("id").filter(|id| !id.is_null()).cloned();
        let kind = value
            .get("type")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let Some(kind) = kind else {
            return Err(RejectedCommand {
                id,
                command: None,
                error: "Unknown command: undefined".to_owned(),
            });
        };
        let known = PI_COMMAND_TYPES.contains(&kind.as_str())
            || OINO_COMMAND_TYPES.contains(&kind.as_str());
        if !known {
            return Err(RejectedCommand {
                id,
                error: format!("Unknown command: {kind}"),
                command: Some(kind),
            });
        }
        let decoded = if kind.starts_with("_oino/") {
            serde_json::from_value::<OinoCommand>(value).map(RpcCommand::Oino)
        } else {
            serde_json::from_value::<PiCommand>(value).map(RpcCommand::Pi)
        };
        decoded
            .map(|command| Self {
                id: id.clone(),
                command,
            })
            .map_err(|error| RejectedCommand {
                id,
                command: Some(kind),
                error: error.to_string(),
            })
    }

    /// The command object with its `id`.
    #[must_use]
    pub fn to_value(&self) -> JsonValue {
        let mut object = JsonObject::new();
        if let Some(id) = &self.id {
            object.insert("id".to_owned(), id.clone());
        }
        if let Ok(JsonValue::Object(fields)) = serde_json::to_value(&self.command) {
            object.extend(fields);
        }
        JsonValue::Object(object)
    }
}

/// Successful reply payload (`None` = no `data` key, `Some(Null)` = `"data": null`).
pub type RpcResponseData = Option<JsonValue>;

/// `{id?, type:"response", command, success, data? | error}` in pi key order.
#[derive(Clone, Debug, PartialEq)]
pub struct RpcResponse {
    /// Correlation id.
    pub id: Option<JsonValue>,
    /// Command type (`None` for a command without `type`).
    pub command: Option<String>,
    /// Outcome.
    pub result: Result<RpcResponseData, String>,
}

impl RpcResponse {
    /// A success reply.
    #[must_use]
    pub fn success(
        id: Option<JsonValue>,
        command: impl Into<String>,
        data: RpcResponseData,
    ) -> Self {
        Self {
            id,
            command: Some(command.into()),
            result: Ok(data),
        }
    }

    /// An error reply.
    #[must_use]
    pub fn error(id: Option<JsonValue>, command: Option<String>, error: impl Into<String>) -> Self {
        Self {
            id,
            command,
            result: Err(error.into()),
        }
    }

    /// Decodes a reply object.
    #[must_use]
    pub fn from_value(value: &JsonValue) -> Option<Self> {
        if value.get("type").and_then(JsonValue::as_str) != Some("response") {
            return None;
        }
        let id = value.get("id").cloned();
        let command = value
            .get("command")
            .and_then(JsonValue::as_str)
            .map(str::to_owned);
        let result = if value.get("success") == Some(&JsonValue::Bool(true)) {
            Ok(value.get("data").cloned())
        } else {
            Err(value
                .get("error")
                .and_then(JsonValue::as_str)
                .unwrap_or_default()
                .to_owned())
        };
        Some(Self {
            id,
            command,
            result,
        })
    }
}

impl Serialize for RpcResponse {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(None)?;
        if let Some(id) = &self.id {
            map.serialize_entry("id", id)?;
        }
        map.serialize_entry("type", "response")?;
        if let Some(command) = &self.command {
            map.serialize_entry("command", command)?;
        }
        match &self.result {
            Ok(data) => {
                map.serialize_entry("success", &true)?;
                if let Some(data) = data {
                    map.serialize_entry("data", data)?;
                }
            }
            Err(error) => {
                map.serialize_entry("success", &false)?;
                map.serialize_entry("error", error)?;
            }
        }
        map.end()
    }
}

/// pi `RpcSessionState` (`get_state` data).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RpcSessionState {
    /// Selected model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<Model>,
    /// Thinking level.
    pub thinking_level: ThinkingLevel,
    /// A run is active.
    pub is_streaming: bool,
    /// Compaction or branch summary running.
    pub is_compacting: bool,
    /// Steering drain mode.
    pub steering_mode: QueueMode,
    /// Follow-up drain mode.
    pub follow_up_mode: QueueMode,
    /// Session file.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_file: Option<String>,
    /// Session id.
    pub session_id: String,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    /// Automatic compaction enabled.
    pub auto_compaction_enabled: bool,
    /// Transcript length.
    pub message_count: usize,
    /// Queued texts.
    pub pending_message_count: usize,
}

/// pi `RpcSlashCommand`.
pub type RpcSlashCommand = oino_core::session::SlashCommandInfo;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn decodes_pi_and_oino_commands() {
        let request = RpcRequest::from_value(json!({"id":"1","type":"prompt","message":"hi","images":[{"type":"image","data":"AA","mimeType":"image/png"}],"streamingBehavior":"followUp"})).unwrap();
        assert_eq!(request.id, Some(json!("1")));
        assert_eq!(
            request.command,
            RpcCommand::Pi(PiCommand::Prompt {
                message: "hi".into(),
                images: vec![ImageContent {
                    data: "AA".into(),
                    mime_type: "image/png".into()
                }],
                streaming_behavior: Some(StreamingBehavior::FollowUp),
            })
        );
        assert_eq!(
            request.to_value(),
            json!({"id":"1","type":"prompt","message":"hi","images":[{"type":"image","data":"AA","mimeType":"image/png"}],"streamingBehavior":"followUp"})
        );
        let hello = RpcRequest::from_value(
            json!({"type":"_oino/hello","client":{"name":"t","version":"1"}}),
        )
        .unwrap();
        assert!(matches!(
            hello.command,
            RpcCommand::Oino(OinoCommand::Hello { protocol: 1, .. })
        ));
        let navigate = RpcRequest::from_value(
            json!({"type":"_oino/navigate_tree","targetId":"ab","summarize":true}),
        )
        .unwrap();
        assert_eq!(navigate.command.kind(), "_oino/navigate_tree");
        assert_eq!(
            RpcRequest::from_value(json!({"type":"abort"}))
                .unwrap()
                .command,
            RpcCommand::Pi(PiCommand::Abort)
        );
    }

    #[test]
    fn rejects_unknown_and_malformed_commands() {
        let unknown = RpcRequest::from_value(json!({"id":7,"type":"nope"})).unwrap_err();
        assert_eq!(unknown.error, "Unknown command: nope");
        assert_eq!(unknown.id, Some(json!(7)));
        let missing = RpcRequest::from_value(json!({})).unwrap_err();
        assert_eq!(missing.command, None);
        assert_eq!(missing.error, "Unknown command: undefined");
        let bad = RpcRequest::from_value(json!({"type":"set_model"})).unwrap_err();
        assert_eq!(bad.command.as_deref(), Some("set_model"));
        assert!(bad.error.contains("provider"), "{}", bad.error);
    }

    #[test]
    fn responses_use_pi_key_order() {
        let ok = RpcResponse::success(Some(json!("a")), "cycle_model", Some(JsonValue::Null));
        assert_eq!(
            serde_json::to_string(&ok).unwrap(),
            r#"{"id":"a","type":"response","command":"cycle_model","success":true,"data":null}"#
        );
        let empty = RpcResponse::success(None, "abort", None);
        assert_eq!(
            serde_json::to_string(&empty).unwrap(),
            r#"{"type":"response","command":"abort","success":true}"#
        );
        let error = RpcResponse::error(None, Some("parse".into()), "Failed to parse command: x");
        let text = serde_json::to_string(&error).unwrap();
        assert_eq!(
            text,
            r#"{"type":"response","command":"parse","success":false,"error":"Failed to parse command: x"}"#
        );
        let back = RpcResponse::from_value(&serde_json::from_str(&text).unwrap()).unwrap();
        assert_eq!(back, error);
    }
}
