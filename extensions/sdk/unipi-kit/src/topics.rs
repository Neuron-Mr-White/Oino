//! Bus topic names and payloads (unipi `UNIPI_EVENTS`, `events.ts`, `bus.ts`).
//!
//! Topic names are unchanged from unipi (`unipi:…`). The five sticky topics are published
//! with `sticky: true` so late subscribers read the last value with `bus::get`; Oino adds the
//! sticky topics that replace unipi's `Symbol.for` holders (unipi-port §2.2) and the work-tray
//! change signal. Payload structs serialise to the exact unipi JSON shapes (camelCase).

use serde::{Deserialize, Serialize};

/// Declares topic constants and the list of all names.
macro_rules! topics {
    ($($(#[$doc:meta])* $name:ident = $value:literal;)*) => {
        $( $(#[$doc])* pub const $name: &str = $value; )*
        /// Every unipi topic name in this module.
        pub const ALL: &[&str] = &[$($value),*];
    };
}

topics! {
    /// Module loaded and ready ([`ModuleReady`]).
    MODULE_READY = "unipi:module:ready";
    /// Permission mode changed ([`PermissionModeChanged`]).
    PERMISSION_MODE_CHANGED = "unipi:permission:mode:changed";
    /// Plan mode toggled ([`PlanModeChanged`]).
    PLAN_MODE_CHANGED = "unipi:plan:mode:changed";
    /// Ralph loop ended ([`RalphLoopEnd`]).
    RALPH_LOOP_END = "unipi:ralph:loop:end";
    /// Ralph iteration completed ([`RalphIterationDone`]).
    RALPH_ITERATION_DONE = "unipi:ralph:iteration:done";
    /// Long-horizon mode resolved for the turn ([`LhModeResolved`]).
    LONG_HORIZON_MODE_RESOLVED = "unipi:long-horizon:mode:resolved";
    /// Long-horizon owner lifecycle ([`LhOwnerChanged`]).
    LONG_HORIZON_OWNER_CHANGED = "unipi:long-horizon:owner:changed";
    /// Memory stored ([`MemoryStored`]).
    MEMORY_STORED = "unipi:memory:stored";
    /// Memory deleted ([`MemoryDeleted`]).
    MEMORY_DELETED = "unipi:memory:deleted";
    /// MCP server started ([`McpServer`]).
    MCP_SERVER_STARTED = "unipi:mcp:server:started";
    /// MCP server error ([`McpServer`]).
    MCP_SERVER_ERROR = "unipi:mcp:server:error";
    /// MCP tools registered ([`McpTools`]).
    MCP_TOOLS_REGISTERED = "unipi:mcp:tools:registered";
    /// Compaction completed ([`Compacted`]).
    COMPACTOR_COMPACTED = "unipi:compactor:compacted";
    /// Notification sent ([`NotificationSent`]).
    NOTIFICATION_SENT = "unipi:notify:sent";
    /// `ask_user` invoked ([`AskUserPrompt`]).
    ASK_USER_PROMPT = "unipi:ask-user:prompt";
    /// Update check performed ([`UpdateCheck`]).
    UPDATE_CHECK = "unipi:update:check";
    /// Update available ([`UpdateAvailable`]).
    UPDATE_AVAILABLE = "unipi:update:available";
    /// Update applied ([`UpdateApplied`]).
    UPDATE_APPLIED = "unipi:update:applied";
    /// Long-horizon footer state, sticky ([`LhState`]).
    LH_STATE = "unipi:long-horizon:state";
    /// Long-horizon progress snapshot, sticky (JSON, owned by long-horizon).
    LH_PROGRESS = "unipi:long-horizon:progress";
    /// Kanboard claims/autowork, sticky ([`KanboardStatus`]).
    KANBOARD_STATUS = "unipi:kanboard:status";
    /// Fusion lead/sidekick status, sticky ([`FusionStatus`]; `null` = cleared).
    FUSION_STATUS = "unipi:fusion:status";
    /// Workflow plan/permission mode, sticky ([`WorkflowStatus`]).
    WORKFLOW_STATUS = "unipi:workflow:status";
    /// Reveal hidden skills by name ([`SkillsReveal`]).
    SKILLS_REVEAL = "unipi:skills:reveal";
    /// Oino: background task list, sticky (replaces the shared registry, unipi-port §2.2).
    BG_TASKS = "unipi:bg:tasks";
    /// Oino: subagent list, sticky (replaces `unipi.subagents.shared-list`).
    SUBAGENTS_LIST = "unipi:subagents:list";
    /// Oino: graph-run completion `{runId, result}`.
    SUBAGENTS_RUN_DONE = "unipi:subagents:run-done";
    /// Oino: live tokens per second, sticky (replaces `unipi.footer.shared-tps`).
    FOOTER_TPS = "unipi:footer:tps";
    /// Oino: "a work-tray tab may have changed" (one-shot, any payload).
    WORK_TRAY_CHANGED = "unipi:work-tray:changed";
}

/// Topics published with `sticky: true`: unipi's five `STICKY_EVENTS` plus the Oino holder
/// replacements.
pub const STICKY: &[&str] = &[
    LH_STATE,
    LH_PROGRESS,
    KANBOARD_STATUS,
    FUSION_STATUS,
    WORKFLOW_STATUS,
    BG_TASKS,
    SUBAGENTS_LIST,
    FOOTER_TPS,
];

/// Foreign channels kept as bus topics (unipi `pi.events` names; `herdr:*` dropped, D22).
pub mod foreign {
    /// rpiv ask-user prompt mirror.
    pub const RPIV_ASK_USER_PROMPT: &str = "rpiv:ask-user:prompt";
    /// ACP `session/new` MCP servers (published by the ACP adapter).
    pub const ACP_MCP_SERVERS: &str = "oino:acp:mcp-servers";
    /// Prefix of host settings-change topics (`oino:settings:<namespace>`).
    pub const SETTINGS_PREFIX: &str = "oino:settings:";
}

/// Topic must be published sticky.
#[must_use]
pub fn is_sticky(topic: &str) -> bool {
    STICKY.contains(&topic)
}

/// A known unipi topic.
#[must_use]
pub fn is_unipi_topic(topic: &str) -> bool {
    ALL.contains(&topic)
}

/// A named payload type bound to its topic.
pub trait Topic: Serialize + for<'de> Deserialize<'de> {
    /// Topic name.
    const NAME: &'static str;
}

macro_rules! bind {
    ($($ty:ty => $name:ident;)*) => {
        $( impl Topic for $ty { const NAME: &'static str = $name; } )*
    };
}

/// `MODULE_READY`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModuleReady {
    /// Module name (`@pi-unipi/workflow`).
    pub name: String,
    /// Module version.
    pub version: String,
    /// Commands registered.
    pub commands: Vec<String>,
    /// Tools registered.
    pub tools: Vec<String>,
    /// Load time in milliseconds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_time_ms: Option<f64>,
}

/// `PERMISSION_MODE_CHANGED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionModeChanged {
    /// `ask | auto | full`.
    pub mode: String,
}

/// `PLAN_MODE_CHANGED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanModeChanged {
    /// Plan mode is on.
    pub active: bool,
    /// Plan file path (display form).
    pub plan_file: String,
    /// `toggled | approved | discarded`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `RALPH_LOOP_END`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalphLoopEnd {
    /// Loop name.
    pub name: String,
    /// Terminal reason.
    pub reason: String,
    /// Iterations completed.
    pub iterations: u32,
}

/// `RALPH_ITERATION_DONE`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RalphIterationDone {
    /// Loop name.
    pub name: String,
    /// Iteration just completed.
    pub iteration: u32,
    /// Iterations remaining.
    pub remaining: u32,
}

/// `LONG_HORIZON_MODE_RESOLVED`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LhModeResolved {
    /// Resolved mode id.
    pub mode: String,
    /// `explicit | owner | judge | default | …`.
    pub source: String,
    /// Judge confidence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// `LONG_HORIZON_OWNER_CHANGED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LhOwnerChanged {
    /// `activated | suspended | resumed | finished | cleared | restored`.
    pub event: String,
    /// Owner id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<String>,
    /// Owner kind.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// Owner status.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// Terminal/pause reason.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// `MEMORY_STORED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryStored {
    /// Memory id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Memory type.
    #[serde(rename = "type")]
    pub kind: String,
    /// Project name.
    pub project: String,
    /// `created | updated`.
    pub action: String,
}

/// `MEMORY_DELETED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryDeleted {
    /// Memory id.
    pub id: String,
    /// Title.
    pub title: String,
    /// Project name.
    pub project: String,
}

