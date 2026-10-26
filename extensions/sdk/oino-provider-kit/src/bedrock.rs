//! Amazon Bedrock `ConverseStream` (pi-ai `api/bedrock-converse-stream.js` plus the AWS SDK's
//! REST-JSON serialization, endpoint/region resolution, auth selection and error texts).
//!
//! The `before-request` payload is pi's `onPayload` command input (`{modelId, messages, system,
//! inferenceConfig, toolConfig, additionalModelRequestFields, requestMetadata}`, image bytes as
//! base64); [`finalize`] drops `modelId` (it moves into the URL). IAM requests carry a
//! [`Signing`] that the component applies to the final body (`SigV4`); bearer tokens go into
//! `Authorization` directly.

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use oino_ai::transcript::{collapse_system_messages, current_tools, system_message_text};
use oino_ai::transform::{split_error_tool_result_images, transform_messages};
use oino_ai::{
    AssistantContent, AssistantDiagnostic, AssistantMessage, CacheRetention, JsonObject, Message,
    Model, ProviderEvent, StopReason, TextContent, ThinkingContent, ThinkingLevel, ToolCall,
    ToolDecl, UserContent, UserContentField,
};
use serde_json::{Value, json};

use crate::constrained::{json_schema_tool_parameters, resolve_json_schema_strict};
use crate::emit::Emitter;
use crate::errors::SdkError;
use crate::eventstream::EventStreamDecoder;
use crate::exchange::{
    ErrorStyle, Exchange, Framing, Step, StreamError, StreamFailure, StreamParser,
};
use crate::json::{parse_streaming_json, parse_tool_call_arguments};
use crate::options::{adjust_max_tokens_for_thinking, clamp_max_tokens_to_context};
use crate::request::{Headers, HttpRequest, ProviderRequest, USER_AGENT};
use crate::sigv4::{Signing, escape_uri};
use crate::sse::SseEvent;
use crate::util::{calculate_cost, mapped_level};

const EMPTY_TEXT_PLACEHOLDER: &str = "<empty>";
const THINKING_BINDING_CONTROLS_BETA: &str = "thinking-binding-controls-2026-08-01";
const REDACTED_THINKING_PLACEHOLDER: &str = "[Reasoning redacted]";
const DEFAULT_REGION: &str = "us-east-1";
/// The SDK's message when no credential source works.
pub const NO_CREDENTIALS: &str = "Could not load credentials from any providers";
/// Environment variables whose value may reach the component as the "API key" only because the
/// registration lists them as credential markers (pi resolves these as ambient credentials).
const AMBIENT_MARKERS: [&str; 5] = [
    "AWS_PROFILE",
    "AWS_ACCESS_KEY_ID",
    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
];

fn match_candidates(model: &Model) -> Vec<String> {
    let mut values = vec![model.id.as_str()];
    if !model.name.is_empty() {
        values.push(model.name.as_str());
    }
    values
        .iter()
        .flat_map(|value| {
            let lower = value.to_lowercase();
            let mut dashed = String::with_capacity(lower.len());
            let mut in_run = false;
            for c in lower.chars() {
                if c.is_whitespace() || c == '_' || c == '.' || c == ':' {
                    if !in_run {
                        dashed.push('-');
                    }
                    in_run = true;
                } else {
                    dashed.push(c);
                    in_run = false;
                }
            }
            [lower, dashed]
        })
        .collect()
}

fn any_candidate(model: &Model, needles: &[&str]) -> bool {
    match_candidates(model)
        .iter()
        .any(|s| needles.iter().any(|n| s.contains(n)))
}

/// pi `isAnthropicClaudeModel`.
fn is_claude(model: &Model) -> bool {
    let id = model.id.to_lowercase();
    let name = model.name.to_lowercase();
    id.contains("anthropic.claude")
        || id.contains("anthropic/claude")
        || name.contains("anthropic.claude")
        || name.contains("anthropic/claude")
        || name.contains("claude")
}

/// pi `supportsAdaptiveThinking`.
fn adaptive_thinking(model: &Model) -> bool {
    any_candidate(
        model,
        &[
            "opus-4-6",
            "opus-4-7",
            "opus-4-8",
            "opus-5",
            "sonnet-4-6",
            "sonnet-5",
            "haiku-5",
            "fable-5",
        ],
    )
}

/// pi `supportsNativeXhighEffort` / `supportsThinkingBlockBinding` (same model set).
fn native_xhigh(model: &Model) -> bool {
    any_candidate(
        model,
        &[
            "opus-4-7", "opus-4-8", "opus-5", "sonnet-5", "haiku-5", "fable-5",
        ],
    )
}

/// pi `supportsPromptCaching`.
fn prompt_caching(model: &Model, request: &ProviderRequest) -> bool {
    let candidates = match_candidates(model);
    if !candidates.iter().any(|s| s.contains("claude")) {
        return request.env_value("AWS_BEDROCK_FORCE_CACHE") == Some("1");
    }
    candidates.iter().any(|s| {
        [
            "fable-5",
            "opus-5",
            "sonnet-5",
            "haiku-5",
            "-4-",
            "claude-3-7-sonnet",
            "claude-3-5-haiku",
        ]
        .iter()
        .any(|n| s.contains(n))
    })
}

/// pi `mapThinkingLevelToEffort` (Bedrock).
fn effort_for(model: &Model, level: ThinkingLevel) -> String {
    if level == ThinkingLevel::Xhigh && native_xhigh(model) {
        return "xhigh".to_owned();
    }
    if let oino_ai::MapEntry::Value(v) = mapped_level(model, level) {
        return v.to_owned();
    }
    match level {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        _ => "high",
    }
    .to_owned()
}

fn resolve_retention(request: &ProviderRequest) -> CacheRetention {
    request
        .options
        .cache_retention
        .unwrap_or(if request.env_long_retention {
            CacheRetention::Long
        } else {
            CacheRetention::Short
        })
}

