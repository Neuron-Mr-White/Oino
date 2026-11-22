//! Image recognition (unipi `src/image/recognize.ts`).
//!
//! Provider models go through `models.complete` (core builds the provider-specific image part
//! and holds the keys, so unipi's per-API request code is not needed); a custom
//! OpenAI-compatible endpoint from the settings is called directly. Gateways that stream even
//! when asked not to are tolerated (SSE frames are concatenated).

use serde_json::{Value, json};

use super::source::LoadedImage;

/// Vision response time box.
pub const TIMEOUT_MS: u32 = 120_000;
/// Reply cap so a verbose model cannot flood the context.
pub const MAX_TOKENS: u32 = 2_048;

/// `CompletionRequest` for a provider model.
#[must_use]
pub fn completion_request(model: &str, image: &LoadedImage, prompt: &str, system: &str) -> Value {
    json!({
        "model": model,
        "context": {
            "systemPrompt": system,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "image", "data": image.data, "mimeType": image.mime_type},
                    {"type": "text", "text": prompt},
                ],
                "timestamp": 0,
            }],
        },
        "options": {"maxTokens": MAX_TOKENS},
    })
}

/// Text of a completion (`AssistantMessage`).
///
/// # Errors
///
/// The provider's error message, or an empty response.
pub fn completion_text(message: &Value) -> Result<String, String> {
    if message.get("stopReason").and_then(Value::as_str) == Some("error") {
        let m = message
            .get("errorMessage")
            .and_then(Value::as_str)
            .unwrap_or("unknown error");
        return Err(format!("Vision request failed: {m}"));
    }
    let text = crate::answer::extract::message_text(message.get("content").unwrap_or(&Value::Null));
    non_empty(text.trim())
}

fn non_empty(text: &str) -> Result<String, String> {
    if text.is_empty() {
        Err("The model returned an empty response for this image.".into())
    } else {
        Ok(text.to_owned())
    }
}

/// OpenAI-compatible chat request to a custom endpoint: `(url, body)`.
#[must_use]
pub fn openai_request(
    base_url: &str,
    model: &str,
    image: &LoadedImage,
    prompt: &str,
    system: &str,
) -> (String, Value) {
    (
        format!("{}/chat/completions", base_url.trim_end_matches('/')),
        json!({
            "model": model,
            "max_tokens": MAX_TOKENS,
            "stream": false,
            "messages": [
                {"role": "system", "content": system},
                {"role": "user", "content": [
                    {"type": "text", "text": prompt},
                    {"type": "image_url", "image_url": {"url": format!("data:{};base64,{}", image.mime_type, image.data)}},
                ]},
            ],
        }),
    )
}

/// Provider error (`Vision request failed: <status>` + detail).
#[must_use]
pub fn describe_http_error(status: u16, body: &[u8]) -> String {
    let text = String::from_utf8_lossy(body);
    let detail = match serde_json::from_str::<Value>(&text) {
        Ok(v) => v
            .get("error")
            .and_then(|e| {
                e.as_str()
                    .map(ToOwned::to_owned)
                    .or_else(|| e.get("message")?.as_str().map(ToOwned::to_owned))
            })
            .unwrap_or_else(|| text.chars().take(300).collect()),
        Err(_) => text.chars().take(300).collect(),
    };
    let base = format!("Vision request failed: {status}");
    if detail.is_empty() {
        base
    } else {
        format!("{base}\n{detail}")
    }
}

/// Text pieces of one SSE frame (OpenAI `choices[].delta.content` / `message.content`,
/// Anthropic `delta.text`).
#[must_use]
pub fn delta_text(frame: &Value) -> Vec<String> {
    let mut out = Vec::new();
    for choice in frame
        .get("choices")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        if let Some(s) = choice.pointer("/delta/content").and_then(Value::as_str) {
            out.push(s.to_owned());
        }
        if let Some(s) = choice.pointer("/message/content").and_then(Value::as_str) {
            out.push(s.to_owned());
        }
    }
    if let Some(s) = frame.pointer("/delta/text").and_then(Value::as_str) {
        out.push(s.to_owned());
    }
    out
}

/// Body as JSON, or the concatenated deltas of an SSE stream (`Err(text)` when neither).
///
/// # Errors
///
/// A readable message for unparseable non-SSE bodies.
pub fn read_json_or_stream(body: &str) -> Result<Result<Value, String>, String> {
    if !body.trim_start().starts_with("data:") {
        return serde_json::from_str(body).map(Ok).map_err(|_| {
            format!(
                "The model returned a response that could not be parsed:\n{}",
                body.chars().take(200).collect::<String>()
            )
        });
    }
    let mut parts = Vec::new();
    for line in body.lines() {
        let Some(payload) = line.strip_prefix("data:").map(str::trim) else {
            continue;
        };
        if payload.is_empty() || payload == "[DONE]" {
            continue;
        }
        if let Ok(frame) = serde_json::from_str::<Value>(payload) {
            parts.extend(delta_text(&frame));
        }
    }
    Ok(Err(parts.concat()))
}

