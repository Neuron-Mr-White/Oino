#![doc = "Ratatui chat interface primitives for Oino."]
#![forbid(unsafe_code)]

pub mod action;
pub mod app;
pub mod command;
pub mod composer;
mod fuzzy;
mod help;
pub mod keymap;
mod markdown;
pub mod message;
pub mod render;
pub mod resource;
pub mod settings;
mod text;
pub mod theme;
mod transcript;

pub use action::TuiAction;
pub use app::{
    ChordState, OverlayKind, SessionListItem, SessionsState, TuiFocus, TuiState, HELP_STATUS,
};
pub use command::{
    chat_style_value, collapse_mode_value, collapse_target_value, command_query,
    command_suggestions_for, parse_chat_style, parse_collapse_mode, parse_collapse_target,
    parse_command, parse_thinking_level, thinking_level_value, CommandKind, CommandSpec,
    CommandSuggestionCategory, CommandSuggestionItem, CommandSuggestionsState,
    CommandSuggestionsView, ParsedCommand, SettingsCommand, COMMANDS,
};
pub use composer::{is_newline_key, is_word_cursor_modifier, ComposerState, INPUT_PLACEHOLDER};
pub use keymap::{
    key_action_rows, KeyAction, KeyActionInfo, KeyContext, KeySequence, KeyStroke, KeymapConfig,
    KeymapMatch, KeymapPreset, ShortcutKind,
};
pub use message::{project_message, project_messages, MessageView, ToolCallView};
pub use render::{
    render, terminal_cursor_position, transcript_click_targets, transcript_url_overlays,
    transcript_visible_lines, TerminalClickTarget, TerminalClickTargetKind, TerminalUrlOverlay,
};
pub use resource::{PromptResource, ResourceBrowserState, SkillResource};
pub use settings::{
    all_thinking_levels, chat_style_label, collapse_mode_label, thinking_label, ChatStyle,
    CollapseMode, CollapseTarget, ModelOption, SettingsAction, SettingsMenuItem, SettingsPage,
    SettingsState, ToolSettingsItem, ToolSettingsScope,
};
pub use theme::Theme;
