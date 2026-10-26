//! Anthropic Messages (pi-ai `api/anthropic-messages.js`): request params, headers and the SSE
//! stream parser.

use std::collections::{BTreeMap, HashSet};

use oino_ai::transcript::{
    collapse_system_messages, current_tools, render_system_message_update, system_message_text,
};
use oino_ai::transform::{split_error_tool_result_images, transform_messages};
use oino_ai::{
    AssistantContent, AssistantDiagnostic, AssistantMessage, CacheRetention, Context,
    DiagnosticError, JsonObject, MapEntry, Message, Model, ModelCost, ProviderEvent, StopReason,
    TextContent, ThinkingContent, ThinkingLevel, ToolCall, ToolDecl, UserContent, UserContentField,
};
use serde_json::{Value, json};

use crate::constrained::{json_schema_tool_parameters, prefers_strict, resolve_json_schema_strict};
use crate::emit::Emitter;
use crate::exchange::{ErrorStyle, Exchange, Step, StreamError, StreamParser};
use crate::json::{parse_json_with_repair, parse_streaming_object, parse_tool_call_arguments};
use crate::options::{adjust_max_tokens_for_thinking, clamp_max_tokens_to_context};
use crate::request::{Headers, HttpRequest, ProviderRequest, USER_AGENT, join_url};
use crate::sse::SseEvent;
use crate::util::{calculate_cost, compat_bool, compat_value, mapped_level};

const CLAUDE_CODE_VERSION: &str = "2.1.280";
const CLAUDE_CODE_TOOLS: [&str; 17] = [
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];
const FINE_GRAINED_TOOL_STREAMING_BETA: &str = "fine-grained-tool-streaming-2025-05-14";
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
const SERVER_SIDE_FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const MID_CONVERSATION_OUTPUT_CONFIG_BETA: &str = "mid-conversation-output-config-2026-07-01";
const THINKING_BINDING_CONTROLS_BETA: &str = "thinking-binding-controls-2026-08-01";
const INLINE_TOOLS_BETA: &str = "inline-tools-2026-09-15";
const STRICT_LIMIT_MESSAGES: [&str; 3] = [
    "The compiled grammar is too large",
    "Too many strict tools (",
    "Schemas contains too many parameters with union types (",
];

fn deferred_tool_placeholder() -> Value {
    json!({
        "name": "__pi_deferred_placeholder__",
        "description": "Reserved placeholder. Never available. Never call this.",
        "input_schema": {"type": "object", "properties": {}, "required": []},
        "defer_loading": true,
    })
}

fn to_claude_code_name(name: &str) -> String {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|tool| tool.eq_ignore_ascii_case(name))
        .map_or_else(|| name.to_owned(), |tool| (*tool).to_owned())
}

fn from_claude_code_name(name: &str, tools: &[ToolDecl]) -> String {
    tools
        .iter()
        .find(|tool| tool.name.to_lowercase() == name.to_lowercase())
        .map_or_else(|| name.to_owned(), |tool| tool.name.clone())
}

/// pi `getAnthropicCompat`.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors pi's compat flag record"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AnthropicCompat {
    /// `eager_input_streaming` on tools.
    pub eager_tool_input_streaming: bool,
    /// 1h cache TTL.
    pub long_cache_retention: bool,
    /// Session-affinity headers.
    pub session_affinity_headers: bool,
    /// `openrouter` or other.
    pub session_affinity_format: Option<String>,
    /// `cache_control` on the last tool.
    pub cache_control_on_tools: bool,
    /// `temperature` accepted.
    pub temperature: bool,
    /// Empty thinking signatures accepted.
    pub allow_empty_signature: bool,
    /// Strict tools.
    pub strict_tools: bool,
    /// Later system messages accepted.
    pub mid_convo_system_messages: bool,
    /// Native tool changes.
    pub mid_convo_tool_changes: bool,
}

/// pi `getAnthropicCompat(model)`.
#[must_use]
pub fn compat(model: &Model) -> AnthropicCompat {
    let open_router = model.provider == "openrouter" || model.base_url.contains("openrouter.ai");
    let flag = |key: &str, default: bool| compat_bool(model, key).unwrap_or(default);
    AnthropicCompat {
        eager_tool_input_streaming: flag("supportsEagerToolInputStreaming", true),
        long_cache_retention: flag("supportsLongCacheRetention", true),
        session_affinity_headers: flag("sendSessionAffinityHeaders", open_router),
        session_affinity_format: compat_value(model, "sessionAffinityFormat")
            .and_then(Value::as_str)
            .map(str::to_owned)
            .or_else(|| open_router.then(|| "openrouter".to_owned())),
        cache_control_on_tools: flag("supportsCacheControlOnTools", true),
        temperature: flag("supportsTemperature", true),
        allow_empty_signature: flag("allowEmptySignature", false),
        strict_tools: flag("supportsStrictTools", false),
        mid_convo_system_messages: flag("supportsMidConvoSystemMessages", false),
        mid_convo_tool_changes: flag("supportsMidConvoToolChanges", false),
    }
}

/// pi `isOAuthToken`.
#[must_use]
pub fn is_oauth_token(key: &str) -> bool {
    key.contains("sk-ant-oat")
}

