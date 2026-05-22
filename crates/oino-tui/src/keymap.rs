#![forbid(unsafe_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, str::FromStr};

#[derive(
    Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
)]
#[serde(rename_all = "snake_case")]
pub enum KeymapPreset {
    #[default]
    Chord,
    Combination,
}

impl KeymapPreset {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Chord => "Chord",
            Self::Combination => "Combination",
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 2] {
        [Self::Chord, Self::Combination]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShortcutKind {
    Chord,
    Combination,
}

impl ShortcutKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Chord => "Chord",
            Self::Combination => "Combination",
        }
    }

    #[must_use]
    pub const fn all() -> [Self; 2] {
        [Self::Chord, Self::Combination]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum KeyContext {
    Common,
    Global,
    Composer,
    CommandSuggestions,
    Transcript,
    Help,
    HelpSearch,
    SendPanel,
    SendPanelConfirm,
    Sessions,
    Search,
    ResourceBrowser,
    Inspect,
    Settings,
    SettingsTools,
    SettingsKeymaps,
    SettingsKeymapDetail,
    SettingsKeymapType,
    SettingsKeymapPreset,
    SettingsKeymapPresetConfirm,
}

impl KeyContext {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Common => "Common",
            Self::Global => "Global",
            Self::Composer => "Composer",
            Self::CommandSuggestions => "Suggestions",
            Self::Transcript => "Transcript",
            Self::Help => "Help",
            Self::HelpSearch => "Help Search",
            Self::SendPanel => "Send Panel",
            Self::SendPanelConfirm => "Send Confirm",
            Self::Sessions => "Sessions",
            Self::Search => "Search Input",
            Self::ResourceBrowser => "Resource Browser",
            Self::Inspect => "Inspect",
            Self::Settings => "Settings",
            Self::SettingsTools => "Settings Tools",
            Self::SettingsKeymaps => "Keymaps",
            Self::SettingsKeymapDetail => "Keymap Detail",
            Self::SettingsKeymapType => "Shortcut Type",
            Self::SettingsKeymapPreset => "Keymap Preset",
            Self::SettingsKeymapPresetConfirm => "Preset Confirm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyAction {
    CommonClose,
    CommonBack,
    CommonUp,
    CommonDown,
    CommonPageUp,
    CommonPageDown,
    CommonTop,
    CommonBottom,
    CommonConfirm,
    CommonSearch,
    CommonRefresh,
    CommonBackspace,
    CommonNext,
    CommonPrevious,
    AppQuit,
    HelpOpen,
    SettingsOpen,
    SendPanelOpen,
    TranscriptFocus,
    ComposerExpandReference,
    ComposerSubmit,
    ComposerNewline,
    ComposerQueuePrompt,
    ComposerDraftPrompt,
    SuggestionsClose,
    SuggestionsUp,
    SuggestionsDown,
    SuggestionsAccept,
    SuggestionsConfirm,
    TranscriptUnfocus,
    TranscriptPageUp,
    TranscriptPageDown,
    TranscriptLineUp,
    TranscriptLineDown,
    TranscriptTop,
    TranscriptBottom,
    HelpClose,
    HelpSearch,
    HelpUp,
    HelpDown,
    HelpPageUp,
    HelpPageDown,
    HelpTop,
    HelpBottom,
    SearchClose,
    SearchAccept,
    SearchBackspace,
    SearchUp,
    SearchDown,
    SearchPageUp,
    SearchPageDown,
    SearchTop,
    SearchBottom,
    SendPanelClose,
    SendPanelUp,
    SendPanelDown,
    SendPanelQueue,
    SendPanelDraft,
    SendPanelDelete,
    SendPanelLoad,
    ConfirmYes,
    ConfirmNo,
    SessionsClose,
    SessionsUp,
    SessionsDown,
    SessionsSearch,
    SessionsRefresh,
    SessionsOpen,
    ResourceClose,
    ResourceUp,
    ResourceDown,
    ResourceSearch,
    ResourceRefresh,
    ResourceComplete,
    InspectClose,
    InspectUp,
    InspectDown,
    InspectPageUp,
    InspectPageDown,
    InspectTop,
    InspectExportHtml,
    SettingsClose,
    SettingsBack,
    SettingsOpenPage,
    SettingsUp,
    SettingsDown,
    SettingsNext,
    SettingsPrevious,
    SettingsApply,
    SettingsSearch,
    SettingsToolToggleGlobal,
    SettingsToolToggleProject,
    KeymapEditChordKey,
    KeymapAddShortcut,
    KeymapRemoveShortcut,
    KeymapClearShortcuts,
    KeymapResetAction,
    KeymapSelectPreset,
    ExtensionSurfaceFocusNext,
    ExtensionSurfaceFocusPrevious,
    ExtensionSurfaceTabNext,
    ExtensionSurfaceTabPrevious,
    ExtensionSurfaceClose,
    ExtensionSidebarToggle,
    ExtensionMainPanelToggle,
}

#[derive(Debug, Clone, Copy)]
pub struct KeyActionInfo {
    pub action: KeyAction,
    pub context: KeyContext,
    pub label: &'static str,
    pub description: &'static str,
}

impl KeyAction {
    #[must_use]
    pub fn info(self) -> KeyActionInfo {
        match ACTION_INFOS
            .iter()
            .copied()
            .find(|info| info.action == self)
        {
            Some(info) => info,
            None => unreachable!("all key actions have metadata"),
        }
    }

    #[must_use]
    pub fn id(self) -> &'static str {
        match self {
            Self::CommonClose => "common.close",
            Self::CommonBack => "common.back",
            Self::CommonUp => "common.up",
            Self::CommonDown => "common.down",
            Self::CommonPageUp => "common.page_up",
            Self::CommonPageDown => "common.page_down",
            Self::CommonTop => "common.top",
            Self::CommonBottom => "common.bottom",
            Self::CommonConfirm => "common.confirm",
            Self::CommonSearch => "common.search",
            Self::CommonRefresh => "common.refresh",
            Self::CommonBackspace => "common.backspace",
            Self::CommonNext => "common.next",
            Self::CommonPrevious => "common.previous",
            Self::AppQuit => "app.quit",
            Self::HelpOpen => "help.open",
            Self::SettingsOpen => "settings.open",
            Self::SendPanelOpen => "send_panel.open",
            Self::TranscriptFocus => "transcript.focus",
            Self::ComposerExpandReference => "composer.expand_reference",
            Self::ComposerSubmit => "composer.submit",
            Self::ComposerNewline => "composer.newline",
            Self::ComposerQueuePrompt => "composer.queue_prompt",
            Self::ComposerDraftPrompt => "composer.draft_prompt",
            Self::SuggestionsClose => "suggestions.close",
            Self::SuggestionsUp => "suggestions.up",
            Self::SuggestionsDown => "suggestions.down",
            Self::SuggestionsAccept => "suggestions.accept",
            Self::SuggestionsConfirm => "suggestions.confirm",
            Self::TranscriptUnfocus => "transcript.unfocus",
            Self::TranscriptPageUp => "transcript.page_up",
            Self::TranscriptPageDown => "transcript.page_down",
            Self::TranscriptLineUp => "transcript.line_up",
            Self::TranscriptLineDown => "transcript.line_down",
            Self::TranscriptTop => "transcript.top",
            Self::TranscriptBottom => "transcript.bottom",
            Self::HelpClose => "help.close",
            Self::HelpSearch => "help.search",
            Self::HelpUp => "help.up",
            Self::HelpDown => "help.down",
            Self::HelpPageUp => "help.page_up",
            Self::HelpPageDown => "help.page_down",
            Self::HelpTop => "help.top",
            Self::HelpBottom => "help.bottom",
            Self::SearchClose => "search.close",
            Self::SearchAccept => "search.accept",
            Self::SearchBackspace => "search.backspace",
            Self::SearchUp => "search.up",
            Self::SearchDown => "search.down",
            Self::SearchPageUp => "search.page_up",
            Self::SearchPageDown => "search.page_down",
            Self::SearchTop => "search.top",
            Self::SearchBottom => "search.bottom",
            Self::SendPanelClose => "send_panel.close",
            Self::SendPanelUp => "send_panel.up",
            Self::SendPanelDown => "send_panel.down",
            Self::SendPanelQueue => "send_panel.queue",
            Self::SendPanelDraft => "send_panel.draft",
            Self::SendPanelDelete => "send_panel.delete",
            Self::SendPanelLoad => "send_panel.load",
            Self::ConfirmYes => "confirm.yes",
            Self::ConfirmNo => "confirm.no",
            Self::SessionsClose => "sessions.close",
            Self::SessionsUp => "sessions.up",
            Self::SessionsDown => "sessions.down",
            Self::SessionsSearch => "sessions.search",
            Self::SessionsRefresh => "sessions.refresh",
            Self::SessionsOpen => "sessions.open",
            Self::ResourceClose => "resources.close",
            Self::ResourceUp => "resources.up",
            Self::ResourceDown => "resources.down",
            Self::ResourceSearch => "resources.search",
            Self::ResourceRefresh => "resources.refresh",
            Self::ResourceComplete => "resources.complete",
            Self::InspectClose => "inspect.close",
            Self::InspectUp => "inspect.up",
            Self::InspectDown => "inspect.down",
            Self::InspectPageUp => "inspect.page_up",
            Self::InspectPageDown => "inspect.page_down",
            Self::InspectTop => "inspect.top",
            Self::InspectExportHtml => "inspect.export_html",
            Self::SettingsClose => "settings.close",
            Self::SettingsBack => "settings.back",
            Self::SettingsOpenPage => "settings.open_page",
            Self::SettingsUp => "settings.up",
            Self::SettingsDown => "settings.down",
            Self::SettingsNext => "settings.next",
            Self::SettingsPrevious => "settings.previous",
            Self::SettingsApply => "settings.apply",
            Self::SettingsSearch => "settings.search",
            Self::SettingsToolToggleGlobal => "settings.tools.toggle_global",
            Self::SettingsToolToggleProject => "settings.tools.toggle_project",
            Self::KeymapEditChordKey => "settings.keymaps.edit_chord_key",
            Self::KeymapAddShortcut => "settings.keymaps.add_shortcut",
            Self::KeymapRemoveShortcut => "settings.keymaps.remove_shortcut",
            Self::KeymapClearShortcuts => "settings.keymaps.clear_shortcuts",
            Self::KeymapResetAction => "settings.keymaps.reset_action",
            Self::KeymapSelectPreset => "settings.keymaps.select_preset",
            Self::ExtensionSurfaceFocusNext => "extensions.surface.focus_next",
            Self::ExtensionSurfaceFocusPrevious => "extensions.surface.focus_previous",
            Self::ExtensionSurfaceTabNext => "extensions.surface.tab_next",
            Self::ExtensionSurfaceTabPrevious => "extensions.surface.tab_previous",
            Self::ExtensionSurfaceClose => "extensions.surface.close",
            Self::ExtensionSidebarToggle => "extensions.sidebar.toggle",
            Self::ExtensionMainPanelToggle => "extensions.main_panel.toggle",
        }
    }
}

