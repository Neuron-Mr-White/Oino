#![forbid(unsafe_code)]

use crate::{
    app::ExtensionManagementTarget,
    ask_user::AskUserOutcome,
    command::{AgentMode, RalphCommand},
    keymap::KeymapConfig,
    settings::{
        ChatStyle, CollapseMode, CollapseTarget, NotifyEventKind, NotifyField, ToolSettingsScope,
    },
};
use oino_types::ThinkingLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    None,
    SubmitPrompt(String),
    SteerPrompt(String),
    QueuePrompt(String),
    OpenBtw,
    SubmitBtwPrompt(String),
    ResetBtwSession,
    ConfigureBtwModel(Option<String>),
    NewSession,
    ListSessions,
    OpenSession(String),
    ReloadResources,
    OpenInspect,
    ExportChatHtml,
    SetModel(String),
    SetThinkingLevel(ThinkingLevel),
    SetCollapseMode(CollapseTarget, CollapseMode),
    SetChatStyle(ChatStyle),
    SetKeymap(KeymapConfig),
    SetToolEnabled {
        name: String,
        scope: ToolSettingsScope,
        enabled: bool,
    },
    SetTheme {
        id: String,
        scope: ToolSettingsScope,
    },
    ResetTheme {
        scope: ToolSettingsScope,
    },
    SetNotifyEnabled {
        scope: ToolSettingsScope,
        enabled: bool,
    },
    SetNotifyField {
        scope: ToolSettingsScope,
        field: NotifyField,
        value: Option<String>,
    },
    SetNotifyEvent {
        scope: ToolSettingsScope,
        event: NotifyEventKind,
        enabled: bool,
    },
    RunExtensionUiAction {
        surface_id: String,
        action_id: String,
    },
    RunExtensionAction {
        action: String,
    },
    Compact,
    CompactMethodOverride {
        method: crate::command::CompactMethodOverride,
    },
    CompactThreshold {
        pct: Option<u8>,
    },
    CompactAuto {
        enabled: bool,
    },
    CompactModel {
        model: Option<Option<String>>,
    },
    CompactPrompt {
        path: Option<String>,
    },
    Recall {
        query: Option<String>,
    },
    RefreshUsage,
    RefreshAuthStatus {
        provider: Option<String>,
    },
    AnswerAskUser(AskUserOutcome),
    Ralph(RalphCommand),
    SetAgentMode(AgentMode),
    SetExtensionEnabled {
        target: ExtensionManagementTarget,
        id: String,
        scope: ToolSettingsScope,
        enabled: bool,
    },
    SetExtensionOverride {
        contribution_id: String,
        entry_key: String,
        scope: ToolSettingsScope,
    },
    ClearExtensionOverride {
        contribution_id: String,
        scope: ToolSettingsScope,
    },
    InstallExtensionPackage {
        source: String,
        scope: ToolSettingsScope,
    },
    UpdateExtensionPackages,
    RemoveExtensionPackage {
        package_id: String,
        scope: ToolSettingsScope,
    },
    SetSessionTitle(String),
    AuthQuickstart,
    RunExtensionCommand {
        input: String,
    },
    AbortPrompt,
    Quit,
}