/// Resolved stream options (pi `AnthropicOptions` after `streamSimple`).
#[derive(Clone, Debug, PartialEq)]
struct Options {
    thinking_enabled: bool,
    effort: Option<String>,
    thinking_budget_tokens: Option<u64>,
    max_tokens: u64,
    temperature: Option<f64>,
    tool_choice: Option<Value>,
    metadata: Option<JsonObject>,
    retention: CacheRetention,
    session_id: Option<String>,
    interleaved_thinking: Option<bool>,
    thinking_display: Option<String>,
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

/// pi `mapThinkingLevelToEffort`.
fn effort_for(model: &Model, level: ThinkingLevel) -> String {
    if let MapEntry::Value(value) = mapped_level(model, level) {
        return value.to_owned();
    }
    match level {
        ThinkingLevel::Minimal | ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        _ => "high",
    }
    .to_owned()
}

/// pi `streamSimple` option resolution.
fn simple_options(request: &ProviderRequest) -> Options {
    let model = &request.model;
    let options = &request.options;
    let base_max = clamp_max_tokens_to_context(
        model,
        &request.context.messages,
        options.max_tokens.unwrap_or(model.max_tokens),
    );
    let mut resolved = Options {
        thinking_enabled: false,
        effort: None,
        thinking_budget_tokens: None,
        max_tokens: base_max,
        temperature: options.temperature,
        tool_choice: options.tool_choice.clone(),
        metadata: options.metadata.clone(),
        retention: resolve_retention(request),
        session_id: options.session_id.clone(),
        interleaved_thinking: request.extra.interleaved_thinking,
        thinking_display: request.extra.thinking_display.clone(),
    };
    let Some(level) = options.reasoning else {
        return resolved;
    };
    resolved.thinking_enabled = true;
    if compat_bool(model, "forceAdaptiveThinking") == Some(true) {
        resolved.effort = Some(effort_for(model, level));
        return resolved;
    }
    let (adjusted, budget) = adjust_max_tokens_for_thinking(
        Some(base_max),
        model.max_tokens,
        level,
        options.thinking_budgets.as_ref(),
    );
    let max_tokens = clamp_max_tokens_to_context(model, &request.context.messages, adjusted);
    resolved.max_tokens = max_tokens;
    resolved.thinking_budget_tokens = Some(budget.min(max_tokens.saturating_sub(1024)));
    resolved
}

fn cache_control(model: &Model, retention: CacheRetention) -> Option<Value> {
    match retention {
        CacheRetention::None => None,
        CacheRetention::Long if compat(model).long_cache_retention => {
            Some(json!({"type": "ephemeral", "ttl": "1h"}))
        }
        _ => Some(json!({"type": "ephemeral"})),
    }
}

/// pi `normalizeToolCallId`: `[^a-zA-Z0-9_-]` → `_` per UTF-16 unit, at most 64 units.
fn normalize_tool_call_id(id: &str) -> String {
    id.encode_utf16()
        .take(64)
        .map(|unit| match char::from_u32(u32::from(unit)) {
            Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            _ => '_',
        })
        .collect()
}

fn image_block(data: &str, mime_type: &str) -> Value {
    json!({"type": "image", "source": {"type": "base64", "media_type": mime_type, "data": data}})
}

/// pi `convertContentBlocks` (tool-result content).
fn convert_content_blocks(content: &[UserContent]) -> Value {
    let has_images = content.iter().any(|c| matches!(c, UserContent::Image(_)));
    if !has_images {
        let text: Vec<&str> = content
            .iter()
            .filter_map(|c| match c {
                UserContent::Text(t) => Some(t.text.as_str()),
                UserContent::Image(_) => None,
            })
            .collect();
        return Value::String(text.join("\n"));
    }
    let mut blocks: Vec<Value> = content
        .iter()
        .map(|block| match block {
            UserContent::Text(text) => json!({"type": "text", "text": text.text}),
            UserContent::Image(image) => image_block(&image.data, &image.mime_type),
        })
        .collect();
    if !blocks.iter().any(|b| b["type"] == "text") {
        blocks.insert(0, json!({"type": "text", "text": "(see attached image)"}));
    }
    Value::Array(blocks)
}

fn convert_tool_result(id: &str, content: &[UserContent], is_error: bool) -> Value {
    json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": convert_content_blocks(content),
        "is_error": is_error,
    })
}

/// Keywords Anthropic strict tool use rejects.
fn strict_unsupported_keyword(key: &str, value: &Value) -> bool {
    const KEYWORDS: [&str; 11] = [
        "minimum",
        "maximum",
        "exclusiveMinimum",
        "exclusiveMaximum",
        "multipleOf",
        "maxItems",
        "uniqueItems",
        "minContains",
        "maxContains",
        "minProperties",
        "maxProperties",
    ];
    const FORMATS: [&str; 10] = [
        "date-time",
        "time",
        "date",
        "duration",
        "email",
        "hostname",
        "uri",
        "ipv4",
        "ipv6",
        "uuid",
    ];
    if KEYWORDS.contains(&key) {
        return true;
    }
    if key == "minItems" {
        return value != &json!(0) && value != &json!(1);
    }
    if key == "format" {
        return !value.as_str().is_some_and(|f| FORMATS.contains(&f));
    }
    false
}

struct ToolConverter<'a> {
    is_oauth: bool,
    compat: &'a AnthropicCompat,
    disable_preferred_strict: bool,
    has_preferred_strict: bool,
}

impl ToolConverter<'_> {
    /// pi `convertTools`.
    fn convert(
        &mut self,
        tools: &[ToolDecl],
        cache_control: Option<&Value>,
    ) -> Result<Vec<Value>, String> {
        let mut out = Vec::with_capacity(tools.len());
        for (index, tool) in tools.iter().enumerate() {
            let strict = if self.disable_preferred_strict && prefers_strict(tool) {
                None
            } else {
                resolve_json_schema_strict(
                    tool,
                    self.compat.strict_tools,
                    Some(strict_unsupported_keyword),
                )?
            };
            let strict = strict == Some(true);
            let parameters = json_schema_tool_parameters(tool, strict)?;
            let legacy = json!({
                "type": "object",
                "properties": parameters.get("properties").cloned().unwrap_or_else(|| json!({})),
                "required": parameters.get("required").cloned().unwrap_or_else(|| json!([])),
            });
            let input_schema = if strict {
                let mut merged = parameters.as_object().cloned().unwrap_or_default();
                for (key, value) in legacy.as_object().into_iter().flatten() {
                    merged.insert(key.clone(), value.clone());
                }
                Value::Object(merged)
            } else {
                legacy
            };
            let mut definition = JsonObject::new();
            definition.insert(
                "name".into(),
                json!(if self.is_oauth {
                    to_claude_code_name(&tool.name)
                } else {
                    tool.name.clone()
                }),
            );
            definition.insert("description".into(), json!(tool.description));
            if self.compat.eager_tool_input_streaming {
                definition.insert("eager_input_streaming".into(), json!(true));
            }
            if strict {
                definition.insert("strict".into(), json!(true));
                if prefers_strict(tool) {
                    self.has_preferred_strict = true;
                }
            }
            definition.insert("input_schema".into(), input_schema);
            if let Some(cache) = cache_control.filter(|_| index == tools.len() - 1) {
                definition.insert("cache_control".into(), cache.clone());
            }
            out.push(Value::Object(definition));
        }
        Ok(out)
    }
}

