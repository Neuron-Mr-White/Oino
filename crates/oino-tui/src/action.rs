#![forbid(unsafe_code)]

use crate::settings::{CollapseMode, CollapseTarget};
use oino_types::ThinkingLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TuiAction {
    None,
    SubmitPrompt(String),
    SetModel(String),
    SetThinkingLevel(ThinkingLevel),
    SetCollapseMode(CollapseTarget, CollapseMode),
    Quit,
}
