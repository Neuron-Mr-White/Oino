//! Composer buffer (pi-tui `Editor` semantics): multi-line text with a cursor, prompt history,
//! kill ring (yank / yank-pop), undo, large-paste markers and attachment tokens.
//!
//! Positions are character indices into logical lines; frontends wrap for display.

use oino_ai::ImageContent;

/// Pastes with more lines than this collapse into a marker (pi `[paste #n +m lines]`).
pub const PASTE_COLLAPSE_LINES: usize = 10;
/// Pastes with more characters than this collapse into a marker.
pub const PASTE_COLLAPSE_CHARS: usize = 1000;
const HISTORY_LIMIT: usize = 100;
const UNDO_LIMIT: usize = 200;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Snapshot {
    lines: Vec<String>,
    line: usize,
    col: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LastEdit {
    None,
    Insert,
    Kill,
    Yank,
}

/// A collapsed paste.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Paste {
    marker: String,
    text: String,
}

/// An attachment token in the composer (`insertEditorToken`, pasted images).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Attachment {
    /// Marker text in the buffer (`[image #1]`).
    pub marker: String,
    /// Token id (extension tokens) or generated id.
    pub token_id: String,
    /// Image payload.
    pub image: Option<ImageContent>,
    /// File path (sent as `@path` text).
    pub path: Option<String>,
}

/// Text and images produced on submit.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Submission {
    /// Text with paste markers expanded and file tokens as `@path`.
    pub text: String,
    /// Image attachments still referenced by the text.
    pub images: Vec<ImageContent>,
}

/// The composer.
#[derive(Clone, Debug)]
pub struct Composer {
    lines: Vec<String>,
    line: usize,
    col: usize,
    history: Vec<String>,
    history_index: Option<usize>,
    draft: Option<String>,
    kill_ring: Vec<String>,
    yank_index: usize,
    last_yank_len: usize,
    last_edit: LastEdit,
    undo: Vec<Snapshot>,
    pastes: Vec<Paste>,
    paste_counter: usize,
    attachments: Vec<Attachment>,
    attachment_counter: usize,
}

impl Default for Composer {
    fn default() -> Self {
        Self::new()
    }
}

fn char_len(s: &str) -> usize {
    s.chars().count()
}

