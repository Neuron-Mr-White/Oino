//! Host operation enums: one variant per WIT function of the interfaces that
//! [`crate::HostServices`] serves (same names in `UpperCamelCase`, same parameters).

use std::fmt;

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::json::RawJson;

// ── session ──────────────────────────────────────────────────────────────

/// `session.delivery` (pi `deliverAs`).
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

/// `session.provenance` (unipi harness provenance).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provenance {
    /// Source (extension or subsystem).
    pub source: String,
    /// Title.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Synopsis.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub synopsis: Option<String>,
    /// Severity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub severity: Option<String>,
}

/// `session.send-options`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SendOptions {
    /// Trigger a turn when idle.
    pub trigger_turn: bool,
    /// Delivery while streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver_as: Option<Delivery>,
    /// Provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

/// `session.user-message-options`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserMessageOptions {
    /// Delivery while streaming.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deliver_as: Option<Delivery>,
    /// Expand prompt templates.
    pub expand_prompt_templates: bool,
    /// Provenance.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provenance: Option<Provenance>,
}

/// `session.sessions-scope`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionsScope {
    /// Sessions of the current cwd.
    Cwd,
    /// All sessions.
    All,
}

/// Session reads (pi `ReadonlySessionManager`); results are `oino_session` JSON.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case")]
pub enum SessionQuery {
    /// `header` → `SessionHeader`.
    Header,
    /// `session-id` → string.
    SessionId,
    /// `session-file` → string | null.
    SessionFile,
    /// `leaf-id` → string | null.
    LeafId,
    /// `entry` → `SessionEntry` | null.
    Entry {
        /// Entry id.
        id: String,
    },
    /// `entries` → all entries in file order.
    Entries,
    /// `branch` → root → `from` (default leaf).
    Branch {
        /// Branch tip.
        from: Option<String>,
    },
    /// `context-entries`.
    ContextEntries,
    /// `projection`.
    Projection,
    /// `tree`.
    Tree,
    /// `label` → string | null.
    Label {
        /// Entry id.
        id: String,
    },
    /// `session-name` → string | null.
    SessionName,
}

/// Session writes.
#[derive(Clone, Debug, PartialEq)]
pub enum SessionWrite {
    /// `set-session-name`.
    SetSessionName(Option<String>),
    /// `set-label`.
    SetLabel {
        /// Entry id.
        id: String,
        /// Label (`None` clears).
        label: Option<String>,
    },
    /// `append-entry` → entry id.
    AppendEntry {
        /// Custom type.
        custom_type: String,
        /// Data.
        data: Option<RawJson>,
    },
    /// `send-message`: `{customType, content, display, details?}`.
    SendMessage {
        /// Message JSON.
        message: RawJson,
        /// Options.
        options: SendOptions,
    },
    /// `send-user-message`: `UserContentField`.
    SendUserMessage {
        /// Content JSON.
        content: RawJson,
        /// Options.
        options: UserMessageOptions,
    },
}

/// Sessions index (C7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionsIndexQuery {
    /// `sessions-list` → `Vec<SessionInfo>`.
    List(SessionsScope),
    /// `session-read` → `{header, entries}`.
    Read {
        /// Session file path.
        path: String,
    },
}

// ── control ──────────────────────────────────────────────────────────────

