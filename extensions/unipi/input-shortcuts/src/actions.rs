//! Chord actions, the last-reply extractor and kanboard capture outcomes (unipi
//! `src/chord-overlay.ts`, `src/last-response.ts`, `doKanboard`).

use serde_json::{Value, json};

/// A chord action.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// `S` stash or restore.
    Stash,
    /// `U` undo.
    Undo,
    /// `R` redo.
    Redo,
    /// `A` append the stash.
    AppendStash,
    /// `Y` copy the last reply.
    CopyLastResponse,
    /// `K` add to the kanboard backlog.
    Kanboard,
}

/// The chord legend in unipi order: `(key, action id, label, action)`.
pub const ROOT_ACTIONS: &[(&str, &str, &str, Action)] = &[
    ("S", "stash", "Stash / Restore", Action::Stash),
    ("U", "undo", "Undo", Action::Undo),
    ("R", "redo", "Redo", Action::Redo),
    ("A", "appendStash", "Append stash", Action::AppendStash),
    (
        "Y",
        "copyLastResponse",
        "Copy last response",
        Action::CopyLastResponse,
    ),
    ("K", "kanboard", "Add to kanboard backlog", Action::Kanboard),
];

impl Action {
    /// Action for a dialog action id or a pressed key (case-insensitive); unknown → `None`
    /// (the overlay just closes).
    #[must_use]
    pub fn parse(id_or_key: &str) -> Option<Self> {
        ROOT_ACTIONS
            .iter()
            .find(|(key, id, _, _)| *id == id_or_key || key.eq_ignore_ascii_case(id_or_key))
            .map(|(_, _, _, a)| *a)
    }
}

/// The chord overlay as a `custom` dialog (ui-protocol §4.3): one button per action plus
/// single-letter keys (Oino extension field `keys`; Esc or any other key closes).
#[must_use]
pub fn chord_dialog() -> Value {
    let buttons: Vec<Value> = ROOT_ACTIONS
        .iter()
        .map(|(key, id, label, _)| {
            json!({"t": "button", "key": id, "label": format!("[{key}] {label}"), "actionId": id})
        })
        .collect();
    json!({
        "method": "custom",
        "title": "Input Shortcuts",
        "size": "small",
        "spec": {"t": "stack", "direction": "vertical", "gap": 0, "children": buttons},
        "keys": ROOT_ACTIONS.iter().map(|(key, id, label, _)| {
            json!({"key": key.to_lowercase(), "actionId": id, "label": label})
        }).collect::<Vec<_>>(),
        "closeOnUnknownKey": true,
    })
}

/// The text of the last assistant response on the branch (unipi `getLastResponseText`, = pi's
/// `/copy`): text blocks joined, thinking/tool calls skipped, aborted empty messages and
/// whitespace-only replies skipped.
#[must_use]
pub fn last_response_text(entries: &[Value]) -> Option<String> {
    entries.iter().rev().find_map(|entry| {
        if entry.get("type")?.as_str()? != "message" {
            return None;
        }
        let m = entry.get("message")?;
        if m.get("role")?.as_str()? != "assistant" {
            return None;
        }
        let content = m.get("content").and_then(Value::as_array);
        if m.get("stopReason").and_then(Value::as_str) == Some("aborted")
            && content.is_none_or(Vec::is_empty)
        {
            return None;
        }
        let text: String = content
            .into_iter()
            .flatten()
            .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|b| b.get("text").and_then(Value::as_str))
            .collect();
        let text = text.trim();
        (!text.is_empty()).then(|| text.to_owned())
    })
}

/// Outcome of a kanboard capture reply (`{ok, id, attachments}` / `{ok: false, reason}`).
///
/// # Errors
///
/// The capture was refused; the error is the status text.
pub fn capture_outcome(reply: &Value) -> Result<(String, u64), String> {
    if reply.get("ok").and_then(Value::as_bool) == Some(true) {
        let id = reply
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned();
        let n = reply
            .get("attachments")
            .and_then(Value::as_u64)
            .unwrap_or(0);
        return Ok((id, n));
    }
    Err(reply
        .get("reason")
        .and_then(Value::as_str)
        .unwrap_or("kanboard: capture failed")
        .to_owned())
}

/// `✓ T-12 added to Backlog (1 attachment)`.
#[must_use]
pub fn capture_status(id: &str, attachments: u64) -> String {
    let s = if attachments == 1 { "" } else { "s" };
    format!("✓ {id} added to Backlog ({attachments} attachment{s})")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(role: &str, content: &Value) -> Value {
        json!({"type": "message", "message": {"role": role, "content": content}})
    }

    #[test]
    fn chord_keys_map_like_unipi() {
        for (key, action) in [
            ("s", Action::Stash),
            ("U", Action::Undo),
            ("r", Action::Redo),
            ("a", Action::AppendStash),
            ("y", Action::CopyLastResponse),
            ("k", Action::Kanboard),
            ("kanboard", Action::Kanboard),
        ] {
            assert_eq!(Action::parse(key), Some(action), "{key}");
        }
        assert_eq!(Action::parse("x"), None);
        assert_eq!(Action::parse("escape"), None);
        let d = chord_dialog();
        assert_eq!(d["method"], "custom");
        assert_eq!(d["title"], "Input Shortcuts");
        assert_eq!(d["spec"]["children"][0]["label"], "[S] Stash / Restore");
        assert_eq!(
            d["keys"][5],
            json!({"key": "k", "actionId": "kanboard", "label": "Add to kanboard backlog"})
        );
    }

    #[test]
    fn last_response_rules() {
        assert_eq!(last_response_text(&[]), None);
        assert_eq!(last_response_text(&[msg("user", &json!("hi"))]), None);
        let branch = [
            msg("assistant", &json!([{"type": "text", "text": "first"}])),
            msg("assistant", &json!([{"type": "text", "text": "second"}])),
        ];
        assert_eq!(last_response_text(&branch).as_deref(), Some("second"));
        let joined = [msg(
            "assistant",
            &json!([{"type": "thinking", "thinking": "hmm"}, {"type": "text", "text": "a"}, {"type": "toolCall", "name": "x"}, {"type": "text", "text": "b"}]),
        )];
        assert_eq!(last_response_text(&joined).as_deref(), Some("ab"));
        let tools_only = [
            msg("assistant", &json!([{"type": "text", "text": "answer"}])),
            msg("assistant", &json!([{"type": "toolCall", "name": "x"}])),
        ];
        assert_eq!(last_response_text(&tools_only).as_deref(), Some("answer"));
        let aborted = [
            msg("assistant", &json!([{"type": "text", "text": "kept"}])),
            json!({"type": "message", "message": {"role": "assistant", "stopReason": "aborted", "content": []}}),
            msg("assistant", &json!([{"type": "text", "text": "   "}])),
            json!({"type": "compaction", "summary": "s"}),
        ];
        assert_eq!(last_response_text(&aborted).as_deref(), Some("kept"));
    }

    #[test]
    fn kanboard_outcomes() {
        assert_eq!(
            capture_outcome(&json!({"ok": true, "id": "T-1", "attachments": 2})),
            Ok(("T-1".to_owned(), 2))
        );
        assert_eq!(
            capture_outcome(
                &json!({"ok": false, "reason": "kanboard is not set up here — run /kanboard"})
            ),
            Err("kanboard is not set up here — run /kanboard".to_owned())
        );
        assert_eq!(
            capture_status("T-1", 1),
            "✓ T-1 added to Backlog (1 attachment)"
        );
        assert_eq!(
            capture_status("T-2", 0),
            "✓ T-2 added to Backlog (0 attachments)"
        );
    }
}
