#![forbid(unsafe_code)]

use crate::{
    action::TuiAction,
    command::{
        command_suggestions_for, file_suggestions_for, parse_command, CommandSuggestionCategory,
        CommandSuggestionItem, CommandSuggestionsState, CommandSuggestionsView, ParsedCommand,
        SettingsCommand,
    },
    composer::{
        char_count, collapsed_paste_summary, normalize_paste_text, should_collapse_paste,
        ComposerState, MAX_PASTE_CHARS,
    },
    fuzzy::{fuzzy_indices, FuzzyMode},
    help::{help_entry_match_text, HELP_ENTRIES},
    message::{project_content_blocks, project_message, project_messages, MessageView},
    resource::{PromptResource, ResourceBrowserState, SkillResource},
    settings::{
        chat_style_label, collapse_mode_label, ModelOption, SettingsAction, SettingsState,
        ToolSettingsItem,
    },
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use oino_types::{ContentBlock, Message, OinoId, ThinkingLevel};

pub const HELP_STATUS: &str = "Type /help for shortcuts and commands";

const DEFAULT_TRANSCRIPT_PAGE_LINES: usize = 10;
const TRANSCRIPT_SCROLL_LINE_STEP: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Help,
    Settings,
    SendPanel,
    Sessions,
    Prompts,
    Skills,
    Inspect,
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

#[derive(Debug, Clone, PartialEq, Eq)]
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
    pub command_suggestions: CommandSuggestionsState,
    pub chord: ChordState,
    pub transcript_scroll: TranscriptScroll,
    pub send_panel: SendPanelState,
    pub sessions: SessionsState,
    pub inspect: InspectState,
    pub prompts: ResourceBrowserState,
    pub skills: ResourceBrowserState,
    pub prompt_resources: Vec<PromptResource>,
    pub skill_resources: Vec<SkillResource>,
    pub resource_diagnostics: Vec<String>,
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
    file_paths: Vec<String>,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
            session_title: String::new(),
            composer: ComposerState::new(),
            focus: TuiFocus::Composer,
            status: HELP_STATUS.into(),
            working: false,
            error: None,
            overlay: None,
            settings: SettingsState::new("", ThinkingLevel::Off),
            command_suggestions: CommandSuggestionsState::new(),
            chord: ChordState::None,
            transcript_scroll: TranscriptScroll::default(),
            send_panel: SendPanelState::default(),
            sessions: SessionsState::default(),
            inspect: InspectState::default(),
            prompts: ResourceBrowserState::default(),
            skills: ResourceBrowserState::default(),
            prompt_resources: Vec::new(),
            skill_resources: Vec::new(),
            resource_diagnostics: Vec::new(),
            help_scroll: 0,
            help_search: String::new(),
            help_search_active: false,
            filtered_help_indices: (0..HELP_ENTRIES.len()).collect(),
            steer_items: Vec::new(),
            queued_items: Vec::new(),
            draft_items: Vec::new(),
            transcript_page_lines: DEFAULT_TRANSCRIPT_PAGE_LINES,
            transcript_version: 0,
            quit_pending: false,
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
        Self {
            settings: SettingsState::new(model, thinking_level),
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
        )
        .or_else(|| file_suggestions_for(input, cursor, &self.file_paths))
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

    pub fn set_tool_settings(&mut self, tools: Vec<ToolSettingsItem>) {
        self.settings.set_tools(tools);
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
            if self.quit_pending {
                return TuiAction::Quit;
            }
            self.quit_pending = true;
            self.status = "Press Ctrl-C again to quit • Esc stops a running response".into();
            return TuiAction::None;
        }
        self.quit_pending = false;

        if self.chord != ChordState::None {
            return self.handle_chord_key(key);
        }

        if is_ctrl_o_key(key) {
            self.chord = ChordState::CtrlO;
            self.status =
                "Ctrl-O: s settings • q send panel • t transcript • e expand • Esc cancel".into();
            return TuiAction::None;
        }

        match self.overlay {
            Some(OverlayKind::Help) => return self.handle_help_key(key),
            Some(OverlayKind::Settings) => return self.handle_settings_key(key),
            Some(OverlayKind::SendPanel) => return self.handle_send_panel_key(key),
            Some(OverlayKind::Sessions) => return self.handle_sessions_key(key),
            Some(OverlayKind::Prompts) => return self.handle_prompts_key(key),
            Some(OverlayKind::Skills) => return self.handle_skills_key(key),
            Some(OverlayKind::Inspect) => return self.handle_inspect_key(key),
            None => {}
        }

        if self.command_suggestions_view().is_some() {
            let handled = self.handle_command_suggestion_key(key);
            if handled != CommandSuggestionKeyResult::Unhandled {
                return handled.into_action();
            }
        }

        if self.handle_transcript_scroll_key(key) {
            return TuiAction::None;
        }

        if matches!(key.code, KeyCode::Esc) {
            if self.working {
                self.status = "Stopping response…".into();
                return TuiAction::AbortPrompt;
            }
            self.status = "Esc ignored • press Ctrl-C twice to quit".into();
            return TuiAction::None;
        }

        match key.code {
            KeyCode::Enter if key.modifiers.is_empty() => self.submit_input(),
            _ => {
                if self.focus == TuiFocus::Composer {
                    let before = self.composer.text().to_string();
                    self.composer.handle_edit_key(key);
                    self.after_composer_edit(&before);
                }
                TuiAction::None
            }
        }
    }

    fn handle_chord_key(&mut self, key: KeyEvent) -> TuiAction {
        let chord = self.chord;
        self.chord = ChordState::None;
        match (chord, key.code) {
            (ChordState::CtrlO, KeyCode::Char('s' | 'S')) if key.modifiers.is_empty() => {
                self.open_settings_overlay();
                TuiAction::None
            }
            (ChordState::CtrlO, KeyCode::Char('q' | 'Q')) if key.modifiers.is_empty() => {
                self.open_send_panel();
                TuiAction::None
            }
            (ChordState::CtrlO, KeyCode::Char('t' | 'T')) if key.modifiers.is_empty() => {
                self.focus = TuiFocus::Transcript;
                self.status = "Transcript focus • ↑/↓ or j/k line • PgUp/PgDn page • Home/End top/bottom • Esc composer".into();
                TuiAction::None
            }
            (ChordState::CtrlO, KeyCode::Char('e' | 'E')) if key.modifiers.is_empty() => {
                if self.composer.expand_collapsed_paste_at_cursor() {
                    self.status = "Expanded pasted block".into();
                } else {
                    match self.expand_prompt_references_in_composer() {
                        PromptReferenceExpansionResult::Expanded(count) => {
                            let plural = if count == 1 { "" } else { "s" };
                            self.status = format!("Expanded {count} prompt template{plural}");
                        }
                        PromptReferenceExpansionResult::NoPromptReference => {
                            self.status =
                                "No collapsed paste block or prompt reference to expand".into();
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
                TuiAction::None
            }
            (_, KeyCode::Esc) => {
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            _ => {
                self.status = "Unknown Ctrl-O chord".into();
                TuiAction::None
            }
        }
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

    fn handle_transcript_scroll_key(&mut self, key: KeyEvent) -> bool {
        let page_lines = self.transcript_page_lines.saturating_sub(1).max(1);
        let no_mods = key.modifiers.is_empty();
        let alt = key.modifiers == KeyModifiers::ALT;
        let ctrl = key.modifiers == KeyModifiers::CONTROL;
        let composer_empty = self.focus == TuiFocus::Composer && self.composer.is_empty();
        let transcript_focus = self.focus == TuiFocus::Transcript;

        match key.code {
            KeyCode::Esc if transcript_focus => {
                self.focus = TuiFocus::Composer;
                self.status = self.transcript_scroll_status();
                true
            }
            KeyCode::PageUp if no_mods => {
                self.scroll_transcript_up(page_lines);
                true
            }
            KeyCode::PageDown if no_mods => {
                self.scroll_transcript_down(page_lines);
                true
            }
            KeyCode::Up if alt || (no_mods && (transcript_focus || composer_empty)) => {
                self.scroll_transcript_up(TRANSCRIPT_SCROLL_LINE_STEP);
                true
            }
            KeyCode::Down if alt || (no_mods && (transcript_focus || composer_empty)) => {
                self.scroll_transcript_down(TRANSCRIPT_SCROLL_LINE_STEP);
                true
            }
            KeyCode::Char('k' | 'K') if no_mods && transcript_focus => {
                self.scroll_transcript_up(TRANSCRIPT_SCROLL_LINE_STEP);
                true
            }
            KeyCode::Char('j' | 'J') if no_mods && transcript_focus => {
                self.scroll_transcript_down(TRANSCRIPT_SCROLL_LINE_STEP);
                true
            }
            KeyCode::Home if ctrl || (no_mods && (transcript_focus || composer_empty)) => {
                self.scroll_transcript_to_top();
                true
            }
            KeyCode::Char('g') if no_mods && transcript_focus => {
                self.scroll_transcript_to_top();
                true
            }
            KeyCode::End if ctrl || (no_mods && (transcript_focus || composer_empty)) => {
                self.scroll_transcript_to_bottom();
                true
            }
            KeyCode::Char('G') if no_mods && transcript_focus => {
                self.scroll_transcript_to_bottom();
                true
            }
            _ => false,
        }
    }

    fn handle_command_suggestion_key(&mut self, key: KeyEvent) -> CommandSuggestionKeyResult {
        match key.code {
            KeyCode::Esc => {
                self.command_suggestions.dismiss_for(self.composer.text());
                CommandSuggestionKeyResult::Handled(TuiAction::None)
            }
            KeyCode::Up => {
                let len = self
                    .command_suggestions_view()
                    .map_or(0, |view| view.items.len());
                self.command_suggestions.move_selection(-1, len);
                CommandSuggestionKeyResult::Handled(TuiAction::None)
            }
            KeyCode::Down => {
                let len = self
                    .command_suggestions_view()
                    .map_or(0, |view| view.items.len());
                self.command_suggestions.move_selection(1, len);
                CommandSuggestionKeyResult::Handled(TuiAction::None)
            }
            KeyCode::Tab => {
                self.accept_command_suggestion(false);
                CommandSuggestionKeyResult::Handled(TuiAction::None)
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                if self
                    .command_suggestions_view()
                    .and_then(|view| view.selected_item().cloned())
                    .is_none()
                {
                    CommandSuggestionKeyResult::Unhandled
                } else if self.accept_command_suggestion(true) {
                    CommandSuggestionKeyResult::Handled(self.submit_input())
                } else {
                    CommandSuggestionKeyResult::Handled(TuiAction::None)
                }
            }
            _ => CommandSuggestionKeyResult::Unhandled,
        }
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

    fn handle_help_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.help_search_active {
            return self.handle_help_search_key(key);
        }

        let page = self.transcript_page_lines.max(5);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'Q') if key.modifiers.is_empty() => {
                self.overlay = None;
                self.status = HELP_STATUS.into();
            }
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.help_search_active = true;
                self.help_search.clear();
                self.refresh_help_filter();
                self.status = "Help search active".into();
            }
            KeyCode::Up | KeyCode::Char('k' | 'K') if key.modifiers.is_empty() => {
                self.scroll_help_up(1);
            }
            KeyCode::Down | KeyCode::Char('j' | 'J') if key.modifiers.is_empty() => {
                self.scroll_help_down(1);
            }
            KeyCode::PageUp if key.modifiers.is_empty() => {
                self.scroll_help_up(page);
            }
            KeyCode::PageDown if key.modifiers.is_empty() => {
                self.scroll_help_down(page);
            }
            KeyCode::Home if key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL => {
                self.help_scroll = 0;
            }
            KeyCode::End if key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL => {
                self.help_scroll = self.filtered_help_indices.len().saturating_sub(1);
            }
            _ => {}
        }
        TuiAction::None
    }

    fn handle_help_search_key(&mut self, key: KeyEvent) -> TuiAction {
        let page = self.transcript_page_lines.max(5);
        match key.code {
            KeyCode::Esc => {
                self.help_search_active = false;
                self.help_search.clear();
                self.refresh_help_filter();
                self.status = "Help search cleared".into();
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.help_search_active = false;
            }
            KeyCode::Backspace => {
                self.help_search.pop();
                self.refresh_help_filter();
                self.status = help_search_status(&self.help_search);
            }
            KeyCode::Up => self.scroll_help_up(1),
            KeyCode::Down => self.scroll_help_down(1),
            KeyCode::PageUp if key.modifiers.is_empty() => self.scroll_help_up(page),
            KeyCode::PageDown if key.modifiers.is_empty() => self.scroll_help_down(page),
            KeyCode::Home if key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL => {
                self.help_scroll = 0;
            }
            KeyCode::End if key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL => {
                self.help_scroll = self.filtered_help_indices.len().saturating_sub(1);
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.help_search.push(ch);
                self.refresh_help_filter();
                self.status = help_search_status(&self.help_search);
            }
            _ => {}
        }
        TuiAction::None
    }

    fn refresh_help_filter(&mut self) {
        self.filtered_help_indices = fuzzy_indices(
            HELP_ENTRIES,
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

    fn handle_inspect_key(&mut self, key: KeyEvent) -> TuiAction {
        let page = self.transcript_page_lines.max(5);
        match key.code {
            KeyCode::Esc | KeyCode::Char('q' | 'Q') if key.modifiers.is_empty() => {
                self.overlay = None;
                self.inspect.loading = false;
                self.status = HELP_STATUS.into();
            }
            KeyCode::Up | KeyCode::Char('k' | 'K') if key.modifiers.is_empty() => {
                self.inspect.scroll = self.inspect.scroll.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j' | 'J') if key.modifiers.is_empty() => {
                self.inspect.scroll = self.inspect.scroll.saturating_add(1);
            }
            KeyCode::PageUp if key.modifiers.is_empty() => {
                self.inspect.scroll = self.inspect.scroll.saturating_sub(page);
            }
            KeyCode::PageDown if key.modifiers.is_empty() => {
                self.inspect.scroll = self.inspect.scroll.saturating_add(page);
            }
            KeyCode::Home if key.modifiers.is_empty() || key.modifiers == KeyModifiers::CONTROL => {
                self.inspect.scroll = 0;
            }
            KeyCode::Char('e' | 'E') if key.modifiers.is_empty() => {
                self.status = "Exporting chat…".into();
                return TuiAction::ExportChatHtml;
            }
            _ => {}
        }
        TuiAction::None
    }

    fn handle_send_panel_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.send_panel.confirm_delete {
            match key.code {
                KeyCode::Char('y' | 'Y') if key.modifiers.is_empty() => {
                    let deleted = self
                        .selected_send_panel_item()
                        .and_then(|item| self.delete_send_panel_item(&item));
                    self.send_panel.confirm_delete = false;
                    self.status = deleted.map_or_else(
                        || "Nothing selected to delete".into(),
                        |text| format!("Deleted {}", summarize_panel_text(&text)),
                    );
                }
                KeyCode::Char('n' | 'N') | KeyCode::Esc if key.modifiers.is_empty() => {
                    self.send_panel.confirm_delete = false;
                    self.status = "Delete canceled".into();
                }
                _ => {
                    self.status = "Press y to confirm deletion • n/Esc cancel".into();
                }
            }
            return TuiAction::None;
        }

        match key.code {
            KeyCode::Esc => {
                self.overlay = None;
                if self.working {
                    self.set_calling_status();
                } else {
                    self.status = HELP_STATUS.into();
                }
                TuiAction::None
            }
            KeyCode::Up | KeyCode::Char('k' | 'K') if key.modifiers.is_empty() => {
                self.move_send_panel_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down | KeyCode::Char('j' | 'J') if key.modifiers.is_empty() => {
                self.move_send_panel_cursor(1);
                TuiAction::None
            }
            KeyCode::Char('q' | 'Q') if key.modifiers.is_empty() => {
                let Some(prompt) = self.take_composer_text() else {
                    self.status = "No input to queue".into();
                    return TuiAction::None;
                };
                self.enqueue_prompt(prompt.clone());
                self.status = format!("Queued {}", summarize_panel_text(&prompt));
                TuiAction::QueuePrompt(prompt)
            }
            KeyCode::Char('d' | 'D') if key.modifiers.is_empty() => {
                if self.draft_current_input() {
                    self.status = "Moved current input to Draft".into();
                } else {
                    self.status = "No input to draft".into();
                }
                TuiAction::None
            }
            KeyCode::Char('x' | 'X') if key.modifiers.is_empty() => {
                if self.selected_send_panel_item().is_some() {
                    self.send_panel.confirm_delete = true;
                    self.status = "Press y to confirm deletion • n/Esc cancel".into();
                } else {
                    self.status = "Nothing selected to delete".into();
                }
                TuiAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
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

    fn handle_sessions_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.sessions.search_active {
            return self.handle_sessions_search_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.overlay = None;
                self.sessions.loading = false;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyCode::Up | KeyCode::Char('k' | 'K') if key.modifiers.is_empty() => {
                self.move_sessions_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down | KeyCode::Char('j' | 'J') if key.modifiers.is_empty() => {
                self.move_sessions_cursor(1);
                TuiAction::None
            }
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.sessions.search_active = true;
                self.sessions.search.clear();
                self.refresh_session_filter();
                self.status = "Session search active".into();
                TuiAction::None
            }
            KeyCode::Char('r' | 'R') if key.modifiers.is_empty() => {
                self.sessions.loading = true;
                self.status = "Loading sessions…".into();
                TuiAction::ListSessions
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.open_selected_session_action(),
            _ => TuiAction::None,
        }
    }

    fn handle_sessions_search_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Esc => {
                self.sessions.search_active = false;
                self.sessions.search.clear();
                self.refresh_session_filter();
                self.status = "Session search cleared".into();
                TuiAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => self.open_selected_session_action(),
            KeyCode::Backspace => {
                self.sessions.search.pop();
                self.refresh_session_filter();
                self.status = session_search_status(&self.sessions.search);
                TuiAction::None
            }
            KeyCode::Up => {
                self.move_sessions_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down => {
                self.move_sessions_cursor(1);
                TuiAction::None
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.sessions.search.push(ch);
                self.refresh_session_filter();
                self.status = session_search_status(&self.sessions.search);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_prompts_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.prompts.search_active {
            return self.handle_prompts_search_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.overlay = None;
                self.prompts.loading = false;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyCode::Up | KeyCode::Char('k' | 'K') if key.modifiers.is_empty() => {
                self.move_prompt_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down | KeyCode::Char('j' | 'J') if key.modifiers.is_empty() => {
                self.move_prompt_cursor(1);
                TuiAction::None
            }
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.prompts.search_active = true;
                self.prompts.search.clear();
                self.refresh_prompt_filter();
                self.status = "Prompt search active".into();
                TuiAction::None
            }
            KeyCode::Char('r' | 'R') if key.modifiers.is_empty() => {
                self.prompts.loading = true;
                self.status = "Reloading resources…".into();
                TuiAction::ReloadResources
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.complete_selected_prompt_command();
                TuiAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.complete_selected_prompt_command();
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_prompts_search_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Esc => {
                self.prompts.search_active = false;
                self.prompts.search.clear();
                self.refresh_prompt_filter();
                self.status = "Prompt search cleared".into();
                TuiAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.complete_selected_prompt_command();
                TuiAction::None
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.complete_selected_prompt_command();
                TuiAction::None
            }
            KeyCode::Backspace => {
                self.prompts.search.pop();
                self.refresh_prompt_filter();
                self.status = prompt_search_status(&self.prompts.search);
                TuiAction::None
            }
            KeyCode::Up => {
                self.move_prompt_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down => {
                self.move_prompt_cursor(1);
                TuiAction::None
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.prompts.search.push(ch);
                self.refresh_prompt_filter();
                self.status = prompt_search_status(&self.prompts.search);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_skills_key(&mut self, key: KeyEvent) -> TuiAction {
        if self.skills.search_active {
            return self.handle_skills_search_key(key);
        }

        match key.code {
            KeyCode::Esc => {
                self.overlay = None;
                self.skills.loading = false;
                self.status = HELP_STATUS.into();
                TuiAction::None
            }
            KeyCode::Up | KeyCode::Char('k' | 'K') if key.modifiers.is_empty() => {
                self.move_skill_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down | KeyCode::Char('j' | 'J') if key.modifiers.is_empty() => {
                self.move_skill_cursor(1);
                TuiAction::None
            }
            KeyCode::Char('/') if key.modifiers.is_empty() => {
                self.skills.search_active = true;
                self.skills.search.clear();
                self.refresh_skill_filter();
                self.status = "Skill search active".into();
                TuiAction::None
            }
            KeyCode::Char('r' | 'R') if key.modifiers.is_empty() => {
                self.skills.loading = true;
                self.status = "Reloading resources…".into();
                TuiAction::ReloadResources
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.complete_selected_skill_command();
                TuiAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.complete_selected_skill_command();
                TuiAction::None
            }
            _ => TuiAction::None,
        }
    }

    fn handle_skills_search_key(&mut self, key: KeyEvent) -> TuiAction {
        match key.code {
            KeyCode::Esc => {
                self.skills.search_active = false;
                self.skills.search.clear();
                self.refresh_skill_filter();
                self.status = "Skill search cleared".into();
                TuiAction::None
            }
            KeyCode::Enter if key.modifiers.is_empty() => {
                self.complete_selected_skill_command();
                TuiAction::None
            }
            KeyCode::Tab if key.modifiers.is_empty() => {
                self.complete_selected_skill_command();
                TuiAction::None
            }
            KeyCode::Backspace => {
                self.skills.search.pop();
                self.refresh_skill_filter();
                self.status = skill_search_status(&self.skills.search);
                TuiAction::None
            }
            KeyCode::Up => {
                self.move_skill_cursor(-1);
                TuiAction::None
            }
            KeyCode::Down => {
                self.move_skill_cursor(1);
                TuiAction::None
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.skills.search.push(ch);
                self.refresh_skill_filter();
                self.status = skill_search_status(&self.skills.search);
                TuiAction::None
            }
            _ => TuiAction::None,
        }
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

    fn open_settings_overlay(&mut self) {
        self.clear_error();
        self.settings.open_menu();
        self.overlay = Some(OverlayKind::Settings);
        self.status = "Settings: arrows/jk move • Enter open • Esc close".into();
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CommandSuggestionKeyResult {
    Handled(TuiAction),
    Unhandled,
}

impl CommandSuggestionKeyResult {
    fn into_action(self) -> TuiAction {
        match self {
            Self::Handled(action) => action,
            Self::Unhandled => TuiAction::None,
        }
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

fn is_ctrl_o_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O') if key.modifiers.contains(KeyModifiers::CONTROL))
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
    fuzzy_indices(items, query, FuzzyMode::Path, None, session_match_text)
}

fn filtered_prompt_indices(items: &[PromptResource], query: &str) -> Vec<usize> {
    fuzzy_indices(items, query, FuzzyMode::Text, None, prompt_match_text)
}

fn filtered_skill_indices(items: &[SkillResource], query: &str) -> Vec<usize> {
    fuzzy_indices(items, query, FuzzyMode::Text, None, skill_match_text)
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

fn prompt_search_status(query: &str) -> String {
    if query.is_empty() {
        "Prompt search active".into()
    } else {
        format!("Searching prompts for `{query}`")
    }
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
    fn empty_composer_arrows_scroll_transcript_like_deepseek() {
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
        assert!(state
            .filtered_help_indices()
            .iter()
            .all(|index| !matches!(HELP_ENTRIES[*index], crate::help::HelpEntry::Blank)));
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