fn cache_point(retention: CacheRetention) -> Value {
    if retention == CacheRetention::Long {
        json!({"cachePoint": {"type": "default", "ttl": "1h"}})
    } else {
        json!({"cachePoint": {"type": "default"}})
    }
}

fn non_blank_text(text: &str) -> Option<Value> {
    (!text.trim().is_empty()).then(|| json!({"text": text}))
}

fn image_block(mime_type: &str, data: &str) -> Result<Value, String> {
    let format = match mime_type {
        "image/jpeg" | "image/jpg" => "jpeg",
        "image/png" => "png",
        "image/gif" => "gif",
        "image/webp" => "webp",
        other => return Err(format!("Unknown image type: {other}")),
    };
    let bytes = STANDARD
        .decode(data.trim())
        .map_err(|_| "The string to be decoded is not correctly encoded.".to_owned())?;
    Ok(json!({"image": {"format": format, "source": {"bytes": STANDARD.encode(bytes)}}}))
}

/// pi `sanitizeBedrockDocument`: drops empty object keys.
fn sanitize_document(value: &Value) -> Value {
    match value {
        Value::Array(items) => Value::Array(items.iter().map(sanitize_document).collect()),
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !k.is_empty())
                .map(|(k, v)| (k.clone(), sanitize_document(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

fn tool_result_content(content: &[UserContent]) -> Result<Vec<Value>, String> {
    let mut out = Vec::new();
    for block in content {
        match block {
            UserContent::Image(i) => out.push(image_block(&i.mime_type, &i.data)?),
            UserContent::Text(t) => out.extend(non_blank_text(&t.text)),
        }
    }
    if out.is_empty() {
        out.push(json!({"text": EMPTY_TEXT_PLACEHOLDER}));
    }
    Ok(out)
}

/// pi `normalizeToolCallId` (Bedrock).
fn normalize_tool_call_id(id: &str) -> String {
    id.encode_utf16()
        .map(|unit| match char::from_u32(u32::from(unit)) {
            Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            _ => '_',
        })
        .take(64)
        .collect()
}

/// pi `convertMessages` (Bedrock).
#[allow(clippy::too_many_lines, reason = "1:1 port of pi convertMessages")]
fn convert_messages(
    request: &ProviderRequest,
    collapsed: &[Message],
    retention: CacheRetention,
    now: i64,
) -> Result<Vec<Value>, String> {
    let model = &request.model;
    let conversation = match collapsed.first() {
        Some(Message::System(_)) => &collapsed[1..],
        _ => collapsed,
    };
    let transformed = transform_messages(
        conversation.to_vec(),
        model,
        Some(&|id: &str, _: &Model, _: &AssistantMessage| normalize_tool_call_id(id)),
        now,
    );
    let claude = is_claude(model);
    let mut result: Vec<Value> = Vec::new();
    let mut i = 0;
    while i < transformed.len() {
        match &transformed[i] {
            Message::User(user) => {
                let mut content = Vec::new();
                match &user.content {
                    UserContentField::Text(text) => content.push(
                        non_blank_text(text)
                            .unwrap_or_else(|| json!({"text": EMPTY_TEXT_PLACEHOLDER})),
                    ),
                    UserContentField::Blocks(blocks) => {
                        for block in blocks {
                            match block {
                                UserContent::Text(t) => content.extend(non_blank_text(&t.text)),
                                UserContent::Image(img) => {
                                    content.push(image_block(&img.mime_type, &img.data)?);
                                }
                            }
                        }
                        if content.is_empty() {
                            content.push(json!({"text": EMPTY_TEXT_PLACEHOLDER}));
                        }
                    }
                }
                result.push(json!({"role": "user", "content": content}));
            }
            Message::Assistant(assistant) => {
                let mut blocks = Vec::new();
                for block in &assistant.content {
                    match block {
                        AssistantContent::Text(t) => blocks.extend(non_blank_text(&t.text)),
                        AssistantContent::ToolCall(call) => blocks.push(json!({
                            "toolUse": {
                                "toolUseId": call.id,
                                "name": call.name,
                                "input": sanitize_document(&Value::Object(call.arguments.clone())),
                            }
                        })),
                        AssistantContent::Thinking(thinking) => {
                            if thinking.redacted == Some(true) {
                                if let Some(bytes) = thinking
                                    .thinking_signature
                                    .as_deref()
                                    .filter(|s| !s.is_empty())
                                    .and_then(|s| STANDARD.decode(s).ok())
                                    .filter(|b| !b.is_empty())
                                {
                                    blocks.push(json!({"reasoningContent": {"redactedContent": STANDARD.encode(bytes)}}));
                                }
                                continue;
                            }
                            if thinking.thinking.trim().is_empty() {
                                continue;
                            }
                            if claude {
                                match thinking
                                    .thinking_signature
                                    .as_deref()
                                    .filter(|s| !s.trim().is_empty())
                                {
                                    None => blocks.push(json!({"text": thinking.thinking})),
                                    Some(signature) => blocks.push(json!({
                                        "reasoningContent": {"reasoningText": {"text": thinking.thinking, "signature": signature}}
                                    })),
                                }
                            } else {
                                blocks.push(json!({"reasoningContent": {"reasoningText": {"text": thinking.thinking}}}));
                            }
                        }
                    }
                }
                if !blocks.is_empty() {
                    result.push(json!({"role": "assistant", "content": blocks}));
                }
            }
            Message::ToolResult(_) => {
                let mut results = Vec::new();
                let mut error_images = Vec::new();
                while let Some(Message::ToolResult(tool)) = transformed.get(i) {
                    let split = if claude {
                        split_error_tool_result_images(tool)
                    } else {
                        None
                    };
                    let content = split.as_ref().map_or(&tool.content, |s| &s.content);
                    results.push(json!({
                        "toolResult": {
                            "toolUseId": tool.tool_call_id,
                            "content": tool_result_content(content)?,
                            "status": if tool.is_error { "error" } else { "success" },
                        }
                    }));
                    if let Some(split) = split {
                        error_images.push(json!({"text": split.caption}));
                        for image in &split.images {
                            if let UserContent::Image(img) = image {
                                error_images.push(image_block(&img.mime_type, &img.data)?);
                            }
                        }
                    }
                    i += 1;
                }
                results.extend(error_images);
                result.push(json!({"role": "user", "content": results}));
                continue;
            }
            Message::System(_) => {}
        }
        i += 1;
    }
    if retention != CacheRetention::None
        && prompt_caching(model, request)
        && let Some(last) = result.last_mut()
        && last["role"] == "user"
        && let Some(content) = last["content"].as_array_mut()
    {
        content.push(cache_point(retention));
    }
    Ok(result)
}

/// pi `convertToolConfig`.
fn tool_config(
    tools: &[ToolDecl],
    choice: Option<&Value>,
    strict_supported: bool,
) -> Result<Option<Value>, String> {
    if tools.is_empty() || choice.and_then(Value::as_str) == Some("none") {
        return Ok(None);
    }
    let specs = tools
        .iter()
        .map(|tool| {
            let strict = resolve_json_schema_strict(tool, strict_supported, None)? == Some(true);
            let mut spec = json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": {"json": json_schema_tool_parameters(tool, strict)?},
            });
            if strict {
                spec["strict"] = json!(true);
            }
            Ok(json!({"toolSpec": spec}))
        })
        .collect::<Result<Vec<_>, String>>()?;
    let mut config = json!({"tools": specs});
    match choice {
        Some(Value::String(c)) if c == "auto" => config["toolChoice"] = json!({"auto": {}}),
        Some(Value::String(c)) if c == "any" => config["toolChoice"] = json!({"any": {}}),
        Some(Value::Object(c)) if c.get("type").and_then(Value::as_str) == Some("tool") => {
            config["toolChoice"] =
                json!({"tool": {"name": c.get("name").cloned().unwrap_or(Value::Null)}});
        }
        _ => {}
    }
    Ok(Some(config))
}

fn configured_region(request: &ProviderRequest) -> Option<String> {
    request
        .extra
        .region
        .clone()
        .filter(|r| !r.is_empty())
        .or_else(|| request.env_value("AWS_REGION").map(str::to_owned))
        .or_else(|| request.env_value("AWS_DEFAULT_REGION").map(str::to_owned))
}

fn is_gov_cloud(request: &ProviderRequest) -> bool {
    if configured_region(request).is_some_and(|r| r.to_lowercase().starts_with("us-gov-")) {
        return true;
    }
    let id = request.model.id.to_lowercase();
    id.starts_with("us-gov.") || id.starts_with("arn:aws-us-gov:")
}

/// pi `buildAdditionalModelRequestFields`.
fn additional_fields(
    request: &ProviderRequest,
    reasoning: Option<ThinkingLevel>,
    budgets: &oino_ai::ThinkingBudgets,
) -> Option<Value> {
    let model = &request.model;
    let level = reasoning.filter(|_| model.reasoning)?;
    if is_claude(model) {
        let gov = is_gov_cloud(request);
        let display = (!gov).then(|| {
            request
                .extra
                .thinking_display
                .clone()
                .unwrap_or_else(|| "summarized".to_owned())
        });
        let adaptive = adaptive_thinking(model);
        let mut result = if adaptive {
            let binding = !gov && native_xhigh(model);
            let mut thinking = json!({"type": "adaptive"});
            if let Some(display) = &display {
                thinking["display"] = json!(display);
            }
            if binding {
                thinking["block_binding"] = json!({"prefix_mismatch_behavior": "drop_block"});
            }
            let mut out = json!({"thinking": thinking, "output_config": {"effort": effort_for(model, level)}});
            if binding {
                out["anthropic_beta"] = json!([THINKING_BINDING_CONTROLS_BETA]);
            }
            out
        } else {
            let default = match level {
                ThinkingLevel::Minimal => 1024,
                ThinkingLevel::Low => 2048,
                ThinkingLevel::Medium => 8192,
                _ => 16_384,
            };
            let custom = match level {
                ThinkingLevel::Minimal => budgets.minimal,
                ThinkingLevel::Low => budgets.low,
                ThinkingLevel::Medium => budgets.medium,
                _ => budgets.high,
            };
            let mut thinking =
                json!({"type": "enabled", "budget_tokens": custom.unwrap_or(default)});
            if let Some(display) = &display {
                thinking["display"] = json!(display);
            }
            json!({"thinking": thinking})
        };
        if !adaptive && request.extra.interleaved_thinking.unwrap_or(true) {
            result["anthropic_beta"] = json!(["interleaved-thinking-2025-05-14"]);
        }
        return Some(result);
    }
    if any_candidate(model, &["gpt-oss"]) {
        let effort = match level {
            ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            _ => "high",
        };
        return Some(json!({"reasoning_effort": effort}));
    }
    if any_candidate(model, &["gpt-"]) {
        let effort = match mapped_level(model, level) {
            oino_ai::MapEntry::Value(v) => v.to_owned(),
            _ => match level {
                ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
                ThinkingLevel::Medium => "medium",
                ThinkingLevel::High | ThinkingLevel::Off => "high",
                ThinkingLevel::Xhigh => "xhigh",
                ThinkingLevel::Max => "max",
            }
            .to_owned(),
        };
        return Some(json!({"reasoning": {"effort": effort}}));
    }
    None
}

/// pi `streamSimple` + `stream` up to `ConverseStreamCommand`: the command input.
fn build_command(request: &ProviderRequest, now: i64) -> Result<Value, String> {
    let model = &request.model;
    let options = &request.options;
    let base_max = clamp_max_tokens_to_context(
        model,
        &request.context.messages,
        options.max_tokens.unwrap_or(model.max_tokens),
    );
    let mut max_tokens = base_max;
    let mut budgets = options.thinking_budgets.clone().unwrap_or_default();
    let reasoning = options.reasoning;
    if let Some(level) = reasoning
        && is_claude(model)
        && !adaptive_thinking(model)
    {
        let (adjusted, budget) =
            adjust_max_tokens_for_thinking(Some(base_max), model.max_tokens, level, Some(&budgets));
        max_tokens = clamp_max_tokens_to_context(model, &request.context.messages, adjusted);
        let capped = budget.min(max_tokens.saturating_sub(1024));
        match level {
            ThinkingLevel::Minimal => budgets.minimal = Some(capped),
            ThinkingLevel::Low => budgets.low = Some(capped),
            ThinkingLevel::Medium => budgets.medium = Some(capped),
            _ => budgets.high = Some(capped),
        }
    }
    let retention = resolve_retention(request);
    let collapsed = collapse_system_messages(request.context.clone()).messages;
    let system_text = match collapsed.first() {
        Some(Message::System(system)) => system_message_text(system),
        _ => String::new(),
    };
    let mut command = JsonObject::new();
    command.insert("modelId".into(), json!(model.id));
    command.insert(
        "messages".into(),
        Value::Array(convert_messages(request, &collapsed, retention, now)?),
    );
    if !system_text.is_empty() {
        let mut system = vec![json!({"text": system_text})];
        if retention != CacheRetention::None && prompt_caching(model, request) {
            system.push(cache_point(retention));
        }
        command.insert("system".into(), Value::Array(system));
    }
    let mut inference = JsonObject::new();
    inference.insert("maxTokens".into(), json!(max_tokens));
    if let Some(temperature) = options.temperature {
        inference.insert("temperature".into(), json!(temperature));
    }
    command.insert("inferenceConfig".into(), Value::Object(inference));
    let strict = crate::util::compat_bool(model, "supportsStrictMode").unwrap_or(false);
    if let Some(config) = tool_config(
        &current_tools(&collapsed),
        options.tool_choice.as_ref(),
        strict,
    )? {
        command.insert("toolConfig".into(), config);
    }
    if let Some(fields) = additional_fields(request, reasoning, &budgets) {
        command.insert("additionalModelRequestFields".into(), fields);
    }
    if let Some(metadata) = &request.extra.request_metadata {
        command.insert("requestMetadata".into(), metadata.clone());
    }
    Ok(Value::Object(command))
}

/// The REST-JSON body for a (possibly hook-replaced) command input.
#[must_use]
pub fn finalize(payload: Value) -> Value {
    match payload {
        Value::Object(mut map) => {
            map.shift_remove("modelId");
            map.retain(|_, v| !v.is_null());
            Value::Object(map)
        }
        other => other,
    }
}

/// The host of a standard Bedrock Runtime endpoint (`bedrock-runtime[-fips].<region>.amazonaws.com[.cn]`).
fn endpoint_region(base_url: &str) -> Option<String> {
    let host = base_url
        .split_once("://")
        .map_or(base_url, |(_, rest)| rest)
        .split(['/', ':', '?'])
        .next()?
        .to_lowercase();
    let rest = host
        .strip_prefix("bedrock-runtime-fips.")
        .or_else(|| host.strip_prefix("bedrock-runtime."))?;
    let region = rest
        .strip_suffix(".amazonaws.com.cn")
        .or_else(|| rest.strip_suffix(".amazonaws.com"))?;
    (!region.is_empty()
        && region
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'))
    .then(|| region.to_owned())
}

/// Resolved endpoint and signing region (pi's region logic + the SDK endpoint ruleset).
fn endpoint(request: &ProviderRequest) -> (String, String) {
    let model = &request.model;
    let configured = configured_region(request);
    let ambient_profile = request.env_value("AWS_PROFILE").is_some();
    let from_endpoint = endpoint_region(&model.base_url);
    let explicit = from_endpoint.is_none() || (configured.is_none() && !ambient_profile);
    let arn_region = model.id.strip_prefix("arn:").and_then(|rest| {
        let mut parts = rest.split(':');
        let partition = parts.next()?;
        let service = parts.next()?;
        let region = parts.next()?;
        (partition.starts_with("aws") && service == "bedrock" && !region.is_empty())
            .then(|| region.to_owned())
    });
    let region = arn_region
        .or(configured)
        .or_else(|| from_endpoint.clone().filter(|_| explicit))
        .or_else(|| request.env_value("AWS_PROFILE_REGION").map(str::to_owned))
        .unwrap_or_else(|| DEFAULT_REGION.to_owned());
    let base = if explicit && !model.base_url.is_empty() {
        model.base_url.trim_end_matches('/').to_owned()
    } else {
        format!("https://bedrock-runtime.{region}.amazonaws.com")
    };
    (base, region)
}

/// The SDK's `extendedEncodeURIComponent`.
fn encode_path_component(text: &str) -> String {
    escape_uri(text)
}

/// Bearer token (pi `options.bearerToken || options.apiKey || AWS_BEARER_TOKEN_BEDROCK`), with
/// registration credential markers ignored.
fn bearer_token(request: &ProviderRequest) -> Option<String> {
    let markers: Vec<&str> = AMBIENT_MARKERS
        .iter()
        .filter_map(|n| request.env_value(n))
        .collect();
    request
        .extra
        .bearer_token
        .clone()
        .filter(|t| !t.is_empty())
        .or_else(|| {
            request
                .api_key
                .clone()
                .filter(|k| !k.is_empty() && !markers.contains(&k.as_str()))
        })
        .or_else(|| {
            request
                .env_value("AWS_BEARER_TOKEN_BEDROCK")
                .map(str::to_owned)
        })
}

/// Credentials and region of a shared-config profile (`~/.aws/credentials`, `~/.aws/config`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProfileCredentials {
    /// `aws_access_key_id`.
    pub access_key_id: Option<String>,
    /// `aws_secret_access_key`.
    pub secret_access_key: Option<String>,
    /// `aws_session_token`.
    pub session_token: Option<String>,
    /// `region` (config file).
    pub region: Option<String>,
}

/// The `key = value` pairs of `[section]` in an INI file.
fn ini_section(text: &str, section: &str) -> Vec<(String, String)> {
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name.trim() == section;
            continue;
        }
        if inside && let Some((key, value)) = line.split_once('=') {
            out.push((key.trim().to_ascii_lowercase(), value.trim().to_owned()));
        }
    }
    out
}

