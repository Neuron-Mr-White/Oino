//! Host operation enums: one variant per WIT function of the interfaces that
//! [`crate::HostServices`] serves (same names in `UpperCamelCase`, same parameters).

use std::fmt;

use futures::stream::BoxStream;
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use crate::json::RawJson;

// ── session ──────────────────────────────────────────────────────────────

/// `session.delivery`.
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

/// `session.provenance` (the embedding harness provenance).
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

/// Session reads; results are `oino_session` JSON.
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

/// Which entries an [`EntryQuery`] starts from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueryScope {
    /// Every entry in file order (all branches), like `entries`.
    #[default]
    All,
    /// Root → leaf, like `branch(none)`.
    Branch,
}

/// `session.query-entries` (perf-fix #1): a bounded session read. The host filters before it
/// serializes, so an extension that only needs its own state entries, the last user message
/// or the tail since the last compaction never copies the whole transcript into its linear
/// memory (which never shrinks: four extensions reading a 200 MB chat held ≈ 1.9 GB).
///
/// An entry is returned when it passes every given filter: `types` (the entry `type`),
/// `customTypes` (only checked on `custom` / `custom_message` entries), `roles` (only checked
/// on `message` entries). Then `last` keeps the newest N matches and `maxString` clips every
/// longer string value (on a char boundary). Results are oldest first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EntryQuery {
    /// Starting set.
    pub scope: QueryScope,
    /// Start at the live context of the newest `compaction` of the scope: its
    /// `firstKeptEntryId` when that entry is in scope, else the compaction entry itself
    /// (no compaction: from the start).
    pub since_compaction: bool,
    /// Entry `type`s to keep (empty: all).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub types: Vec<String>,
    /// `customType`s to keep among `custom` / `custom_message` entries (empty: all).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub custom_types: Vec<String>,
    /// Message roles to keep among `message` entries (empty: all).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub roles: Vec<String>,
    /// Keep only the newest N matches.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last: Option<usize>,
    /// Clip string values longer than this many bytes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_string: Option<usize>,
}

impl EntryQuery {
    /// Whether an entry with these identity fields passes the filters.
    #[must_use]
    pub fn matches(&self, entry_type: &str, custom_type: Option<&str>, role: Option<&str>) -> bool {
        let listed = |list: &[String], value: &str| list.iter().any(|item| item == value);
        if !self.types.is_empty() && !listed(&self.types, entry_type) {
            return false;
        }
        if !self.custom_types.is_empty()
            && matches!(entry_type, "custom" | "custom_message")
            && !custom_type.is_some_and(|t| listed(&self.custom_types, t))
        {
            return false;
        }
        if !self.roles.is_empty()
            && entry_type == "message"
            && !role.is_some_and(|r| listed(&self.roles, r))
        {
            return false;
        }
        true
    }

    /// [`Self::matches`] on an entry's JSON.
    #[must_use]
    pub fn matches_value(&self, entry: &serde_json::Value) -> bool {
        let field = |pointer: &str| entry.pointer(pointer).and_then(serde_json::Value::as_str);
        self.matches(
            field("/type").unwrap_or_default(),
            field("/customType"),
            field("/message/role"),
        )
    }

    /// Applies the query to `entries` (the scope's entries, oldest first) — the reference
    /// implementation hosts without an indexed session use.
    #[must_use]
    pub fn apply_values(&self, entries: Vec<serde_json::Value>) -> Vec<serde_json::Value> {
        let start = if self.since_compaction {
            let str_of = |e: &serde_json::Value, key: &str| {
                e.get(key)
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_owned)
            };
            compaction_start(
                entries.len(),
                |i| str_of(&entries[i], "type").as_deref() == Some("compaction"),
                |i| str_of(&entries[i], "firstKeptEntryId"),
                |i| str_of(&entries[i], "id"),
            )
        } else {
            0
        };
        let mut matched: Vec<serde_json::Value> = entries
            .into_iter()
            .skip(start)
            .filter(|e| self.matches_value(e))
            .collect();
        if let Some(last) = self.last {
            let skip = matched.len().saturating_sub(last);
            matched.drain(..skip);
        }
        if let Some(max) = self.max_string {
            for entry in &mut matched {
                clip_strings(entry, max);
            }
        }
        matched
    }
}

/// Where [`EntryQuery::since_compaction`] starts in a list of `len` entries: the newest
/// compaction's first kept entry when it precedes the compaction in the list, else the
/// compaction itself, else 0.
pub fn compaction_start(
    len: usize,
    is_compaction: impl Fn(usize) -> bool,
    first_kept: impl Fn(usize) -> Option<String>,
    id: impl Fn(usize) -> Option<String>,
) -> usize {
    let Some(compaction) = (0..len).rev().find(|&i| is_compaction(i)) else {
        return 0;
    };
    first_kept(compaction)
        .filter(|kept| !kept.is_empty())
        .and_then(|kept| {
            (0..compaction)
                .rev()
                .find(|&i| id(i).as_deref() == Some(&kept))
        })
        .unwrap_or(compaction)
}