fn is_anthropic_effort(value: &str) -> bool {
    matches!(value, "low" | "medium" | "high" | "xhigh" | "max")
}

struct Converted {
    messages: Vec<Value>,
    assistant_levels: BTreeMap<usize, String>,
}

/// pi `convertMessages`.
#[allow(clippy::too_many_lines, reason = "1:1 port of pi convertMessages")]
fn convert_messages(
    messages: &[Message],
    is_oauth: bool,
    cache_control: Option<&Value>,
    allow_empty_signature: bool,
    managed_provider: Option<&str>,
    mut tools: Option<&mut ToolConverter<'_>>,
) -> Result<Converted, String> {
    let mut params: Vec<Value> = Vec::new();
    let mut levels = BTreeMap::new();
    let mut pending_system: Vec<Value> = Vec::new();
    let mut index = 0;
    while index < messages.len() {
        match &messages[index] {
            Message::System(system) => {
                let text = render_system_message_update(system);
                let mut blocks = Vec::new();
                if !text.is_empty() {
                    blocks.push(json!({"type": "text", "text": text}));
                }
                if let Some(converter) = tools.as_deref_mut() {
                    let added = system.tools_added.clone().unwrap_or_default();
                    let redefined: HashSet<&str> = added.iter().map(|t| t.name.as_str()).collect();
                    for tool in system.tools_removed.iter().flatten() {
                        if redefined.contains(tool.name.as_str()) {
                            continue;
                        }
                        let name = if is_oauth {
                            to_claude_code_name(&tool.name)
                        } else {
                            tool.name.clone()
                        };
                        blocks.push(json!({"type": "tool_removal", "tool": {"type": "tool_reference", "name": name}}));
                    }
                    for definition in converter.convert(&added, None)? {
                        blocks.push(json!({"type": "tool_addition", "tool": {"type": "tool_definition", "definition": definition}}));
                    }
                }
                if !blocks.is_empty() {
                    pending_system.push(json!({"role": "system", "content": blocks}));
                }
            }
            Message::User(user) => match &user.content {
                UserContentField::Text(text) => {
                    if !text.trim().is_empty() {
                        params.push(json!({"role": "user", "content": text}));
                    }
                }
                UserContentField::Blocks(blocks) => {
                    let converted: Vec<Value> = blocks
                        .iter()
                        .filter(|b| match b {
                            UserContent::Text(t) => !t.text.trim().is_empty(),
                            UserContent::Image(_) => true,
                        })
                        .map(|block| match block {
                            UserContent::Text(t) => json!({"type": "text", "text": t.text}),
                            UserContent::Image(i) => image_block(&i.data, &i.mime_type),
                        })
                        .collect();
                    if !converted.is_empty() {
                        params.push(json!({"role": "user", "content": converted}));
                    }
                }
            },
            Message::Assistant(assistant) => {
                params.append(&mut pending_system);
                let blocks = assistant_blocks(assistant, is_oauth, allow_empty_signature);
                if !blocks.is_empty() {
                    let message_index = params.len();
                    params.push(json!({"role": "assistant", "content": blocks}));
                    if let (Some(provider), Some(level)) = (
                        managed_provider,
                        assistant.provider_thinking_level.as_deref(),
                    ) && assistant.api == "anthropic-messages"
                        && assistant.provider == provider
                        && is_anthropic_effort(level)
                    {
                        levels.insert(message_index, level.to_owned());
                    }
                }
            }
            Message::ToolResult(_) => {
                let mut results = Vec::new();
                let mut error_images = Vec::new();
                while let Some(Message::ToolResult(result)) = messages.get(index) {
                    match split_error_tool_result_images(result) {
                        Some(split) => {
                            results.push(convert_tool_result(
                                &result.tool_call_id,
                                &split.content,
                                result.is_error,
                            ));
                            error_images.push(json!({"type": "text", "text": split.caption}));
                            for image in &split.images {
                                if let UserContent::Image(i) = image {
                                    error_images.push(image_block(&i.data, &i.mime_type));
                                }
                            }
                        }
                        None => results.push(convert_tool_result(
                            &result.tool_call_id,
                            &result.content,
                            result.is_error,
                        )),
                    }
                    index += 1;
                }
                results.extend(error_images);
                params.push(json!({"role": "user", "content": results}));
                continue;
            }
        }
        index += 1;
    }
    params.append(&mut pending_system);
    if let (Some(cache), Some(last)) = (cache_control, params.last_mut())
        && matches!(last["role"].as_str(), Some("user" | "system"))
    {
        match &mut last["content"] {
            Value::Array(blocks) => {
                if let Some(block) = blocks.last_mut()
                    && matches!(
                        block["type"].as_str(),
                        Some("text" | "image" | "tool_result" | "tool_addition" | "tool_removal")
                    )
                {
                    block["cache_control"] = cache.clone();
                }
            }
            Value::String(text) => {
                let text = std::mem::take(text);
                last["content"] = json!([{"type": "text", "text": text, "cache_control": cache}]);
            }
            _ => {}
        }
    }
    Ok(Converted {
        messages: params,
        assistant_levels: levels,
    })
}