/// Static credentials of `profile` (SDK `fromIni` for `aws_access_key_id` profiles; the
/// config file's `[profile name]` / `[default]` section supplies `region` and may also hold
/// keys). SSO, `credential_process`, `role_arn` and web-identity profiles are not resolved.
#[must_use]
pub fn profile_credentials(
    credentials: Option<&str>,
    config: Option<&str>,
    profile: &str,
) -> ProfileCredentials {
    let config_section = if profile == "default" {
        "default".to_owned()
    } else {
        format!("profile {profile}")
    };
    let mut pairs = config
        .map(|c| ini_section(c, &config_section))
        .unwrap_or_default();
    pairs.extend(
        credentials
            .map(|c| ini_section(c, profile))
            .unwrap_or_default(),
    );
    let get = |key: &str| {
        pairs
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .filter(|v| !v.is_empty())
    };
    ProfileCredentials {
        access_key_id: get("aws_access_key_id"),
        secret_access_key: get("aws_secret_access_key"),
        session_token: get("aws_session_token"),
        region: get("region"),
    }
}

/// Which profile the component should load before [`prepare`] (pi: an explicit `profile`
/// option or credential `AWS_PROFILE` wins over env keys; the SDK chain uses `AWS_PROFILE` or
/// `default` when no env keys exist). `None` when env keys or a bearer token apply.
#[must_use]
pub fn profile_to_load(request: &ProviderRequest) -> Option<String> {
    if let Some(profile) = request.extra.profile.clone().filter(|p| !p.is_empty()) {
        return Some(profile);
    }
    if bearer_token(request).is_some() || request.env_value("AWS_BEDROCK_SKIP_AUTH") == Some("1") {
        return None;
    }
    if request.env_value("AWS_ACCESS_KEY_ID").is_some()
        && request.env_value("AWS_SECRET_ACCESS_KEY").is_some()
    {
        return None;
    }
    Some(
        request
            .env_value("AWS_PROFILE")
            .unwrap_or("default")
            .to_owned(),
    )
}

