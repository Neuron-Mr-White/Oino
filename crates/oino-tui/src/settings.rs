#![forbid(unsafe_code)]

use crate::fuzzy::{fuzzy_indices, FuzzyMode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use oino_types::ThinkingLevel;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub id: String,
    pub display_name: String,
    pub thinking_levels: Vec<ThinkingLevel>,
}

impl ModelOption {
    #[must_use]
    pub fn new(id: impl Into<String>) -> Self {
        let id = id.into();
        Self {
            display_name: id.clone(),
            id,
            thinking_levels: vec![ThinkingLevel::Off],
        }
    }

    #[must_use]
    pub fn with_display_name(mut self, display_name: impl Into<String>) -> Self {
        self.display_name = display_name.into();
        self
    }

    #[must_use]
    pub fn with_thinking_levels(mut self, thinking_levels: Vec<ThinkingLevel>) -> Self {
        self.thinking_levels = normalize_thinking_levels(thinking_levels);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Menu,
    Models,
    Thinking,
    Collapse,
    ChatStyle,
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
}

impl SettingsMenuItem {
    #[must_use]
    pub fn label(self) -> &'static str {
        match self {
            Self::ModelSelection => "Model Selection",
            Self::ThinkingLevel => "Thinking Level",
            Self::CollapseMode => "Collapse Mode",
            Self::ChatStyle => "Chat Style",
        }
    }