fn assistant_blocks(
    assistant: &AssistantMessage,
    is_oauth: bool,
    allow_empty_signature: bool,
) -> Vec<Value> {
    let mut blocks = Vec::new();
    for block in &assistant.content {
        match block {
            AssistantContent::Text(text) => {
                if !text.text.trim().is_empty() {
                    blocks.push(json!({"type": "text", "text": text.text}));
                }
            }
            AssistantContent::Thinking(thinking) => {
                if thinking.redacted == Some(true) {
                    blocks.push(
                        json!({"type": "redacted_thinking", "data": thinking.thinking_signature}),
                    );
                    continue;
                }
                let signature = thinking
                    .thinking_signature
                    .as_deref()
                    .filter(|s| !s.trim().is_empty());
                if thinking.thinking.trim().is_empty() && signature.is_none() {
                    continue;
                }
                match signature {
                    None if allow_empty_signature => blocks.push(
                        json!({"type": "thinking", "thinking": thinking.thinking, "signature": ""}),
                    ),
                    None => blocks.push(json!({"type": "text", "text": thinking.thinking})),
                    Some(signature) => blocks.push(
                        json!({"type": "thinking", "thinking": thinking.thinking, "signature": signature}),
                    ),
                }
            }
            AssistantContent::ToolCall(call) => {
                let name = if is_oauth {
                    to_claude_code_name(&call.name)
                } else {
                    call.name.clone()
                };
                blocks.push(json!({"type": "tool_use", "id": call.id, "name": name, "input": call.arguments}));
            }
        }
    }
    blocks
}

fn insert_thinking_level_messages(converted: Converted, active: &str) -> Vec<Value> {
    let mut messages = Vec::new();
    for (index, message) in converted.messages.into_iter().enumerate() {
        if let Some(level) = converted.assistant_levels.get(&index) {
            messages
                .push(json!({"role": "system", "content": [], "output_config": {"effort": level}}));
        }
        messages.push(message);
    }
    messages.push(json!({"role": "system", "content": [], "output_config": {"effort": active}}));
    messages
}

/// A configured `anthropic-beta` header (model or request headers).
enum ConfiguredBetas {
    /// No header: pi derives the betas.
    Unset,
    /// `null` override: no betas at all.
    Disabled,
    /// Explicit comma-separated list.
    List(String),
}

fn configured_betas(model: &Model, overrides: &[(String, Option<String>)]) -> ConfiguredBetas {
    let mut configured = ConfiguredBetas::Unset;
    for (name, value) in model.headers.iter().flatten() {
        if name.eq_ignore_ascii_case("anthropic-beta") {
            configured = ConfiguredBetas::List(value.clone());
        }
    }
    for (name, value) in overrides {
        if name.eq_ignore_ascii_case("anthropic-beta") {
            configured = value
                .clone()
                .map_or(ConfiguredBetas::Disabled, ConfiguredBetas::List);
        }
    }
    configured
}

fn beta_features(
    model: &Model,
    context: &Context,
    is_oauth: bool,
    native_tool_changes: bool,
    options: &Options,
    overrides: &[(String, Option<String>)],
) -> Vec<String> {
    let mut features: Vec<String> = Vec::new();
    match configured_betas(model, overrides) {
        ConfiguredBetas::Disabled => return features,
        ConfiguredBetas::List(configured) => {
            for feature in configured
                .split(',')
                .map(str::trim)
                .filter(|f| !f.is_empty())
            {
                if !features.iter().any(|f| f == feature) {
                    features.push(feature.to_owned());
                }
            }
            return features;
        }
        ConfiguredBetas::Unset => {}
    }
    let compat = compat(model);
    let mut push = |feature: &str| {
        if !features.iter().any(|f| f == feature) {
            features.push(feature.to_owned());
        }
    };
    if is_oauth {
        push("claude-code-20250219");
        push("oauth-2025-04-20");
    }
    if !current_tools(&context.messages).is_empty() && !compat.eager_tool_input_streaming {
        push(FINE_GRAINED_TOOL_STREAMING_BETA);
    }
    if model.reasoning
        && options.thinking_enabled
        && options.interleaved_thinking.unwrap_or(true)
        && compat_bool(model, "forceAdaptiveThinking") != Some(true)
    {
        push(INTERLEAVED_THINKING_BETA);
    }
    if fallback_models(model).is_some_and(|f| !f.is_empty()) {
        push(SERVER_SIDE_FALLBACK_BETA);
    }
    if compat_bool(model, "supportsMidConvoEffort") == Some(true) {
        push(MID_CONVERSATION_OUTPUT_CONFIG_BETA);
        push(THINKING_BINDING_CONTROLS_BETA);
    }
    if native_tool_changes {
        push(INLINE_TOOLS_BETA);
    }
    features
}

fn fallback_models(model: &Model) -> Option<&Vec<Value>> {
    compat_value(model, "allowedFallbackModels").and_then(Value::as_array)
}

struct Built {
    params: Value,
    has_preferred_strict_tools: bool,
    provider_thinking_level: Option<String>,
}