/// `control` operations (except `tool-progress`, which is fire-and-forget).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ControlOp {
    /// `model` → `Model` | null.
    Model,
    /// `routed-model`.
    RoutedModel,
    /// `set-model` → bool.
    SetModel {
        /// Provider id.
        provider: String,
        /// Model id.
        id: String,
    },
    /// `thinking-level` → string.
    ThinkingLevel,
    /// `set-thinking-level` → clamped level.
    SetThinkingLevel(String),
    /// `models-list` → `Vec<Model>`.
    ModelsList,
    /// `scoped-models`.
    ScopedModels,
    /// `run-state`.
    RunState,
    /// `is-idle` → bool.
    IsIdle,
    /// `has-pending-messages` → bool.
    HasPendingMessages,
    /// `context-usage`.
    ContextUsage,
    /// `session-stats`.
    SessionStats,
    /// `system-prompt` → string.
    SystemPrompt,
    /// `is-project-trusted` → bool.
    IsProjectTrusted,
    /// `get-settings`.
    GetSettings,
    /// `abort`.
    Abort,
    /// `compact` → `CompactionResult`.
    Compact {
        /// Custom instructions.
        custom_instructions: Option<String>,
    },
    /// `shutdown`.
    Shutdown,
    /// `system-prompt-options` (command context).
    SystemPromptOptions,
    /// `wait-for-idle` (command context).
    WaitForIdle,
    /// `new-session` → cancelled? (command context).
    NewSession {
        /// Parent session file.
        parent_session: Option<String>,
    },
    /// `fork` → `{cancelled, selectedText?}` (command context).
    Fork {
        /// Entry id.
        entry_id: String,
        /// Fork at (true) or before (false) the entry.
        at: bool,
    },
    /// `switch-session` → cancelled? (command context).
    SwitchSession {
        /// Session file.
        path: String,
    },
    /// `navigate-tree` (command context).
    NavigateTree {
        /// Target entry.
        target_id: String,
        /// `NavigateTreeOptions` JSON.
        options: RawJson,
    },
    /// `reload` (command context).
    Reload,
    /// `active-tools` → names.
    ActiveTools,
    /// `set-active-tools`.
    SetActiveTools(Vec<String>),
    /// `all-tools` → `Vec<ToolInfo>`.
    AllTools,
    /// `commands-list` → `Vec<SlashCommandInfo>`.
    CommandsList,
    /// `command-invoke` → optional JSON.
    CommandInvoke {
        /// Invocation name.
        name: String,
        /// Arguments.
        args: String,
    },
    /// `execute-tool` → `AgentToolCallOutcome` (tool-call context).
    ExecuteTool {
        /// Tool name.
        name: String,
        /// Arguments JSON.
        args: RawJson,
        /// Caller cancellation.
        cancel: Option<CancellationToken>,
    },
    /// `bash-detach` → task id.
    BashDetach {
        /// Running bash tool call.
        tool_call_id: String,
        /// Reason.
        reason: String,
    },
    /// `running-bash`.
    RunningBash,
}

impl ControlOp {
    /// WIT function name (for errors and permission messages).
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::Model => "model",
            Self::RoutedModel => "routed-model",
            Self::SetModel { .. } => "set-model",
            Self::ThinkingLevel => "thinking-level",
            Self::SetThinkingLevel(_) => "set-thinking-level",
            Self::ModelsList => "models-list",
            Self::ScopedModels => "scoped-models",
            Self::RunState => "run-state",
            Self::IsIdle => "is-idle",
            Self::HasPendingMessages => "has-pending-messages",
            Self::ContextUsage => "context-usage",
            Self::SessionStats => "session-stats",
            Self::SystemPrompt => "system-prompt",
            Self::IsProjectTrusted => "is-project-trusted",
            Self::GetSettings => "get-settings",
            Self::Abort => "abort",
            Self::Compact { .. } => "compact",
            Self::Shutdown => "shutdown",
            Self::SystemPromptOptions => "system-prompt-options",
            Self::WaitForIdle => "wait-for-idle",
            Self::NewSession { .. } => "new-session",
            Self::Fork { .. } => "fork",
            Self::SwitchSession { .. } => "switch-session",
            Self::NavigateTree { .. } => "navigate-tree",
            Self::Reload => "reload",
            Self::ActiveTools => "active-tools",
            Self::SetActiveTools(_) => "set-active-tools",
            Self::AllTools => "all-tools",
            Self::CommandsList => "commands-list",
            Self::CommandInvoke { .. } => "command-invoke",
            Self::ExecuteTool { .. } => "execute-tool",
            Self::BashDetach { .. } => "bash-detach",
            Self::RunningBash => "running-bash",
        }
    }

    /// Requires manifest permission `sessionControl`.
    #[must_use]
    pub fn needs_session_control(&self) -> bool {
        matches!(
            self,
            Self::SetModel { .. }
                | Self::SetThinkingLevel(_)
                | Self::Abort
                | Self::Compact { .. }
                | Self::Shutdown
                | Self::SetActiveTools(_)
                | Self::NewSession { .. }
                | Self::Fork { .. }
                | Self::SwitchSession { .. }
                | Self::NavigateTree { .. }
                | Self::Reload
        )
    }

    /// Only valid in command/shortcut contexts (pi `ExtensionCommandContext`).
    #[must_use]
    pub fn needs_command_context(&self) -> bool {
        matches!(
            self,
            Self::SystemPromptOptions
                | Self::WaitForIdle
                | Self::NewSession { .. }
                | Self::Fork { .. }
                | Self::SwitchSession { .. }
                | Self::NavigateTree { .. }
                | Self::Reload
        )
    }
}

