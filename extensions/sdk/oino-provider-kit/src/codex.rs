//! `OpenAI` Codex Responses over `ChatGPT` sign-in (pi-ai `api/openai-codex-responses.js`): the
//! shared Responses input/stream code with `instructions`, the `ChatGPT` account header and the
//! Codex error texts. SSE transport only (see the deviations: no WebSocket import in WIT, no
//! zstd request compression).

use base64::Engine as _;
use base64::engine::general_purpose::{STANDARD_NO_PAD, URL_SAFE_NO_PAD};
use oino_ai::transcript::{
    collapse_system_messages, declared_tools, resolve_transcript_tools, system_message_text,
};
use oino_ai::{CacheRetention, JsonObject, MapEntry, Message, ThinkingLevel};
use serde_json::{Value, json};

use crate::constrained::grammar_input_properties;
use crate::emit::Emitter;
use crate::errors::SdkError;
use crate::exchange::{ErrorStyle, Exchange};
use crate::request::{Headers, HttpRequest, ProviderRequest, USER_AGENT};
use crate::responses::{
    Flavor, MessageOptions, Parser, ToolOptions, compat, convert_messages, convert_tools,
    tool_support,
};
use crate::util::{clamp_prompt_cache_key, clamp_thinking_level, compat_bool, mapped_level};

/// Default Codex backend.
pub const DEFAULT_CODEX_BASE_URL: &str = "https://chatgpt.com/backend-api";
const JWT_CLAIM_PATH: &str = "https://api.openai.com/auth";
const DEFAULT_INSTRUCTIONS: &str = "You are a helpful assistant.";

/// Decodes a JWT payload (no verification), like pi's `atob` + `JSON.parse`.
#[must_use]
pub fn decode_jwt(token: &str) -> Option<Value> {
    let mut parts = token.split('.');
    let (_, payload, _) = (parts.next()?, parts.next()?, parts.next()?);
    if parts.next().is_some() {
        return None;
    }
    let trimmed = payload.trim_end_matches('=');
    let bytes = URL_SAFE_NO_PAD
        .decode(trimmed)
        .or_else(|_| STANDARD_NO_PAD.decode(trimmed))
        .ok()?;
    serde_json::from_slice(&bytes).ok()
}

/// pi `extractAccountId`: the `ChatGPT` account id claim of an access token.
///
/// # Errors
/// `Failed to extract accountId from token`.
pub fn extract_account_id(token: &str) -> Result<String, String> {
    decode_jwt(token)
        .and_then(|payload| {
            payload[JWT_CLAIM_PATH]["chatgpt_account_id"]
                .as_str()
                .filter(|id| !id.is_empty())
                .map(str::to_owned)
        })
        .ok_or_else(|| "Failed to extract accountId from token".to_owned())
}

/// pi `resolveCodexUrl`.
#[must_use]
pub fn codex_url(base_url: &str) -> String {
    let raw = if base_url.trim().is_empty() {
        DEFAULT_CODEX_BASE_URL
    } else {
        base_url
    };
    let normalized = raw.trim_end_matches('/');
    if normalized.ends_with("/codex/responses") {
        normalized.to_owned()
    } else if normalized.ends_with("/codex") {
        format!("{normalized}/responses")
    } else {
        format!("{normalized}/codex/responses")
    }
}

/// pi `parseErrorResponse` → the thrown `Error` message.
#[must_use]
pub fn status_error(status: u16, body: &str, now: i64) -> SdkError {
    let mut message = if body.is_empty() {
        "Request failed".to_owned()
    } else {
        body.to_owned()
    };
    let mut friendly = None;
    if let Ok(parsed) = serde_json::from_str::<Value>(body) {
        let err = &parsed["error"];
        if !err.is_null() && err != &json!(false) {
            let code = err["code"]
                .as_str()
                .filter(|c| !c.is_empty())
                .or_else(|| err["type"].as_str())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if code.contains("usage_limit_reached")
                || code.contains("usage_not_included")
                || code.contains("rate_limit_exceeded")
                || status == 429
            {
                let plan = err["plan_type"]
                    .as_str()
                    .filter(|p| !p.is_empty())
                    .map(|p| format!(" ({} plan)", p.to_lowercase()))
                    .unwrap_or_default();
                let when = err["resets_at"]
                    .as_f64()
                    .filter(|r| *r != 0.0)
                    .map(|resets| {
                        #[allow(
                            clippy::cast_precision_loss,
                            clippy::cast_possible_truncation,
                            reason = "minutes until a reset, as JavaScript computes them"
                        )]
                        let minutes =
                            ((resets * 1000.0 - now as f64) / 60_000.0).round().max(0.0) as i64;
                        format!(" Try again in ~{minutes} min.")
                    })
                    .unwrap_or_default();
                friendly = Some(
                    format!("You have hit your ChatGPT usage limit{plan}.{when}")
                        .trim()
                        .to_owned(),
                );
            }
            if let Some(text) = err["message"].as_str().filter(|m| !m.is_empty()) {
                text.clone_into(&mut message);
            } else if let Some(friendly) = &friendly {
                friendly.clone_into(&mut message);
            }
        }
    }
    SdkError {
        status: None,
        message: friendly.unwrap_or(message),
        error: None,
    }
}

/// A prepared Codex request.
pub struct Prepared {
    /// Outbound request.
    pub http: HttpRequest,
    /// Streaming state.
    pub exchange: Exchange,
}