/// Applies loaded profile credentials to `request.env` (region as a fallback only).
pub fn apply_profile(request: &mut ProviderRequest, profile: &ProfileCredentials) {
    if let (Some(key), Some(secret)) = (&profile.access_key_id, &profile.secret_access_key) {
        request.env.insert("AWS_ACCESS_KEY_ID".into(), key.clone());
        request
            .env
            .insert("AWS_SECRET_ACCESS_KEY".into(), secret.clone());
        match &profile.session_token {
            Some(token) => request
                .env
                .insert("AWS_SESSION_TOKEN".into(), token.clone()),
            None => request.env.remove("AWS_SESSION_TOKEN"),
        };
    }
    if let Some(region) = &profile.region {
        request
            .env
            .insert("AWS_PROFILE_REGION".into(), region.clone());
    }
}

/// A prepared Bedrock request.
pub struct Prepared {
    /// Outbound request (command-input payload; `signing` set for IAM credentials).
    pub http: HttpRequest,
    /// Streaming state.
    pub exchange: Exchange,
}

/// Builds the request.
///
/// # Errors
/// No usable credentials, an unknown image type, or a tool that cannot be expressed.
pub fn prepare(request: &ProviderRequest, now: i64) -> Result<Prepared, String> {
    let model = &request.model;
    let skip_auth = request.env_value("AWS_BEDROCK_SKIP_AUTH") == Some("1");
    let bearer = bearer_token(request).filter(|_| !skip_auth);
    let (base, region) = endpoint(request);
    let signing = if bearer.is_some() {
        None
    } else if skip_auth {
        Some(Signing {
            access_key_id: "dummy-access-key".into(),
            secret_access_key: "dummy-secret-key".into(),
            session_token: None,
            region: region.clone(),
            service: "bedrock".into(),
        })
    } else {
        let key = request.env_value("AWS_ACCESS_KEY_ID");
        let secret = request.env_value("AWS_SECRET_ACCESS_KEY");
        match (key, secret) {
            (Some(key), Some(secret)) => Some(Signing {
                access_key_id: key.to_owned(),
                secret_access_key: secret.to_owned(),
                session_token: request.env_value("AWS_SESSION_TOKEN").map(str::to_owned),
                region: region.clone(),
                service: "bedrock".into(),
            }),
            _ => return Err(NO_CREDENTIALS.to_owned()),
        }
    };
    let command = build_command(request, now)?;
    let mut headers = Headers::new();
    headers.set("content-type", "application/json");
    headers.set("user-agent", USER_AGENT);
    for (name, value) in request.header_overrides() {
        let lower = name.to_ascii_lowercase();
        if lower.starts_with("x-amz-") || lower == "authorization" || lower == "host" {
            continue;
        }
        headers.apply(&name, value.as_deref());
    }
    if let Some(token) = &bearer {
        headers.set("Authorization", format!("Bearer {token}"));
    }
    let url = format!(
        "{base}/model/{}/converse-stream",
        encode_path_component(&model.id)
    );
    let parser = Parser {
        model: model.clone(),
        now,
        open: Vec::new(),
        redacted: Vec::new(),
        request_id: None,
    };
    Ok(Prepared {
        http: HttpRequest {
            url,
            headers,
            payload: command,
            timeout_ms: request.options.timeout_ms,
            signing,
        },
        exchange: Exchange::new(
            Emitter::new(model, now),
            Box::new(parser),
            ErrorStyle::Bedrock,
        )
        .with_framing(Framing::EventStream(EventStreamDecoder::new())),
    })
}

