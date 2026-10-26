//! `registry.wit`: event kinds and registration records.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::json::RawJson;

macro_rules! event_kinds {
    ($($(#[$doc:meta])* $variant:ident => $wire:literal,)*) => {
        /// `event-kind`: pi 1.4.0 extension events plus Oino host events (`oino_*`).
        /// [`EventKind::wire`] is the pi `type` string (kebab → snake).
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[non_exhaustive]
        pub enum EventKind {
            $($(#[$doc])* #[doc = concat!("`", $wire, "`")] $variant,)*
        }

        impl EventKind {
            /// Every event kind in WIT order.
            pub const ALL: &'static [EventKind] = &[$(EventKind::$variant,)*];

            /// Wire name (pi `type`).
            #[must_use]
            pub fn wire(self) -> &'static str {
                match self { $(EventKind::$variant => $wire,)* }
            }
        }

        impl FromStr for EventKind {
            type Err = UnknownEventKind;
            fn from_str(s: &str) -> Result<Self, Self::Err> {
                match s { $($wire => Ok(EventKind::$variant),)* _ => Err(UnknownEventKind(s.to_owned())) }
            }
        }
    };
}

event_kinds! {
    ProjectTrust => "project_trust",
    ResourcesDiscover => "resources_discover",
    McpServersChange => "mcp_servers_change",
    SessionStart => "session_start",
    SessionInfoChanged => "session_info_changed",
    SessionBeforeSwitch => "session_before_switch",
    SessionBeforeFork => "session_before_fork",
    SessionBeforeCompact => "session_before_compact",
    SessionCompact => "session_compact",
    SessionCompactFailed => "session_compact_failed",
    SessionShutdown => "session_shutdown",
    SessionBeforeTree => "session_before_tree",
    SessionTree => "session_tree",
    Context => "context",
    ContextWithSystem => "context_with_system",
    CacheWarmingDecision => "cache_warming_decision",
    BeforeProviderHeaders => "before_provider_headers",
    BeforeProviderRequest => "before_provider_request",
    AfterProviderResponse => "after_provider_response",
    ProviderStreamEvent => "provider_stream_event",
    BeforeAgentStart => "before_agent_start",
    AgentStart => "agent_start",
    AgentEnd => "agent_end",
    AgentBeforeSettle => "agent_before_settle",
    AgentSettled => "agent_settled",
    UiPromptStart => "ui_prompt_start",
    UiPromptEnd => "ui_prompt_end",
    TurnStart => "turn_start",
    TurnEnd => "turn_end",
    MessageStart => "message_start",
    MessageUpdate => "message_update",
    MessageEnd => "message_end",
    ToolExecutionStart => "tool_execution_start",
    ToolExecutionUpdate => "tool_execution_update",
    ToolExecutionEnd => "tool_execution_end",
    ModelSelect => "model_select",
    ThinkingLevelSelect => "thinking_level_select",
    ToolCall => "tool_call",
    ToolResult => "tool_result",
    UserBash => "user_bash",
    Input => "input",
    OinoTimer => "oino_timer",
    OinoBus => "oino_bus",
    OinoUiAction => "oino_ui_action",
    OinoEditor => "oino_editor",
    OinoSettingsChanged => "oino_settings_changed",
    OinoProcess => "oino_process",
    OinoAgent => "oino_agent",
    OinoTask => "oino_task",
}

/// An event name that is not part of `event-kind`.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("unknown event kind `{0}`")]
pub struct UnknownEventKind(pub String);

impl EventKind {
    /// Oino host events (`oino_*`), produced by the host rather than by pi's runner.
    #[must_use]
    pub fn is_oino(self) -> bool {
        self.wire().starts_with("oino_")
    }
}

impl fmt::Display for EventKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.wire())
    }
}

impl Serialize for EventKind {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.wire())
    }
}

impl<'de> Deserialize<'de> for EventKind {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// `exposure` (pi `ToolExposure`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Exposure {
    /// Declared to the model and callable by tools.
    #[default]
    Direct,
    /// Declared to the model only.
    ModelOnly,
    /// Declared through tool search.
    Deferred,
    /// Not declared.
    Hidden,
}

/// `execution-mode`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecutionMode {
    /// May run in parallel batches.
    #[default]
    Parallel,
    /// Forces sequential batches.
    Sequential,
}

/// `replay`: may the call be replayed after recovery.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Replay {
    /// Never replay.
    Never,
    /// Safe to replay.
    Safe,
}

/// `tool-kind` (ACP tool kind).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    /// Read.
    Read,
    /// Edit.
    Edit,
    /// Delete.
    Delete,
    /// Move.
    Move,
    /// Search.
    Search,
    /// Execute.
    Execute,
    /// Think.
    Think,
    /// Fetch.
    Fetch,
    /// Switch mode.
    SwitchMode,
    /// Other.
    #[default]
    Other,
}

