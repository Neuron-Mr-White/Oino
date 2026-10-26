//! Google Generative AI and Google Vertex (pi-ai `api/google-shared.js`,
//! `api/google-generative-ai.js`, `api/google-vertex.js`, plus the `@google/genai` SDK's
//! request conversion and error texts).
//!
//! The payload handed to the `before-request` hook is pi's `onPayload` shape
//! (`{model, contents, config}`); [`finalize`] turns it into the `streamGenerateContent` body
//! the SDK sends (`systemInstruction` content, `generationConfig`).

use oino_ai::transcript::{collapse_system_messages, current_tools, system_message_text};
use oino_ai::transform::transform_messages;
use oino_ai::{
    AssistantContent, AssistantMessage, JsonObject, MapEntry, Message, Model, ProviderEvent,
    StopReason, TextContent, ThinkingContent, ThinkingLevel, ToolCall, ToolDecl, UserContent,
    UserContentField,
};
use serde_json::{Value, json};

use crate::constrained::{json_schema_tool_parameters, resolve_json_schema_strict};
use crate::emit::Emitter;
use crate::errors::SdkError;
use crate::exchange::{ErrorStyle, Exchange, Step, StreamError, StreamParser};
use crate::options::clamp_max_tokens_to_context;
use crate::request::{Headers, HttpRequest, ProviderRequest, USER_AGENT};
use crate::sse::SseEvent;
use crate::util::{calculate_cost, clamp_thinking_level, mapped_level};

const GENERATIVE_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const VERTEX_GLOBAL_HOST: &str = "https://aiplatform.googleapis.com";
const VERTEX_API_VERSION: &str = "v1";
/// pi's marker for "use Application Default Credentials".
pub const VERTEX_CREDENTIALS_MARKER: &str = "gcp-vertex-credentials";

/// Which Google API.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GoogleApi {
    /// `google-generative-ai` (Gemini API key).
    Generative,
    /// `google-vertex` (API key or ADC).
    Vertex,
}

impl GoogleApi {
    const fn pending_message(self) -> &'static str {
        match self {
            Self::Generative => "Google stream ended without a finish reason",
            Self::Vertex => "Google Vertex stream ended without a finish reason",
        }
    }
}

/// How a Vertex request authenticates.
#[derive(Clone, Debug, PartialEq, Eq)]
enum VertexAuth {
    ApiKey(String),
    Adc {
        project: String,
        location: String,
        token: Option<String>,
    },
}

/// pi `resolveGoogleThinkingLevel`: the standard Google level for a pi level.
fn resolve_thinking_level(model: &Model, level: ThinkingLevel) -> Result<&'static str, String> {
    let mapped = mapped_level(model, level);
    let resolved = match mapped {
        MapEntry::Value(value) => value.to_ascii_lowercase(),
        _ => level.as_str().to_owned(),
    };
    match resolved.as_str() {
        "minimal" => Ok("MINIMAL"),
        "low" => Ok("LOW"),
        "medium" => Ok("MEDIUM"),
        "high" => Ok("HIGH"),
        _ => Err(format!(
            "Unsupported Google thinking level mapping for {}/{}: {} -> {}",
            model.provider,
            model.id,
            level.as_str(),
            match mapped {
                MapEntry::Value(v) => v.to_owned(),
                MapEntry::Null => "null".to_owned(),
                MapEntry::Absent => "undefined".to_owned(),
            }
        )),
    }
}

/// pi `usesGoogleThinkingLevel`.
#[must_use]
pub fn uses_thinking_level(model: &Model) -> bool {
    let id = model.id.to_ascii_lowercase();
    let after = |prefix: &str| -> Vec<String> {
        id.match_indices(prefix)
            .map(|(i, _)| id[i + prefix.len()..].to_owned())
            .collect()
    };
    let gemini3 = after("gemini-3").iter().any(|rest| {
        let rest = rest.strip_prefix('.').map_or(rest.as_str(), |r| {
            r.trim_start_matches(|c: char| c.is_ascii_digit())
        });
        rest.starts_with("-pro") || rest.starts_with("-flash")
    });
    let gemini4plus = after("gemini-").iter().any(|rest| {
        let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
        match digits.len() {
            1 => digits.as_bytes()[0] >= b'4',
            0 => false,
            _ => !digits.starts_with('0'),
        }
    });
    gemini3
        || gemini4plus
        || id == "gemini-flash-latest"
        || id == "gemini-flash-lite-latest"
        || id.contains("gemma-4")
        || id.contains("gemma4")
}

