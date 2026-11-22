//! The naming side session (unipi `src/rename/session.ts`).
//!
//! unipi ran a throwaway in-memory pi session whose only tool is `rename_session`. Oino runs
//! the same shape as a nested agent (D8): memory persistence, no seed, no other tools,
//! extensions limited to `unipi.utility` (which serves `rename_session` to children with role
//! [`CHILD_ROLE`]), system prompt replaced. When nested agents are unavailable it falls back
//! to one `models.complete` call offering the same tool, which is equivalent for a one-tool,
//! one-turn session.

use serde_json::{Value, json};

use super::gate::sanitize_name;

/// Child role of the naming session.
pub const CHILD_ROLE: &str = "rename";
/// The only tool of the naming session.
pub const TOOL: &str = "rename_session";
/// Abort the naming session after this long.
pub const TIMEOUT_MS: u64 = 45_000;

/// System prompt of the naming session.
pub const SYSTEM_PROMPT: &str = "You name coding-agent sessions so the user can find them later.\nCall rename_session exactly once with a short title (2-6 words, Title Case) describing the user's current task.\nName the task, not the conversation: 'Kanboard Lane Ordering', not 'User Asks About Lanes'.\nIf the current title still describes the current task, call rename_session with the current title unchanged.\nDo not write anything else.";

/// What the naming session is told.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RenameRequest {
    /// Current session name.
    pub current_name: Option<String>,
    /// Recent user requests (oldest first).
    pub requests: Vec<String>,
    /// `""` = session model, else `provider/model-id`.
    pub model: String,
    /// The gate found the latest request leaves the current title's topic.
    pub topic_changed: bool,
    /// The agent's reply in the triggering round.
    pub reply: String,
}

fn clip(r: &str) -> String {
    r.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(400)
        .collect()
}

/// The single user prompt (unipi `renamePrompt`).
#[must_use]
pub fn rename_prompt(req: &RenameRequest) -> String {
    if req.topic_changed
        && let Some(name) = &req.current_name
    {
        let latest = req.requests.last().map(String::as_str).unwrap_or_default();
        let mut lines = vec![
            format!("Current title: {name}"),
            String::new(),
            "The user has moved on to a different task, so the current title no longer fits. Name the NEW task:".into(),
            format!("- {}", clip(latest)),
        ];
        if !req.reply.is_empty() {
            lines.push(String::new());
            lines.push(format!(
                "What the assistant did about it:\n{}",
                clip(&req.reply)
            ));
        }
        return lines.join("\n");
    }
    let recent: Vec<String> = req.requests[req.requests.len().saturating_sub(4)..]
        .iter()
        .map(|r| format!("- {}", clip(r)))
        .collect();
    let mut lines = vec![
        req.current_name.as_ref().map_or_else(
            || "The session has no title yet.".to_owned(),
            |n| format!("Current title: {n}"),
        ),
        String::new(),
        "Recent user requests (oldest first; the last one is the current task):".into(),
        if recent.is_empty() {
            "- (none)".into()
        } else {
            recent.join("\n")
        },
    ];
    if !req.reply.is_empty() {
        lines.push(String::new());
        lines.push(format!(
            "The assistant's latest reply (use it when the request itself is vague):\n{}",
            clip(&req.reply)
        ));
    }
    lines.join("\n")
}

/// JSON schema of `rename_session`.
#[must_use]
pub fn tool_parameters() -> Value {
    json!({
        "type": "object",
        "properties": {"name": {"type": "string", "description": "The new session title"}},
        "required": ["name"],
    })
}

/// `AgentSpec` of the naming session (`model` = `provider/id` or none for the session model).
#[must_use]
pub fn agent_spec(model: Option<&str>) -> Value {
    let mut spec = json!({
        "label": "session name",
        "thinking": "off",
        "systemPrompt": {"replace": SYSTEM_PROMPT},
        "tools": {"builtin": [], "extensionTools": [TOOL], "deny": []},
        "extensions": {"only": [crate::EXTENSION_ID]},
        "seed": "none",
        "persistence": "memory",
        "childRole": CHILD_ROLE,
        "ui": "none",
        "maxDepth": 1,
    });
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        spec["model"] = json!(m);
    }
    spec
}

/// `CompletionRequest` of the one-shot fallback.
#[must_use]
pub fn completion_request(req: &RenameRequest, model: Option<&str>) -> Value {
    let mut r = json!({
        "context": {
            "systemPrompt": SYSTEM_PROMPT,
            "messages": [{"role": "user", "content": [{"type": "text", "text": rename_prompt(req)}], "timestamp": 0}],
            "tools": [{"name": TOOL, "description": "Set the session title (2-6 words, Title Case).", "parameters": tool_parameters()}],
        },
        "options": {"maxTokens": 200, "reasoning": "off", "toolChoice": "required"},
    });
    if let Some(m) = model.filter(|m| !m.is_empty()) {
        r["model"] = json!(m);
    }
    r
}

