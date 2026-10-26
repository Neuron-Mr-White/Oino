//! Notification texts (unipi `input-needed-message.ts`, `ask-user-prompt-message.ts`,
//! `permission-prompt-message.ts`, `events.ts` message builders, `activity.ts`).

use serde_json::Value;

/// Product name in titles (`Pi — …` in unipi; D13 substitution).
pub(crate) const PRODUCT: &str = "Oino";

/// Longest prompt title kept verbatim (unipi `INPUT_TITLE_MAX_CHARS`).
pub(crate) const INPUT_TITLE_MAX_CHARS: usize = 120;

/// unipi `BUILTIN_EVENTS` labels.
pub(crate) fn event_label(event: &str) -> &'static str {
    match event {
        "agent_end" => "Agent Run Complete",
        "agent_settled" => "Agent Complete",
        "ralph_loop_end" => "Ralph Complete",
        "mcp_server_error" => "MCP Error",
        "session_shutdown" => "Session End",
        "ask_user_prompt" => "Question Asked",
        "permission_request" => "Permission Request",
        "input_needed" => "Input Needed",
        _ => "Notification",
    }
}

/// `Oino — <label>`.
pub(crate) fn title_for(event: &str) -> String {
    format!("{PRODUCT} — {}", event_label(event))
}

/// Human-blocking events: high priority, renotify, never muted by recent input.
pub(crate) fn is_blocking(event: &str) -> bool {
    matches!(
        event,
        "ask_user_prompt" | "permission_request" | "input_needed"
    )
}

/// Agent lifecycle notifications with custom handling.
pub(crate) fn is_agent_event(event: &str) -> bool {
    matches!(event, "agent_end" | "agent_settled")
}

/// unipi `defaultEventPriority`.
pub(crate) fn default_priority(event: &str) -> Option<&'static str> {
    if is_blocking(event) {
        Some("high")
    } else if is_agent_event(event) {
        Some("low")
    } else {
        None
    }
}

fn non_empty<'a>(v: Option<&'a Value>, fallback: &'a str) -> &'a str {
    v.and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(fallback)
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_owned()
    } else {
        format!("{}…", text.chars().take(max).collect::<String>())
    }
}

/// unipi `buildInputNeededMessage` (`ui_prompt_start` payload `{title?, kind?}`).
pub(crate) fn input_needed(payload: &Value) -> String {
    let title = payload
        .get("title")
        .and_then(Value::as_str)
        .map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    if title.is_empty() {
        let kind = payload
            .get("kind")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|k| !k.is_empty())
            .unwrap_or("prompt");
        return format!("Waiting for your input ({kind})");
    }
    format!(
        "Waiting for your input: {}",
        truncate_chars(&title, INPUT_TITLE_MAX_CHARS)
    )
}

/// unipi `buildAskUserPromptMessage` (flat `unipi:ask-user:prompt` or rpiv questionnaire).
pub(crate) fn ask_user_prompt(payload: &Value) -> String {
    let p = if payload.is_object() {
        payload
    } else {
        &Value::Null
    };
    let questions: Vec<&Value> = p
        .get("questions")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter(|q| q.is_object()).collect())
        .unwrap_or_default();
    if questions.is_empty() && (p.get("question").is_some() || p.get("context").is_some()) {
        let question = non_empty(p.get("question"), "A question");
        let context = non_empty(p.get("context"), "");
        return if context.is_empty() {
            format!("Agent asks: {question}")
        } else {
            format!("Agent asks: {question} — {context}")
        };
    }
    let first = questions.first();
    let base = first.map_or("A question", |q| non_empty(q.get("question"), "A question"));
    let suffix = if questions.len() > 1 {
        format!(" (+{} more)", questions.len() - 1)
    } else {
        String::new()
    };
    let labels: Vec<&str> = first
        .and_then(|q| q.get("options"))
        .and_then(Value::as_array)
        .map(|opts| {
            opts.iter()
                .filter(|o| o.is_object())
                .map(|o| non_empty(o.get("label"), ""))
                .filter(|l| !l.is_empty())
                .collect()
        })
        .unwrap_or_default();
    if labels.is_empty() {
        format!("Agent asks: {base}{suffix}")
    } else {
        format!("Agent asks: {base}{suffix} — {}", labels.join(", "))
    }
}

fn clean(v: Option<&Value>) -> Option<String> {
    v.and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToOwned::to_owned)
}