/// pi `getGeminiMajorVersion`.
fn gemini_major(id: &str) -> Option<u32> {
    let lower = id.to_ascii_lowercase();
    let rest = lower.strip_prefix("gemini-")?;
    let rest = rest.strip_prefix("live-").unwrap_or(rest);
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// pi `requiresToolCallId`.
fn requires_tool_call_id(id: &str) -> bool {
    id.starts_with("claude-")
        || id.starts_with("gpt-oss-")
        || gemini_major(id).is_some_and(|v| v >= 3)
}

/// pi `supportsMultimodalFunctionResponse`.
fn multimodal_function_response(id: &str) -> bool {
    gemini_major(id).is_none_or(|v| v >= 3)
}

/// pi `supportsGoogleStrictToolSampling`.
fn strict_tool_sampling(id: &str) -> bool {
    gemini_major(id).is_some_and(|v| v >= 3)
}

fn normalize_tool_call_id(model: &Model, id: &str) -> String {
    if !requires_tool_call_id(&model.id) {
        return id.to_owned();
    }
    id.encode_utf16()
        .map(|unit| match char::from_u32(u32::from(unit)) {
            Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            _ => '_',
        })
        .take(64)
        .collect()
}

/// Valid base64 thought signature (`TYPE_BYTES`).
fn valid_signature(signature: Option<&str>) -> Option<&str> {
    let signature = signature.filter(|s| !s.is_empty())?;
    if signature.len() % 4 != 0 {
        return None;
    }
    let body = signature.trim_end_matches('=');
    let padding = signature.len() - body.len();
    (padding <= 2
        && !body.is_empty()
        && body
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'+' || b == b'/'))
    .then_some(signature)
}

fn inline_data(mime_type: &str, data: &str) -> Value {
    json!({"inlineData": {"mimeType": mime_type, "data": data}})
}

