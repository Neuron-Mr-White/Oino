#![forbid(unsafe_code)]

use crate::{
    action::TuiAction,
    ask_user::{AskUserOutcome, AskUserOverlayState, AskUserRequest},
    command::{
        command_suggestions_for, file_suggestions_for, parse_command, AgentMode,
        CommandSuggestionCategory, CommandSuggestionItem, CommandSuggestionsState,
        CommandSuggestionsView, ExtensionCommandSuggestion, ParsedCommand, SettingsCommand,
    },
    composer::{
        char_count, collapsed_paste_summary, normalize_paste_text, should_collapse_paste,
        ComposerState, MAX_PASTE_CHARS,
    },
    fuzzy::{ascii_subsequence_match_parts, fuzzy_indices, FuzzyMode},
    help::{help_entries, help_entry_match_text},
    keymap::{KeyAction, KeyContext, KeySequence, KeyStroke, KeymapConfig, KeymapMatch},
    message::{project_content_blocks, project_message, project_messages, MessageView},
    resource::{PromptResource, ResourceBrowserState, SkillResource},
    settings::{
        chat_style_label, collapse_mode_label, KeymapsMode, ModelOption, SettingsAction,
        SettingsState, ToolSettingsItem, ToolSettingsScope,
    },
    theme::{resolve_effective_theme, ResolvedTheme, ThemeCatalog, ThemeSettings},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use oino_extension_core::{
    detect_ui_surface_conflicts, validate_ui_surface_update, ActiveContribution, ContributionId,
    UiSurfaceAction, UiSurfaceConflict, UiSurfaceContribution, UiSurfaceKind, UiSurfaceStateUpdate,
    UiSurfaceValidationError,
};
use oino_types::{ContentBlock, Message, OinoId, ThinkingLevel};
use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

pub const HELP_STATUS: &str = "Type /help for shortcuts and commands";

const DEFAULT_TRANSCRIPT_PAGE_LINES: usize = 10;
const TRANSCRIPT_SCROLL_LINE_STEP: usize = 1;
const QUIT_ARM_WINDOW: Duration = Duration::from_secs(2);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Help,
    Settings,
    SendPanel,
    Sessions,
    Prompts,
    Skills,
    Extensions,
    Inspect,
    Usage,
    AskUser,
    Btw,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SendPanelSection {
    Steer,
    Queue,
    Draft,
}

impl SendPanelSection {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Steer => "Steer",
            Self::Queue => "Queue",
            Self::Draft => "Draft",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SendPanelItem {
    pub section: SendPanelSection,
    pub index: usize,
    pub text: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SendPanelState {
    pub cursor: usize,
    pub confirm_delete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionListItem {
    pub id: String,
    pub name: String,
    pub cwd: String,
    pub message_count: usize,
    pub preview: String,
    pub current: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SessionsState {
    pub cursor: usize,
    pub loading: bool,
    pub items: Vec<SessionListItem>,
    pub filtered_indices: Vec<usize>,
    pub search: String,
    pub search_active: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InspectState {
    pub full_prompt: String,
    pub token_count: usize,
    pub loading: bool,
    pub scroll: usize,
    pub export_message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TuiFocus {
    Transcript,
    Composer,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ChordState {
    #[default]
    None,
    CtrlO,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionManagementTarget {
    Extension,
    Package,
    Contribution,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExtensionManagementView {
    Manage,
    Registry,
}

impl ExtensionManagementView {
    pub const ALL: [Self; 2] = [Self::Manage, Self::Registry];

    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Manage => "Manage",
            Self::Registry => "Registered",
        }
    }

    #[must_use]
    pub const fn accepts(self, target: ExtensionManagementTarget) -> bool {
        match self {
            Self::Manage => matches!(target, ExtensionManagementTarget::Package),
            Self::Registry => matches!(
                target,
                ExtensionManagementTarget::Extension | ExtensionManagementTarget::Contribution
            ),
        }
    }
}

impl ExtensionManagementTarget {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Extension => "extension",
            Self::Package => "package",
            Self::Contribution => "contribution",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManagementItem {
    pub target: ExtensionManagementTarget,
    pub id: String,
    pub title: String,
    pub family: String,
    pub scope: String,
    pub health: String,
    pub state: String,
    pub permission: String,
    pub provenance: String,
    pub diagnostics: Vec<String>,
    pub conflicts: Vec<String>,
    pub entry_key: Option<String>,
    pub canonical_id: Option<String>,
    pub global_override: bool,
    pub project_override: bool,
    pub global_enabled: bool,
    pub project_enabled: bool,
}

impl ExtensionManagementItem {
    #[must_use]
    pub fn haystack(&self) -> String {
        format!(
            "{} {} {} {} {} {} {} {} {}",
            self.target.label(),
            self.id,
            self.title,
            self.family,
            self.scope,
            self.health,
            self.state,
            self.permission,
            self.provenance
        )
    }

    #[must_use]
    pub fn enabled(&self, scope: ToolSettingsScope) -> bool {
        match scope {
            ToolSettingsScope::Global => self.global_enabled,
            ToolSettingsScope::Project => self.project_enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionPackageSelection {
    pub package_id: String,
    pub scope: ToolSettingsScope,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionManagementState {
    pub items: Vec<ExtensionManagementItem>,
    pub filtered_indices: Vec<usize>,
    pub cursor: usize,
    pub search: String,
    pub search_active: bool,
    pub install_input: String,
    pub install_scope: ToolSettingsScope,
    pub install_active: bool,
    pub remove_confirm: Option<ExtensionPackageSelection>,
    pub view: ExtensionManagementView,
}

impl Default for ExtensionManagementState {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            filtered_indices: Vec::new(),
            cursor: 0,
            search: String::new(),
            search_active: false,
            install_input: String::new(),
            install_scope: ToolSettingsScope::Project,
            install_active: false,
            remove_confirm: None,
            view: ExtensionManagementView::Manage,
        }
    }
}

impl ExtensionManagementState {
    pub fn set_items(&mut self, items: Vec<ExtensionManagementItem>) {
        self.items = items;
        self.cursor = self.cursor.min(self.items.len().saturating_sub(1));
        self.refresh_filter();
    }

    pub fn refresh_filter(&mut self) {
        let candidates = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, item)| self.view.accepts(item.target))
            .map(|(index, item)| (index, item.haystack()))
            .collect::<Vec<_>>();
        self.filtered_indices = if self.search.trim().is_empty() {
            candidates.iter().map(|(index, _)| *index).collect()
        } else {
            fuzzy_indices(
                &candidates,
                &self.search,
                FuzzyMode::Text,
                None,
                |(_, haystack)| haystack.clone(),
            )
            .into_iter()
            .filter_map(|candidate_index| candidates.get(candidate_index).map(|(index, _)| *index))
            .collect()
        };
        if self.filtered_indices.is_empty() {
            self.cursor = 0;
        } else {
            self.cursor = self
                .cursor
                .min(self.filtered_indices.len().saturating_sub(1));
        }
    }

    pub fn set_view(&mut self, view: ExtensionManagementView) {
        if self.view == view {
            return;
        }
        self.view = view;
        self.cursor = 0;
        self.remove_confirm = None;
        self.refresh_filter();
    }

    pub fn cycle_view(&mut self, delta: isize) {
        let current = ExtensionManagementView::ALL
            .iter()
            .position(|view| *view == self.view)
            .unwrap_or_default();
        let next = (current as isize + delta)
            .rem_euclid(ExtensionManagementView::ALL.len() as isize) as usize;
        self.set_view(ExtensionManagementView::ALL[next]);
    }

    #[must_use]
    pub fn count_for_view(&self, view: ExtensionManagementView) -> usize {
        self.items
            .iter()
            .filter(|item| view.accepts(item.target))
            .count()
    }

    pub fn move_cursor(&mut self, delta: isize) {
        self.cursor = move_index(self.cursor, self.filtered_indices.len(), delta);
    }

    #[must_use]
    pub fn selected_item(&self) -> Option<&ExtensionManagementItem> {
        self.filtered_indices
            .get(self.cursor)
            .and_then(|index| self.items.get(*index))
    }

    pub fn begin_install(&mut self, scope: ToolSettingsScope) {
        self.set_view(ExtensionManagementView::Manage);
        self.install_scope = scope;
        self.install_active = true;
        self.install_input.clear();
        self.search_active = false;
        self.remove_confirm = None;
    }

    pub fn cancel_install(&mut self) {
        self.install_active = false;
        self.install_input.clear();
    }

    pub fn package_selection_for_selected(&self) -> Option<ExtensionPackageSelection> {
        let item = self.selected_item()?;
        if item.target != ExtensionManagementTarget::Package {
            return None;
        }
        let scope = match item.scope.as_str() {
            "global" => ToolSettingsScope::Global,
            "project" => ToolSettingsScope::Project,
            _ => ToolSettingsScope::Project,
        };
        Some(ExtensionPackageSelection {
            package_id: item.id.clone(),
            scope,
        })
    }

    pub fn override_selection_for_selected(&self) -> Option<(String, String)> {
        let item = self.selected_item()?;
        if item.target != ExtensionManagementTarget::Contribution {
            return None;
        }
        let contribution_id = item.canonical_id.clone().unwrap_or_else(|| item.id.clone());
        let entry_key = item.entry_key.clone()?;
        Some((contribution_id, entry_key))
    }

    pub fn set_selected_enabled(&mut self, scope: ToolSettingsScope, enabled: bool) {
        let Some(index) = self.filtered_indices.get(self.cursor).copied() else {
            return;
        };
        let Some(item) = self.items.get_mut(index) else {
            return;
        };
        match scope {
            ToolSettingsScope::Global => item.global_enabled = enabled,
            ToolSettingsScope::Project => item.project_enabled = enabled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionShortcut {
    pub action: String,
    pub sequence: KeySequence,
    pub source: String,
    pub conflicts: BTreeSet<String>,
}

impl ExtensionShortcut {
    #[must_use]
    pub fn new(
        action: impl Into<String>,
        sequence: KeySequence,
        source: impl Into<String>,
    ) -> Self {
        Self {
            action: action.into(),
            sequence,
            source: source.into(),
            conflicts: BTreeSet::new(),
        }
    }

    #[must_use]
    pub fn is_active(&self) -> bool {
        self.conflicts.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExtensionAutosuggestItem {
    pub label: String,
    pub summary: String,
    pub replacement: String,
    pub trigger: String,
    pub source: String,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionThemeState {
    pub label: Option<String>,
    pub tokens: BTreeMap<String, String>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionSurfaceControllerState {
    pub hidden_slots: BTreeSet<String>,
    pub active_tabs: BTreeMap<String, usize>,
    pub focused_slot: Option<String>,
}

impl ExtensionSurfaceControllerState {
    #[must_use]
    pub fn is_slot_hidden(&self, slot: &str) -> bool {
        self.hidden_slots.contains(slot)
    }

    pub fn set_slot_hidden(&mut self, slot: impl Into<String>, hidden: bool) {
        let slot = slot.into();
        if hidden {
            self.hidden_slots.insert(slot);
        } else {
            self.hidden_slots.remove(&slot);
        }
    }

    #[must_use]
    pub fn active_tab(&self, slot: &str, len: usize) -> usize {
        if len == 0 {
            0
        } else {
            self.active_tabs
                .get(slot)
                .copied()
                .unwrap_or_default()
                .min(len.saturating_sub(1))
        }
    }

    pub fn set_active_tab(&mut self, slot: impl Into<String>, index: usize) {
        self.active_tabs.insert(slot.into(), index);
    }

    pub fn prune(&mut self, slots: &BTreeSet<String>) {
        self.hidden_slots.retain(|slot| slots.contains(slot));
        self.active_tabs.retain(|slot, _| slots.contains(slot));
        if self
            .focused_slot
            .as_ref()
            .is_some_and(|slot| !slots.contains(slot))
        {
            self.focused_slot = None;
        }
    }
}

#[must_use]
pub fn extension_surface_slot_key(surface: &UiSurfaceContribution) -> String {
    let slot = if surface.layout.slot == "primary" {
        surface.surface.default_slot()
    } else {
        surface.layout.slot.as_str()
    };
    format!("{:?}:{slot}", surface.surface)
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ExtensionUiState {
    pub surfaces: Vec<ActiveContribution<UiSurfaceContribution>>,
    pub state_summaries: BTreeMap<ContributionId, String>,
    pub actions: BTreeMap<ContributionId, Vec<UiSurfaceAction>>,
    pub focused_surface: Option<ContributionId>,
    pub surface_controller: ExtensionSurfaceControllerState,
    pub conflicts: Vec<UiSurfaceConflict>,
    pub shortcuts: Vec<ExtensionShortcut>,
    pub autosuggest_items: Vec<ExtensionAutosuggestItem>,
    pub theme: ExtensionThemeState,
}

impl ExtensionUiState {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.surfaces.is_empty()
    }

    pub fn set_surfaces(&mut self, mut surfaces: Vec<ActiveContribution<UiSurfaceContribution>>) {
        surfaces.sort_by(|left, right| {
            right
                .entry
                .contribution
                .layout
                .priority
                .cmp(&left.entry.contribution.layout.priority)
                .then_with(|| left.effective_id.cmp(&right.effective_id))
        });
        self.conflicts = detect_ui_surface_conflicts(&surfaces);
        let slots = surfaces
            .iter()
            .map(|surface| extension_surface_slot_key(&surface.entry.contribution))
            .collect::<BTreeSet<_>>();
        self.surface_controller.prune(&slots);
        self.state_summaries
            .retain(|id, _| surfaces.iter().any(|surface| &surface.effective_id == id));
        self.actions
            .retain(|id, _| surfaces.iter().any(|surface| &surface.effective_id == id));
        if self
            .focused_surface
            .as_ref()
            .is_some_and(|id| !surfaces.iter().any(|surface| &surface.effective_id == id))
        {
            self.focused_surface = None;
        }
        self.surfaces = surfaces;
    }

    pub fn set_shortcuts(&mut self, shortcuts: Vec<ExtensionShortcut>, keymap: &KeymapConfig) {
        self.shortcuts = mark_extension_shortcut_conflicts(shortcuts, keymap);
    }

    pub fn set_autosuggest_items(&mut self, mut items: Vec<ExtensionAutosuggestItem>) {
        items.sort_by(|left, right| {
            left.trigger
                .cmp(&right.trigger)
                .then(left.label.cmp(&right.label))
                .then(left.source.cmp(&right.source))
        });
        self.autosuggest_items = items;
    }

    pub fn set_theme(&mut self, theme: ExtensionThemeState) {
        self.theme = theme;
    }

    pub fn apply_update(
        &mut self,
        update: UiSurfaceStateUpdate,
    ) -> Result<(), UiSurfaceValidationError> {
        let surface = self
            .surfaces
            .iter()
            .find(|surface| surface.effective_id == update.surface_id)
            .ok_or_else(|| UiSurfaceValidationError::UnknownSurface {
                surface_id: update.surface_id.clone(),
            })?;
        validate_ui_surface_update(surface, &update)?;
        self.state_summaries.insert(
            update.surface_id.clone(),
            summarize_extension_ui_state(&update.state),
        );
        self.actions.insert(update.surface_id, update.actions);
        Ok(())
    }

    pub fn focus_surface(&mut self, surface_id: &ContributionId) -> bool {
        if self
            .surfaces
            .iter()
            .any(|surface| &surface.effective_id == surface_id)
        {
            self.focused_surface = Some(surface_id.clone());
            return true;
        }
        false
    }

    #[must_use]
    pub fn action_for_scope(&self, scope: &str) -> Option<TuiAction> {
        let focused = self.focused_surface.as_ref()?;
        let actions = self.actions.get(focused)?;
        let action = actions
            .iter()
            .find(|action| action.key_scope.as_deref() == Some(scope))?;
        Some(TuiAction::RunExtensionUiAction {
            surface_id: focused.as_str().to_string(),
            action_id: action.id.clone(),
        })
    }

    #[must_use]
    pub fn surface_slot_keys(&self) -> Vec<String> {
        self.surfaces
            .iter()
            .map(|surface| extension_surface_slot_key(&surface.entry.contribution))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn visible_surface_slot_keys(&self) -> Vec<String> {
        self.surface_slot_keys()
            .into_iter()
            .filter(|slot| !self.surface_controller.is_slot_hidden(slot))
            .collect()
    }

    #[must_use]
    pub fn surface_slot_keys_for_kind(&self, kind: UiSurfaceKind) -> Vec<String> {
        self.surfaces
            .iter()
            .filter(|surface| surface.entry.contribution.surface == kind)
            .map(|surface| extension_surface_slot_key(&surface.entry.contribution))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect()
    }

    #[must_use]
    pub fn active_surface_for_slot(
        &self,
        slot: &str,
    ) -> Option<&ActiveContribution<UiSurfaceContribution>> {
        let surfaces = self
            .surfaces
            .iter()
            .filter(|surface| extension_surface_slot_key(&surface.entry.contribution) == slot)
            .collect::<Vec<_>>();
        let index = self.surface_controller.active_tab(slot, surfaces.len());
        surfaces.get(index).copied()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ExtensionShortcutMatch {
    None,
    Pending,
    Matched(String),
}

fn mark_extension_shortcut_conflicts(
    mut shortcuts: Vec<ExtensionShortcut>,
    keymap: &KeymapConfig,
) -> Vec<ExtensionShortcut> {
    let all_contexts = [
        KeyContext::Common,
        KeyContext::Global,
        KeyContext::Composer,
        KeyContext::CommandSuggestions,
        KeyContext::Transcript,
        KeyContext::Help,
        KeyContext::HelpSearch,
        KeyContext::SendPanel,
        KeyContext::SendPanelConfirm,
        KeyContext::Sessions,
        KeyContext::Search,
        KeyContext::ResourceBrowser,
        KeyContext::Inspect,
        KeyContext::Settings,
        KeyContext::SettingsTools,
        KeyContext::SettingsKeymaps,
        KeyContext::SettingsKeymapDetail,
        KeyContext::SettingsKeymapType,
        KeyContext::SettingsKeymapPreset,
        KeyContext::SettingsKeymapPresetConfirm,
    ];
    for shortcut in &mut shortcuts {
        match keymap.resolve(&all_contexts, shortcut.sequence.strokes()) {
            KeymapMatch::Matched(action) => {
                shortcut
                    .conflicts
                    .insert(format!("built-in:{}", action.id()));
            }
            KeymapMatch::Pending => {
                shortcut.conflicts.insert("built-in:prefix".into());
            }
            KeymapMatch::None => {}
        }
    }
    let len = shortcuts.len();
    for left in 0..len {
        for right in (left + 1)..len {
            let left_sequence = shortcuts[left].sequence.clone();
            let right_sequence = shortcuts[right].sequence.clone();
            if left_sequence == right_sequence
                || left_sequence.starts_with(right_sequence.strokes())
                || right_sequence.starts_with(left_sequence.strokes())
            {
                let left_action = shortcuts[left].action.clone();
                let right_action = shortcuts[right].action.clone();
                shortcuts[left]
                    .conflicts
                    .insert(format!("extension:{right_action}"));
                shortcuts[right]
                    .conflicts
                    .insert(format!("extension:{left_action}"));
            }
        }
    }
    shortcuts.sort_by(|left, right| {
        left.sequence
            .cmp(&right.sequence)
            .then(left.action.cmp(&right.action))
            .then(left.source.cmp(&right.source))
    });
    shortcuts
}

fn resolve_extension_shortcut(
    shortcuts: &[ExtensionShortcut],
    strokes: &[KeyStroke],
) -> ExtensionShortcutMatch {
    if strokes.is_empty() {
        return ExtensionShortcutMatch::None;
    }
    let mut pending = false;
    for shortcut in shortcuts.iter().filter(|shortcut| shortcut.is_active()) {
        if shortcut.sequence.strokes() == strokes {
            return ExtensionShortcutMatch::Matched(shortcut.action.clone());
        }
        if shortcut.sequence.len() > strokes.len() && shortcut.sequence.starts_with(strokes) {
            pending = true;
        }
    }
    if pending {
        ExtensionShortcutMatch::Pending
    } else {
        ExtensionShortcutMatch::None
    }
}

fn summarize_extension_ui_state(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(text) => text.clone(),
        serde_json::Value::Array(items) => format!("{} item{}", items.len(), plural(items.len())),
        serde_json::Value::Object(map) => {
            if let Some(summary) = map.get("summary").and_then(serde_json::Value::as_str) {
                return summary.to_string();
            }
            if let Some(title) = map.get("title").and_then(serde_json::Value::as_str) {
                return title.to_string();
            }
            if let Some(rows) = map.get("rows").and_then(serde_json::Value::as_array) {
                return format!("{} row{}", rows.len(), plural(rows.len()));
            }
            format!("{} field{}", map.len(), plural(map.len()))
        }
        serde_json::Value::Bool(value) => value.to_string(),
        serde_json::Value::Number(value) => value.to_string(),
    }
}

fn plural(count: usize) -> &'static str {
    if count == 1 {
        ""
    } else {
        "s"
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum KeymapKeyResult {
    Unhandled,
    Consumed,
    Matched(KeyAction),
    MatchedExtension(String),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranscriptScroll {
    offset_from_bottom: usize,
}

impl TranscriptScroll {
    #[must_use]
    pub const fn is_at_bottom(self) -> bool {
        self.offset_from_bottom == 0
    }

    #[must_use]
    pub const fn offset_from_bottom(self) -> usize {
        self.offset_from_bottom
    }

    #[must_use]
    pub fn visible_start(self, total_lines: usize, visible_lines: usize) -> usize {
        if total_lines <= visible_lines {
            return 0;
        }
        let max_start = total_lines.saturating_sub(visible_lines);
        max_start.saturating_sub(self.offset_from_bottom.min(max_start))
    }

    #[must_use]
    pub fn resolved_offset_from_bottom(self, total_lines: usize, visible_lines: usize) -> usize {
        if total_lines <= visible_lines {
            return 0;
        }
        let max_start = total_lines.saturating_sub(visible_lines);
        self.offset_from_bottom.min(max_start)
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_add(lines.max(1));
    }

    pub fn scroll_down(&mut self, lines: usize) {
        self.offset_from_bottom = self.offset_from_bottom.saturating_sub(lines.max(1));
    }

    pub fn jump_top(&mut self) {
        self.offset_from_bottom = usize::MAX;
    }

    pub fn jump_bottom(&mut self) {
        self.offset_from_bottom = 0;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeStatusState {
    pub working_directory: String,
    pub context_tokens: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsagePanelState {
    pub loading: bool,
    pub error: Option<String>,
    pub report: Option<UsagePanelReport>,
    pub cursor: usize,
    pub search: String,
    pub search_active: bool,
    pub filtered_indices: Vec<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsagePanelReport {
    pub generated_at_unix: u64,
    pub status_line: String,
    pub session: UsagePanelSession,
    pub providers: Vec<UsagePanelProvider>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsagePanelSession {
    pub assistant_turns: u64,
    pub reported_turns: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub total_tokens: u64,
    pub costs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsagePanelProvider {
    pub provider_id: String,
    pub display_name: String,
    pub status: String,
    pub message: String,
    pub assistant_turns: u64,
    pub reported_turns: u64,
    pub total_tokens: u64,
    pub costs: Vec<String>,
    pub account_source: Option<String>,
    pub account_balance: Option<String>,
    pub account_limits: Vec<String>,
}

impl UsagePanelState {
    pub fn set_loading(&mut self) {
        self.loading = true;
        self.error = None;
    }

    pub fn set_report(&mut self, report: UsagePanelReport) {
        self.loading = false;
        self.error = None;
        self.report = Some(report);
        self.refresh_filter();
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.loading = false;
        self.error = Some(error.into());
    }

    pub fn refresh_filter(&mut self) {
        let Some(report) = &self.report else {
            self.filtered_indices.clear();
            self.cursor = 0;
            return;
        };
        let query = self.search.trim().to_ascii_lowercase();
        self.filtered_indices = report
            .providers
            .iter()
            .enumerate()
            .filter_map(|(index, provider)| {
                (query.is_empty() || usage_provider_matches(provider, &query)).then_some(index)
            })
            .collect();
        if let Some(cursor) = self
            .filtered_indices
            .iter()
            .position(|index| *index == self.cursor)
        {
            self.cursor = self.filtered_indices[cursor];
        } else {
            self.cursor = self.filtered_indices.first().copied().unwrap_or(0);
        }
    }

    pub fn move_cursor(&mut self, delta: isize) {
        if self.filtered_indices.is_empty() {
            self.cursor = 0;
            return;
        }
        let position = self
            .filtered_indices
            .iter()
            .position(|index| *index == self.cursor)
            .unwrap_or(0);
        let len = self.filtered_indices.len();
        let next = if delta.is_negative() {
            position.saturating_sub(delta.unsigned_abs())
        } else {
            position
                .saturating_add(delta as usize)
                .min(len.saturating_sub(1))
        };
        self.cursor = self.filtered_indices[next];
    }

    #[must_use]
    pub fn selected_provider(&self) -> Option<&UsagePanelProvider> {
        self.report
            .as_ref()
            .and_then(|report| report.providers.get(self.cursor))
    }

    #[must_use]
    pub fn filtered_provider_indices(&self) -> &[usize] {
        &self.filtered_indices
    }
}

fn usage_provider_matches(provider: &UsagePanelProvider, query: &str) -> bool {
    provider.provider_id.to_ascii_lowercase().contains(query)
        || provider.display_name.to_ascii_lowercase().contains(query)
        || provider.status.to_ascii_lowercase().contains(query)
        || provider.message.to_ascii_lowercase().contains(query)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtwState {
    pub input: String,
    pub messages: Vec<MessageView>,
    pub working: bool,
    pub error: Option<String>,
    pub configured_model: Option<String>,
    pub effective_model: String,
    pub inherited: bool,
}

impl Default for BtwState {
    fn default() -> Self {
        Self {
            input: String::new(),
            messages: Vec::new(),
            working: false,
            error: None,
            configured_model: None,
            effective_model: String::new(),
            inherited: true,
        }
    }
}

pub struct TuiState {
    pub messages: Vec<MessageView>,
    pub session_title: String,
    pub composer: ComposerState,
    pub focus: TuiFocus,
    pub status: String,
    pub working: bool,
    pub error: Option<String>,
    pub overlay: Option<OverlayKind>,
    pub settings: SettingsState,
    pub agent_mode: AgentMode,
    pub theme_catalog: ThemeCatalog,
    pub resolved_theme: ResolvedTheme,
    pub preview_theme: Option<ResolvedTheme>,
    pub command_suggestions: CommandSuggestionsState,
    pub chord: ChordState,
    key_sequence: Vec<KeyStroke>,
    pub transcript_scroll: TranscriptScroll,
    pub send_panel: SendPanelState,
    pub sessions: SessionsState,
    pub inspect: InspectState,
    pub ask_user: Option<AskUserOverlayState>,
    pub btw: BtwState,
    pub prompts: ResourceBrowserState,
    pub skills: ResourceBrowserState,
    pub prompt_resources: Vec<PromptResource>,
    pub skill_resources: Vec<SkillResource>,
    pub resource_diagnostics: Vec<String>,
    pub extension_commands: Vec<ExtensionCommandSuggestion>,
    pub extension_ui: ExtensionUiState,
    pub extension_management: ExtensionManagementState,
    pub runtime_status: RuntimeStatusState,
    pub usage: UsagePanelState,
    pub help_scroll: usize,
    pub help_search: String,
    pub help_search_active: bool,
    pub filtered_help_indices: Vec<usize>,
    pub steer_items: Vec<String>,
    pub queued_items: Vec<String>,
    pub draft_items: Vec<String>,
    transcript_page_lines: usize,
    transcript_version: u64,
    quit_pending: bool,
    quit_armed_at: Option<Instant>,
    file_paths: Vec<String>,
}

impl Default for TuiState {
    fn default() -> Self {
        let theme_catalog = ThemeCatalog::builtins();
        let global_theme = ThemeSettings::default();
        let project_theme = ThemeSettings::default();
        let resolved_theme = resolve_effective_theme(&theme_catalog, &global_theme, &project_theme);
        let mut settings = SettingsState::new("", ThinkingLevel::Off);
        settings.set_theme_state(
            &theme_catalog,
            &global_theme,
            &project_theme,
            &resolved_theme,
        );
        Self {
            messages: Vec::new(),
            session_title: String::new(),
            composer: ComposerState::new(),
            focus: TuiFocus::Composer,
            status: HELP_STATUS.into(),
            working: false,
            error: None,
            overlay: None,
            settings,
            agent_mode: AgentMode::default(),
            theme_catalog,
            resolved_theme,
            preview_theme: None,
            command_suggestions: CommandSuggestionsState::new(),
            chord: ChordState::None,
            key_sequence: Vec::new(),
            transcript_scroll: TranscriptScroll::default(),
            send_panel: SendPanelState::default(),
            sessions: SessionsState::default(),
            inspect: InspectState::default(),
            ask_user: None,
            btw: BtwState::default(),
            prompts: ResourceBrowserState::default(),
            skills: ResourceBrowserState::default(),
            prompt_resources: Vec::new(),
            skill_resources: Vec::new(),
            resource_diagnostics: Vec::new(),
            extension_commands: Vec::new(),
            extension_ui: ExtensionUiState::default(),
            extension_management: ExtensionManagementState::default(),
            runtime_status: RuntimeStatusState::default(),
            usage: UsagePanelState::default(),
            help_scroll: 0,
            help_search: String::new(),
            help_search_active: false,
            filtered_help_indices: (0..help_entries(&KeymapConfig::default()).len()).collect(),
            steer_items: Vec::new(),
            queued_items: Vec::new(),
            draft_items: Vec::new(),
            transcript_page_lines: DEFAULT_TRANSCRIPT_PAGE_LINES,
            transcript_version: 0,
            quit_pending: false,
            quit_armed_at: None,
            file_paths: Vec::new(),
        }
    }
}

impl TuiState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_settings(model: impl Into<String>, thinking_level: ThinkingLevel) -> Self {
        let theme_catalog = ThemeCatalog::builtins();
        let global_theme = ThemeSettings::default();
        let project_theme = ThemeSettings::default();
        let resolved_theme = resolve_effective_theme(&theme_catalog, &global_theme, &project_theme);
        let mut settings = SettingsState::new(model, thinking_level);
        settings.set_theme_state(
            &theme_catalog,
            &global_theme,
            &project_theme,
            &resolved_theme,
        );
        Self {
            settings,
            theme_catalog,
            resolved_theme,
            preview_theme: None,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn input(&self) -> &str {
        self.composer.text()
    }

    #[must_use]
    pub fn cursor_position(&self) -> usize {
        self.composer.cursor()
    }

    #[must_use]
    pub fn has_queued_prompts(&self) -> bool {
        !self.queued_items.is_empty()
    }

    #[must_use]
    pub fn next_queued_prompt(&self) -> Option<&str> {
        self.queued_items.first().map(String::as_str)
    }

    pub fn pop_next_queued_prompt(&mut self) -> Option<String> {
        if self.queued_items.is_empty() {
            None
        } else {
            Some(self.queued_items.remove(0))
        }
    }

    #[must_use]
    pub fn send_panel_items(&self) -> Vec<SendPanelItem> {
        let mut items = Vec::new();
        items.extend(
            self.steer_items
                .iter()
                .enumerate()
                .map(|(index, text)| SendPanelItem {
                    section: SendPanelSection::Steer,
                    index,
                    text: text.clone(),
                }),
        );
        items.extend(
            self.queued_items
                .iter()
                .enumerate()
                .map(|(index, text)| SendPanelItem {
                    section: SendPanelSection::Queue,
                    index,
                    text: text.clone(),
                }),
        );
        items.extend(
            self.draft_items
                .iter()
                .enumerate()
                .map(|(index, text)| SendPanelItem {
                    section: SendPanelSection::Draft,
                    index,
                    text: text.clone(),
                }),
        );
        items
    }

    #[must_use]
    pub fn selected_send_panel_item(&self) -> Option<SendPanelItem> {
        self.send_panel_items().get(self.send_panel.cursor).cloned()
    }

    #[must_use]
    pub fn selected_session_item(&self) -> Option<&SessionListItem> {
        self.sessions
            .filtered_indices
            .contains(&self.sessions.cursor)
            .then(|| self.sessions.items.get(self.sessions.cursor))
            .flatten()
    }

    #[must_use]
    pub fn filtered_session_indices(&self) -> &[usize] {
        &self.sessions.filtered_indices
    }

    #[must_use]
    pub fn filtered_help_indices(&self) -> &[usize] {
        &self.filtered_help_indices
    }

    #[must_use]
    pub fn has_session_content(&self) -> bool {
        !self.messages.is_empty()
            || !self.steer_items.is_empty()
            || !self.queued_items.is_empty()
            || !self.draft_items.is_empty()
    }

    #[must_use]
    pub fn session_cursor_filtered_position(&self) -> usize {
        self.sessions
            .filtered_indices
            .iter()
            .position(|index| *index == self.sessions.cursor)
            .unwrap_or(0)
    }

    pub fn set_sessions(&mut self, sessions: Vec<SessionListItem>) {
        self.sessions.items = sessions;
        self.sessions.loading = false;
        self.refresh_session_filter();
        self.status = if self.sessions.items.is_empty() {
            "No saved sessions yet".into()
        } else {
            format!("Loaded {} saved sessions", self.sessions.items.len())
        };
    }

    pub fn set_resources(
        &mut self,
        prompts: Vec<PromptResource>,
        skills: Vec<SkillResource>,
        diagnostics: Vec<String>,
    ) {
        self.prompt_resources = prompts;
        self.skill_resources = skills;
        self.resource_diagnostics = diagnostics;
        self.prompts.loading = false;
        self.skills.loading = false;
        self.refresh_prompt_filter();
        self.refresh_skill_filter();
        self.refresh_command_suggestions();
        self.status = format!(
            "Loaded {} prompts and {} skills",
            self.prompt_resources.len(),
            self.skill_resources.len()
        );
    }

    #[must_use]
    pub fn selected_prompt_item(&self) -> Option<&PromptResource> {
        self.prompts
            .filtered_indices
            .contains(&self.prompts.cursor)
            .then(|| self.prompt_resources.get(self.prompts.cursor))
            .flatten()
    }

    #[must_use]
    pub fn selected_skill_item(&self) -> Option<&SkillResource> {
        self.skills
            .filtered_indices
            .contains(&self.skills.cursor)
            .then(|| self.skill_resources.get(self.skills.cursor))
            .flatten()
    }

    #[must_use]
    pub fn filtered_prompt_indices(&self) -> &[usize] {
        &self.prompts.filtered_indices
    }

    #[must_use]
    pub fn filtered_skill_indices(&self) -> &[usize] {
        &self.skills.filtered_indices
    }

    #[must_use]
    pub fn prompt_cursor_filtered_position(&self) -> usize {
        self.prompts
            .filtered_indices
            .iter()
            .position(|index| *index == self.prompts.cursor)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn skill_cursor_filtered_position(&self) -> usize {
        self.skills
            .filtered_indices
            .iter()
            .position(|index| *index == self.skills.cursor)
            .unwrap_or(0)
    }

    #[must_use]
    pub fn activity_status(&self) -> Option<String> {
        if self.overlay.is_some() {
            return None;
        }
        self.working
            .then(|| self.status.clone())
            .filter(|status| !status.trim().is_empty())
    }

    pub fn notice_status(&self) -> Option<String> {
        if self.working || self.overlay.is_some() || self.error.is_some() {
            return None;
        }
        let status = self.status.trim();
        if status.is_empty() || status == HELP_STATUS {
            None
        } else {
            Some(self.status.clone())
        }
    }

    pub fn set_transcript_page_lines(&mut self, lines: usize) {
        self.transcript_page_lines = lines.max(1);
    }

    pub fn set_file_paths(&mut self, paths: Vec<String>) {
        self.file_paths = paths;
        self.refresh_command_suggestions();
    }

    pub fn scroll_transcript_up(&mut self, lines: usize) {
        self.transcript_scroll.scroll_up(lines);
        self.status = self.transcript_scroll_status();
    }

    pub fn scroll_transcript_down(&mut self, lines: usize) {
        self.transcript_scroll.scroll_down(lines);
        self.status = self.transcript_scroll_status();
    }

    pub fn scroll_transcript_to_top(&mut self) {
        self.transcript_scroll.jump_top();
        self.status = self.transcript_scroll_status();
    }

    pub fn scroll_transcript_to_bottom(&mut self) {
        self.transcript_scroll.jump_bottom();
        self.status = HELP_STATUS.into();
    }

    fn clamp_send_panel_cursor(&mut self) {
        let len = self.send_panel_items().len();
        if len == 0 {
            self.send_panel.cursor = 0;
        } else {
            self.send_panel.cursor = self.send_panel.cursor.min(len.saturating_sub(1));
        }
    }

    fn move_send_panel_cursor(&mut self, delta: isize) {
        let len = self.send_panel_items().len();
        if len == 0 {
            self.send_panel.cursor = 0;
            return;
        }
        let max = len.saturating_sub(1) as isize;
        self.send_panel.cursor = (self.send_panel.cursor as isize + delta).clamp(0, max) as usize;
    }

    fn clamp_sessions_cursor(&mut self) {
        if self.sessions.items.is_empty() {
            self.sessions.cursor = 0;
        } else {
            self.sessions.cursor = self
                .sessions
                .cursor
                .min(self.sessions.items.len().saturating_sub(1));
        }
    }

    fn move_sessions_cursor(&mut self, delta: isize) {
        let indices = &self.sessions.filtered_indices;
        if indices.is_empty() {
            self.sessions.cursor = 0;
            return;
        }
        let current = indices
            .iter()
            .position(|index| *index == self.sessions.cursor)
            .unwrap_or(0);
        let next = move_index(current, indices.len(), delta);
        self.sessions.cursor = indices[next];
    }

    fn refresh_session_filter(&mut self) {
        self.sessions.filtered_indices =
            filtered_session_indices(&self.sessions.items, self.sessions.search.trim());
        self.sync_session_cursor_to_filter();
    }

    fn sync_session_cursor_to_filter(&mut self) {
        self.clamp_sessions_cursor();
        let indices = &self.sessions.filtered_indices;
        if let Some(first) = indices.first().copied() {
            if !indices.contains(&self.sessions.cursor) {
                self.sessions.cursor = first;
            }
        }
    }

    fn clamp_prompt_cursor(&mut self) {
        if self.prompt_resources.is_empty() {
            self.prompts.cursor = 0;
        } else {
            self.prompts.cursor = self
                .prompts
                .cursor
                .min(self.prompt_resources.len().saturating_sub(1));
        }
    }

    fn move_prompt_cursor(&mut self, delta: isize) {
        let indices = &self.prompts.filtered_indices;
        if indices.is_empty() {
            self.prompts.cursor = 0;
            return;
        }
        let current = indices
            .iter()
            .position(|index| *index == self.prompts.cursor)
            .unwrap_or(0);
        let next = move_index(current, indices.len(), delta);
        self.prompts.cursor = indices[next];
    }

    fn refresh_prompt_filter(&mut self) {
        self.prompts.filtered_indices =
            filtered_prompt_indices(&self.prompt_resources, self.prompts.search.trim());
        self.sync_prompt_cursor_to_filter();
    }

    fn sync_prompt_cursor_to_filter(&mut self) {
        self.clamp_prompt_cursor();
        let indices = &self.prompts.filtered_indices;
        if let Some(first) = indices.first().copied() {
            if !indices.contains(&self.prompts.cursor) {
                self.prompts.cursor = first;
            }
        }
    }

    fn clamp_skill_cursor(&mut self) {
        if self.skill_resources.is_empty() {
            self.skills.cursor = 0;
        } else {
            self.skills.cursor = self
                .skills
                .cursor
                .min(self.skill_resources.len().saturating_sub(1));
        }
    }

    fn move_skill_cursor(&mut self, delta: isize) {
        let indices = &self.skills.filtered_indices;
        if indices.is_empty() {
            self.skills.cursor = 0;
            return;
        }
        let current = indices
            .iter()
            .position(|index| *index == self.skills.cursor)
            .unwrap_or(0);
        let next = move_index(current, indices.len(), delta);
        self.skills.cursor = indices[next];
    }

    fn refresh_skill_filter(&mut self) {
        self.skills.filtered_indices =
            filtered_skill_indices(&self.skill_resources, self.skills.search.trim());
        self.sync_skill_cursor_to_filter();
    }

    fn sync_skill_cursor_to_filter(&mut self) {
        self.clamp_skill_cursor();
        let indices = &self.skills.filtered_indices;
        if let Some(first) = indices.first().copied() {
            if !indices.contains(&self.skills.cursor) {
                self.skills.cursor = first;
            }
        }
    }

    fn enqueue_prompt(&mut self, prompt: String) {
        self.queued_items.push(prompt);
        self.clamp_send_panel_cursor();
    }

    fn record_steer(&mut self, prompt: String) {
        self.steer_items.push(prompt);
        self.clamp_send_panel_cursor();
    }

    fn draft_current_input(&mut self) -> bool {
        let Some(text) = self.take_composer_text() else {
            return false;
        };
        self.draft_items.push(text);
        self.clamp_send_panel_cursor();
        true
    }

    fn take_composer_text(&mut self) -> Option<String> {
        let text = self.composer.expanded_text().trim().to_string();
        if text.is_empty() {
            return None;
        }
        self.composer.clear();
        Some(text)
    }

    fn delete_send_panel_item(&mut self, item: &SendPanelItem) -> Option<String> {
        let list = match item.section {
            SendPanelSection::Steer => &mut self.steer_items,
            SendPanelSection::Queue => &mut self.queued_items,
            SendPanelSection::Draft => &mut self.draft_items,
        };
        if item.index >= list.len() {
            self.clamp_send_panel_cursor();
            return None;
        }
        let removed = list.remove(item.index);
        self.clamp_send_panel_cursor();
        Some(removed)
    }

    fn remove_or_copy_send_panel_item_for_input(&mut self, item: &SendPanelItem) -> String {
        match item.section {
            SendPanelSection::Steer => item.text.clone(),
            SendPanelSection::Queue | SendPanelSection::Draft => self
                .delete_send_panel_item(item)
                .unwrap_or_else(|| item.text.clone()),
        }
    }

    fn transcript_scroll_status(&self) -> String {
        if self.transcript_scroll.is_at_bottom() {
            HELP_STATUS.into()
        } else {
            "Transcript scrolled • PgUp/PgDn page • Alt-↑/↓ line • Ctrl-Home top • End bottom"
                .into()
        }
    }

    #[must_use]
    pub fn command_suggestions_view(&self) -> Option<CommandSuggestionsView> {
        if !self.can_show_command_suggestions() {
            return None;
        }
        let input = self.composer.text();
        if self.command_suggestions.is_dismissed_for(input) {
            return None;
        }
        self.command_suggestions
            .cached_view(input, self.composer.cursor())
    }

    fn can_show_command_suggestions(&self) -> bool {
        self.overlay.is_none() && self.focus == TuiFocus::Composer && self.composer.is_enabled()
    }

    fn build_command_suggestions(&self) -> Option<CommandSuggestionsView> {
        let input = self.composer.text();
        let cursor = self.composer.cursor();
        command_suggestions_for(
            input,
            cursor,
            &self.settings.models,
            &self.prompt_resources,
            &self.skill_resources,
            &self.extension_commands,
        )
        .or_else(|| self.extension_autosuggestions(input, cursor))
        .or_else(|| file_suggestions_for(input, cursor, &self.file_paths))
    }

    fn extension_autosuggestions(
        &self,
        input: &str,
        cursor: usize,
    ) -> Option<CommandSuggestionsView> {
        let cursor = cursor.min(input.len());
        let before_cursor = input.get(..cursor)?;
        let context = self
            .extension_ui
            .autosuggest_items
            .iter()
            .filter_map(|item| {
                let start = before_cursor.rfind(&item.trigger)?;
                let query = before_cursor[start + item.trigger.len()..].to_string();
                let token_has_space = query.chars().any(char::is_whitespace);
                (!token_has_space).then_some((start, query, item.trigger.clone()))
            })
            .min_by(|left, right| right.0.cmp(&left.0))?;
        let (replace_start, query, trigger) = context;
        let candidates = self
            .extension_ui
            .autosuggest_items
            .iter()
            .filter(|item| item.trigger == trigger)
            .cloned()
            .collect::<Vec<_>>();
        let indices = fuzzy_indices(&candidates, &query, FuzzyMode::Text, Some(10), |item| {
            format!("{} {} {}", item.label, item.summary, item.source)
        });
        let items = indices
            .into_iter()
            .map(|index| {
                let item = &candidates[index];
                CommandSuggestionItem {
                    label: item.label.clone(),
                    summary: format!("{} • {}", item.summary, item.source),
                    replacement: item.replacement.clone(),
                    replace_start,
                    replace_end: cursor,
                    complete_on_enter: false,
                    category: CommandSuggestionCategory::Extension,
                }
            })
            .collect::<Vec<_>>();
        (!items.is_empty()).then_some(CommandSuggestionsView {
            query,
            title: "Extension Suggestions".into(),
            items,
            selected: 0,
        })
    }

    pub fn set_extension_commands(&mut self, commands: Vec<ExtensionCommandSuggestion>) {
        self.extension_commands = commands;
        self.refresh_command_suggestions();
    }

    pub(crate) fn refresh_command_suggestions(&mut self) {
        let input = self.composer.text().to_string();
        let cursor = self.composer.cursor();
        if !self.can_show_command_suggestions() || self.command_suggestions.is_dismissed_for(&input)
        {
            self.command_suggestions.clear_cache();
            return;
        }
        let view = self.build_command_suggestions();
        self.command_suggestions.cache_view(&input, cursor, view);
    }

    #[must_use]
    pub const fn transcript_version(&self) -> u64 {
        self.transcript_version
    }

    fn mark_transcript_changed(&mut self) {
        self.transcript_version = self.transcript_version.wrapping_add(1);
        if self.transcript_version == 0 {
            self.transcript_version = 1;
        }
    }

    pub fn set_messages_from_oino(&mut self, messages: &[Message]) {
        self.messages = project_messages(messages);
        self.mark_transcript_changed();
    }

    pub fn start_message(&mut self, id: OinoId, role: impl Into<String>) {
        if self.messages.iter().any(|message| message.id == id) {
            return;
        }
        let role = role.into();
        let title = self.title_for_role(&role);
        self.messages.push(MessageView {
            id,
            role,
            title,
            content: String::new(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        self.mark_transcript_changed();
    }

    pub fn update_message(&mut self, id: OinoId, content: &[ContentBlock]) {
        let projected = project_content_blocks(content);
        let mut changed = false;
        if let Some(index) = self.messages.iter().position(|message| message.id == id) {
            let message = &mut self.messages[index];
            if message.content != projected.content
                || message.thinking != projected.thinking
                || message.thinking_redacted != projected.thinking_redacted
                || message.tool_calls != projected.tool_calls
            {
                message.content = projected.content;
                message.thinking = projected.thinking;
                message.thinking_redacted = projected.thinking_redacted;
                message.tool_calls = projected.tool_calls;
                changed = true;
            }
        } else {
            self.messages.push(MessageView {
                id,
                role: "assistant".into(),
                title: self.title_for_role("assistant"),
                content: projected.content,
                thinking: projected.thinking,
                thinking_redacted: projected.thinking_redacted,
                tool_call_id: None,
                tool_calls: projected.tool_calls,
                is_error: false,
            });
            changed = true;
        }
        if changed {
            self.mark_transcript_changed();
        }
    }

    pub fn finish_message(&mut self, message: &Message) {
        let mut view = project_message(message);
        let fallback_title = self.title_for_role(&view.role);
        let mut changed = false;
        if let Some(index) = self
            .messages
            .iter()
            .position(|existing| existing.id == view.id)
        {
            let existing = &mut self.messages[index];
            if view.title.is_none() {
                view.title = existing.title.clone().or(fallback_title);
            }
            if *existing != view {
                *existing = view;
                changed = true;
            }
        } else {
            if view.title.is_none() {
                view.title = fallback_title;
            }
            self.messages.push(view);
            changed = true;
        }
        if changed {
            self.mark_transcript_changed();
        }
    }

    fn title_for_role(&self, role: &str) -> Option<String> {
        if role == "assistant" {
            Some(self.settings.selected_model_label().to_string())
        } else {
            None
        }
    }

    pub fn set_working(&mut self, working: bool) {
        self.working = working;
        self.composer.set_enabled(true);
        if working {
            self.set_calling_status();
        } else {
            self.status = HELP_STATUS.into();
        }
    }

    pub fn set_calling_status(&mut self) {
        self.status = format!(
            "Calling {}… type and Enter to steer • Ctrl-O q queue/drafts",
            provider_label(self.settings.selected_model_label())
        );
    }

    pub fn set_model_catalog(&mut self, models: Vec<ModelOption>, status: impl Into<String>) {
        self.settings.set_models(models, status);
        self.refresh_command_suggestions();
    }

    pub fn set_working_directory(&mut self, working_directory: impl Into<String>) {
        self.runtime_status.working_directory = working_directory.into();
    }

    pub const fn set_context_tokens(&mut self, context_tokens: Option<usize>) {
        self.runtime_status.context_tokens = context_tokens;
    }

    pub fn set_usage_loading(&mut self) {
        self.usage.set_loading();
    }

    pub fn set_usage_report(&mut self, report: UsagePanelReport) {
        self.usage.set_report(report);
    }

    pub fn set_usage_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.usage.set_error(message.clone());
        self.set_error(message);
        self.status = HELP_STATUS.into();
    }

    pub fn set_agent_mode(&mut self, mode: AgentMode) {
        let label = mode.label();
        self.agent_mode = mode;
        self.status = format!("Mode set to {label}");
    }

    pub fn set_tool_settings(&mut self, tools: Vec<ToolSettingsItem>) {
        self.settings.set_tools(tools);
    }

    pub fn set_auth_status_items(
        &mut self,
        items: Vec<crate::settings::AuthStatusItem>,
        selected_provider: Option<&str>,
    ) {
        self.settings.set_auth_items(items);
        if let Some(provider) = selected_provider {
            self.settings.select_auth_provider(provider);
        }
    }

    pub fn set_auth_status_message(&mut self, message: impl Into<String>) {
        self.clear_error();
        self.status = message.into();
    }

    pub fn set_auth_status_error(&mut self, message: impl Into<String>) {
        self.set_error(message.into());
        self.status = HELP_STATUS.into();
    }

    pub fn append_command_output(&mut self, title: impl Into<String>, content: impl Into<String>) {
        self.messages.push(MessageView {
            id: OinoId::new_v4(),
            role: "assistant".into(),
            title: Some(title.into()),
            content: content.into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        self.transcript_scroll.jump_bottom();
        self.mark_transcript_changed();
    }

    pub fn set_theme_settings(&mut self, global: &ThemeSettings, project: &ThemeSettings) {
        self.resolved_theme = resolve_effective_theme(&self.theme_catalog, global, project);
        self.clear_theme_preview();
        self.settings
            .set_theme_state(&self.theme_catalog, global, project, &self.resolved_theme);
    }

    fn set_theme_preview(&mut self, id: String) {
        let mut project = ThemeSettings::default();
        project.set_active(id);
        let preview =
            resolve_effective_theme(&self.theme_catalog, &ThemeSettings::default(), &project);
        self.status = format!(
            "Previewing theme `{}` • p project / g global / Esc cancel",
            preview.display_name
        );
        self.settings.set_theme_preview(Some(preview.clone()));
        self.preview_theme = Some(preview);
    }

    fn clear_theme_preview(&mut self) {
        self.preview_theme = None;
        self.settings.clear_theme_preview();
    }

    pub fn active_theme(&self) -> &ResolvedTheme {
        self.preview_theme.as_ref().unwrap_or(&self.resolved_theme)
    }

    pub fn set_theme_catalog(
        &mut self,
        catalog: ThemeCatalog,
        global: &ThemeSettings,
        project: &ThemeSettings,
    ) {
        self.theme_catalog = catalog;
        self.set_theme_settings(global, project);
    }

    pub fn set_extension_ui_surfaces(
        &mut self,
        surfaces: Vec<ActiveContribution<UiSurfaceContribution>>,
    ) {
        self.extension_ui.set_surfaces(surfaces);
    }

    pub fn set_extension_shortcuts(&mut self, shortcuts: Vec<ExtensionShortcut>) {
        self.extension_ui
            .set_shortcuts(shortcuts, &self.settings.keymap);
    }

    pub fn set_extension_autosuggest_items(&mut self, items: Vec<ExtensionAutosuggestItem>) {
        self.extension_ui.set_autosuggest_items(items);
        self.refresh_command_suggestions();
    }

    pub fn set_extension_theme(&mut self, theme: ExtensionThemeState) {
        self.extension_ui.set_theme(theme);
    }

    pub fn set_extension_management_items(&mut self, items: Vec<ExtensionManagementItem>) {
        self.extension_management.set_items(items);
    }

    pub fn apply_extension_ui_update(
        &mut self,
        update: UiSurfaceStateUpdate,
    ) -> Result<(), UiSurfaceValidationError> {
        self.extension_ui.apply_update(update)
    }

    pub fn focus_extension_surface(&mut self, surface_id: &ContributionId) -> bool {
        if !self.extension_ui.focus_surface(surface_id) {
            return false;
        }
        if let Some(slot) = self
            .extension_ui
            .surfaces
            .iter()
            .find(|surface| &surface.effective_id == surface_id)
            .map(|surface| extension_surface_slot_key(&surface.entry.contribution))
        {
            self.extension_ui
                .surface_controller
                .set_slot_hidden(slot.clone(), false);
            self.extension_ui.surface_controller.focused_slot = Some(slot);
        }
        true
    }

    #[must_use]
    pub fn extension_key_action_for_scope(&self, scope: &str) -> Option<TuiAction> {
        self.extension_ui.action_for_scope(scope)
    }

    pub fn focus_next_extension_surface_slot(&mut self, delta: isize) -> bool {
        let slots = self.extension_ui.visible_surface_slot_keys();
        if slots.is_empty() {
            self.extension_ui.surface_controller.focused_slot = None;
            self.extension_ui.focused_surface = None;
            return false;
        }
        let current = self
            .extension_ui
            .surface_controller
            .focused_slot
            .as_ref()
            .and_then(|slot| slots.iter().position(|candidate| candidate == slot));
        let next = current.map_or_else(
            || {
                if delta.is_negative() {
                    slots.len().saturating_sub(1)
                } else {
                    0
                }
            },
            |current| (current as isize + delta).rem_euclid(slots.len() as isize) as usize,
        );
        self.focus_extension_surface_slot(slots[next].clone())
    }

    pub fn advance_extension_surface_tab(&mut self, delta: isize) -> bool {
        let slot = self
            .extension_ui
            .surface_controller
            .focused_slot
            .clone()
            .or_else(|| {
                self.extension_ui
                    .visible_surface_slot_keys()
                    .into_iter()
                    .next()
            });
        let Some(slot) = slot else {
            return false;
        };
        let len = self
            .extension_ui
            .surfaces
            .iter()
            .filter(|surface| extension_surface_slot_key(&surface.entry.contribution) == slot)
            .count();
        if len == 0 {
            return false;
        }
        let current = self.extension_ui.surface_controller.active_tab(&slot, len);
        let next = (current as isize + delta).rem_euclid(len as isize) as usize;
        self.extension_ui
            .surface_controller
            .set_active_tab(slot.clone(), next);
        self.focus_extension_surface_slot(slot)
    }

    pub fn toggle_extension_surface_kind(&mut self, kind: UiSurfaceKind) -> bool {
        let slots = self.extension_ui.surface_slot_keys_for_kind(kind);
        if slots.is_empty() {
            return false;
        }
        let hide = slots
            .iter()
            .any(|slot| !self.extension_ui.surface_controller.is_slot_hidden(slot));
        for slot in slots {
            self.extension_ui
                .surface_controller
                .set_slot_hidden(slot, hide);
        }
        if hide {
            self.extension_ui.surface_controller.focused_slot = None;
            self.extension_ui.focused_surface = None;
        }
        true
    }

    pub fn close_focused_extension_surface_slot(&mut self) -> bool {
        let slot = self
            .extension_ui
            .surface_controller
            .focused_slot
            .clone()
            .or_else(|| self.first_visible_extension_surface_slot(UiSurfaceKind::FloatingPanel))
            .or_else(|| self.first_visible_extension_surface_slot(UiSurfaceKind::Overlay));
        let Some(slot) = slot else {
            return false;
        };
        self.extension_ui
            .surface_controller
            .set_slot_hidden(slot, true);
        self.extension_ui.surface_controller.focused_slot = None;
        self.extension_ui.focused_surface = None;
        true
    }

    fn focus_extension_surface_slot(&mut self, slot: String) -> bool {
        self.extension_ui
            .surface_controller
            .set_slot_hidden(slot.clone(), false);
        self.extension_ui.surface_controller.focused_slot = Some(slot.clone());
        if let Some(surface) = self.extension_ui.active_surface_for_slot(&slot) {
            self.extension_ui.focused_surface = Some(surface.effective_id.clone());
        } else {
            self.extension_ui.focused_surface = None;
        }
        true
    }

    fn first_visible_extension_surface_slot(&self, kind: UiSurfaceKind) -> Option<String> {
        self.extension_ui
            .surface_slot_keys_for_kind(kind)
            .into_iter()
            .find(|slot| !self.extension_ui.surface_controller.is_slot_hidden(slot))
    }

    pub fn set_keymap(&mut self, keymap: KeymapConfig) {
        self.settings.set_keymap(keymap);
        let shortcuts = self.extension_ui.shortcuts.clone();
        self.extension_ui
            .set_shortcuts(shortcuts, &self.settings.keymap);
        self.refresh_help_filter();
    }

    pub fn set_session_title(&mut self, title: impl Into<String>) {
        self.session_title = title.into();
    }

    pub fn set_model_catalog_refreshing(&mut self, refreshing: bool) {
        self.settings.set_refreshing(refreshing);
    }

    pub fn set_error(&mut self, error: impl Into<String>) {
        self.error = Some(error.into());
        self.working = false;
        self.composer.set_enabled(true);
    }

    pub fn clear_error(&mut self) {
        self.error = None;
    }

    pub fn handle_paste(&mut self, text: &str) -> TuiAction {
        if self.overlay.is_some() || self.chord != ChordState::None || !self.composer.is_enabled() {
            return TuiAction::None;
        }
        self.focus = TuiFocus::Composer;
        let normalized = normalize_paste_text(text);
        let paste_chars = char_count(&normalized);
        if paste_chars > MAX_PASTE_CHARS {
            self.status = format!(
                "Paste rejected: {paste_chars} chars exceeds the {MAX_PASTE_CHARS} char limit"
            );
            return TuiAction::None;
        }

        let before = self.composer.text().to_string();
        let inserted = if should_collapse_paste(&normalized) {
            let summary = collapsed_paste_summary(&normalized);
            let inserted = self.composer.insert_collapsed_paste(&normalized).is_some();
            if inserted {
                self.status =
                    format!("Collapsed {summary} • Ctrl-O e expand • Enter sends full text");
            }
            inserted
        } else {
            self.composer.insert_text(&normalized)
        };
        if inserted {
            self.after_composer_edit(&before);
        }
        TuiAction::None
    }

    pub fn insert_literal(&mut self, text: &str) -> TuiAction {
        if self.overlay.is_some() || self.chord != ChordState::None || !self.composer.is_enabled() {
            return TuiAction::None;
        }
        self.focus = TuiFocus::Composer;
        let before = self.composer.text().to_string();
        if self.composer.insert_text(text) {
            self.after_composer_edit(&before);
        }
        TuiAction::None
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> TuiAction {
        if is_force_quit_key(key) {
            return self.arm_or_quit_with_status(
                "Press Ctrl-C again to quit • Esc stops a running response",
            );
        }
        self.expire_quit_pending();

        if self.overlay == Some(OverlayKind::Settings)
            && (matches!(self.settings.keymaps_mode, KeymapsMode::Capture { .. })
                || self.settings.notify.edit.is_some())
        {
            return self.handle_settings_key(key);
        }

        if self.overlay == Some(OverlayKind::AskUser) {
            return self.handle_ask_user_key(key);
        }
        if self.overlay == Some(OverlayKind::Usage) {
            return self.handle_usage_key(key);
        }
        if self.overlay == Some(OverlayKind::Btw) {
            return self.handle_btw_key(key);
        }

        if self.overlay.is_none()
            && matches!(key.code, KeyCode::Esc)
            && self.close_focused_extension_surface_slot()
        {
            self.status = "Closed focused extension surface slot".into();
            return TuiAction::None;
        }

        match self.resolve_keymap_key(key) {
            KeymapKeyResult::Matched(action) => return self.execute_key_action(action),
            KeymapKeyResult::MatchedExtension(action) => {
                return TuiAction::RunExtensionAction { action }
            }
            KeymapKeyResult::Consumed => return TuiAction::None,
            KeymapKeyResult::Unhandled => {}
        }

        match self.overlay {
            Some(OverlayKind::Help) if self.help_search_active => {
                return self.handle_help_search_text_key(key);
            }
            Some(OverlayKind::Settings) => return self.handle_settings_text_key(key),
            Some(OverlayKind::Sessions) if self.sessions.search_active => {
                return self.handle_sessions_search_text_key(key);
            }
            Some(OverlayKind::Prompts) if self.prompts.search_active => {
                return self.handle_prompts_search_text_key(key);
            }
            Some(OverlayKind::Skills) if self.skills.search_active => {
                return self.handle_skills_search_text_key(key);
            }
            Some(OverlayKind::Extensions) if self.extension_management.install_active => {
                return self.handle_extensions_install_text_key(key);
            }
            Some(OverlayKind::Extensions) if self.extension_management.search_active => {
                return self.handle_extensions_search_text_key(key);
            }
            Some(OverlayKind::Extensions) => return self.handle_extensions_plain_key(key),
            Some(_) => return TuiAction::None,
            None => {}
        }

        if matches!(key.code, KeyCode::Esc) {
            if self.close_focused_extension_surface_slot() {
                self.status = "Closed focused extension surface slot".into();
                return TuiAction::None;
            }
            if self.working {
                self.status = "Stopping response…".into();
                return TuiAction::AbortPrompt;
            }
            self.status = "Esc ignored • press Ctrl-C twice to quit".into();
            return TuiAction::None;
        }

        if self.focus == TuiFocus::Composer {
            let before = self.composer.text().to_string();
            self.composer.handle_edit_key(key);
            self.after_composer_edit(&before);
        }
        TuiAction::None
    }

    fn handle_btw_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Esc => {
                self.overlay = None;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyCode::Enter if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.btw.input.push('\n');
                TuiAction::None
            }
            KeyCode::Enter => {
                let prompt = self.btw.input.trim().to_string();
                if prompt == "/new" {
                    self.btw.input.clear();
                    self.btw.messages.clear();
                    self.btw.error = None;
                    self.status = "BTW session reset".into();
                    TuiAction::ResetBtwSession
                } else if prompt.is_empty() {
                    self.status = "BTW: type a prompt before pressing Enter".into();
                    TuiAction::None
                } else if self.btw.working {
                    self.status = "BTW prompt is already running".into();
                    TuiAction::None
                } else {
                    self.btw.input.clear();
                    self.btw.working = true;
                    self.btw.error = None;
                    self.status = "BTW running…".into();
                    TuiAction::SubmitBtwPrompt(prompt)
                }
            }
            KeyCode::Backspace => {
                self.btw.input.pop();
                TuiAction::None
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.btw.input.push(ch);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_usage_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.usage.search_active {
            return match key.code {
                KeyCode::Esc => {
                    self.usage.search_active = false;
                    self.usage.search.clear();
                    self.usage.refresh_filter();
                    self.status = "Usage search cleared".into();
                    TuiAction::None
                }
                KeyCode::Enter => {
                    self.usage.search_active = false;
                    self.status = usage_panel_status(&self.usage);
                    TuiAction::None
                }
                KeyCode::Backspace => {
                    self.usage.search.pop();
                    self.usage.refresh_filter();
                    self.status = usage_search_status(&self.usage.search);
                    TuiAction::None
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.usage.move_cursor(-1);
                    TuiAction::None
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.usage.move_cursor(1);
                    TuiAction::None
                }
                KeyCode::Char(ch) if key.modifiers.is_empty() => {
                    self.usage.search.push(ch);
                    self.usage.refresh_filter();
                    self.status = usage_search_status(&self.usage.search);
                    TuiAction::None
                }
                _ => TuiAction::None,
            };
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.overlay = None;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyCode::Up | KeyCode::Char('k') => {
                self.usage.move_cursor(-1);
                self.status = usage_panel_status(&self.usage);
                TuiAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.usage.move_cursor(1);
                self.status = usage_panel_status(&self.usage);
                TuiAction::None
            }
            KeyCode::PageUp => {
                self.usage.move_cursor(-5);
                self.status = usage_panel_status(&self.usage);
                TuiAction::None
            }
            KeyCode::PageDown => {
                self.usage.move_cursor(5);
                self.status = usage_panel_status(&self.usage);
                TuiAction::None
            }
            KeyCode::Home => {
                self.usage.cursor = self.usage.filtered_indices.first().copied().unwrap_or(0);
                self.status = usage_panel_status(&self.usage);
                TuiAction::None
            }
            KeyCode::End => {
                self.usage.cursor = self.usage.filtered_indices.last().copied().unwrap_or(0);
                self.status = usage_panel_status(&self.usage);
                TuiAction::None
            }
            KeyCode::Char('/') => {
                self.usage.search_active = true;
                self.usage.search.clear();
                self.usage.refresh_filter();
                self.status = "Usage search active".into();
                TuiAction::None
            }
            KeyCode::Char('r') | KeyCode::Char('R') => {
                self.set_usage_loading();
                self.status = "Refreshing usage report…".into();
                TuiAction::RefreshUsage
            }
            _ => TuiAction::None,
        }
    }

    fn handle_ask_user_key(&mut self, key: KeyEvent) -> TuiAction {
        let Some(state) = &mut self.ask_user else {
            self.overlay = None;
            return TuiAction::None;
        };
        if state.custom_active {
            match key.code {
                KeyCode::Esc => {
                    state.custom_active = false;
                    state.custom_input.clear();
                    self.status = "Ask user: custom answer canceled".into();
                }
                KeyCode::Enter => {
                    if let Some(outcome) = state.answer_custom() {
                        self.ask_user = None;
                        self.overlay = None;
                        self.status = "Ask user answered".into();
                        return TuiAction::AnswerAskUser(outcome);
                    }
                    self.status = "Type a custom answer before pressing Enter".into();
                }
                KeyCode::Backspace => {
                    state.custom_input.pop();
                }
                KeyCode::Char(ch) if state.custom_input.len() < 2_000 => {
                    state.custom_input.push(ch);
                }
                _ => {}
            }
            return TuiAction::None;
        }

        match key.code {
            KeyCode::Esc => {
                self.ask_user = None;
                self.overlay = None;
                self.status = "Ask user canceled".into();
                TuiAction::AnswerAskUser(AskUserOutcome {
                    answers: Vec::new(),
                    cancelled: true,
                    error: None,
                })
            }
            KeyCode::Up | KeyCode::Char('k') => {
                state.move_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.move_cursor(1);
                TuiAction::None
            }
            KeyCode::Char(' ') => {
                state.toggle_current();
                TuiAction::None
            }
            KeyCode::Char('c') => {
                state.custom_active = true;
                state.custom_input.clear();
                self.status = "Ask user: type custom answer • Enter submit • Esc cancel".into();
                TuiAction::None
            }
            KeyCode::Char('t') => {
                if let Some(outcome) = state.answer_chat() {
                    self.ask_user = None;
                    self.overlay = None;
                    self.status = "Ask user answered".into();
                    TuiAction::AnswerAskUser(outcome)
                } else {
                    TuiAction::None
                }
            }
            KeyCode::Enter => {
                if let Some(outcome) = state.answer_current_option() {
                    self.ask_user = None;
                    self.overlay = None;
                    self.status = "Ask user answered".into();
                    TuiAction::AnswerAskUser(outcome)
                } else {
                    self.status = "Ask user: select an option or type c for custom".into();
                    TuiAction::None
                }
            }
            _ => TuiAction::None,
        }
    }

    fn resolve_keymap_key(&mut self, key: KeyEvent) -> KeymapKeyResult {
        let Some(stroke) = KeyStroke::from_event(key) else {
            self.key_sequence.clear();
            self.chord = ChordState::None;
            return KeymapKeyResult::Unhandled;
        };
        let contexts = self.active_key_contexts();
        let mut sequence = self.key_sequence.clone();
        sequence.push(stroke);
        match self.settings.keymap.resolve(&contexts, &sequence) {
            KeymapMatch::Matched(action) => {
                self.key_sequence.clear();
                self.chord = ChordState::None;
                KeymapKeyResult::Matched(action)
            }
            KeymapMatch::Pending => {
                self.key_sequence = sequence;
                self.chord = if self.key_sequence.len() == 1
                    && self.key_sequence[0]
                        .to_string()
                        .eq_ignore_ascii_case("ctrl-o")
                {
                    ChordState::CtrlO
                } else {
                    ChordState::None
                };
                self.status = format!(
                    "{} prefix active • press next key or Esc cancel",
                    self.key_sequence
                        .iter()
                        .map(ToString::to_string)
                        .collect::<Vec<_>>()
                        .join(" ")
                );
                KeymapKeyResult::Consumed
            }
            KeymapMatch::None if !self.key_sequence.is_empty() => {
                if self.extension_shortcuts_are_active() {
                    match resolve_extension_shortcut(&self.extension_ui.shortcuts, &sequence) {
                        ExtensionShortcutMatch::Matched(action) => {
                            self.key_sequence.clear();
                            self.chord = ChordState::None;
                            return KeymapKeyResult::MatchedExtension(action);
                        }
                        ExtensionShortcutMatch::Pending => {
                            self.key_sequence = sequence;
                            self.chord = ChordState::None;
                            self.status = format!(
                                "{} extension prefix active • press next key or Esc cancel",
                                self.key_sequence
                                    .iter()
                                    .map(ToString::to_string)
                                    .collect::<Vec<_>>()
                                    .join(" ")
                            );
                            return KeymapKeyResult::Consumed;
                        }
                        ExtensionShortcutMatch::None => {}
                    }
                }
                self.key_sequence.clear();
                self.chord = ChordState::None;
                if stroke.is_escape() {
                    self.status = HELP_STATUS.into();
                } else {
                    self.status = "Unknown key chord".into();
                }
                KeymapKeyResult::Consumed
            }
            KeymapMatch::None if self.extension_shortcuts_are_active() => {
                match resolve_extension_shortcut(&self.extension_ui.shortcuts, &sequence) {
                    ExtensionShortcutMatch::Matched(action) => {
                        self.key_sequence.clear();
                        self.chord = ChordState::None;
                        KeymapKeyResult::MatchedExtension(action)
                    }
                    ExtensionShortcutMatch::Pending => {
                        self.key_sequence = sequence;
                        self.chord = ChordState::None;
                        self.status = format!(
                            "{} extension prefix active • press next key or Esc cancel",
                            self.key_sequence
                                .iter()
                                .map(ToString::to_string)
                                .collect::<Vec<_>>()
                                .join(" ")
                        );
                        KeymapKeyResult::Consumed
                    }
                    ExtensionShortcutMatch::None => KeymapKeyResult::Unhandled,
                }
            }
            KeymapMatch::None => KeymapKeyResult::Unhandled,
        }
    }

    fn extension_shortcuts_are_active(&self) -> bool {
        self.overlay.is_none()
    }

    fn active_key_contexts(&self) -> Vec<KeyContext> {
        let mut contexts = Vec::new();
        match self.overlay {
            Some(OverlayKind::Help) if self.help_search_active => contexts.push(KeyContext::Search),
            Some(OverlayKind::Help) => contexts.push(KeyContext::Help),
            Some(OverlayKind::Settings) => self.push_settings_key_contexts(&mut contexts),
            Some(OverlayKind::SendPanel) if self.send_panel.confirm_delete => {
                contexts.push(KeyContext::SendPanelConfirm);
            }
            Some(OverlayKind::SendPanel) => contexts.push(KeyContext::SendPanel),
            Some(OverlayKind::Sessions) if self.sessions.search_active => {
                contexts.push(KeyContext::Search);
            }
            Some(OverlayKind::Sessions) => contexts.push(KeyContext::Sessions),
            Some(OverlayKind::Prompts) if self.prompts.search_active => {
                contexts.push(KeyContext::Search);
            }
            Some(OverlayKind::Skills) if self.skills.search_active => {
                contexts.push(KeyContext::Search);
            }
            Some(OverlayKind::Extensions)
                if self.extension_management.search_active
                    || self.extension_management.install_active =>
            {
                contexts.push(KeyContext::Search);
            }
            Some(OverlayKind::Prompts | OverlayKind::Skills) => {
                contexts.push(KeyContext::ResourceBrowser);
            }
            Some(OverlayKind::Extensions) => contexts.push(KeyContext::Sessions),
            Some(OverlayKind::Inspect) => contexts.push(KeyContext::Inspect),
            Some(OverlayKind::Usage) => contexts.push(KeyContext::Sessions),
            Some(OverlayKind::AskUser) => contexts.push(KeyContext::Sessions),
            Some(OverlayKind::Btw) => contexts.push(KeyContext::Sessions),
            None => {
                if self.command_suggestions_view().is_some() {
                    contexts.push(KeyContext::CommandSuggestions);
                }
                if self.focus == TuiFocus::Transcript
                    || (self.focus == TuiFocus::Composer && self.composer.is_empty())
                {
                    contexts.push(KeyContext::Transcript);
                }
                if self.focus == TuiFocus::Composer {
                    contexts.push(KeyContext::Composer);
                }
            }
        }
        if self.overlay.is_none() {
            contexts.push(KeyContext::Global);
        }
        contexts
    }

    fn push_settings_key_contexts(&self, contexts: &mut Vec<KeyContext>) {
        if self.settings.page == crate::settings::SettingsPage::Models
            && self.settings.model_search_active
        {
            contexts.push(KeyContext::Search);
        }
        match self.settings.page {
            crate::settings::SettingsPage::Tools => contexts.push(KeyContext::SettingsTools),
            crate::settings::SettingsPage::Keymaps => match self.settings.keymaps_mode {
                KeymapsMode::Detail => contexts.push(KeyContext::SettingsKeymapDetail),
                KeymapsMode::ShortcutType { .. } => contexts.push(KeyContext::SettingsKeymapType),
                KeymapsMode::ChordKeyCapture => contexts.push(KeyContext::SettingsKeymaps),
                KeymapsMode::PresetSelect => contexts.push(KeyContext::SettingsKeymapPreset),
                KeymapsMode::PresetConfirm { .. } => {
                    contexts.push(KeyContext::SettingsKeymapPresetConfirm);
                    contexts.push(KeyContext::SendPanelConfirm);
                }
                KeymapsMode::List | KeymapsMode::Capture { .. } => {
                    contexts.push(KeyContext::SettingsKeymaps);
                }
            },
            _ => {}
        }
        contexts.push(KeyContext::Settings);
    }

    fn execute_key_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::CommonClose
            | KeyAction::CommonBack
            | KeyAction::CommonUp
            | KeyAction::CommonDown
            | KeyAction::CommonPageUp
            | KeyAction::CommonPageDown
            | KeyAction::CommonTop
            | KeyAction::CommonBottom
            | KeyAction::CommonConfirm
            | KeyAction::CommonSearch
            | KeyAction::CommonRefresh
            | KeyAction::CommonBackspace
            | KeyAction::CommonNext
            | KeyAction::CommonPrevious => TuiAction::None,
            KeyAction::AppQuit => self.arm_or_quit(),
            KeyAction::HelpOpen => {
                self.open_help_overlay();
                TuiAction::None
            }
            KeyAction::SettingsOpen => {
                self.open_settings_overlay();
                TuiAction::None
            }
            KeyAction::SendPanelOpen => {
                self.open_send_panel();
                TuiAction::None
            }
            KeyAction::BtwOpen => {
                self.open_btw_overlay();
                TuiAction::OpenBtw
            }
            KeyAction::TranscriptFocus => {
                self.focus = TuiFocus::Transcript;
                self.status = "Transcript focus".into();
                TuiAction::None
            }
            KeyAction::ComposerExpandReference => {
                self.expand_reference_action();
                TuiAction::None
            }
            KeyAction::ComposerSubmit => self.submit_input(),
            KeyAction::ComposerNewline => {
                let before = self.composer.text().to_string();
                self.composer.insert_text("\n");
                self.after_composer_edit(&before);
                TuiAction::None
            }
            KeyAction::ComposerQueuePrompt => self.queue_composer_input(),
            KeyAction::ComposerDraftPrompt => {
                if self.draft_current_input() {
                    self.status = "Moved current input to Draft".into();
                } else {
                    self.status = "No input to draft".into();
                }
                TuiAction::None
            }
            KeyAction::SuggestionsClose
            | KeyAction::SuggestionsUp
            | KeyAction::SuggestionsDown
            | KeyAction::SuggestionsAccept
            | KeyAction::SuggestionsConfirm => self.execute_suggestion_action(action),
            KeyAction::TranscriptUnfocus
            | KeyAction::TranscriptPageUp
            | KeyAction::TranscriptPageDown
            | KeyAction::TranscriptLineUp
            | KeyAction::TranscriptLineDown
            | KeyAction::TranscriptTop
            | KeyAction::TranscriptBottom => self.execute_transcript_action(action),
            KeyAction::HelpClose
            | KeyAction::HelpSearch
            | KeyAction::HelpUp
            | KeyAction::HelpDown
            | KeyAction::HelpPageUp
            | KeyAction::HelpPageDown
            | KeyAction::HelpTop
            | KeyAction::HelpBottom => self.execute_help_action(action),
            KeyAction::SearchClose
            | KeyAction::SearchAccept
            | KeyAction::SearchBackspace
            | KeyAction::SearchUp
            | KeyAction::SearchDown
            | KeyAction::SearchPageUp
            | KeyAction::SearchPageDown
            | KeyAction::SearchTop
            | KeyAction::SearchBottom => self.execute_search_action(action),
            KeyAction::SendPanelClose
            | KeyAction::SendPanelUp
            | KeyAction::SendPanelDown
            | KeyAction::SendPanelQueue
            | KeyAction::SendPanelDraft
            | KeyAction::SendPanelDelete
            | KeyAction::SendPanelLoad => self.execute_send_panel_action(action),
            KeyAction::ConfirmYes | KeyAction::ConfirmNo => self.execute_confirm_action(action),
            KeyAction::SessionsClose
            | KeyAction::SessionsUp
            | KeyAction::SessionsDown
            | KeyAction::SessionsSearch
            | KeyAction::SessionsRefresh
            | KeyAction::SessionsOpen => self.execute_sessions_action(action),
            KeyAction::ResourceClose
            | KeyAction::ResourceUp
            | KeyAction::ResourceDown
            | KeyAction::ResourceSearch
            | KeyAction::ResourceRefresh
            | KeyAction::ResourceComplete => self.execute_resource_action(action),
            KeyAction::InspectClose
            | KeyAction::InspectUp
            | KeyAction::InspectDown
            | KeyAction::InspectPageUp
            | KeyAction::InspectPageDown
            | KeyAction::InspectTop
            | KeyAction::InspectExportHtml => self.execute_inspect_action(action),
            KeyAction::SettingsClose
            | KeyAction::SettingsBack
            | KeyAction::SettingsOpenPage
            | KeyAction::SettingsUp
            | KeyAction::SettingsDown
            | KeyAction::SettingsNext
            | KeyAction::SettingsPrevious
            | KeyAction::SettingsApply
            | KeyAction::SettingsSearch
            | KeyAction::SettingsToolToggleGlobal
            | KeyAction::SettingsToolToggleProject
            | KeyAction::KeymapEditChordKey
            | KeyAction::KeymapAddShortcut
            | KeyAction::KeymapRemoveShortcut
            | KeyAction::KeymapClearShortcuts
            | KeyAction::KeymapResetAction
            | KeyAction::KeymapSelectPreset => self.execute_settings_action(action),
            KeyAction::ExtensionSurfaceFocusNext
            | KeyAction::ExtensionSurfaceFocusPrevious
            | KeyAction::ExtensionSurfaceTabNext
            | KeyAction::ExtensionSurfaceTabPrevious
            | KeyAction::ExtensionSurfaceClose
            | KeyAction::ExtensionSidebarToggle
            | KeyAction::ExtensionMainPanelToggle => self.execute_extension_surface_action(action),
        }
    }

    fn arm_or_quit(&mut self) -> TuiAction {
        self.arm_or_quit_with_status("Press quit again to exit • Esc stops a running response")
    }

    fn arm_or_quit_with_status(&mut self, status: &'static str) -> TuiAction {
        self.expire_quit_pending();
        if self.quit_pending {
            TuiAction::Quit
        } else {
            self.quit_pending = true;
            self.quit_armed_at = Some(Instant::now());
            self.status = status.into();
            TuiAction::None
        }
    }

    fn expire_quit_pending(&mut self) {
        if self
            .quit_armed_at
            .is_some_and(|armed_at| armed_at.elapsed() > QUIT_ARM_WINDOW)
        {
            self.quit_pending = false;
            self.quit_armed_at = None;
        }
    }

    fn queue_composer_input(&mut self) -> TuiAction {
        let Some(prompt) = self.take_composer_text() else {
            self.status = "No input to queue".into();
            return TuiAction::None;
        };
        self.enqueue_prompt(prompt.clone());
        self.status = format!("Queued {}", summarize_panel_text(&prompt));
        TuiAction::QueuePrompt(prompt)
    }

    fn expand_reference_action(&mut self) {
        if self.composer.expand_collapsed_paste_at_cursor() {
            self.status = "Expanded pasted block".into();
        } else {
            match self.expand_prompt_references_in_composer() {
                PromptReferenceExpansionResult::Expanded(count) => {
                    let plural = if count == 1 { "" } else { "s" };
                    self.status = format!("Expanded {count} prompt template{plural}");
                }
                PromptReferenceExpansionResult::NoPromptReference => {
                    self.status = "No collapsed paste block or prompt reference to expand".into();
                }
                PromptReferenceExpansionResult::Incomplete(token) => {
                    self.set_error(format!("Incomplete resource reference `{token}`"));
                    self.status = HELP_STATUS.into();
                }
                PromptReferenceExpansionResult::UnknownPrompt(name) => {
                    self.set_error(format!("Unknown prompt `/prompt:{name}`"));
                    self.status = HELP_STATUS.into();
                }
            }
        }
    }

    fn execute_suggestion_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::SuggestionsClose => {
                self.command_suggestions.dismiss_for(self.composer.text());
                TuiAction::None
            }
            KeyAction::SuggestionsUp => {
                let len = self
                    .command_suggestions_view()
                    .map_or(0, |view| view.items.len());
                self.command_suggestions.move_selection(-1, len);
                TuiAction::None
            }
            KeyAction::SuggestionsDown => {
                let len = self
                    .command_suggestions_view()
                    .map_or(0, |view| view.items.len());
                self.command_suggestions.move_selection(1, len);
                TuiAction::None
            }
            KeyAction::SuggestionsAccept => {
                self.accept_command_suggestion(false);
                TuiAction::None
            }
            KeyAction::SuggestionsConfirm => {
                let has_selection = self
                    .command_suggestions_view()
                    .and_then(|view| view.selected_item().cloned())
                    .is_some();
                if !has_selection || self.accept_command_suggestion(true) {
                    self.submit_input()
                } else {
                    TuiAction::None
                }
            }
            _ => TuiAction::None,
        }
    }

    fn execute_transcript_action(&mut self, action: KeyAction) -> TuiAction {
        let page_lines = self.transcript_page_lines.saturating_sub(1).max(1);
        match action {
            KeyAction::TranscriptUnfocus => {
                if self.focus == TuiFocus::Transcript {
                    self.focus = TuiFocus::Composer;
                    self.status = self.transcript_scroll_status();
                } else if self.working {
                    self.status = "Stopping response…".into();
                    return TuiAction::AbortPrompt;
                } else {
                    self.status = "Esc ignored • press Ctrl-C twice to quit".into();
                }
            }
            KeyAction::TranscriptPageUp => self.scroll_transcript_up(page_lines),
            KeyAction::TranscriptPageDown => self.scroll_transcript_down(page_lines),
            KeyAction::TranscriptLineUp => self.scroll_transcript_up(TRANSCRIPT_SCROLL_LINE_STEP),
            KeyAction::TranscriptLineDown => {
                self.scroll_transcript_down(TRANSCRIPT_SCROLL_LINE_STEP);
            }
            KeyAction::TranscriptTop => self.scroll_transcript_to_top(),
            KeyAction::TranscriptBottom => self.scroll_transcript_to_bottom(),
            _ => {}
        }
        TuiAction::None
    }

    fn execute_help_action(&mut self, action: KeyAction) -> TuiAction {
        let page = self.transcript_page_lines.max(5);
        match action {
            KeyAction::HelpClose => {
                self.overlay = None;
                self.status = HELP_STATUS.into();
            }
            KeyAction::HelpSearch => {
                self.help_search_active = true;
                self.help_search.clear();
                self.refresh_help_filter();
                self.status = "Help search active".into();
            }
            KeyAction::HelpUp => self.scroll_help_up(1),
            KeyAction::HelpDown => self.scroll_help_down(1),
            KeyAction::HelpPageUp => self.scroll_help_up(page),
            KeyAction::HelpPageDown => self.scroll_help_down(page),
            KeyAction::HelpTop => self.help_scroll = 0,
            KeyAction::HelpBottom => {
                self.help_scroll = self.filtered_help_indices.len().saturating_sub(1);
            }
            _ => {}
        }
        TuiAction::None
    }

    fn execute_search_action(&mut self, action: KeyAction) -> TuiAction {
        match self.overlay {
            Some(OverlayKind::Help) => self.execute_help_search_action(action),
            Some(OverlayKind::Sessions) => self.execute_sessions_search_action(action),
            Some(OverlayKind::Extensions) if self.extension_management.install_active => {
                self.execute_extensions_install_action(action)
            }
            Some(OverlayKind::Extensions) => self.execute_extensions_search_action(action),
            Some(OverlayKind::Prompts) => self.execute_prompts_search_action(action),
            Some(OverlayKind::Skills) => self.execute_skills_search_action(action),
            Some(OverlayKind::Settings) => self.execute_settings_search_action(action),
            _ => TuiAction::None,
        }
    }

    fn execute_settings_search_action(&mut self, action: KeyAction) -> TuiAction {
        let key = match action {
            KeyAction::SearchClose => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            KeyAction::SearchAccept => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            KeyAction::SearchBackspace => KeyEvent::new(KeyCode::Backspace, KeyModifiers::NONE),
            KeyAction::SearchUp => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            KeyAction::SearchDown => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            _ => KeyEvent::new(KeyCode::Null, KeyModifiers::NONE),
        };
        self.handle_settings_key(key)
    }

    fn execute_help_search_action(&mut self, action: KeyAction) -> TuiAction {
        let page = self.transcript_page_lines.max(5);
        match action {
            KeyAction::SearchClose => {
                self.help_search_active = false;
                self.help_search.clear();
                self.refresh_help_filter();
                self.status = "Help search cleared".into();
            }
            KeyAction::SearchAccept => self.help_search_active = false,
            KeyAction::SearchBackspace => {
                self.help_search.pop();
                self.refresh_help_filter();
                self.status = help_search_status(&self.help_search);
            }
            KeyAction::SearchUp => self.scroll_help_up(1),
            KeyAction::SearchDown => self.scroll_help_down(1),
            KeyAction::SearchPageUp => self.scroll_help_up(page),
            KeyAction::SearchPageDown => self.scroll_help_down(page),
            KeyAction::SearchTop => self.help_scroll = 0,
            KeyAction::SearchBottom => {
                self.help_scroll = self.filtered_help_indices.len().saturating_sub(1);
            }
            _ => {}
        }
        TuiAction::None
    }

    fn execute_send_panel_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::SendPanelClose => {
                self.overlay = None;
                if self.working {
                    self.set_calling_status();
                } else {
                    self.status = HELP_STATUS.into();
                }
                TuiAction::None
            }
            KeyAction::SendPanelUp => {
                self.move_send_panel_cursor(-1);
                TuiAction::None
            }
            KeyAction::SendPanelDown => {
                self.move_send_panel_cursor(1);
                TuiAction::None
            }
            KeyAction::SendPanelQueue => self.queue_composer_input(),
            KeyAction::SendPanelDraft => {
                if self.draft_current_input() {
                    self.status = "Moved current input to Draft".into();
                } else {
                    self.status = "No input to draft".into();
                }
                TuiAction::None
            }
            KeyAction::SendPanelDelete => {
                if self.selected_send_panel_item().is_some() {
                    self.send_panel.confirm_delete = true;
                    self.status = "Press y to confirm deletion • n/Esc cancel".into();
                } else {
                    self.status = "Nothing selected to delete".into();
                }
                TuiAction::None
            }
            KeyAction::SendPanelLoad => {
                let Some(item) = self.selected_send_panel_item() else {
                    self.status = "Nothing selected to load".into();
                    return TuiAction::None;
                };
                if self.draft_current_input() {
                    self.status = "Moved current input to Draft and loaded selection".into();
                } else {
                    self.status = "Loaded selection into input".into();
                }
                let text = self.remove_or_copy_send_panel_item_for_input(&item);
                self.composer.replace_text(text);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn execute_confirm_action(&mut self, action: KeyAction) -> TuiAction {
        if self.overlay == Some(OverlayKind::Settings)
            && matches!(
                self.settings.keymaps_mode,
                KeymapsMode::PresetConfirm { .. }
            )
        {
            let key = match action {
                KeyAction::ConfirmYes => KeyEvent::new(KeyCode::Char('y'), KeyModifiers::NONE),
                KeyAction::ConfirmNo => KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE),
                _ => KeyEvent::new(KeyCode::Null, KeyModifiers::NONE),
            };
            return self.handle_settings_key(key);
        }
        if self.overlay == Some(OverlayKind::SendPanel) && self.send_panel.confirm_delete {
            match action {
                KeyAction::ConfirmYes => {
                    let deleted = self
                        .selected_send_panel_item()
                        .and_then(|item| self.delete_send_panel_item(&item));
                    self.send_panel.confirm_delete = false;
                    self.status = deleted.map_or_else(
                        || "Nothing selected to delete".into(),
                        |text| format!("Deleted {}", summarize_panel_text(&text)),
                    );
                }
                KeyAction::ConfirmNo => {
                    self.send_panel.confirm_delete = false;
                    self.status = "Delete canceled".into();
                }
                _ => {}
            }
        }
        TuiAction::None
    }

    fn execute_sessions_action(&mut self, action: KeyAction) -> TuiAction {
        if self.overlay == Some(OverlayKind::Extensions) {
            return self.execute_extensions_action(action);
        }
        match action {
            KeyAction::SessionsClose => {
                self.overlay = None;
                self.sessions.loading = false;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyAction::SessionsUp => {
                self.move_sessions_cursor(-1);
                TuiAction::None
            }
            KeyAction::SessionsDown => {
                self.move_sessions_cursor(1);
                TuiAction::None
            }
            KeyAction::SessionsSearch => {
                self.sessions.search_active = true;
                self.sessions.search.clear();
                self.refresh_session_filter();
                self.status = "Session search active".into();
                TuiAction::None
            }
            KeyAction::SessionsRefresh => {
                self.sessions.loading = true;
                self.status = "Loading sessions…".into();
                TuiAction::ListSessions
            }
            KeyAction::SessionsOpen => self.open_selected_session_action(),
            _ => TuiAction::None,
        }
    }

    fn execute_sessions_search_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::SearchClose => {
                self.sessions.search_active = false;
                self.sessions.search.clear();
                self.refresh_session_filter();
                self.status = "Session search cleared".into();
                TuiAction::None
            }
            KeyAction::SearchAccept => self.open_selected_session_action(),
            KeyAction::SearchBackspace => {
                self.sessions.search.pop();
                self.refresh_session_filter();
                self.status = session_search_status(&self.sessions.search);
                TuiAction::None
            }
            KeyAction::SearchUp => {
                self.move_sessions_cursor(-1);
                TuiAction::None
            }
            KeyAction::SearchDown => {
                self.move_sessions_cursor(1);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn execute_extensions_action(&mut self, action: KeyAction) -> TuiAction {
        if self.extension_management.remove_confirm.is_some() {
            return match action {
                KeyAction::SessionsClose => {
                    self.extension_management.remove_confirm = None;
                    self.status = "Extension uninstall canceled".into();
                    TuiAction::None
                }
                KeyAction::SessionsOpen => self.confirm_extension_package_remove(),
                _ => TuiAction::None,
            };
        }
        match action {
            KeyAction::SessionsClose => {
                self.overlay = None;
                self.extension_management.search_active = false;
                self.extension_management.install_active = false;
                self.extension_management.remove_confirm = None;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyAction::SessionsUp => {
                self.extension_management.move_cursor(-1);
                TuiAction::None
            }
            KeyAction::SessionsDown => {
                self.extension_management.move_cursor(1);
                TuiAction::None
            }
            KeyAction::SessionsSearch => {
                self.extension_management.search_active = true;
                self.extension_management.search.clear();
                self.extension_management.refresh_filter();
                self.status = "Extension search active".into();
                TuiAction::None
            }
            KeyAction::SessionsOpen => {
                self.toggle_selected_extension_scope(ToolSettingsScope::Project)
            }
            KeyAction::SessionsRefresh => {
                self.status = "Extension snapshot is already loaded".into();
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn execute_extensions_install_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::SearchClose => {
                self.extension_management.cancel_install();
                self.status = "Extension install canceled".into();
                TuiAction::None
            }
            KeyAction::SearchAccept => self.submit_extension_install(),
            KeyAction::SearchBackspace => {
                self.extension_management.install_input.pop();
                self.status = extension_install_status(&self.extension_management);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn execute_extensions_search_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::SearchClose => {
                self.extension_management.search_active = false;
                self.extension_management.search.clear();
                self.extension_management.refresh_filter();
                self.status = "Extension search cleared".into();
                TuiAction::None
            }
            KeyAction::SearchAccept => {
                self.toggle_selected_extension_scope(ToolSettingsScope::Project)
            }
            KeyAction::SearchBackspace => {
                self.extension_management.search.pop();
                self.extension_management.refresh_filter();
                self.status = extension_search_status(&self.extension_management.search);
                TuiAction::None
            }
            KeyAction::SearchUp => {
                self.extension_management.move_cursor(-1);
                TuiAction::None
            }
            KeyAction::SearchDown => {
                self.extension_management.move_cursor(1);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_extensions_install_text_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Char(ch) if key.modifiers.is_empty() => {
                self.extension_management.install_input.push(ch);
                self.status = extension_install_status(&self.extension_management);
            }
            _ => {}
        }
        TuiAction::None
    }

    fn handle_extensions_search_text_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Char(ch) if key.modifiers.is_empty() => {
                self.extension_management.search.push(ch);
                self.extension_management.refresh_filter();
                self.status = extension_search_status(&self.extension_management.search);
            }
            _ => {}
        }
        TuiAction::None
    }

    fn handle_extensions_plain_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.extension_management.remove_confirm.is_some() {
            return match key.code {
                KeyCode::Char('y') if key.modifiers.is_empty() => {
                    self.confirm_extension_package_remove()
                }
                KeyCode::Char('n') if key.modifiers.is_empty() => {
                    self.extension_management.remove_confirm = None;
                    self.status = "Extension uninstall canceled".into();
                    TuiAction::None
                }
                _ => TuiAction::None,
            };
        }
        match key.code {
            KeyCode::Tab if key.modifiers.is_empty() => self.cycle_extension_management_view(1),
            KeyCode::BackTab => self.cycle_extension_management_view(-1),
            KeyCode::Char('1') if key.modifiers.is_empty() => {
                self.set_extension_management_view(ExtensionManagementView::Manage)
            }
            KeyCode::Char('2') if key.modifiers.is_empty() => {
                self.set_extension_management_view(ExtensionManagementView::Registry)
            }
            KeyCode::Char('g') if key.modifiers.is_empty() => {
                self.toggle_selected_extension_scope(ToolSettingsScope::Global)
            }
            KeyCode::Char('p') if key.modifiers.is_empty() => {
                self.toggle_selected_extension_scope(ToolSettingsScope::Project)
            }
            KeyCode::Char('o') if key.modifiers.is_empty() => {
                self.set_selected_extension_override(ToolSettingsScope::Project)
            }
            KeyCode::Char('O')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.set_selected_extension_override(ToolSettingsScope::Global)
            }
            KeyCode::Char('c') if key.modifiers.is_empty() => {
                self.clear_selected_extension_override(ToolSettingsScope::Project)
            }
            KeyCode::Char('C')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.clear_selected_extension_override(ToolSettingsScope::Global)
            }
            KeyCode::Char('i') if key.modifiers.is_empty() => {
                self.extension_management
                    .begin_install(ToolSettingsScope::Project);
                self.status = extension_install_status(&self.extension_management);
                TuiAction::None
            }
            KeyCode::Char('I')
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                self.extension_management
                    .begin_install(ToolSettingsScope::Global);
                self.status = extension_install_status(&self.extension_management);
                TuiAction::None
            }
            KeyCode::Char('u') | KeyCode::Char('x') if key.modifiers.is_empty() => {
                self.request_extension_package_remove()
            }
            _ => TuiAction::None,
        }
    }

    fn cycle_extension_management_view(&mut self, delta: isize) -> TuiAction {
        self.extension_management.cycle_view(delta);
        self.status = format!(
            "Extensions {} tab • {} items",
            self.extension_management.view.label(),
            self.extension_management.filtered_indices.len()
        );
        TuiAction::None
    }

    fn set_extension_management_view(&mut self, view: ExtensionManagementView) -> TuiAction {
        self.extension_management.set_view(view);
        self.status = format!(
            "Extensions {} tab • {} items",
            self.extension_management.view.label(),
            self.extension_management.filtered_indices.len()
        );
        TuiAction::None
    }

    fn submit_extension_install(&mut self) -> TuiAction {
        let source = self.extension_management.install_input.trim().to_string();
        if source.is_empty() {
            self.status = "Enter a package path, Git URL, or owner/repo to install".into();
            return TuiAction::None;
        }
        let scope = self.extension_management.install_scope;
        self.extension_management.cancel_install();
        self.status = format!("Installing extension package from `{source}`…");
        TuiAction::InstallExtensionPackage { source, scope }
    }

    fn request_extension_package_remove(&mut self) -> TuiAction {
        let Some(selection) = self.extension_management.package_selection_for_selected() else {
            self.status = "Select a package row to uninstall".into();
            return TuiAction::None;
        };
        self.status = format!(
            "Uninstall {} package `{}`? Enter/Y confirms • N/Esc cancels",
            selection.scope.label(),
            selection.package_id
        );
        self.extension_management.remove_confirm = Some(selection);
        TuiAction::None
    }

    fn confirm_extension_package_remove(&mut self) -> TuiAction {
        let Some(selection) = self.extension_management.remove_confirm.take() else {
            return TuiAction::None;
        };
        self.status = format!(
            "Uninstalling {} package `{}`…",
            selection.scope.label(),
            selection.package_id
        );
        TuiAction::RemoveExtensionPackage {
            package_id: selection.package_id,
            scope: selection.scope,
        }
    }

    fn toggle_selected_extension_scope(&mut self, scope: ToolSettingsScope) -> TuiAction {
        let Some(item) = self.extension_management.selected_item().cloned() else {
            self.status = "No extension item selected".into();
            return TuiAction::None;
        };
        let enabled = !item.enabled(scope);
        self.extension_management
            .set_selected_enabled(scope, enabled);
        let status = if enabled { "enabled" } else { "disabled" };
        self.status = format!(
            "{} {} `{}` {status}",
            scope.label(),
            item.target.label(),
            item.id
        );
        TuiAction::SetExtensionEnabled {
            target: item.target,
            id: item.id,
            scope,
            enabled,
        }
    }

    fn set_selected_extension_override(&mut self, scope: ToolSettingsScope) -> TuiAction {
        let Some((contribution_id, entry_key)) =
            self.extension_management.override_selection_for_selected()
        else {
            self.status = "Select a contribution row to prefer as a conflict winner".into();
            return TuiAction::None;
        };
        self.status = format!(
            "{} conflict override: `{contribution_id}` now prefers `{entry_key}`",
            scope.label()
        );
        TuiAction::SetExtensionOverride {
            contribution_id,
            entry_key,
            scope,
        }
    }

    fn clear_selected_extension_override(&mut self, scope: ToolSettingsScope) -> TuiAction {
        let Some(item) = self.extension_management.selected_item() else {
            self.status = "No extension contribution selected".into();
            return TuiAction::None;
        };
        if item.target != ExtensionManagementTarget::Contribution {
            self.status = "Select a contribution row to clear an override".into();
            return TuiAction::None;
        }
        let contribution_id = item.canonical_id.clone().unwrap_or_else(|| item.id.clone());
        self.status = format!(
            "{} conflict override cleared for `{contribution_id}`",
            scope.label()
        );
        TuiAction::ClearExtensionOverride {
            contribution_id,
            scope,
        }
    }

    fn execute_resource_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::ResourceClose => {
                if self.overlay == Some(OverlayKind::Prompts) {
                    self.prompts.loading = false;
                } else {
                    self.skills.loading = false;
                }
                self.overlay = None;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyAction::ResourceUp => {
                if self.overlay == Some(OverlayKind::Prompts) {
                    self.move_prompt_cursor(-1);
                } else {
                    self.move_skill_cursor(-1);
                }
                TuiAction::None
            }
            KeyAction::ResourceDown => {
                if self.overlay == Some(OverlayKind::Prompts) {
                    self.move_prompt_cursor(1);
                } else {
                    self.move_skill_cursor(1);
                }
                TuiAction::None
            }
            KeyAction::ResourceSearch => {
                if self.overlay == Some(OverlayKind::Prompts) {
                    self.prompts.search_active = true;
                    self.prompts.search.clear();
                    self.refresh_prompt_filter();
                    self.status = "Prompt search active".into();
                } else {
                    self.skills.search_active = true;
                    self.skills.search.clear();
                    self.refresh_skill_filter();
                    self.status = "Skill search active".into();
                }
                TuiAction::None
            }
            KeyAction::ResourceRefresh => {
                if self.overlay == Some(OverlayKind::Prompts) {
                    self.prompts.loading = true;
                } else {
                    self.skills.loading = true;
                }
                self.status = "Reloading resources…".into();
                TuiAction::ReloadResources
            }
            KeyAction::ResourceComplete => {
                if self.overlay == Some(OverlayKind::Prompts) {
                    self.complete_selected_prompt_command();
                } else {
                    self.complete_selected_skill_command();
                }
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn execute_prompts_search_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::SearchClose => {
                self.prompts.search_active = false;
                self.prompts.search.clear();
                self.refresh_prompt_filter();
                self.status = "Prompt search cleared".into();
            }
            KeyAction::SearchAccept => self.complete_selected_prompt_command(),
            KeyAction::SearchBackspace => {
                self.prompts.search.pop();
                self.refresh_prompt_filter();
                self.status = prompt_search_status(&self.prompts.search);
            }
            KeyAction::SearchUp => self.move_prompt_cursor(-1),
            KeyAction::SearchDown => self.move_prompt_cursor(1),
            _ => {}
        }
        TuiAction::None
    }

    fn execute_skills_search_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::SearchClose => {
                self.skills.search_active = false;
                self.skills.search.clear();
                self.refresh_skill_filter();
                self.status = "Skill search cleared".into();
            }
            KeyAction::SearchAccept => self.complete_selected_skill_command(),
            KeyAction::SearchBackspace => {
                self.skills.search.pop();
                self.refresh_skill_filter();
                self.status = skill_search_status(&self.skills.search);
            }
            KeyAction::SearchUp => self.move_skill_cursor(-1),
            KeyAction::SearchDown => self.move_skill_cursor(1),
            _ => {}
        }
        TuiAction::None
    }

    fn execute_inspect_action(&mut self, action: KeyAction) -> TuiAction {
        let page = self.transcript_page_lines.max(5);
        match action {
            KeyAction::InspectClose => {
                self.overlay = None;
                self.inspect.loading = false;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyAction::InspectUp => {
                self.inspect.scroll = self.inspect.scroll.saturating_sub(1);
                TuiAction::None
            }
            KeyAction::InspectDown => {
                self.inspect.scroll = self.inspect.scroll.saturating_add(1);
                TuiAction::None
            }
            KeyAction::InspectPageUp => {
                self.inspect.scroll = self.inspect.scroll.saturating_sub(page);
                TuiAction::None
            }
            KeyAction::InspectPageDown => {
                self.inspect.scroll = self.inspect.scroll.saturating_add(page);
                TuiAction::None
            }
            KeyAction::InspectTop => {
                self.inspect.scroll = 0;
                TuiAction::None
            }
            KeyAction::InspectExportHtml => {
                self.status = "Exporting chat…".into();
                TuiAction::ExportChatHtml
            }
            _ => TuiAction::None,
        }
    }

    fn execute_settings_action(&mut self, action: KeyAction) -> TuiAction {
        let key = key_event_for_settings_action(action);
        self.handle_settings_key(key)
    }

    fn execute_extension_surface_action(&mut self, action: KeyAction) -> TuiAction {
        match action {
            KeyAction::ExtensionSurfaceFocusNext => {
                if self.focus_next_extension_surface_slot(1) {
                    self.status = "Focused next extension surface slot".into();
                } else {
                    self.status = "No extension surfaces to focus".into();
                }
            }
            KeyAction::ExtensionSurfaceFocusPrevious => {
                if self.focus_next_extension_surface_slot(-1) {
                    self.status = "Focused previous extension surface slot".into();
                } else {
                    self.status = "No extension surfaces to focus".into();
                }
            }
            KeyAction::ExtensionSurfaceTabNext => {
                if self.advance_extension_surface_tab(1) {
                    self.status = "Activated next extension surface tab".into();
                } else {
                    self.status = "No extension surface tabs to switch".into();
                }
            }
            KeyAction::ExtensionSurfaceTabPrevious => {
                if self.advance_extension_surface_tab(-1) {
                    self.status = "Activated previous extension surface tab".into();
                } else {
                    self.status = "No extension surface tabs to switch".into();
                }
            }
            KeyAction::ExtensionSurfaceClose => {
                if self.close_focused_extension_surface_slot() {
                    self.status = "Closed focused extension surface slot".into();
                } else {
                    self.status = "No focused extension surface to close".into();
                }
            }
            KeyAction::ExtensionSidebarToggle => {
                if self.toggle_extension_surface_kind(UiSurfaceKind::Sidebar) {
                    self.status = "Toggled extension sidebar slots".into();
                } else {
                    self.status = "No extension sidebar slots registered".into();
                }
            }
            KeyAction::ExtensionMainPanelToggle => {
                if self.toggle_extension_surface_kind(UiSurfaceKind::MainPanel) {
                    self.status = "Toggled extension main panel slots".into();
                } else {
                    self.status = "No extension main panel slots registered".into();
                }
            }
            _ => {}
        }
        TuiAction::None
    }

    fn handle_help_search_text_key(&mut self, key: KeyEvent) -> TuiAction {
        if let KeyCode::Char(ch) = key.code {
            if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() {
                self.help_search.push(ch);
                self.refresh_help_filter();
                self.status = help_search_status(&self.help_search);
            }
        }
        TuiAction::None
    }

    fn refresh_help_filter(&mut self) {
        let entries = help_entries(&self.settings.keymap);
        self.filtered_help_indices = fuzzy_indices(
            &entries,
            self.help_search.trim(),
            FuzzyMode::Text,
            None,
            help_entry_match_text,
        );
        self.help_scroll = self
            .help_scroll
            .min(self.filtered_help_indices.len().saturating_sub(1));
    }

    fn scroll_help_up(&mut self, lines: usize) {
        self.help_scroll = self.help_scroll.saturating_sub(lines.max(1));
    }

    fn scroll_help_down(&mut self, lines: usize) {
        self.help_scroll = self
            .help_scroll
            .saturating_add(lines.max(1))
            .min(self.filtered_help_indices.len().saturating_sub(1));
    }

    fn handle_settings_text_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.settings.page == crate::settings::SettingsPage::Models
            && self.settings.model_search_active
        {
            if let KeyCode::Char(ch) = key.code {
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() {
                    return self
                        .handle_settings_key(KeyEvent::new(KeyCode::Char(ch), key.modifiers));
                }
            }
        }
        if self.settings.page == crate::settings::SettingsPage::Theme {
            match key.code {
                KeyCode::Char('p' | 'P' | 'g' | 'G' | 'r' | 'R') if key.modifiers.is_empty() => {
                    return self.handle_settings_key(key);
                }
                _ => {}
            }
        }
        TuiAction::None
    }

    fn handle_sessions_search_text_key(&mut self, key: KeyEvent) -> TuiAction {
        if let KeyCode::Char(ch) = key.code {
            if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() {
                self.sessions.search.push(ch);
                self.refresh_session_filter();
                self.status = session_search_status(&self.sessions.search);
            }
        }
        TuiAction::None
    }

    fn handle_prompts_search_text_key(&mut self, key: KeyEvent) -> TuiAction {
        if let KeyCode::Char(ch) = key.code {
            if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() {
                self.prompts.search.push(ch);
                self.refresh_prompt_filter();
                self.status = prompt_search_status(&self.prompts.search);
            }
        }
        TuiAction::None
    }

    fn handle_skills_search_text_key(&mut self, key: KeyEvent) -> TuiAction {
        if let KeyCode::Char(ch) = key.code {
            if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() {
                self.skills.search.push(ch);
                self.refresh_skill_filter();
                self.status = skill_search_status(&self.skills.search);
            }
        }
        TuiAction::None
    }

    fn expand_prompt_references_in_composer(&mut self) -> PromptReferenceExpansionResult {
        let input = self.composer.expanded_text();
        let Some(references) = resource_references(&input) else {
            return PromptReferenceExpansionResult::NoPromptReference;
        };
        if references.prompts.is_empty() {
            return PromptReferenceExpansionResult::NoPromptReference;
        }
        if let Some(token) = references.incomplete.first() {
            return PromptReferenceExpansionResult::Incomplete(token.clone());
        }

        let mut prompts = Vec::new();
        for name in &references.prompts {
            let Some(prompt) = self
                .prompt_resources
                .iter()
                .find(|resource| resource.name == *name)
                .cloned()
            else {
                return PromptReferenceExpansionResult::UnknownPrompt(name.clone());
            };
            prompts.push(prompt);
        }

        let expanded_prompts = prompts
            .iter()
            .map(|prompt| prompt.expand(&references.user_input))
            .collect::<Vec<_>>();
        let mut expanded = String::new();
        if !references.skills.is_empty() {
            expanded.push_str(
                &references
                    .skills
                    .iter()
                    .map(|name| format!("/skill:{name}"))
                    .collect::<Vec<_>>()
                    .join(" "),
            );
            if !expanded_prompts.is_empty() {
                expanded.push_str("\n\n");
            }
        }
        expanded.push_str(&expanded_prompts.join("\n\n"));

        self.composer.replace_text(expanded);
        self.focus = TuiFocus::Composer;
        self.clear_error();
        self.refresh_command_suggestions();
        PromptReferenceExpansionResult::Expanded(prompts.len())
    }

    fn accept_command_suggestion(&mut self, submit_ready: bool) -> bool {
        let Some(item) = self
            .command_suggestions_view()
            .and_then(|view| view.selected_item().cloned())
        else {
            return false;
        };
        let should_submit = submit_ready && item.complete_on_enter;
        let replacement = if should_submit || resource_suggestion_completes_without_space(&item) {
            item.replacement.clone()
        } else {
            format!("{} ", item.replacement.trim_end())
        };
        self.composer
            .replace_char_range(item.replace_start, item.replace_end, &replacement);
        if should_submit {
            self.command_suggestions.dismiss_for(self.composer.text());
        } else {
            self.refresh_command_suggestions();
        }
        should_submit
    }

    fn after_composer_edit(&mut self, before: &str) {
        let input = self.composer.text();
        if before != input {
            self.command_suggestions
                .clear_dismissal_if_input_changed(input);
        }
        self.refresh_command_suggestions();
    }

    fn complete_selected_prompt_command(&mut self) {
        let Some(command) = self.selected_prompt_item().map(PromptResource::command) else {
            self.status = "No prompt selected".into();
            return;
        };
        self.overlay = None;
        self.composer.replace_text(command.clone());
        self.focus = TuiFocus::Composer;
        self.status = format!("Completed {command}");
        self.refresh_command_suggestions();
    }

    fn complete_selected_skill_command(&mut self) {
        let Some(command) = self.selected_skill_item().map(SkillResource::command) else {
            self.status = "No skill selected".into();
            return;
        };
        self.overlay = None;
        self.composer.replace_text(command.clone());
        self.focus = TuiFocus::Composer;
        self.status = format!("Completed {command}");
        self.refresh_command_suggestions();
    }

    fn open_selected_session_action(&mut self) -> TuiAction {
        let Some(session_id) = self
            .selected_session_item()
            .map(|session| session.id.clone())
        else {
            self.status = "No saved session selected".into();
            return TuiAction::None;
        };
        self.status = format!("Opening session {session_id}…");
        TuiAction::OpenSession(session_id)
    }

    fn handle_settings_key(&mut self, key: KeyEvent) -> TuiAction {
        match self.settings.handle_key(key) {
            SettingsAction::None => TuiAction::None,
            SettingsAction::Close => {
                self.overlay = None;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            SettingsAction::SetModel(model) => {
                self.status = format!("Model set to {model}");
                TuiAction::SetModel(model)
            }
            SettingsAction::SetThinkingLevel(level) => {
                self.status = format!(
                    "Thinking level set to {}",
                    crate::settings::thinking_label(level)
                );
                TuiAction::SetThinkingLevel(level)
            }
            SettingsAction::SetCollapseMode(target, mode) => {
                self.status = format!("Collapse mode set to {}", collapse_mode_label(mode));
                TuiAction::SetCollapseMode(target, mode)
            }
            SettingsAction::SetChatStyle(style) => {
                self.status = format!("Chat style set to {}", chat_style_label(style));
                TuiAction::SetChatStyle(style)
            }
            SettingsAction::SetKeymap(keymap) => {
                self.set_keymap(keymap.clone());
                TuiAction::SetKeymap(keymap)
            }
            SettingsAction::SetToolEnabled {
                name,
                scope,
                enabled,
            } => {
                let status = if enabled { "ON" } else { "OFF" };
                self.status = format!("{} tool `{name}` set {status}", scope.label());
                TuiAction::SetToolEnabled {
                    name,
                    scope,
                    enabled,
                }
            }
            SettingsAction::OpenExtensions => {
                self.open_extensions_overlay();
                TuiAction::None
            }
            SettingsAction::PreviewTheme { id } => {
                self.set_theme_preview(id);
                TuiAction::None
            }
            SettingsAction::ClearThemePreview => {
                self.clear_theme_preview();
                self.status = "Theme preview canceled".into();
                TuiAction::None
            }
            SettingsAction::SetTheme { id, scope } => {
                self.clear_theme_preview();
                self.status = format!("{} theme set to `{id}`", scope.label());
                TuiAction::SetTheme { id, scope }
            }
            SettingsAction::ResetTheme { scope } => {
                self.clear_theme_preview();
                self.status = format!("{} theme reset", scope.label());
                TuiAction::ResetTheme { scope }
            }
            SettingsAction::SetNotifyEnabled { scope, enabled } => {
                let status = if enabled { "enabled" } else { "disabled" };
                self.status = format!("{} notify {status}", scope.label());
                TuiAction::SetNotifyEnabled { scope, enabled }
            }
            SettingsAction::SetNotifyField {
                scope,
                field,
                value,
            } => {
                self.status = format!(
                    "{} notify {} {}",
                    scope.label(),
                    field.label(),
                    if value.as_ref().is_some_and(|value| !value.trim().is_empty()) {
                        "updated"
                    } else {
                        "cleared"
                    }
                );
                TuiAction::SetNotifyField {
                    scope,
                    field,
                    value,
                }
            }
            SettingsAction::SetNotifyEvent {
                scope,
                event,
                enabled,
            } => {
                let status = if enabled { "enabled" } else { "disabled" };
                self.status = format!("{} notify event {} {status}", scope.label(), event.label());
                TuiAction::SetNotifyEvent {
                    scope,
                    event,
                    enabled,
                }
            }
        }
    }

    fn submit_input(&mut self) -> TuiAction {
        match self.composer.submit() {
            Some(prompt) if self.working => self.submit_while_working(prompt),
            Some(prompt) => self.submit_text(prompt),
            None => TuiAction::None,
        }
    }

    fn submit_while_working(&mut self, prompt: String) -> TuiAction {
        if let Some(command) = parse_command(&prompt) {
            return self.execute_command(command);
        }

        if prompt.trim_start().starts_with('/') {
            if self.extension_command_matches_prompt(&prompt) {
                self.clear_error();
                self.status = "Running extension command…".into();
                return TuiAction::RunExtensionCommand { input: prompt };
            }
            self.error = Some(format!("Unknown command `{prompt}`"));
            self.set_calling_status();
            return TuiAction::None;
        }

        self.record_steer(prompt.clone());
        self.status = "Steering current response…".into();
        TuiAction::SteerPrompt(prompt)
    }

    fn submit_text(&mut self, prompt: String) -> TuiAction {
        if let Some(command) = parse_command(&prompt) {
            return self.execute_command(command);
        }

        if let Some(action) = self.submit_with_resource_references(&prompt) {
            return action;
        }

        if prompt.starts_with('/') {
            if self.extension_command_matches_prompt(&prompt) {
                self.clear_error();
                self.status = "Running extension command…".into();
                return TuiAction::RunExtensionCommand { input: prompt };
            }
            self.set_error(format!("Unknown command `{prompt}`"));
            self.status = HELP_STATUS.into();
            return TuiAction::None;
        }

        self.clear_error();
        self.transcript_scroll.jump_bottom();
        TuiAction::SubmitPrompt(prompt)
    }

    fn submit_with_resource_references(&mut self, input: &str) -> Option<TuiAction> {
        let references = resource_references(input)?;
        if let Some(token) = references.incomplete.first() {
            self.set_error(format!("Incomplete resource reference `{token}`"));
            self.status = HELP_STATUS.into();
            return Some(TuiAction::None);
        }

        let mut prompts = Vec::new();
        for name in &references.prompts {
            let Some(prompt) = self
                .prompt_resources
                .iter()
                .find(|resource| resource.name == *name)
                .cloned()
            else {
                self.set_error(format!("Unknown prompt `/prompt:{name}`"));
                self.status = HELP_STATUS.into();
                return Some(TuiAction::None);
            };
            prompts.push(prompt);
        }

        let mut skills = Vec::new();
        for name in &references.skills {
            let Some(skill) = self
                .skill_resources
                .iter()
                .find(|resource| resource.name == *name)
                .cloned()
            else {
                self.set_error(format!("Unknown skill `/skill:{name}`"));
                self.status = HELP_STATUS.into();
                return Some(TuiAction::None);
            };
            skills.push(skill);
        }

        let prompt = build_resource_augmented_prompt(&prompts, &skills, &references.user_input);
        self.clear_error();
        self.transcript_scroll.jump_bottom();
        self.status = resource_reference_status(prompts.len(), skills.len());
        Some(TuiAction::SubmitPrompt(prompt))
    }

    fn extension_command_matches_prompt(&self, prompt: &str) -> bool {
        let trimmed = prompt.trim();
        self.extension_commands.iter().any(|command| {
            let label = command.label.trim();
            trimmed == label
                || trimmed
                    .strip_prefix(label)
                    .is_some_and(|rest| rest.starts_with(char::is_whitespace))
        })
    }

    fn execute_command(&mut self, command: ParsedCommand) -> TuiAction {
        match command {
            ParsedCommand::Help => {
                self.open_help_overlay();
                TuiAction::None
            }
            ParsedCommand::NewSession => {
                self.clear_error();
                if !self.has_session_content() {
                    self.status = "Already in a blank session".into();
                    return TuiAction::None;
                }
                self.status = "Starting new session…".into();
                TuiAction::NewSession
            }
            ParsedCommand::Sessions => {
                self.open_sessions_overlay();
                TuiAction::ListSessions
            }
            ParsedCommand::Prompts => {
                self.open_prompts_overlay();
                TuiAction::None
            }
            ParsedCommand::Skills => {
                self.open_skills_overlay();
                TuiAction::None
            }
            ParsedCommand::ReloadResources => {
                self.prompts.loading = true;
                self.skills.loading = true;
                self.status = "Reloading resources…".into();
                TuiAction::ReloadResources
            }
            ParsedCommand::Inspect => {
                self.open_inspect_overlay();
                TuiAction::OpenInspect
            }
            ParsedCommand::Extensions => {
                self.open_extensions_overlay();
                TuiAction::None
            }
            ParsedCommand::ExtensionsUpdate => {
                self.clear_error();
                self.status = "Updating installed extension packages…".into();
                TuiAction::UpdateExtensionPackages
            }
            ParsedCommand::Compact => {
                self.clear_error();
                self.status = "Compacting session…".into();
                TuiAction::Compact
            }
            ParsedCommand::CompactMethod(method) => {
                self.clear_error();
                self.status = format!(
                    "Compacting session with {}…",
                    match &method {
                        crate::command::CompactMethodOverride::Vcc => "VCC",
                        crate::command::CompactMethodOverride::Llm => "LLM",
                    }
                );
                TuiAction::CompactMethodOverride { method }
            }
            ParsedCommand::CompactThreshold(pct) => {
                self.clear_error();
                TuiAction::CompactThreshold { pct }
            }
            ParsedCommand::CompactAuto(enabled) => {
                self.clear_error();
                TuiAction::CompactAuto { enabled }
            }
            ParsedCommand::CompactModel(model) => {
                self.clear_error();
                TuiAction::CompactModel { model }
            }
            ParsedCommand::CompactPrompt(path) => {
                self.clear_error();
                TuiAction::CompactPrompt { path }
            }
            ParsedCommand::Recall { query } => {
                self.clear_error();
                self.status = "Searching session history…".into();
                TuiAction::Recall { query }
            }
            ParsedCommand::Usage => {
                self.open_usage_overlay();
                self.set_usage_loading();
                self.status = "Refreshing usage report…".into();
                TuiAction::RefreshUsage
            }
            ParsedCommand::BtwOpen => {
                self.open_btw_overlay();
                TuiAction::OpenBtw
            }
            ParsedCommand::BtwReset => {
                self.open_btw_overlay();
                self.btw.messages.clear();
                self.btw.error = None;
                self.status = "BTW session reset".into();
                TuiAction::ResetBtwSession
            }
            ParsedCommand::BtwConfigure { model } => match model {
                None => {
                    self.status = "Usage: /model btw inherit OR /model btw <provider:model>".into();
                    TuiAction::None
                }
                Some(model) => TuiAction::ConfigureBtwModel(model),
            },
            ParsedCommand::SetNotifySummaryModel { model } => match model {
                None => {
                    self.status = "Usage: /model notify-summary inherit|off OR /model notify-summary <provider:model>".into();
                    TuiAction::None
                }
                Some(model) => TuiAction::SetNotifyField {
                    scope: ToolSettingsScope::Global,
                    field: crate::settings::NotifyField::SummaryModel,
                    value: model,
                },
            },
            ParsedCommand::AuthStatus { provider } => {
                self.open_auth_overlay();
                self.status = provider.as_ref().map_or_else(
                    || "Refreshing provider auth status…".to_string(),
                    |provider| format!("Refreshing auth status for `{provider}`…"),
                );
                TuiAction::RefreshAuthStatus { provider }
            }
            ParsedCommand::AuthQuickstart => {
                self.open_auth_overlay();
                self.status = "Showing 9router-first auth quickstart guide…".into();
                TuiAction::AuthQuickstart
            }
            ParsedCommand::Ralph(command) => {
                self.clear_error();
                self.status = "Running Ralph command…".into();
                TuiAction::Ralph(command)
            }
            ParsedCommand::ShowAgentModeUsage => {
                self.clear_error();
                self.status = "Usage: /mode plan | /mode work | /mode <profile>".into();
                TuiAction::None
            }
            ParsedCommand::SetAgentMode(mode) => {
                self.clear_error();
                self.status = format!("Switching to {} mode…", mode.label());
                TuiAction::SetAgentMode(mode)
            }
            ParsedCommand::Settings(SettingsCommand::Open) => {
                self.open_settings_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenModelSelection) => {
                self.open_model_selection_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenThinkingLevel) => {
                self.open_thinking_level_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenChatStyle) => {
                self.open_chat_style_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenTools) => {
                self.open_tools_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenAuth) => {
                self.open_auth_overlay();
                TuiAction::RefreshAuthStatus { provider: None }
            }
            ParsedCommand::Settings(SettingsCommand::OpenKeymaps) => {
                self.open_keymaps_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenTheme) => {
                self.open_theme_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenExtensions) => {
                self.open_extensions_overlay();
                TuiAction::None
            }
            ParsedCommand::Settings(SettingsCommand::OpenNotify) => {
                self.open_notify_overlay();
                TuiAction::None
            }
            ParsedCommand::SetSessionTitle(title) => {
                self.set_session_title(title.clone());
                self.status = format!("Session title set to {title}");
                self.clear_error();
                TuiAction::SetSessionTitle(title)
            }
            ParsedCommand::Settings(SettingsCommand::SetModel(model)) => {
                let identifier = model.identifier();
                self.settings.select_model_identifier(&identifier);
                self.status = format!("Model set to {identifier}");
                self.clear_error();
                TuiAction::SetModel(identifier)
            }
            ParsedCommand::Settings(SettingsCommand::SetThinkingLevel(level)) => {
                self.settings.select_thinking_level(level);
                self.status = format!(
                    "Thinking level set to {}",
                    crate::settings::thinking_label(level)
                );
                self.clear_error();
                TuiAction::SetThinkingLevel(level)
            }
            ParsedCommand::Settings(SettingsCommand::SetCollapseMode { target, mode }) => {
                self.settings.set_collapse_mode(target, mode);
                self.status = format!("Collapse mode set to {}", collapse_mode_label(mode));
                self.clear_error();
                TuiAction::SetCollapseMode(target, mode)
            }
            ParsedCommand::Settings(SettingsCommand::SetChatStyle(style)) => {
                self.settings.set_chat_style(style);
                self.status = format!("Chat style set to {}", chat_style_label(style));
                self.clear_error();
                TuiAction::SetChatStyle(style)
            }
        }
    }

    pub fn open_settings(&mut self) {
        self.open_settings_overlay();
    }

    pub fn reset_for_new_session(&mut self, session_id: &str) {
        self.messages.clear();
        self.mark_transcript_changed();
        self.clear_session_runtime_state();
        self.session_title.clear();
        self.status = format!("Started new session {session_id}");
    }

    pub fn switch_to_session(&mut self, session_id: &str, messages: &[Message]) {
        self.set_messages_from_oino(messages);
        self.clear_session_runtime_state();
        self.status = format!("Continuing session {session_id}");
    }

    fn clear_session_runtime_state(&mut self) {
        self.composer.clear();
        self.focus = TuiFocus::Composer;
        self.working = false;
        self.agent_mode = AgentMode::default();
        self.error = None;
        self.overlay = None;
        self.command_suggestions = CommandSuggestionsState::new();
        self.chord = ChordState::None;
        self.transcript_scroll.jump_bottom();
        self.send_panel = SendPanelState::default();
        self.inspect = InspectState::default();
        self.prompts = ResourceBrowserState::default();
        self.skills = ResourceBrowserState::default();
        self.refresh_prompt_filter();
        self.refresh_skill_filter();
        self.help_scroll = 0;
        self.help_search.clear();
        self.help_search_active = false;
        self.refresh_help_filter();
        self.steer_items.clear();
        self.queued_items.clear();
        self.draft_items.clear();
        self.quit_pending = false;
        self.quit_armed_at = None;
    }

    pub fn open_send_panel(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::SendPanel);
        self.send_panel.confirm_delete = false;
        self.clamp_send_panel_cursor();
        self.status = "Send panel: ↑/↓ select • q queue input • Enter load • d draft input • x delete • Esc close".into();
    }

    fn open_sessions_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Sessions);
        self.sessions.loading = true;
        self.sessions.search_active = false;
        self.sessions.search.clear();
        self.refresh_session_filter();
        self.status = "Loading sessions…".into();
    }

    fn open_prompts_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Prompts);
        self.prompts.loading = false;
        self.prompts.search_active = false;
        self.prompts.search.clear();
        self.refresh_prompt_filter();
        self.status = "Prompts".into();
    }

    fn open_skills_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Skills);
        self.skills.loading = false;
        self.skills.search_active = false;
        self.skills.search.clear();
        self.refresh_skill_filter();
        self.status = "Skills".into();
    }

    fn open_extensions_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Extensions);
        self.extension_management.search_active = false;
        self.extension_management.search.clear();
        self.extension_management.refresh_filter();
        self.status = "Extensions: Tab switch Manage/Registered • / search • i/I install • u/x uninstall • g/p toggles • o/O prefer conflict winner • c/C clear override • Esc close".into();
    }

    fn open_inspect_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Inspect);
        self.inspect.loading = true;
        self.inspect.scroll = 0;
        self.inspect.full_prompt.clear();
        self.inspect.token_count = 0;
        self.inspect.export_message = None;
        self.status = "Inspect: loading full prompt…".into();
    }

    fn open_btw_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Btw);
        self.status = "BTW: Enter send • Ctrl-Enter newline • blank /new resets • Esc close".into();
    }

    pub fn set_btw_configured_model(&mut self, model: Option<String>, current_model: &str) {
        self.btw.configured_model = model;
        self.btw.inherited = self.btw.configured_model.is_none();
        self.btw.effective_model = self
            .btw
            .configured_model
            .clone()
            .unwrap_or_else(|| current_model.to_string());
    }

    pub fn set_btw_messages_from_oino(&mut self, messages: &[Message]) {
        self.btw.messages = project_messages(messages);
        self.btw.working = false;
        self.btw.error = None;
    }

    pub fn set_btw_error(&mut self, error: impl Into<String>) {
        self.btw.working = false;
        self.btw.error = Some(error.into());
    }

    fn open_usage_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Usage);
        self.status = "Usage: ↑/↓ providers • / filter • r refresh • Esc/q close".into();
    }

    pub fn open_ask_user_overlay(&mut self, request: AskUserRequest) {
        self.clear_error();
        self.overlay = Some(OverlayKind::AskUser);
        self.ask_user = Some(AskUserOverlayState::new(request));
        self.status = "Ask user: ↑/↓ move • Space toggle • Enter select/next • c custom • t chat • Esc cancel".into();
    }

    pub fn set_inspect_full_prompt(&mut self, content: impl Into<String>, token_count: usize) {
        self.inspect.full_prompt = content.into();
        self.inspect.token_count = token_count;
        self.inspect.loading = false;
        self.inspect.scroll = 0;
        self.status = "Inspect: full prompt".into();
    }

    pub fn set_inspect_export_message(&mut self, message: impl Into<String>) {
        let message = message.into();
        self.inspect.export_message = Some(message.clone());
        self.status = message;
    }

    pub fn open_help(&mut self) {
        self.open_help_overlay();
    }

    fn open_help_overlay(&mut self) {
        self.clear_error();
        self.overlay = Some(OverlayKind::Help);
        self.help_scroll = 0;
        self.help_search.clear();
        self.help_search_active = false;
        self.refresh_help_filter();
        self.status = "Help".into();
    }

    fn open_model_selection_overlay(&mut self) {
        self.clear_error();
        self.settings.open_model_selection();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Model Selection: arrows/jk move • Enter apply • Esc back".into();
    }

    fn open_thinking_level_overlay(&mut self) {
        self.clear_error();
        self.settings.open_thinking_level();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Thinking Level: arrows/jk move • Enter apply • Esc back".into();
    }

    fn open_chat_style_overlay(&mut self) {
        self.clear_error();
        self.settings.open_chat_style();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Chat Style: arrows/jk move • Enter apply • Esc back".into();
    }

    fn open_tools_overlay(&mut self) {
        self.clear_error();
        self.settings.open_tools();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Tools: arrows/jk move • g global • p/Enter project • Esc back".into();
    }

    fn open_auth_overlay(&mut self) {
        self.clear_error();
        self.settings.open_auth();
        self.overlay = Some(OverlayKind::Settings);
        self.status =
            "Auth: arrows/jk move • recommended /9router setup • extension readiness only".into();
    }

    fn open_keymaps_overlay(&mut self) {
        self.clear_error();
        self.settings.open_keymaps();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Keymaps: Enter detail • a add in detail • p preset • Esc back".into();
    }

    fn open_theme_overlay(&mut self) {
        self.clear_error();
        self.settings.open_theme();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Theme: Enter/p project • g global • r/R reset • Esc back".into();
    }

    fn open_notify_overlay(&mut self) {
        self.clear_error();
        self.settings.open_notify();
        self.overlay = Some(OverlayKind::Settings);
        self.status =
            "Notify: ↑/↓ row • Enter edit/toggle • p project • g global • x clear • Esc back"
                .into();
    }

    fn open_settings_overlay(&mut self) {
        self.clear_error();
        self.settings.open_menu();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Settings: arrows/jk move • Enter open • Esc close".into();
    }
}

fn resource_suggestion_completes_without_space(item: &CommandSuggestionItem) -> bool {
    matches!(
        item.category,
        CommandSuggestionCategory::Prompt | CommandSuggestionCategory::Skill
    )
}

fn is_force_quit_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C') if key.modifiers.contains(KeyModifiers::CONTROL))
}

fn key_event_for_settings_action(action: KeyAction) -> KeyEvent {
    match action {
        KeyAction::SettingsClose => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
        KeyAction::SettingsBack => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
        KeyAction::SettingsOpenPage => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
        KeyAction::SettingsUp => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
        KeyAction::SettingsDown => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
        KeyAction::SettingsNext => KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE),
        KeyAction::SettingsPrevious => KeyEvent::new(KeyCode::BackTab, KeyModifiers::SHIFT),
        KeyAction::SettingsApply => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
        KeyAction::SettingsSearch => KeyEvent::new(KeyCode::Char('/'), KeyModifiers::NONE),
        KeyAction::SettingsToolToggleGlobal => {
            KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE)
        }
        KeyAction::SettingsToolToggleProject => {
            KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)
        }
        KeyAction::KeymapEditChordKey => KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE),
        KeyAction::KeymapAddShortcut => KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE),
        KeyAction::KeymapRemoveShortcut => KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE),
        KeyAction::KeymapClearShortcuts => KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE),
        KeyAction::KeymapResetAction => KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE),
        KeyAction::KeymapSelectPreset => KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE),
        _ => KeyEvent::new(KeyCode::Null, KeyModifiers::NONE),
    }
}

fn move_index(current: usize, len: usize, delta: isize) -> usize {
    if len == 0 {
        return 0;
    }
    let last = len.saturating_sub(1);
    if delta.is_negative() {
        current.saturating_sub(delta.unsigned_abs()).min(last)
    } else {
        current.saturating_add(delta as usize).min(last)
    }
}

fn filtered_session_indices(items: &[SessionListItem], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    let candidate_indices = session_filter_candidate_indices(items, query);
    fuzzy_indices(&candidate_indices, query, FuzzyMode::Path, None, |index| {
        session_match_text(&items[*index])
    })
    .into_iter()
    .map(|candidate_index| candidate_indices[candidate_index])
    .collect()
}

fn session_filter_candidate_indices(items: &[SessionListItem], query: &str) -> Vec<usize> {
    if !query.is_ascii() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter_map(|(index, session)| {
            ascii_subsequence_match_parts(
                [
                    session.name.as_str(),
                    " ",
                    session.id.as_str(),
                    " ",
                    session.preview.as_str(),
                    " ",
                    session.cwd.as_str(),
                ],
                query,
            )
            .then_some(index)
        })
        .collect()
}

fn filtered_prompt_indices(items: &[PromptResource], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    let candidate_indices = prompt_filter_candidate_indices(items, query);
    fuzzy_indices(&candidate_indices, query, FuzzyMode::Text, None, |index| {
        prompt_match_text(&items[*index])
    })
    .into_iter()
    .map(|candidate_index| candidate_indices[candidate_index])
    .collect()
}

fn prompt_filter_candidate_indices(items: &[PromptResource], query: &str) -> Vec<usize> {
    if !query.is_ascii() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter_map(|(index, prompt)| {
            ascii_subsequence_match_parts(
                [
                    prompt.name.as_str(),
                    " ",
                    prompt.description.as_str(),
                    " ",
                    prompt.source.as_str(),
                    " ",
                    prompt.scope.as_str(),
                ],
                query,
            )
            .then_some(index)
        })
        .collect()
}

fn filtered_skill_indices(items: &[SkillResource], query: &str) -> Vec<usize> {
    let query = query.trim();
    if query.is_empty() {
        return (0..items.len()).collect();
    }
    let candidate_indices = skill_filter_candidate_indices(items, query);
    fuzzy_indices(&candidate_indices, query, FuzzyMode::Text, None, |index| {
        skill_match_text(&items[*index])
    })
    .into_iter()
    .map(|candidate_index| candidate_indices[candidate_index])
    .collect()
}

fn skill_filter_candidate_indices(items: &[SkillResource], query: &str) -> Vec<usize> {
    if !query.is_ascii() {
        return (0..items.len()).collect();
    }
    items
        .iter()
        .enumerate()
        .filter_map(|(index, skill)| {
            ascii_subsequence_match_parts(
                [
                    skill.name.as_str(),
                    " ",
                    skill.description.as_str(),
                    " ",
                    skill.source.as_str(),
                    " ",
                    skill.scope.as_str(),
                ],
                query,
            )
            .then_some(index)
        })
        .collect()
}

fn session_match_text(session: &SessionListItem) -> String {
    format!(
        "{} {} {} {}",
        session.name, session.id, session.preview, session.cwd
    )
}

fn prompt_match_text(prompt: &PromptResource) -> String {
    format!(
        "{} {} {} {}",
        prompt.name, prompt.description, prompt.source, prompt.scope
    )
}

fn skill_match_text(skill: &SkillResource) -> String {
    format!(
        "{} {} {} {}",
        skill.name, skill.description, skill.source, skill.scope
    )
}

fn session_search_status(query: &str) -> String {
    if query.is_empty() {
        "Session search active".into()
    } else {
        format!("Searching sessions for `{query}`")
    }
}

fn usage_search_status(query: &str) -> String {
    if query.is_empty() {
        "Usage search active".into()
    } else {
        format!("Searching usage providers for `{query}`")
    }
}

fn usage_panel_status(usage: &UsagePanelState) -> String {
    if usage.loading {
        return "Refreshing usage report…".into();
    }
    if let Some(error) = &usage.error {
        return format!("Usage error: {error}");
    }
    if let Some(provider) = usage.selected_provider() {
        return format!("Usage: {} — {}", provider.display_name, provider.message);
    }
    usage.report.as_ref().map_or_else(
        || "Usage: no report loaded".into(),
        |report| report.status_line.clone(),
    )
}

fn prompt_search_status(query: &str) -> String {
    if query.is_empty() {
        "Prompt search active".into()
    } else {
        format!("Searching prompts for `{query}`")
    }
}

fn extension_search_status(query: &str) -> String {
    if query.is_empty() {
        "Extension search active".into()
    } else {
        format!("Searching extensions for `{query}`")
    }
}

fn extension_install_status(management: &ExtensionManagementState) -> String {
    let input = if management.install_input.is_empty() {
        "<package path, Git URL, or owner/repo>"
    } else {
        &management.install_input
    };
    format!(
        "Install {} extension package from {input} • Enter confirms • Esc cancels",
        management.install_scope.label()
    )
}

fn skill_search_status(query: &str) -> String {
    if query.is_empty() {
        "Skill search active".into()
    } else {
        format!("Searching skills for `{query}`")
    }
}

fn help_search_status(query: &str) -> String {
    if query.is_empty() {
        "Help search active".into()
    } else {
        format!("Searching help for `{query}`")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum PromptReferenceExpansionResult {
    Expanded(usize),
    NoPromptReference,
    Incomplete(String),
    UnknownPrompt(String),
}

#[derive(Debug, Default, PartialEq, Eq)]
struct ResourceReferences {
    prompts: Vec<String>,
    skills: Vec<String>,
    incomplete: Vec<String>,
    user_input: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResourceReferenceKind {
    Prompt,
    Skill,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ByteToken<'a> {
    text: &'a str,
    start: usize,
    end: usize,
}

fn resource_references(input: &str) -> Option<ResourceReferences> {
    let tokens = byte_tokens(input);
    let mut references = ResourceReferences::default();
    let mut found = false;
    let mut stripped = String::new();
    let mut copied_until = 0;

    for token in tokens {
        let Some((kind, name)) = resource_reference_token(token.text) else {
            continue;
        };
        found = true;
        stripped.push_str(&input[copied_until..token.start]);
        copied_until = token.end;

        if name.is_empty() {
            references.incomplete.push(token.text.to_string());
            continue;
        }
        match kind {
            ResourceReferenceKind::Prompt => push_unique(&mut references.prompts, name),
            ResourceReferenceKind::Skill => push_unique(&mut references.skills, name),
        }
    }

    if !found {
        return None;
    }

    stripped.push_str(&input[copied_until..]);
    references.user_input = clean_resource_user_input(&stripped);
    Some(references)
}

fn resource_reference_token(token: &str) -> Option<(ResourceReferenceKind, String)> {
    if let Some(name) = token.strip_prefix("/prompt:") {
        return Some((ResourceReferenceKind::Prompt, name.to_string()));
    }
    token
        .strip_prefix("/skill:")
        .map(|name| (ResourceReferenceKind::Skill, name.to_string()))
}

fn push_unique(items: &mut Vec<String>, value: String) {
    if !items.iter().any(|item| item == &value) {
        items.push(value);
    }
}

fn byte_tokens(input: &str) -> Vec<ByteToken<'_>> {
    let mut tokens = Vec::new();
    let mut start = None;
    for (index, ch) in input.char_indices() {
        if ch.is_whitespace() {
            if let Some(token_start) = start.take() {
                tokens.push(ByteToken {
                    text: &input[token_start..index],
                    start: token_start,
                    end: index,
                });
            }
        } else if start.is_none() {
            start = Some(index);
        }
    }
    if let Some(token_start) = start {
        tokens.push(ByteToken {
            text: &input[token_start..],
            start: token_start,
            end: input.len(),
        });
    }
    tokens
}

fn clean_resource_user_input(input: &str) -> String {
    input
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

fn build_resource_augmented_prompt(
    prompts: &[PromptResource],
    skills: &[SkillResource],
    user_input: &str,
) -> String {
    if skills.is_empty() {
        let expanded = prompts
            .iter()
            .map(|prompt| prompt.expand(user_input))
            .collect::<Vec<_>>();
        return expanded.join("\n\n");
    }

    let mut output = String::from("Use the following Oino resources for this request.");
    if !prompts.is_empty() {
        output.push_str("\n\n# Included Prompt Templates");
        for prompt in prompts {
            output.push_str("\n\n");
            output.push_str(&markdown_resource_block(
                "Prompt",
                &prompt.name,
                &prompt.source,
                &prompt.expand(user_input),
            ));
        }
    }
    if !skills.is_empty() {
        output.push_str("\n\n# Included Skills");
        for skill in skills {
            output.push_str("\n\n");
            output.push_str(&markdown_resource_block(
                "Skill",
                &skill.name,
                &skill.source,
                &skill.content,
            ));
        }
    }
    if !user_input.is_empty() {
        output.push_str("\n\n# User Request\n\n");
        output.push_str(user_input);
    }
    output
}

fn markdown_resource_block(kind: &str, name: &str, source: &str, content: &str) -> String {
    format!(
        "## Included {kind}: `{name}`\nSource: `{source}`\n\n{}",
        fenced_markdown(content)
    )
}

fn fenced_markdown(content: &str) -> String {
    let fence = "`".repeat(longest_backtick_run(content).saturating_add(1).max(4));
    format!("{fence}markdown\n{}\n{fence}", content.trim_end())
}

fn longest_backtick_run(content: &str) -> usize {
    let mut longest = 0;
    let mut current = 0;
    for ch in content.chars() {
        if ch == '`' {
            current += 1;
            longest = longest.max(current);
        } else {
            current = 0;
        }
    }
    longest
}

fn resource_reference_status(prompt_count: usize, skill_count: usize) -> String {
    match (prompt_count, skill_count) {
        (0, 0) => "No Oino resources included".into(),
        (prompts, 0) => format!("Included {prompts} prompt resource(s)"),
        (0, skills) => format!("Included {skills} skill resource(s)"),
        (prompts, skills) => {
            format!("Included {prompts} prompt resource(s) and {skills} skill resource(s)")
        }
    }
}

fn provider_label(model: &str) -> String {
    let provider = model
        .split_once(':')
        .map_or(model, |(provider, _)| provider);
    match provider.to_ascii_lowercase().as_str() {
        "openrouter" => "OpenRouter".into(),
        "openai" => "OpenAI".into(),
        "" => "model".into(),
        other => {
            let mut chars = other.chars();
            match chars.next() {
                Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
                None => "model".into(),
            }
        }
    }
}

fn summarize_panel_text(text: &str) -> String {
    let first_line = text.lines().next().unwrap_or_default().trim();
    let summary = if first_line.is_empty() {
        text.trim()
    } else {
        first_line
    };
    let chars = summary.chars().collect::<Vec<_>>();
    if chars.len() <= 48 {
        format!("`{summary}`")
    } else {
        format!("`{}…`", chars.into_iter().take(47).collect::<String>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    fn test_surface(
        id: &str,
        kind: UiSurfaceKind,
        title: &str,
        slot: &str,
    ) -> ActiveContribution<UiSurfaceContribution> {
        use oino_extension_core::{
            ContributionMetadata, ExtensionId, RegistryEntry, RegistryEntryKey, SourceDescriptor,
            SourceKind, SourceScope, UiFocusPolicy, UiKeyDispatchPolicy, UiLayoutPolicy,
            UiTinyTerminalFallback, UiVisibilityPolicy,
        };

        let contribution_id = ContributionId::new(id).unwrap_or_else(|err| panic!("bad id: {err}"));
        let owner = ExtensionId::new(format!("owner.{id}"))
            .unwrap_or_else(|err| panic!("bad owner id: {err}"));
        ActiveContribution {
            effective_id: contribution_id.clone(),
            entry: RegistryEntry::new(
                RegistryEntryKey::new(format!("test:{id}")),
                ContributionMetadata::new(
                    contribution_id.clone(),
                    SourceDescriptor {
                        scope: SourceScope::Project,
                        kind: SourceKind::LocalPackage,
                        path: None,
                        registry: None,
                    },
                )
                .with_extension_id(owner),
                UiSurfaceContribution {
                    id: contribution_id,
                    surface: kind,
                    title: title.into(),
                    state_schema: Some("object".into()),
                    layout: UiLayoutPolicy {
                        slot: slot.into(),
                        priority: 0,
                        min_width: 20,
                        min_height: 3,
                        max_width: Some(32),
                        tiny_terminal: UiTinyTerminalFallback::CompactBadge,
                    },
                    visibility: UiVisibilityPolicy::Visible,
                    focus: UiFocusPolicy::Focusable,
                    key_dispatch: UiKeyDispatchPolicy::default(),
                    conflict: Default::default(),
                },
            ),
        }
    }

    fn add_test_resources(state: &mut TuiState) {
        state.set_resources(
            vec![PromptResource {
                name: "review".into(),
                description: "Review current changes".into(),
                argument_hint: Some("[focus]".into()),
                source: ".oino/prompts/review.md".into(),
                scope: "project".into(),
                content: "Review $ARGUMENTS".into(),
            }],
            vec![SkillResource {
                name: "debug".into(),
                description: "Investigate bugs".into(),
                source: ".oino/skills/debug/SKILL.md".into(),
                scope: "project".into(),
                content: "# Debug Skill".into(),
            }],
            Vec::new(),
        );
    }

    #[test]
    fn state_submits_non_empty_composer() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Char('h'))), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Char('i'))), TuiAction::None);
        assert_eq!(state.input(), "hi");
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SubmitPrompt("hi".into())
        );
        assert_eq!(state.input(), "");
    }

    #[test]
    fn extension_shortcuts_dispatch_and_expose_conflicts() {
        let mut state = TuiState::new();
        let active = "alt-x"
            .parse::<KeySequence>()
            .unwrap_or_else(|err| panic!("shortcut parse failed: {err}"));
        let conflicting = "ctrl-c"
            .parse::<KeySequence>()
            .unwrap_or_else(|err| panic!("shortcut parse failed: {err}"));
        assert!(matches!(
            state
                .settings
                .keymap
                .resolve(&[KeyContext::Global], conflicting.strokes()),
            KeymapMatch::Matched(_)
        ));
        state.set_extension_shortcuts(vec![
            ExtensionShortcut::new("extension.process.stop", active, "process-manager"),
            ExtensionShortcut::new("extension.conflict", conflicting, "process-manager"),
        ]);

        assert!(state
            .extension_ui
            .shortcuts
            .iter()
            .any(|shortcut| shortcut.action == "extension.conflict"
                && !shortcut.conflicts.is_empty()));
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::ALT)),
            TuiAction::RunExtensionAction {
                action: "extension.process.stop".into(),
            }
        );
    }

    #[test]
    fn extension_chord_shortcuts_continue_after_builtin_chord_prefix() {
        let mut state = TuiState::new();
        let shortcut = "ctrl-o x"
            .parse::<KeySequence>()
            .unwrap_or_else(|err| panic!("shortcut parse failed: {err}"));
        state.set_extension_shortcuts(vec![ExtensionShortcut::new(
            "extension.example.action",
            shortcut,
            "example-extension",
        )]);

        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.chord, ChordState::CtrlO);
        assert_eq!(
            state.handle_key(key(KeyCode::Char('x'))),
            TuiAction::RunExtensionAction {
                action: "extension.example.action".into(),
            }
        );
        assert!(state.key_sequence.is_empty());
        assert_eq!(state.chord, ChordState::None);
    }

    #[test]
    fn extension_shortcuts_do_not_hijack_extension_overlay_chords() {
        let mut state = TuiState::new();
        let shortcut = "ctrl-o x"
            .parse::<KeySequence>()
            .unwrap_or_else(|err| panic!("shortcut parse failed: {err}"));
        state.set_extension_shortcuts(vec![ExtensionShortcut::new(
            "extension.example.action",
            shortcut,
            "example-extension",
        )]);
        state.composer.replace_text("/extensions");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Extensions));

        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert!(state.key_sequence.is_empty());
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(
            state.extension_management.view,
            ExtensionManagementView::Registry
        );
        assert_ne!(state.status, "Unknown key chord");
    }

    #[test]
    fn extension_surface_controller_tabs_toggles_and_closes_slots() {
        let mut state = TuiState::new();
        state.set_extension_ui_surfaces(vec![
            test_surface("ui.one", UiSurfaceKind::Sidebar, "One", "sidebar:right"),
            test_surface("ui.two", UiSurfaceKind::Sidebar, "Two", "sidebar:right"),
            test_surface("ui.main", UiSurfaceKind::MainPanel, "Main", "main:primary"),
            test_surface(
                "ui.float",
                UiSurfaceKind::FloatingPanel,
                "Float",
                "floating:center",
            ),
        ]);

        assert!(state.focus_next_extension_surface_slot(1));
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .focused_slot
                .as_deref(),
            Some("FloatingPanel:floating:center")
        );
        assert!(state.focus_next_extension_surface_slot(1));
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .focused_slot
                .as_deref(),
            Some("MainPanel:main:primary")
        );
        assert!(state.focus_next_extension_surface_slot(-1));
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .focused_slot
                .as_deref(),
            Some("FloatingPanel:floating:center")
        );
        assert!(state.focus_next_extension_surface_slot(-1));
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .focused_slot
                .as_deref(),
            Some("Sidebar:sidebar:right")
        );
        assert!(state.advance_extension_surface_tab(1));
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .active_tab("Sidebar:sidebar:right", 2),
            1
        );
        assert!(state.advance_extension_surface_tab(1));
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .active_tab("Sidebar:sidebar:right", 2),
            0
        );
        assert!(state.close_focused_extension_surface_slot());
        assert!(state
            .extension_ui
            .surface_controller
            .is_slot_hidden("Sidebar:sidebar:right"));
        assert!(state.toggle_extension_surface_kind(UiSurfaceKind::Sidebar));
        assert!(!state
            .extension_ui
            .surface_controller
            .is_slot_hidden("Sidebar:sidebar:right"));

        state.extension_ui.surface_controller.focused_slot = None;
        state.extension_ui.focused_surface = None;
        assert!(state.close_focused_extension_surface_slot());
        assert!(state
            .extension_ui
            .surface_controller
            .is_slot_hidden("FloatingPanel:floating:center"));
    }

    #[test]
    fn extension_surface_keybindings_use_global_chord_controls() {
        let mut state = TuiState::new();
        state.set_extension_ui_surfaces(vec![
            test_surface("ui.one", UiSurfaceKind::Sidebar, "One", "sidebar:right"),
            test_surface("ui.two", UiSurfaceKind::Sidebar, "Two", "sidebar:right"),
        ]);

        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .focused_slot
                .as_deref(),
            Some("Sidebar:sidebar:right")
        );
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Char(']'))), TuiAction::None);
        assert_eq!(
            state
                .extension_ui
                .surface_controller
                .active_tab("Sidebar:sidebar:right", 2),
            1
        );
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert!(state
            .extension_ui
            .surface_controller
            .is_slot_hidden("Sidebar:sidebar:right"));
    }

    #[test]
    fn extension_autosuggest_refreshes_cached_suggestions() {
        let mut state = TuiState::new();
        state.set_extension_autosuggest_items(vec![ExtensionAutosuggestItem {
            label: "process:list".into(),
            summary: "List managed processes".into(),
            replacement: "#process:list".into(),
            trigger: "#".into(),
            source: "process-manager".into(),
        }]);
        state.insert_literal("#pro");
        let suggestions = state
            .command_suggestions_view()
            .unwrap_or_else(|| panic!("missing extension suggestions"));
        assert_eq!(suggestions.title, "Extension Suggestions");
        assert_eq!(suggestions.items[0].label, "process:list");
        assert_eq!(
            suggestions.items[0].category,
            CommandSuggestionCategory::Extension
        );
    }

    #[test]
    fn extensions_overlay_searches_and_toggles_project_policy() {
        let mut state = TuiState::new();
        state.set_extension_management_items(vec![
            ExtensionManagementItem {
                target: ExtensionManagementTarget::Extension,
                id: "process.manager".into(),
                title: "Process Manager".into(),
                family: "extension".into(),
                scope: "project".into(),
                health: "Healthy".into(),
                state: "Active".into(),
                permission: "tools:1".into(),
                provenance: "process.package process.manager".into(),
                diagnostics: Vec::new(),
                conflicts: Vec::new(),
                entry_key: None,
                canonical_id: None,
                global_override: false,
                project_override: false,
                global_enabled: true,
                project_enabled: true,
            },
            ExtensionManagementItem {
                target: ExtensionManagementTarget::Package,
                id: "process.package".into(),
                title: "Process Package".into(),
                family: "package".into(),
                scope: "project".into(),
                health: "Healthy".into(),
                state: "Active".into(),
                permission: "package".into(),
                provenance: String::new(),
                diagnostics: Vec::new(),
                conflicts: Vec::new(),
                entry_key: None,
                canonical_id: None,
                global_override: false,
                project_override: false,
                global_enabled: false,
                project_enabled: true,
            },
        ]);
        state.composer.replace_text("/extensions");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Extensions));
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE)),
            TuiAction::SetExtensionEnabled {
                target: ExtensionManagementTarget::Package,
                id: "process.package".into(),
                scope: ToolSettingsScope::Project,
                enabled: false,
            }
        );
        assert!(!state.extension_management.items[1].project_enabled);
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(
            state.extension_management.view,
            ExtensionManagementView::Registry
        );
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(
            state.extension_management.view,
            ExtensionManagementView::Manage
        );

        assert_eq!(state.handle_key(key(KeyCode::Char('i'))), TuiAction::None);
        for ch in "examples/extensions/rust-wasm-fixture".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::InstallExtensionPackage {
                source: "examples/extensions/rust-wasm-fixture".into(),
                scope: ToolSettingsScope::Project,
            }
        );

        assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Char('u'))), TuiAction::None);
        assert!(state.extension_management.remove_confirm.is_some());
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::RemoveExtensionPackage {
                package_id: "process.package".into(),
                scope: ToolSettingsScope::Project,
            }
        );
    }

    #[test]
    fn settings_menu_can_open_extension_manager() {
        let mut state = TuiState::new();
        state.open_settings();
        for _ in 0..state.settings.menu_items().len() {
            if state.settings.current_menu_item() == crate::settings::SettingsMenuItem::Extensions {
                break;
            }
            assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        }

        assert_eq!(
            state.settings.current_menu_item(),
            crate::settings::SettingsMenuItem::Extensions
        );
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Extensions));
    }

    #[test]
    fn settings_extensions_command_opens_extension_manager() {
        let mut state = TuiState::new();
        state.composer.replace_text("/settings extensions");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Extensions));
    }

    #[test]
    fn theme_settings_page_sets_project_and_global_theme() {
        let mut state = TuiState::new();
        state.composer.replace_text("/theme");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert_eq!(state.settings.page, crate::settings::SettingsPage::Theme);
        assert!(!state.settings.theme_options.is_empty());

        assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        let selected = state.settings.theme_options[state.settings.theme_cursor]
            .id
            .clone();
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(
            state.preview_theme.as_ref().map(|theme| theme.id.as_str()),
            Some(selected.as_str())
        );
        assert_eq!(state.settings.preview_theme_id(), Some(selected.as_str()));
        assert_eq!(
            state.handle_key(key(KeyCode::Char('p'))),
            TuiAction::SetTheme {
                id: selected.clone(),
                scope: ToolSettingsScope::Project,
            }
        );
        assert!(state.preview_theme.is_none());
        assert_eq!(
            state.handle_key(key(KeyCode::Char('g'))),
            TuiAction::SetTheme {
                id: selected,
                scope: ToolSettingsScope::Global,
            }
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('r'))),
            TuiAction::ResetTheme {
                scope: ToolSettingsScope::Project,
            }
        );
    }

    #[test]
    fn set_theme_settings_updates_effective_theme_and_picker_state() {
        let mut state = TuiState::new();
        let mut global = ThemeSettings::default();
        global.set_active("oino-light");
        let mut project = ThemeSettings::default();
        project.set_active("oino-aurora");
        state.set_theme_settings(&global, &project);

        assert_eq!(state.resolved_theme.id, "oino-aurora");
        assert_eq!(
            state.resolved_theme.selected_scope,
            crate::theme::EffectiveThemeScope::Project
        );
        let selected = &state.settings.theme_options[state.settings.theme_cursor];
        assert_eq!(selected.id, "oino-aurora");
        assert!(selected.project_active);
        assert!(selected.effective);
    }

    #[test]
    fn theme_preview_clears_when_leaving_theme_page() {
        let mut state = TuiState::new();
        state.composer.replace_text("/theme");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert!(state.preview_theme.is_some());
        assert!(state.settings.preview_theme.is_some());
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert!(state.preview_theme.is_none());
        assert!(state.settings.preview_theme.is_none());
        assert_eq!(state.settings.page, crate::settings::SettingsPage::Menu);
    }

    #[test]
    fn extensions_overlay_sets_and_clears_conflict_overrides() {
        let mut state = TuiState::new();
        state.set_extension_management_items(vec![ExtensionManagementItem {
            target: ExtensionManagementTarget::Contribution,
            id: "acme.example.command".into(),
            title: "command:acme:example:/tmp".into(),
            family: "command".into(),
            scope: "project".into(),
            health: "Degraded".into(),
            state: "Shadowed".into(),
            permission: "granted".into(),
            provenance: "acme.example acme.example".into(),
            diagnostics: Vec::new(),
            conflicts: vec!["duplicate command".into()],
            entry_key: Some("command:acme:example:/tmp".into()),
            canonical_id: Some("example".into()),
            global_override: false,
            project_override: false,
            global_enabled: true,
            project_enabled: true,
        }]);
        state.composer.replace_text("/extensions");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(
            state.extension_management.view,
            ExtensionManagementView::Registry
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('o'))),
            TuiAction::SetExtensionOverride {
                contribution_id: "example".into(),
                entry_key: "command:acme:example:/tmp".into(),
                scope: ToolSettingsScope::Project,
            }
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Char('c'))),
            TuiAction::ClearExtensionOverride {
                contribution_id: "example".into(),
                scope: ToolSettingsScope::Project,
            }
        );
    }

    #[test]
    fn inspect_command_opens_loading_overlay() {
        let mut state = TuiState::new();
        state.composer.replace_text("/inspect");

        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::OpenInspect
        );
        assert_eq!(state.overlay, Some(OverlayKind::Inspect));
        assert!(state.inspect.loading);
    }

    #[test]
    fn inspect_panel_e_exports_chat_html() {
        let mut state = TuiState::new();
        state.composer.replace_text("/inspect");
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::OpenInspect
        );

        assert_eq!(
            state.handle_key(key(KeyCode::Char('e'))),
            TuiAction::ExportChatHtml
        );
        assert_eq!(state.status, "Exporting chat…");
    }

    #[test]
    fn inspect_snapshot_updates_full_prompt_and_token_count() {
        let mut state = TuiState::new();
        state.set_inspect_full_prompt("# Full\nPrompt", 2);

        assert!(!state.inspect.loading);
        assert_eq!(state.inspect.token_count, 2);
        assert_eq!(state.inspect.full_prompt, "# Full\nPrompt");
    }

    #[test]
    fn assistant_stream_title_uses_current_model() {
        let mut state = TuiState::with_settings("test/model", oino_types::ThinkingLevel::Off);
        state.start_message(oino_types::OinoId::nil(), "assistant");
        assert_eq!(state.messages[0].role, "assistant");
        assert_eq!(state.messages[0].title.as_deref(), Some("test/model"));
    }

    #[test]
    fn update_message_preserves_streamed_thinking_section() {
        let mut state = TuiState::new();
        state.update_message(
            oino_types::OinoId::nil(),
            &[
                ContentBlock::Thinking {
                    text: "streamed thought".into(),
                    redacted: false,
                },
                ContentBlock::Text {
                    text: "answer".into(),
                },
            ],
        );
        assert_eq!(state.messages[0].content, "answer");
        assert_eq!(
            state.messages[0].thinking.as_deref(),
            Some("streamed thought")
        );
    }

    #[test]
    fn transcript_version_tracks_message_changes() {
        let mut state = TuiState::new();
        let initial = state.transcript_version();
        let id = oino_types::OinoId::nil();

        state.start_message(id, "assistant");
        assert!(state.transcript_version() > initial);
        let after_start = state.transcript_version();

        state.update_message(
            id,
            &[ContentBlock::Text {
                text: "answer".into(),
            }],
        );
        assert!(state.transcript_version() > after_start);
        let after_update = state.transcript_version();

        state.update_message(
            id,
            &[ContentBlock::Text {
                text: "answer".into(),
            }],
        );
        assert_eq!(state.transcript_version(), after_update);

        state.reset_for_new_session("next");
        assert!(state.transcript_version() > after_update);
    }

    #[test]
    fn working_state_accepts_input_and_enter_steers() {
        let mut state = TuiState::with_settings("openrouter:test/model", ThinkingLevel::Off);
        state.set_working(true);
        assert_eq!(state.handle_key(key(KeyCode::Char('x'))), TuiAction::None);
        assert_eq!(state.input(), "x");
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SteerPrompt("x".into())
        );
        assert_eq!(state.input(), "");
        assert_eq!(state.steer_items, vec!["x".to_string()]);
        assert_eq!(state.handle_paste("pasted"), TuiAction::None);
        assert_eq!(state.input(), "pasted");
        assert!(state.status.contains("Steering") || state.status.contains("Calling"));
    }

    #[test]
    fn working_state_runs_slash_commands_instead_of_steering_them() {
        let mut state = TuiState::with_settings("openrouter:test/model", ThinkingLevel::Off);
        state.set_working(true);
        for ch in "/settings".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert!(state.working);
        assert!(state.steer_items.is_empty());
    }

    #[test]
    fn pasted_newlines_insert_without_submitting() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_paste("first\nsecond"), TuiAction::None);
        assert_eq!(state.input(), "first\nsecond");
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SubmitPrompt("first\nsecond".into())
        );
    }

    #[test]
    fn large_paste_collapses_but_submits_full_text() {
        let mut state = TuiState::new();
        let pasted = (0..10)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(state.handle_paste(&pasted), TuiAction::None);
        assert!(state.input().contains("pasted 10 lines"));
        assert!(!state.input().contains("line 9"));
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SubmitPrompt(pasted)
        );
    }

    #[test]
    fn ctrl_o_e_expands_collapsed_paste_at_cursor() {
        let mut state = TuiState::new();
        let pasted = (0..8)
            .map(|index| format!("line {index}"))
            .collect::<Vec<_>>()
            .join("\n");

        assert_eq!(state.handle_paste(&pasted), TuiAction::None);
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Char('e'))), TuiAction::None);
        assert_eq!(state.input(), pasted);
    }

    #[test]
    fn ctrl_o_e_expands_prompt_references_in_composer() {
        let mut state = TuiState::new();
        add_test_resources(&mut state);
        for ch in "/prompt:review bugs security".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Char('e'))), TuiAction::None);
        assert_eq!(state.input(), "Review bugs security");
        assert!(state.status.contains("Expanded 1 prompt template"));
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SubmitPrompt("Review bugs security".into())
        );
    }

    #[test]
    fn ctrl_o_e_preserves_skill_references_when_expanding_prompts() {
        let mut state = TuiState::new();
        add_test_resources(&mut state);
        for ch in "/skill:debug /prompt:review crash".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Char('e'))), TuiAction::None);
        assert_eq!(state.input(), "/skill:debug\n\nReview crash");
        match state.handle_key(key(KeyCode::Enter)) {
            TuiAction::SubmitPrompt(prompt) => {
                assert!(prompt.contains("## Included Skill: `debug`"));
                assert!(prompt.contains("# User Request\n\nReview crash"));
            }
            other => panic!("expected skill submit, got {other:?}"),
        }
    }

    #[test]
    fn oversized_paste_is_rejected() {
        let mut state = TuiState::new();
        let pasted = "x".repeat(MAX_PASTE_CHARS + 1);

        assert_eq!(state.handle_paste(&pasted), TuiAction::None);
        assert_eq!(state.input(), "");
        assert!(state.status.contains("Paste rejected"));
    }

    #[test]
    fn ctrl_o_s_opens_settings_and_ctrl_o_q_opens_send_panel() {
        let mut state = TuiState::new();
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.chord, ChordState::CtrlO);
        assert_eq!(state.handle_key(key(KeyCode::Char('s'))), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert_eq!(state.chord, ChordState::None);

        state.overlay = None;
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Char('q'))), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::SendPanel));
        assert_eq!(state.chord, ChordState::None);
    }

    #[test]
    fn chord_enter_queues_and_chord_slash_drafts_composer_input() {
        let mut state = TuiState::new();
        state.composer.replace_text("next task");
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::QueuePrompt("next task".into())
        );
        assert_eq!(state.input(), "");
        assert_eq!(state.queued_items, vec!["next task".to_string()]);

        state.composer.replace_text("draft this");
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert_eq!(state.input(), "");
        assert_eq!(state.draft_items, vec!["draft this".to_string()]);
    }

    #[test]
    fn send_panel_q_queues_current_input() {
        let mut state = TuiState::new();
        state.composer.replace_text("next task");
        state.open_send_panel();
        assert_eq!(
            state.handle_key(key(KeyCode::Char('q'))),
            TuiAction::QueuePrompt("next task".into())
        );
        assert_eq!(state.input(), "");
        assert_eq!(state.queued_items, vec!["next task".to_string()]);
    }

    #[test]
    fn send_panel_drafts_current_input_and_enter_loads_selection() {
        let mut state = TuiState::new();
        state.composer.replace_text("draft me");
        state.open_send_panel();
        assert_eq!(state.handle_key(key(KeyCode::Char('d'))), TuiAction::None);
        assert_eq!(state.input(), "");
        assert_eq!(state.draft_items, vec!["draft me".to_string()]);

        state.composer.replace_text("keep this safe");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.input(), "draft me");
        assert_eq!(state.draft_items, vec!["keep this safe".to_string()]);
    }

    #[test]
    fn send_panel_x_confirms_delete_with_y() {
        let mut state = TuiState::new();
        state.draft_items.push("old draft".into());
        state.open_send_panel();
        assert_eq!(state.handle_key(key(KeyCode::Char('x'))), TuiAction::None);
        assert!(state.send_panel.confirm_delete);
        assert!(state.status.contains("Press y"));
        assert_eq!(state.handle_key(key(KeyCode::Char('y'))), TuiAction::None);
        assert!(!state.send_panel.confirm_delete);
        assert!(state.draft_items.is_empty());
    }

    #[test]
    fn overlay_prompts_do_not_leak_into_main_activity_status() {
        let mut state = TuiState::new();
        state.set_working(true);
        state.draft_items.push("old draft".into());
        state.open_send_panel();
        assert_eq!(state.handle_key(key(KeyCode::Char('x'))), TuiAction::None);

        assert_eq!(state.activity_status(), None);
        assert!(state.status.contains("Press y"));
    }

    #[test]
    fn escape_exits_ctrl_o_chord_mode() {
        let mut state = TuiState::new();
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.chord, ChordState::CtrlO);
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert_eq!(state.chord, ChordState::None);
        assert_eq!(state.overlay, None);
    }

    #[test]
    fn transcript_scroll_keys_do_not_edit_composer() {
        let mut state = TuiState::new();
        state.set_transcript_page_lines(6);

        assert_eq!(state.handle_key(key(KeyCode::PageUp)), TuiAction::None);
        assert_eq!(state.transcript_scroll.offset_from_bottom(), 5);
        assert_eq!(state.input(), "");

        assert_eq!(state.handle_key(key(KeyCode::PageDown)), TuiAction::None);
        assert_eq!(state.transcript_scroll.offset_from_bottom(), 0);

        state.composer.replace_text("draft");
        assert_eq!(state.handle_key(key(KeyCode::PageUp)), TuiAction::None);
        assert_eq!(state.transcript_scroll.offset_from_bottom(), 5);
        assert_eq!(state.input(), "draft");
    }

    #[test]
    fn empty_composer_arrows_scroll_transcript() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Up)), TuiAction::None);
        assert_eq!(state.transcript_scroll.offset_from_bottom(), 1);
        assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        assert_eq!(state.transcript_scroll.offset_from_bottom(), 0);
    }

    #[test]
    fn ctrl_o_t_focuses_transcript_and_escape_returns_to_composer() {
        let mut state = TuiState::new();
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.handle_key(key(KeyCode::Char('t'))), TuiAction::None);
        assert_eq!(state.focus, TuiFocus::Transcript);
        assert_eq!(state.handle_key(key(KeyCode::Char('k'))), TuiAction::None);
        assert_eq!(state.transcript_scroll.offset_from_bottom(), 1);
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert_eq!(state.focus, TuiFocus::Composer);
    }

    #[test]
    fn extension_command_submission_returns_extension_action() {
        let mut state = TuiState::new();
        state.set_extension_commands(vec![ExtensionCommandSuggestion::new(
            "/9router",
            "Set up 9router",
            "/9router ",
        )]);

        for ch in "/9router setup".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::RunExtensionCommand {
                input: "/9router setup".into(),
            }
        );
        assert_eq!(state.input(), "");
        assert!(state.status.contains("Running extension command"));
    }

    #[test]
    fn settings_command_opens_overlay_without_submitting_prompt() {
        let mut state = TuiState::new();
        for ch in "/settings".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert_eq!(state.input(), "");
    }

    #[test]
    fn new_command_emits_new_session_action_and_reset_clears_state() {
        let mut state = TuiState::new();
        state.messages.push(crate::message::MessageView {
            id: oino_types::OinoId::nil(),
            role: "assistant".into(),
            title: None,
            content: "old".into(),
            thinking: None,
            thinking_redacted: false,
            tool_call_id: None,
            tool_calls: Vec::new(),
            is_error: false,
        });
        state.queued_items.push("queued".into());
        for ch in "/new".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::NewSession);
        state.reset_for_new_session("abc");

        assert!(state.messages.is_empty());
        assert!(state.queued_items.is_empty());
        assert_eq!(state.input(), "");
        assert!(state.status.contains("abc"));
    }

    #[test]
    fn new_command_is_noop_in_blank_session() {
        let mut state = TuiState::new();
        for ch in "/new".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, None);
        assert_eq!(state.input(), "");
        assert!(state.status.contains("blank session"));
    }

    #[test]
    fn sessions_command_opens_browser_and_enter_selects_session() {
        let mut state = TuiState::new();
        for ch in "/sessions".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::ListSessions
        );
        assert_eq!(state.overlay, Some(OverlayKind::Sessions));
        assert!(state.sessions.loading);

        state.set_sessions(vec![
            SessionListItem {
                id: "alpha".into(),
                name: "first".into(),
                cwd: "/tmp/alpha".into(),
                message_count: 2,
                preview: "hello world".into(),
                current: false,
            },
            SessionListItem {
                id: "beta".into(),
                name: "design".into(),
                cwd: "/tmp/beta".into(),
                message_count: 4,
                preview: "markdown rendering".into(),
                current: false,
            },
        ]);
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::OpenSession("alpha".into())
        );

        state.open_sessions_overlay();
        state.set_sessions(state.sessions.items.clone());
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert!(state.sessions.search_active);
        for ch in "md render".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.selected_session_item().map(|item| item.id.as_str()),
            Some("beta")
        );
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::OpenSession("beta".into())
        );
    }

    #[test]
    fn prompts_and_skills_commands_use_resource_state() {
        let mut state = TuiState::new();
        add_test_resources(&mut state);

        for ch in "/prompt:review bugs".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        match state.handle_key(key(KeyCode::Enter)) {
            TuiAction::SubmitPrompt(prompt) => assert_eq!(prompt, "Review bugs"),
            other => panic!("expected prompt submit, got {other:?}"),
        }

        state.composer.clear();
        for ch in "fix crash /skill:debug /skill:debug".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        match state.handle_key(key(KeyCode::Enter)) {
            TuiAction::SubmitPrompt(prompt) => {
                assert!(prompt.contains("# Included Skills"));
                assert!(prompt.contains("## Included Skill: `debug`"));
                assert!(prompt.contains("````markdown\n# Debug Skill\n````"));
                assert!(prompt.contains("# User Request\n\nfix crash"));
                assert!(!prompt.contains("<skill"));
                assert_eq!(prompt.matches("## Included Skill: `debug`").count(), 1);
            }
            other => panic!("expected skill submit, got {other:?}"),
        }
    }

    #[test]
    fn resource_blocks_use_markdown_fences_that_survive_nested_code() {
        let block = markdown_resource_block(
            "Skill",
            "debug",
            ".oino/skills/debug/SKILL.md",
            "# Debug\n\n```rust\nfn main() {}\n```",
        );
        assert!(block.contains("## Included Skill: `debug`"));
        assert!(block.contains("````markdown"));
        assert!(block.contains("```rust"));
        assert!(block.ends_with("````"));
    }

    #[test]
    fn resource_browsers_search_complete_and_reload() {
        let mut state = TuiState::new();
        add_test_resources(&mut state);

        for ch in "/prompts".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Prompts));
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert!(state.prompts.search_active);
        for ch in "rev".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.selected_prompt_item().map(|item| item.name.as_str()),
            Some("review")
        );
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(state.input(), "/prompt:review");

        state.composer.clear();
        for ch in "/skills".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Skills));
        assert_eq!(
            state.handle_key(key(KeyCode::Char('r'))),
            TuiAction::ReloadResources
        );
    }

    #[test]
    fn resource_and_session_filters_prefilter_candidates() {
        let prompts = vec![
            PromptResource {
                name: "review".into(),
                description: "Review changes".into(),
                argument_hint: None,
                source: "/tmp/prompts/review.md".into(),
                scope: "project".into(),
                content: String::new(),
            },
            PromptResource {
                name: "debug".into(),
                description: "Debug failure".into(),
                argument_hint: None,
                source: "/tmp/prompts/debug.md".into(),
                scope: "project".into(),
                content: String::new(),
            },
        ];
        let skills = vec![
            SkillResource {
                name: "research".into(),
                description: "Read only".into(),
                source: "/tmp/skills/research/SKILL.md".into(),
                scope: "project".into(),
                content: String::new(),
            },
            SkillResource {
                name: "quick-fix".into(),
                description: "Patch bug".into(),
                source: "/tmp/skills/quick-fix/SKILL.md".into(),
                scope: "project".into(),
                content: String::new(),
            },
        ];
        let sessions = vec![SessionListItem {
            id: "abc".into(),
            name: "Planning".into(),
            cwd: "/repo/oino".into(),
            message_count: 3,
            preview: "Discuss resources".into(),
            current: false,
        }];

        assert_eq!(prompt_filter_candidate_indices(&prompts, "rev"), vec![0]);
        assert_eq!(filtered_skill_indices(&skills, "qfix"), vec![1]);
        assert_eq!(session_filter_candidate_indices(&sessions, "repo"), vec![0]);
    }

    #[test]
    fn slash_opens_command_suggestions_and_enter_runs_selected_command() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        let suggestions = state
            .command_suggestions_view()
            .unwrap_or_else(|| panic!("missing command suggestions"));
        assert_eq!(suggestions.items[0].label, "/settings");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert_eq!(state.input(), "");
    }

    #[test]
    fn help_command_opens_help_overlay_and_escape_closes() {
        let mut state = TuiState::new();
        for ch in "/help".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Help));
        assert_eq!(state.input(), "");

        assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        assert_eq!(state.help_scroll, 1);
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert_eq!(state.overlay, None);
    }

    #[test]
    fn help_overlay_supports_slash_fuzzy_search() {
        let mut state = TuiState::new();
        state.open_help();
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert!(state.help_search_active);
        for ch in "queue".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.help_search, "queue");
        assert!(!state.filtered_help_indices().is_empty());
        let entries = help_entries(&state.settings.keymap);
        assert!(state
            .filtered_help_indices()
            .iter()
            .all(|index| !matches!(entries[*index], crate::help::HelpEntry::Blank)));
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert!(!state.help_search_active);
        assert_eq!(state.help_search, "queue");
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert!(state.help_search_active);
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert_eq!(state.help_search, "");
        assert!(!state.help_search_active);
    }

    #[test]
    fn tab_completes_command_suggestion_without_submitting() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(state.input(), "/settings ");
        assert_eq!(state.overlay, None);
    }

    #[test]
    fn resource_prefix_and_name_completion_do_not_insert_space() {
        let mut state = TuiState::new();
        add_test_resources(&mut state);

        for ch in "/sk".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        let suggestions = state
            .command_suggestions_view()
            .unwrap_or_else(|| panic!("missing resource prefix suggestions"));
        assert!(suggestions.items.iter().any(|item| item.label == "/skill:"));
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(state.input(), "/skill:");

        for ch in "deb".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(state.input(), "/skill:debug");

        state.composer.clear();
        for ch in "please /P:rev".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(state.input(), "please /prompt:review");
    }

    #[test]
    fn escape_dismisses_command_suggestions_without_quitting() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert!(state.command_suggestions_view().is_some());
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert!(state.command_suggestions_view().is_none());
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert!(state.status.contains("Esc ignored"));
    }

    #[test]
    fn ctrl_c_requires_two_presses_to_quit() {
        let mut state = TuiState::new();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(state.handle_key(ctrl_c), TuiAction::None);
        assert_eq!(state.handle_key(ctrl_c), TuiAction::Quit);
    }

    #[test]
    fn ctrl_c_quits_even_if_input_arrives_between_presses() {
        let mut state = TuiState::new();
        let ctrl_c = KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
        assert_eq!(state.handle_key(ctrl_c), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Char(';'))), TuiAction::None);
        assert_eq!(state.handle_key(ctrl_c), TuiAction::Quit);
    }

    #[test]
    fn escape_aborts_when_working() {
        let mut state = TuiState::new();
        state.set_working(true);
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::AbortPrompt);
    }

    #[test]
    fn settings_model_command_emits_model_change() {
        let mut state = TuiState::new();
        for ch in "/settings model openrouter:xai/glm-5.1".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SetModel("openrouter:xai/glm-5.1".into())
        );
        assert_eq!(state.settings.selected_model, "openrouter:xai/glm-5.1");
    }

    #[test]
    fn title_command_updates_session_title() {
        let mut state = TuiState::new();
        for ch in "/title Design polish".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SetSessionTitle("Design polish".into())
        );
        assert_eq!(state.session_title, "Design polish");
    }

    #[test]
    fn settings_tools_command_opens_tools_page() {
        let mut state = TuiState::new();
        state.set_tool_settings(vec![ToolSettingsItem::global("bash")]);
        for ch in "/settings tools".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert_eq!(state.settings.page, crate::settings::SettingsPage::Tools);
    }

    #[test]
    fn settings_chat_style_command_applies_immediately() {
        let mut state = TuiState::new();
        for ch in "/settings chat-style minimal".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SetChatStyle(crate::settings::ChatStyle::Minimal)
        );
        assert_eq!(
            state.settings.chat_style,
            crate::settings::ChatStyle::Minimal
        );
    }

    #[test]
    fn usage_command_requests_usage_refresh() {
        let mut state = TuiState::new();
        for ch in "/usage".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::RefreshUsage
        );
        assert!(state.usage.loading);
        assert_eq!(state.overlay, Some(OverlayKind::Usage));
        assert!(state.status.contains("Refreshing usage"));
    }

    #[test]
    fn usage_overlay_filters_and_refreshes() {
        let mut state = TuiState::new();
        state.overlay = Some(OverlayKind::Usage);
        state.set_usage_report(UsagePanelReport {
            generated_at_unix: 1,
            status_line: "Usage: 2 turns".into(),
            session: UsagePanelSession {
                assistant_turns: 2,
                reported_turns: 2,
                total_tokens: 30,
                ..UsagePanelSession::default()
            },
            providers: vec![
                UsagePanelProvider {
                    provider_id: "openrouter".into(),
                    display_name: "OpenRouter".into(),
                    status: "available".into(),
                    message: "available: 1 turn".into(),
                    reported_turns: 1,
                    total_tokens: 10,
                    ..UsagePanelProvider::default()
                },
                UsagePanelProvider {
                    provider_id: "local-proxy".into(),
                    display_name: "Local Proxy".into(),
                    status: "not configured".into(),
                    message: "run /9router setup".into(),
                    ..UsagePanelProvider::default()
                },
            ],
        });

        assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        assert_eq!(
            state
                .usage
                .selected_provider()
                .map(|item| item.provider_id.as_str()),
            Some("local-proxy")
        );
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        for ch in "open".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(state.usage.filtered_provider_indices(), &[0]);
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(
            state.handle_key(key(KeyCode::Char('r'))),
            TuiAction::RefreshUsage
        );
        assert!(state.usage.loading);
    }

    #[test]
    fn at_file_suggestions_tab_insert_relative_path() {
        let mut state = TuiState::new();
        state.set_file_paths(vec![
            "README.md".into(),
            "crates/oino-tui/src/app.rs".into(),
            "crates/oino-app/src/main.rs".into(),
        ]);
        for ch in "check @tui/app".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }

        let suggestions = state
            .command_suggestions_view()
            .unwrap_or_else(|| panic!("missing file suggestions"));
        assert_eq!(suggestions.title, "Files");
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(state.input(), "check @crates/oino-tui/src/app.rs ");
    }

    #[test]
    fn model_and_thinking_alias_commands_work() {
        let mut state = TuiState::new();
        for ch in "/model openrouter:xai/glm-5.1".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SetModel("openrouter:xai/glm-5.1".into())
        );

        let mut state = TuiState::new();
        for ch in "/thinking high".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SetThinkingLevel(ThinkingLevel::High)
        );
    }

    #[test]
    fn bare_model_alias_opens_model_settings_page() {
        let mut state = TuiState::new();
        for ch in "/model".chars() {
            assert_eq!(state.handle_key(key(KeyCode::Char(ch))), TuiAction::None);
        }
        state.command_suggestions.dismiss_for("/model");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert_eq!(state.settings.page, crate::settings::SettingsPage::Models);
    }

    #[test]
    fn settings_overlay_can_emit_model_change() {
        let mut state = TuiState::with_settings("a", ThinkingLevel::Off);
        state.set_model_catalog(vec![ModelOption::new("a"), ModelOption::new("b")], "loaded");
        state.overlay = Some(OverlayKind::Settings);
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Down)), TuiAction::None);
        assert_eq!(
            state.handle_key(key(KeyCode::Enter)),
            TuiAction::SetModel("b".into())
        );
    }
}