/// pi `BEDROCK_ERROR_PREFIXES`.
fn error_prefix(name: &str) -> &str {
    match name {
        "InternalServerException" => "Internal server error",
        "ModelStreamErrorException" => "Model stream error",
        "ValidationException" => "Validation error",
        "ThrottlingException" => "Throttling error",
        "ServiceUnavailableException" => "Service unavailable",
        other => other,
    }
}

const DATA_RETENTION_DOCS_URL: &str =
    "https://docs.aws.amazon.com/bedrock/latest/userguide/data-retention.html";

/// pi `formatBedrockError` for a service exception.
fn format_service_error(name: &str, message: &str) -> String {
    let hint = if message.to_lowercase().contains("data retention mode") {
        format!(" See {DATA_RETENTION_DOCS_URL} for supported data retention modes.")
    } else {
        String::new()
    };
    format!("{}: {message}{hint}", error_prefix(name))
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.as_str())
}

/// The SDK error code of a failed response (`x-amzn-errortype`, `__type`, `code`).
fn error_code(headers: &[(String, String)], body: &Value) -> Option<String> {
    let sanitize = |raw: &str| {
        let raw = raw.split(':').next().unwrap_or(raw);
        raw.rsplit('#').next().unwrap_or(raw).trim().to_owned()
    };
    header(headers, "x-amzn-errortype")
        .map(sanitize)
        .or_else(|| body["__type"].as_str().map(sanitize))
        .or_else(|| body["code"].as_str().map(sanitize))
        .filter(|c| !c.is_empty())
}