impl fmt::Display for KeyAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.id())
    }
}

pub const ACTION_INFOS: &[KeyActionInfo] = &[
    info(
        KeyAction::CommonClose,
        KeyContext::Common,
        "Close / Cancel",
        "close the current overlay, cancel search, or return from transient focus",
    ),
    info(
        KeyAction::CommonBack,
        KeyContext::Common,
        "Back",
        "return to the previous page inside an overlay",
    ),
    info(
        KeyAction::CommonUp,
        KeyContext::Common,
        "Move Up",
        "move the active list or document up",
    ),
    info(
        KeyAction::CommonDown,
        KeyContext::Common,
        "Move Down",
        "move the active list or document down",
    ),
    info(
        KeyAction::CommonPageUp,
        KeyContext::Common,
        "Page Up",
        "page the active list or document up",
    ),
    info(
        KeyAction::CommonPageDown,
        KeyContext::Common,
        "Page Down",
        "page the active list or document down",
    ),
    info(
        KeyAction::CommonTop,
        KeyContext::Common,
        "Jump Top",
        "jump the active list or document to top",
    ),
    info(
        KeyAction::CommonBottom,
        KeyContext::Common,
        "Jump Bottom",
        "jump the active list or document to bottom",
    ),
    info(
        KeyAction::CommonConfirm,
        KeyContext::Common,
        "Confirm / Open",
        "confirm the active selection",
    ),
    info(
        KeyAction::CommonSearch,
        KeyContext::Common,
        "Search",
        "start search in the active overlay",
    ),
    info(
        KeyAction::CommonRefresh,
        KeyContext::Common,
        "Refresh",
        "refresh the active browser",
    ),
    info(
        KeyAction::CommonBackspace,
        KeyContext::Common,
        "Backspace",
        "delete one character in active search input",
    ),
    info(
        KeyAction::CommonNext,
        KeyContext::Common,
        "Next",
        "move to the next focusable/settings item",
    ),
    info(
        KeyAction::CommonPrevious,
        KeyContext::Common,
        "Previous",
        "move to the previous focusable/settings item",
    ),
    info(
        KeyAction::AppQuit,
        KeyContext::Global,
        "Quit",
        "quit Oino after confirmation",
    ),
    info(
        KeyAction::HelpOpen,
        KeyContext::Global,
        "Open Help",
        "open keyboard and command help",
    ),
    info(
        KeyAction::SettingsOpen,
        KeyContext::Global,
        "Open Settings",
        "open settings pages",
    ),
    info(
        KeyAction::SendPanelOpen,
        KeyContext::Global,
        "Open Send Panel",
        "open steering, queue, and draft panel",
    ),
    info(
        KeyAction::TranscriptFocus,
        KeyContext::Global,
        "Focus Transcript",
        "move focus from composer to transcript",
    ),
    info(
        KeyAction::ComposerExpandReference,
        KeyContext::Global,
        "Expand Reference",
        "expand a collapsed paste block or prompt reference",
    ),
    info(
        KeyAction::ComposerSubmit,
        KeyContext::Composer,
        "Submit Composer",
        "submit current input",
    ),
    info(
        KeyAction::ComposerNewline,
        KeyContext::Composer,
        "Insert Newline",
        "insert a composer newline",
    ),
    info(
        KeyAction::ComposerQueuePrompt,
        KeyContext::Composer,
        "Queue Composer",
        "queue the current composer input for the next turn",
    ),
    info(
        KeyAction::ComposerDraftPrompt,
        KeyContext::Composer,
        "Draft Composer",
        "move the current composer input to Draft",
    ),
    info(
        KeyAction::SuggestionsClose,
        KeyContext::CommandSuggestions,
        "Close Suggestions",
        "dismiss command suggestions",
    ),
    info(
        KeyAction::SuggestionsUp,
        KeyContext::CommandSuggestions,
        "Suggestion Up",
        "move suggestion selection up",
    ),
    info(
        KeyAction::SuggestionsDown,
        KeyContext::CommandSuggestions,
        "Suggestion Down",
        "move suggestion selection down",
    ),
    info(
        KeyAction::SuggestionsAccept,
        KeyContext::CommandSuggestions,
        "Accept Suggestion",
        "accept suggestion without submitting",
    ),
    info(
        KeyAction::SuggestionsConfirm,
        KeyContext::CommandSuggestions,
        "Confirm Suggestion",
        "accept suggestion and submit when ready",
    ),
    info(
        KeyAction::TranscriptUnfocus,
        KeyContext::Transcript,
        "Return to Composer",
        "leave transcript focus",
    ),
    info(
        KeyAction::TranscriptPageUp,
        KeyContext::Global,
        "Transcript Page Up",
        "scroll transcript up by page",
    ),
    info(
        KeyAction::TranscriptPageDown,
        KeyContext::Global,
        "Transcript Page Down",
        "scroll transcript down by page",
    ),
    info(
        KeyAction::TranscriptLineUp,
        KeyContext::Transcript,
        "Transcript Line Up",
        "scroll transcript up by line",
    ),
    info(
        KeyAction::TranscriptLineDown,
        KeyContext::Transcript,
        "Transcript Line Down",
        "scroll transcript down by line",
    ),
    info(
        KeyAction::TranscriptTop,
        KeyContext::Transcript,
        "Transcript Top",
        "jump transcript to top",
    ),
    info(
        KeyAction::TranscriptBottom,
        KeyContext::Transcript,
        "Transcript Bottom",
        "jump transcript to bottom",
    ),
    info(
        KeyAction::HelpClose,
        KeyContext::Help,
        "Close Help",
        "close help overlay",
    ),
    info(
        KeyAction::HelpSearch,
        KeyContext::Help,
        "Search Help",
        "start help fuzzy search",
    ),
    info(
        KeyAction::HelpUp,
        KeyContext::Help,
        "Help Up",
        "scroll help up",
    ),
    info(
        KeyAction::HelpDown,
        KeyContext::Help,
        "Help Down",
        "scroll help down",
    ),
    info(
        KeyAction::HelpPageUp,
        KeyContext::Help,
        "Help Page Up",
        "page help up",
    ),
    info(
        KeyAction::HelpPageDown,
        KeyContext::Help,
        "Help Page Down",
        "page help down",
    ),
    info(
        KeyAction::HelpTop,
        KeyContext::Help,
        "Help Top",
        "jump help to top",
    ),
    info(
        KeyAction::HelpBottom,
        KeyContext::Help,
        "Help Bottom",
        "jump help to bottom",
    ),
    info(
        KeyAction::SearchClose,
        KeyContext::Search,
        "Clear Search",
        "close or clear active search input",
    ),
    info(
        KeyAction::SearchAccept,
        KeyContext::Search,
        "Accept Search",
        "keep active search results",
    ),
    info(
        KeyAction::SearchBackspace,
        KeyContext::Search,
        "Search Backspace",
        "delete one search character",
    ),
    info(
        KeyAction::SearchUp,
        KeyContext::Search,
        "Search Up",
        "move search selection up",
    ),
    info(
        KeyAction::SearchDown,
        KeyContext::Search,
        "Search Down",
        "move search selection down",
    ),
    info(
        KeyAction::SearchPageUp,
        KeyContext::Search,
        "Search Page Up",
        "page search results up",
    ),
    info(
        KeyAction::SearchPageDown,
        KeyContext::Search,
        "Search Page Down",
        "page search results down",
    ),
    info(
        KeyAction::SearchTop,
        KeyContext::Search,
        "Search Top",
        "jump search results to top",
    ),
    info(
        KeyAction::SearchBottom,
        KeyContext::Search,
        "Search Bottom",
        "jump search results to bottom",
    ),
    info(
        KeyAction::SendPanelClose,
        KeyContext::SendPanel,
        "Close Send Panel",
        "close send panel",
    ),
    info(
        KeyAction::SendPanelUp,
        KeyContext::SendPanel,
        "Send Panel Up",
        "move send panel selection up",
    ),
    info(
        KeyAction::SendPanelDown,
        KeyContext::SendPanel,
        "Send Panel Down",
        "move send panel selection down",
    ),
    info(
        KeyAction::SendPanelQueue,
        KeyContext::SendPanel,
        "Queue Prompt",
        "queue current input for the next turn",
    ),
    info(
        KeyAction::SendPanelDraft,
        KeyContext::SendPanel,
        "Draft Prompt",
        "move current input to draft",
    ),
    info(
        KeyAction::SendPanelDelete,
        KeyContext::SendPanel,
        "Delete Panel Item",
        "delete selected queued or draft item",
    ),
    info(
        KeyAction::SendPanelLoad,
        KeyContext::SendPanel,
        "Load Panel Item",
        "load selected panel item into the composer",
    ),
    info(
        KeyAction::ConfirmYes,
        KeyContext::SendPanelConfirm,
        "Confirm Yes",
        "answer yes in a confirmation",
    ),
    info(
        KeyAction::ConfirmNo,
        KeyContext::SendPanelConfirm,
        "Confirm No",
        "answer no in a confirmation",
    ),
    info(
        KeyAction::SessionsClose,
        KeyContext::Sessions,
        "Close Sessions",
        "close sessions browser",
    ),
    info(
        KeyAction::SessionsUp,
        KeyContext::Sessions,
        "Sessions Up",
        "move session selection up",
    ),
    info(
        KeyAction::SessionsDown,
        KeyContext::Sessions,
        "Sessions Down",
        "move session selection down",
    ),
    info(
        KeyAction::SessionsSearch,
        KeyContext::Sessions,
        "Search Sessions",
        "start sessions search",
    ),
    info(
        KeyAction::SessionsRefresh,
        KeyContext::Sessions,
        "Refresh Sessions",
        "reload saved sessions",
    ),
    info(
        KeyAction::SessionsOpen,
        KeyContext::Sessions,
        "Open Session",
        "open selected session",
    ),
    info(
        KeyAction::ResourceClose,
        KeyContext::ResourceBrowser,
        "Close Resource Browser",
        "close prompts or skills browser",
    ),
    info(
        KeyAction::ResourceUp,
        KeyContext::ResourceBrowser,
        "Resource Up",
        "move resource selection up",
    ),
    info(
        KeyAction::ResourceDown,
        KeyContext::ResourceBrowser,
        "Resource Down",
        "move resource selection down",
    ),
    info(
        KeyAction::ResourceSearch,
        KeyContext::ResourceBrowser,
        "Search Resources",
        "start resource search",
    ),
    info(
        KeyAction::ResourceRefresh,
        KeyContext::ResourceBrowser,
        "Refresh Resources",
        "reload prompts and skills",
    ),
    info(
        KeyAction::ResourceComplete,
        KeyContext::ResourceBrowser,
        "Complete Resource",
        "insert selected prompt or skill command",
    ),
    info(
        KeyAction::InspectClose,
        KeyContext::Inspect,
        "Close Inspect",
        "close inspect overlay",
    ),
    info(
        KeyAction::InspectUp,
        KeyContext::Inspect,
        "Inspect Up",
        "scroll inspect up",
    ),
    info(
        KeyAction::InspectDown,
        KeyContext::Inspect,
        "Inspect Down",
        "scroll inspect down",
    ),
    info(
        KeyAction::InspectPageUp,
        KeyContext::Inspect,
        "Inspect Page Up",
        "page inspect up",
    ),
    info(
        KeyAction::InspectPageDown,
        KeyContext::Inspect,
        "Inspect Page Down",
        "page inspect down",
    ),
    info(
        KeyAction::InspectTop,
        KeyContext::Inspect,
        "Inspect Top",
        "jump inspect to top",
    ),
    info(
        KeyAction::InspectExportHtml,
        KeyContext::Inspect,
        "Export Chat HTML",
        "export chat HTML from inspect",
    ),
    info(
        KeyAction::SettingsClose,
        KeyContext::Settings,
        "Close Settings",
        "close settings overlay",
    ),
    info(
        KeyAction::SettingsBack,
        KeyContext::Settings,
        "Settings Back",
        "return to settings menu",
    ),
    info(
        KeyAction::SettingsOpenPage,
        KeyContext::Settings,
        "Open Settings Page",
        "open selected settings page",
    ),
    info(
        KeyAction::SettingsUp,
        KeyContext::Settings,
        "Settings Up",
        "move settings selection up",
    ),
    info(
        KeyAction::SettingsDown,
        KeyContext::Settings,
        "Settings Down",
        "move settings selection down",
    ),
    info(
        KeyAction::SettingsNext,
        KeyContext::Settings,
        "Settings Next",
        "move to next settings item",
    ),
    info(
        KeyAction::SettingsPrevious,
        KeyContext::Settings,
        "Settings Previous",
        "move to previous settings item",
    ),
    info(
        KeyAction::SettingsApply,
        KeyContext::Settings,
        "Apply Setting",
        "apply selected setting",
    ),
    info(
        KeyAction::SettingsSearch,
        KeyContext::Settings,
        "Search Settings List",
        "start search inside a settings list",
    ),
    info(
        KeyAction::SettingsToolToggleGlobal,
        KeyContext::SettingsTools,
        "Toggle Global Tool",
        "toggle global tool availability",
    ),
    info(
        KeyAction::SettingsToolToggleProject,
        KeyContext::SettingsTools,
        "Toggle Project Tool",
        "toggle project tool availability",
    ),
    info(
        KeyAction::KeymapEditChordKey,
        KeyContext::SettingsKeymaps,
        "Edit Chord Key",
        "set the global chord prefix key",
    ),
    info(
        KeyAction::KeymapAddShortcut,
        KeyContext::SettingsKeymapDetail,
        "Add Shortcut",
        "add another shortcut for an action",
    ),
    info(
        KeyAction::KeymapRemoveShortcut,
        KeyContext::SettingsKeymapDetail,
        "Remove Shortcut",
        "remove selected shortcut",
    ),
    info(
        KeyAction::KeymapClearShortcuts,
        KeyContext::SettingsKeymapDetail,
        "Clear Shortcuts",
        "unassign all shortcuts for an action",
    ),
    info(
        KeyAction::KeymapResetAction,
        KeyContext::SettingsKeymapDetail,
        "Reset Action",
        "reset one action to preset defaults",
    ),
    info(
        KeyAction::KeymapSelectPreset,
        KeyContext::SettingsKeymaps,
        "Select Preset",
        "reset all keybinds to a preset",
    ),
    info(
        KeyAction::ExtensionSurfaceFocusNext,
        KeyContext::Global,
        "Focus Next Extension Surface",
        "move focus to the next visible extension surface slot",
    ),
    info(
        KeyAction::ExtensionSurfaceFocusPrevious,
        KeyContext::Global,
        "Focus Previous Extension Surface",
        "move focus to the previous visible extension surface slot",
    ),
    info(
        KeyAction::ExtensionSurfaceTabNext,
        KeyContext::Global,
        "Next Extension Tab",
        "activate the next extension surface in the focused slot",
    ),
    info(
        KeyAction::ExtensionSurfaceTabPrevious,
        KeyContext::Global,
        "Previous Extension Tab",
        "activate the previous extension surface in the focused slot",
    ),
    info(
        KeyAction::ExtensionSurfaceClose,
        KeyContext::Global,
        "Close Extension Surface",
        "hide the focused extension surface slot",
    ),
    info(
        KeyAction::ExtensionSidebarToggle,
        KeyContext::Global,
        "Toggle Extension Sidebar",
        "show or hide extension sidebar slots",
    ),
    info(
        KeyAction::ExtensionMainPanelToggle,
        KeyContext::Global,
        "Toggle Extension Main Panel",
        "show or hide extension main panel slots",
    ),
];

