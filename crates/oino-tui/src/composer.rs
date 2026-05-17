#![forbid(unsafe_code)]

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

pub const INPUT_PLACEHOLDER: &str = "Ask Oino • /help • @ file paths";
pub(crate) const COLLAPSED_PASTE_MIN_LINES: usize = 8;
pub(crate) const COLLAPSED_PASTE_MIN_CHARS: usize = 1200;
pub(crate) const MAX_PASTE_CHARS: usize = 200_000;

#[derive(Debug, Clone, PartialEq, Eq)]
struct PasteBlock {
    token: String,
    text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComposerState {
    text: String,
    cursor: usize,
    enabled: bool,
    paste_blocks: Vec<PasteBlock>,
    next_paste_block_id: u64,
}

impl Default for ComposerState {
    fn default() -> Self {
        Self {
            text: String::new(),
            cursor: 0,
            enabled: true,
            paste_blocks: Vec::new(),
            next_paste_block_id: 1,
        }
    }
}

impl ComposerState {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    #[must_use]
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    #[must_use]
    pub fn is_enabled(&self) -> bool {
        self.enabled
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        self.enabled = enabled;
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.text.trim().is_empty()
    }

    pub fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.paste_blocks.clear();
    }

    pub fn replace_text(&mut self, text: impl Into<String>) {
        self.text = text.into();
        self.cursor = char_count(&self.text);
        self.paste_blocks.clear();
    }

    pub fn insert_text(&mut self, text: &str) -> bool {
        if !self.enabled {
            return false;
        }
        if text.is_empty() {
            return true;
        }
        let normalized = normalize_paste_text(text);
        let cursor = self.cursor.min(char_count(&self.text));
        let byte_index = byte_index_at_char(&self.text, cursor);
        self.text.insert_str(byte_index, &normalized);
        self.cursor = cursor + char_count(&normalized);
        true
    }

    pub fn replace_char_range(&mut self, start: usize, end: usize, replacement: &str) {
        let len = char_count(&self.text);
        let start = start.min(len);
        let end = end.min(len).max(start);
        let start_byte = byte_index_at_char(&self.text, start);
        let end_byte = byte_index_at_char(&self.text, end);
        self.text.replace_range(start_byte..end_byte, replacement);
        self.cursor = start + char_count(replacement);
        self.retain_existing_paste_blocks();
    }

    pub fn insert_collapsed_paste(&mut self, text: &str) -> Option<String> {
        if !self.enabled || text.is_empty() {
            return None;
        }
        let normalized = normalize_paste_text(text);
        let id = self.next_paste_block_id;
        self.next_paste_block_id = self.next_paste_block_id.saturating_add(1);
        let token = collapsed_paste_token(id, &normalized);
        if !self.insert_text(&token) {
            return None;
        }
        self.paste_blocks.push(PasteBlock {
            token: token.clone(),
            text: normalized,
        });
        Some(token)
    }

    pub fn expand_collapsed_paste_at_cursor(&mut self) -> bool {
        let Some(index) = self.paste_block_index_at_cursor() else {
            return false;
        };
        let block = self.paste_blocks.remove(index);
        let Some((start, end)) = char_range_of(&self.text, &block.token) else {
            return false;
        };
        self.replace_char_range(start, end, &block.text);
        true
    }

    #[must_use]
    pub fn expanded_text(&self) -> String {
        let mut expanded = self.text.clone();
        for block in &self.paste_blocks {
            expanded = expanded.replace(&block.token, &block.text);
        }
        expanded
    }

    #[must_use]
    pub fn has_collapsed_paste_blocks(&self) -> bool {
        !self.paste_blocks.is_empty()
    }

    #[must_use]
    pub fn submit(&mut self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let prompt = self.expanded_text().trim().to_string();
        if prompt.is_empty() {
            return None;
        }
        self.clear();
        Some(prompt)
    }