/// pi `buildParams`.
#[allow(clippy::too_many_lines, reason = "1:1 port of pi buildParams")]
fn build_params(
    request: &ProviderRequest,
    context: &Context,
    is_oauth: bool,
    options: &Options,
    disable_preferred_strict: bool,
    now: i64,
) -> Result<Built, String> {
    let model = &request.model;
    let cache = cache_control(model, options.retention);
    let compat = compat(model);
    let initial = match context.messages.first() {
        Some(Message::System(system)) => Some(system),
        _ => None,
    };
    let initial_text = initial.map(system_message_text).unwrap_or_default();
    let transformed = transform_messages(
        context.messages.clone(),
        model,
        Some(&|id: &str, _: &Model, _: &AssistantMessage| normalize_tool_call_id(id)),
        now,
    );
    let conversation = if initial.is_some() {
        &transformed[1..]
    } else {
        &transformed[..]
    };
    let mut tool_converter = ToolConverter {
        is_oauth,
        compat: &compat,
        disable_preferred_strict,
        has_preferred_strict: false,
    };
    let initial_tools = initial
        .and_then(|s| s.tools_added.clone())
        .unwrap_or_default();
    let native_tool_changes = compat.mid_convo_system_messages
        && compat.mid_convo_tool_changes
        && !initial_tools.is_empty();
    let thinking_disabled = model.reasoning
        && !options.thinking_enabled
        && mapped_level(model, ThinkingLevel::Off) != MapEntry::Null;
    let effort_messages =
        compat_bool(model, "supportsMidConvoEffort") == Some(true) && !thinking_disabled;
    let converted = convert_messages(
        conversation,
        is_oauth,
        cache.as_ref(),
        compat.allow_empty_signature,
        effort_messages.then_some(model.provider.as_str()),
        native_tool_changes.then_some(&mut tool_converter),
    )?;
    let active_effort = options.effort.clone().unwrap_or_else(|| "high".to_owned());
    let betas = beta_features(
        model,
        context,
        is_oauth,
        native_tool_changes,
        options,
        &request.header_overrides(),
    );
    let mut params = JsonObject::new();
    params.insert("model".into(), json!(model.id));
    params.insert(
        "messages".into(),
        Value::Array(if effort_messages {
            insert_thinking_level_messages(converted, &active_effort)
        } else {
            converted.messages
        }),
    );
    params.insert("max_tokens".into(), json!(options.max_tokens));
    params.insert("stream".into(), json!(true));
    if !betas.is_empty() {
        params.insert("betas".into(), json!(betas));
    }
    let with_cache = |text: &str| {
        let mut block = json!({"type": "text", "text": text});
        if let Some(cache) = &cache {
            block["cache_control"] = cache.clone();
        }
        block
    };
    if is_oauth {
        let mut system = vec![with_cache(
            "You are Claude Code, Anthropic's official CLI for Claude.",
        )];
        if !initial_text.is_empty() {
            system.push(with_cache(&initial_text));
        }
        params.insert("system".into(), Value::Array(system));
    } else if !initial_text.is_empty() {
        params.insert("system".into(), json!([with_cache(&initial_text)]));
    }
    if let Some(temperature) = options.temperature
        && !options.thinking_enabled
        && compat_bool(model, "supportsMidConvoEffort") != Some(true)
        && compat.temperature
    {
        params.insert("temperature".into(), json!(temperature));
    }
    let tool_cache = cache.as_ref().filter(|_| compat.cache_control_on_tools);
    if native_tool_changes {
        let mut tools = tool_converter.convert(&initial_tools, tool_cache)?;
        tools.push(deferred_tool_placeholder());
        params.insert("tools".into(), Value::Array(tools));
    } else {
        let tools = current_tools(&context.messages);
        if !tools.is_empty() {
            params.insert(
                "tools".into(),
                Value::Array(tool_converter.convert(&tools, tool_cache)?),
            );
        }
    }
    let display = options
        .thinking_display
        .clone()
        .unwrap_or_else(|| "summarized".to_owned());
    if thinking_disabled {
        params.insert("thinking".into(), json!({"type": "disabled"}));
    } else if effort_messages {
        params.insert(
            "thinking".into(),
            json!({"type": "adaptive", "display": display, "block_binding": {"prefix_mismatch_behavior": "drop_block"}}),
        );
        params.insert("output_config".into(), json!({"effort": "high"}));
    } else if model.reasoning && options.thinking_enabled {
        if compat_bool(model, "forceAdaptiveThinking") == Some(true) {
            params.insert(
                "thinking".into(),
                json!({"type": "adaptive", "display": display}),
            );
            if let Some(effort) = &options.effort {
                params.insert("output_config".into(), json!({"effort": effort}));
            }
        } else {
            let budget = options
                .thinking_budget_tokens
                .filter(|b| *b > 0)
                .unwrap_or(1024);
            params.insert(
                "thinking".into(),
                json!({"type": "enabled", "budget_tokens": budget, "display": display}),
            );
        }
    }
    if let Some(Value::String(user_id)) = options.metadata.as_ref().and_then(|m| m.get("user_id")) {
        params.insert("metadata".into(), json!({"user_id": user_id}));
    }
    match &options.tool_choice {
        Some(Value::String(choice)) if !choice.is_empty() => {
            params.insert("tool_choice".into(), json!({"type": choice}));
        }
        Some(choice) if !choice.is_null() && !matches!(choice, Value::String(_)) => {
            params.insert("tool_choice".into(), choice.clone());
        }
        _ => {}
    }
    if let Some(fallbacks) = fallback_models(model).filter(|f| !f.is_empty()) {
        let list: Vec<Value> = fallbacks
            .iter()
            .map(|f| json!({"model": f.get("model")}))
            .collect();
        params.insert("fallbacks".into(), Value::Array(list));
    }
    Ok(Built {
        params: Value::Object(params),
        has_preferred_strict_tools: tool_converter.has_preferred_strict,
        provider_thinking_level: effort_messages.then_some(active_effort),
    })
}

/// A prepared Anthropic request.
pub struct Prepared {
    /// Outbound request (payload as pi's `onPayload` sees it).
    pub http: HttpRequest,
    /// Whether a strict-limit 400 may be retried without preferred strict tools.
    pub has_preferred_strict_tools: bool,
    /// Streaming state.
    pub exchange: Exchange,
}

