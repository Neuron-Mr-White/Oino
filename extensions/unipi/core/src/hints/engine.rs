//! Per-session hint engine (unipi `installHints` state machine), host-independent.
//!
//! Every handler mutates a [`Session`] through an [`Engine`] (catalog + store + clock) and
//! records [`Effect`]s; the extension renders them (`setWidget`) and persists the store when
//! [`Engine::dirty`] is set.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::catalog::{self, Hint, MAX_SESSION_EVENTS};
use super::store::StoreData;

/// Rendering effect.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Show hint `id`.
    Show(String),
    /// Clear the widget.
    Clear,
}

/// One handler invocation's context.
pub struct Engine<'a> {
    /// Catalog.
    pub hints: &'a [Hint],
    /// Persisted history.
    pub store: &'a mut StoreData,
    /// Wall clock (epoch ms).
    pub now_ms: i64,
    /// Effects in order.
    pub effects: Vec<Effect>,
    /// The store changed.
    pub dirty: bool,
    /// Hints may show (UI attached and `hints.enabled`); when `false` only bookkeeping runs.
    pub enabled: bool,
}

impl<'a> Engine<'a> {
    /// New context.
    pub fn new(hints: &'a [Hint], store: &'a mut StoreData, now_ms: i64) -> Self {
        Self {
            hints,
            store,
            now_ms,
            effects: Vec::new(),
            dirty: false,
            enabled: true,
        }
    }

    fn hint(&self, id: &str) -> Option<&'a Hint> {
        self.hints.iter().find(|h| h.id == id)
    }
}

/// Per-session state.
#[derive(Debug, Default)]
pub struct Session {
    active: Option<String>,
    input_seq: u64,
    shown_seq: Option<u64>,
    turn_hint_shown_seq: Option<u64>,
    event_hints: u32,
    context_high_fired: bool,
    consecutive_errors: u32,
    clean_streak: u32,
    once: BTreeSet<String>,
    bash_starts: BTreeMap<String, i64>,
    history: Vec<String>,
    history_index: Option<usize>,
}

/// Bash calls at least this long fire `hints:long-bash`.
pub const LONG_BASH_MS: i64 = 30_000;

impl Session {
    /// Active hint id.
    #[must_use]
    pub fn active(&self) -> Option<&str> {
        self.active.as_deref()
    }

    /// Shows `id` (recording a show when `record`).
    pub fn show(&mut self, e: &mut Engine<'_>, id: &str, record: bool) {
        if !e.enabled {
            return;
        }
        if record {
            e.store.record_show(id, e.now_ms);
            e.dirty = true;
        }
        self.active = Some(id.to_owned());
        self.shown_seq = Some(self.input_seq);
        self.turn_hint_shown_seq = Some(self.input_seq);
        let at_top = self
            .history_index
            .and_then(|i| self.history.get(i))
            .is_some_and(|h| h == id);
        if !at_top {
            self.history.push(id.to_owned());
            self.history_index = Some(self.history.len() - 1);
        }
        e.effects.push(Effect::Show(id.to_owned()));
    }

    /// Clears the active hint.
    pub fn clear(&mut self, e: &mut Engine<'_>) {
        if self.active.take().is_some() {
            self.shown_seq = None;
            e.effects.push(Effect::Clear);
        }
    }

    /// `session_start`: resets the session and picks the startup (or what's-new) hint.
    pub fn start(&mut self, e: &mut Engine<'_>, current_version: &str) {
        *self = Self::default();
        if !e.enabled {
            return;
        }
        let last_seen = e.store.last_seen_version.clone();
        let pick = match last_seen.as_deref() {
            Some(seen) if seen != current_version => catalog::pick_whats_new(
                e.hints,
                seen,
                current_version,
                &e.store.counts,
                &e.store.learned,
            )
            .or_else(|| catalog::pick_startup(e.hints, &e.store.counts, &e.store.learned)),
            _ => catalog::pick_startup(e.hints, &e.store.counts, &e.store.learned),
        }
        .map(|h| h.id.clone());
        if last_seen.as_deref() != Some(current_version) {
            e.store.last_seen_version = Some(current_version.to_owned());
            e.dirty = true;
        }
        if let Some(id) = pick {
            self.show(e, &id, true);
        }
    }

