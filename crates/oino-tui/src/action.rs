#![forbid(unsafe_code)]

use crate::{
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
    SetSessionTitle(String),
    AbortPrompt,
    Quit,
}
