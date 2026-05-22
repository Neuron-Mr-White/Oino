#![doc = r#"Ratatui chat interface state, rendering, input, settings, and theme primitives for Oino.

`oino-tui` is deliberately UI-only. It owns the terminal-facing state machine
([`TuiState`]), user actions ([`TuiAction`]), composer state ([`ComposerState`]),
keymap configuration ([`KeymapConfig`]), settings overlays ([`SettingsState`]),
resource browser models ([`PromptResource`] and [`SkillResource`]), and theme
resolution types ([`ThemeCatalog`] and [`ResolvedTheme`]). Runtime wiring,
provider/auth decisions, session persistence, filesystem/process tools, and
extension package loading belong to outer crates such as `oino-app`,
`oino-harness`, and the extension manager.

Contributor map:

- [`app`] handles focus, overlays, command execution decisions, extension surface
  controller state, and conversion from keys into [`TuiAction`] values.
- [`mod@render`] draws the current state with Ratatui widgets; keep rendering
  deterministic and side-effect free.
- [`keymap`] defines customizable shortcuts and context-aware key dispatch.
- [`command`] parses slash commands and builds command/resource suggestions.
- [`composer`] owns multiline input, collapsed pasted blocks, and resource-token
  expansion affordances.
- [`settings`] owns settings-page state for models, thinking, tools, themes,
  extension entry points, keymaps, and chat style.
- [`theme`] loads built-in/file/extension themes and resolves component roles.
- [`resource`] and [`message`] contain lightweight view models used by overlays
  and transcript rendering.

When adding UI behavior, prefer explicit focus/mode state in [`TuiState`], add a
semantic [`TuiAction`] when the app layer must notify the runtime, and update the
help/keymap/theme roles so visible hints stay truthful. User-facing resource and
theme docs live under `docs/resources.md` and `docs/theme-system/`.
"#]
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
    ChordState, ExtensionAutosuggestItem, ExtensionManagementItem, ExtensionManagementState,
    ExtensionManagementTarget, ExtensionManagementView, ExtensionShortcut, ExtensionThemeState,
    OverlayKind, SessionListItem, SessionsState, TuiFocus, TuiState, HELP_STATUS,
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
pub use theme::{
    builtin_theme_documents, normalize_theme_id, normalize_theme_token, parse_theme_color,
    resolve_effective_theme, EffectiveThemeScope, ResolvedTheme, Theme, ThemeCatalog,
    ThemeCatalogEntry, ThemeDiagnostic, ThemeDiagnosticLevel, ThemeDocument, ThemeMode,
    ThemeSettings, ThemeSource, ThemeSourceKind, ThemeSourceScope,
};