    /// An event that may trigger an event hint (pi event, bus topic or derived `hints:*`).
    pub fn fire(&mut self, e: &mut Engine<'_>, name: &str, payload: &Value) {
        if !e.enabled
            || self.turn_hint_shown_seq == Some(self.input_seq)
            || self.event_hints >= MAX_SESSION_EVENTS
        {
            return;
        }
        if let Some(h) =
            catalog::pick_event(e.hints, name, payload, &e.store.counts, &e.store.learned)
        {
            let id = h.id.clone();
            self.event_hints += 1;
            self.show(e, &id, true);
        }
    }

    fn fire_once(&mut self, e: &mut Engine<'_>, name: &str, payload: &Value) {
        if self.once.insert(name.to_owned()) {
            self.fire(e, name, payload);
        }
    }

    /// `input`: new turn, clear, learned tokens, derived `long-prompt`/`remember`/`image-input`.
    pub fn on_input(&mut self, e: &mut Engine<'_>, text: &str, images: usize) {
        self.input_seq += 1;
        self.clear(e);
        if let Some(first) = text.split_whitespace().next() {
            for h in e.hints {
                if h.teaches.as_deref() == Some(first) && e.store.record_learned(&h.id) {
                    e.dirty = true;
                }
            }
        }
        if !text.is_empty() && !text.starts_with('/') {
            let paragraphs = paragraph_count(text);
            let length = text.encode_utf16().count();
            if length >= 800 || paragraphs >= 3 {
                self.fire(
                    e,
                    "hints:long-prompt",
                    &json!({"length": length, "paragraphs": paragraphs}),
                );
            }
        }
        if has_word_ci(text, "remember") {
            self.fire(e, "hints:remember", &json!({"text": text}));
        }
        if images > 0 {
            self.fire(e, "hints:image-input", &json!({"count": images}));
        }
    }

    /// `before_agent_start`: a hint shown in an earlier turn goes away.
    pub fn before_agent_start(&mut self, e: &mut Engine<'_>) {
        if self.shown_seq != Some(self.input_seq) {
            self.clear(e);
        }
    }

    /// `turn_end`: context ≥ 70 % (once per session), late night (01:00–04:59 local), hot cache
    /// (last reply ≥ 20k context with ≥ 95 % cache reads).
    pub fn on_turn_end(
        &mut self,
        e: &mut Engine<'_>,
        context_percent: Option<f64>,
        local_hour: Option<u32>,
        last_usage: Option<(f64, f64)>,
    ) {
        if !self.context_high_fired
            && let Some(p) = context_percent.filter(|p| *p >= 70.0)
        {
            self.context_high_fired = true;
            self.fire(e, "hints:context-high", &json!({"percent": p}));
        }
        if let Some(h) = local_hour.filter(|h| (1..5).contains(h)) {
            self.fire_once(e, "hints:late-night", &json!({"hour": h}));
        }
        if let Some((input, cache_read)) = last_usage {
            let total = input + cache_read;
            if total >= 20_000.0 && cache_read / total >= 0.95 {
                self.fire_once(e, "hints:cache-hot", &json!({"ratio": cache_read / total}));
            }
        }
    }

    /// `tool_execution_start`.
    pub fn on_tool_start(&mut self, tool: &str, call_id: &str, now_ms: i64) {
        if tool == "bash" && !call_id.is_empty() {
            self.bash_starts.insert(call_id.to_owned(), now_ms);
        }
    }

    /// `tool_execution_end`: bash ≥ 30 s fires `hints:long-bash`.
    pub fn on_tool_end(&mut self, e: &mut Engine<'_>, tool: &str, call_id: &str) {
        if tool != "bash" {
            return;
        }
        if let Some(start) = self.bash_starts.remove(call_id) {
            let duration = e.now_ms - start;
            if duration >= LONG_BASH_MS {
                self.fire(e, "hints:long-bash", &json!({"durationMs": duration}));
            }
        }
    }

    /// `tool_result`: 3 errors in a row (then reset), 50 clean results once.
    pub fn on_tool_result(&mut self, e: &mut Engine<'_>, is_error: bool) {
        if is_error {
            self.clean_streak = 0;
            self.consecutive_errors += 1;
            if self.consecutive_errors >= 3 {
                self.consecutive_errors = 0;
                self.fire(e, "hints:tool-errors", &json!({"count": 3}));
            }
        } else {
            self.consecutive_errors = 0;
            self.clean_streak += 1;
            if self.clean_streak == 50 {
                self.fire_once(e, "hints:clean-streak", &json!({"count": 50}));
            }
        }
    }