/// `tool-namespace`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolNamespace {
    /// Namespace name.
    pub name: String,
    /// Aliases.
    #[serde(default)]
    pub aliases: Vec<String>,
    /// Description.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// Instructions.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub instructions: Option<String>,
}

/// `tool-annotations` (MCP-style hints).
#[allow(clippy::struct_field_names, reason = "mirrors the WIT record")]
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolAnnotations {
    /// Read-only hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_only: Option<bool>,
    /// Destructive hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub destructive: Option<bool>,
    /// Idempotent hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotent: Option<bool>,
    /// Open-world hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub open_world: Option<bool>,
}

/// `reaches-tools`: which tools a tool may call through `control.execute-tool`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "names", rename_all = "lowercase")]
pub enum ReachesTools {
    /// None.
    #[default]
    None,
    /// Every callable tool.
    All,
    /// Named tools.
    Names(Vec<String>),
    /// Decided per input by the guest export `reaches-tools`.
    Dynamic,
}

/// `tool-spec`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolSpec {
    /// Model-facing name.
    pub name: String,
    /// Frontend label.
    pub label: String,
    /// Model-facing description.
    pub description: String,
    /// JSON Schema object (draft 2020-12).
    pub parameters: RawJson,
    /// Output schema.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_schema: Option<RawJson>,
    /// One-line prompt snippet.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_snippet: Option<String>,
    /// Prompt guidelines.
    #[serde(default)]
    pub prompt_guidelines: Vec<String>,
    /// Exposure.
    #[serde(default)]
    pub exposure: Exposure,
    /// Activated on registration.
    pub default_active: bool,
    /// Execution mode.
    #[serde(default)]
    pub execution_mode: ExecutionMode,
    /// Replay policy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replay: Option<Replay>,
    /// ACP kind.
    #[serde(default)]
    pub kind: ToolKind,
    /// Namespace.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub namespace: Option<ToolNamespace>,
    /// Annotations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub annotations: Option<ToolAnnotations>,
    /// Nested tool reach.
    #[serde(default)]
    pub reaches_tools: ReachesTools,
    /// Guest implements `prepare-arguments` for this tool.
    #[serde(default)]
    pub prepare_arguments: bool,
    /// Guest implements `prepare-loadout` for this tool.
    #[serde(default)]
    pub prepare_loadout: bool,
    /// `ToolRenderHints` JSON (ui-protocol §6.4).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub render: Option<RawJson>,
}

/// `command-spec`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommandSpec {
    /// Name without `/`.
    pub name: String,
    /// Description.
    pub description: String,
    /// Argument hint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argument_hint: Option<String>,
    /// Guest implements `complete-command`.
    #[serde(default)]
    pub completions: bool,
    /// Not listed in `get_commands`.
    #[serde(default)]
    pub hidden: bool,
}

/// `shortcut-spec`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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

/// `flag-type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlagType {
    /// `--name` / `--no-name`.
    Boolean,
    /// `--name <value>`.
    String,
}

/// `flag-spec`.
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
    pub default_value: Option<RawJson>,
    /// Help text.
    #[serde(default)]
    pub description: String,
}

/// `renderer-target`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RendererTarget {
    /// Custom message (`customType`).
    CustomMessage,
    /// Custom entry (`customType`).
    CustomEntry,
    /// Tool (tool name).
    Tool,
}

/// `renderer-spec`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RendererSpec {
    /// Target.
    pub target: RendererTarget,
    /// `customType` or tool name.
    pub key: String,
}

/// `arbiter-roles` (runtime §7.6).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ArbiterRoles {
    /// Implements `propose-nudge`.
    pub nudge_provider: bool,
    /// Implements `wait-reason`.
    pub wait_source: bool,
}

/// `registration`: everything returned by `init`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    /// Subscribed events.
    #[serde(default)]
    pub events: Vec<EventKind>,
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
    /// Renderers.
    #[serde(default)]
    pub renderers: Vec<RendererSpec>,
    /// Service names.
    #[serde(default)]
    pub services: Vec<String>,
    /// Arbiter roles.
    #[serde(default)]
    pub arbiter: ArbiterRoles,
    /// `SettingsNamespace` JSON values.
    #[serde(default)]
    pub settings: Vec<RawJson>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wire_names_round_trip() {
        assert_eq!(EventKind::ALL.len(), 49);
        assert_eq!(EventKind::ALL.iter().filter(|k| !k.is_oino()).count(), 41);
        for kind in EventKind::ALL {
            assert_eq!(kind.wire().parse::<EventKind>().unwrap(), *kind);
            assert!(!kind.wire().contains('-'));
        }
        assert_eq!(
            serde_json::to_string(&EventKind::SessionBeforeCompact).unwrap(),
            "\"session_before_compact\""
        );
        assert!("nope".parse::<EventKind>().is_err());
    }
}