    #[must_use]
    pub fn page(self) -> SettingsPage {
        match self {
            Self::ModelSelection => SettingsPage::Models,
            Self::ThinkingLevel => SettingsPage::Thinking,
            Self::CollapseMode => SettingsPage::Collapse,
            Self::ChatStyle => SettingsPage::ChatStyle,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsState {
    pub models: Vec<ModelOption>,
    pub selected_model: String,
    pub selected_thinking_level: ThinkingLevel,
    pub page: SettingsPage,
    pub menu_cursor: usize,
    pub model_cursor: usize,
    pub thinking_cursor: usize,
    pub collapse_cursor: usize,
    pub chat_style_cursor: usize,
    pub thinking_collapse_mode: CollapseMode,
    pub tool_collapse_mode: CollapseMode,
    pub chat_style: ChatStyle,
    pub model_search: String,
    pub model_search_active: bool,
    pub filtered_model_indices: Vec<usize>,
    pub status: String,
    pub refreshing: bool,
}

impl SettingsState {
    #[must_use]
    pub fn new(model: impl Into<String>, thinking_level: ThinkingLevel) -> Self {
        Self {
            models: Vec::new(),
            selected_model: model.into(),
            selected_thinking_level: thinking_level,
            page: SettingsPage::Menu,
            menu_cursor: 0,
            model_cursor: 0,
            thinking_cursor: thinking_index(thinking_level, &all_thinking_levels()),
            collapse_cursor: 0,
            chat_style_cursor: 0,
            thinking_collapse_mode: CollapseMode::Full,
            tool_collapse_mode: CollapseMode::Full,
            chat_style: ChatStyle::Chat,
            model_search: String::new(),
            model_search_active: false,
            filtered_model_indices: Vec::new(),
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
        self.model_search_active = false;
        self.model_search.clear();
        self.refresh_model_filter();
    }

    pub fn open_thinking_level(&mut self) {
        self.page = SettingsPage::Thinking;
        self.thinking_cursor =
            thinking_index(self.selected_thinking_level, &self.thinking_levels());
    }

    pub fn set_models(&mut self, models: Vec<ModelOption>, status: impl Into<String>) {
        self.models = models;
        self.status = status.into();
        self.model_cursor = self
            .models
            .iter()
            .position(|model| model.id == self.selected_model)
            .unwrap_or_else(|| self.model_cursor.min(self.models.len().saturating_sub(1)));
        self.clamp_thinking_to_selected_model();
        self.refresh_model_filter();
    }

    pub fn set_refreshing(&mut self, refreshing: bool) {
        self.refreshing = refreshing;
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

    pub fn open_chat_style(&mut self) {
        self.page = SettingsPage::ChatStyle;
        self.chat_style_cursor = chat_style_index(self.chat_style);
    }

    pub fn select_model_identifier(&mut self, model: &str) {
        self.selected_model = model.to_string();
        if let Some(index) = self.models.iter().position(|option| option.id == model) {
            self.model_cursor = index;
        }
        self.clamp_thinking_to_selected_model();
    }

    pub fn select_thinking_level(&mut self, level: ThinkingLevel) {
        self.selected_thinking_level = level;
        self.clamp_thinking_to_selected_model();
    }

    #[must_use]
    pub fn menu_items(&self) -> [SettingsMenuItem; 4] {
        [
            SettingsMenuItem::ModelSelection,
            SettingsMenuItem::ThinkingLevel,
            SettingsMenuItem::CollapseMode,
            SettingsMenuItem::ChatStyle,
        ]
    }

    #[must_use]
    pub fn current_menu_item(&self) -> SettingsMenuItem {
        self.menu_items()
            .get(self.menu_cursor)
            .copied()
            .unwrap_or(SettingsMenuItem::ModelSelection)
    }

    #[must_use]
    pub fn thinking_levels(&self) -> Vec<ThinkingLevel> {
        self.models
            .iter()
            .find(|model| model.id == self.selected_model)
            .map_or_else(all_thinking_levels, |model| {
                normalize_thinking_levels(model.thinking_levels.clone())
            })
    }

    #[must_use]
    pub fn selected_model_label(&self) -> &str {
        self.models
            .iter()
            .find(|model| model.id == self.selected_model)
            .map_or(self.selected_model.as_str(), |model| {
                model.display_name.as_str()
            })
    }

    #[must_use]
    pub fn filtered_model_indices(&self) -> &[usize] {
        &self.filtered_model_indices
    }

    #[must_use]
    pub fn model_cursor_filtered_position(&self) -> usize {
        self.filtered_model_indices
            .iter()
            .position(|index| *index == self.model_cursor)
            .unwrap_or(0)
    }

    pub fn handle_key(&mut self, key: KeyEvent) -> SettingsAction {
        if self.page == SettingsPage::Models && self.model_search_active {
            return self.handle_model_search_key(key);
        }

        match key.code {
            KeyCode::Esc => self.close_or_return_to_menu(),
            KeyCode::Backspace | KeyCode::Left if self.page != SettingsPage::Menu => {
                self.page = SettingsPage::Menu;
                SettingsAction::None
            }
            KeyCode::Right if self.page == SettingsPage::Menu => self.open_current_menu_item(),
            KeyCode::Right if self.page == SettingsPage::Collapse => self.apply_collapse_mode(),
            KeyCode::Char('/') if self.page == SettingsPage::Models && key.modifiers.is_empty() => {
                self.model_search_active = true;
                self.model_search.clear();
                self.refresh_model_filter();
                SettingsAction::None
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

    fn handle_model_search_key(&mut self, key: KeyEvent) -> SettingsAction {
        match key.code {
            KeyCode::Esc => {
                self.model_search_active = false;
                self.model_search.clear();
                self.refresh_model_filter();
                self.model_cursor = self
                    .models
                    .iter()
                    .position(|model| model.id == self.selected_model)
                    .unwrap_or_else(|| self.model_cursor.min(self.models.len().saturating_sub(1)));
                SettingsAction::None
            }
            KeyCode::Enter => {
                self.model_search_active = false;
                SettingsAction::None
            }
            KeyCode::Backspace => {
                self.model_search.pop();
                self.refresh_model_filter();
                SettingsAction::None
            }
            KeyCode::Up => {
                self.move_model_cursor_filtered(-1);
                SettingsAction::None
            }
            KeyCode::Down => {
                self.move_model_cursor_filtered(1);
                SettingsAction::None
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.model_search.push(ch);
                self.refresh_model_filter();
                SettingsAction::None
            }
            _ => SettingsAction::None,
        }
    }

    fn close_or_return_to_menu(&mut self) -> SettingsAction {
        if self.page == SettingsPage::Menu {
            SettingsAction::Close
        } else {
            self.model_search_active = false;
            self.model_search.clear();
            self.refresh_model_filter();
            self.page = SettingsPage::Menu;
            SettingsAction::None
        }
    }

    fn open_current_menu_item(&mut self) -> SettingsAction {
        self.page = self.current_menu_item().page();
        SettingsAction::None
    }

    fn apply_or_open(&mut self) -> SettingsAction {
        match self.page {
            SettingsPage::Menu => self.open_current_menu_item(),
            SettingsPage::Models => self.apply_model(),
            SettingsPage::Thinking => self.apply_thinking_level(),
            SettingsPage::Collapse => self.apply_collapse_mode(),
            SettingsPage::ChatStyle => self.apply_chat_style(),
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        match self.page {
            SettingsPage::Menu => {
                self.menu_cursor = move_index(self.menu_cursor, self.menu_items().len(), delta);
            }
            SettingsPage::Models => {
                self.move_model_cursor_filtered(delta);
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
        }
    }

    fn move_model_cursor_filtered(&mut self, delta: isize) {
        let indices = &self.filtered_model_indices;
        if indices.is_empty() {
            return;
        }
        let current = indices
            .iter()
            .position(|index| *index == self.model_cursor)
            .unwrap_or(0);
        let next = move_index(current, indices.len(), delta);
        self.model_cursor = indices[next];
    }

    fn refresh_model_filter(&mut self) {
        self.filtered_model_indices = fuzzy_indices(
            &self.models,
            self.model_search.trim(),
            FuzzyMode::Text,
            None,
            |model| format!("{} {}", model.id, model.display_name),
        );
        self.sync_model_cursor_to_filter();
    }

    fn sync_model_cursor_to_filter(&mut self) {
        let indices = &self.filtered_model_indices;
        if let Some(index) = indices.first().copied() {
            if !indices.contains(&self.model_cursor) {
                self.model_cursor = index;
            }
        }
    }

    fn apply_model(&mut self) -> SettingsAction {
        if self.model_search_active {
            self.model_search_active = false;
        }
        let Some(model) = self.models.get(self.model_cursor) else {
            return SettingsAction::None;
        };
        if self.selected_model == model.id {
            return SettingsAction::None;
        }
        self.selected_model = model.id.clone();
        self.clamp_thinking_to_selected_model();
        SettingsAction::SetModel(self.selected_model.clone())
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
        assert!(settings.model_search_active);
        assert_eq!(
            settings.handle_key(key(KeyCode::Char('g'))),
            SettingsAction::None
        );
        assert_eq!(settings.model_search, "g");
        assert_eq!(settings.filtered_model_indices().len(), 2);
        assert!(settings.filtered_model_indices().contains(&1));
        assert!(settings.filtered_model_indices().contains(&2));
        assert!(matches!(settings.model_cursor, 1 | 2));
        assert_eq!(settings.handle_key(key(KeyCode::Esc)), SettingsAction::None);
        assert!(!settings.model_search_active);
        assert_eq!(settings.model_search, "");
        assert_eq!(settings.model_cursor, 1);
    }
}