fn is_forwarded(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// unipi `buildPermissionPromptMessage` (`permissions:ui_prompt` payload).
pub(crate) fn permission_prompt(payload: &Value) -> String {
    let agent = clean(payload.get("agentName"));
    let surface = clean(payload.get("surface"));
    let value = clean(payload.get("value"));
    let message = clean(payload.get("message"));
    let who = agent.as_deref().unwrap_or("Agent");
    let clause = match (&surface, &value) {
        (Some(s), Some(v)) => Some(format!("{who} requested {s} '{v}'.")),
        (Some(s), None) => Some(format!("{who} requested {s} access.")),
        (None, Some(v)) => Some(format!("{who} requested '{v}'.")),
        (None, None) => None,
    };
    let mut parts: Vec<String> = Vec::new();
    if let Some(c) = &clause {
        parts.push(c.clone());
    }
    if let Some(m) = message
        && Some(&m) != clause.as_ref()
    {
        parts.push(m);
    }
    if parts.is_empty() {
        parts.push(format!("{PRODUCT} is waiting for a permission decision."));
    }
    if is_forwarded(payload.get("forwarding")) {
        parts.push("(forwarded)".to_owned());
    }
    parts.join(" ")
}

/// unipi `buildEventMessage`.
pub(crate) fn event_message(event: &str, payload: &Value) -> String {
    let s = |k: &str, d: &str| -> String {
        payload
            .get(k)
            .and_then(|v| {
                v.as_str()
                    .map(ToOwned::to_owned)
                    .or_else(|| (!v.is_null()).then(|| v.to_string()))
            })
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| d.to_owned())
    };
    match event {
        "ralph_loop_end" => format!(
            "Ralph loop \"{}\" {}",
            s("name", "unknown"),
            s("status", "completed")
        ),
        "mcp_server_error" => format!(
            "Server \"{}\" error: {}",
            s("name", "unknown"),
            s("error", "unknown error")
        ),
        "agent_end" => "Agent run finished responding".to_owned(),
        "agent_settled" => "Agent is complete".to_owned(),
        "session_shutdown" => "Session ending".to_owned(),
        "ask_user_prompt" => ask_user_prompt(payload),
        "permission_request" => permission_prompt(payload),
        _ => payload
            .get("message")
            .and_then(Value::as_str)
            .map_or_else(|| "Event occurred".to_owned(), ToOwned::to_owned),
    }
}

/// unipi `buildAgentLifecycleMessage`.
pub(crate) fn lifecycle_message(event: &str, session_name: Option<&str>) -> String {
    let status = if event == "agent_end" {
        "Agent run is complete"
    } else {
        "Agent is complete"
    };
    match session_name.filter(|n| !n.is_empty()) {
        Some(n) => format!("{n} - {status}"),
        None => status.to_owned(),
    }
}

/// "All done" text after pending work clears.
pub(crate) fn all_done_message(session_name: Option<&str>) -> String {
    match session_name.filter(|n| !n.is_empty()) {
        Some(n) => format!("{n} - All pending work finished"),
        None => "All pending work finished".to_owned(),
    }
}

/// unipi `extractAssistantText`.
pub(crate) fn assistant_text(message: &Value) -> Option<String> {
    if message.get("role").and_then(Value::as_str) != Some("assistant") {
        return None;
    }
    match message.get("content") {
        Some(Value::String(s)) => Some(s.clone()),
        Some(Value::Array(blocks)) => {
            let parts: Vec<&str> = blocks
                .iter()
                .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|b| b.get("text").and_then(Value::as_str))
                .collect();
            (!parts.is_empty()).then(|| parts.join("\n"))
        }
        _ => None,
    }
}

/// unipi `extractLastAssistantText` over a `messages` payload.
pub(crate) fn last_assistant_in_messages(payload: &Value) -> Option<String> {
    payload
        .get("messages")?
        .as_array()?
        .iter()
        .rev()
        .find_map(assistant_text)
}

/// unipi `extractLastAssistantTextFromSession` over session entries.
pub(crate) fn last_assistant_in_entries(entries: &[Value]) -> Option<String> {
    entries
        .iter()
        .rev()
        .filter(|e| e.get("type").and_then(Value::as_str) == Some("message"))
        .find_map(|e| e.get("message").and_then(assistant_text))
}