    pub fn handle_edit_key(&mut self, key: KeyEvent) -> bool {
        if !self.enabled {
            return false;
        }

        match key.code {
            _ if is_newline_key(key) => {
                self.insert_char('\n');
                true
            }
            KeyCode::Backspace => {
                self.delete_char();
                true
            }
            KeyCode::Delete => {
                self.delete_char_forward();
                true
            }
            KeyCode::Left if is_word_cursor_modifier(key.modifiers) => {
                self.move_cursor_word_backward();
                true
            }
            KeyCode::Left => {
                self.move_cursor_left();
                true
            }
            KeyCode::Right if is_word_cursor_modifier(key.modifiers) => {
                self.move_cursor_word_forward();
                true
            }
            KeyCode::Right => {
                self.move_cursor_right();
                true
            }
            KeyCode::Up => {
                self.move_cursor_up();
                true
            }
            KeyCode::Down => {
                self.move_cursor_down();
                true
            }
            KeyCode::Home | KeyCode::Char('a') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_cursor_start();
                true
            }
            KeyCode::Home => {
                self.move_cursor_start();
                true
            }
            KeyCode::End | KeyCode::Char('e') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                self.move_cursor_end();
                true
            }
            KeyCode::End => {
                self.move_cursor_end();
                true
            }
            KeyCode::Char('u') | KeyCode::Char('U')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.clear();
                true
            }
            KeyCode::Char('w') | KeyCode::Char('W')
                if key.modifiers.contains(KeyModifiers::CONTROL) =>
            {
                self.delete_word_backward();
                true
            }
            KeyCode::Char(ch)
                if !key.modifiers.contains(KeyModifiers::CONTROL) && !ch.is_control() =>
            {
                self.insert_char(ch);
                true
            }
            _ => false,
        }
    }

    fn insert_char(&mut self, ch: char) {
        let cursor = self.cursor.min(char_count(&self.text));
        let byte_index = byte_index_at_char(&self.text, cursor);
        self.text.insert(byte_index, ch);
        self.cursor = cursor + 1;
        self.retain_existing_paste_blocks();
    }

    fn delete_char(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let target = self.cursor.saturating_sub(1);
        if remove_char_at(&mut self.text, target) {
            self.cursor = target;
            self.retain_existing_paste_blocks();
        }
    }

    fn delete_char_forward(&mut self) {
        if remove_char_at(&mut self.text, self.cursor) {
            self.cursor = self.cursor.min(char_count(&self.text));
            self.retain_existing_paste_blocks();
        }
    }

    fn delete_word_backward(&mut self) {
        if self.cursor == 0 {
            return;
        }
        let cursor_byte = byte_index_at_char(&self.text, self.cursor);
        let mut word_start = cursor_byte;
        while word_start > 0 {
            let Some((prev, ch)) = self.text[..word_start].char_indices().next_back() else {
                break;
            };
            if !ch.is_whitespace() {
                break;
            }
            word_start = prev;
        }
        while word_start > 0 {
            let Some((prev, ch)) = self.text[..word_start].char_indices().next_back() else {
                break;
            };
            if ch.is_whitespace() {
                break;
            }
            word_start = prev;
        }
        if word_start < cursor_byte {
            self.text.replace_range(word_start..cursor_byte, "");
            self.cursor = char_count(&self.text[..word_start]);
            self.retain_existing_paste_blocks();
        }
    }

    fn move_cursor_left(&mut self) {
        self.cursor = self.cursor.saturating_sub(1);
    }

    fn move_cursor_right(&mut self) {
        self.cursor = (self.cursor + 1).min(char_count(&self.text));
    }

    fn move_cursor_start(&mut self) {
        self.cursor = 0;
    }

    fn move_cursor_end(&mut self) {
        self.cursor = char_count(&self.text);
    }

    fn move_cursor_word_backward(&mut self) {
        let cursor_byte = byte_index_at_char(&self.text, self.cursor);
        let mut word_start = cursor_byte;
        while word_start > 0 {
            let Some((prev, ch)) = self.text[..word_start].char_indices().next_back() else {
                break;
            };
            word_start = prev;
            if !ch.is_whitespace() {
                break;
            }
        }
        while word_start > 0 {
            let Some((prev, ch)) = self.text[..word_start].char_indices().next_back() else {
                break;
            };
            if ch.is_whitespace() {
                break;
            }
            word_start = prev;
        }
        self.cursor = char_count(&self.text[..word_start]);
    }

    fn move_cursor_word_forward(&mut self) {
        let cursor_byte = byte_index_at_char(&self.text, self.cursor);
        let mut word_end = cursor_byte;
        while word_end < self.text.len() {
            let Some(ch) = self.text[word_end..].chars().next() else {
                break;
            };
            if !ch.is_whitespace() {
                break;
            }
            word_end += ch.len_utf8();
        }
        while word_end < self.text.len() {
            let Some(ch) = self.text[word_end..].chars().next() else {
                break;
            };
            if ch.is_whitespace() {
                break;
            }
            word_end += ch.len_utf8();
        }
        self.cursor = char_count(&self.text[..word_end]);
    }

    fn move_cursor_up(&mut self) {
        let cursor_byte = byte_index_at_char(&self.text, self.cursor);
        let Some(prev_newline) = self.text[..cursor_byte].rfind('\n') else {
            return;
        };
        let line_start = prev_newline + 1;
        let col = char_count(&self.text[line_start..cursor_byte]);
        let prev_line_end = prev_newline;
        let prev_line_start = self.text[..prev_line_end]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let prev_line_len = char_count(&self.text[prev_line_start..prev_line_end]);
        let target_col = col.min(prev_line_len);
        self.cursor = char_count(&self.text[..prev_line_start]) + target_col;
    }

    fn move_cursor_down(&mut self) {
        let cursor_byte = byte_index_at_char(&self.text, self.cursor);
        let Some(relative_newline) = self.text[cursor_byte..].find('\n') else {
            return;
        };
        let line_start = self.text[..cursor_byte]
            .rfind('\n')
            .map_or(0, |index| index + 1);
        let col = char_count(&self.text[line_start..cursor_byte]);
        let next_line_start = cursor_byte + relative_newline + 1;
        let next_line = &self.text[next_line_start..];
        let next_line_len_bytes = next_line.find('\n').unwrap_or(next_line.len());
        let next_line_len =
            char_count(&self.text[next_line_start..next_line_start + next_line_len_bytes]);
        let target_col = col.min(next_line_len);
        self.cursor = char_count(&self.text[..next_line_start]) + target_col;
    }

    fn paste_block_index_at_cursor(&self) -> Option<usize> {
        self.paste_blocks.iter().position(|block| {
            char_range_of(&self.text, &block.token).is_some_and(|(start, end)| {
                self.cursor >= start.saturating_sub(1) && self.cursor <= end.saturating_add(1)
            })
        })
    }

    fn retain_existing_paste_blocks(&mut self) {
        self.paste_blocks
            .retain(|block| self.text.contains(&block.token));
    }
}

