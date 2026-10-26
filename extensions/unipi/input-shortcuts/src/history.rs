//! Linear checkpoint history for the editor (unipi `src/undo-redo.ts`).
//!
//! `states` is the ordered list of editor texts and `index` points at the current one; undo
//! steps back, redo steps forward. `record` is idempotent for the current state. Memory is
//! bounded by [`MAX_STATES`] checkpoints and [`MAX_CHARS`] characters (oldest evicted first).

use std::collections::VecDeque;

/// Checkpoint cap.
pub const MAX_STATES: usize = 100;
/// Character budget across all checkpoints.
pub const MAX_CHARS: usize = 2_000_000;

/// Result of an undo/redo step.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Step {
    /// Text to show.
    pub text: String,
    /// The step happened.
    pub ok: bool,
    /// Why not (`nothing to undo` / `nothing to redo`).
    pub reason: Option<&'static str>,
}

impl Step {
    fn ok(text: &str) -> Self {
        Self {
            text: text.to_owned(),
            ok: true,
            reason: None,
        }
    }

    fn fail(text: &str, reason: &'static str) -> Self {
        Self {
            text: text.to_owned(),
            ok: false,
            reason: Some(reason),
        }
    }
}

/// The history (session-only).
#[derive(Clone, Debug, Default)]
pub struct EditHistory {
    states: VecDeque<String>,
    /// Index of the current state (`None` = empty).
    index: Option<usize>,
    chars: usize,
}

fn len(s: &str) -> usize {
    s.encode_utf16().count()
}

impl EditHistory {
    fn current(&self) -> Option<&String> {
        self.index.and_then(|i| self.states.get(i))
    }

    /// Commits `text` as the current state; no-op when unchanged.
    pub fn record(&mut self, text: &str) {
        if self.current().is_some_and(|c| c == text) {
            return;
        }
        let keep = self.index.map_or(0, |i| i + 1);
        while self.states.len() > keep {
            if let Some(s) = self.states.pop_back() {
                self.chars -= len(&s);
            }
        }
        self.states.push_back(text.to_owned());
        self.chars += len(text);
        self.index = Some(self.states.len() - 1);
        self.evict();
    }

    /// Steps back; `current` is recorded first so un-flushed typing is not lost.
    pub fn undo(&mut self, current: &str) -> Step {
        self.record(current);
        match self.index {
            Some(i) if i > 0 => {
                self.index = Some(i - 1);
                Step::ok(self.states.get(i - 1).map_or("", String::as_str))
            }
            _ => Step::fail(current, "nothing to undo"),
        }
    }

    /// Steps forward; an edit since the last undo (`current` differs) invalidates the redo.
    pub fn redo(&mut self, current: &str) -> Step {
        if self.current().map(String::as_str) != Some(current) {
            self.record(current);
            return Step::fail(current, "nothing to redo");
        }
        match self.index {
            Some(i) if i + 1 < self.states.len() => {
                self.index = Some(i + 1);
                Step::ok(self.states.get(i + 1).map_or("", String::as_str))
            }
            _ => Step::fail(current, "nothing to redo"),
        }
    }

    /// Drops everything (session shutdown).
    pub fn clear(&mut self) {
        *self = Self::default();
    }

    fn evict(&mut self) {
        while self.states.len() > 1 && (self.states.len() > MAX_STATES || self.chars > MAX_CHARS) {
            if let Some(s) = self.states.pop_front() {
                self.chars -= len(&s);
            }
            self.index = self.index.and_then(|i| i.checked_sub(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn step(text: &str) -> Step {
        Step::ok(text)
    }

    #[test]
    fn linear_undo_redo() {
        let mut h = EditHistory::default();
        h.record("");
        h.record("a");
        h.record("ab");
        assert_eq!(h.undo("ab"), step("a"));
        assert_eq!(h.undo("a"), step(""));
        assert!(!h.undo("").ok);
        assert_eq!(h.undo("").reason, Some("nothing to undo"));
        assert_eq!(h.redo(""), step("a"));
        assert_eq!(h.redo("a"), step("ab"));
        assert!(!h.redo("ab").ok);
        assert_eq!(h.redo("ab").reason, Some("nothing to redo"));
    }

    #[test]
    fn dedupe() {
        let mut h = EditHistory::default();
        h.record("same");
        h.record("same");
        assert!(!h.undo("same").ok);
        h.record("same");
        h.record("other");
        assert_eq!(h.undo("other"), step("same"));
    }

    #[test]
    fn new_edit_truncates_redo() {
        let mut h = EditHistory::default();
        h.record("");
        h.record("one");
        assert!(h.undo("one").ok);
        h.record("one!");
        assert!(!h.redo("one!").ok);
        assert_eq!(h.undo("one!"), step(""));
    }

    #[test]
    fn undo_captures_unflushed_typing() {
        let mut h = EditHistory::default();
        h.record("committed");
        assert_eq!(h.undo("committed plus typing"), step("committed"));
        assert_eq!(h.redo("committed"), step("committed plus typing"));
    }

    #[test]
    fn flush_of_restored_text_keeps_redo() {
        let mut h = EditHistory::default();
        h.record("");
        h.record("hello");
        assert!(h.undo("hello").ok);
        assert_eq!(h.redo(""), step("hello"));
        h.record("hello");
        assert!(!h.redo("hello").ok);
        assert_eq!(h.undo("hello"), step(""));
        assert_eq!(h.redo(""), step("hello"));
    }

    #[test]
    fn count_cap() {
        let mut h = EditHistory::default();
        for i in 0..=MAX_STATES {
            h.record(&format!("state {i}"));
        }
        let mut current = format!("state {MAX_STATES}");
        let mut steps = 0;
        loop {
            let r = h.undo(&current);
            if !r.ok {
                break;
            }
            current = r.text;
            steps += 1;
        }
        assert_eq!(steps, MAX_STATES - 1);
        assert_eq!(current, "state 1");
    }

    #[test]
    fn char_budget() {
        let mut h = EditHistory::default();
        let chunk = "x".repeat(900_000);
        h.record(&chunk);
        h.record(&format!("{chunk}a"));
        h.record(&format!("{chunk}b"));
        let first = h.undo(&format!("{chunk}b"));
        assert!(first.ok);
        assert_eq!(first.text, format!("{chunk}a"));
        assert!(!h.undo(&first.text).ok);
        let mut lone = EditHistory::default();
        let huge = "x".repeat(MAX_CHARS + 10);
        lone.record(&huge);
        assert!(!lone.undo(&huge).ok);
    }

    #[test]
    fn clear_resets() {
        let mut h = EditHistory::default();
        h.record("something");
        h.clear();
        assert!(!h.undo("something").ok);
        assert!(!h.redo("something").ok);
    }
}