/// V8's `JSON.parse` message for a non-JSON body.
fn v8_json_error(text: &str) -> String {
    let Some(first) = text.chars().next() else {
        return "Unexpected end of JSON input".to_owned();
    };
    let shown: String = text.chars().take(10).collect();
    let ellipsis = if text.chars().count() > 10 { "..." } else { "" };
    format!("Unexpected token '{first}', \"{shown}\"{ellipsis} is not valid JSON")
}

/// AWS SDK error for a non-2xx response, rendered like pi `formatBedrockError`.
#[must_use]
pub fn status_error(status: u16, headers: &[(String, String)], body: &str) -> SdkError {
    let message = match serde_json::from_str::<Value>(body) {
        Ok(json) => {
            let code = error_code(headers, &json).unwrap_or_else(|| "UnknownError".to_owned());
            let text = json["message"]
                .as_str()
                .or_else(|| json["Message"].as_str())
                .map_or_else(|| code.clone(), str::to_owned);
            format_service_error(&code, &text)
        }
        Err(_) if body.trim().is_empty() => format_service_error("UnknownError", "UnknownError"),
        Err(_) => format!(
            "{}\n  Deserialization error: to see the raw response, inspect the hidden field {{error}}.$response on this object.",
            v8_json_error(body)
        ),
    };
    SdkError {
        status: Some(status),
        message,
        error: None,
    }
}

/// An open content block keyed by Bedrock `contentBlockIndex`.
struct Open {
    bedrock_index: i64,
    index: usize,
    partial_json: Option<String>,
}

struct Parser {
    model: Model,
    now: i64,
    open: Vec<Open>,
    /// Redacted reasoning chunks by content index (pi `redactedChunks`).
    redacted: Vec<(usize, Vec<u8>)>,
    request_id: Option<String>,
}

impl Parser {
    fn find(&self, bedrock_index: i64) -> Option<usize> {
        self.open
            .iter()
            .position(|o| o.bedrock_index == bedrock_index)
    }

    fn flush_redacted(&mut self, index: usize, out: &mut Emitter) {
        if let Some(position) = self.redacted.iter().position(|(i, _)| *i == index) {
            let (_, bytes) = self.redacted.remove(position);
            if let Some(AssistantContent::Thinking(block)) = out.block(index) {
                block.thinking_signature = Some(STANDARD.encode(bytes));
            }
        }
    }

    fn finalize_all(&mut self, out: &mut Emitter) {
        let indices: Vec<usize> = self.redacted.iter().map(|(i, _)| *i).collect();
        for index in indices {
            self.flush_redacted(index, out);
        }
        self.open.clear();
    }