fn has_request_auth(request: &ProviderRequest) -> bool {
    request.api_key.as_deref().is_some_and(|k| !k.is_empty())
        || request.has_header("authorization")
        || request.has_header("x-api-key")
        || request.has_header("cf-aig-authorization")
}

/// Builds the request (pi `streamSimple` + `stream` up to the HTTP call).
///
/// # Errors
/// Missing credentials or a tool that cannot be expressed.
pub fn prepare(
    request: &ProviderRequest,
    disable_preferred_strict: bool,
    now: i64,
) -> Result<Prepared, String> {
    let model = &request.model;
    if !has_request_auth(request) {
        return Err(format!("No API key for provider: {}", model.provider));
    }
    let compat = compat(model);
    let context = if compat.mid_convo_system_messages {
        request.context.clone()
    } else {
        collapse_system_messages(request.context.clone())
    };
    let options = simple_options(request);
    let api_key = request.api_key.clone().unwrap_or_default();
    let is_oauth = !api_key.is_empty() && is_oauth_token(&api_key);
    let built = build_params(
        request,
        &context,
        is_oauth,
        &options,
        disable_preferred_strict,
        now,
    )?;
    let mut headers = Headers::new();
    headers.set("accept", "application/json");
    headers.set("anthropic-version", "2023-06-01");
    headers.set("content-type", "application/json");
    if is_oauth {
        headers.set("authorization", format!("Bearer {api_key}"));
        headers.set("anthropic-dangerous-direct-browser-access", "true");
        headers.set("user-agent", format!("claude-cli/{CLAUDE_CODE_VERSION}"));
        headers.set("x-app", "cli");
    } else {
        if !api_key.is_empty() {
            headers.set("x-api-key", &api_key);
        }
        headers.set("user-agent", USER_AGENT);
        headers.set("anthropic-dangerous-direct-browser-access", "true");
        let session = options
            .session_id
            .as_deref()
            .filter(|_| options.retention != CacheRetention::None);
        if let (Some(session), true) = (session, compat.session_affinity_headers) {
            let name = if compat.session_affinity_format.as_deref() == Some("openrouter") {
                "x-session-id"
            } else {
                "x-session-affinity"
            };
            headers.set(name, session);
        }
    }
    for (name, value) in model.headers.iter().flatten() {
        headers.set(name, value);
    }
    for (name, value) in request.header_overrides() {
        headers.apply(&name, value.as_deref());
    }
    let mut out = Emitter::new(model, now);
    if let Some(level) = &built.provider_thinking_level {
        out.message.provider_thinking_level = Some(level.clone());
    }
    let parser = Parser {
        model: model.clone(),
        usage_cost: model.cost.clone(),
        is_oauth,
        tools: current_tools(&context.messages),
        slots: Vec::new(),
        saw_start: false,
        saw_stop: false,
        input_transformations: None,
    };
    Ok(Prepared {
        http: HttpRequest {
            url: join_url(&model.base_url, "/v1/messages?beta=true"),
            headers,
            payload: built.params,
            timeout_ms: request.options.timeout_ms,
        },
        has_preferred_strict_tools: built.has_preferred_strict_tools,
        exchange: Exchange::new(out, Box::new(parser), ErrorStyle::Anthropic),
    })
}

/// Turns a (possibly hook-replaced) payload into the wire body: `stream: true`, and `betas`
/// moved into the `anthropic-beta` header like the SDK.
#[must_use]
pub fn finalize(mut payload: Value, headers: &mut Headers) -> Value {
    if let Some(object) = payload.as_object_mut() {
        object.insert("stream".into(), json!(true));
        if let Some(betas) = object.shift_remove("betas") {
            let list: Vec<String> = match betas {
                Value::Array(list) => list
                    .iter()
                    .map(|v| v.as_str().map_or_else(|| v.to_string(), str::to_owned))
                    .collect(),
                Value::String(text) => vec![text],
                _ => Vec::new(),
            };
            if !list.is_empty() {
                headers.set("anthropic-beta", list.join(","));
            }
        }
    }
    payload
}

/// Whether a 400 response is one of the strict-tool limit errors pi retries without
/// preferred strict tools.
#[must_use]
pub fn is_strict_limit_error(status: u16, body: &str) -> bool {
    if status != 400 {
        return false;
    }
    let json: Value = serde_json::from_str(body).unwrap_or(Value::Null);
    let message = json
        .pointer("/error/message")
        .and_then(Value::as_str)
        .unwrap_or(body);
    json.pointer("/error/type").and_then(Value::as_str) == Some("invalid_request_error")
        && STRICT_LIMIT_MESSAGES.iter().any(|m| message.contains(m))
}

/// Diagnostic recorded when retrying without preferred strict tools.
#[must_use]
pub fn strict_fallback_diagnostic(message: &str, now: i64) -> AssistantDiagnostic {
    let mut details = JsonObject::new();
    details.insert("reason".into(), json!("strict_tool_limits_exceeded"));
    AssistantDiagnostic {
        kind: "anthropic_strict_tools_fallback".into(),
        timestamp: now,
        error: Some(DiagnosticError {
            name: None,
            message: message.to_owned(),
            stack: None,
            code: None,
        }),
        details: Some(details),
    }
}

/// Per-block scratch state (pi `block.index`, `block.partialJson`).
struct Slot {
    index: i64,
    partial_json: Option<String>,
    open: bool,
}

struct Parser {
    model: Model,
    usage_cost: ModelCost,
    is_oauth: bool,
    tools: Vec<ToolDecl>,
    slots: Vec<Slot>,
    saw_start: bool,
    saw_stop: bool,
    input_transformations: Option<Vec<Value>>,
}

const MESSAGE_EVENTS: [&str; 6] = [
    "message_start",
    "message_delta",
    "message_stop",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
];

fn tokens(value: &Value) -> u64 {
    value.as_u64().unwrap_or(0)
}

impl Parser {
    fn position(&self, index: i64) -> Option<usize> {
        self.slots.iter().position(|s| s.open && s.index == index)
    }