const fn info(
    action: KeyAction,
    context: KeyContext,
    label: &'static str,
    description: &'static str,
) -> KeyActionInfo {
    KeyActionInfo {
        action,
        context,
        label,
        description,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeyStroke {
    code: StrokeCode,
    modifiers: StrokeModifiers,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
struct StrokeModifiers {
    ctrl: bool,
    alt: bool,
    shift: bool,
    super_key: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
enum StrokeCode {
    Char(char),
    Enter,
    Esc,
    Backspace,
    Delete,
    Tab,
    BackTab,
    Left,
    Right,
    Up,
    Down,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    F(u8),
    Space,
}

impl KeyStroke {
    #[must_use]
    pub fn from_event(key: KeyEvent) -> Option<Self> {
        let mut modifiers = StrokeModifiers {
            ctrl: key.modifiers.contains(KeyModifiers::CONTROL),
            alt: key.modifiers.contains(KeyModifiers::ALT),
            shift: key.modifiers.contains(KeyModifiers::SHIFT),
            super_key: key.modifiers.contains(KeyModifiers::SUPER)
                || key.modifiers.contains(KeyModifiers::META),
        };
        let code = match key.code {
            KeyCode::Char(' ') => StrokeCode::Space,
            KeyCode::Char(ch) => {
                if modifiers.shift && ch.is_ascii_uppercase() {
                    modifiers.shift = false;
                }
                StrokeCode::Char(ch)
            }
            KeyCode::Enter => StrokeCode::Enter,
            KeyCode::Esc => StrokeCode::Esc,
            KeyCode::Backspace => StrokeCode::Backspace,
            KeyCode::Delete => StrokeCode::Delete,
            KeyCode::Tab => StrokeCode::Tab,
            KeyCode::BackTab => {
                modifiers.shift = true;
                StrokeCode::Tab
            }
            KeyCode::Left => StrokeCode::Left,
            KeyCode::Right => StrokeCode::Right,
            KeyCode::Up => StrokeCode::Up,
            KeyCode::Down => StrokeCode::Down,
            KeyCode::Home => StrokeCode::Home,
            KeyCode::End => StrokeCode::End,
            KeyCode::PageUp => StrokeCode::PageUp,
            KeyCode::PageDown => StrokeCode::PageDown,
            KeyCode::Insert => StrokeCode::Insert,
            KeyCode::F(n) => StrokeCode::F(n),
            _ => return None,
        };
        Some(Self { code, modifiers })
    }

    #[must_use]
    pub const fn is_escape(self) -> bool {
        matches!(self.code, StrokeCode::Esc) && self.modifiers.is_empty()
    }

    #[must_use]
    pub const fn is_plain_text_key(self) -> bool {
        self.modifiers.is_empty() && matches!(self.code, StrokeCode::Char(_) | StrokeCode::Space)
    }

    #[must_use]
    pub fn matches_event(self, key: KeyEvent) -> bool {
        Self::from_event(key) == Some(self)
    }
}

impl StrokeModifiers {
    const fn is_empty(self) -> bool {
        !self.ctrl && !self.alt && !self.shift && !self.super_key
    }
}

impl fmt::Display for KeyStroke {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut parts = Vec::new();
        if self.modifiers.ctrl {
            parts.push("Ctrl".to_string());
        }
        if self.modifiers.alt {
            parts.push("Alt".to_string());
        }
        if self.modifiers.shift {
            parts.push("Shift".to_string());
        }
        if self.modifiers.super_key {
            parts.push("Super".to_string());
        }
        parts.push(match self.code {
            StrokeCode::Char(ch) if !self.modifiers.is_empty() && ch.is_ascii_alphabetic() => {
                ch.to_ascii_uppercase().to_string()
            }
            StrokeCode::Char(ch) => ch.to_string(),
            StrokeCode::Enter => "Enter".into(),
            StrokeCode::Esc => "Esc".into(),
            StrokeCode::Backspace => "Backspace".into(),
            StrokeCode::Delete => "Delete".into(),
            StrokeCode::Tab => "Tab".into(),
            StrokeCode::BackTab => "Shift-Tab".into(),
            StrokeCode::Left => "Left".into(),
            StrokeCode::Right => "Right".into(),
            StrokeCode::Up => "Up".into(),
            StrokeCode::Down => "Down".into(),
            StrokeCode::Home => "Home".into(),
            StrokeCode::End => "End".into(),
            StrokeCode::PageUp => "PgUp".into(),
            StrokeCode::PageDown => "PgDn".into(),
            StrokeCode::Insert => "Insert".into(),
            StrokeCode::F(n) => format!("F{n}"),
            StrokeCode::Space => "Space".into(),
        });
        f.write_str(&parts.join("-"))
    }
}

impl FromStr for KeyStroke {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let value = value.trim();
        if value.is_empty() {
            return Err("empty key stroke".into());
        }
        let parts = value.split('-').collect::<Vec<_>>();
        let (modifiers, key_parts) = parts.split_at(parts.len().saturating_sub(1));
        let mut parsed_modifiers = StrokeModifiers::default();
        for modifier in modifiers {
            match modifier.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => parsed_modifiers.ctrl = true,
                "alt" | "option" => parsed_modifiers.alt = true,
                "shift" => parsed_modifiers.shift = true,
                "cmd" | "command" | "super" | "win" | "meta" => parsed_modifiers.super_key = true,
                unknown => return Err(format!("unknown modifier `{unknown}`")),
            }
        }
        let Some(key) = key_parts.first().copied() else {
            return Err("missing key".into());
        };
        let lower = key.to_ascii_lowercase();
        let code = match lower.as_str() {
            "enter" | "return" => StrokeCode::Enter,
            "esc" | "escape" => StrokeCode::Esc,
            "backspace" | "bs" => StrokeCode::Backspace,
            "delete" | "del" => StrokeCode::Delete,
            "tab" => StrokeCode::Tab,
            "backtab" => StrokeCode::BackTab,
            "left" => StrokeCode::Left,
            "right" => StrokeCode::Right,
            "up" => StrokeCode::Up,
            "down" => StrokeCode::Down,
            "home" => StrokeCode::Home,
            "end" => StrokeCode::End,
            "pageup" | "pgup" => StrokeCode::PageUp,
            "pagedown" | "pgdn" => StrokeCode::PageDown,
            "insert" | "ins" => StrokeCode::Insert,
            "space" => StrokeCode::Space,
            key if key.starts_with('f') && key.len() > 1 => {
                let n = key[1..]
                    .parse::<u8>()
                    .map_err(|_| format!("invalid function key `{key}`"))?;
                StrokeCode::F(n)
            }
            _ => {
                let mut chars = key.chars();
                match (chars.next(), chars.next()) {
                    (Some(ch), None) => StrokeCode::Char(ch),
                    _ => return Err(format!("unknown key `{key}`")),
                }
            }
        };
        if parsed_modifiers.shift && matches!(code, StrokeCode::Tab) {
            parsed_modifiers.shift = true;
        }
        Ok(Self {
            code,
            modifiers: parsed_modifiers,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct KeySequence(Vec<KeyStroke>);

impl KeySequence {
    #[must_use]
    pub fn new(strokes: Vec<KeyStroke>) -> Option<Self> {
        if strokes.is_empty() {
            None
        } else {
            Some(Self(strokes))
        }
    }

    #[must_use]
    pub fn from_event(key: KeyEvent) -> Option<Self> {
        KeyStroke::from_event(key).and_then(|stroke| Self::new(vec![stroke]))
    }

    #[must_use]
    pub fn chord(prefix: KeyStroke, suffix: KeyStroke) -> Self {
        Self(vec![prefix, suffix])
    }

    pub fn replace_first(&mut self, old: KeyStroke, new: KeyStroke) {
        if self.0.first().copied() == Some(old) {
            self.0[0] = new;
        }
    }

    #[must_use]
    pub fn strokes(&self) -> &[KeyStroke] {
        &self.0
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    #[must_use]
    pub fn starts_with(&self, strokes: &[KeyStroke]) -> bool {
        self.0.starts_with(strokes)
    }
}

impl fmt::Display for KeySequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts = self.0.iter().map(ToString::to_string).collect::<Vec<_>>();
        f.write_str(&parts.join(" "))
    }
}

impl FromStr for KeySequence {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let strokes = value
            .split_whitespace()
            .map(KeyStroke::from_str)
            .collect::<Result<Vec<_>, _>>()?;
        Self::new(strokes).ok_or_else(|| "empty key sequence".into())
    }
}

impl Serialize for KeyStroke {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for KeyStroke {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse::<Self>().map_err(serde::de::Error::custom)
    }
}

impl Serialize for KeySequence {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for KeySequence {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        value.parse::<Self>().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct KeymapConfig {
    pub preset: KeymapPreset,
    #[serde(default = "default_chord_key")]
    pub chord_key: KeyStroke,
    pub bindings: BTreeMap<KeyAction, Vec<KeySequence>>,
}

impl Default for KeymapConfig {
    fn default() -> Self {
        Self::for_preset(KeymapPreset::Chord)
    }
}

impl KeymapConfig {
    #[must_use]
    pub fn for_preset(preset: KeymapPreset) -> Self {
        let chord_key = default_chord_key();
        let mut bindings = BTreeMap::new();
        for info in key_action_rows() {
            bindings.insert(
                info.action,
                default_bindings(info.action, preset, chord_key),
            );
        }
        Self {
            preset,
            chord_key,
            bindings,
        }
    }

    #[must_use]
    pub fn bindings_for(&self, action: KeyAction) -> Vec<KeySequence> {
        let canonical = canonical_action(action);
        self.bindings
            .get(&canonical)
            .cloned()
            .unwrap_or_else(|| default_bindings(canonical, self.preset, self.chord_key))
    }

    #[must_use]
    pub fn primary_label(&self, action: KeyAction) -> String {
        self.bindings_for(action)
            .first()
            .map_or_else(|| "Unassigned".into(), ToString::to_string)
    }

    #[must_use]
    pub fn label_for(&self, action: KeyAction) -> String {
        let bindings = self.bindings_for(action);
        if bindings.is_empty() {
            "Unassigned".into()
        } else {
            bindings
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>()
                .join(", ")
        }
    }

    pub fn set_bindings(&mut self, action: KeyAction, bindings: Vec<KeySequence>) {
        self.bindings.insert(canonical_action(action), bindings);
    }

    pub fn reset_action(&mut self, action: KeyAction) {
        let canonical = canonical_action(action);
        self.bindings.insert(
            canonical,
            default_bindings(canonical, self.preset, self.chord_key),
        );
    }

    pub fn set_chord_key(&mut self, chord_key: KeyStroke) {
        let old = self.chord_key;
        if old == chord_key {
            return;
        }
        for bindings in self.bindings.values_mut() {
            for binding in bindings {
                if binding.len() > 1 {
                    binding.replace_first(old, chord_key);
                }
            }
        }
        self.chord_key = chord_key;
    }

    #[must_use]
    pub fn chord_key_conflict(&self, candidate: KeyStroke) -> Option<KeyAction> {
        for info in key_action_rows() {
            if self.bindings_for(info.action).iter().any(|binding| {
                binding.len() == 1 && binding.strokes().first().copied() == Some(candidate)
            }) {
                return Some(info.action);
            }
        }
        None
    }

    #[must_use]
    pub fn conflict_for(
        &self,
        action: KeyAction,
        replacement_index: Option<usize>,
        candidate: &KeySequence,
    ) -> Option<KeyAction> {
        let canonical = canonical_action(action);
        let contexts = effective_contexts(canonical);
        for info in ACTION_INFOS {
            if !contexts.contains(&info.context) || canonical_action(info.action) == canonical {
                continue;
            }
            if self.bindings_for(info.action).iter().any(|binding| {
                binding == candidate
                    || binding.starts_with(candidate.strokes())
                    || candidate.starts_with(binding.strokes())
            }) {
                return Some(info.action);
            }
        }
        self.bindings_for(canonical)
            .iter()
            .enumerate()
            .find(|(index, binding)| {
                Some(*index) != replacement_index
                    && (*binding == candidate
                        || binding.starts_with(candidate.strokes())
                        || candidate.starts_with(binding.strokes()))
            })
            .map(|_| canonical)
    }

    #[must_use]
    pub fn resolve(&self, contexts: &[KeyContext], strokes: &[KeyStroke]) -> KeymapMatch {
        if strokes.is_empty() {
            return KeymapMatch::None;
        }
        let mut pending = false;
        for context in contexts {
            for info in ACTION_INFOS.iter().filter(|info| info.context == *context) {
                for binding in self.bindings_for(info.action) {
                    if binding.strokes() == strokes {
                        return KeymapMatch::Matched(info.action);
                    }
                    if binding.len() > strokes.len() && binding.starts_with(strokes) {
                        pending = true;
                    }
                }
            }
        }
        if pending {
            KeymapMatch::Pending
        } else {
            KeymapMatch::None
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeymapMatch {
    None,
    Pending,
    Matched(KeyAction),
}

#[must_use]
pub fn key_action_rows() -> Vec<KeyActionInfo> {
    ACTION_INFOS
        .iter()
        .copied()
        .filter(|info| canonical_action(info.action) == info.action)
        .collect()
}

#[must_use]
pub const fn canonical_action(action: KeyAction) -> KeyAction {
    match action {
        KeyAction::SuggestionsClose
        | KeyAction::TranscriptUnfocus
        | KeyAction::HelpClose
        | KeyAction::SearchClose
        | KeyAction::SendPanelClose
        | KeyAction::SessionsClose
        | KeyAction::ResourceClose
        | KeyAction::InspectClose
        | KeyAction::SettingsClose => KeyAction::CommonClose,
        KeyAction::SettingsBack => KeyAction::CommonBack,
        KeyAction::TranscriptLineUp
        | KeyAction::HelpUp
        | KeyAction::SendPanelUp
        | KeyAction::SessionsUp
        | KeyAction::ResourceUp
        | KeyAction::InspectUp
        | KeyAction::SettingsUp => KeyAction::CommonUp,
        KeyAction::TranscriptLineDown
        | KeyAction::HelpDown
        | KeyAction::SendPanelDown
        | KeyAction::SessionsDown
        | KeyAction::ResourceDown
        | KeyAction::InspectDown
        | KeyAction::SettingsDown => KeyAction::CommonDown,
        KeyAction::TranscriptPageUp
        | KeyAction::HelpPageUp
        | KeyAction::SearchPageUp
        | KeyAction::InspectPageUp => KeyAction::CommonPageUp,
        KeyAction::TranscriptPageDown
        | KeyAction::HelpPageDown
        | KeyAction::SearchPageDown
        | KeyAction::InspectPageDown => KeyAction::CommonPageDown,
        KeyAction::TranscriptTop
        | KeyAction::HelpTop
        | KeyAction::SearchTop
        | KeyAction::InspectTop => KeyAction::CommonTop,
        KeyAction::TranscriptBottom | KeyAction::HelpBottom | KeyAction::SearchBottom => {
            KeyAction::CommonBottom
        }
        KeyAction::SendPanelLoad | KeyAction::SessionsOpen | KeyAction::SettingsApply => {
            KeyAction::CommonConfirm
        }
        KeyAction::HelpSearch
        | KeyAction::SessionsSearch
        | KeyAction::ResourceSearch
        | KeyAction::SettingsSearch => KeyAction::CommonSearch,
        KeyAction::SessionsRefresh | KeyAction::ResourceRefresh => KeyAction::CommonRefresh,
        KeyAction::SearchBackspace => KeyAction::CommonBackspace,
        KeyAction::SettingsNext => KeyAction::CommonNext,
        KeyAction::SettingsPrevious => KeyAction::CommonPrevious,
        _ => action,
    }
}

fn effective_contexts(action: KeyAction) -> Vec<KeyContext> {
    ACTION_INFOS
        .iter()
        .filter_map(|info| (canonical_action(info.action) == action).then_some(info.context))
        .collect()
}

fn default_bindings(
    action: KeyAction,
    preset: KeymapPreset,
    chord_key: KeyStroke,
) -> Vec<KeySequence> {
    let values: &[&str] = match (preset, action) {
        (_, KeyAction::CommonClose) => &["esc"],
        (_, KeyAction::CommonBack) => &["left", "backspace"],
        (_, KeyAction::CommonUp) => &["up", "k", "K"],
        (_, KeyAction::CommonDown) => &["down", "j", "J"],
        (_, KeyAction::CommonPageUp) => &["pageup"],
        (_, KeyAction::CommonPageDown) => &["pagedown"],
        (_, KeyAction::CommonTop) => &["home", "ctrl-home"],
        (_, KeyAction::CommonBottom) => &["end", "ctrl-end"],
        (_, KeyAction::CommonConfirm) => &["enter"],
        (_, KeyAction::CommonSearch) => &["/"],
        (_, KeyAction::CommonRefresh) => &["r", "R"],
        (_, KeyAction::CommonBackspace) => &["backspace"],
        (_, KeyAction::CommonNext) => &["tab"],
        (_, KeyAction::CommonPrevious) => &["shift-tab"],
        (_, KeyAction::AppQuit) => &["ctrl-c"],
        (KeymapPreset::Chord, KeyAction::HelpOpen) => return chord_defaults(chord_key, &["h"]),
        (KeymapPreset::Combination, KeyAction::HelpOpen) => &["f1"],
        (KeymapPreset::Chord, KeyAction::SettingsOpen) => return chord_defaults(chord_key, &["s"]),
        (KeymapPreset::Combination, KeyAction::SettingsOpen) => &["f2"],
        (KeymapPreset::Chord, KeyAction::SendPanelOpen) => {
            return chord_defaults(chord_key, &["q"])
        }
        (KeymapPreset::Combination, KeyAction::SendPanelOpen) => &["f4"],
        (KeymapPreset::Chord, KeyAction::TranscriptFocus) => {
            return chord_defaults(chord_key, &["t"])
        }
        (KeymapPreset::Combination, KeyAction::TranscriptFocus) => &["f3"],
        (KeymapPreset::Chord, KeyAction::ComposerExpandReference) => {
            return chord_defaults(chord_key, &["e"])
        }
        (KeymapPreset::Combination, KeyAction::ComposerExpandReference) => &["f5"],
        (_, KeyAction::ComposerSubmit) => &["enter"],
        (_, KeyAction::ComposerNewline) => &["ctrl-j", "alt-enter", "shift-enter"],
        (_, KeyAction::ComposerQueuePrompt) => return chord_defaults(chord_key, &["enter"]),
        (_, KeyAction::ComposerDraftPrompt) => return chord_defaults(chord_key, &["/"]),
        (_, KeyAction::SuggestionsClose) => &["esc"],
        (_, KeyAction::SuggestionsUp) => &["up"],
        (_, KeyAction::SuggestionsDown) => &["down"],
        (_, KeyAction::SuggestionsAccept) => &["tab"],
        (_, KeyAction::SuggestionsConfirm) => &["enter"],
        (_, KeyAction::TranscriptUnfocus) => &["esc"],
        (_, KeyAction::TranscriptPageUp) => &["pageup"],
        (_, KeyAction::TranscriptPageDown) => &["pagedown"],
        (_, KeyAction::TranscriptLineUp) => &["alt-up", "up", "k", "K"],
        (_, KeyAction::TranscriptLineDown) => &["alt-down", "down", "j", "J"],
        (_, KeyAction::TranscriptTop) => &["ctrl-home", "home", "g"],
        (_, KeyAction::TranscriptBottom) => &["ctrl-end", "end", "G"],
        (_, KeyAction::HelpClose) => &["esc", "q", "Q"],
        (_, KeyAction::HelpSearch) => &["/"],
        (_, KeyAction::HelpUp) => &["up", "k", "K"],
        (_, KeyAction::HelpDown) => &["down", "j", "J"],
        (_, KeyAction::HelpPageUp) => &["pageup"],
        (_, KeyAction::HelpPageDown) => &["pagedown"],
        (_, KeyAction::HelpTop) => &["home", "ctrl-home"],
        (_, KeyAction::HelpBottom) => &["end", "ctrl-end"],
        (_, KeyAction::SearchClose) => &["esc"],
        (_, KeyAction::SearchAccept) => &["enter", "tab"],
        (_, KeyAction::SearchBackspace) => &["backspace"],
        (_, KeyAction::SearchUp) => &["up"],
        (_, KeyAction::SearchDown) => &["down"],
        (_, KeyAction::SearchPageUp) => &["pageup"],
        (_, KeyAction::SearchPageDown) => &["pagedown"],
        (_, KeyAction::SearchTop) => &["home", "ctrl-home"],
        (_, KeyAction::SearchBottom) => &["end", "ctrl-end"],
        (_, KeyAction::SendPanelClose) => &["esc"],
        (_, KeyAction::SendPanelUp) => &["up", "k", "K"],
        (_, KeyAction::SendPanelDown) => &["down", "j", "J"],
        (_, KeyAction::SendPanelQueue) => &["q", "Q"],
        (_, KeyAction::SendPanelDraft) => &["d", "D"],
        (_, KeyAction::SendPanelDelete) => &["x", "X"],
        (_, KeyAction::SendPanelLoad) => &["enter"],
        (_, KeyAction::ConfirmYes) => &["y", "Y"],
        (_, KeyAction::ConfirmNo) => &["n", "N", "esc"],
        (_, KeyAction::SessionsClose) => &["esc"],
        (_, KeyAction::SessionsUp) => &["up", "k", "K"],
        (_, KeyAction::SessionsDown) => &["down", "j", "J"],
        (_, KeyAction::SessionsSearch) => &["/"],
        (_, KeyAction::SessionsRefresh) => &["r", "R"],
        (_, KeyAction::SessionsOpen) => &["enter"],
        (_, KeyAction::ResourceClose) => &["esc"],
        (_, KeyAction::ResourceUp) => &["up", "k", "K"],
        (_, KeyAction::ResourceDown) => &["down", "j", "J"],
        (_, KeyAction::ResourceSearch) => &["/"],
        (_, KeyAction::ResourceRefresh) => &["r", "R"],
        (_, KeyAction::ResourceComplete) => &["tab", "enter"],
        (_, KeyAction::InspectClose) => &["esc", "q", "Q"],
        (_, KeyAction::InspectUp) => &["up", "k", "K"],
        (_, KeyAction::InspectDown) => &["down", "j", "J"],
        (_, KeyAction::InspectPageUp) => &["pageup"],
        (_, KeyAction::InspectPageDown) => &["pagedown"],
        (_, KeyAction::InspectTop) => &["home", "ctrl-home"],
        (_, KeyAction::InspectExportHtml) => &["e", "E"],
        (_, KeyAction::SettingsClose) => &["esc"],
        (_, KeyAction::SettingsBack) => &["left", "backspace"],
        (_, KeyAction::SettingsOpenPage) => &["right"],
        (_, KeyAction::SettingsUp) => &["up", "k"],
        (_, KeyAction::SettingsDown) => &["down", "j"],
        (_, KeyAction::SettingsNext) => &["tab"],
        (_, KeyAction::SettingsPrevious) => &["shift-tab"],
        (_, KeyAction::SettingsApply) => &["enter"],
        (_, KeyAction::SettingsSearch) => &["/"],
        (_, KeyAction::SettingsToolToggleGlobal) => &["g", "G"],
        (_, KeyAction::SettingsToolToggleProject) => &["p", "P", "space", "enter", "right"],
        (_, KeyAction::KeymapEditChordKey) => &["g", "G"],
        (_, KeyAction::KeymapAddShortcut) => &["a", "A"],
        (_, KeyAction::KeymapRemoveShortcut) => &["x", "X"],
        (_, KeyAction::KeymapClearShortcuts) => &["c", "C"],
        (_, KeyAction::KeymapResetAction) => &["r", "R"],
        (_, KeyAction::KeymapSelectPreset) => &["p", "P"],
        (_, KeyAction::ExtensionSurfaceFocusNext) => return chord_defaults(chord_key, &["tab"]),
        (_, KeyAction::ExtensionSurfaceFocusPrevious) => {
            return chord_defaults(chord_key, &["shift-tab"])
        }
        (_, KeyAction::ExtensionSurfaceTabNext) => return chord_defaults(chord_key, &["]"]),
        (_, KeyAction::ExtensionSurfaceTabPrevious) => return chord_defaults(chord_key, &["["]),
        (_, KeyAction::ExtensionSurfaceClose) => return chord_defaults(chord_key, &["w"]),
        (_, KeyAction::ExtensionSidebarToggle) => return chord_defaults(chord_key, &["b"]),
        (_, KeyAction::ExtensionMainPanelToggle) => return chord_defaults(chord_key, &["m"]),
    };
    values
        .iter()
        .filter_map(|value| value.parse::<KeySequence>().ok())
        .collect()
}

#[must_use]
pub fn default_chord_key() -> KeyStroke {
    match "ctrl-o".parse::<KeyStroke>() {
        Ok(stroke) => stroke,
        Err(_) => unreachable!("default chord key parses"),
    }
}

fn chord_defaults(chord_key: KeyStroke, suffixes: &[&str]) -> Vec<KeySequence> {
    suffixes
        .iter()
        .filter_map(|suffix| suffix.parse::<KeyStroke>().ok())
        .map(|suffix| KeySequence::chord(chord_key, suffix))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_displays_chords() {
        let seq: KeySequence = match "ctrl-o s".parse() {
            Ok(seq) => seq,
            Err(err) => panic!("parse chord failed: {err}"),
        };
        assert_eq!(seq.to_string(), "Ctrl-O s");
        assert_eq!(seq.len(), 2);
    }

    #[test]
    fn resolves_default_chord_prefix_and_action() {
        let keymap = KeymapConfig::default();
        let ctrl_o =
            match KeyStroke::from_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)) {
                Some(stroke) => stroke,
                None => panic!("missing ctrl-o stroke"),
            };
        assert_eq!(
            keymap.resolve(&[KeyContext::Global], &[ctrl_o]),
            KeymapMatch::Pending
        );
        let s = match KeyStroke::from_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE)) {
            Some(stroke) => stroke,
            None => panic!("missing s stroke"),
        };
        assert_eq!(
            keymap.resolve(&[KeyContext::Global], &[ctrl_o, s]),
            KeymapMatch::Matched(KeyAction::SettingsOpen)
        );
    }
}