/// pi `convertMessages` (google-shared).
#[allow(clippy::too_many_lines, reason = "1:1 port of pi convertMessages")]
fn convert_messages(model: &Model, collapsed: &[Message], now: i64) -> Vec<Value> {
    let conversation = match collapsed.first() {
        Some(Message::System(_)) => &collapsed[1..],
        _ => collapsed,
    };
    let transformed = transform_messages(
        conversation.to_vec(),
        model,
        Some(&|id: &str, _: &Model, _: &AssistantMessage| normalize_tool_call_id(model, id)),
        now,
    );
    let include_id = requires_tool_call_id(&model.id);
    let mut contents: Vec<Value> = Vec::new();
    for message in &transformed {
        match message {
            Message::User(user) => match &user.content {
                UserContentField::Text(text) => {
                    contents.push(json!({"role": "user", "parts": [{"text": text}]}));
                }
                UserContentField::Blocks(blocks) => {
                    let parts: Vec<Value> = blocks
                        .iter()
                        .map(|block| match block {
                            UserContent::Text(t) => json!({"text": t.text}),
                            UserContent::Image(i) => inline_data(&i.mime_type, &i.data),
                        })
                        .collect();
                    if !parts.is_empty() {
                        contents.push(json!({"role": "user", "parts": parts}));
                    }
                }
            },
            Message::Assistant(assistant) => {
                let same = assistant.provider == model.provider && assistant.model == model.id;
                let signature = |s: Option<&'_ String>| -> Option<String> {
                    if same {
                        valid_signature(s.map(String::as_str)).map(str::to_owned)
                    } else {
                        None
                    }
                };
                let mut parts = Vec::new();
                for block in &assistant.content {
                    match block {
                        AssistantContent::Text(text) => {
                            let sig = signature(text.text_signature.as_ref());
                            if text.text.trim().is_empty() && sig.is_none() {
                                continue;
                            }
                            let mut part = json!({"text": text.text});
                            if let Some(sig) = sig {
                                part["thoughtSignature"] = json!(sig);
                            }
                            parts.push(part);
                        }
                        AssistantContent::Thinking(thinking) => {
                            if same {
                                let sig = signature(thinking.thinking_signature.as_ref());
                                if thinking.thinking.trim().is_empty() && sig.is_none() {
                                    continue;
                                }
                                let mut part = json!({"thought": true, "text": thinking.thinking});
                                if let Some(sig) = sig {
                                    part["thoughtSignature"] = json!(sig);
                                }
                                parts.push(part);
                            } else if !thinking.thinking.trim().is_empty() {
                                parts.push(json!({"text": thinking.thinking}));
                            }
                        }
                        AssistantContent::ToolCall(call) => {
                            let mut function_call =
                                json!({"name": call.name, "args": call.arguments});
                            if include_id {
                                function_call["id"] = json!(call.id);
                            }
                            let mut part = json!({"functionCall": function_call});
                            if let Some(sig) = signature(call.thought_signature.as_ref()) {
                                part["thoughtSignature"] = json!(sig);
                            }
                            parts.push(part);
                        }
                    }
                }
                if !parts.is_empty() {
                    contents.push(json!({"role": "model", "parts": parts}));
                }
            }
            Message::ToolResult(result) => {
                let text = result
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        UserContent::Text(t) => Some(t.text.as_str()),
                        UserContent::Image(_) => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let images: Vec<Value> = if model.input.contains(&oino_ai::InputModality::Image) {
                    result
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            UserContent::Image(i) => Some(inline_data(&i.mime_type, &i.data)),
                            UserContent::Text(_) => None,
                        })
                        .collect()
                } else {
                    Vec::new()
                };
                let has_text = !text.trim().is_empty();
                let multimodal = multimodal_function_response(&model.id);
                let value = if has_text {
                    text
                } else if images.is_empty() {
                    String::new()
                } else {
                    "(see attached image)".to_owned()
                };
                let mut response = json!({
                    "name": result.tool_name,
                    "response": if result.is_error { json!({"error": value}) } else { json!({"output": value}) },
                });
                if !images.is_empty() && multimodal {
                    response["parts"] = json!(images);
                }
                if include_id {
                    response["id"] = json!(result.tool_call_id);
                }
                let part = json!({"functionResponse": response});
                let merge = contents.last().is_some_and(|last| {
                    last["role"] == "user"
                        && last["parts"]
                            .as_array()
                            .is_some_and(|p| p.iter().any(|p| p.get("functionResponse").is_some()))
                });
                if merge {
                    if let Some(parts) = contents
                        .last_mut()
                        .and_then(|last| last["parts"].as_array_mut())
                    {
                        parts.push(part);
                    }
                } else {
                    contents.push(json!({"role": "user", "parts": [part]}));
                }
                if !images.is_empty() && !multimodal {
                    let mut parts = vec![json!({"text": "Tool result image:"})];
                    parts.extend(images);
                    contents.push(json!({"role": "user", "parts": parts}));
                }
            }
            Message::System(_) => {}
        }
    }
    contents
}