    fn recompute(&self, out: &mut Emitter) {
        let usage = &mut out.message.usage;
        usage.total_tokens = usage.input + usage.output + usage.cache_read + usage.cache_write;
        let mut model = self.model.clone();
        model.cost = self.usage_cost.clone();
        calculate_cost(&model, usage);
    }

    fn message_start(&mut self, event: &Value, out: &mut Emitter) {
        let message = &event["message"];
        out.message.response_id = message["id"].as_str().map(str::to_owned);
        if let Some(list) = message["input_transformations"].as_array() {
            self.input_transformations = Some(list.clone());
        }
        let response_model = message["model"].as_str();
        if let Some(response_model) = response_model.filter(|m| *m != self.model.id) {
            out.message.response_model = Some(response_model.to_owned());
        }
        if response_model.is_some_and(|m| m != self.model.id) {
            let fallback = fallback_models(&self.model).and_then(|list| {
                list.iter().find(|f| {
                    f["provider"].as_str() == Some(self.model.provider.as_str())
                        && f["model"].as_str() == response_model
                })
            });
            if let Some(cost) = fallback
                .and_then(|f| f.get("cost"))
                .and_then(|c| serde_json::from_value::<ModelCost>(c.clone()).ok())
            {
                self.usage_cost = cost;
            }
        }
        let usage = &message["usage"];
        let u = &mut out.message.usage;
        u.input = tokens(&usage["input_tokens"]);
        u.output = tokens(&usage["output_tokens"]);
        u.cache_read = tokens(&usage["cache_read_input_tokens"]);
        u.cache_write = tokens(&usage["cache_creation_input_tokens"]);
        u.cache_write_1h = Some(tokens(
            &usage["cache_creation"]["ephemeral_1h_input_tokens"],
        ));
        self.recompute(out);
    }

    fn block_start(&mut self, event: &Value, out: &mut Emitter) -> Result<(), StreamError> {
        let block = &event["content_block"];
        let index = event["index"].as_i64().unwrap_or(-1);
        let slot = |partial_json| Slot {
            index,
            partial_json,
            open: true,
        };
        match block["type"].as_str() {
            Some("fallback") => {
                if !out.message.content.is_empty() {
                    return Err(
                        "Anthropic performed an unsupported mid-output model fallback".into(),
                    );
                }
            }
            Some("text") => {
                let at = out.add(AssistantContent::Text(TextContent::new(
                    block["text"].as_str().unwrap_or_default(),
                )));
                self.slots.push(slot(None));
                out.push(ProviderEvent::TextStart { content_index: at });
            }
            Some("thinking") => {
                let at = out.add(AssistantContent::Thinking(ThinkingContent {
                    thinking: block["thinking"].as_str().unwrap_or_default().to_owned(),
                    thinking_signature: Some(
                        block["signature"].as_str().unwrap_or_default().to_owned(),
                    ),
                    redacted: None,
                }));
                self.slots.push(slot(None));
                out.push(ProviderEvent::ThinkingStart { content_index: at });
            }
            Some("redacted_thinking") => {
                let at = out.add(AssistantContent::Thinking(ThinkingContent {
                    thinking: "[Reasoning redacted]".to_owned(),
                    thinking_signature: block["data"].as_str().map(str::to_owned),
                    redacted: Some(true),
                }));
                self.slots.push(slot(None));
                out.push(ProviderEvent::ThinkingStart { content_index: at });
            }
            Some("tool_use") => {
                let raw_name = block["name"].as_str().unwrap_or_default();
                let name = if self.is_oauth {
                    from_claude_code_name(raw_name, &self.tools)
                } else {
                    raw_name.to_owned()
                };
                let id = block["id"].as_str().unwrap_or_default().to_owned();
                let at = out.add(AssistantContent::ToolCall(ToolCall {
                    id: id.clone(),
                    name: name.clone(),
                    arguments: block["input"].as_object().cloned().unwrap_or_default(),
                    ..ToolCall::default()
                }));
                self.slots.push(slot(Some(String::new())));
                out.push(ProviderEvent::ToolcallStart {
                    content_index: at,
                    id: Some(id),
                    tool_name: Some(name),
                });
            }
            _ => {}
        }
        Ok(())
    }

    fn block_delta(&mut self, event: &Value, out: &mut Emitter) {
        let Some(at) = self.position(event["index"].as_i64().unwrap_or(-1)) else {
            return;
        };
        let delta = &event["delta"];
        match (delta["type"].as_str(), out.block(at)) {
            (Some("text_delta"), Some(AssistantContent::Text(block))) => {
                let text = delta["text"].as_str().unwrap_or_default();
                block.text.push_str(text);
                out.push(ProviderEvent::TextDelta {
                    content_index: at,
                    delta: text.to_owned(),
                });
            }
            (Some("thinking_delta"), Some(AssistantContent::Thinking(block))) => {
                let text = delta["thinking"].as_str().unwrap_or_default();
                block.thinking.push_str(text);
                out.push(ProviderEvent::ThinkingDelta {
                    content_index: at,
                    delta: text.to_owned(),
                });
            }
            (Some("input_json_delta"), Some(AssistantContent::ToolCall(call))) => {
                let text = delta["partial_json"].as_str().unwrap_or_default();
                let buffer = self.slots[at].partial_json.get_or_insert_with(String::new);
                buffer.push_str(text);
                call.arguments = parse_streaming_object(buffer);
                out.push(ProviderEvent::ToolcallDelta {
                    content_index: at,
                    delta: text.to_owned(),
                });
            }
            (Some("signature_delta"), Some(AssistantContent::Thinking(block))) => {
                block
                    .thinking_signature
                    .get_or_insert_with(String::new)
                    .push_str(delta["signature"].as_str().unwrap_or_default());
            }
            _ => {}
        }
    }