    /// `tool_call`: `rm -rf`, force push, ≥ 40 000-char write (each once per session).
    pub fn on_tool_call(&mut self, e: &mut Engine<'_>, tool: &str, input: &Value) {
        if tool == "bash" {
            let cmd = input.get("command").map(value_text).unwrap_or_default();
            if is_rm_rf(&cmd) {
                self.fire_once(e, "hints:rm-rf", &json!({}));
            }
            if is_force_push(&cmd) {
                self.fire_once(e, "hints:force-push", &json!({}));
            }
        }
        if tool == "write" {
            let chars = input
                .get("content")
                .map(value_text)
                .unwrap_or_default()
                .encode_utf16()
                .count();
            if chars >= 40_000 {
                self.fire_once(e, "hints:big-write", &json!({"chars": chars}));
            }
        }
    }

    /// Alt+H: forward in the session history, else the next cycle hint.
    pub fn cycle_next(&mut self, e: &mut Engine<'_>) {
        if let Some(i) = self.history_index
            && i + 1 < self.history.len()
        {
            self.history_index = Some(i + 1);
            let id = self.history[i + 1].clone();
            if e.hint(&id).is_some() {
                self.show(e, &id, true);
                return;
            }
        }
        let next = catalog::pick_next_cycle(
            e.hints,
            self.active.as_deref(),
            &e.store.counts,
            &e.store.learned,
        )
        .map(|h| h.id.clone());
        if let Some(id) = next {
            self.show(e, &id, true);
        }
    }

    /// Alt+Shift+H: back in the session history (not recorded as a show).
    pub fn cycle_prev(&mut self, e: &mut Engine<'_>) {
        if let Some(i) = self.history_index.filter(|i| *i > 0) {
            self.history_index = Some(i - 1);
            let id = self.history[i - 1].clone();
            if e.hint(&id).is_some() {
                self.show(e, &id, false);
            }
        }
    }
}

fn value_text(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// `text.trim().split(/\n\s*\n+/).filter(Boolean).length`.
fn paragraph_count(text: &str) -> usize {
    let mut count = 0;
    let mut in_para = false;
    let mut blank_run = false;
    for line in text.trim().lines() {
        if line.trim().is_empty() {
            blank_run = true;
            continue;
        }
        if !in_para || blank_run {
            count += 1;
        }
        in_para = true;
        blank_run = false;
    }
    count
}

/// `\b<word>\b`, ASCII case-insensitive.
fn has_word_ci(text: &str, word: &str) -> bool {
    let lower = text.to_ascii_lowercase();
    lower
        .match_indices(word)
        .any(|(i, _)| word_bounded(&lower, i, word.len()))
}

fn word_bounded(s: &str, start: usize, len: usize) -> bool {
    let before = s[..start].chars().next_back().is_none_or(|c| !is_word(c));
    let after = s[start + len..].chars().next().is_none_or(|c| !is_word(c));
    before && after
}

/// Positions after `\b<word>` followed by at least one whitespace char.
fn after_word_ws<'a>(s: &'a str, word: &str) -> impl Iterator<Item = &'a str> {
    let w = word.to_owned();
    s.match_indices(word).filter_map(move |(i, _)| {
        let before_ok = s[..i].chars().next_back().is_none_or(|c| !is_word(c));
        let rest = &s[i + w.len()..];
        let trimmed = rest.trim_start();
        (before_ok && trimmed.len() < rest.len()).then_some(trimmed)
    })
}

/// `/\brm\s+-(?:[a-z]*r[a-z]*f|[a-z]*f[a-z]*r)\b/i`.
#[must_use]
pub fn is_rm_rf(cmd: &str) -> bool {
    let lower = cmd.to_ascii_lowercase();
    after_word_ws(&lower, "rm").any(|rest| {
        let Some(flags) = rest.strip_prefix('-') else {
            return false;
        };
        let run: String = flags.chars().take_while(char::is_ascii_lowercase).collect();
        let next = flags[run.len()..].chars().next();
        if next.is_some_and(is_word) || run.is_empty() {
            return false;
        }
        let head = &run[..run.len() - 1];
        (run.ends_with('f') && head.contains('r')) || (run.ends_with('r') && head.contains('f'))
    })
}

