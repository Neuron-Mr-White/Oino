#![forbid(unsafe_code)]

use crate::{
    app::ExtensionManagementTarget,
    keymap::KeymapConfig,
    settings::{ChatStyle, CollapseMode, CollapseTarget, ToolSettingsScope},
};
use oino_types::ThinkingLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    None,
    SubmitPrompt(String),
    SteerPrompt(String),
    QueuePrompt(String),
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
    RunExtensionUiAction {
        surface_id: String,
        action_id: String,
    },
    RunExtensionAction {
        action: String,
    },
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
    RemoveExtensionPackage {
        package_id: String,
        scope: ToolSettingsScope,
    },
    SetSessionTitle(String),
    AbortPrompt,
    Quit,
}