/// `MCP_SERVER_STARTED` / `MCP_SERVER_ERROR`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpServer {
    /// Server name.
    pub name: String,
    /// Tool count (started).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_count: Option<u32>,
    /// Error message (error).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Process id.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// `MCP_TOOLS_REGISTERED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct McpTools {
    /// Server name.
    pub server_name: String,
    /// Tool names.
    pub tool_names: Vec<String>,
}

/// `COMPACTOR_COMPACTED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compacted {
    /// `boundary | percentage | …`.
    pub method: String,
    /// Messages summarized.
    pub summarized: u64,
    /// Messages kept.
    pub kept: u64,
    /// Tokens before.
    pub tokens_before: u64,
    /// Tokens after.
    pub tokens_after: u64,
    /// Estimated tokens saved.
    pub tokens_saved: u64,
}

/// `NOTIFICATION_SENT`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationSent {
    /// Triggering event type.
    pub event_type: String,
    /// Platforms sent to.
    pub platforms: Vec<String>,
    /// All platforms succeeded.
    pub success: bool,
    /// Platforms suppressed (e.g. window focused).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suppressed_platforms: Option<Vec<String>>,
    /// ISO timestamp.
    pub timestamp: String,
}

/// `ASK_USER_PROMPT`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AskUserPrompt {
    /// Question.
    pub question: String,
    /// Additional context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    /// Option count.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option_count: Option<u32>,
    /// Multi-select.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_multiple: Option<bool>,
    /// Free-form allowed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub allow_freeform: Option<bool>,
}