/// unipi `filterPlatformsAfterInput` → `(send, silenced)`.
pub(crate) fn filter_after_input(
    platforms: &[String],
    silence: &crate::config::SilenceConfig,
    last_input_at: i64,
    now: i64,
    event: &str,
) -> (Vec<String>, Vec<String>) {
    if is_blocking(event) {
        return (platforms.to_vec(), Vec::new());
    }
    let active = silence.enabled
        && last_input_at > 0
        && now - last_input_at < i64::try_from(silence.window_ms).unwrap_or(i64::MAX);
    if !active {
        return (platforms.to_vec(), Vec::new());
    }
    if silence.platforms.is_empty() {
        return (Vec::new(), platforms.to_vec());
    }
    platforms
        .iter()
        .cloned()
        .partition(|p| !silence.platforms.contains(p))
}

/// Telegram `MarkdownV2` escaping (unipi `escapeMarkdown`).
pub(crate) fn escape_markdown_v2(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if "_*[]()~`>#+-=|{}.!\\".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// unipi `mapNotifyPriority`.
pub(crate) fn map_priority(platform: &str, priority: &str) -> Option<i64> {
    let table: [i64; 3] = match platform {
        "gotify" => [2, 5, 8],
        "ntfy" => [2, 3, 5],
        _ => return None,
    };
    let idx = ["low", "normal", "high"]
        .iter()
        .position(|p| *p == priority)?;
    Some(table[idx])
}

/// Native notification urgency for a priority.
pub(crate) fn urgency(priority: Option<&str>) -> &'static str {
    match priority {
        Some("high") => "critical",
        Some("low") => "low",
        _ => "normal",
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::config::SilenceConfig;

    #[test]
    fn input_needed_like_unipi() {
        assert_eq!(input_needed(&json!({})), "Waiting for your input (prompt)");
        assert_eq!(
            input_needed(&json!({"kind": " select "})),
            "Waiting for your input (select)"
        );
        assert_eq!(
            input_needed(&json!({"title": "  Pick\n a   model "})),
            "Waiting for your input: Pick a model"
        );
        let long = "é".repeat(130);
        let msg = input_needed(&json!({"title": long}));
        assert!(msg.ends_with('…'));
        assert_eq!(msg.chars().count(), "Waiting for your input: ".len() + 121);
        assert_eq!(
            input_needed(&Value::Null),
            "Waiting for your input (prompt)"
        );
    }

    #[test]
    fn ask_user_like_unipi() {
        assert_eq!(
            ask_user_prompt(&json!({"question": "Ship?", "context": "release"})),
            "Agent asks: Ship? — release"
        );
        assert_eq!(
            ask_user_prompt(&json!({"question": "Ship?"})),
            "Agent asks: Ship?"
        );
        assert_eq!(
            ask_user_prompt(&json!({"context": "x"})),
            "Agent asks: A question — x"
        );
        let rpiv = json!({"questions": [
            {"question": "Which db?", "header": "DB", "multiSelect": false, "options": [{"label": "pg"}, {"label": ""}, {"label": "sqlite"}]},
            {"question": "Second"}
        ]});
        assert_eq!(
            ask_user_prompt(&rpiv),
            "Agent asks: Which db? (+1 more) — pg, sqlite"
        );
        assert_eq!(
            ask_user_prompt(&json!({"questions": []})),
            "Agent asks: A question"
        );
        assert_eq!(ask_user_prompt(&json!("junk")), "Agent asks: A question");
    }

    #[test]
    fn permission_like_unipi() {
        assert_eq!(
            permission_prompt(&json!({"surface": "bash", "value": "rm -rf x", "agentName": "dev"})),
            "dev requested bash 'rm -rf x'."
        );
        assert_eq!(
            permission_prompt(&json!({"surface": "mcp"})),
            "Agent requested mcp access."
        );
        assert_eq!(
            permission_prompt(&json!({"value": "x"})),
            "Agent requested 'x'."
        );
        assert_eq!(
            permission_prompt(&json!({"message": "Allow?", "forwarding": {"from": "child"}})),
            "Allow? (forwarded)"
        );
        assert_eq!(
            permission_prompt(
                &json!({"surface": "bash", "message": "Agent requested bash access."})
            ),
            "Agent requested bash access.",
            "duplicate prompt text is dropped"
        );
        assert_eq!(
            permission_prompt(&json!({"forwarding": false})),
            "Oino is waiting for a permission decision."
        );
        assert_eq!(
            permission_prompt(&json!({"forwarding": ""})),
            "Oino is waiting for a permission decision."
        );
    }

    #[test]
    fn event_messages() {
        assert_eq!(
            event_message(
                "ralph_loop_end",
                &json!({"name": "goal", "status": "stopped"})
            ),
            "Ralph loop \"goal\" stopped"
        );
        assert_eq!(
            event_message("ralph_loop_end", &json!({})),
            "Ralph loop \"unknown\" completed"
        );
        assert_eq!(
            event_message("mcp_server_error", &json!({"name": "gh", "error": "boom"})),
            "Server \"gh\" error: boom"
        );
        assert_eq!(
            event_message("session_shutdown", &json!({})),
            "Session ending"
        );
        assert_eq!(event_message("custom", &json!({"message": "hi"})), "hi");
        assert_eq!(event_message("custom", &json!({})), "Event occurred");
        assert_eq!(
            lifecycle_message("agent_end", Some("s")),
            "s - Agent run is complete"
        );
        assert_eq!(
            lifecycle_message("agent_settled", None),
            "Agent is complete"
        );
        assert_eq!(all_done_message(Some("s")), "s - All pending work finished");
        assert_eq!(title_for("agent_settled"), "Oino — Agent Complete");
        assert_eq!(default_priority("input_needed"), Some("high"));
        assert_eq!(default_priority("agent_end"), Some("low"));
        assert_eq!(default_priority("mcp_server_error"), None);
    }

    #[test]
    fn assistant_text_extraction() {
        let msgs = json!({"messages": [
            {"role": "assistant", "content": [{"type": "text", "text": "first"}]},
            {"role": "user", "content": "q"},
            {"role": "assistant", "content": [{"type": "thinking", "thinking": "x"}, {"type": "text", "text": "a"}, {"type": "text", "text": "b"}]},
        ]});
        assert_eq!(last_assistant_in_messages(&msgs).as_deref(), Some("a\nb"));
        let entries = vec![
            json!({"type": "message", "message": {"role": "assistant", "content": "old"}}),
            json!({"type": "custom", "customType": "x"}),
            json!({"type": "message", "message": {"role": "user", "content": "u"}}),
        ];
        assert_eq!(last_assistant_in_entries(&entries).as_deref(), Some("old"));
        assert_eq!(last_assistant_in_messages(&json!({})), None);
    }

    #[test]
    fn silence_after_input() {
        let cfg = SilenceConfig {
            enabled: true,
            window_ms: 10_000,
            platforms: vec!["native".into()],
        };
        let all = vec!["native".to_owned(), "ntfy".to_owned()];
        assert_eq!(
            filter_after_input(&all, &cfg, 1000, 5000, "agent_end"),
            (vec!["ntfy".to_owned()], vec!["native".to_owned()])
        );
        assert_eq!(
            filter_after_input(&all, &cfg, 1000, 20_000, "agent_end"),
            (all.clone(), Vec::new()),
            "outside the window"
        );
        assert_eq!(
            filter_after_input(&all, &cfg, 1000, 5000, "input_needed"),
            (all.clone(), Vec::new()),
            "blocking events bypass"
        );
        assert_eq!(
            filter_after_input(&all, &cfg, 0, 5000, "x"),
            (all.clone(), Vec::new())
        );
        let every = SilenceConfig {
            platforms: vec![],
            ..cfg.clone()
        };
        assert_eq!(
            filter_after_input(&all, &every, 1000, 5000, "x"),
            (Vec::new(), all.clone())
        );
        let off = SilenceConfig {
            enabled: false,
            ..cfg
        };
        assert_eq!(
            filter_after_input(&all, &off, 1000, 5000, "x"),
            (all, Vec::new())
        );
    }

    #[test]
    fn priorities_and_escaping() {
        assert_eq!(map_priority("gotify", "low"), Some(2));
        assert_eq!(map_priority("gotify", "high"), Some(8));
        assert_eq!(map_priority("ntfy", "normal"), Some(3));
        assert_eq!(map_priority("ntfy", "high"), Some(5));
        assert_eq!(map_priority("native", "high"), None);
        assert_eq!(escape_markdown_v2("a_b*c.d!"), "a\\_b\\*c\\.d\\!");
        assert_eq!(urgency(Some("high")), "critical");
        assert_eq!(urgency(None), "normal");
    }
}