/// `/\bgit\s+push\b[^|;&]*\s--force(?:-with-lease)?\b|\bgit\s+push\s+-f\b/`.
#[must_use]
pub fn is_force_push(cmd: &str) -> bool {
    after_word_ws(cmd, "git").any(|rest| {
        let Some(after) = rest.strip_prefix("push") else {
            return false;
        };
        if after.chars().next().is_some_and(is_word) {
            return false;
        }
        let segment = after.split(['|', ';', '&']).next().unwrap_or("");
        let long = segment.match_indices("--force").any(|(i, _)| {
            segment[..i]
                .chars()
                .next_back()
                .is_some_and(char::is_whitespace)
                && segment[i + 7..].chars().next().is_none_or(|c| !is_word(c))
        });
        let trimmed = after.trim_start();
        let short = trimmed.len() < after.len()
            && trimmed
                .strip_prefix("-f")
                .is_some_and(|r| r.chars().next().is_none_or(|c| !is_word(c)));
        long || short
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hints::catalog::{Category, Hint, StartupTag, When};

    fn startup(id: &str) -> Hint {
        Hint {
            id: id.into(),
            category: Category::Command,
            text: id.into(),
            when: When::Startup(StartupTag::Startup),
            max_shows: None,
            teaches: None,
            since: None,
        }
    }

    fn event(id: &str, ev: &str) -> Hint {
        Hint {
            category: Category::Explain,
            when: When::Event {
                event: ev.into(),
                matcher: None,
            },
            max_shows: Some(5),
            ..startup(id)
        }
    }

    fn run(
        hints: &[Hint],
        store: &mut StoreData,
        now: i64,
        f: impl FnOnce(&mut Engine<'_>),
    ) -> Vec<Effect> {
        let mut e = Engine::new(hints, store, now);
        f(&mut e);
        e.effects
    }

    #[test]
    fn startup_shows_then_input_clears() {
        let hints = [startup("a")];
        let mut store = StoreData::default();
        let mut s = Session::default();
        let fx = run(&hints, &mut store, 1, |e| s.start(e, "3.0.0"));
        assert_eq!(fx, [Effect::Show("a".into())]);
        assert_eq!(store.last_seen_version.as_deref(), Some("3.0.0"));
        assert_eq!(store.counts["a"].count, 1);
        let fx = run(&hints, &mut store, 2, |e| s.on_input(e, "hello", 0));
        assert_eq!(fx, [Effect::Clear]);
        let fx = run(&hints, &mut store, 3, |e| {
            e.enabled = false;
            s.start(e, "3.0.0");
        });
        assert_eq!(fx, Vec::<Effect>::new(), "disabled: nothing");
    }

    #[test]
    fn whats_new_first_after_upgrade() {
        let new = Hint {
            category: Category::Whatsnew,
            since: Some("3.0.0".into()),
            ..startup("new")
        };
        let hints = [startup("a"), new];
        let mut store = StoreData {
            last_seen_version: Some("3.0.0-alpha.25".into()),
            ..StoreData::default()
        };
        let mut s = Session::default();
        assert_eq!(
            run(&hints, &mut store, 1, |e| s.start(e, "3.0.0")),
            [Effect::Show("new".into())]
        );
        assert_eq!(
            run(&hints, &mut store, 1, |e| s.start(e, "3.0.0")),
            [Effect::Show("a".into())]
        );
    }

    #[test]
    fn session_event_cap_and_same_turn_no_replace() {
        let hints: Vec<Hint> = (1..=5)
            .map(|i| event(&format!("e{i}"), &format!("test:ev{i}")))
            .collect();
        let mut store = StoreData::default();
        let mut s = Session::default();
        run(&hints, &mut store, 0, |e| s.start(e, "v"));
        for i in 1..=4 {
            run(&hints, &mut store, 0, |e| {
                s.on_input(e, &format!("turn {i}"), 0);
            });
            let fx = run(&hints, &mut store, 0, |e| {
                s.fire(e, &format!("test:ev{i}"), &Value::Null);
            });
            assert_eq!(fx, [Effect::Show(format!("e{i}"))]);
        }
        run(&hints, &mut store, 0, |e| s.on_input(e, "turn 5", 0));
        assert_eq!(
            run(&hints, &mut store, 0, |e| s.fire(
                e,
                "test:ev5",
                &Value::Null
            )),
            Vec::<Effect>::new()
        );

        let mut s = Session::default();
        run(&hints, &mut store, 0, |e| s.start(e, "v"));
        run(&hints, &mut store, 0, |e| s.on_input(e, "new turn", 0));
        let mut fresh = StoreData::default();
        assert_eq!(
            run(&hints, &mut fresh, 0, |e| s.fire(
                e,
                "test:ev1",
                &Value::Null
            ))
            .len(),
            1
        );
        assert_eq!(
            run(&hints, &mut fresh, 0, |e| s.fire(
                e,
                "test:ev2",
                &Value::Null
            )),
            Vec::<Effect>::new()
        );
        assert_eq!(s.active(), Some("e1"));
    }

    #[test]
    fn teaches_exact_first_token() {
        let hints = [Hint {
            teaches: Some("/kanboard".into()),
            ..startup("cmd.kanboard")
        }];
        let mut store = StoreData::default();
        let mut s = Session::default();
        run(&hints, &mut store, 0, |e| {
            s.on_input(e, "/kanboard-add my task", 0);
        });
        assert_eq!(store.learned, Vec::<String>::new());
        run(&hints, &mut store, 0, |e| {
            s.on_input(e, "/kanboard open", 0);
        });
        assert_eq!(store.learned, ["cmd.kanboard"]);
    }

    #[test]
    #[allow(
        clippy::too_many_lines,
        reason = "one scenario per detector, kept together like the unipi test file"
    )]
    fn derived_detectors() {
        let names = [
            "hints:context-high",
            "hints:long-bash",
            "hints:tool-errors",
            "hints:long-prompt",
            "hints:remember",
            "hints:image-input",
            "hints:late-night",
            "hints:cache-hot",
        ];
        let hints: Vec<Hint> = names.iter().map(|n| event(n, n)).collect();
        let shown = |s: &mut Session,
                     store: &mut StoreData,
                     now: i64,
                     f: &dyn Fn(&mut Session, &mut Engine<'_>)| {
            let mut e = Engine::new(&hints, store, now);
            s.on_input(&mut e, "next", 0);
            e.effects.clear();
            f(s, &mut e);
            e.effects.into_iter().find_map(|x| {
                if let Effect::Show(id) = x {
                    Some(id)
                } else {
                    None
                }
            })
        };
        let mut store = StoreData::default();
        let mut s = Session::default();
        assert_eq!(
            shown(&mut s, &mut store, 0, &|s, e| s.on_turn_end(
                e,
                Some(69.0),
                None,
                None
            )),
            None
        );
        assert_eq!(
            shown(&mut s, &mut store, 0, &|s, e| s.on_turn_end(
                e,
                Some(71.0),
                None,
                None
            ))
            .as_deref(),
            Some("hints:context-high")
        );
        assert_eq!(
            shown(&mut s, &mut store, 0, &|s, e| s.on_turn_end(
                e,
                Some(90.0),
                None,
                None
            )),
            None,
            "once per session"
        );

        s.on_tool_start("bash", "b1", 1_000);
        assert_eq!(
            shown(&mut s, &mut store, 20_000, &|s, e| s
                .on_tool_end(e, "bash", "b1")),
            None
        );
        s.on_tool_start("bash", "b2", 1_000);
        assert_eq!(
            shown(&mut s, &mut store, 31_000, &|s, e| s
                .on_tool_end(e, "bash", "b2"))
            .as_deref(),
            Some("hints:long-bash")
        );

        let errs = |s: &mut Session, e: &mut Engine<'_>, pattern: &[bool]| {
            for p in pattern {
                s.on_tool_result(e, *p);
            }
        };
        assert_eq!(
            shown(&mut s, &mut store, 0, &|s, e| errs(
                s,
                e,
                &[true, true, false, true, true]
            )),
            None,
            "reset on success"
        );
        assert_eq!(
            shown(&mut s, &mut store, 0, &|s, e| errs(
                s,
                e,
                &[true, true, true]
            ))
            .as_deref(),
            Some("hints:tool-errors")
        );

        // The session cap (4 event hints) is reached; the input detectors get a fresh session.
        let mut s = Session::default();
        let mut store = StoreData::default();
        let mut e = Engine::new(&hints, &mut store, 0);
        s.on_input(&mut e, &"x".repeat(800), 0);
        assert!(
            e.effects
                .contains(&Effect::Show("hints:long-prompt".into()))
        );
        let mut e = Engine::new(&hints, &mut store, 0);
        s.on_input(&mut e, "a\n\nb\n \nc", 0);
        assert!(
            e.effects
                .contains(&Effect::Show("hints:long-prompt".into())),
            "3 paragraphs"
        );
        let mut e = Engine::new(&hints, &mut store, 0);
        s.on_input(&mut e, "/cmd\n\nb\n\nc", 0);
        assert!(
            !e.effects.iter().any(|x| matches!(x, Effect::Show(_))),
            "commands never count"
        );
        let mut fresh = StoreData::default();
        let mut s2 = Session::default();
        let mut e = Engine::new(&hints, &mut fresh, 0);
        s2.on_input(&mut e, "Please REMEMBER this", 0);
        assert!(e.effects.contains(&Effect::Show("hints:remember".into())));
        let mut e = Engine::new(&hints, &mut fresh, 0);
        s2.on_input(&mut e, "remembering is not a word match", 2);
        assert_eq!(
            e.effects.last(),
            Some(&Effect::Show("hints:image-input".into()))
        );
        assert_eq!(
            shown(&mut s2, &mut fresh, 0, &|s, e| s.on_turn_end(
                e,
                None,
                Some(3),
                None
            ))
            .as_deref(),
            Some("hints:late-night")
        );
        assert_eq!(
            shown(&mut s2, &mut fresh, 0, &|s, e| s.on_turn_end(
                e,
                None,
                None,
                Some((500.0, 19_600.0))
            ))
            .as_deref(),
            Some("hints:cache-hot")
        );
    }

    #[test]
    fn fun_detectors_fire_once() {
        let names = [
            "hints:rm-rf",
            "hints:force-push",
            "hints:big-write",
            "hints:clean-streak",
        ];
        let hints: Vec<Hint> = names.iter().map(|n| event(n, n)).collect();
        let mut store = StoreData::default();
        let mut s = Session::default();
        let mut seen = Vec::new();
        for (tool, input) in [
            ("bash", json!({"command": "rm -rf build"})),
            ("bash", json!({"command": "rm -rf again"})),
            ("bash", json!({"command": "git push origin main --force"})),
            ("write", json!({"content": "y".repeat(40_000)})),
        ] {
            let mut e = Engine::new(&hints, &mut store, 0);
            s.on_input(&mut e, "turn", 0);
            s.on_tool_call(&mut e, tool, &input);
            seen.extend(e.effects.into_iter().filter_map(|x| {
                if let Effect::Show(id) = x {
                    Some(id)
                } else {
                    None
                }
            }));
        }
        assert_eq!(seen, ["hints:rm-rf", "hints:force-push", "hints:big-write"]);
        let mut e = Engine::new(&hints, &mut store, 0);
        s.on_input(&mut e, "turn", 0);
        for _ in 0..50 {
            s.on_tool_result(&mut e, false);
        }
        assert_eq!(
            e.effects.last(),
            Some(&Effect::Show("hints:clean-streak".into()))
        );
    }

    #[test]
    fn command_patterns() {
        for yes in [
            "rm -rf x",
            "sudo rm -fr /tmp/a",
            "rm  -Rf x",
            "x && rm -rvf y",
        ] {
            assert!(is_rm_rf(yes), "{yes}");
        }
        for no in [
            "rm -r x",
            "rm file",
            "farm -rf",
            "rm -rf1",
            "echo rm-rf",
            "rm -frv",
        ] {
            assert!(!is_rm_rf(no), "{no}");
        }
        for yes in [
            "git push --force",
            "git push origin main --force-with-lease",
            "git push -f",
            "cd a; git  push -f origin",
        ] {
            assert!(is_force_push(yes), "{yes}");
        }
        for no in [
            "git push origin main",
            "git push | grep --force",
            "git pushx --force",
            "git push origin --forced",
            "git push -fx",
        ] {
            assert!(!is_force_push(no), "{no}");
        }
    }

    #[test]
    fn cycle_history() {
        let hints = [startup("a"), startup("b"), startup("c")];
        let mut store = StoreData::default();
        let mut s = Session::default();
        let ids = |fx: Vec<Effect>| {
            fx.into_iter()
                .filter_map(|x| {
                    if let Effect::Show(id) = x {
                        Some(id)
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(ids(run(&hints, &mut store, 1, |e| s.start(e, "v"))), ["a"]);
        assert_eq!(ids(run(&hints, &mut store, 2, |e| s.cycle_next(e))), ["b"]);
        assert_eq!(ids(run(&hints, &mut store, 3, |e| s.cycle_next(e))), ["c"]);
        assert_eq!(ids(run(&hints, &mut store, 4, |e| s.cycle_prev(e))), ["b"]);
        assert_eq!(store.counts["b"].count, 1, "going back is not a show");
        assert_eq!(ids(run(&hints, &mut store, 5, |e| s.cycle_prev(e))), ["a"]);
        assert_eq!(
            ids(run(&hints, &mut store, 6, |e| s.cycle_prev(e))),
            Vec::<String>::new()
        );
        assert_eq!(
            ids(run(&hints, &mut store, 7, |e| s.cycle_next(e))),
            ["b"],
            "forward through history"
        );
    }
}