    #[allow(
        clippy::too_many_lines,
        reason = "1:1 port of pi handleContentBlockDelta"
    )]
    fn delta(&mut self, event: &Value, out: &mut Emitter) {
        let bedrock_index = event["contentBlockIndex"].as_i64().unwrap_or(0);
        let delta = &event["delta"];
        let mut slot = self.find(bedrock_index);
        if let Some(text) = delta.get("text").and_then(Value::as_str) {
            if slot.is_none() {
                let index = out.add(AssistantContent::Text(TextContent::new("")));
                self.open.push(Open {
                    bedrock_index,
                    index,
                    partial_json: None,
                });
                slot = Some(self.open.len() - 1);
                out.push(ProviderEvent::TextStart {
                    content_index: index,
                });
            }
            if let Some(slot) = slot {
                let index = self.open[slot].index;
                if let Some(AssistantContent::Text(block)) = out.block(index) {
                    block.text.push_str(text);
                    out.push(ProviderEvent::TextDelta {
                        content_index: index,
                        delta: text.to_owned(),
                    });
                }
            }
        } else if delta["toolUse"].is_object() {
            let Some(slot) = slot else { return };
            let open = &mut self.open[slot];
            let index = open.index;
            let Some(partial) = open.partial_json.as_mut() else {
                return;
            };
            let input = delta["toolUse"]["input"].as_str().unwrap_or_default();
            partial.push_str(input);
            let parsed = parse_streaming_json(partial);
            if let Some(AssistantContent::ToolCall(call)) = out.block(index) {
                call.arguments = parsed.as_object().cloned().unwrap_or_default();
                out.push(ProviderEvent::ToolcallDelta {
                    content_index: index,
                    delta: input.to_owned(),
                });
            }
        } else if delta["reasoningContent"].is_object() {
            let reasoning = &delta["reasoningContent"];
            if slot.is_none() {
                let index = out.add(AssistantContent::Thinking(ThinkingContent {
                    thinking: String::new(),
                    thinking_signature: Some(String::new()),
                    redacted: None,
                }));
                self.open.push(Open {
                    bedrock_index,
                    index,
                    partial_json: None,
                });
                slot = Some(self.open.len() - 1);
                out.push(ProviderEvent::ThinkingStart {
                    content_index: index,
                });
            }
            let Some(slot) = slot else { return };
            let index = self.open[slot].index;
            let mut events = Vec::new();
            if let Some(AssistantContent::Thinking(block)) = out.block(index) {
                if let Some(text) = reasoning["text"].as_str().filter(|t| !t.is_empty()) {
                    block.thinking.push_str(text);
                    events.push(ProviderEvent::ThinkingDelta {
                        content_index: index,
                        delta: text.to_owned(),
                    });
                }
                if let Some(signature) = reasoning["signature"].as_str().filter(|s| !s.is_empty())
                    && block.redacted != Some(true)
                {
                    block
                        .thinking_signature
                        .get_or_insert_with(String::new)
                        .push_str(signature);
                }
                let chunk = reasoning["redactedContent"]
                    .as_str()
                    .and_then(|b| STANDARD.decode(b).ok())
                    .filter(|b| !b.is_empty());
                if let Some(chunk) = chunk {
                    if block.redacted != Some(true) {
                        block.redacted = Some(true);
                        block.thinking_signature = Some(String::new());
                        block.thinking.push_str(REDACTED_THINKING_PLACEHOLDER);
                        events.push(ProviderEvent::ThinkingDelta {
                            content_index: index,
                            delta: REDACTED_THINKING_PLACEHOLDER.to_owned(),
                        });
                    }
                    match self.redacted.iter_mut().find(|(i, _)| *i == index) {
                        Some((_, bytes)) => bytes.extend(chunk),
                        None => self.redacted.push((index, chunk)),
                    }
                }
            }
            for event in events {
                out.push(event);
            }
        }
    }

    fn stop(&mut self, event: &Value, out: &mut Emitter) {
        let bedrock_index = event["contentBlockIndex"].as_i64().unwrap_or(0);
        let Some(slot) = self.find(bedrock_index) else {
            return;
        };
        let open = self.open.remove(slot);
        let index = open.index;
        match out.block(index) {
            Some(AssistantContent::Text(block)) => {
                let content = block.text.clone();
                out.push(ProviderEvent::TextEnd {
                    content_index: index,
                    content,
                });
            }
            Some(AssistantContent::Thinking(_)) => {
                self.flush_redacted(index, out);
                if let Some(AssistantContent::Thinking(block)) = out.block(index) {
                    let content = block.thinking.clone();
                    out.push(ProviderEvent::ThinkingEnd {
                        content_index: index,
                        content,
                    });
                }
            }
            Some(AssistantContent::ToolCall(call)) => {
                let parsed = parse_tool_call_arguments(open.partial_json.as_deref().unwrap_or(""));
                call.arguments = parsed.arguments;
                call.arguments_error = parsed.error;
                let tool_call = call.clone();
                out.push(ProviderEvent::ToolcallEnd {
                    content_index: index,
                    tool_call,
                });
            }
            None => {}
        }
    }
}