/// `UPDATE_CHECK`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCheck {
    /// Installed version.
    pub current_version: String,
    /// Latest version found.
    pub latest_version: String,
    /// An update is available.
    pub update_available: bool,
    /// Check error.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// `UPDATE_AVAILABLE`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateAvailable {
    /// Installed version.
    pub current_version: String,
    /// Latest version.
    pub latest_version: String,
}

/// `UPDATE_APPLIED`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateApplied {
    /// Previous version.
    pub previous_version: String,
    /// New version.
    pub new_version: String,
}

/// Long-horizon owner summary in [`LhState`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LhOwnerSummary {
    /// Owner kind.
    pub kind: String,
    /// `active | parked`.
    pub status: String,
}

/// Last stop in [`LhState`].
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LhLastStop {
    /// `complete | paused | budget | other`.
    pub kind: String,
    /// Epoch ms.
    pub at: i64,
}

/// `LH_STATE` (sticky).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LhState {
    /// Display mode id; `none` = regular.
    pub mode: String,
    /// Mode of a parked owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub paused: Option<String>,
    /// Owner.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner: Option<LhOwnerSummary>,
    /// Last stop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_stop: Option<LhLastStop>,
}

/// `KANBOARD_STATUS` (sticky).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct KanboardStatus {
    /// Claimed task ids.
    pub claims: Vec<String>,
    /// Autowork on.
    pub autowork: bool,
    /// CLI prefix for board writes.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cli: Option<String>,
}

/// `WORKFLOW_STATUS` (sticky).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowStatus {
    /// Plan mode on.
    pub plan_mode: bool,
    /// Permission mode (`null` when unset).
    pub permission_mode: Option<String>,
}