/// Builds the request (pi `streamSimple` + `buildRequestBody` + `buildSSEHeaders`).
///
/// # Errors
/// Missing or malformed token, or a tool that cannot be expressed.
#[allow(clippy::too_many_lines, reason = "1:1 port of pi buildRequestBody")]
pub fn prepare(request: &ProviderRequest, now: i64) -> Result<Prepared, String> {
    let model = &request.model;
    let key = request
        .api_key
        .clone()
        .filter(|k| !k.is_empty())
        .ok_or_else(|| format!("No API key for provider: {}", model.provider))?;
    let account_id = extract_account_id(&key)?;
    let mut compat = compat(model);
    compat.supports_strict_mode = compat_bool(model, "supportsStrictMode").unwrap_or(true);
    let normalized = if compat.supports_mid_convo_system_messages {
        request.context.clone()
    } else {
        collapse_system_messages(request.context.clone())
    };
    let grammar = grammar_input_properties(
        &declared_tools(&normalized.messages),
        compat.supports_openai_grammar_tools,
    )?;
    let options = &request.options;
    let cache_session = options
        .session_id
        .as_deref()
        .filter(|_| options.cache_retention != Some(CacheRetention::None))
        .map(clamp_prompt_cache_key);
    let transcript_tools = resolve_transcript_tools(&normalized.messages, tool_support(&compat));
    let input = convert_messages(
        model,
        &normalized,
        &MessageOptions {
            grammar: &grammar,
            compat: &compat,
            configuration_update_effort: None,
            flavor: Flavor::Codex,
            include_system_prompt: false,
            default_strict: None,
        },
        now,
    )?;
    let instructions = match normalized.messages.first() {
        Some(Message::System(system)) => system_message_text(system),
        _ => String::new(),
    };
    let mut body = JsonObject::new();
    body.insert("model".into(), json!(model.id));
    body.insert("store".into(), json!(false));
    body.insert("stream".into(), json!(true));
    body.insert(
        "instructions".into(),
        json!(if instructions.is_empty() {
            DEFAULT_INSTRUCTIONS.to_owned()
        } else {
            instructions
        }),
    );
    body.insert("input".into(), Value::Array(input));
    body.insert(
        "text".into(),
        json!({"verbosity": request.extra.text_verbosity.clone().filter(|v| !v.is_empty()).unwrap_or_else(|| "low".to_owned())}),
    );
    body.insert("include".into(), json!(["reasoning.encrypted_content"]));
    if let Some(session) = &cache_session {
        body.insert("prompt_cache_key".into(), json!(session));
    }
    body.insert(
        "tool_choice".into(),
        options
            .tool_choice
            .clone()
            .filter(|c| !c.is_null())
            .unwrap_or_else(|| json!("auto")),
    );
    body.insert("parallel_tool_calls".into(), json!(true));
    if let Some(temperature) = options.temperature {
        body.insert("temperature".into(), json!(temperature));
    }
    if !transcript_tools.request_tools.is_empty() {
        body.insert(
            "tools".into(),
            Value::Array(convert_tools(
                &transcript_tools.request_tools,
                ToolOptions {
                    supports_strict_mode: compat.supports_strict_mode,
                    supports_grammar: compat.supports_openai_grammar_tools,
                    tool_search_result: false,
                    default_strict: None,
                },
            )?),
        );
    }
    let effort = options
        .reasoning
        .map(|level| clamp_thinking_level(model, level))
        .filter(|level| *level != ThinkingLevel::Off);
    if let Some(level) = effort {
        let value = match mapped_level(model, level) {
            MapEntry::Value(v) => Some(v.to_owned()),
            MapEntry::Null => None,
            MapEntry::Absent => Some(level.as_str().to_owned()),
        };
        if let Some(value) = value {
            body.insert(
                "reasoning".into(),
                json!({"effort": value, "summary": request.extra.reasoning_summary.clone().unwrap_or_else(|| "auto".to_owned())}),
            );
        }
    } else if model.reasoning && mapped_level(model, ThinkingLevel::Off) != MapEntry::Null {
        let off = match mapped_level(model, ThinkingLevel::Off) {
            MapEntry::Value(v) => v.to_owned(),
            _ => "none".to_owned(),
        };
        body.insert("reasoning".into(), json!({"effort": off}));
    }
    let mut headers = Headers::new();
    headers.set("originator", "pi");
    headers.set("user-agent", USER_AGENT);
    for (name, value) in model.headers.iter().flatten() {
        headers.set(name, value);
    }
    for (name, value) in request.header_overrides() {
        headers.apply(&name, value.as_deref());
    }
    headers.set("authorization", format!("Bearer {key}"));
    headers.set("chatgpt-account-id", account_id);
    headers.set("openai-beta", "responses=experimental");
    headers.set("accept", "text/event-stream");
    headers.set("content-type", "application/json");
    if let Some(session) = &cache_session {
        headers.set("session-id", session);
        headers.set("x-client-request-id", session);
    }
    // pi `streamSimple` → `buildBaseOptions` does not forward `serviceTier`.
    let parser = Parser::new(model, grammar, None, Flavor::Codex);
    Ok(Prepared {
        http: HttpRequest {
            url: codex_url(&model.base_url),
            headers,
            payload: Value::Object(body),
            timeout_ms: options.timeout_ms,
            signing: None,
        },
        exchange: Exchange::new(
            Emitter::new(model, now),
            Box::new(parser),
            ErrorStyle::Codex { now },
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        assert_eq!(
            codex_url(""),
            "https://chatgpt.com/backend-api/codex/responses"
        );
        assert_eq!(codex_url("https://x/codex/"), "https://x/codex/responses");
        assert_eq!(
            codex_url("https://x/codex/responses"),
            "https://x/codex/responses"
        );
    }

    #[test]
    fn account_id() {
        let payload = URL_SAFE_NO_PAD
            .encode(br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"a-1"}}"#);
        assert_eq!(
            extract_account_id(&format!("h.{payload}.s")).unwrap(),
            "a-1"
        );
        assert!(extract_account_id("nope").is_err());
    }
}