fn byte_at(s: &str, col: usize) -> usize {
    s.char_indices().nth(col).map_or(s.len(), |(b, _)| b)
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

impl Composer {
    /// An empty composer.
    #[must_use]
    pub fn new() -> Self {
        Self {
            lines: vec![String::new()],
            line: 0,
            col: 0,
            history: Vec::new(),
            history_index: None,
            draft: None,
            kill_ring: Vec::new(),
            yank_index: 0,
            last_yank_len: 0,
            last_edit: LastEdit::None,
            undo: Vec::new(),
            pastes: Vec::new(),
            paste_counter: 0,
            attachments: Vec::new(),
            attachment_counter: 0,
        }
    }

    /// Raw buffer text (markers unexpanded).
    #[must_use]
    pub fn text(&self) -> String {
        self.lines.join("\n")
    }

    /// Logical lines.
    #[must_use]
    pub fn lines(&self) -> &[String] {
        &self.lines
    }

    /// Cursor `(line, column)` in characters.
    #[must_use]
    pub const fn cursor(&self) -> (usize, usize) {
        (self.line, self.col)
    }

    /// Whether the buffer is empty.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.lines.len() == 1 && self.lines[0].is_empty()
    }

    /// Whether the cursor is on the first line (history browsing at the top).
    #[must_use]
    pub const fn on_first_line(&self) -> bool {
        self.line == 0
    }

    /// Whether the cursor is on the last line.
    #[must_use]
    pub fn on_last_line(&self) -> bool {
        self.line + 1 == self.lines.len()
    }

    fn snapshot(&mut self) {
        let snap = Snapshot {
            lines: self.lines.clone(),
            line: self.line,
            col: self.col,
        };
        if self.undo.last() != Some(&snap) {
            self.undo.push(snap);
            if self.undo.len() > UNDO_LIMIT {
                self.undo.remove(0);
            }
        }
    }

    fn edit(&mut self, kind: LastEdit) {
        if kind != LastEdit::Insert || self.last_edit != LastEdit::Insert {
            self.snapshot();
        }
        self.last_edit = kind;
        self.history_index = None;
    }

    /// Replaces the whole text; cursor at the end.
    pub fn set_text(&mut self, text: &str) {
        self.snapshot();
        self.lines = text.split('\n').map(str::to_owned).collect();
        if self.lines.is_empty() {
            self.lines.push(String::new());
        }
        self.line = self.lines.len() - 1;
        self.col = char_len(&self.lines[self.line]);
        self.last_edit = LastEdit::None;
    }

    /// Clears text, pastes and attachments (keeps history).
    pub fn clear(&mut self) {
        self.set_text("");
        self.pastes.clear();
        self.attachments.clear();
        self.history_index = None;
        self.draft = None;
    }

    /// Inserts one character.
    pub fn insert_char(&mut self, c: char) {
        if c == '\n' {
            self.newline();
            return;
        }
        self.edit(LastEdit::Insert);
        let line = &mut self.lines[self.line];
        let at = byte_at(line, self.col);
        line.insert(at, c);
        self.col += 1;
    }

    /// Inserts text (newlines split lines).
    pub fn insert_str(&mut self, text: &str) {
        if text.is_empty() {
            return;
        }
        self.edit(LastEdit::None);
        self.raw_insert(text);
    }

    fn raw_insert(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let current = &self.lines[self.line];
        let at = byte_at(current, self.col);
        let tail = current[at..].to_owned();
        let head = current[..at].to_owned();
        let mut parts: Vec<&str> = text.split('\n').collect();
        let last = parts.pop().unwrap_or_default();
        if parts.is_empty() {
            self.lines[self.line] = format!("{head}{last}{tail}");
            self.col += char_len(last);
        } else {
            self.lines[self.line] = format!("{head}{}", parts[0]);
            let mut index = self.line;
            for part in &parts[1..] {
                index += 1;
                self.lines.insert(index, (*part).to_owned());
            }
            index += 1;
            self.lines.insert(index, format!("{last}{tail}"));
            self.line = index;
            self.col = char_len(last);
        }
    }

    /// Inserts a line break at the cursor.
    pub fn newline(&mut self) {
        self.edit(LastEdit::None);
        self.raw_insert("\n");
    }

    /// Bracketed paste: large pastes collapse into a marker expanded on submit.
    pub fn paste(&mut self, text: &str) {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let line_count = text.split('\n').count();
        if line_count > PASTE_COLLAPSE_LINES || char_len(&text) > PASTE_COLLAPSE_CHARS {
            self.paste_counter += 1;
            let marker = if line_count > PASTE_COLLAPSE_LINES {
                format!("[paste #{} +{line_count} lines]", self.paste_counter)
            } else {
                format!("[paste #{} {} chars]", self.paste_counter, char_len(&text))
            };
            self.pastes.push(Paste {
                marker: marker.clone(),
                text,
            });
            self.insert_str(&marker);
        } else {
            self.insert_str(&text);
        }
    }

    /// Adds an attachment token at the cursor.
    pub fn insert_attachment(
        &mut self,
        label: &str,
        token_id: Option<String>,
        image: Option<ImageContent>,
        path: Option<String>,
    ) {
        self.attachment_counter += 1;
        let marker = if label.is_empty() {
            format!("[image #{}]", self.attachment_counter)
        } else {
            format!("[{label}]")
        };
        self.attachments.push(Attachment {
            marker: marker.clone(),
            token_id: token_id.unwrap_or_else(|| format!("att-{}", self.attachment_counter)),
            image,
            path,
        });
        self.insert_str(&marker);
        self.insert_char(' ');
    }

    /// Attachments still present in the text.
    #[must_use]
    pub fn attachments(&self) -> Vec<&Attachment> {
        let text = self.text();
        self.attachments
            .iter()
            .filter(|a| text.contains(&a.marker))
            .collect()
    }

    /// The attachment whose marker ends right before the cursor (for `tokenBackspace`).
    #[must_use]
    pub fn attachment_before_cursor(&self) -> Option<&Attachment> {
        let line = &self.lines[self.line];
        let before = &line[..byte_at(line, self.col)];
        self.attachments
            .iter()
            .find(|a| before.trim_end().ends_with(&a.marker))
    }

    /// Text and images to send (pi submit).
    #[must_use]
    pub fn submission(&self) -> Submission {
        let mut text = self.text();
        for paste in &self.pastes {
            text = text.replace(&paste.marker, &paste.text);
        }
        let mut images = Vec::new();
        for attachment in &self.attachments {
            if !text.contains(&attachment.marker) {
                continue;
            }
            if let Some(image) = &attachment.image {
                images.push(image.clone());
                text = text.replace(&attachment.marker, "");
            } else if let Some(path) = &attachment.path {
                text = text.replace(&attachment.marker, &format!("@{path}"));
            }
        }
        Submission {
            text: text.trim().to_owned(),
            images,
        }
    }

    /// Deletes the character before the cursor (joins lines at column 0).
    pub fn backspace(&mut self) {
        if self.col == 0 && self.line == 0 {
            return;
        }
        self.edit(LastEdit::None);
        if self.col == 0 {
            let current = self.lines.remove(self.line);
            self.line -= 1;
            self.col = char_len(&self.lines[self.line]);
            self.lines[self.line].push_str(&current);
        } else {
            let line = &mut self.lines[self.line];
            let at = byte_at(line, self.col - 1);
            line.remove(at);
            self.col -= 1;
        }
    }

    /// Deletes the character under the cursor (joins with the next line at end).
    pub fn delete(&mut self) {
        let len = char_len(&self.lines[self.line]);
        if self.col == len && self.on_last_line() {
            return;
        }
        self.edit(LastEdit::None);
        if self.col == len {
            let next = self.lines.remove(self.line + 1);
            self.lines[self.line].push_str(&next);
        } else {
            let line = &mut self.lines[self.line];
            let at = byte_at(line, self.col);
            line.remove(at);
        }
    }

    fn kill_range(&mut self, from: (usize, usize), to: (usize, usize), prepend: bool) {
        let continuing = self.last_edit == LastEdit::Kill;
        if !continuing {
            self.snapshot();
        }
        self.history_index = None;
        let removed = self.take_range(from, to);
        match self.kill_ring.last_mut() {
            Some(last) if continuing => {
                if prepend {
                    last.insert_str(0, &removed);
                } else {
                    last.push_str(&removed);
                }
            }
            _ if !removed.is_empty() => {
                self.kill_ring.push(removed);
                if self.kill_ring.len() > 30 {
                    self.kill_ring.remove(0);
                }
            }
            _ => {}
        }
        self.last_edit = LastEdit::Kill;
    }

    fn word_left_col(&self) -> (usize, usize) {
        if self.col == 0 {
            return if self.line == 0 {
                (0, 0)
            } else {
                (self.line - 1, char_len(&self.lines[self.line - 1]))
            };
        }
        let chars: Vec<char> = self.lines[self.line].chars().collect();
        let mut i = self.col;
        while i > 0 && !is_word(chars[i - 1]) {
            i -= 1;
        }
        while i > 0 && is_word(chars[i - 1]) {
            i -= 1;
        }
        (self.line, i)
    }

    fn word_right_col(&self) -> (usize, usize) {
        let chars: Vec<char> = self.lines[self.line].chars().collect();
        if self.col >= chars.len() {
            return if self.on_last_line() {
                (self.line, self.col)
            } else {
                (self.line + 1, 0)
            };
        }
        let mut i = self.col;
        while i < chars.len() && !is_word(chars[i]) {
            i += 1;
        }
        while i < chars.len() && is_word(chars[i]) {
            i += 1;
        }
        (self.line, i)
    }

    fn take_range(&mut self, from: (usize, usize), to: (usize, usize)) -> String {
        let (start, end) = if from <= to { (from, to) } else { (to, from) };
        let mut text = self.lines.join("\n");
        let offset = |lines: &[String], (l, c): (usize, usize)| -> usize {
            lines[..l].iter().map(|s| s.len() + 1).sum::<usize>() + byte_at(&lines[l], c)
        };
        let a = offset(&self.lines, start);
        let b = offset(&self.lines, end);
        let removed: String = text.drain(a..b).collect();
        self.lines = text.split('\n').map(str::to_owned).collect();
        self.line = start.0;
        self.col = start.1;
        removed
    }

    /// `tui.editor.deleteWordBackward` (killed text goes to the kill ring).
    pub fn delete_word_backward(&mut self) {
        let target = self.word_left_col();
        self.kill_range(target, (self.line, self.col), true);
    }

    /// `tui.editor.deleteWordForward`.
    pub fn delete_word_forward(&mut self) {
        let target = self.word_right_col();
        self.kill_range((self.line, self.col), target, false);
    }

    /// `tui.editor.deleteToLineStart`.
    pub fn delete_to_line_start(&mut self) {
        let target = if self.col == 0 && self.line > 0 {
            (self.line - 1, char_len(&self.lines[self.line - 1]))
        } else {
            (self.line, 0)
        };
        self.kill_range(target, (self.line, self.col), true);
    }

    /// `tui.editor.deleteToLineEnd`.
    pub fn delete_to_line_end(&mut self) {
        let len = char_len(&self.lines[self.line]);
        let target = if self.col == len && !self.on_last_line() {
            (self.line + 1, 0)
        } else {
            (self.line, len)
        };
        self.kill_range((self.line, self.col), target, false);
    }

    /// `tui.editor.yank`.
    pub fn yank(&mut self) {
        let Some(text) = self.kill_ring.last().cloned() else {
            return;
        };
        self.edit(LastEdit::Yank);
        self.raw_insert(&text);
        self.yank_index = self.kill_ring.len() - 1;
        self.last_yank_len = char_len(&text);
        self.last_edit = LastEdit::Yank;
    }

    /// `tui.editor.yankPop`: replaces the last yank with the previous kill.
    pub fn yank_pop(&mut self) {
        if self.last_edit != LastEdit::Yank || self.kill_ring.len() < 2 {
            return;
        }
        let end = (self.line, self.col);
        let mut start = end;
        for _ in 0..self.last_yank_len {
            if start.1 == 0 {
                if start.0 == 0 {
                    break;
                }
                start = (start.0 - 1, char_len(&self.lines[start.0 - 1]));
            } else {
                start.1 -= 1;
            }
        }
        self.take_range(start, end);
        self.yank_index = if self.yank_index == 0 {
            self.kill_ring.len() - 1
        } else {
            self.yank_index - 1
        };
        let text = self.kill_ring[self.yank_index].clone();
        self.raw_insert(&text);
        self.last_yank_len = char_len(&text);
        self.last_edit = LastEdit::Yank;
    }

    /// `tui.editor.undo`.
    pub fn undo(&mut self) {
        if let Some(snap) = self.undo.pop() {
            self.lines = snap.lines;
            self.line = snap.line;
            self.col = snap.col;
            self.last_edit = LastEdit::None;
        }
    }

    fn moved(&mut self) {
        self.last_edit = LastEdit::None;
    }

    /// Cursor left (wraps to the previous line).
    pub fn left(&mut self) {
        self.moved();
        if self.col > 0 {
            self.col -= 1;
        } else if self.line > 0 {
            self.line -= 1;
            self.col = char_len(&self.lines[self.line]);
        }
    }

    /// Cursor right (wraps to the next line).
    pub fn right(&mut self) {
        self.moved();
        if self.col < char_len(&self.lines[self.line]) {
            self.col += 1;
        } else if !self.on_last_line() {
            self.line += 1;
            self.col = 0;
        }
    }

    /// Word left.
    pub fn word_left(&mut self) {
        self.moved();
        (self.line, self.col) = self.word_left_col();
    }

    /// Word right.
    pub fn word_right(&mut self) {
        self.moved();
        (self.line, self.col) = self.word_right_col();
    }

    /// Line start.
    pub fn home(&mut self) {
        self.moved();
        self.col = 0;
    }

    /// Line end.
    pub fn end(&mut self) {
        self.moved();
        self.col = char_len(&self.lines[self.line]);
    }

    /// Cursor up one logical line; returns `false` at the first line.
    pub fn up(&mut self) -> bool {
        self.moved();
        if self.line == 0 {
            return false;
        }
        self.line -= 1;
        self.col = self.col.min(char_len(&self.lines[self.line]));
        true
    }

    /// Cursor down one logical line; returns `false` at the last line.
    pub fn down(&mut self) -> bool {
        self.moved();
        if self.on_last_line() {
            return false;
        }
        self.line += 1;
        self.col = self.col.min(char_len(&self.lines[self.line]));
        true
    }

    /// `tui.editor.jumpForward` / `jumpBackward`: moves to the next/previous `c`.
    pub fn jump(&mut self, c: char, forward: bool) {
        self.moved();
        let chars: Vec<char> = self.lines[self.line].chars().collect();
        if forward {
            if let Some(pos) = chars.iter().skip(self.col + 1).position(|&x| x == c) {
                self.col += pos + 1;
            }
        } else if let Some(pos) = chars[..self.col.min(chars.len())]
            .iter()
            .rposition(|&x| x == c)
        {
            self.col = pos;
        }
    }

    /// Adds a submitted prompt to history (dedups consecutive entries).
    pub fn push_history(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || self.history.last().is_some_and(|h| h == text) {
            return;
        }
        self.history.push(text.to_owned());
        if self.history.len() > HISTORY_LIMIT {
            self.history.remove(0);
        }
    }

    /// Older history entry; returns `false` when there is none.
    pub fn history_prev(&mut self) -> bool {
        if self.history.is_empty() {
            return false;
        }
        let next = match self.history_index {
            None => {
                self.draft = Some(self.text());
                self.history.len() - 1
            }
            Some(0) => return false,
            Some(i) => i - 1,
        };
        self.history_index = Some(next);
        let text = self.history[next].clone();
        self.set_text(&text);
        self.history_index = Some(next);
        true
    }

    /// Newer history entry (restores the draft past the newest); `false` when not browsing.
    pub fn history_next(&mut self) -> bool {
        let Some(index) = self.history_index else {
            return false;
        };
        if index + 1 >= self.history.len() {
            let draft = self.draft.take().unwrap_or_default();
            self.set_text(&draft);
            self.history_index = None;
        } else {
            let text = self.history[index + 1].clone();
            self.set_text(&text);
            self.history_index = Some(index + 1);
        }
        true
    }

    /// The token under construction before the cursor (for `/` and `@` completion).
    #[must_use]
    pub fn current_token(&self) -> &str {
        let line = &self.lines[self.line];
        let before = &line[..byte_at(line, self.col)];
        let start = before.rfind(char::is_whitespace).map_or(0, |i| i + 1);
        &before[start..]
    }

    /// The cursor as a character offset into [`Self::text`] (`get_completions` `cursor`).
    #[must_use]
    pub fn cursor_offset(&self) -> usize {
        self.lines[..self.line]
            .iter()
            .map(|l| char_len(l) + 1)
            .sum::<usize>()
            + self.col
    }

    /// Replaces the text between character offset `from` (clamped to the cursor line) and
    /// the cursor with `replacement` (an accepted `get_completions` item).
    pub fn replace_before_cursor(&mut self, from: usize, replacement: &str) {
        let line_start = self.cursor_offset() - self.col;
        let col = from.clamp(line_start, line_start + self.col) - line_start;
        self.edit(LastEdit::None);
        self.take_range((self.line, col), (self.line, self.col));
        self.raw_insert(replacement);
    }

    /// Replaces the token before the cursor with `replacement`.
    pub fn replace_current_token(&mut self, replacement: &str) {
        let token_len = char_len(self.current_token());
        self.edit(LastEdit::None);
        let start = (self.line, self.col - token_len);
        self.take_range(start, (self.line, self.col));
        self.raw_insert(replacement);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Composer {
        let mut c = Composer::new();
        for ch in text.chars() {
            c.insert_char(ch);
        }
        c
    }

    #[test]
    fn offsets_and_range_replacement() {
        let mut c = typed("one\n/probe se");
        assert_eq!(c.cursor_offset(), 13);
        c.replace_before_cursor(11, "select");
        assert_eq!(c.text(), "one\n/probe select");
        c.replace_before_cursor(0, "/x");
        assert_eq!(c.text(), "one\n/x");
    }

    #[test]
    fn editing_and_cursor_movement() {
        let mut c = typed("hello world");
        c.word_left();
        assert_eq!(c.cursor(), (0, 6));
        c.delete_word_backward();
        assert_eq!(c.text(), "world");
        c.end();
        c.newline();
        c.insert_str("two");
        assert_eq!(c.lines().len(), 2);
        assert!(c.up());
        assert_eq!(c.cursor().0, 0);
        c.home();
        c.delete_to_line_end();
        assert_eq!(c.text(), "\ntwo");
        c.yank();
        assert_eq!(c.text(), "world\ntwo");
        c.undo();
        assert_eq!(c.text(), "\ntwo");
    }

    #[test]
    fn kill_ring_accumulates_and_yank_pops() {
        let mut c = typed("aa bb cc");
        c.delete_word_backward();
        c.delete_word_backward();
        assert_eq!(c.text(), "aa ");
        c.yank();
        assert_eq!(c.text(), "aa bb cc");
        c.home();
        c.delete_to_line_end();
        c.yank();
        c.yank_pop();
        assert_eq!(c.text(), "bb cc");
    }

    #[test]
    fn history_and_draft() {
        let mut c = Composer::new();
        c.push_history("first");
        c.push_history("second");
        c.insert_str("draft");
        assert!(c.history_prev());
        assert_eq!(c.text(), "second");
        assert!(c.history_prev());
        assert_eq!(c.text(), "first");
        assert!(!c.history_prev());
        assert!(c.history_next());
        assert!(c.history_next());
        assert_eq!(c.text(), "draft");
    }

    #[test]
    fn large_paste_collapses_and_expands() {
        let mut c = typed("see ");
        let big: String = (0..12).fold(String::new(), |acc, i| acc + &format!("line {i}\n"));
        c.paste(&big);
        assert_eq!(c.text(), "see [paste #1 +13 lines]");
        let sub = c.submission();
        assert!(sub.text.starts_with("see line 0\nline 1"));
        c.clear();
        c.insert_attachment(
            "",
            None,
            Some(ImageContent {
                data: "AA".into(),
                mime_type: "image/png".into(),
            }),
            None,
        );
        c.insert_str("what is this");
        let sub = c.submission();
        assert_eq!(sub.text, "what is this");
        assert_eq!(sub.images.len(), 1);
    }

    #[test]
    fn token_replacement() {
        let mut c = typed("/mo");
        assert_eq!(c.current_token(), "/mo");
        c.replace_current_token("/model ");
        assert_eq!(c.text(), "/model ");
        c.jump('m', false);
        assert_eq!(c.cursor(), (0, 1));
    }
}