#[must_use]
pub fn is_word_cursor_modifier(modifiers: KeyModifiers) -> bool {
    modifiers.contains(KeyModifiers::CONTROL) || modifiers.contains(KeyModifiers::ALT)
}

#[must_use]
pub fn is_newline_key(key: KeyEvent) -> bool {
    match key.code {
        KeyCode::Char('\n') | KeyCode::Char('\r') => true,
        KeyCode::Char('j') | KeyCode::Char('J') => key.modifiers.contains(KeyModifiers::CONTROL),
        KeyCode::Enter => {
            key.modifiers.contains(KeyModifiers::ALT)
                || (key.modifiers.contains(KeyModifiers::SHIFT)
                    && !key.modifiers.contains(KeyModifiers::CONTROL))
        }
        _ => false,
    }
}

#[must_use]
pub(crate) fn normalize_paste_text(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[must_use]
pub(crate) fn should_collapse_paste(text: &str) -> bool {
    let lines = text.lines().count();
    lines >= COLLAPSED_PASTE_MIN_LINES || char_count(text) >= COLLAPSED_PASTE_MIN_CHARS
}

#[must_use]
pub(crate) fn collapsed_paste_summary(text: &str) -> String {
    let lines = text.lines().count().max(1);
    let chars = char_count(text);
    format!("pasted {lines} lines · {}", human_char_count(chars))
}

fn collapsed_paste_token(id: u64, text: &str) -> String {
    format!(
        "⟦{} · #{id} · Ctrl-O e expand⟧",
        collapsed_paste_summary(text)
    )
}

fn human_char_count(chars: usize) -> String {
    if chars >= 1000 {
        format!("{:.1}k chars", chars as f64 / 1000.0)
    } else {
        format!("{chars} chars")
    }
}

fn char_range_of(text: &str, needle: &str) -> Option<(usize, usize)> {
    let byte_start = text.find(needle)?;
    let start = char_count(&text[..byte_start]);
    Some((start, start + char_count(needle)))
}

#[must_use]
pub(crate) fn char_count(text: &str) -> usize {
    text.chars().count()
}

#[must_use]
pub(crate) fn byte_index_at_char(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(index, _)| index)
}