impl StreamParser for Parser {
    #[allow(
        clippy::too_many_lines,
        reason = "one match over the ConverseStream event union"
    )]
    fn event(&mut self, sse: &SseEvent, out: &mut Emitter) -> Result<Step, StreamError> {
        let kind = sse.event.as_deref().unwrap_or_default();
        if kind == "corrupt" {
            return Err(sse.data.clone().into());
        }
        let payload: Value = serde_json::from_str(&sse.data).unwrap_or(Value::Null);
        if let Some(member) = kind.strip_prefix("exception:") {
            // The stream member (`throttlingException`) names the modeled exception class.
            let mut chars = member.chars();
            let name: String = chars
                .next()
                .map(|c| c.to_ascii_uppercase())
                .into_iter()
                .chain(chars)
                .collect();
            let message = payload["message"]
                .as_str()
                .or_else(|| payload["Message"].as_str())
                .unwrap_or(&name);
            return Err(StreamError::Message(format_service_error(&name, message)));
        }
        if let Some(code) = kind.strip_prefix("error:") {
            return Err(StreamError::Message(format_service_error(code, &sse.data)));
        }
        match kind {
            "messageStart" => {
                if payload["role"] != "assistant" {
                    return Err(
                        "Unexpected assistant message start but got user message start instead"
                            .into(),
                    );
                }
                out.start();
            }
            "contentBlockStart" => {
                let tool = &payload["start"]["toolUse"];
                if tool.is_object() {
                    let call = ToolCall {
                        id: tool["toolUseId"].as_str().unwrap_or_default().to_owned(),
                        name: tool["name"].as_str().unwrap_or_default().to_owned(),
                        ..ToolCall::default()
                    };
                    let (id, name) = (call.id.clone(), call.name.clone());
                    let index = out.add(AssistantContent::ToolCall(call));
                    self.open.push(Open {
                        bedrock_index: payload["contentBlockIndex"].as_i64().unwrap_or(0),
                        index,
                        partial_json: Some(String::new()),
                    });
                    out.push(ProviderEvent::ToolcallStart {
                        content_index: index,
                        id: Some(id),
                        tool_name: Some(name),
                    });
                }
            }
            "contentBlockDelta" => self.delta(&payload, out),
            "contentBlockStop" => self.stop(&payload, out),
            "messageStop" => {
                let reason = payload["stopReason"].as_str().unwrap_or_default();
                out.message.raw_stop_reason = payload["stopReason"].as_str().map(str::to_owned);
                let (stop, message) = match reason {
                    "end_turn" | "stop_sequence" => (StopReason::Stop, None),
                    "max_tokens" | "model_context_window_exceeded" => (StopReason::Length, None),
                    "tool_use" => (StopReason::ToolUse, None),
                    "" => (StopReason::Error, None),
                    other => (
                        StopReason::Error,
                        Some(format!("Provider stopped with: {other}")),
                    ),
                };
                out.message.stop_reason = stop;
                if message.is_some() {
                    out.message.error_message = message;
                }
            }
            "metadata" => {
                let usage = &payload["usage"];
                if usage.is_object() {
                    let count = |key: &str| usage[key].as_u64().unwrap_or(0);
                    let u = &mut out.message.usage;
                    u.input = count("inputTokens");
                    u.output = count("outputTokens");
                    u.cache_read = count("cacheReadInputTokens");
                    u.cache_write = count("cacheWriteInputTokens");
                    u.cache_write_1h = usage["cacheDetails"].as_array().map(|details| {
                        details
                            .iter()
                            .filter(|d| d["ttl"] == "1h")
                            .map(|d| d["inputTokens"].as_u64().unwrap_or(0))
                            .sum()
                    });
                    let total = count("totalTokens");
                    u.total_tokens = if total == 0 {
                        u.input + u.output
                    } else {
                        total
                    };
                    calculate_cost(&self.model, &mut out.message.usage);
                }
            }
            _ => return Ok(Step::Skip),
        }
        Ok(Step::Event(json!({kind: payload})))
    }

    fn finish(&mut self, out: &mut Emitter) -> Result<(), StreamError> {
        match out.message.stop_reason {
            StopReason::Pending => Err("Bedrock stream ended without a stop reason".into()),
            StopReason::Error | StopReason::Aborted => Err(out
                .message
                .error_message
                .clone()
                .unwrap_or_else(|| "An unknown error occurred".to_owned())
                .into()),
            _ => {
                self.finalize_all(out);
                Ok(())
            }
        }
    }

    fn cleanup(&mut self, out: &mut Emitter) {
        self.finalize_all(out);
    }

    fn emits_start(&self) -> bool {
        true
    }

    fn response_headers(&mut self, headers: &[(String, String)]) {
        self.request_id = header(headers, "x-amzn-requestid").map(str::to_owned);
    }

    fn failure_diagnostics(&mut self, error: &StreamFailure<'_>, out: &mut Emitter) {
        let mut details = JsonObject::new();
        let normalize = |v: &str| {
            let t = v.trim();
            (!t.is_empty() && t.chars().count() <= 200).then(|| t.to_owned())
        };
        let mut request_id = self.request_id.as_deref().and_then(normalize);
        match error {
            StreamFailure::Status {
                status,
                headers,
                body,
            } => {
                details.insert("status".into(), json!(status));
                let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
                if json.is_object()
                    && let Some(code) =
                        error_code(headers, &json).filter(|c| c.ends_with("Exception"))
                {
                    details.insert("errorCode".into(), json!(code));
                }
                request_id = header(headers, "x-amzn-requestid").and_then(normalize);
            }
            StreamFailure::Stream(StreamError::Message(message)) => {
                if let Some(code) = stream_error_code(message) {
                    details.insert("errorCode".into(), json!(code));
                }
            }
            StreamFailure::Stream(StreamError::Sdk(_)) | StreamFailure::Other => {}
        }
        if let Some(id) = request_id {
            details.insert("requestId".into(), json!(id));
        }
        if details.is_empty() {
            return;
        }
        out.diagnostic(AssistantDiagnostic {
            kind: "bedrock_response_failure".into(),
            timestamp: self.now,
            error: None,
            details: Some(details),
        });
    }
}

/// Modeled exception name behind a rendered in-stream error.
fn stream_error_code(message: &str) -> Option<&'static str> {
    [
        ("Internal server error: ", "InternalServerException"),
        ("Model stream error: ", "ModelStreamErrorException"),
        ("Validation error: ", "ValidationException"),
        ("Throttling error: ", "ThrottlingException"),
        ("Service unavailable: ", "ServiceUnavailableException"),
    ]
    .iter()
    .find(|(prefix, _)| message.starts_with(prefix))
    .map(|(_, code)| *code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_regions() {
        assert_eq!(
            endpoint_region("https://bedrock-runtime.us-east-1.amazonaws.com").as_deref(),
            Some("us-east-1")
        );
        assert_eq!(
            endpoint_region("https://vpce-1.bedrock-runtime.example.com"),
            None
        );
    }

    #[test]
    fn path_encoding() {
        assert_eq!(
            encode_path_component("amazon.nova-2-lite-v1:0"),
            "amazon.nova-2-lite-v1%3A0"
        );
    }

    #[test]
    fn profiles() {
        let creds = "[default]\naws_access_key_id = A\naws_secret_access_key = S\n[work]\naws_access_key_id=W\naws_secret_access_key=WS\naws_session_token=T\n";
        let config = "[profile work]\nregion = eu-central-1\n";
        let work = profile_credentials(Some(creds), Some(config), "work");
        assert_eq!(work.access_key_id.as_deref(), Some("W"));
        assert_eq!(work.session_token.as_deref(), Some("T"));
        assert_eq!(work.region.as_deref(), Some("eu-central-1"));
        let default = profile_credentials(Some(creds), None, "default");
        assert_eq!(default.secret_access_key.as_deref(), Some("S"));
        assert_eq!(default.region, None);
    }

    #[test]
    fn json_errors() {
        assert_eq!(
            v8_json_error("<html>Forbidden</html>"),
            "Unexpected token '<', \"<html>Forb\"... is not valid JSON"
        );
    }
}
