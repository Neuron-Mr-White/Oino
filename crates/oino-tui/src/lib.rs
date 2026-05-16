#![doc = "Ratatui chat interface primitives for Oino."]
#![forbid(unsafe_code)]

pub mod action;
pub mod app;
pub mod command;
pub mod composer;
pub mod message;
pub mod render;
pub mod settings;
pub mod theme;

pub use action::TuiAction;
pub use app::{ChordState, OverlayKind, TuiFocus, TuiState, HELP_STATUS};
pub use command::{
    command_query, command_suggestions_for, parse_command, CommandKind, CommandSpec,
    CommandSuggestionsState, CommandSuggestionsView, COMMANDS,
};
pub use composer::{is_newline_key, is_word_cursor_modifier, ComposerState, INPUT_PLACEHOLDER};
pub use message::{project_message, project_messages, MessageView};
pub use render::render;
pub use settings::{
    all_thinking_levels, collapse_mode_label, thinking_label, CollapseMode, CollapseTarget,
    ModelOption, SettingsAction, SettingsMenuItem, SettingsPage, SettingsState,
};
pub use theme::Theme;
