#![forbid(unsafe_code)]

use crate::{
    keymap::{
        key_action_rows, KeyAction, KeySequence, KeyStroke, KeymapConfig, KeymapPreset,
        ShortcutKind,
    },
    model_selector::{ModelSelector, ModelSelectorAction, ModelSelectorContext},
    theme::{ResolvedTheme, ThemeCatalog, ThemeMode, ThemeSettings, ThemeSource},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use oino_types::ThinkingLevel;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelAvailability {
    Configured,
    Unknown,
    NeedsProviderKey,
}

impl Default for ModelAvailability {
    fn default() -> Self {
        Self::Unknown
    }
}

impl ModelAvailability {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Configured => "configured",
            Self::Unknown => "unknown",
            Self::NeedsProviderKey => "needs key",
        }
    }

    pub const fn display_rank(self) -> u8 {
        match self {
            Self::Configured => 0,
            Self::Unknown => 1,
            Self::NeedsProviderKey => 2,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub id: String,
    pub display_name: String,
    pub provider: String,
    pub provider_label: String,
    pub availability: ModelAvailability,
    pub thinking_levels: Vec<ThinkingLevel>,
    pub context_length: Option<usize>,
    pub pricing: Option<ModelPricing>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelPricing {
    /// USD per token, provider-reported. String preserves exact decimal payload.
    pub input_per_token: Option<String>,
    /// USD per token, provider-reported. String preserves exact decimal payload.
    pub output_per_token: Option<String>,
    /// USD per cached-input token read / cache hit, provider-reported.
    pub cache_hit_per_token: Option<String>,
    /// USD per cached-input token write, provider-reported.
    pub cache_write_per_token: Option<String>,
    /// Pricing source, e.g. `router` or `openrouter`.
    pub source: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AuthStatusItem {
    pub provider_id: String,
    pub display_name: String,
    pub auth_kind: String,
    pub runtime: String,
    pub state: String,
    pub readiness: String,
    pub source: String,
    pub detail: String,
    pub setup_url: Option<String>,
    pub current: bool,
}

impl AuthStatusItem {
    #[must_use]
    pub fn label(&self) -> String {
        let current = if self.current { "current" } else { "" };
        let suffix = [
            current,
            self.auth_kind.as_str(),
            self.runtime.as_str(),
            self.source.as_str(),
        ]
        .into_iter()
        .filter(|value| !value.trim().is_empty())
        .collect::<Vec<_>>()
        .join(" • ");
        if suffix.is_empty() {
            format!(
                "{} ({}) — {} / {}",
                self.display_name, self.provider_id, self.state, self.readiness
            )
        } else {
            format!(
                "{} ({}) — {} / {} — {}",
                self.display_name, self.provider_id, self.state, self.readiness, suffix
            )
        }
    }
}

impl ModelOption {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        let provider = id.split(':').next().unwrap_or("unknown").to_string();
        Self {
            display_name: id.clone(),
            id,
            provider_label: provider.clone(),
            provider,
            availability: ModelAvailability::Unknown,
            thinking_levels: vec![ThinkingLevel::Off],
            context_length: None,
            pricing: None,
        }
    }

    #[must_use]
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }

    #[must_use]
    pub fn with_provider(mut self, provider: impl Into<String>) -> Self {
        self.provider = provider.into();
        self
    }

    #[must_use]
    pub fn with_provider_label(mut self, provider_label: impl Into<String>) -> Self {
        self.provider_label = provider_label.into();
        self
    }

    #[must_use]
    pub const fn with_availability(mut self, availability: ModelAvailability) -> Self {
        self.availability = availability;
        self
    }

    #[must_use]
    pub fn with_thinking_levels(mut self, thinking_levels: Vec<ThinkingLevel>) -> Self {
        self.thinking_levels = normalize_thinking_levels(thinking_levels);
        self
    }

    #[must_use]
    pub const fn with_context_length(mut self, context_length: Option<usize>) -> Self {
        self.context_length = context_length;
        self
    }

    #[must_use]
    pub fn with_pricing(mut self, pricing: Option<ModelPricing>) -> Self {
        self.pricing = pricing;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolSettingsScope {
    Global,
    Project,
}

impl ToolSettingsScope {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Global => "Global",
            Self::Project => "Project",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolSettingsItem {
    pub name: String,
    pub display_name: String,
    pub global_enabled: bool,
    pub project_enabled: bool,
}

impl ToolSettingsItem {
    #[must_use]
    pub fn global(name: impl Into<String>) -> Self {
        let name = name.into();
        Self {
            display_name: display_tool_name(&name),
            name,
            global_enabled: true,
            project_enabled: false,
        }
    }

    #[must_use]
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }

    #[must_use]
    pub fn with_scopes(mut self, global_enabled: bool, project_enabled: bool) -> Self {
        self.global_enabled = global_enabled;
        self.project_enabled = project_enabled;
        self
    }

    pub fn set_enabled(&mut self, scope: ToolSettingsScope, enabled: bool) {
        match scope {
            ToolSettingsScope::Global => self.global_enabled = enabled,
            ToolSettingsScope::Project => self.project_enabled = enabled,
        }
    }

    #[must_use]
    pub fn enabled(&self, scope: ToolSettingsScope) -> bool {
        match scope {
            ToolSettingsScope::Global => self.global_enabled,
            ToolSettingsScope::Project => self.project_enabled,
        }
    }

    #[must_use]
    pub fn label(&self) -> String {
        format!(
            "{} - [Global - {}] [Project - {}]",
            self.display_name,
            on_off(self.global_enabled),
            on_off(self.project_enabled)
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Menu,
    Models,
    Thinking,
    Collapse,
    ChatStyle,
    Tools,
    Auth,
    Keymaps,
    Theme,
    Notify,
    Compaction,
    Extensions,
    /// Sub-page: picking a model for the notify summary.
    NotifyModelPicker,
    /// Sub-page: picking a model for compaction LLM.
    CompactionModelPicker,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KeymapsMode {
    List,
    Detail,
    ShortcutType {
        edit_index: Option<usize>,
    },
    Capture {
        edit_index: Option<usize>,
        kind: ShortcutKind,
        strokes: Vec<KeyStroke>,
    },
    ChordKeyCapture,
    PresetSelect,
    PresetConfirm {
        preset: KeymapPreset,
    },
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CollapseMode {
    #[default]
    Full,
    Truncate,
    Collapse,
}

impl CollapseMode {
    #[must_use]
    pub fn next(self) -> Self {
        match self {
            Self::Full => Self::Truncate,
            Self::Truncate => Self::Collapse,
            Self::Collapse => Self::Full,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChatStyle {
    #[default]
    Chat,
    Agentic,
    Minimal,
}

impl ChatStyle {
    #[must_use]
    pub fn all() -> [Self; 3] {
        [Self::Chat, Self::Agentic, Self::Minimal]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollapseTarget {
    Thinking,
    Tool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsMenuItem {
    ModelSelection,
    ThinkingLevel,
    CollapseMode,
    ChatStyle,
    Tools,
    Auth,
    Keymaps,
    Theme,
    Notify,
    Compaction,
    Extensions,
}

impl SettingsMenuItem {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ModelSelection => "Model Selection",
            Self::ThinkingLevel => "Thinking Level",
            Self::CollapseMode => "Collapse Mode",
            Self::ChatStyle => "Chat Style",
            Self::Tools => "Tools",
            Self::Auth => "Auth & Providers",
            Self::Keymaps => "Keymaps",
            Self::Theme => "Theme",
            Self::Notify => "Notify",
            Self::Compaction => "Compaction",
            Self::Extensions => "Extensions",
        }
    }

    #[must_use]
    pub fn page(self) -> SettingsPage {
        match self {
            Self::ModelSelection => SettingsPage::Models,
            Self::ThinkingLevel => SettingsPage::Thinking,
            Self::CollapseMode => SettingsPage::Collapse,
            Self::ChatStyle => SettingsPage::ChatStyle,
            Self::Tools => SettingsPage::Tools,
            Self::Auth => SettingsPage::Auth,
            Self::Keymaps => SettingsPage::Keymaps,
            Self::Theme => SettingsPage::Theme,
            Self::Notify => SettingsPage::Notify,
            Self::Compaction => SettingsPage::Compaction,
            Self::Extensions => SettingsPage::Extensions,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsAction {
    None,
    Close,
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
    OpenExtensions,
    PreviewTheme {
        id: String,
    },
    ClearThemePreview,
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
    SetCompactSettings {
        method_is_llm: bool,
        auto_enabled: bool,
        threshold_pct: Option<u8>,
    },
    SetCompactModel {
        id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThemeOption {
    pub id: String,
    pub display_name: String,
    pub description: String,
    pub mode: ThemeMode,
    pub source: ThemeSource,
    pub global_active: bool,
    pub project_active: bool,
    pub effective: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyField {
    Server,
    Topic,
    Token,
    Priority,
    Tags,
    SummaryModel,
    SummaryPrompt,
    SummaryMaxChars,
}

impl NotifyField {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Server => "server",
            Self::Topic => "topic",
            Self::Token => "token",
            Self::Priority => "priority",
            Self::Tags => "tags",
            Self::SummaryModel => "summary model",
            Self::SummaryPrompt => "summary prompt",
            Self::SummaryMaxChars => "summary max chars",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyEventKind {
    AgentEnd,
    ToolError,
}

impl NotifyEventKind {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::AgentEnd => "agent_end",
            Self::ToolError => "tool_error",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NotifyScopeSettings {
    pub enabled: Option<bool>,
    pub server: Option<String>,
    pub topic: Option<String>,
    pub token: Option<String>,
    pub priority: Option<String>,
    pub tags: Option<Vec<String>>,
    pub events: Option<Vec<NotifyEventKind>>,
    pub summary_enabled: Option<bool>,
    pub summary_model: Option<String>,
    pub summary_prompt: Option<String>,
    pub summary_max_chars: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifyEditState {
    pub scope: ToolSettingsScope,
    pub field: NotifyField,
    pub input: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NotifySettingsState {
    pub global: NotifyScopeSettings,
    pub project: NotifyScopeSettings,
    pub cursor: usize,
    pub scope: ToolSettingsScope,
    pub edit: Option<NotifyEditState>,
    pub available: bool,
}

impl Default for NotifySettingsState {
    fn default() -> Self {
        Self {
            global: NotifyScopeSettings::default(),
            project: NotifyScopeSettings::default(),
            cursor: 0,
            scope: ToolSettingsScope::Project,
            edit: None,
            available: false,
        }
    }
}

impl NotifySettingsState {
    pub const ROWS: [NotifyRow; 12] = [
        NotifyRow::Enabled,
        NotifyRow::Server,
        NotifyRow::Topic,
        NotifyRow::Token,
        NotifyRow::Priority,
        NotifyRow::Tags,
        NotifyRow::AgentEnd,
        NotifyRow::ToolError,
        NotifyRow::SummaryEnabled,
        NotifyRow::SummaryModel,
        NotifyRow::SummaryPrompt,
        NotifyRow::SummaryMaxChars,
    ];

    #[must_use]
    pub fn scope_settings(&self, scope: ToolSettingsScope) -> &NotifyScopeSettings {
        match scope {
            ToolSettingsScope::Global => &self.global,
            ToolSettingsScope::Project => &self.project,
        }
    }

    #[must_use]
    pub fn effective_enabled(&self) -> bool {
        self.project
            .enabled
            .or(self.global.enabled)
            .unwrap_or(false)
    }

    #[must_use]
    pub fn effective_text(&self, field: NotifyField) -> Option<String> {
        choose_notify_text(
            project_field(&self.project, field),
            project_field(&self.global, field),
        )
    }

    #[must_use]
    pub fn effective_events(&self) -> Vec<NotifyEventKind> {
        self.project
            .events
            .clone()
            .or_else(|| self.global.events.clone())
            .unwrap_or_else(|| vec![NotifyEventKind::AgentEnd, NotifyEventKind::ToolError])
    }

    #[must_use]
    pub fn effective_event_enabled(&self, event: NotifyEventKind) -> bool {
        self.effective_events().contains(&event)
    }

    pub fn set_available(&mut self, available: bool) {
        self.available = available;
    }

    pub fn set_config(&mut self, global: NotifyScopeSettings, project: NotifyScopeSettings) {
        self.global = global;
        self.project = project;
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotifyRow {
    Enabled,
    Server,
    Topic,
    Token,
    Priority,
    Tags,
    AgentEnd,
    ToolError,
    SummaryEnabled,
    SummaryModel,
    SummaryPrompt,
    SummaryMaxChars,
}

impl NotifyRow {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Enabled => "Enabled",
            Self::Server => "Server",
            Self::Topic => "Topic",
            Self::Token => "Token",
            Self::Priority => "Priority",
            Self::Tags => "Tags",
            Self::AgentEnd => "Event: agent_end",
            Self::ToolError => "Event: tool_error",
            Self::SummaryEnabled => "Summarizer",
            Self::SummaryModel => "Summary model",
            Self::SummaryPrompt => "Summary prompt",
            Self::SummaryMaxChars => "Summary max chars",
        }
    }

    #[must_use]
    pub const fn field(self) -> Option<NotifyField> {
        match self {
            Self::Server => Some(NotifyField::Server),
            Self::Topic => Some(NotifyField::Topic),
            Self::Token => Some(NotifyField::Token),
            Self::Priority => Some(NotifyField::Priority),
            Self::Tags => Some(NotifyField::Tags),
            Self::SummaryModel => Some(NotifyField::SummaryModel),
            Self::SummaryPrompt => Some(NotifyField::SummaryPrompt),
            Self::SummaryMaxChars => Some(NotifyField::SummaryMaxChars),
            Self::Enabled | Self::AgentEnd | Self::ToolError | Self::SummaryEnabled => None,
        }
    }

    #[must_use]
    pub const fn event(self) -> Option<NotifyEventKind> {
        match self {
            Self::AgentEnd => Some(NotifyEventKind::AgentEnd),
            Self::ToolError => Some(NotifyEventKind::ToolError),
            _ => None,
        }
    }
}

fn choose_notify_text(project: Option<String>, global: Option<String>) -> Option<String> {
    project
        .filter(|value| !value.trim().is_empty())
        .or_else(|| global.filter(|value| !value.trim().is_empty()))
}

fn project_field(settings: &NotifyScopeSettings, field: NotifyField) -> Option<String> {
    match field {
        NotifyField::Server => settings.server.clone(),
        NotifyField::Topic => settings.topic.clone(),
        NotifyField::Token => settings.token.clone(),
        NotifyField::Priority => settings.priority.clone(),
        NotifyField::Tags => settings.tags.as_ref().map(|tags| tags.join(",")),
        NotifyField::SummaryModel => settings.summary_model.clone(),
        NotifyField::SummaryPrompt => settings.summary_prompt.clone(),
        NotifyField::SummaryMaxChars => settings.summary_max_chars.map(|value| value.to_string()),
    }
}

fn notify_scope_enabled(settings: &NotifyScopeSettings) -> bool {
    settings.enabled.unwrap_or(false)
}

fn notify_scope_text(settings: &NotifyScopeSettings, field: NotifyField) -> Option<String> {
    project_field(settings, field).filter(|value| !value.trim().is_empty())
}

fn notify_scope_event_enabled(settings: &NotifyScopeSettings, event: NotifyEventKind) -> bool {
    settings
        .events
        .as_ref()
        .is_none_or(|events| events.contains(&event))
}

fn normalize_optional_text(input: &str) -> Option<String> {
    let trimmed = input.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// State for the Compaction settings page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompactionSettingsState {
    pub cursor: usize,
    /// Loaded from ~/.oino/settings.json
    pub threshold_pct: Option<u8>,
    pub method_is_llm: bool,
    pub auto_enabled: bool,
    pub model: Option<String>,
    pub prompt: Option<String>,
}

impl Default for CompactionSettingsState {
    fn default() -> Self {
        Self {
            cursor: 0,
            threshold_pct: Some(80),
            method_is_llm: false,
            auto_enabled: true,
            model: None,
            prompt: None,
        }
    }
}

impl CompactionSettingsState {
    pub const ROW_COUNT: usize = 5;

    /// Update state from loaded user settings.
    pub fn update_from_settings(
        &mut self,
        threshold_pct: Option<u8>,
        method_is_llm: bool,
        auto_enabled: bool,
        model: Option<String>,
        prompt: Option<String>,
    ) {
        self.threshold_pct = threshold_pct;
        self.method_is_llm = method_is_llm;
        self.auto_enabled = auto_enabled;
        self.model = model;
        self.prompt = prompt;
    }

    pub fn clamp_cursor(&mut self) {
        self.cursor = self.cursor.min(Self::ROW_COUNT.saturating_sub(1));
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsState {
    pub model_selector: ModelSelector,
    /// Reusable sub-page picker for Notify summary model and Compaction LLM model.
    pub sub_model_picker: ModelSelector,
    pub selected_thinking_level: ThinkingLevel,
    pub page: SettingsPage,
    pub menu_cursor: usize,
    pub thinking_cursor: usize,
    pub collapse_cursor: usize,
    pub chat_style_cursor: usize,
    pub tool_cursor: usize,
    pub auth_cursor: usize,
    pub theme_cursor: usize,
    pub keymap_cursor: usize,
    pub keymap_binding_cursor: usize,
    pub keymap_shortcut_kind_cursor: usize,
    pub keymap_preset_cursor: usize,
    pub keymaps_mode: KeymapsMode,
    pub thinking_collapse_mode: CollapseMode,
    pub tool_collapse_mode: CollapseMode,
    pub chat_style: ChatStyle,
    pub tools: Vec<ToolSettingsItem>,
    pub auth_items: Vec<AuthStatusItem>,
    pub theme_options: Vec<ThemeOption>,
    pub notify: NotifySettingsState,
    pub compact: CompactionSettingsState,
    pub global_theme: ThemeSettings,
    pub project_theme: ThemeSettings,
    pub effective_theme: Option<ResolvedTheme>,
    pub preview_theme: Option<ResolvedTheme>,
    pub keymap: KeymapConfig,
    pub status: String,
    pub refreshing: bool,
}

impl SettingsState {
    #[must_use]
    pub fn new(model: impl Into<String>, thinking_level: ThinkingLevel) -> Self {
        let model_string = model.into();
        Self {
            model_selector: ModelSelector::new(ModelSelectorContext::Main, model_string.clone()),
            sub_model_picker: ModelSelector::new(ModelSelectorContext::NotifySummary, model_string),
            selected_thinking_level: thinking_level,
            page: SettingsPage::Menu,
            menu_cursor: 0,
            thinking_cursor: thinking_index(thinking_level, &all_thinking_levels()),
            collapse_cursor: 0,
            chat_style_cursor: 0,
            tool_cursor: 0,
            auth_cursor: 0,
            theme_cursor: 0,
            keymap_cursor: 0,
            keymap_binding_cursor: 0,
            keymap_shortcut_kind_cursor: 0,
            keymap_preset_cursor: 0,
            keymaps_mode: KeymapsMode::List,
            thinking_collapse_mode: CollapseMode::Full,
            tool_collapse_mode: CollapseMode::Full,
            chat_style: ChatStyle::Chat,
            tools: Vec::new(),
            auth_items: Vec::new(),
            theme_options: Vec::new(),
            notify: NotifySettingsState::default(),
            compact: CompactionSettingsState::default(),
            global_theme: ThemeSettings::default(),
            project_theme: ThemeSettings::default(),
            effective_theme: None,
            preview_theme: None,
            keymap: KeymapConfig::default(),
            status: "Model catalog not loaded yet".into(),
            refreshing: false,
        }
    }

    pub fn open_menu(&mut self) {
        self.page = SettingsPage::Menu;
        self.menu_cursor = 0;
    }

    pub fn open_model_selection(&mut self) {
        self.page = SettingsPage::Models;
        self.model_selector.open();
    }

    pub fn open_thinking_level(&mut self) {
        self.page = SettingsPage::Thinking;
        self.thinking_cursor =
            thinking_index(self.selected_thinking_level, &self.thinking_levels());
    }

    pub fn set_models(&mut self, models: Vec<ModelOption>, status: impl Into<String>) {
        let status_str = status.into();
        self.model_selector.set_models(models.clone(), &status_str);
        self.sub_model_picker.set_models(models, &status_str);
        self.status = self.model_selector.status.clone();
        self.clamp_thinking_to_selected_model();
    }

    pub fn set_refreshing(&mut self, refreshing: bool) {
        self.refreshing = refreshing;
        self.model_selector.set_refreshing(refreshing);
        self.sub_model_picker.set_refreshing(refreshing);
    }

    pub fn set_collapse_modes(&mut self, thinking: CollapseMode, tool: CollapseMode) {
        self.thinking_collapse_mode = thinking;
        self.tool_collapse_mode = tool;
    }

    pub fn set_collapse_mode(&mut self, target: CollapseTarget, mode: CollapseMode) {
        match target {
            CollapseTarget::Thinking => self.thinking_collapse_mode = mode,
            CollapseTarget::Tool => self.tool_collapse_mode = mode,
        }
    }

    pub fn set_chat_style(&mut self, style: ChatStyle) {
        self.chat_style = style;
        self.chat_style_cursor = chat_style_index(style);
    }

    pub fn set_tools(&mut self, tools: Vec<ToolSettingsItem>) {
        self.tools = tools;
        self.tool_cursor = self.tool_cursor.min(self.tools.len().saturating_sub(1));
    }

    pub fn set_theme_state(
        &mut self,
        catalog: &ThemeCatalog,
        global: &ThemeSettings,
        project: &ThemeSettings,
        effective: &ResolvedTheme,
    ) {
        self.global_theme = global.clone();
        self.project_theme = project.clone();
        self.effective_theme = Some(effective.clone());
        self.preview_theme = None;
        let global_active = global.active_id();
        let project_active = project.active_id();
        self.theme_options = catalog
            .entries()
            .iter()
            .filter_map(|entry| {
                let id = entry.document.normalized_id()?;
                Some(ThemeOption {
                    global_active: global_active.as_deref() == Some(id.as_str()),
                    project_active: project_active.as_deref() == Some(id.as_str()),
                    effective: effective.id == id,
                    display_name: if entry.document.display_name.trim().is_empty() {
                        id.clone()
                    } else {
                        entry.document.display_name.clone()
                    },
                    description: entry.document.description.clone().unwrap_or_default(),
                    mode: entry.document.mode,
                    source: entry.source,
                    id,
                })
            })
            .collect();
        self.theme_options.sort_by(|left, right| {
            right
                .project_active
                .cmp(&left.project_active)
                .then(right.global_active.cmp(&left.global_active))
                .then(right.effective.cmp(&left.effective))
                .then_with(|| {
                    right
                        .source
                        .precedence_rank()
                        .cmp(&left.source.precedence_rank())
                })
                .then(left.display_name.cmp(&right.display_name))
        });
        self.theme_cursor = self
            .theme_options
            .iter()
            .position(|option| option.effective)
            .unwrap_or(0)
            .min(self.theme_options.len().saturating_sub(1));
    }

    pub fn open_chat_style(&mut self) {
        self.page = SettingsPage::ChatStyle;
        self.chat_style_cursor = chat_style_index(self.chat_style);
    }

    pub fn open_tools(&mut self) {
        self.page = SettingsPage::Tools;
        self.tool_cursor = self.tool_cursor.min(self.tools.len().saturating_sub(1));
    }

    pub fn open_auth(&mut self) {
        self.page = SettingsPage::Auth;
        self.auth_cursor = self
            .auth_cursor
            .min(self.auth_items.len().saturating_sub(1));
    }

    pub fn set_auth_items(&mut self, items: Vec<AuthStatusItem>) {
        self.auth_items = items;
        self.auth_cursor = self
            .auth_cursor
            .min(self.auth_items.len().saturating_sub(1));
    }

    pub fn select_auth_provider(&mut self, provider_id: &str) {
        if let Some(index) = self
            .auth_items
            .iter()
            .position(|item| item.provider_id == provider_id)
        {
            self.auth_cursor = index;
        }
    }

    pub fn open_keymaps(&mut self) {
        self.page = SettingsPage::Keymaps;
        self.keymaps_mode = KeymapsMode::List;
        self.keymap_cursor = self
            .keymap_cursor
            .min(key_action_rows().len().saturating_sub(1));
    }

    pub fn open_theme(&mut self) {
        self.page = SettingsPage::Theme;
        self.theme_cursor = self
            .theme_cursor
            .min(self.theme_options.len().saturating_sub(1));
    }

    pub fn open_notify(&mut self) {
        self.page = SettingsPage::Notify;
        self.notify.cursor = self
            .notify
            .cursor
            .min(NotifySettingsState::ROWS.len().saturating_sub(1));
        self.notify.edit = None;
    }

    pub fn set_notify_available(&mut self, available: bool) {
        self.notify.set_available(available);
        if !available && self.page == SettingsPage::Notify {
            self.page = SettingsPage::Menu;
        }
        self.menu_cursor = self
            .menu_cursor
            .min(self.menu_items().len().saturating_sub(1));
    }

    pub fn set_notify_settings(
        &mut self,
        global: NotifyScopeSettings,
        project: NotifyScopeSettings,
    ) {
        self.notify.set_config(global, project);
    }

    pub fn set_theme_preview(&mut self, preview: Option<ResolvedTheme>) {
        self.preview_theme = preview;
    }

    pub fn clear_theme_preview(&mut self) {
        self.preview_theme = None;
    }

    pub fn active_or_preview_theme(&self) -> Option<&ResolvedTheme> {
        self.preview_theme
            .as_ref()
            .or(self.effective_theme.as_ref())
    }

    pub fn preview_theme_id(&self) -> Option<&str> {
        self.preview_theme.as_ref().map(|theme| theme.id.as_str())
    }

    pub fn selected_theme_id(&self) -> Option<String> {
        self.theme_options
            .get(self.theme_cursor)
            .map(|option| option.id.clone())
    }

    pub fn set_keymap(&mut self, keymap: KeymapConfig) {
        self.keymap = keymap;
        self.keymap_cursor = self
            .keymap_cursor
            .min(key_action_rows().len().saturating_sub(1));
        self.keymap_binding_cursor = self
            .keymap_binding_cursor
            .min(self.current_keymap_bindings().len().saturating_sub(1));
    }

    pub fn select_model_identifier(&mut self, model: &str) {
        self.model_selector.initial_model = model.to_string();
        self.model_selector.cursor = self
            .model_selector
            .models
            .iter()
            .position(|m| m.id == model)
            .unwrap_or(self.model_selector.cursor);
        self.clamp_thinking_to_selected_model();
    }

    pub fn select_thinking_level(&mut self, level: ThinkingLevel) {
        self.selected_thinking_level = level;
        self.clamp_thinking_to_selected_model();
    }

    #[must_use]
    pub fn menu_items(&self) -> Vec<SettingsMenuItem> {
        let mut items = vec![
            SettingsMenuItem::ModelSelection,
            SettingsMenuItem::ThinkingLevel,
            SettingsMenuItem::CollapseMode,
            SettingsMenuItem::ChatStyle,
            SettingsMenuItem::Tools,
            SettingsMenuItem::Auth,
            SettingsMenuItem::Keymaps,
            SettingsMenuItem::Theme,
        ];
        if self.notify.available {
            items.push(SettingsMenuItem::Notify);
        }
        items.push(SettingsMenuItem::Compaction);
        items.push(SettingsMenuItem::Extensions);
        items
    }

    #[must_use]
    pub fn current_menu_item(&self) -> SettingsMenuItem {
        self.menu_items()
            .get(self.menu_cursor)
            .copied()
            .unwrap_or(SettingsMenuItem::ModelSelection)
    }

    #[must_use]
    pub fn current_keymap_action(&self) -> KeyAction {
        key_action_rows()
            .get(self.keymap_cursor)
            .map_or(KeyAction::SettingsOpen, |info| info.action)
    }

    #[must_use]
    pub fn current_keymap_bindings(&self) -> Vec<KeySequence> {
        self.keymap.bindings_for(self.current_keymap_action())
    }

    #[must_use]
    pub fn keymap_preset_cursor_preset(&self) -> KeymapPreset {
        KeymapPreset::all()
            .get(self.keymap_preset_cursor)
            .copied()
            .unwrap_or_default()
    }

    #[must_use]
    pub fn shortcut_kind_cursor_kind(&self) -> ShortcutKind {
        ShortcutKind::all()
            .get(self.keymap_shortcut_kind_cursor)
            .copied()
            .unwrap_or(ShortcutKind::Combination)
    }

    #[must_use]
    pub fn thinking_levels(&self) -> Vec<ThinkingLevel> {
        self.model_selector
            .models
            .iter()
            .find(|model| model.id == self.model_selector.initial_model)
            .map_or_else(all_thinking_levels, |model| {
                normalize_thinking_levels(model.thinking_levels.clone())
            })
    }

    /// The currently selected model ID (primary convenience accessor used throughout the app).
    #[must_use]
    pub fn selected_model(&self) -> &str {
        &self.model_selector.initial_model
    }

    #[must_use]
    pub fn selected_model_label(&self) -> &str {
        self.model_selector.selected_model_label()
    }

    #[must_use]
    pub fn selected_model_context_length(&self) -> Option<usize> {
        self.model_selector
            .models
            .iter()
            .find(|model| model.id == self.model_selector.initial_model)
            .and_then(|model| model.context_length)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        if self.page == SettingsPage::Keymaps {
            return self.handle_keymaps_key(key);
        }
        if self.page == SettingsPage::Notify {
            return self.handle_notify_key(key);
        }
        if self.page == SettingsPage::Models {
            return self.handle_model_page_key(key);
        }
        if self.page == SettingsPage::NotifyModelPicker {
            return self.handle_sub_model_picker_key(key, SettingsPage::Notify, |id, settings| {
                SettingsAction::SetNotifyField {
                    scope: settings.notify.scope,
                    field: NotifyField::SummaryModel,
                    value: Some(id),
                }
            });
        }
        if self.page == SettingsPage::CompactionModelPicker {
            return self.handle_sub_model_picker_key(
                key,
                SettingsPage::Compaction,
                |id, _settings| SettingsAction::SetCompactModel { id },
            );
        }

        match key.code {
            KeyCode::Esc => self.close_or_return_to_menu(),
            KeyCode::Backspace | KeyCode::Left if self.page == SettingsPage::Theme => {
                self.page = SettingsPage::Menu;
                SettingsAction::ClearThemePreview
            }
            KeyCode::Backspace | KeyCode::Left if self.page != SettingsPage::Menu => {
                self.page = SettingsPage::Menu;
                SettingsAction::None
            }
            KeyCode::Right if self.page == SettingsPage::Menu => self.open_current_menu_item(),
            KeyCode::Right if self.page == SettingsPage::Collapse => self.apply_collapse_mode(),
            KeyCode::Right if self.page == SettingsPage::Tools => {
                self.toggle_tool(ToolSettingsScope::Project)
            }
            KeyCode::Right if self.page == SettingsPage::Compaction => {
                self.toggle_compaction_field()
            }
            KeyCode::Left if self.page == SettingsPage::Compaction => {
                self.toggle_compaction_field_left()
            }
            KeyCode::Char('g' | 'G')
                if self.page == SettingsPage::Tools && key.modifiers.is_empty() =>
            {
                self.toggle_tool(ToolSettingsScope::Global)
            }
            KeyCode::Char('p' | 'P' | ' ')
                if self.page == SettingsPage::Tools && key.modifiers.is_empty() =>
            {
                self.toggle_tool(ToolSettingsScope::Project)
            }
            KeyCode::Char('p' | 'P')
                if self.page == SettingsPage::Theme && key.modifiers.is_empty() =>
            {
                self.apply_theme(ToolSettingsScope::Project)
            }
            KeyCode::Char('g' | 'G')
                if self.page == SettingsPage::Theme && key.modifiers.is_empty() =>
            {
                self.apply_theme(ToolSettingsScope::Global)
            }
            KeyCode::Char('r') if self.page == SettingsPage::Theme && key.modifiers.is_empty() => {
                SettingsAction::ResetTheme {
                    scope: ToolSettingsScope::Project,
                }
            }
            KeyCode::Char('R') if self.page == SettingsPage::Theme && key.modifiers.is_empty() => {
                SettingsAction::ResetTheme {
                    scope: ToolSettingsScope::Global,
                }
            }
            KeyCode::Up => {
                self.move_cursor(-1);
                SettingsAction::None
            }
            KeyCode::Down => {
                self.move_cursor(1);
                SettingsAction::None
            }
            KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.move_cursor(-1);
                SettingsAction::None
            }
            KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.move_cursor(1);
                SettingsAction::None
            }
            KeyCode::Tab if self.page == SettingsPage::Menu => {
                self.move_cursor(1);
                SettingsAction::None
            }
            KeyCode::BackTab if self.page == SettingsPage::Menu => {
                self.move_cursor(-1);
                SettingsAction::None
            }
            KeyCode::Enter => self.apply_or_open(),
            _ => SettingsAction::None,
        }
    }

    fn handle_notify_key(&mut self, key: KeyEvent) -> SettingsAction {
        if self.notify.edit.is_some() {
            return self.handle_notify_edit_key(key);
        }
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => {
                self.page = SettingsPage::Menu;
                SettingsAction::None
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.notify.cursor =
                    move_index(self.notify.cursor, NotifySettingsState::ROWS.len(), -1);
                SettingsAction::None
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.notify.cursor =
                    move_index(self.notify.cursor, NotifySettingsState::ROWS.len(), 1);
                SettingsAction::None
            }
            KeyCode::Tab | KeyCode::Char('p') if key.modifiers.is_empty() => {
                self.notify.scope = ToolSettingsScope::Project;
                self.status = "Editing project notify settings".into();
                SettingsAction::None
            }
            KeyCode::BackTab | KeyCode::Char('g') if key.modifiers.is_empty() => {
                self.notify.scope = ToolSettingsScope::Global;
                self.status = "Editing global notify settings".into();
                SettingsAction::None
            }
            KeyCode::Char('x') if key.modifiers.is_empty() => self.clear_notify_value(),
            KeyCode::Enter | KeyCode::Right => self.apply_notify_row(),
            _ => SettingsAction::None,
        }
    }

    fn handle_notify_edit_key(&mut self, key: KeyEvent) -> SettingsAction {
        let Some(mut edit) = self.notify.edit.clone() else {
            return SettingsAction::None;
        };
        match key.code {
            KeyCode::Esc => {
                self.notify.edit = None;
                SettingsAction::None
            }
            KeyCode::Enter => {
                self.notify.edit = None;
                let value = normalize_optional_text(&edit.input);
                SettingsAction::SetNotifyField {
                    scope: edit.scope,
                    field: edit.field,
                    value,
                }
            }
            KeyCode::Backspace => {
                edit.input.pop();
                self.notify.edit = Some(edit);
                SettingsAction::None
            }
            KeyCode::Char(ch)
                if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT =>
            {
                edit.input.push(ch);
                self.notify.edit = Some(edit);
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn toggle_compaction_field(&mut self) -> SettingsAction {
        match self.compact.cursor {
            0 => {
                // Toggle method: VCC <-> LLM
                self.compact.method_is_llm = !self.compact.method_is_llm;
            }
            1 => {
                // Toggle auto on/off
                self.compact.auto_enabled = !self.compact.auto_enabled;
            }
            _ => return SettingsAction::None,
        }
        SettingsAction::SetCompactSettings {
            method_is_llm: self.compact.method_is_llm,
            auto_enabled: self.compact.auto_enabled,
            threshold_pct: self.compact.threshold_pct,
        }
    }

    fn toggle_compaction_field_left(&mut self) -> SettingsAction {
        // Same as right for toggle fields
        self.toggle_compaction_field()
    }

    fn open_compaction_model_picker(&mut self) -> SettingsAction {
        let current = self
            .compact
            .model
            .clone()
            .unwrap_or_else(|| self.model_selector.initial_model.clone());
        self.sub_model_picker.context = ModelSelectorContext::CompactionModel;
        self.sub_model_picker.initial_model = current;
        self.sub_model_picker.open();
        self.page = SettingsPage::CompactionModelPicker;
        SettingsAction::None
    }

    fn apply_notify_row(&mut self) -> SettingsAction {
        let row = NotifySettingsState::ROWS
            .get(self.notify.cursor)
            .copied()
            .unwrap_or(NotifyRow::Enabled);
        match row {
            NotifyRow::Enabled => SettingsAction::SetNotifyEnabled {
                scope: self.notify.scope,
                enabled: !notify_scope_enabled(self.notify.scope_settings(self.notify.scope)),
            },
            NotifyRow::SummaryEnabled => SettingsAction::SetNotifyField {
                scope: self.notify.scope,
                field: NotifyField::SummaryPrompt,
                value: Some(
                    if self
                        .notify
                        .scope_settings(self.notify.scope)
                        .summary_enabled
                        .unwrap_or(true)
                    {
                        "__summary_enabled:false".into()
                    } else {
                        "__summary_enabled:true".into()
                    },
                ),
            },
            NotifyRow::AgentEnd | NotifyRow::ToolError => {
                let event = row.event().unwrap_or(NotifyEventKind::AgentEnd);
                SettingsAction::SetNotifyEvent {
                    scope: self.notify.scope,
                    event,
                    enabled: !notify_scope_event_enabled(
                        self.notify.scope_settings(self.notify.scope),
                        event,
                    ),
                }
            }
            NotifyRow::SummaryModel => {
                let current = self
                    .notify
                    .scope_settings(self.notify.scope)
                    .summary_model
                    .clone()
                    .unwrap_or_else(|| self.model_selector.initial_model.clone());
                self.sub_model_picker.context = ModelSelectorContext::NotifySummary;
                self.sub_model_picker.initial_model = current;
                self.sub_model_picker.open();
                self.page = SettingsPage::NotifyModelPicker;
                SettingsAction::None
            }
            _ => {
                let field = row.field().unwrap_or(NotifyField::Topic);
                let input = notify_scope_text(self.notify.scope_settings(self.notify.scope), field)
                    .unwrap_or_default();
                self.notify.edit = Some(NotifyEditState {
                    scope: self.notify.scope,
                    field,
                    input,
                });
                SettingsAction::None
            }
        }
    }

    fn clear_notify_value(&mut self) -> SettingsAction {
        let row = NotifySettingsState::ROWS
            .get(self.notify.cursor)
            .copied()
            .unwrap_or(NotifyRow::Enabled);
        if row == NotifyRow::Enabled {
            return SettingsAction::SetNotifyEnabled {
                scope: self.notify.scope,
                enabled: false,
            };
        }
        if row == NotifyRow::SummaryEnabled {
            return SettingsAction::SetNotifyField {
                scope: self.notify.scope,
                field: NotifyField::SummaryPrompt,
                value: Some("__summary_enabled:false".into()),
            };
        }
        if let Some(event) = row.event() {
            return SettingsAction::SetNotifyEvent {
                scope: self.notify.scope,
                event,
                enabled: false,
            };
        }
        let Some(field) = row.field() else {
            return SettingsAction::None;
        };
        SettingsAction::SetNotifyField {
            scope: self.notify.scope,
            field,
            value: None,
        }
    }

    fn handle_keymaps_key(&mut self, key: KeyEvent) -> SettingsAction {
        match self.keymaps_mode.clone() {
            KeymapsMode::List => self.handle_keymaps_list_key(key),
            KeymapsMode::Detail => self.handle_keymaps_detail_key(key),
            KeymapsMode::ShortcutType { edit_index } => {
                self.handle_keymap_shortcut_type_key(key, edit_index)
            }
            KeymapsMode::Capture {
                edit_index,
                kind,
                mut strokes,
            } => self.handle_keymap_capture_key(key, edit_index, kind, &mut strokes),
            KeymapsMode::ChordKeyCapture => self.handle_chord_key_capture_key(key),
            KeymapsMode::PresetSelect => self.handle_keymap_preset_select_key(key),
            KeymapsMode::PresetConfirm { preset } => {
                self.handle_keymap_preset_confirm_key(key, preset)
            }
        }
    }

    fn handle_keymaps_list_key(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => {
                self.page = SettingsPage::Menu;
                self.keymaps_mode = KeymapsMode::List;
                SettingsAction::None
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.keymap_cursor = move_index(self.keymap_cursor, key_action_rows().len(), -1);
                SettingsAction::None
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.keymap_cursor = move_index(self.keymap_cursor, key_action_rows().len(), 1);
                SettingsAction::None
            }
            KeyCode::Char('g' | 'G') if key.modifiers.is_empty() => {
                self.keymaps_mode = KeymapsMode::ChordKeyCapture;
                self.status = "Listening for global chord key • Esc cancel".into();
                SettingsAction::None
            }
            KeyCode::Char('p' | 'P') if key.modifiers.is_empty() => {
                self.keymap_preset_cursor = KeymapPreset::all()
                    .iter()
                    .position(|preset| *preset == self.keymap.preset)
                    .unwrap_or(0);
                self.keymaps_mode = KeymapsMode::PresetSelect;
                SettingsAction::None
            }
            KeyCode::Enter | KeyCode::Right => self.open_keymap_detail(),
            _ => SettingsAction::None,
        }
    }

    fn handle_keymaps_detail_key(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => {
                self.keymaps_mode = KeymapsMode::List;
                SettingsAction::None
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.keymap_binding_cursor = move_index(
                    self.keymap_binding_cursor,
                    self.current_keymap_bindings().len().max(1),
                    -1,
                );
                SettingsAction::None
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.keymap_binding_cursor = move_index(
                    self.keymap_binding_cursor,
                    self.current_keymap_bindings().len().max(1),
                    1,
                );
                SettingsAction::None
            }
            KeyCode::Enter => {
                let edit_index = (!self.current_keymap_bindings().is_empty())
                    .then_some(self.keymap_binding_cursor);
                self.keymaps_mode = KeymapsMode::ShortcutType { edit_index };
                SettingsAction::None
            }
            KeyCode::Char('a' | 'A') if key.modifiers.is_empty() => {
                self.keymaps_mode = KeymapsMode::ShortcutType { edit_index: None };
                SettingsAction::None
            }
            KeyCode::Char('x' | 'X') if key.modifiers.is_empty() => self.remove_keymap_binding(),
            KeyCode::Char('c' | 'C') if key.modifiers.is_empty() => self.clear_keymap_bindings(),
            KeyCode::Char('r' | 'R') if key.modifiers.is_empty() => self.reset_keymap_action(),
            _ => SettingsAction::None,
        }
    }

    fn handle_keymap_shortcut_type_key(
        &mut self,
        key: KeyEvent,
        edit_index: Option<usize>,
    ) -> SettingsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => {
                self.keymaps_mode = KeymapsMode::Detail;
                SettingsAction::None
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.keymap_shortcut_kind_cursor = move_index(
                    self.keymap_shortcut_kind_cursor,
                    ShortcutKind::all().len(),
                    -1,
                );
                SettingsAction::None
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.keymap_shortcut_kind_cursor = move_index(
                    self.keymap_shortcut_kind_cursor,
                    ShortcutKind::all().len(),
                    1,
                );
                SettingsAction::None
            }
            KeyCode::Enter | KeyCode::Right => {
                let kind = self.shortcut_kind_cursor_kind();
                self.keymaps_mode = KeymapsMode::Capture {
                    edit_index,
                    kind,
                    strokes: Vec::new(),
                };
                self.status = match kind {
                    ShortcutKind::Combination => {
                        "Listening for one key combination • Esc cancel".into()
                    }
                    ShortcutKind::Chord => "Listening for chord suffix key • Esc cancel".into(),
                };
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn handle_keymap_capture_key(
        &mut self,
        key: KeyEvent,
        edit_index: Option<usize>,
        kind: ShortcutKind,
        strokes: &mut Vec<KeyStroke>,
    ) -> SettingsAction {
        let Some(stroke) = KeyStroke::from_event(key) else {
            self.status = "Unsupported terminal key event".into();
            return SettingsAction::None;
        };
        if stroke.is_escape() {
            self.keymaps_mode = KeymapsMode::ShortcutType { edit_index };
            self.status = "Shortcut capture canceled".into();
            return SettingsAction::None;
        }
        match kind {
            ShortcutKind::Combination => {
                strokes.push(stroke);
                self.apply_captured_key_sequence(edit_index, strokes)
            }
            ShortcutKind::Chord => {
                let sequence = KeySequence::chord(self.keymap.chord_key, stroke);
                self.apply_key_sequence(edit_index, sequence)
            }
        }
    }

    fn handle_chord_key_capture_key(&mut self, key: KeyEvent) -> SettingsAction {
        let Some(stroke) = KeyStroke::from_event(key) else {
            self.status = "Unsupported terminal key event".into();
            return SettingsAction::None;
        };
        if stroke.is_escape() {
            self.keymaps_mode = KeymapsMode::List;
            self.status = "Chord key capture canceled".into();
            return SettingsAction::None;
        }
        if stroke.is_plain_text_key() {
            self.status = "Global chord key cannot be plain text; use Ctrl/Alt/F-key to avoid blocking typing".into();
            return SettingsAction::None;
        }
        if let Some(conflict) = self.keymap.chord_key_conflict(stroke) {
            self.status = format!(
                "{} conflicts with {} ({})",
                stroke,
                conflict.info().label,
                conflict.id()
            );
            return SettingsAction::None;
        }
        self.keymap.set_chord_key(stroke);
        self.keymaps_mode = KeymapsMode::List;
        self.status = format!("Global chord key set to {stroke}");
        SettingsAction::SetKeymap(self.keymap.clone())
    }

    fn handle_keymap_preset_select_key(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc | KeyCode::Left | KeyCode::Backspace => {
                self.keymaps_mode = KeymapsMode::List;
                SettingsAction::None
            }
            KeyCode::Up | KeyCode::Char('k') if key.modifiers.is_empty() => {
                self.keymap_preset_cursor =
                    move_index(self.keymap_preset_cursor, KeymapPreset::all().len(), -1);
                SettingsAction::None
            }
            KeyCode::Down | KeyCode::Char('j') if key.modifiers.is_empty() => {
                self.keymap_preset_cursor =
                    move_index(self.keymap_preset_cursor, KeymapPreset::all().len(), 1);
                SettingsAction::None
            }
            KeyCode::Enter | KeyCode::Right => {
                let preset = self.keymap_preset_cursor_preset();
                self.keymaps_mode = KeymapsMode::PresetConfirm { preset };
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn handle_keymap_preset_confirm_key(
        &mut self,
        key: KeyEvent,
        preset: KeymapPreset,
    ) -> SettingsAction {
        match key.code {
            KeyCode::Char('y' | 'Y') if key.modifiers.is_empty() => {
                self.keymap = KeymapConfig::for_preset(preset);
                self.keymaps_mode = KeymapsMode::List;
                self.keymap_binding_cursor = 0;
                self.status = format!("Reset all keybinds to {} preset", preset.label());
                SettingsAction::SetKeymap(self.keymap.clone())
            }
            KeyCode::Char('n' | 'N') | KeyCode::Esc if key.modifiers.is_empty() => {
                self.keymaps_mode = KeymapsMode::PresetSelect;
                self.status = "Preset reset canceled".into();
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn open_keymap_detail(&mut self) -> SettingsAction {
        self.keymaps_mode = KeymapsMode::Detail;
        self.keymap_binding_cursor = self
            .keymap_binding_cursor
            .min(self.current_keymap_bindings().len().saturating_sub(1));
        SettingsAction::None
    }

    fn remove_keymap_binding(&mut self) -> SettingsAction {
        let action = self.current_keymap_action();
        let mut bindings = self.keymap.bindings_for(action);
        if bindings.is_empty() {
            self.status = "No shortcut to remove".into();
            return SettingsAction::None;
        }
        let index = self
            .keymap_binding_cursor
            .min(bindings.len().saturating_sub(1));
        let removed = bindings.remove(index);
        self.keymap.set_bindings(action, bindings);
        self.keymap_binding_cursor = self
            .keymap_binding_cursor
            .min(self.current_keymap_bindings().len().saturating_sub(1));
        self.status = format!("Removed {} from {}", removed, action.info().label);
        SettingsAction::SetKeymap(self.keymap.clone())
    }

    fn clear_keymap_bindings(&mut self) -> SettingsAction {
        let action = self.current_keymap_action();
        self.keymap.set_bindings(action, Vec::new());
        self.keymap_binding_cursor = 0;
        self.status = format!("{} is now unassigned", action.info().label);
        SettingsAction::SetKeymap(self.keymap.clone())
    }

    fn reset_keymap_action(&mut self) -> SettingsAction {
        let action = self.current_keymap_action();
        self.keymap.reset_action(action);
        self.keymap_binding_cursor = 0;
        self.status = format!("Reset {} to preset default", action.info().label);
        SettingsAction::SetKeymap(self.keymap.clone())
    }

    fn apply_captured_key_sequence(
        &mut self,
        edit_index: Option<usize>,
        strokes: &[KeyStroke],
    ) -> SettingsAction {
        let Some(sequence) = KeySequence::new(strokes.to_vec()) else {
            self.status = "Shortcut cannot be empty; use Clear to unassign".into();
            return SettingsAction::None;
        };
        self.apply_key_sequence(edit_index, sequence)
    }

    fn apply_key_sequence(
        &mut self,
        edit_index: Option<usize>,
        sequence: KeySequence,
    ) -> SettingsAction {
        let action = self.current_keymap_action();
        if let Some(conflict) = self.keymap.conflict_for(action, edit_index, &sequence) {
            self.status = if conflict == action {
                format!(
                    "{} is already assigned to {}",
                    sequence,
                    action.info().label
                )
            } else {
                format!(
                    "{} conflicts with {} ({})",
                    sequence,
                    conflict.info().label,
                    conflict.id()
                )
            };
            self.keymaps_mode = KeymapsMode::ShortcutType { edit_index };
            return SettingsAction::None;
        }
        let mut bindings = self.keymap.bindings_for(action);
        if let Some(index) = edit_index.filter(|index| *index < bindings.len()) {
            bindings[index] = sequence.clone();
            self.keymap_binding_cursor = index;
        } else {
            bindings.push(sequence.clone());
            self.keymap_binding_cursor = bindings.len().saturating_sub(1);
        }
        self.keymap.set_bindings(action, bindings);
        self.keymaps_mode = KeymapsMode::Detail;
        self.status = format!("Set {} to {}", action.info().label, sequence);
        SettingsAction::SetKeymap(self.keymap.clone())
    }

    fn handle_model_page_key(&mut self, key: KeyEvent) -> SettingsAction {
        // Esc/Backspace/Left at top level means "return to menu"
        let is_escape = matches!(key.code, KeyCode::Esc);
        let is_back = matches!(key.code, KeyCode::Backspace | KeyCode::Left)
            && !self.model_selector.search_active;
        if is_escape {
            self.model_selector.cancel();
            self.page = SettingsPage::Menu;
            return SettingsAction::None;
        }
        if is_back {
            self.model_selector.cancel();
            self.page = SettingsPage::Menu;
            return SettingsAction::None;
        }
        match self.model_selector.handle_key(key) {
            ModelSelectorAction::Select { id } => {
                self.model_selector.initial_model = id.clone();
                self.clamp_thinking_to_selected_model();
                self.page = SettingsPage::Menu;
                SettingsAction::SetModel(id)
            }
            ModelSelectorAction::Cancel => {
                // Cancel was already handled above for Esc/Back; but the selector
                // can also cancel on Enter with the same model.
                self.page = SettingsPage::Menu;
                SettingsAction::None
            }
            ModelSelectorAction::None => SettingsAction::None,
        }
    }

    fn apply_model_from_selector(&mut self) -> SettingsAction {
        match self
            .model_selector
            .handle_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
        {
            ModelSelectorAction::Select { id } => {
                self.model_selector.initial_model = id.clone();
                self.clamp_thinking_to_selected_model();
                self.page = SettingsPage::Menu;
                SettingsAction::SetModel(id)
            }
            ModelSelectorAction::Cancel => {
                self.page = SettingsPage::Menu;
                SettingsAction::None
            }
            ModelSelectorAction::None => SettingsAction::None,
        }
    }

    /// Handle keys for the sub-page model picker (Notify summary model, Compaction model).
    /// `return_page` is the page to go back to on cancel/select.
    /// `make_action` constructs the appropriate [`SettingsAction`] from the selected model id.
    fn handle_sub_model_picker_key(
        &mut self,
        key: KeyEvent,
        return_page: SettingsPage,
        make_action: impl FnOnce(String, &Self) -> SettingsAction,
    ) -> SettingsAction {
        let is_escape = matches!(key.code, KeyCode::Esc);
        let is_back = matches!(key.code, KeyCode::Backspace | KeyCode::Left)
            && !self.sub_model_picker.search_active;
        if is_escape || is_back {
            self.sub_model_picker.cancel();
            self.page = return_page;
            return SettingsAction::None;
        }
        match self.sub_model_picker.handle_key(key) {
            ModelSelectorAction::Select { id } => {
                self.sub_model_picker.initial_model = id.clone();
                let action = make_action(id, self);
                self.page = return_page;
                action
            }
            ModelSelectorAction::Cancel => {
                self.page = return_page;
                SettingsAction::None
            }
            ModelSelectorAction::None => SettingsAction::None,
        }
    }

    fn close_or_return_to_menu(&mut self) -> SettingsAction {
        if self.page == SettingsPage::Menu {
            self.clear_theme_preview();
            SettingsAction::Close
        } else if self.page == SettingsPage::Models {
            self.model_selector.cancel();
            self.page = SettingsPage::Menu;
            SettingsAction::None
        } else if self.page == SettingsPage::NotifyModelPicker {
            self.sub_model_picker.cancel();
            self.page = SettingsPage::Notify;
            SettingsAction::None
        } else if self.page == SettingsPage::CompactionModelPicker {
            self.sub_model_picker.cancel();
            self.page = SettingsPage::Compaction;
            SettingsAction::None
        } else {
            let was_theme = self.page == SettingsPage::Theme;
            self.notify.edit = None;
            self.page = SettingsPage::Menu;
            if was_theme {
                SettingsAction::ClearThemePreview
            } else {
                SettingsAction::None
            }
        }
    }

    fn open_current_menu_item(&mut self) -> SettingsAction {
        if self.current_menu_item() == SettingsMenuItem::Extensions {
            return SettingsAction::OpenExtensions;
        }
        self.page = self.current_menu_item().page();
        SettingsAction::None
    }

    fn apply_or_open(&mut self) -> SettingsAction {
        match self.page {
            SettingsPage::Menu => self.open_current_menu_item(),
            SettingsPage::Models => self.apply_model_from_selector(),
            SettingsPage::Thinking => self.apply_thinking_level(),
            SettingsPage::Collapse => self.apply_collapse_mode(),
            SettingsPage::ChatStyle => self.apply_chat_style(),
            SettingsPage::Tools => self.toggle_tool(ToolSettingsScope::Project),
            SettingsPage::Auth => SettingsAction::None,
            SettingsPage::Keymaps => self.open_keymap_detail(),
            SettingsPage::Theme => self.preview_selected_theme(),
            SettingsPage::Notify => self.apply_notify_row(),
            SettingsPage::Compaction => {
                if self.compact.cursor == 3 {
                    self.open_compaction_model_picker()
                } else {
                    self.toggle_compaction_field()
                }
            }
            SettingsPage::Extensions => SettingsAction::OpenExtensions,
            SettingsPage::NotifyModelPicker => self.handle_sub_model_picker_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                SettingsPage::Notify,
                |id, settings| SettingsAction::SetNotifyField {
                    scope: settings.notify.scope,
                    field: NotifyField::SummaryModel,
                    value: Some(id),
                },
            ),
            SettingsPage::CompactionModelPicker => self.handle_sub_model_picker_key(
                KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
                SettingsPage::Compaction,
                |id, _settings| SettingsAction::SetCompactModel { id },
            ),
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.page {
            SettingsPage::Menu => {
                self.menu_cursor = move_index(self.menu_cursor, self.menu_items().len(), delta);
            }
            SettingsPage::Models => {
                if delta < 0 {
                    self.model_selector
                        .handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                } else {
                    self.model_selector
                        .handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                }
            }
            SettingsPage::Thinking => {
                let levels = self.thinking_levels();
                self.thinking_cursor = move_index(self.thinking_cursor, levels.len(), delta);
            }
            SettingsPage::Collapse => {
                self.collapse_cursor = move_index(self.collapse_cursor, 2, delta);
            }
            SettingsPage::ChatStyle => {
                self.chat_style_cursor =
                    move_index(self.chat_style_cursor, ChatStyle::all().len(), delta);
            }
            SettingsPage::Tools => {
                self.tool_cursor = move_index(self.tool_cursor, self.tools.len(), delta);
            }
            SettingsPage::Auth => {
                self.auth_cursor = move_index(self.auth_cursor, self.auth_items.len(), delta);
            }
            SettingsPage::Theme => {
                self.theme_cursor = move_index(self.theme_cursor, self.theme_options.len(), delta);
            }
            SettingsPage::Notify => {
                self.notify.cursor =
                    move_index(self.notify.cursor, NotifySettingsState::ROWS.len(), delta);
            }
            SettingsPage::Compaction => {
                self.compact.cursor = move_index(
                    self.compact.cursor,
                    CompactionSettingsState::ROW_COUNT,
                    delta,
                );
            }
            SettingsPage::Extensions => {}
            SettingsPage::NotifyModelPicker | SettingsPage::CompactionModelPicker => {
                if delta < 0 {
                    self.sub_model_picker
                        .handle_key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE));
                } else {
                    self.sub_model_picker
                        .handle_key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
                }
            }
            SettingsPage::Keymaps => {
                self.keymap_cursor = move_index(self.keymap_cursor, key_action_rows().len(), delta);
                self.keymap_binding_cursor = self
                    .keymap_binding_cursor
                    .min(self.current_keymap_bindings().len().saturating_sub(1));
            }
        }
    }

    fn apply_thinking_level(&mut self) -> SettingsAction {
        let levels = self.thinking_levels();
        let level = levels
            .get(self.thinking_cursor)
            .copied()
            .unwrap_or(ThinkingLevel::Off);
        if self.selected_thinking_level == level {
            return SettingsAction::None;
        }
        self.selected_thinking_level = level;
        SettingsAction::SetThinkingLevel(level)
    }

    fn apply_collapse_mode(&mut self) -> SettingsAction {
        match self.collapse_cursor {
            0 => {
                self.thinking_collapse_mode = self.thinking_collapse_mode.next();
                SettingsAction::SetCollapseMode(
                    CollapseTarget::Thinking,
                    self.thinking_collapse_mode,
                )
            }
            _ => {
                self.tool_collapse_mode = self.tool_collapse_mode.next();
                SettingsAction::SetCollapseMode(CollapseTarget::Tool, self.tool_collapse_mode)
            }
        }
    }

    fn apply_chat_style(&mut self) -> SettingsAction {
        let style = ChatStyle::all()
            .get(self.chat_style_cursor)
            .copied()
            .unwrap_or(ChatStyle::Chat);
        if self.chat_style == style {
            return SettingsAction::None;
        }
        self.chat_style = style;
        SettingsAction::SetChatStyle(style)
    }

    fn toggle_tool(&mut self, scope: ToolSettingsScope) -> SettingsAction {
        let Some(tool) = self.tools.get_mut(self.tool_cursor) else {
            return SettingsAction::None;
        };
        let enabled = !tool.enabled(scope);
        tool.set_enabled(scope, enabled);
        SettingsAction::SetToolEnabled {
            name: tool.name.clone(),
            scope,
            enabled,
        }
    }

    fn preview_selected_theme(&self) -> SettingsAction {
        let Some(id) = self.selected_theme_id() else {
            return SettingsAction::None;
        };
        SettingsAction::PreviewTheme { id }
    }

    fn apply_theme(&mut self, scope: ToolSettingsScope) -> SettingsAction {
        let Some(id) = self.selected_theme_id() else {
            return SettingsAction::None;
        };
        SettingsAction::SetTheme { id, scope }
    }

    fn clamp_thinking_to_selected_model(&mut self) {
        let levels = self.thinking_levels();
        if !levels.contains(&self.selected_thinking_level) {
            self.selected_thinking_level = ThinkingLevel::Off;
        }
        self.thinking_cursor = thinking_index(self.selected_thinking_level, &levels);
    }
}

#[must_use]
pub fn all_thinking_levels() -> Vec<ThinkingLevel> {
    vec![
        ThinkingLevel::Off,
        ThinkingLevel::Minimal,
        ThinkingLevel::Low,
        ThinkingLevel::Medium,
        ThinkingLevel::High,
        ThinkingLevel::XHigh,
    ]
}

#[must_use]
pub fn collapse_mode_label(mode: CollapseMode) -> &'static str {
    match mode {
        CollapseMode::Full => "Full",
        CollapseMode::Truncate => "Truncate",
        CollapseMode::Collapse => "Collapse",
    }
}

#[must_use]
pub fn chat_style_label(style: ChatStyle) -> &'static str {
    match style {
        ChatStyle::Chat => "Chat",
        ChatStyle::Agentic => "Agentic",
        ChatStyle::Minimal => "Minimal",
    }
}

#[must_use]
pub fn chat_style_value(style: ChatStyle) -> &'static str {
    match style {
        ChatStyle::Chat => "chat",
        ChatStyle::Agentic => "agentic",
        ChatStyle::Minimal => "minimal",
    }
}

#[must_use]
pub fn parse_chat_style(value: &str) -> Option<ChatStyle> {
    match value {
        "chat" => Some(ChatStyle::Chat),
        "agentic" => Some(ChatStyle::Agentic),
        "minimal" => Some(ChatStyle::Minimal),
        _ => None,
    }
}

pub fn thinking_label(level: ThinkingLevel) -> &'static str {
    match level {
        ThinkingLevel::Off => "Off",
        ThinkingLevel::Minimal => "Minimal",
        ThinkingLevel::Low => "Low",
        ThinkingLevel::Medium => "Medium",
        ThinkingLevel::High => "High",
        ThinkingLevel::XHigh => "X High",
    }
}

fn normalize_thinking_levels(mut levels: Vec<ThinkingLevel>) -> Vec<ThinkingLevel> {
    if levels.is_empty() {
        levels.push(ThinkingLevel::Off);
    }
    if !levels.contains(&ThinkingLevel::Off) {
        levels.insert(0, ThinkingLevel::Off);
    }
    levels.dedup();
    levels
}

fn thinking_index(level: ThinkingLevel, levels: &[ThinkingLevel]) -> usize {
    levels.iter().position(|item| *item == level).unwrap_or(0)
}

fn chat_style_index(style: ChatStyle) -> usize {
    ChatStyle::all()
        .iter()
        .position(|item| *item == style)
        .unwrap_or(0)
}

fn on_off(value: bool) -> &'static str {
    if value {
        "ON"
    } else {
        "OFF"
    }
}

fn display_tool_name(name: &str) -> String {
    name.split(['_', '-'])
        .filter(|part| !part.is_empty())
        .map(|part| {
            let mut chars = part.chars();
            chars.next().map_or_else(String::new, |first| {
                format!("{}{}", first.to_uppercase(), chars.as_str())
            })
        })
        .collect::<Vec<_>>()
        .join(" ")
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

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyModifiers;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn settings_opens_nested_model_selection_before_applying_model() {
        let mut settings = SettingsState::new("a", ThinkingLevel::High);
        settings.set_models(
            vec![
                ModelOption::new("a").with_thinking_levels(all_thinking_levels()),
                ModelOption::new("b"),
            ],
            "loaded",
        );
        assert_eq!(settings.page, SettingsPage::Menu);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::None
        );
        assert_eq!(settings.page, SettingsPage::Models);
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetModel("b".into())
        );
        assert_eq!(settings.selected_thinking_level, ThinkingLevel::Off);
        assert_eq!(settings.thinking_levels(), vec![ThinkingLevel::Off]);
    }

    #[test]
    fn settings_child_page_escape_returns_to_menu() {
        let mut settings = SettingsState::new("a", ThinkingLevel::Off);
        settings.handle_key(key(KeyCode::Enter));
        assert_eq!(settings.page, SettingsPage::Models);
        assert_eq!(settings.handle_key(key(KeyCode::Esc)), SettingsAction::None);
        assert_eq!(settings.page, SettingsPage::Menu);
        assert_eq!(
            settings.handle_key(key(KeyCode::Esc)),
            SettingsAction::Close
        );
    }

    #[test]
    fn thinking_selection_uses_nested_thinking_page() {
        let mut settings = SettingsState::new("a", ThinkingLevel::Off);
        settings.set_models(
            vec![ModelOption::new("a").with_thinking_levels(vec![
                ThinkingLevel::Off,
                ThinkingLevel::Low,
                ThinkingLevel::High,
            ])],
            "loaded",
        );
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(
            settings.current_menu_item(),
            SettingsMenuItem::ThinkingLevel
        );
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::None
        );
        assert_eq!(settings.page, SettingsPage::Thinking);
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetThinkingLevel(ThinkingLevel::Low)
        );
    }

    #[test]
    fn collapse_mode_cycles_from_settings_page() {
        let mut settings = SettingsState::new("a", ThinkingLevel::Off);
        settings.handle_key(key(KeyCode::Down));
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(settings.current_menu_item(), SettingsMenuItem::CollapseMode);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::None
        );
        assert_eq!(settings.page, SettingsPage::Collapse);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetCollapseMode(CollapseTarget::Thinking, CollapseMode::Truncate)
        );
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetCollapseMode(CollapseTarget::Tool, CollapseMode::Truncate)
        );
    }

    #[test]
    fn chat_style_selection_uses_nested_page() {
        let mut settings = SettingsState::new("a", ThinkingLevel::Off);
        for _ in 0..3 {
            settings.handle_key(key(KeyCode::Down));
        }
        assert_eq!(settings.current_menu_item(), SettingsMenuItem::ChatStyle);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::None
        );
        assert_eq!(settings.page, SettingsPage::ChatStyle);
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetChatStyle(ChatStyle::Agentic)
        );
        assert_eq!(settings.chat_style, ChatStyle::Agentic);
    }

    #[test]
    fn tools_page_lists_registered_tools_and_toggles_each_scope() {
        let mut settings = SettingsState::new("a", ThinkingLevel::Off);
        settings.set_tools(vec![
            ToolSettingsItem::global("bash"),
            ToolSettingsItem::global("set_session_title").with_scopes(false, false),
        ]);
        for _ in 0..4 {
            settings.handle_key(key(KeyCode::Down));
        }
        assert_eq!(settings.current_menu_item(), SettingsMenuItem::Tools);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::None
        );
        assert_eq!(settings.page, SettingsPage::Tools);
        assert_eq!(
            settings.tools[0].label(),
            "Bash - [Global - ON] [Project - OFF]"
        );
        assert_eq!(
            settings.tools[1].label(),
            "Set Session Title - [Global - OFF] [Project - OFF]"
        );
        assert_eq!(
            settings.handle_key(key(KeyCode::Char('g'))),
            SettingsAction::SetToolEnabled {
                name: "bash".into(),
                scope: ToolSettingsScope::Global,
                enabled: false,
            }
        );
        assert!(!settings.tools[0].global_enabled);
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(settings.tool_cursor, 1);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetToolEnabled {
                name: "set_session_title".into(),
                scope: ToolSettingsScope::Project,
                enabled: true,
            }
        );
        assert!(settings.tools[1].project_enabled);
    }

    #[test]
    fn extensions_menu_item_requests_extension_manager() {
        let mut settings = SettingsState::new("a", ThinkingLevel::Off);
        for _ in 0..settings.menu_items().len() {
            if settings.current_menu_item() == SettingsMenuItem::Extensions {
                break;
            }
            settings.handle_key(key(KeyCode::Down));
        }

        assert_eq!(settings.current_menu_item(), SettingsMenuItem::Extensions);
        assert_eq!(settings.page, SettingsPage::Menu);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::OpenExtensions
        );
        assert_eq!(settings.page, SettingsPage::Menu);
    }

    #[test]
    fn notify_page_is_registered_when_available_and_edits_project_topic() {
        let mut settings = SettingsState::new("a", ThinkingLevel::Off);
        assert!(!settings.menu_items().contains(&SettingsMenuItem::Notify));
        settings.set_notify_available(true);
        assert!(settings.menu_items().contains(&SettingsMenuItem::Notify));
        while settings.current_menu_item() != SettingsMenuItem::Notify {
            settings.handle_key(key(KeyCode::Down));
        }
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::None
        );
        assert_eq!(settings.page, SettingsPage::Notify);
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetNotifyEnabled {
                scope: ToolSettingsScope::Project,
                enabled: true,
            }
        );
        settings.handle_key(key(KeyCode::Down));
        settings.handle_key(key(KeyCode::Down));
        assert_eq!(
            NotifySettingsState::ROWS[settings.notify.cursor],
            NotifyRow::Topic
        );
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::None
        );
        assert!(settings.notify.edit.is_some());
        for ch in "oino-topic".chars() {
            settings.handle_key(key(KeyCode::Char(ch)));
        }
        assert_eq!(
            settings.handle_key(key(KeyCode::Enter)),
            SettingsAction::SetNotifyField {
                scope: ToolSettingsScope::Project,
                field: NotifyField::Topic,
                value: Some("oino-topic".into()),
            }
        );
    }

    #[test]
    fn model_catalog_refresh_preserves_browsing_cursor_on_model_page() {
        let mut settings = SettingsState::new("model-a", ThinkingLevel::Off);
        settings.set_models(
            vec![ModelOption::new("model-a"), ModelOption::new("model-b")],
            "loaded",
        );
        settings.page = SettingsPage::Models;
        settings.model_selector.cursor = 1;

        settings.set_models(
            vec![
                ModelOption::new("model-a"),
                ModelOption::new("model-b"),
                ModelOption::new("model-c"),
            ],
            "refreshed",
        );

        assert_eq!(settings.model_selector.cursor, 1);
        assert_eq!(
            settings.model_selector.models[settings.model_selector.cursor].id,
            "model-b"
        );
    }

    #[test]
    fn model_list_sorts_definitely_unconfigured_models_to_bottom() {
        let mut settings = SettingsState::new("router:openai/a", ThinkingLevel::Off);
        settings.set_models(
            vec![
                ModelOption::new("router:openai/a")
                    .with_availability(ModelAvailability::NeedsProviderKey),
                ModelOption::new("router:anthropic/b")
                    .with_availability(ModelAvailability::Configured),
                ModelOption::new("extension:model").with_availability(ModelAvailability::Unknown),
            ],
            "loaded",
        );

        assert_eq!(settings.model_selector.models[0].id, "router:anthropic/b");
        assert_eq!(settings.model_selector.models[1].id, "extension:model");
        assert_eq!(settings.model_selector.models[2].id, "router:openai/a");
    }

    #[test]
    fn model_filter_prefilter_checks_provider_label_id_and_display_name() {
        let models = vec![
            ModelOption::new("openrouter:a/alpha"),
            ModelOption::new("openrouter:b/bravo").with_display_name("Displayed Model"),
            ModelOption::new("router:kr/test")
                .with_display_name("KR Test")
                .with_provider_label("OmniRoute extension"),
        ];

        assert_eq!(
            crate::model_selector::model_filter_candidate_indices(&models, "displayed"),
            vec![1]
        );
        assert_eq!(
            crate::model_selector::model_filter_candidate_indices(&models, "alpha"),
            vec![0]
        );
        assert_eq!(
            crate::model_selector::model_filter_candidate_indices(&models, "extension"),
            vec![2]
        );
    }

    #[test]
    fn model_selection_supports_slash_search_and_escape_clear() {
        let mut settings = SettingsState::new("openai/gpt", ThinkingLevel::Off);
        settings.set_models(
            vec![
                ModelOption::new("anthropic/claude"),
                ModelOption::new("openai/gpt"),
                ModelOption::new("google/gemini"),
            ],
            "loaded",
        );
        settings.page = SettingsPage::Models;
        assert_eq!(
            settings.handle_key(key(KeyCode::Char('/'))),
            SettingsAction::None
        );
        assert!(settings.model_selector.search_active);
        assert_eq!(
            settings.handle_key(key(KeyCode::Char('g'))),
            SettingsAction::None
        );
        assert_eq!(settings.model_selector.search, "g");
        assert_eq!(settings.model_selector.filtered_indices.len(), 2);
        assert!(settings.model_selector.filtered_indices.contains(&1));
        assert!(settings.model_selector.filtered_indices.contains(&2));
        assert!(matches!(settings.model_selector.cursor, 1 | 2));
        assert_eq!(settings.handle_key(key(KeyCode::Esc)), SettingsAction::None);
        assert!(!settings.model_selector.search_active);
        assert_eq!(settings.model_selector.search, "");
        assert_eq!(settings.model_selector.cursor, 1);
    }
}