/// The sanitized name of the first `rename_session` call in an assistant message.
#[must_use]
pub fn name_from_message(message: &Value) -> Option<String> {
    message
        .get("content")
        .and_then(Value::as_array)?
        .iter()
        .filter(|c| c.get("type").and_then(Value::as_str) == Some("toolCall"))
        .filter(|c| c.get("name").and_then(Value::as_str) == Some(TOOL))
        .find_map(|c| {
            let args = c.get("arguments").or_else(|| c.get("args"))?;
            let args = match args {
                Value::String(s) => serde_json::from_str::<Value>(s).ok()?,
                v => v.clone(),
            };
            Some(sanitize_name(args.get("name")?.as_str()?)).filter(|n| !n.is_empty())
        })
}

/// The sanitized name from a child `ClientEvent` (`tool_execution_start` of `rename_session`).
#[must_use]
pub fn name_from_event(event: &Value) -> Option<String> {
    let ev = event.get("event").unwrap_or(event);
    if ev.get("type").and_then(Value::as_str) != Some("tool_execution_start")
        || ev.get("toolName").and_then(Value::as_str) != Some(TOOL)
    {
        return None;
    }
    let name = ev.get("args")?.get("name")?.as_str()?;
    Some(sanitize_name(name)).filter(|n| !n.is_empty())
}

/// Model candidates: the configured naming model first, then the session model (unipi falls
/// back once to the session model when a configured model fails).
#[must_use]
pub fn candidates(configured: &str, session: Option<&str>) -> Vec<Option<String>> {
    let configured = configured.trim();
    if configured.is_empty() || !configured.contains('/') || Some(configured) == session {
        return vec![None];
    }
    vec![Some(configured.to_owned()), None]
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use super::*;

    #[test]
    fn prompt_uses_current_name_and_recent_requests() {
        let req = RenameRequest {
            current_name: Some("Old Name".into()),
            requests: ["a", "b", "c", "d", "e"].map(String::from).to_vec(),
            ..RenameRequest::default()
        };
        let p = rename_prompt(&req);
        assert!(p.contains("Current title: Old Name"));
        assert!(!p.contains("- a\n"));
        assert!(p.ends_with("- e"));
        let moved = RenameRequest {
            current_name: Some("Kanboard Lanes".into()),
            requests: vec![
                "kanboard stuff".into(),
                "bash one-liner for big files".into(),
            ],
            topic_changed: true,
            ..RenameRequest::default()
        };
        let m = rename_prompt(&moved);
        assert!(m.contains("no longer fits"));
        assert!(!m.contains("kanboard stuff"));
        assert!(m.contains("bash one-liner"));
        let empty = rename_prompt(&RenameRequest::default());
        assert!(empty.contains("no title yet") && empty.contains("- (none)"));
        let with_reply = rename_prompt(&RenameRequest {
            reply: "did   the\nthing".into(),
            ..RenameRequest::default()
        });
        assert!(with_reply.contains("did the thing"));
    }

    #[test]
    fn spec_and_request_shapes() {
        let s = agent_spec(Some("openrouter/x"));
        assert_eq!(s["tools"]["extensionTools"], json!([TOOL]));
        assert_eq!(s["extensions"]["only"], json!(["unipi.utility"]));
        assert_eq!(s["childRole"], CHILD_ROLE);
        assert_eq!(s["model"], "openrouter/x");
        assert!(agent_spec(None).get("model").is_none());
        let r = completion_request(&RenameRequest::default(), None);
        assert_eq!(r["context"]["tools"][0]["name"], TOOL);
        assert!(r.get("model").is_none());
    }

    #[test]
    fn names_from_messages_and_events() {
        let m = json!({"role": "assistant", "content": [
            {"type": "text", "text": "x"},
            {"type": "toolCall", "name": TOOL, "arguments": {"name": "\"Fix Lanes.\""}}
        ]});
        assert_eq!(name_from_message(&m).as_deref(), Some("Fix Lanes"));
        let s = json!({"content": [{"type": "toolCall", "name": TOOL, "arguments": "{\"name\":\"A B\"}"}]});
        assert_eq!(name_from_message(&s).as_deref(), Some("A B"));
        assert_eq!(
            name_from_message(
                &json!({"content": [{"type": "toolCall", "name": "other", "arguments": {"name": "x"}}]})
            ),
            None
        );
        let ev = json!({"type": "oino_agent", "event": {"type": "tool_execution_start", "toolName": TOOL, "args": {"name": "Image Tools"}}});
        assert_eq!(name_from_event(&ev).as_deref(), Some("Image Tools"));
        assert_eq!(
            name_from_event(&json!({"type": "tool_execution_end"})),
            None
        );
    }

    #[test]
    fn model_candidates() {
        assert_eq!(candidates("", Some("a/b")), vec![None]);
        assert_eq!(candidates("a/b", Some("a/b")), vec![None]);
        assert_eq!(
            candidates("x/y", Some("a/b")),
            vec![Some("x/y".into()), None]
        );
        assert_eq!(candidates("bare", None), vec![None]);
    }
}
