//! Recap summarisation (unipi `summarize.ts`): one sentence for the push notification.
//!
//! unipi called the provider's HTTP API itself with a key from the model registry; Oino uses
//! `models.complete` (permission `models.complete`), which routes through the provider
//! component with the user's credentials.

use serde_json::{Value, json};

/// unipi `SYSTEM_PROMPT`.
pub(crate) const SYSTEM_PROMPT: &str =
    "Summarize this in one concise sentence for a push notification. Reply with ONLY the summary.";
const MAX_INPUT_CHARS: usize = 2000;
const MAX_TOKENS: u32 = 100;
const FALLBACK_TRUNCATE_CHARS: usize = 100;

/// Input truncated to 2 000 chars (+ `...`).
pub(crate) fn clip_input(text: &str) -> String {
    if text.chars().count() > MAX_INPUT_CHARS {
        format!(
            "{}...",
            text.chars().take(MAX_INPUT_CHARS).collect::<String>()
        )
    } else {
        text.to_owned()
    }
}

/// unipi `fallbackSummary`.
pub(crate) fn fallback(text: &str) -> String {
    let t = text.trim();
    if t.chars().count() <= FALLBACK_TRUNCATE_CHARS {
        t.to_owned()
    } else {
        format!(
            "{}...",
            t.chars().take(FALLBACK_TRUNCATE_CHARS).collect::<String>()
        )
    }
}

/// `CompletionRequest` for the recap (`model` = `provider/id`; empty = session model).
pub(crate) fn request(model: &str, text: &str, disable_thinking: bool) -> Value {
    let mut options = json!({"maxTokens": MAX_TOKENS});
    if disable_thinking {
        options["reasoning"] = json!("off");
    }
    let mut req = json!({
        "context": {
            "systemPrompt": SYSTEM_PROMPT,
            "messages": [{"role": "user", "content": clip_input(text), "timestamp": 0}],
        },
        "options": options,
    });
    if !model.trim().is_empty() {
        req["model"] = json!(model.trim());
    }
    req
}

/// Text of an `AssistantMessage` reply; `None` when empty.
pub(crate) fn reply_text(msg: &Value) -> Option<String> {
    let text = crate::messages::assistant_text(&json!({
        "role": "assistant",
        "content": msg.get("content").cloned().unwrap_or(Value::Null),
    }))?;
    let t = text.trim();
    (!t.is_empty()).then(|| t.to_owned())
}

/// Summarises `text`, falling back to a truncation on any failure.
pub(crate) async fn summarize(model: &str, text: &str, disable_thinking: bool) -> String {
    match oino_extension::models::complete(&request(model, text, disable_thinking)).await {
        Ok(msg) if msg.get("stopReason").and_then(Value::as_str) != Some("error") => {
            reply_text(&msg).unwrap_or_else(|| fallback(&clip_input(text)))
        }
        _ => fallback(text),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncation_like_unipi() {
        assert_eq!(fallback("  short  "), "short");
        let long = "x".repeat(150);
        assert_eq!(fallback(&long), format!("{}...", "x".repeat(100)));
        let input = "y".repeat(2500);
        assert_eq!(clip_input(&input).len(), 2003);
        assert_eq!(clip_input("ok"), "ok");
    }

    #[test]
    fn request_shape() {
        let r = request("openrouter/openai/gpt-oss-20b", "hello", true);
        assert_eq!(r["model"], "openrouter/openai/gpt-oss-20b");
        assert_eq!(r["context"]["systemPrompt"], SYSTEM_PROMPT);
        assert_eq!(r["context"]["messages"][0]["content"], "hello");
        assert_eq!(r["options"]["maxTokens"], 100);
        assert_eq!(r["options"]["reasoning"], "off");
        let session = request("", "hello", false);
        assert!(session.get("model").is_none());
        assert!(session["options"].get("reasoning").is_none());
    }

    #[test]
    fn reply_parsing() {
        assert_eq!(
            reply_text(&json!({"content": [{"type": "text", "text": " Done. "}]})).as_deref(),
            Some("Done.")
        );
        assert_eq!(reply_text(&json!({"content": []})), None);
        assert_eq!(reply_text(&json!({})), None);
    }
}
