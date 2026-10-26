//! Ctrl+C keeps what it cleared (unipi `src/cleared-input.ts`).
//!
//! unipi observed Ctrl+C on the focused editor and appended a UI-only `custom` entry
//! `unipi-cleared-input {text}` rendered struck through with an "↑ restores" hint. Oino gets
//! the `cleared` editor intent instead ([`is_interrupt_clear`]); the renderer is an OUS spec.

use serde_json::{Value, json};

/// Custom entry type.
pub const CLEARED_ENTRY: &str = "unipi-cleared-input";
const MAX_LINES: usize = 3;

/// Decides whether a `cleared` intent is a Ctrl+C-style clear worth keeping.
///
/// `reason` is the optional intent reason (`interrupt` | `submit` | `edit`, see deviations);
/// without it, a clear of non-blank text that did not follow a submit counts.
#[must_use]
pub fn is_interrupt_clear(previous: &str, reason: Option<&str>, after_submit: bool) -> bool {
    if previous.trim().is_empty() {
        return false;
    }
    match reason {
        Some(r) => r == "interrupt",
        None => !after_submit,
    }
}

/// OUS spec of a cleared entry: `⌫ <struck line>  ↑ restores` plus up to two more struck lines
/// and `… N more lines`.
#[must_use]
pub fn render(text: &str) -> Option<Value> {
    let trimmed = text.trim_end();
    if trimmed.is_empty() {
        return None;
    }
    let all: Vec<&str> = trimmed.split('\n').collect();
    let mut rows: Vec<Value> = all
        .iter()
        .take(MAX_LINES)
        .enumerate()
        .map(|(i, line)| {
            let mut spans = vec![
                json!({"text": if i == 0 { "⌫ " } else { "  " }, "dim": true}),
                json!({"text": line, "dim": true, "strike": true}),
            ];
            if i == 0 {
                spans.push(json!({"text": "  ↑ restores", "dim": true}));
            }
            json!({"t": "text", "value": spans, "wrap": false})
        })
        .collect();
    let more = all.len().saturating_sub(MAX_LINES);
    if more > 0 {
        let s = if more == 1 { "" } else { "s" };
        rows.push(json!({"t": "text", "value": [{"text": format!("  … {more} more line{s}"), "dim": true}], "wrap": false}));
    }
    Some(json!({"t": "stack", "direction": "vertical", "gap": 0, "children": rows}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_non_empty_interrupt_clears_count() {
        assert!(is_interrupt_clear("fix the test", Some("interrupt"), false));
        assert!(!is_interrupt_clear("fix the test", Some("submit"), false));
        assert!(!is_interrupt_clear("   ", Some("interrupt"), false));
        assert!(is_interrupt_clear("draft", None, false));
        assert!(!is_interrupt_clear("draft", None, true));
    }

    #[test]
    fn renders_struck_lines_with_restore_hint() {
        let v = render("one\ntwo\nthree\nfour\nfive\n").unwrap_or_default();
        let rows = v["children"].as_array().cloned().unwrap_or_default();
        assert_eq!(rows.len(), 4);
        assert_eq!(
            rows[0]["value"][1],
            json!({"text": "one", "dim": true, "strike": true})
        );
        assert_eq!(rows[0]["value"][2]["text"], "  ↑ restores");
        assert_eq!(rows[1]["value"][0]["text"], "  ");
        assert_eq!(rows[3]["value"][0]["text"], "  … 2 more lines");
        let single = render("only").unwrap_or_default();
        assert_eq!(single["children"].as_array().map(Vec::len), Some(1));
        assert_eq!(render("  \n"), None);
    }
}