/// pi `convertTools(tools, false, supportsStrictMode)`.
fn convert_tools(tools: &[ToolDecl], strict_supported: bool) -> Result<Value, String> {
    let declarations = tools
        .iter()
        .map(|tool| {
            let strict = resolve_json_schema_strict(tool, strict_supported, None)? == Some(true);
            Ok(json!({
                "name": tool.name,
                "description": tool.description,
                "parametersJsonSchema": json_schema_tool_parameters(tool, strict)?,
            }))
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(json!([{"functionDeclarations": declarations}]))
}

/// pi `resolveGoogleFunctionCallingMode`.
fn function_calling_mode(
    tools: &[ToolDecl],
    choice: Option<&Value>,
    strict_supported: bool,
) -> Result<Option<&'static str>, String> {
    let mut strict = false;
    for tool in tools {
        strict |= resolve_json_schema_strict(tool, strict_supported, None)? == Some(true);
    }
    let map = |c: &str| match c {
        "none" => "NONE",
        "any" => "ANY",
        _ => "AUTO",
    };
    Ok(match choice {
        Some(Value::String(c)) if c == "none" || c == "any" => Some(map(c)),
        _ if strict => Some("VALIDATED"),
        Some(Value::String(c)) if !c.is_empty() => Some(map(c)),
        Some(v) if !v.is_null() && v.as_str() != Some("") => Some("AUTO"),
        _ => None,
    })
}

/// pi `getGoogleBudget`.
fn google_budget(model: &Model, api: GoogleApi, level: &str, request: &ProviderRequest) -> i64 {
    let custom = request
        .options
        .thinking_budgets
        .as_ref()
        .and_then(|b| match level {
            "minimal" => b.minimal,
            "low" => b.low,
            "medium" => b.medium,
            "high" => b.high,
            _ => None,
        });
    if let Some(budget) = custom {
        return i64::try_from(budget).unwrap_or(i64::MAX);
    }
    let table: Option<[i64; 4]> = if model.id.contains("2.5-pro") {
        Some([128, 2048, 8192, 32_768])
    } else if api == GoogleApi::Generative && model.id.contains("2.5-flash-lite") {
        Some([512, 2048, 8192, 24_576])
    } else if model.id.contains("2.5-flash") {
        Some([128, 2048, 8192, 24_576])
    } else {
        None
    };
    let index = match level {
        "minimal" => 0,
        "low" => 1,
        "medium" => 2,
        _ => 3,
    };
    table.map_or(-1, |t| t[index])
}

/// pi `getDisabledGoogleThinkingConfig`.
fn disabled_thinking_config(model: &Model) -> Result<Value, String> {
    if !uses_thinking_level(model) {
        return Ok(json!({"thinkingBudget": 0}));
    }
    let fallback = clamp_thinking_level(model, ThinkingLevel::Off);
    if fallback == ThinkingLevel::Off {
        return Ok(json!({"thinkingBudget": 0}));
    }
    Ok(json!({"thinkingLevel": resolve_thinking_level(model, fallback)?}))
}

/// pi `streamSimple` thinking option.
enum Thinking {
    Disabled,
    Level(&'static str),
    Budget(i64),
}

fn thinking_option(request: &ProviderRequest, api: GoogleApi) -> Result<Thinking, String> {
    let model = &request.model;
    let Some(level) = request.options.reasoning else {
        return Ok(Thinking::Disabled);
    };
    let clamped = clamp_thinking_level(model, level);
    if clamped == ThinkingLevel::Off {
        return Ok(Thinking::Disabled);
    }
    let resolved = resolve_thinking_level(model, clamped)?;
    if uses_thinking_level(model) {
        return Ok(Thinking::Level(resolved));
    }
    Ok(Thinking::Budget(google_budget(
        model,
        api,
        &resolved.to_ascii_lowercase(),
        request,
    )))
}

/// pi `buildParams` (`onPayload` shape).
fn build_params(request: &ProviderRequest, api: GoogleApi, now: i64) -> Result<Value, String> {
    let model = &request.model;
    let thinking = thinking_option(request, api)?;
    let collapsed = collapse_system_messages(request.context.clone()).messages;
    let contents = convert_messages(model, &collapsed, now);
    let tools = current_tools(&collapsed);
    let strict = strict_tool_sampling(&model.id);
    let mut config = JsonObject::new();
    if let Some(temperature) = request.options.temperature {
        config.insert("temperature".into(), json!(temperature));
    }
    let max_tokens = clamp_max_tokens_to_context(
        model,
        &request.context.messages,
        request.options.max_tokens.unwrap_or(model.max_tokens),
    );
    config.insert("maxOutputTokens".into(), json!(max_tokens));
    let system = match collapsed.first() {
        Some(Message::System(system)) => system_message_text(system),
        _ => String::new(),
    };
    if !system.is_empty() {
        config.insert("systemInstruction".into(), json!(system));
    }
    if !tools.is_empty() {
        config.insert("tools".into(), convert_tools(&tools, strict)?);
        if let Some(mode) =
            function_calling_mode(&tools, request.options.tool_choice.as_ref(), strict)?
        {
            config.insert(
                "toolConfig".into(),
                json!({"functionCallingConfig": {"mode": mode}}),
            );
        }
    }
    match thinking {
        Thinking::Level(level) if model.reasoning => {
            config.insert(
                "thinkingConfig".into(),
                json!({"includeThoughts": true, "thinkingLevel": level}),
            );
        }
        Thinking::Budget(budget) if model.reasoning => {
            config.insert(
                "thinkingConfig".into(),
                json!({"includeThoughts": true, "thinkingBudget": budget}),
            );
        }
        Thinking::Disabled if model.reasoning => {
            config.insert("thinkingConfig".into(), disabled_thinking_config(model)?);
        }
        _ => {}
    }
    Ok(json!({"model": model.id, "contents": contents, "config": config}))
}

/// `generateContent` config fields the SDK sends at the top level of the body.
const TOP_LEVEL_CONFIG: [&str; 6] = [
    "systemInstruction",
    "tools",
    "toolConfig",
    "safetySettings",
    "cachedContent",
    "labels",
];

/// The SDK's `generateContentParameters` conversion: `{model, contents, config}` → wire body.
#[must_use]
pub fn finalize(payload: Value) -> Value {
    let Value::Object(mut payload) = payload else {
        return payload;
    };
    let mut body = JsonObject::new();
    body.insert(
        "contents".into(),
        payload
            .shift_remove("contents")
            .unwrap_or_else(|| json!([])),
    );
    let mut generation = JsonObject::new();
    if let Some(Value::Object(config)) = payload.shift_remove("config") {
        for (key, value) in config {
            if key == "abortSignal" || key == "httpOptions" || value.is_null() {
                continue;
            }
            if key == "systemInstruction" {
                let content = match value {
                    Value::String(text) => json!({"parts": [{"text": text}], "role": "user"}),
                    other => other,
                };
                body.insert(key, content);
            } else if TOP_LEVEL_CONFIG.contains(&key.as_str()) {
                body.insert(key, value);
            } else {
                generation.insert(key, value);
            }
        }
    }
    if !generation.is_empty() {
        body.insert("generationConfig".into(), Value::Object(generation));
    }
    Value::Object(body)
}

fn vertex_auth(request: &ProviderRequest) -> Result<VertexAuth, String> {
    let marker_values: Vec<&str> = ["GOOGLE_CLOUD_PROJECT", "GCLOUD_PROJECT"]
        .iter()
        .filter_map(|n| request.env_value(n))
        .collect();
    let key = request.api_key.as_deref().map(str::trim).filter(|k| {
        let placeholder = k.starts_with('<') && k.ends_with('>') && k.len() > 2;
        !k.is_empty()
            && *k != VERTEX_CREDENTIALS_MARKER
            && !placeholder
            && !marker_values.contains(k)
    });
    if let Some(key) = key {
        return Ok(VertexAuth::ApiKey(key.to_owned()));
    }
    let project = request
        .extra
        .project
        .clone()
        .filter(|p| !p.is_empty())
        .or_else(|| request.env_value("GOOGLE_CLOUD_PROJECT").map(str::to_owned))
        .or_else(|| request.env_value("GCLOUD_PROJECT").map(str::to_owned))
        .ok_or_else(|| {
            "Vertex AI requires a project ID. Set GOOGLE_CLOUD_PROJECT/GCLOUD_PROJECT or pass project in options.".to_owned()
        })?;
    let location = request
        .extra
        .location
        .clone()
        .filter(|l| !l.is_empty())
        .or_else(|| {
            request
                .env_value("GOOGLE_CLOUD_LOCATION")
                .map(str::to_owned)
        })
        .ok_or_else(|| {
            "Vertex AI requires a location. Set GOOGLE_CLOUD_LOCATION or pass location in options."
                .to_owned()
        })?;
    Ok(VertexAuth::Adc {
        project,
        location,
        token: request.extra.vertex_access_token.clone(),
    })
}

/// Whether a Vertex request will use Application Default Credentials (the component then
/// resolves an access token before [`prepare`]).
#[must_use]
pub fn vertex_needs_adc(request: &ProviderRequest) -> bool {
    matches!(vertex_auth(request), Ok(VertexAuth::Adc { .. }))
}

fn includes_api_version(base: &str) -> bool {
    base.split(['/', '?', '#']).any(|part| {
        part.strip_prefix('v').is_some_and(|rest| {
            let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
            let tail = &rest[digits.len()..];
            !digits.is_empty()
                && (tail.is_empty()
                    || tail
                        .strip_prefix("beta")
                        .is_some_and(|t| t.chars().all(|c| c.is_ascii_digit())))
        })
    })
}

fn stream_url(request: &ProviderRequest, api: GoogleApi, auth: Option<&VertexAuth>) -> String {
    let model = &request.model;
    let action = format!("models/{}:streamGenerateContent?alt=sse", model.id);
    match (api, auth) {
        (GoogleApi::Generative, _) | (GoogleApi::Vertex, None) => {
            let base = if model.base_url.is_empty() {
                GENERATIVE_BASE_URL
            } else {
                model.base_url.as_str()
            };
            format!("{}/{action}", base.trim_end_matches('/'))
        }
        (GoogleApi::Vertex, Some(auth)) => {
            let custom = model.base_url.trim();
            let custom = (!custom.is_empty() && !custom.contains("{location}")).then_some(custom);
            if let Some(base) = custom {
                let version = if includes_api_version(base) {
                    String::new()
                } else {
                    format!("/{VERTEX_API_VERSION}")
                };
                return format!(
                    "{}{version}/publishers/google/{action}",
                    base.trim_end_matches('/')
                );
            }
            match auth {
                VertexAuth::ApiKey(_) => {
                    format!("{VERTEX_GLOBAL_HOST}/{VERTEX_API_VERSION}/publishers/google/{action}")
                }
                VertexAuth::Adc {
                    project, location, ..
                } => {
                    let host = if location == "global" {
                        VERTEX_GLOBAL_HOST.to_owned()
                    } else {
                        format!("https://{location}-aiplatform.googleapis.com")
                    };
                    format!(
                        "{host}/{VERTEX_API_VERSION}/projects/{project}/locations/{location}/publishers/google/{action}"
                    )
                }
            }
        }
    }
}

/// A prepared Google request.
pub struct Prepared {
    /// Outbound request (`onPayload` shape payload).
    pub http: HttpRequest,
    /// Streaming state.
    pub exchange: Exchange,
}

/// Builds the request.
///
/// # Errors
/// Missing credentials or project/location, an invalid thinking mapping, or a tool that
/// cannot be expressed.
pub fn prepare(request: &ProviderRequest, api: GoogleApi, now: i64) -> Result<Prepared, String> {
    let model = &request.model;
    let auth = match api {
        GoogleApi::Generative => {
            let key = request
                .api_key
                .clone()
                .filter(|k| !k.is_empty())
                .ok_or_else(|| format!("No API key for provider: {}", model.provider))?;
            VertexAuth::ApiKey(key)
        }
        GoogleApi::Vertex => vertex_auth(request)?,
    };
    let params = build_params(request, api, now)?;
    let mut headers = Headers::new();
    headers.set("content-type", "application/json");
    headers.set("user-agent", USER_AGENT);
    for (name, value) in model.headers.iter().flatten() {
        headers.set(name, value);
    }
    for (name, value) in request.header_overrides() {
        headers.apply(&name, value.as_deref());
    }
    match &auth {
        VertexAuth::ApiKey(key) => {
            if !headers
                .pairs()
                .iter()
                .any(|(k, _)| k.eq_ignore_ascii_case("x-goog-api-key"))
            {
                headers.set("x-goog-api-key", key);
            }
        }
        VertexAuth::Adc { token, .. } => {
            let token = token.as_deref().ok_or_else(|| {
                "Vertex AI Application Default Credentials are unavailable: run `gcloud auth application-default login` or set GOOGLE_APPLICATION_CREDENTIALS".to_owned()
            })?;
            if headers.get("authorization").is_none() {
                headers.set("authorization", format!("Bearer {token}"));
            }
        }
    }
    let url = stream_url(request, api, (api == GoogleApi::Vertex).then_some(&auth));
    let parser = Parser {
        model: model.clone(),
        api,
        now,
        current: None,
        tool_counter: 0,
    };
    Ok(Prepared {
        http: HttpRequest {
            url,
            headers,
            payload: params,
            timeout_ms: request.options.timeout_ms,
            signing: None,
        },
        exchange: Exchange::new(
            Emitter::new(model, now),
            Box::new(parser),
            ErrorStyle::Google,
        ),
    })
}

/// HTTP reason phrases the SDK copies into synthesized error bodies (`response.statusText`).
fn reason_phrase(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        409 => "Conflict",
        413 => "Payload Too Large",
        429 => "Too Many Requests",
        500 => "Internal Server Error",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "",
    }
}

/// `@google/genai` `throwErrorIfNotOK`: `ApiError(JSON.stringify(body))`.
#[must_use]
pub fn status_error(status: u16, headers: &[(String, String)], body: &str) -> SdkError {
    let is_json = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-type"))
        .is_none_or(|(_, v)| v.contains("application/json"));
    let parsed: Option<Value> = if is_json {
        serde_json::from_str(body).ok()
    } else {
        None
    };
    let body_json = parsed.unwrap_or_else(
        || json!({"error": {"message": body, "code": status, "status": reason_phrase(status)}}),
    );
    SdkError {
        status: Some(status),
        message: body_json.to_string(),
        error: None,
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum BlockKind {
    Text,
    Thinking,
}

struct Parser {
    model: Model,
    api: GoogleApi,
    now: i64,
    current: Option<(BlockKind, usize)>,
    tool_counter: u64,
}

impl Parser {
    fn close_current(&mut self, out: &mut Emitter) {
        let Some((kind, index)) = self.current.take() else {
            return;
        };
        let event = match (kind, out.block(index)) {
            (BlockKind::Text, Some(AssistantContent::Text(t))) => ProviderEvent::TextEnd {
                content_index: index,
                content: t.text.clone(),
            },
            (BlockKind::Thinking, Some(AssistantContent::Thinking(t))) => {
                ProviderEvent::ThinkingEnd {
                    content_index: index,
                    content: t.thinking.clone(),
                }
            }
            _ => return,
        };
        out.push(event);
    }

    fn text_part(&mut self, part: &Value, text: &str, out: &mut Emitter) {
        let kind = if part["thought"] == true {
            BlockKind::Thinking
        } else {
            BlockKind::Text
        };
        if self.current.is_none_or(|(k, _)| k != kind) {
            self.close_current(out);
            let index = match kind {
                BlockKind::Thinking => {
                    let index = out.add(AssistantContent::Thinking(ThinkingContent::default()));
                    out.push(ProviderEvent::ThinkingStart {
                        content_index: index,
                    });
                    index
                }
                BlockKind::Text => {
                    let index = out.add(AssistantContent::Text(TextContent::new("")));
                    out.push(ProviderEvent::TextStart {
                        content_index: index,
                    });
                    index
                }
            };
            self.current = Some((kind, index));
        }
        let Some((_, index)) = self.current else {
            return;
        };
        let signature = part["thoughtSignature"]
            .as_str()
            .filter(|s| !s.is_empty())
            .map(str::to_owned);
        match out.block(index) {
            Some(AssistantContent::Thinking(block)) => {
                block.thinking.push_str(text);
                if signature.is_some() {
                    block.thinking_signature = signature;
                }
                out.push(ProviderEvent::ThinkingDelta {
                    content_index: index,
                    delta: text.to_owned(),
                });
            }
            Some(AssistantContent::Text(block)) => {
                block.text.push_str(text);
                if signature.is_some() {
                    block.text_signature = signature;
                }
                out.push(ProviderEvent::TextDelta {
                    content_index: index,
                    delta: text.to_owned(),
                });
            }
            _ => {}
        }
    }

    fn function_call(&mut self, part: &Value, out: &mut Emitter) {
        self.close_current(out);
        let call = &part["functionCall"];
        let name = call["name"].as_str().unwrap_or_default().to_owned();
        let provided = call["id"].as_str().filter(|id| !id.is_empty());
        let duplicate = provided.is_some_and(|id| {
            out.message
                .content
                .iter()
                .any(|b| matches!(b, AssistantContent::ToolCall(c) if c.id == id))
        });
        let id = match provided {
            Some(id) if !duplicate => id.to_owned(),
            _ => {
                self.tool_counter += 1;
                format!("{name}_{}_{}", self.now, self.tool_counter)
            }
        };
        let arguments: JsonObject = call["args"].as_object().cloned().unwrap_or_default();
        let tool_call = ToolCall {
            id: id.clone(),
            name: name.clone(),
            arguments: arguments.clone(),
            thought_signature: part["thoughtSignature"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            ..ToolCall::default()
        };
        let index = out.add(AssistantContent::ToolCall(tool_call.clone()));
        out.push(ProviderEvent::ToolcallStart {
            content_index: index,
            id: Some(id),
            tool_name: Some(name),
        });
        out.push(ProviderEvent::ToolcallDelta {
            content_index: index,
            delta: Value::Object(arguments).to_string(),
        });
        out.push(ProviderEvent::ToolcallEnd {
            content_index: index,
            tool_call,
        });
    }
}

/// pi `mapStopReason` over the SDK `FinishReason` enum.
fn map_stop_reason(reason: &str) -> Result<StopReason, StreamError> {
    match reason {
        "STOP" => Ok(StopReason::Stop),
        "MAX_TOKENS" => Ok(StopReason::Length),
        "BLOCKLIST"
        | "PROHIBITED_CONTENT"
        | "SPII"
        | "SAFETY"
        | "IMAGE_SAFETY"
        | "IMAGE_PROHIBITED_CONTENT"
        | "IMAGE_RECITATION"
        | "IMAGE_OTHER"
        | "RECITATION"
        | "FINISH_REASON_UNSPECIFIED"
        | "OTHER"
        | "LANGUAGE"
        | "MALFORMED_FUNCTION_CALL"
        | "UNEXPECTED_TOOL_CALL"
        | "TOO_MANY_TOOL_CALLS"
        | "NO_IMAGE" => Ok(StopReason::Error),
        other => Err(format!("Unhandled stop reason: {other}").into()),
    }
}

impl StreamParser for Parser {
    fn event(&mut self, sse: &SseEvent, out: &mut Emitter) -> Result<Step, StreamError> {
        let data = sse.data.trim();
        if data.is_empty() {
            return Ok(Step::Skip);
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|e| StreamError::from(format!("Failed to parse stream chunk: {e}")))?;
        if let Some(error) = chunk.get("error").filter(|e| e.is_object())
            && error["code"]
                .as_u64()
                .is_some_and(|c| (400..600).contains(&c))
        {
            return Err(StreamError::from(format!(
                "got status: {}. {chunk}",
                error["status"].as_str().unwrap_or("undefined")
            )));
        }
        if out.message.response_id.is_none()
            && let Some(id) = chunk["responseId"].as_str().filter(|id| !id.is_empty())
        {
            out.message.response_id = Some(id.to_owned());
        }
        let candidate = &chunk["candidates"][0];
        for part in candidate["content"]["parts"]
            .as_array()
            .into_iter()
            .flatten()
        {
            if let Some(text) = part["text"].as_str() {
                self.text_part(part, text, out);
            }
            if part["functionCall"].is_object() {
                self.function_call(part, out);
            }
        }
        if let Some(reason) = candidate["finishReason"].as_str() {
            out.message.raw_stop_reason = Some(reason.to_owned());
            out.message.stop_reason = map_stop_reason(reason)?;
            let has_tool = out
                .message
                .content
                .iter()
                .any(|b| matches!(b, AssistantContent::ToolCall(_)));
            if has_tool && out.message.stop_reason == StopReason::Stop {
                out.message.stop_reason = StopReason::ToolUse;
            }
        }
        let usage = &chunk["usageMetadata"];
        if usage.is_object() {
            let count = |key: &str| usage[key].as_u64().unwrap_or(0);
            let u = &mut out.message.usage;
            u.input = count("promptTokenCount").saturating_sub(count("cachedContentTokenCount"));
            u.output = count("candidatesTokenCount") + count("thoughtsTokenCount");
            u.cache_read = count("cachedContentTokenCount");
            u.cache_write = 0;
            u.cache_write_1h = None;
            u.reasoning = Some(count("thoughtsTokenCount"));
            u.total_tokens = count("totalTokenCount");
            u.cost = oino_ai::UsageCost::default();
            calculate_cost(&self.model, &mut out.message.usage);
        }
        Ok(Step::Event(chunk))
    }

    fn finish(&mut self, out: &mut Emitter) -> Result<(), StreamError> {
        self.close_current(out);
        match out.message.stop_reason {
            StopReason::Pending => Err(self.api.pending_message().into()),
            StopReason::Error | StopReason::Aborted => Err(out
                .message
                .raw_stop_reason
                .as_ref()
                .map_or_else(
                    || "An unknown error occurred".to_owned(),
                    |raw| format!("Provider stopped with: {raw}"),
                )
                .into()),
            _ => Ok(()),
        }
    }

    fn cleanup(&mut self, _out: &mut Emitter) {
        self.current = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model(id: &str) -> Model {
        serde_json::from_value(
            json!({"id": id, "name": id, "api": "google-generative-ai", "provider": "google"}),
        )
        .unwrap()
    }

    #[test]
    fn thinking_level_models() {
        for id in [
            "gemini-3-flash-preview",
            "gemini-3.1-pro-preview",
            "gemini-4-argon",
            "gemini-flash-latest",
            "gemma-4-31b-it",
            "gemini-10-x",
        ] {
            assert!(uses_thinking_level(&model(id)), "{id}");
        }
        for id in [
            "gemini-2.5-flash",
            "gemini-3.1-flash-lite-image",
            "gemini-pro-latest",
        ] {
            assert_eq!(
                uses_thinking_level(&model(id)),
                id == "gemini-3.1-flash-lite-image",
                "{id}"
            );
        }
    }

    #[test]
    fn signatures() {
        assert_eq!(valid_signature(Some("c2lnMQ==")), Some("c2lnMQ=="));
        assert_eq!(valid_signature(Some("bad sig")), None);
        assert_eq!(valid_signature(Some("abc")), None);
    }

    #[test]
    fn api_versions() {
        assert!(includes_api_version("https://x/v1"));
        assert!(includes_api_version("https://x/v1beta1/y"));
        assert!(!includes_api_version("https://x/version"));
    }
}