fn remove_char_at(text: &mut String, char_index: usize) -> bool {
    let start = byte_index_at_char(text, char_index);
    if start >= text.len() {
        return false;
    }
    let end = text[start..]
        .chars()
        .next()
        .map_or(start, |ch| start + ch.len_utf8());
    text.replace_range(start..end, "");
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(code: KeyCode) -> KeyEvent {
        KeyEvent::new(code, KeyModifiers::NONE)
    }

    #[test]
    fn edits_multiline_text_and_submits() {
        let mut composer = ComposerState::new();
        assert!(composer.handle_edit_key(key(KeyCode::Char('h'))));
        assert!(composer.handle_edit_key(key(KeyCode::Char('i'))));
        assert!(composer.handle_edit_key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT)));
        assert!(composer.handle_edit_key(key(KeyCode::Char('t'))));
        assert_eq!(composer.text(), "hi\nt");
        assert_eq!(composer.submit(), Some("hi\nt".into()));
        assert_eq!(composer.text(), "");
        assert_eq!(composer.cursor(), 0);
    }

    #[test]
    fn replace_char_range_updates_text_and_cursor() {
        let mut composer = ComposerState::new();
        composer.replace_text("/settings mo");
        composer.replace_char_range(10, 12, "model ");
        assert_eq!(composer.text(), "/settings model ");
        assert_eq!(composer.cursor(), 16);
    }

    #[test]
    fn insert_text_preserves_pasted_newlines_without_submitting() {
        let mut composer = ComposerState::new();
        assert!(composer.handle_edit_key(key(KeyCode::Char('a'))));
        assert!(composer.insert_text("b\r\nc\rd"));
        assert_eq!(composer.text(), "ab\nc\nd");
        assert_eq!(composer.cursor(), 6);
    }

    #[test]
    fn disabled_composer_ignores_edits_and_submit() {
        let mut composer = ComposerState::new();
        composer.set_enabled(false);
        assert!(!composer.handle_edit_key(key(KeyCode::Char('x'))));
        assert_eq!(composer.submit(), None);
        assert_eq!(composer.text(), "");
    }

    #[test]
    fn newline_shortcuts_are_distinct_from_submit() {
        assert!(is_newline_key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::SHIFT
        )));
        assert!(is_newline_key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::ALT
        )));
        assert!(is_newline_key(KeyEvent::new(
            KeyCode::Char('j'),
            KeyModifiers::CONTROL
        )));
        assert!(!is_newline_key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE
        )));
        assert!(!is_newline_key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::CONTROL
        )));
    }
}