/// `FUSION_STATUS` (sticky; published as `null` when cleared).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FusionStatus {
    /// Lead display name.
    pub lead_name: String,
    /// Lead `provider/id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_key: Option<String>,
    /// Sidekick `provider/id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidekick_key: Option<String>,
    /// Lead thinking level.
    pub lead_effort: String,
    /// Sidekick display name.
    pub sidekick_name: String,
    /// Sidekick thinking level.
    pub sidekick_effort: String,
    /// Estimated savings in USD.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub saved_usd: Option<f64>,
    /// A handoff is running.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub busy: Option<bool>,
    /// Lead tool calls while Fusion was active.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lead_tool_calls: Option<u64>,
    /// Sidekick tool calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sidekick_tool_calls: Option<u64>,
}

/// `SKILLS_REVEAL`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SkillsReveal {
    /// Skill names to reveal.
    pub names: Vec<String>,
}

bind! {
    ModuleReady => MODULE_READY;
    PermissionModeChanged => PERMISSION_MODE_CHANGED;
    PlanModeChanged => PLAN_MODE_CHANGED;
    RalphLoopEnd => RALPH_LOOP_END;
    RalphIterationDone => RALPH_ITERATION_DONE;
    LhModeResolved => LONG_HORIZON_MODE_RESOLVED;
    LhOwnerChanged => LONG_HORIZON_OWNER_CHANGED;
    MemoryStored => MEMORY_STORED;
    MemoryDeleted => MEMORY_DELETED;
    McpTools => MCP_TOOLS_REGISTERED;
    Compacted => COMPACTOR_COMPACTED;
    NotificationSent => NOTIFICATION_SENT;
    AskUserPrompt => ASK_USER_PROMPT;
    UpdateCheck => UPDATE_CHECK;
    UpdateAvailable => UPDATE_AVAILABLE;
    UpdateApplied => UPDATE_APPLIED;
    LhState => LH_STATE;
    KanboardStatus => KANBOARD_STATUS;
    WorkflowStatus => WORKFLOW_STATUS;
    Option<FusionStatus> => FUSION_STATUS;
    SkillsReveal => SKILLS_REVEAL;
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn unipi_event_names_and_sticky_set() {
        // 24 UNIPI_EVENTS + 5 Oino topics.
        assert_eq!(ALL.len(), 29);
        assert!(ALL.iter().all(|t| t.starts_with("unipi:")));
        let unipi_sticky = [
            LH_STATE,
            LH_PROGRESS,
            KANBOARD_STATUS,
            FUSION_STATUS,
            WORKFLOW_STATUS,
        ];
        assert!(unipi_sticky.iter().all(|t| is_sticky(t)));
        assert!(!is_sticky(MODULE_READY));
        assert!(!is_sticky(WORK_TRAY_CHANGED));
        assert!(is_unipi_topic("unipi:skills:reveal"));
        assert!(!is_unipi_topic("herdr:working"));
        assert!(!is_unipi_topic(foreign::RPIV_ASK_USER_PROMPT));
    }

    #[test]
    fn payloads_keep_unipi_shapes() {
        let v = serde_json::to_value(WorkflowStatus {
            plan_mode: true,
            permission_mode: None,
        })
        .unwrap_or_default();
        assert_eq!(v, json!({"planMode": true, "permissionMode": null}));
        let v = serde_json::to_value(MemoryStored {
            id: "m".into(),
            title: "t".into(),
            kind: "decision".into(),
            project: "p".into(),
            action: "created".into(),
        })
        .unwrap_or_default();
        assert_eq!(v["type"], "decision");
        let v = serde_json::to_value(LhState {
            mode: "none".into(),
            ..LhState::default()
        })
        .unwrap_or_default();
        assert_eq!(v, json!({"mode": "none"}));
        let back: LhOwnerChanged = serde_json::from_value(
            json!({"event": "finished", "ownerId": "o", "reason": "complete"}),
        )
        .unwrap_or_default();
        assert_eq!(back.owner_id.as_deref(), Some("o"));
        assert_eq!(<Option<FusionStatus> as Topic>::NAME, FUSION_STATUS);
    }
}
