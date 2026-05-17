#![forbid(unsafe_code)]

use crate::{
    action::TuiAction,
    command::{
        command_suggestions_for, parse_command, CommandSuggestionsState, CommandSuggestionsView,
        ParsedCommand, SettingsCommand,
    },
    composer::ComposerState,
    message::{project_content_blocks, project_message, project_messages, MessageView},
    settings::{chat_style_label, collapse_mode_label, ModelOption, SettingsAction, SettingsState},
};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use oino_types::{ContentBlock, Message, OinoId, ThinkingLevel};

pub const HELP_STATUS: &str =
    "Enter send • PgUp/PgDn scroll transcript • type / or Ctrl-O s settings • Ctrl-J/Alt-Enter newline • Esc/Ctrl-C quit";

const DEFAULT_TRANSCRIPT_PAGE_LINES: usize = 10;
const TRANSCRIPT_SCROLL_LINE_STEP: usize = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayKind {
    Settings,
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
    transcript_page_lines: usize,
}

impl Default for TuiState {
    fn default() -> Self {
        Self {
            messages: Vec::new(),
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
            transcript_page_lines: DEFAULT_TRANSCRIPT_PAGE_LINES,
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

    pub fn set_transcript_page_lines(&mut self, lines: usize) {
        self.transcript_page_lines = lines.max(1);
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
        if self.overlay.is_some() || self.focus != TuiFocus::Composer || !self.composer.is_enabled()
        {
            return None;
        }
        let input = self.composer.text();
        if self.command_suggestions.is_dismissed_for(input) {
            return None;
        }
        let mut view =
            command_suggestions_for(input, self.composer.cursor(), &self.settings.models)?;
        view.selected = if view.items.is_empty() {
            0
        } else {
            self.command_suggestions
                .selected
                .min(view.items.len().saturating_sub(1))
        };
        Some(view)
    }

    pub fn set_messages_from_oino(&mut self, messages: &[Message]) {
        self.messages = project_messages(messages);
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
    }

    pub fn update_message(&mut self, id: OinoId, content: &[ContentBlock]) {
        let projected = project_content_blocks(content);
        if let Some(message) = self.messages.iter_mut().find(|message| message.id == id) {
            message.content = projected.content;
            message.thinking = projected.thinking;
            message.thinking_redacted = projected.thinking_redacted;
            message.tool_calls = projected.tool_calls;
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
        }
    }

    pub fn finish_message(&mut self, message: &Message) {
        let mut view = project_message(message);
        let fallback_title = self.title_for_role(&view.role);
        if let Some(existing) = self
            .messages
            .iter_mut()
            .find(|existing| existing.id == view.id)
        {
            if view.title.is_none() {
                view.title = existing.title.clone().or(fallback_title);
            }
            *existing = view;
        } else {
            if view.title.is_none() {
                view.title = fallback_title;
            }
            self.messages.push(view);
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
        self.composer.set_enabled(!working);
        self.status = if working {
            "Working… input paused".into()
        } else {
            HELP_STATUS.into()
        };
    }

    pub fn set_model_catalog(&mut self, models: Vec<ModelOption>, status: impl Into<String>) {
        self.settings.set_models(models, status);
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

    pub fn handle_key(&mut self, key: KeyEvent) -> TuiAction {
        if is_force_quit_key(key) {
            return TuiAction::Quit;
        }

        if self.chord != ChordState::None {
            return self.handle_chord_key(key);
        }

        if is_ctrl_o_key(key) {
            self.chord = ChordState::CtrlO;
            self.status = "Ctrl-O chord: s settings • t transcript • Esc cancel".into();
            return TuiAction::None;
        }

        if matches!(self.overlay, Some(OverlayKind::Settings)) {
            return self.handle_settings_key(key);
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
            return TuiAction::Quit;
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
            (ChordState::CtrlO, KeyCode::Char('t' | 'T')) if key.modifiers.is_empty() => {
                self.focus = TuiFocus::Transcript;
                self.status = "Transcript focus • ↑/↓ or j/k line • PgUp/PgDn page • Home/End top/bottom • Esc composer".into();
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
        let replacement = if should_submit {
            item.replacement.clone()
        } else {
            format!("{} ", item.replacement.trim_end())
        };
        self.composer
            .replace_char_range(item.replace_start, item.replace_end, &replacement);
        if should_submit {
            self.command_suggestions.dismiss_for(self.composer.text());
        }
        should_submit
    }

    fn after_composer_edit(&mut self, before: &str) {
        let input = self.composer.text();
        if before != input {
            self.command_suggestions
                .clear_dismissal_if_input_changed(input);
        }
        if let Some(view) =
            command_suggestions_for(input, self.composer.cursor(), &self.settings.models)
        {
            self.command_suggestions.clamp(view.items.len());
        }
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
        }
    }

    fn submit_input(&mut self) -> TuiAction {
        match self.composer.submit() {
            Some(prompt) => self.submit_text(prompt),
            None => TuiAction::None,
        }
    }

    fn submit_text(&mut self, prompt: String) -> TuiAction {
        if let Some(command) = parse_command(&prompt) {
            return self.execute_command(command);
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

    fn execute_command(&mut self, command: ParsedCommand) -> TuiAction {
        match command {
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

fn is_force_quit_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('c') | KeyCode::Char('C') if key.modifiers.contains(KeyModifiers::CONTROL))
}

fn is_ctrl_o_key(key: KeyEvent) -> bool {
    matches!(key.code, KeyCode::Char('o') | KeyCode::Char('O') if key.modifiers.contains(KeyModifiers::CONTROL))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
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
    fn working_state_pauses_input() {
        let mut state = TuiState::new();
        state.set_working(true);
        assert_eq!(state.handle_key(key(KeyCode::Char('x'))), TuiAction::None);
        assert_eq!(state.input(), "");
        assert_eq!(state.handle_key(key(KeyCode::Enter)), TuiAction::None);
    }

    #[test]
    fn ctrl_o_s_opens_settings_overlay() {
        let mut state = TuiState::new();
        assert_eq!(
            state.handle_key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL)),
            TuiAction::None
        );
        assert_eq!(state.chord, ChordState::CtrlO);
        assert_eq!(state.handle_key(key(KeyCode::Char('s'))), TuiAction::None);
        assert_eq!(state.overlay, Some(OverlayKind::Settings));
        assert_eq!(state.chord, ChordState::None);
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
    fn tab_completes_command_suggestion_without_submitting() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert_eq!(state.handle_key(key(KeyCode::Tab)), TuiAction::None);
        assert_eq!(state.input(), "/settings ");
        assert_eq!(state.overlay, None);
    }

    #[test]
    fn escape_dismisses_command_suggestions_before_quitting() {
        let mut state = TuiState::new();
        assert_eq!(state.handle_key(key(KeyCode::Char('/'))), TuiAction::None);
        assert!(state.command_suggestions_view().is_some());
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::None);
        assert!(state.command_suggestions_view().is_none());
        assert_eq!(state.handle_key(key(KeyCode::Esc)), TuiAction::Quit);
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
