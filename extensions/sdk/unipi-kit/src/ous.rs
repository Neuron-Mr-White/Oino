//! Small OUS v1 node builders (ui-protocol §6.1) shared by unipi ports.
//!
//! Nodes are plain JSON (`{"t": kind, ...}`); these helpers only remove boilerplate. Tones are
//! the ui-protocol §6.5 names (`muted`, `accent`, `error`, `c1`…).

use serde_json::{Value, json};

/// A text span (`Span = {text, tone?, bold?, dim?}`).
#[must_use]
pub fn span(text: &str, tone: Option<&str>) -> Value {
    let mut s = json!({"text": text});
    if let Some(t) = tone {
        s["tone"] = json!(t);
    }
    s
}

/// A dim span.
#[must_use]
pub fn dim(text: &str) -> Value {
    json!({"text": text, "dim": true})
}

/// A bold span with a tone.
#[must_use]
pub fn bold(text: &str, tone: &str) -> Value {
    json!({"text": text, "tone": tone, "bold": true})
}

/// `text` node from a string or span list.
#[must_use]
pub fn text(value: impl Into<Value>) -> Value {
    json!({"t": "text", "value": value.into()})
}

/// `text` node without wrapping.
#[must_use]
pub fn text_nowrap(value: impl Into<Value>) -> Value {
    json!({"t": "text", "value": value.into(), "wrap": false})
}

/// `stack` node.
#[must_use]
pub fn stack(direction: &str, gap: u32, children: &[Value]) -> Value {
    json!({"t": "stack", "direction": direction, "gap": gap, "children": children})
}

/// `keyHints` node.
#[must_use]
pub fn key_hints(hints: &[(&str, &str)]) -> Value {
    json!({"t": "keyHints", "hints": hints.iter().map(|(k, l)| json!({"keys": k, "label": l})).collect::<Vec<_>>()})
}

/// `empty` node.
#[must_use]
pub fn empty(message: &str) -> Value {
    json!({"t": "empty", "message": message})
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_nodes() {
        assert_eq!(
            span("x", Some("error")),
            json!({"text": "x", "tone": "error"})
        );
        assert_eq!(text("hi"), json!({"t": "text", "value": "hi"}));
        assert_eq!(
            text(vec![dim("a")]),
            json!({"t": "text", "value": [{"text": "a", "dim": true}]})
        );
        assert_eq!(
            stack("vertical", 0, &[empty("none")])["children"][0]["t"],
            "empty"
        );
        assert_eq!(
            key_hints(&[("esc", "close")])["hints"][0],
            json!({"keys": "esc", "label": "close"})
        );
        assert_eq!(text_nowrap("x")["wrap"], false);
        assert_eq!(bold("U", "c1")["bold"], true);
    }
}