/// Text of an OpenAI chat response body.
///
/// # Errors
///
/// HTTP errors, unparseable bodies, empty replies.
pub fn openai_text(status: u16, body: &[u8]) -> Result<String, String> {
    if !(200..300).contains(&status) {
        return Err(describe_http_error(status, body));
    }
    match read_json_or_stream(&String::from_utf8_lossy(body))? {
        Err(streamed) => non_empty(streamed.trim()),
        Ok(v) => {
            let content = v.pointer("/choices/0/message/content");
            let text = match content {
                Some(Value::String(s)) => s.trim().to_owned(),
                Some(Value::Array(parts)) => parts
                    .iter()
                    .filter_map(|p| p.get("text").and_then(Value::as_str))
                    .collect::<String>()
                    .trim()
                    .to_owned(),
                _ => String::new(),
            };
            non_empty(&text)
        }
    }
}

/// Calls a custom endpoint.
///
/// # Errors
///
/// A message worth showing the agent.
pub async fn via_endpoint(
    base_url: &str,
    api_key: &str,
    model: &str,
    image: &LoadedImage,
    prompt: &str,
    system: &str,
) -> Result<String, String> {
    let (url, body) = openai_request(base_url, model, image, prompt, system);
    let mut req = oino_extension::http::post(&url)
        .timeout_ms(TIMEOUT_MS)
        .header("content-type", "application/json");
    if !api_key.is_empty() {
        req = req.header("authorization", &format!("Bearer {api_key}"));
    }
    let resp = req
        .body(body.to_string())
        .send()
        .await
        .map_err(|e| format!("Vision request failed: {e}"))?;
    let status = resp.status();
    openai_text(status, &resp.bytes().await)
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
    use crate::image::source::Source;

    fn img() -> LoadedImage {
        LoadedImage {
            data: "AAAA".into(),
            mime_type: "image/png".into(),
            source: Source::Base64,
            path: None,
        }
    }

    #[test]
    fn returns_the_models_answer_and_joins_arrays() {
        assert_eq!(
            openai_text(200, br#"{"choices":[{"message":{"content":" A cat. "}}]}"#).unwrap(),
            "A cat."
        );
        assert_eq!(
            openai_text(
                200,
                br#"{"choices":[{"message":{"content":[{"text":"a"},{"text":"b"}]}}]}"#
            )
            .unwrap(),
            "ab"
        );
    }

    #[test]
    fn sends_a_data_url_without_streaming_and_no_double_slash() {
        let (url, body) = openai_request("https://gw/v1/", "m", &img(), "what?", "sys");
        assert_eq!(url, "https://gw/v1/chat/completions");
        assert_eq!(body["stream"], false);
        assert_eq!(body["max_tokens"], MAX_TOKENS);
        assert_eq!(body["messages"][0]["content"], "sys");
        assert_eq!(
            body["messages"][1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,AAAA"
        );
    }

    #[test]
    fn surfaces_errors() {
        assert_eq!(
            openai_text(401, br#"{"error":{"message":"bad key"}}"#).unwrap_err(),
            "Vision request failed: 401\nbad key"
        );
        assert_eq!(
            openai_text(500, b"").unwrap_err(),
            "Vision request failed: 500"
        );
        assert!(
            openai_text(200, br#"{"choices":[{"message":{"content":""}}]}"#)
                .unwrap_err()
                .contains("empty response")
        );
        assert!(
            openai_text(200, b"<html>nope</html>")
                .unwrap_err()
                .contains("could not be parsed")
        );
    }

    #[test]
    fn streams_are_tolerated() {
        let openai = "data: {\"choices\":[{\"delta\":{\"content\":\"A \"}}]}\n\ndata: {\"choices\":[{\"delta\":{\"content\":\"cat\"}}]}\n\ndata: [DONE]\n";
        assert_eq!(openai_text(200, openai.as_bytes()).unwrap(), "A cat");
        let anthropic = "data: {\"type\":\"content_block_delta\",\"delta\":{\"text\":\"Hello\"}}\ndata: {\"delta\":{\"text\":\" there\"}}\n";
        assert_eq!(
            openai_text(200, anthropic.as_bytes()).unwrap(),
            "Hello there"
        );
        let broken = "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\ndata: {not json\n";
        assert_eq!(openai_text(200, broken.as_bytes()).unwrap(), "ok");
    }

    #[test]
    fn completion_shapes() {
        let r = completion_request("p/m", &img(), "what?", "sys");
        assert_eq!(r["model"], "p/m");
        assert_eq!(r["context"]["messages"][0]["content"][0]["type"], "image");
        assert_eq!(
            completion_text(&json!({"content": [{"type": "text", "text": "seen"}]})).unwrap(),
            "seen"
        );
        assert!(
            completion_text(&json!({"stopReason": "error", "errorMessage": "nope"}))
                .unwrap_err()
                .contains("nope")
        );
        assert!(
            completion_text(&json!({"content": []}))
                .unwrap_err()
                .contains("empty")
        );
    }
}