/// Result of a [`ControlOp`].
#[derive(Clone, Debug, PartialEq)]
pub enum ControlReply {
    /// No value.
    Unit,
    /// Boolean.
    Bool(bool),
    /// String.
    String(String),
    /// String list.
    Strings(Vec<String>),
    /// JSON value.
    Json(RawJson),
    /// Optional JSON value.
    MaybeJson(Option<RawJson>),
}

// ── models ───────────────────────────────────────────────────────────────

/// `models.credential`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Credential {
    /// Headers to add.
    pub headers: Vec<(String, String)>,
    /// Base URL override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Provider id.
    pub provider: String,
}

/// `models` operations (permission `models.complete`, `credentials`).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum ModelsOp {
    /// `complete` → `AssistantMessage`.
    Complete {
        /// `CompletionRequest` JSON.
        request: RawJson,
        /// Caller cancellation.
        cancel: Option<CancellationToken>,
    },
    /// `stream` → `AssistantMessageEvent` JSON items.
    Stream {
        /// `CompletionRequest` JSON.
        request: RawJson,
    },
    /// `summarize-default` → `{summary, usage?}`.
    SummarizeDefault {
        /// `Vec<AgentMessage>` JSON.
        messages: RawJson,
        /// Custom instructions.
        custom_instructions: Option<String>,
    },
    /// `credential-for`.
    CredentialFor(String),
    /// `classify`.
    Classify(RawJson),
    /// `generate-image`.
    GenerateImage(RawJson),
}

/// Result of a [`ModelsOp`].
pub enum ModelsReply {
    /// JSON value.
    Json(RawJson),
    /// Stream of JSON items.
    Stream(BoxStream<'static, String>),
    /// Credential (none when unavailable).
    Credential(Option<Credential>),
}

impl fmt::Debug for ModelsReply {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(json) => f.debug_tuple("Json").field(json).finish(),
            Self::Stream(_) => f.write_str("Stream(..)"),
            Self::Credential(c) => f.debug_tuple("Credential").field(c).finish(),
        }
    }
}

// ── ui ───────────────────────────────────────────────────────────────────

/// Panel operations (ui-protocol §5).
#[derive(Clone, Debug, PartialEq)]
pub enum PanelOp {
    /// `panel-open` → panel id.
    Open(RawJson),
    /// `panel-update`.
    Update {
        /// Panel id.
        panel_id: String,
        /// Spec.
        spec: RawJson,
    },
    /// `panel-close`.
    Close(String),
}

/// Editor operations (ui-protocol §7).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EditorOp {
    /// `editor-text` → text.
    Text,
    /// `editor-set`.
    Set(String),
    /// `editor-insert`.
    Insert(String),
}