    fn block_stop(&mut self, event: &Value, out: &mut Emitter) {
        let Some(at) = self.position(event["index"].as_i64().unwrap_or(-1)) else {
            return;
        };
        self.slots[at].open = false;
        let partial = self.slots[at].partial_json.take().unwrap_or_default();
        match out.block(at) {
            Some(AssistantContent::Text(block)) => {
                let content = block.text.clone();
                out.push(ProviderEvent::TextEnd {
                    content_index: at,
                    content,
                });
            }
            Some(AssistantContent::Thinking(block)) => {
                let content = block.thinking.clone();
                out.push(ProviderEvent::ThinkingEnd {
                    content_index: at,
                    content,
                });
            }
            Some(AssistantContent::ToolCall(call)) => {
                let parsed = parse_tool_call_arguments(&partial);
                call.arguments = parsed.arguments;
                call.arguments_error = parsed.error;
                let tool_call = call.clone();
                out.push(ProviderEvent::ToolcallEnd {
                    content_index: at,
                    tool_call,
                });
            }
            None => {}
        }
    }

    fn message_delta(&mut self, event: &Value, out: &mut Emitter) -> Result<(), StreamError> {
        if let Some(list) = event["input_transformations"].as_array() {
            self.input_transformations = Some(list.clone());
        }
        if let Some(reason) = event["delta"]["stop_reason"]
            .as_str()
            .filter(|r| !r.is_empty())
        {
            out.message.raw_stop_reason = Some(reason.to_owned());
            let (stop, message) = map_stop_reason(reason, &event["delta"]["stop_details"])?;
            out.message.stop_reason = stop;
            if let Some(message) = message {
                out.message.error_message = Some(message);
            }
        }
        let usage = &event["usage"];
        if usage.is_object() {
            let u = &mut out.message.usage;
            if let Some(v) = usage["input_tokens"].as_u64() {
                u.input = v;
            }
            if let Some(v) = usage["output_tokens"].as_u64() {
                u.output = v;
            }
            if let Some(v) = usage["cache_read_input_tokens"].as_u64() {
                u.cache_read = v;
            }
            if let Some(v) = usage["cache_creation_input_tokens"].as_u64() {
                u.cache_write = v;
            }
            if let Some(v) = usage["cache_creation"]["ephemeral_1h_input_tokens"].as_u64() {
                u.cache_write_1h = Some(v);
            }
            if let Some(v) = usage["output_tokens_details"]["thinking_tokens"].as_u64() {
                u.reasoning = Some(v);
            }
        }
        self.recompute(out);
        Ok(())
    }
}

fn map_stop_reason(
    reason: &str,
    details: &Value,
) -> Result<(StopReason, Option<String>), StreamError> {
    Ok(match reason {
        "end_turn" | "pause_turn" | "stop_sequence" => (StopReason::Stop, None),
        "max_tokens" => (StopReason::Length, None),
        "tool_use" => (StopReason::ToolUse, None),
        "refusal" => (
            StopReason::Error,
            Some(
                details["explanation"]
                    .as_str()
                    .filter(|e| !e.is_empty())
                    .unwrap_or("The model refused to complete the request")
                    .to_owned(),
            ),
        ),
        "sensitive" => (
            StopReason::Error,
            Some("Provider stopped with: sensitive".to_owned()),
        ),
        other => return Err(format!("Unhandled stop reason: {other}").into()),
    })
}

impl StreamParser for Parser {
    fn event(&mut self, sse: &SseEvent, out: &mut Emitter) -> Result<Step, StreamError> {
        let name = sse.event.as_deref().unwrap_or_default();
        if name == "error" {
            return Err(sse.data.clone().into());
        }
        if !MESSAGE_EVENTS.contains(&name) {
            return Ok(Step::Skip);
        }
        let event = parse_json_with_repair(&sse.data).map_err(|error| {
            StreamError::from(format!(
                "Could not parse Anthropic SSE event {name}: {error}; data={}; raw=event: {name}\\ndata: {}",
                sse.data, sse.data
            ))
        })?;
        match event["type"].as_str() {
            Some("message_start") => {
                self.saw_start = true;
                self.message_start(&event, out);
            }
            Some("message_stop") => self.saw_stop = true,
            Some("content_block_start") => self.block_start(&event, out)?,
            Some("content_block_delta") => self.block_delta(&event, out),
            Some("content_block_stop") => self.block_stop(&event, out),
            Some("message_delta") => self.message_delta(&event, out)?,
            _ => {}
        }
        Ok(Step::Event(event))
    }

    fn finish(&mut self, out: &mut Emitter) -> Result<(), StreamError> {
        if self.saw_start && !self.saw_stop {
            return Err("Anthropic stream ended before message_stop".into());
        }
        match out.message.stop_reason {
            StopReason::Pending => {
                return Err("Anthropic stream ended without a stop reason".into());
            }
            StopReason::Aborted | StopReason::Error => {
                return Err(out
                    .message
                    .error_message
                    .clone()
                    .unwrap_or_else(|| "An unknown error occurred".to_owned())
                    .into());
            }
            _ => {}
        }
        if let Some(list) = self.input_transformations.take().filter(|l| !l.is_empty()) {
            let transformations: Vec<Value> = list
                .iter()
                .map(|t| {
                    let mut entry = JsonObject::new();
                    for key in ["type", "path", "reason"] {
                        if let Some(value) = t.get(key).filter(|v| !v.is_null()) {
                            entry.insert(key.into(), value.clone());
                        }
                    }
                    Value::Object(entry)
                })
                .collect();
            let mut details = JsonObject::new();
            details.insert("transformations".into(), Value::Array(transformations));
            out.diagnostic(AssistantDiagnostic {
                kind: "anthropic_input_transformations".into(),
                timestamp: out.message.timestamp,
                error: None,
                details: Some(details),
            });
        }
        Ok(())
    }
}