/// Clips every string in `value` longer than `max` bytes (on a char boundary).
pub fn clip_strings(value: &mut serde_json::Value, max: usize) {
    match value {
        serde_json::Value::String(text) if text.len() > max => {
            let mut end = max;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
        }
        serde_json::Value::Array(items) => items.iter_mut().for_each(|v| clip_strings(v, max)),
        serde_json::Value::Object(map) => map.values_mut().for_each(|v| clip_strings(v, max)),
        _ => {}
    }
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
    /// `append-reminder` → serialized `{id, appended}` reply.
    AppendReminder {
        /// Caller-owned reminder key (`extension-id/topic`).
        key: String,
        /// Plain harness text; the host owns the envelope.
        text: String,
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
    /// `set-default-model` → bool: persists the fresh-session model in core settings.
    SetDefaultModel {
        /// Provider id.
        provider: String,
        /// Model id.
        id: String,
        /// Thinking level to remember for that model.
        thinking: Option<String>,
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
    /// `update-active-tools`: add `enable`, remove `disable`, leave the rest.
    UpdateActiveTools {
        /// Tools to activate.
        enable: Vec<String>,
        /// Tools to deactivate.
        disable: Vec<String>,
    },
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
            Self::SetDefaultModel { .. } => "set-default-model",
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
            Self::UpdateActiveTools { .. } => "update-active-tools",
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
                | Self::SetDefaultModel { .. }
                | Self::SetThinkingLevel(_)
                | Self::Abort
                | Self::Compact { .. }
                | Self::Shutdown
                | Self::SetActiveTools(_)
                | Self::UpdateActiveTools { .. }
                | Self::NewSession { .. }
                | Self::Fork { .. }
                | Self::SwitchSession { .. }
                | Self::NavigateTree { .. }
                | Self::Reload
        )
    }

    /// Only valid in command/shortcut contexts.
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
        /// Summary of the earlier messages: the call then updates it.
        previous_summary: Option<String>,
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

// ── settings ─────────────────────────────────────────────────────────────

/// Settings file a write targets (`registry.settings-scope`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SettingsScope {
    /// `<agent>/settings.json` (and the secret store for secret fields).
    Global,
    /// `<cwd>/.oino/settings.json` (trusted projects only).
    Project,
}

/// Values a settings read returns (`registry.settings-layer`).
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[serde(rename_all = "lowercase")]
pub enum SettingsLayer {
    /// `defaults ⊕ global ⊕ project`.
    #[default]
    Effective,
    /// The global file's values.
    Global,
    /// The project file's values.
    Project,
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

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod entry_query_tests {
    use super::*;
    use serde_json::{Value, json};

    fn entries() -> Vec<Value> {
        vec![
            json!({"type": "message", "id": "u1", "message": {"role": "user", "content": "old"}}),
            json!({"type": "custom", "id": "c1", "customType": "a", "data": {"n": 1}}),
            json!({"type": "message", "id": "k1", "message": {"role": "assistant", "content": "kept"}}),
            json!({"type": "compaction", "id": "z1", "firstKeptEntryId": "k1", "summary": "s"}),
            json!({"type": "custom", "id": "c2", "customType": "b", "data": {"n": 2}}),
            json!({"type": "message", "id": "u2", "message": {"role": "user", "content": "new text here"}}),
            json!({"type": "custom", "id": "c3", "customType": "a", "data": {"n": 3}}),
        ]
    }

    fn ids(found: &[Value]) -> Vec<&str> {
        found.iter().map(|e| e["id"].as_str().unwrap()).collect()
    }

    #[test]
    fn custom_types_filter_only_custom_entries() {
        let query = EntryQuery {
            types: vec!["custom".into()],
            custom_types: vec!["a".into()],
            ..EntryQuery::default()
        };
        assert_eq!(ids(&query.apply_values(entries())), ["c1", "c3"]);
        let last = EntryQuery {
            last: Some(1),
            ..query
        };
        assert_eq!(ids(&last.apply_values(entries())), ["c3"]);
    }

    #[test]
    fn roles_filter_only_messages_and_types_combine() {
        let query = EntryQuery {
            types: vec!["custom".into(), "message".into()],
            custom_types: vec!["b".into()],
            roles: vec!["user".into()],
            ..EntryQuery::default()
        };
        assert_eq!(ids(&query.apply_values(entries())), ["u1", "c2", "u2"]);
    }

    #[test]
    fn since_compaction_starts_at_the_first_kept_entry() {
        let query = EntryQuery {
            since_compaction: true,
            ..EntryQuery::default()
        };
        assert_eq!(
            ids(&query.apply_values(entries())),
            ["k1", "z1", "c2", "u2", "c3"]
        );
        // A first kept id outside the list: from the compaction itself.
        let mut list = entries();
        list[3]["firstKeptEntryId"] = json!("gone");
        assert_eq!(ids(&query.apply_values(list))[0], "z1");
        // No compaction: everything.
        let list: Vec<Value> = entries().into_iter().filter(|e| e["id"] != "z1").collect();
        assert_eq!(query.apply_values(list).len(), 6);
    }

    #[test]
    fn max_string_clips_on_a_char_boundary() {
        let query = EntryQuery {
            max_string: Some(2),
            ..EntryQuery::default()
        };
        let found = query.apply_values(vec![json!({"type": "x", "id": "1", "t": "héllo"})]);
        assert_eq!(found[0]["t"], "h", "2 bytes end inside `é`");
        assert_eq!(found[0]["type"], "x");
    }

    #[test]
    fn the_sdk_wire_form_parses() {
        let query: EntryQuery = serde_json::from_str(
            r#"{"scope":"branch","sinceCompaction":true,"types":["message"],"roles":["user"],"last":1}"#,
        )
        .unwrap();
        assert_eq!(query.scope, QueryScope::Branch);
        assert!(query.since_compaction);
        assert_eq!(query.last, Some(1));
        let empty: EntryQuery = serde_json::from_str("{}").unwrap();
        assert_eq!(empty, EntryQuery::default());
    }
}