/// Device integration (permissions `osNotify`, `clipboard.*`, `openUrl`, `editExternally`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeviceOp {
    /// `os-notify`.
    OsNotify {
        /// Title.
        title: String,
        /// Body.
        body: String,
        /// Urgency (`low`|`normal`|`critical`).
        urgency: String,
        /// URL opened on click.
        action_url: Option<String>,
    },
    /// `clipboard-write-text`.
    ClipboardWriteText(String),
    /// `clipboard-read-image` → `ImageContent` | none.
    ClipboardReadImage,
    /// `open-url`.
    OpenUrl(String),
    /// `edit-externally` → edited text | none.
    EditExternally {
        /// Initial text.
        text: String,
        /// File extension hint.
        extension_hint: Option<String>,
    },
}

// ── tasks / secrets ──────────────────────────────────────────────────────

/// `tasks` operations (C12).
#[derive(Clone, Debug, PartialEq)]
pub enum TaskOp {
    /// `start` (`TaskSpec` JSON) → task id.
    Start(RawJson),
    /// `adopt-bash` → task id.
    AdoptBash {
        /// Running bash tool call.
        tool_call_id: String,
        /// Reason.
        reason: String,
    },
    /// `list` → `Vec<TaskInfo>`.
    List,
    /// `get` → `TaskInfo` | null.
    Get(String),
    /// `logs` → text.
    Logs {
        /// Task id.
        id: String,
        /// Max bytes.
        max_bytes: u32,
        /// From the end.
        tail: bool,
    },
    /// `kill`.
    Kill(String),
    /// `clear-finished`.
    ClearFinished,
}

/// `secrets` operations (namespaced to the calling extension).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SecretOp {
    /// `get`.
    Get(String),
    /// `set`.
    Set {
        /// Name.
        name: String,
        /// Value.
        value: String,
    },
    /// `delete`.
    Delete(String),
}

// ── provider-host ────────────────────────────────────────────────────────

/// `provider-host` calls made by provider components.
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderHook {
    /// `before-request`: dispatch `before_provider_headers` then `before_provider_request`.
    BeforeRequest {
        /// `requestId` from the stream request.
        request_id: String,
        /// Wire payload.
        payload: RawJson,
        /// Request headers.
        headers: Vec<(String, String)>,
    },
    /// `after-response` (`after_provider_response`).
    AfterResponse {
        /// Request id.
        request_id: String,
        /// HTTP status.
        status: u16,
        /// Response headers.
        headers: Vec<(String, String)>,
    },
    /// `stream-event` (`provider_stream_event`).
    StreamEvent {
        /// Request id.
        request_id: String,
        /// Raw event JSON.
        data: RawJson,
    },
}

/// Reply to a [`ProviderHook`].
#[derive(Clone, Debug, PartialEq)]
pub enum ProviderHookReply {
    /// Possibly replaced payload and headers.
    BeforeRequest {
        /// Payload.
        payload: RawJson,
        /// Headers.
        headers: Vec<(String, String)>,
    },
    /// Acknowledged.
    Done,
}

impl ProviderHook {
    /// The pass-through reply (no extension changed anything).
    #[must_use]
    pub fn passthrough(&self) -> ProviderHookReply {
        match self {
            Self::BeforeRequest {
                payload, headers, ..
            } => ProviderHookReply::BeforeRequest {
                payload: payload.clone(),
                headers: headers.clone(),
            },
            Self::AfterResponse { .. } | Self::StreamEvent { .. } => ProviderHookReply::Done,
        }
    }
}

// ── host-info ────────────────────────────────────────────────────────────

/// `host-info.workspace-info` (D10).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceInfo {
    /// `wsid`: first 16 hex chars of sha256(canonical project root).
    pub id: String,
    /// Canonical project root.
    pub root: String,
    /// Git root.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_root: Option<String>,
    /// Git branch.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_branch: Option<String>,
    /// cwd.
    pub cwd: String,
}

/// Host information that needs the runtime.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HostContextInfo {
    /// Attached frontends (`tui`, `desktop`, `mobile`, `rpc`, `acp`).
    pub frontends: Vec<String>,
    /// A frontend currently has focus.
    pub frontend_focused: bool,
    /// Workspace override (`None` = computed by the host from the call cwd).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<WorkspaceInfo>,
}
