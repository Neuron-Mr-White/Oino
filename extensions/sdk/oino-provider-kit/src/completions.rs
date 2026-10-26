//! `OpenAI` chat completions (pi-ai `api/openai-completions.js`): compat detection, request
//! params with cache hints, headers and the chunk stream parser.

use std::collections::BTreeMap;

use oino_ai::transcript::{
    ToolChangeSupport, collapse_system_messages, declared_tools, render_system_message_update,
    resolve_transcript_tools, system_message_text,
};
use oino_ai::transform::transform_messages;
use oino_ai::{
    AssistantContent, AssistantMessage, CacheRetention, Context, InputModality, JsonObject,
    MapEntry, Message, Model, ProviderEvent, StopReason, TextContent, ThinkingContent,
    ThinkingLevel, ToolCall, ToolDecl, UserContent, UserContentField,
};
use serde_json::{Value, json};

use crate::constrained::{
    GrammarBuffer, append_grammar_delta, grammar_input_properties, grammar_tool_input,
    json_schema_tool_parameters, resolve_grammar, resolve_json_schema_strict,
};
use crate::emit::Emitter;
use crate::errors::openai_stream_error;
use crate::exchange::{ErrorStyle, Exchange, Step, StreamError, StreamParser, has_tool_call};
use crate::json::{parse_streaming_object, parse_tool_call_arguments};
use crate::options::{
    clamp_max_tokens_to_context, clamp_thinking_budget_to_answer_room, resolve_sampling_params,
    thinking_budget_for_level,
};
use crate::providers;
use crate::request::{Headers, HttpRequest, ProviderRequest, USER_AGENT, join_url};
use crate::sse::SseEvent;
use crate::util::{
    calculate_cost, clamp_prompt_cache_key, clamp_thinking_level, compat_value, mapped_level,
    mapped_or_level, off_not_null, short_hash,
};

/// pi `getCompat(model)` for chat completions.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors pi's compat flag record"
)]
#[derive(Clone, Debug, PartialEq)]
pub struct CompletionsCompat {
    /// `store: false`.
    pub supports_store: bool,
    /// `developer` instruction role.
    pub supports_developer_role: bool,
    /// `reasoning_effort`.
    pub supports_reasoning_effort: bool,
    /// `stream_options.include_usage`.
    pub supports_usage_in_streaming: bool,
    /// `finish_reason` is reliable.
    pub supports_finish_reason: bool,
    /// `max_tokens` or `max_completion_tokens`.
    pub max_tokens_field: String,
    /// `name` on tool results.
    pub requires_tool_result_name: bool,
    /// Synthetic assistant after tool results.
    pub requires_assistant_after_tool_result: bool,
    /// Replay thinking as text.
    pub requires_thinking_as_text: bool,
    /// `reasoning_content: ""` on assistant messages.
    pub requires_reasoning_content_on_assistant_messages: bool,
    /// Thinking parameter dialect.
    pub thinking_format: String,
    /// `OpenRouter` `provider` routing.
    pub open_router_routing: Value,
    /// Vercel gateway routing.
    pub vercel_gateway_routing: Value,
    /// `chat_template_kwargs` template.
    pub chat_template_kwargs: Value,
    /// Baseten `chat_template_args` template.
    pub chat_template_args: Value,
    /// z.ai `tool_stream`.
    pub zai_tool_stream: bool,
    /// `thinking_token_budget`.
    pub supports_thinking_token_budget: bool,
    /// Custom budget field.
    pub thinking_token_budget_field: Option<String>,
    /// Strict function tools.
    pub supports_strict_mode: bool,
    /// Grammar (custom) tools.
    pub supports_openai_grammar_tools: bool,
    /// Later system messages.
    pub supports_mid_convo_system_messages: bool,
    /// Later tool additions.
    pub supports_mid_convo_tool_additions: bool,
    /// `anthropic` cache-control dialect.
    pub cache_control_format: Option<String>,
    /// Session-affinity headers.
    pub send_session_affinity_headers: bool,
    /// `openrouter` or `openai`.
    pub session_affinity_format: String,
    /// Long cache retention.
    pub supports_long_cache_retention: bool,
    /// vLLM `priority`.
    pub vllm_priority: Option<Value>,
}

/// pi `detectCompat` + `getCompat`.
#[must_use]
#[allow(
    clippy::too_many_lines,
    reason = "1:1 port of pi detectCompat/getCompat"
)]
pub fn compat(model: &Model) -> CompletionsCompat {
    let provider = model.provider.as_str();
    let url = model.base_url.as_str();
    let is_zai = matches!(provider, "zai" | "zai-coding-cn")
        || url.contains("api.z.ai")
        || url.contains("open.bigmodel.cn");
    let is_together = provider == "together"
        || url.contains("api.together.ai")
        || url.contains("api.together.xyz");
    let is_moonshot =
        matches!(provider, "moonshotai" | "moonshotai-cn") || url.contains("api.moonshot.");
    let is_open_router = provider == "openrouter" || url.contains("openrouter.ai");
    let is_cf_workers = provider == "cloudflare-workers-ai" || url.contains("api.cloudflare.com");
    let is_cf_gateway =
        provider == "cloudflare-ai-gateway" || url.contains("gateway.ai.cloudflare.com");
    let is_nvidia = provider == "nvidia" || url.contains("integrate.api.nvidia.com");
    let is_ant_ling = provider == "ant-ling" || url.contains("api.ant-ling.com");
    let is_cerebras = provider == "cerebras" || url.contains("cerebras.ai");
    let is_deepseek = provider == "deepseek" || url.to_lowercase().contains("deepseek.com");
    let is_grok = provider == "xai" || url.contains("api.x.ai");
    let non_standard = is_nvidia
        || is_cerebras
        || is_grok
        || is_together
        || url.contains("chutes.ai")
        || is_deepseek
        || is_zai
        || is_moonshot
        || provider == "opencode"
        || url.contains("opencode.ai")
        || is_cf_workers
        || is_cf_gateway
        || is_ant_ling;
    let use_max_tokens = url.contains("chutes.ai")
        || is_deepseek
        || is_moonshot
        || is_cf_gateway
        || is_together
        || is_nvidia
        || is_ant_ling
        || is_zai;
    let or_developer =
        is_open_router && (model.id.starts_with("anthropic/") || model.id.starts_with("openai/"));
    let thinking_format = if is_deepseek {
        "deepseek"
    } else if is_zai {
        "zai"
    } else if is_together {
        "together"
    } else if is_ant_ling {
        "ant-ling"
    } else if is_open_router {
        "openrouter"
    } else {
        "openai"
    };
    let detected_cache_format = (provider == "openrouter" && model.id.starts_with("anthropic/"))
        .then(|| "anthropic".to_owned());
    let explicit = model.compat.as_ref();
    let flag = |key: &str, default: bool| {
        explicit
            .and_then(|c| c.get(key))
            .and_then(Value::as_bool)
            .unwrap_or(default)
    };
    let text = |key: &str| {
        explicit
            .and_then(|c| c.get(key))
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let object = |key: &str| {
        explicit
            .and_then(|c| c.get(key))
            .filter(|v| !v.is_null())
            .cloned()
            .unwrap_or_else(|| json!({}))
    };
    CompletionsCompat {
        supports_store: flag("supportsStore", !non_standard),
        supports_developer_role: flag(
            "supportsDeveloperRole",
            or_developer || (!non_standard && !is_open_router),
        ),
        supports_reasoning_effort: flag(
            "supportsReasoningEffort",
            !is_grok
                && !is_zai
                && !is_moonshot
                && !is_together
                && !is_cf_gateway
                && !is_nvidia
                && !is_ant_ling,
        ),
        supports_usage_in_streaming: flag("supportsUsageInStreaming", true),
        supports_finish_reason: flag("supportsFinishReason", true),
        max_tokens_field: text("maxTokensField").unwrap_or_else(|| {
            if use_max_tokens {
                "max_tokens"
            } else {
                "max_completion_tokens"
            }
            .to_owned()
        }),
        requires_tool_result_name: flag("requiresToolResultName", false),
        requires_assistant_after_tool_result: flag("requiresAssistantAfterToolResult", false),
        requires_thinking_as_text: flag("requiresThinkingAsText", false),
        requires_reasoning_content_on_assistant_messages: flag(
            "requiresReasoningContentOnAssistantMessages",
            is_deepseek,
        ),
        thinking_format: text("thinkingFormat").unwrap_or_else(|| thinking_format.to_owned()),
        open_router_routing: object("openRouterRouting"),
        vercel_gateway_routing: object("vercelGatewayRouting"),
        chat_template_kwargs: object("chatTemplateKwargs"),
        chat_template_args: object("chatTemplateArgs"),
        zai_tool_stream: flag("zaiToolStream", false),
        supports_thinking_token_budget: flag("supportsThinkingTokenBudget", false),
        thinking_token_budget_field: text("thinkingTokenBudgetField"),
        supports_strict_mode: flag("supportsStrictMode", false),
        supports_openai_grammar_tools: flag("supportsOpenAIGrammarTools", false),
        supports_mid_convo_system_messages: flag("supportsMidConvoSystemMessages", false),
        supports_mid_convo_tool_additions: flag("supportsMidConvoToolAdditions", false),
        cache_control_format: text("cacheControlFormat").or(detected_cache_format),
        send_session_affinity_headers: flag("sendSessionAffinityHeaders", is_open_router),
        session_affinity_format: text("sessionAffinityFormat").unwrap_or_else(|| {
            if is_open_router {
                "openrouter"
            } else {
                "openai"
            }
            .to_owned()
        }),
        supports_long_cache_retention: flag(
            "supportsLongCacheRetention",
            !(is_together || is_cf_workers || is_cf_gateway || is_nvidia || is_ant_ling),
        ),
        vllm_priority: explicit.and_then(|c| c.get("vllmPriority")).cloned(),
    }
}

fn tool_support(compat: &CompletionsCompat) -> ToolChangeSupport {
    if compat.supports_mid_convo_system_messages && compat.supports_mid_convo_tool_additions {
        ToolChangeSupport::Additive
    } else {
        ToolChangeSupport::None
    }
}

/// pi `convertTools`.
fn convert_tools(tools: &[ToolDecl], compat: &CompletionsCompat) -> Result<Vec<Value>, String> {
    tools
        .iter()
        .map(|tool| {
            if let Some(grammar) = resolve_grammar(tool, compat.supports_openai_grammar_tools)? {
                return Ok(json!({
                    "type": "custom",
                    "custom": {
                        "name": tool.name,
                        "description": tool.description,
                        "format": {"type": "grammar", "grammar": {"syntax": grammar.format, "definition": grammar.definition}},
                    },
                }));
            }
            let strict = resolve_json_schema_strict(tool, compat.supports_strict_mode, None)?;
            let mut function = JsonObject::new();
            function.insert("name".into(), json!(tool.name));
            function.insert("description".into(), json!(tool.description));
            function.insert(
                "parameters".into(),
                json_schema_tool_parameters(tool, strict == Some(true))?,
            );
            if compat.supports_strict_mode {
                function.insert("strict".into(), json!(strict.unwrap_or(false)));
            }
            Ok(json!({"type": "function", "function": function}))
        })
        .collect()
}

/// pi `normalizeToolCallId` for chat completions.
fn normalize_tool_call_id(model: &Model, id: &str) -> String {
    let sanitize = |part: &str| -> String {
        part.encode_utf16()
            .map(|unit| match char::from_u32(u32::from(unit)) {
                Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
                _ => '_',
            })
            .collect()
    };
    if let Some((call_id, item_id)) = id.split_once('|') {
        let call_id = sanitize(call_id);
        let item_id = sanitize(item_id);
        let combined = if item_id.is_empty() {
            call_id.clone()
        } else {
            format!("{call_id}_{item_id}")
        };
        if combined.chars().count() <= 40 {
            return combined;
        }
        let hash: String = short_hash(id).chars().take(8).collect();
        let prefix: String = call_id
            .chars()
            .take((40 - hash.chars().count() - 1).max(1))
            .collect();
        return format!("{prefix}_{hash}");
    }
    if model.provider == "openai" {
        let units: Vec<u16> = id.encode_utf16().take(40).collect();
        return String::from_utf16_lossy(&units);
    }
    id.to_owned()
}

fn is_reasoning_detail(detail: &Value) -> bool {
    let Some(object) = detail.as_object() else {
        return false;
    };
    let common = object
        .get("id")
        .is_none_or(|v| v.is_null() || v.is_string())
        && object.get("format").is_none_or(Value::is_string)
        && object.get("index").is_none_or(Value::is_number);
    if !common {
        return false;
    }
    match object.get("type").and_then(Value::as_str) {
        Some("reasoning.summary") => object.get("summary").is_some_and(Value::is_string),
        Some("reasoning.encrypted") => object.get("data").is_some_and(Value::is_string),
        Some("reasoning.text") => {
            object.get("text").is_some_and(Value::is_string)
                && object
                    .get("signature")
                    .is_none_or(|v| v.is_null() || v.is_string())
        }
        _ => false,
    }
}

fn parse_reasoning_details(signature: Option<&str>) -> Option<Vec<Value>> {
    let parsed: Value = serde_json::from_str(signature?).ok()?;
    let list = parsed.as_array()?;
    (!list.is_empty() && list.iter().all(is_reasoning_detail)).then(|| list.clone())
}

fn parse_legacy_encrypted_detail(signature: Option<&str>) -> Option<Value> {
    let parsed: Value = serde_json::from_str(signature?).ok()?;
    (is_reasoning_detail(&parsed)
        && parsed["type"] == "reasoning.encrypted"
        && parsed["id"].as_str().is_some_and(|id| !id.is_empty())
        && parsed["data"].as_str().is_some_and(|d| !d.is_empty()))
    .then_some(parsed)
}

fn image_url(data: &str, mime_type: &str) -> Value {
    json!({"type": "image_url", "image_url": {"url": format!("data:{mime_type};base64,{data}")}})
}

/// pi `convertMessages` for chat completions.
#[allow(clippy::too_many_lines, reason = "1:1 port of pi convertMessages")]
fn convert_messages(
    model: &Model,
    transcript: &Context,
    compat: &CompletionsCompat,
    grammar: &BTreeMap<String, String>,
    now: i64,
) -> Result<Vec<Value>, String> {
    let normalized = if compat.supports_mid_convo_system_messages {
        transcript.clone()
    } else {
        collapse_system_messages(transcript.clone())
    };
    let transformed = transform_messages(
        normalized.messages.clone(),
        model,
        Some(&|id: &str, _: &Model, _: &AssistantMessage| normalize_tool_call_id(model, id)),
        now,
    );
    let tools = resolve_transcript_tools(&normalized.messages, tool_support(compat));
    let instruction_role = if model.reasoning && compat.supports_developer_role {
        "developer"
    } else {
        "system"
    };
    let mut params: Vec<Value> = Vec::new();
    let mut last_role: Option<&'static str> = None;
    let mut system_index = 0;
    let mut i = 0;
    while i < transformed.len() {
        let message = &transformed[i];
        if compat.requires_assistant_after_tool_result
            && last_role == Some("toolResult")
            && matches!(message, Message::User(_))
        {
            params.push(
                json!({"role": "assistant", "content": "I have processed the tool results."}),
            );
        }
        match message {
            Message::System(system) => {
                let added = if system_index >= tools.folded_system_messages {
                    system.tools_added.clone().unwrap_or_default()
                } else {
                    Vec::new()
                };
                system_index += 1;
                if !added.is_empty() {
                    params.push(json!({"role": "system", "tools": convert_tools(&added, compat)?}));
                }
                let text = if i == 0 {
                    system_message_text(system)
                } else {
                    render_system_message_update(system)
                };
                if !text.is_empty() {
                    params.push(json!({"role": instruction_role, "content": text}));
                }
                last_role = Some("system");
            }
            Message::User(user) => {
                match &user.content {
                    UserContentField::Text(text) => {
                        params.push(json!({"role": "user", "content": text}));
                    }
                    UserContentField::Blocks(blocks) => {
                        let content: Vec<Value> = blocks
                            .iter()
                            .filter(|b| match b {
                                UserContent::Text(t) => !t.text.is_empty(),
                                UserContent::Image(_) => true,
                            })
                            .map(|b| match b {
                                UserContent::Text(t) => json!({"type": "text", "text": t.text}),
                                UserContent::Image(img) => image_url(&img.data, &img.mime_type),
                            })
                            .collect();
                        if content.is_empty() {
                            i += 1;
                            continue;
                        }
                        params.push(json!({"role": "user", "content": content}));
                    }
                }
                last_role = Some("user");
            }
            Message::Assistant(assistant) => {
                if let Some(converted) = convert_assistant(model, assistant, compat, grammar)? {
                    params.push(converted);
                    last_role = Some("assistant");
                } else {
                    i += 1;
                    continue;
                }
            }
            Message::ToolResult(_) => {
                let mut images = Vec::new();
                while let Some(Message::ToolResult(result)) = transformed.get(i) {
                    let text: Vec<&str> = result
                        .content
                        .iter()
                        .filter_map(|c| match c {
                            UserContent::Text(t) => Some(t.text.as_str()),
                            UserContent::Image(_) => None,
                        })
                        .collect();
                    let text = text.join("\n");
                    let has_images = result
                        .content
                        .iter()
                        .any(|c| matches!(c, UserContent::Image(_)));
                    let body = if !text.trim().is_empty() {
                        text
                    } else if has_images {
                        "(see attached image)".to_owned()
                    } else {
                        "(no tool output)".to_owned()
                    };
                    let mut entry = json!({"role": "tool", "content": body, "tool_call_id": result.tool_call_id});
                    if compat.requires_tool_result_name && !result.tool_name.is_empty() {
                        entry["name"] = json!(result.tool_name);
                    }
                    params.push(entry);
                    if has_images && model.input.contains(&InputModality::Image) {
                        for block in &result.content {
                            if let UserContent::Image(img) = block {
                                images.push(image_url(&img.data, &img.mime_type));
                            }
                        }
                    }
                    i += 1;
                }
                if images.is_empty() {
                    last_role = Some("toolResult");
                } else {
                    if compat.requires_assistant_after_tool_result {
                        params.push(json!({"role": "assistant", "content": "I have processed the tool results."}));
                    }
                    let mut content = vec![
                        json!({"type": "text", "text": "Attached image(s) from tool result:"}),
                    ];
                    content.extend(images);
                    params.push(json!({"role": "user", "content": content}));
                    last_role = Some("user");
                }
                continue;
            }
        }
        i += 1;
    }
    Ok(params)
}

#[allow(
    clippy::too_many_lines,
    reason = "1:1 port of pi's assistant conversion"
)]
fn convert_assistant(
    model: &Model,
    assistant: &AssistantMessage,
    compat: &CompletionsCompat,
    grammar: &BTreeMap<String, String>,
) -> Result<Option<Value>, String> {
    let mut message = JsonObject::new();
    message.insert("role".into(), json!("assistant"));
    message.insert(
        "content".into(),
        if compat.requires_assistant_after_tool_result {
            json!("")
        } else {
            Value::Null
        },
    );
    let text_parts: Vec<Value> = assistant
        .content
        .iter()
        .filter_map(|b| match b {
            AssistantContent::Text(t) if !t.text.trim().is_empty() => {
                Some(json!({"type": "text", "text": t.text}))
            }
            _ => None,
        })
        .collect();
    let text: String = text_parts
        .iter()
        .filter_map(|p| p["text"].as_str())
        .collect();
    let thinking: Vec<&ThinkingContent> = assistant
        .content
        .iter()
        .filter_map(|b| match b {
            AssistantContent::Thinking(t) => Some(t),
            _ => None,
        })
        .collect();
    let calls: Vec<&ToolCall> = assistant
        .content
        .iter()
        .filter_map(|b| match b {
            AssistantContent::ToolCall(c) => Some(c),
            _ => None,
        })
        .collect();
    let signed = thinking
        .iter()
        .find_map(|t| parse_reasoning_details(t.thinking_signature.as_deref()));
    let legacy: Vec<Value> = calls
        .iter()
        .filter_map(|c| parse_legacy_encrypted_detail(c.thought_signature.as_deref()))
        .collect();
    let preserved = signed.or_else(|| (!legacy.is_empty()).then_some(legacy));
    let non_empty: Vec<&&ThinkingContent> = thinking
        .iter()
        .filter(|t| !t.thinking.trim().is_empty())
        .collect();
    if non_empty.is_empty() {
        if !text.is_empty() {
            message.insert("content".into(), json!(text));
        }
    } else if compat.requires_thinking_as_text {
        let thinking_text: Vec<&str> = non_empty.iter().map(|t| t.thinking.as_str()).collect();
        let mut content = vec![json!({"type": "text", "text": thinking_text.join("\n\n")})];
        content.extend(text_parts);
        message.insert("content".into(), Value::Array(content));
    } else {
        if !text.is_empty() {
            message.insert("content".into(), json!(text));
        }
        if preserved.is_none() {
            let mut signature = non_empty[0].thinking_signature.clone();
            if model.provider == "opencode-go" && signature.as_deref() == Some("reasoning") {
                signature = Some("reasoning_content".to_owned());
            }
            if let Some(field) = signature.filter(|s| {
                matches!(
                    s.as_str(),
                    "reasoning" | "reasoning_content" | "reasoning_text"
                )
            }) {
                let joined: Vec<&str> = non_empty.iter().map(|t| t.thinking.as_str()).collect();
                message.insert(field, json!(joined.join("\n")));
            }
        }
    }
    if !calls.is_empty() {
        let mut list = Vec::new();
        for call in &calls {
            if let Some(property) = grammar.get(&call.name) {
                list.push(json!({
                    "id": call.id,
                    "type": "custom",
                    "custom": {"name": call.name, "input": grammar_tool_input(&call.name, &call.arguments, property)?},
                }));
            } else {
                list.push(json!({
                    "id": call.id,
                    "type": "function",
                    "function": {"name": call.name, "arguments": Value::Object(call.arguments.clone()).to_string()},
                }));
            }
        }
        message.insert("tool_calls".into(), Value::Array(list));
    }
    if let Some(details) = preserved {
        message.insert("reasoning_details".into(), Value::Array(details));
    }
    if compat.requires_reasoning_content_on_assistant_messages
        && model.reasoning
        && !message.contains_key("reasoning_content")
    {
        message.insert("reasoning_content".into(), json!(""));
    }
    let has_content = match message.get("content") {
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        _ => false,
    };
    if !has_content && !message.contains_key("tool_calls") {
        return Ok(None);
    }
    Ok(Some(Value::Object(message)))
}

fn add_cache_control_to_text(message: &mut Value, cache: &Value) -> bool {
    match &mut message["content"] {
        Value::String(text) => {
            if text.is_empty() {
                return false;
            }
            let text = std::mem::take(text);
            message["content"] = json!([{"type": "text", "text": text, "cache_control": cache}]);
            true
        }
        Value::Array(parts) => {
            for part in parts.iter_mut().rev() {
                if part["type"] == "text" {
                    part["cache_control"] = cache.clone();
                    return true;
                }
            }
            false
        }
        _ => false,
    }
}

/// pi `applyAnthropicCacheControl`.
fn apply_anthropic_cache_control(
    messages: &mut [Value],
    tools: Option<&mut Vec<Value>>,
    cache: &Value,
) {
    if let Some(message) = messages
        .iter_mut()
        .find(|m| matches!(m["role"].as_str(), Some("system" | "developer")))
    {
        add_cache_control_to_text(message, cache);
    }
    if let Some(last) = tools.and_then(|t| t.last_mut()) {
        last["cache_control"] = cache.clone();
    }
    for message in messages.iter_mut().rev() {
        if matches!(
            message["role"].as_str(),
            Some("user" | "assistant" | "tool")
        ) && add_cache_control_to_text(message, cache)
        {
            return;
        }
    }
}

fn has_tool_history(messages: &[Message]) -> bool {
    messages.iter().any(|m| match m {
        Message::ToolResult(_) => true,
        Message::Assistant(a) => a
            .content
            .iter()
            .any(|b| matches!(b, AssistantContent::ToolCall(_))),
        _ => false,
    })
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

fn template_value(
    model: &Model,
    effort: Option<ThinkingLevel>,
    value: &Value,
    budget: Option<u64>,
) -> Option<Value> {
    let Some(object) = value.as_object() else {
        return Some(value.clone());
    };
    if effort.is_none() && object.get("omitWhenOff").and_then(Value::as_bool) == Some(true) {
        return None;
    }
    match object.get("$var").and_then(Value::as_str) {
        Some("thinking.enabled") => return Some(json!(effort.is_some())),
        Some("thinking.budget") => return budget.map(|b| json!(b)),
        _ => {}
    }
    let mapped = mapped_level(model, effort.unwrap_or(ThinkingLevel::Off));
    match (mapped, effort) {
        (MapEntry::Absent, Some(level)) => Some(json!(level.as_str())),
        (MapEntry::Absent, None) | (MapEntry::Null, _) => None,
        (MapEntry::Value(text), _) => Some(json!(text)),
    }
}

fn chat_template_values(
    model: &Model,
    effort: Option<ThinkingLevel>,
    values: &Value,
    budget: Option<u64>,
) -> Option<Value> {
    let mut out = JsonObject::new();
    for (key, value) in values.as_object().into_iter().flatten() {
        if let Some(resolved) = template_value(model, effort, value, budget) {
            out.insert(key.clone(), resolved);
        }
    }
    (!out.is_empty()).then_some(Value::Object(out))
}

/// pi `buildParams` for chat completions.
#[allow(clippy::too_many_lines, reason = "1:1 port of pi buildParams")]
fn build_params(
    request: &ProviderRequest,
    context: &Context,
    compat: &CompletionsCompat,
    retention: CacheRetention,
    grammar: &BTreeMap<String, String>,
    now: i64,
) -> Result<Value, String> {
    let model = &request.model;
    let options = &request.options;
    let effort = options
        .reasoning
        .map(|level| clamp_thinking_level(model, level))
        .filter(|level| *level != ThinkingLevel::Off);
    let transcript_tools = resolve_transcript_tools(&context.messages, tool_support(compat));
    let mut messages = convert_messages(model, context, compat, grammar, now)?;
    let cache = (compat.cache_control_format.as_deref() == Some("anthropic")
        && retention != CacheRetention::None)
        .then(|| {
            if retention == CacheRetention::Long && compat.supports_long_cache_retention {
                json!({"type": "ephemeral", "ttl": "1h"})
            } else {
                json!({"type": "ephemeral"})
            }
        });
    let long = retention == CacheRetention::Long && compat.supports_long_cache_retention;
    let mut params = JsonObject::new();
    params.insert("model".into(), json!(model.id));
    params.insert("messages".into(), Value::Null);
    params.insert("stream".into(), json!(true));
    if ((model.base_url.contains("api.openai.com") && retention != CacheRetention::None) || long)
        && let Some(session) = &options.session_id
    {
        params.insert(
            "prompt_cache_key".into(),
            json!(clamp_prompt_cache_key(session)),
        );
    }
    if long {
        params.insert("prompt_cache_retention".into(), json!("24h"));
    }
    if compat.supports_usage_in_streaming {
        params.insert("stream_options".into(), json!({"include_usage": true}));
    }
    if compat.supports_store {
        params.insert("store".into(), json!(false));
    }
    let max_tokens = clamp_max_tokens_to_context(
        model,
        &request.context.messages,
        options.max_tokens.unwrap_or(model.max_tokens),
    );
    if max_tokens > 0 {
        let field = if compat.max_tokens_field == "max_tokens" {
            "max_tokens"
        } else {
            "max_completion_tokens"
        };
        params.insert(field.into(), json!(max_tokens));
    }
    if let Some(temperature) = options.temperature {
        params.insert("temperature".into(), json!(temperature));
    }
    let mut tools = None;
    if !transcript_tools.request_tools.is_empty() {
        tools = Some(convert_tools(&transcript_tools.request_tools, compat)?);
    } else if has_tool_history(&context.messages) {
        tools = Some(Vec::new());
    }
    if let Some(cache) = &cache {
        apply_anthropic_cache_control(&mut messages, tools.as_mut(), cache);
    }
    params.insert("messages".into(), Value::Array(messages));
    if let Some(tools) = tools {
        let non_empty = !tools.is_empty();
        params.insert("tools".into(), Value::Array(tools));
        if non_empty && compat.zai_tool_stream {
            params.insert("tool_stream".into(), json!(true));
        }
    }
    if let Some(choice) = options.tool_choice.as_ref().filter(|c| is_set(c)) {
        params.insert("tool_choice".into(), choice.clone());
    }
    if let Some(priority) = &compat.vllm_priority {
        params.insert("priority".into(), priority.clone());
    }
    let budget_field = compat.thinking_token_budget_field.clone().or_else(|| {
        compat
            .supports_thinking_token_budget
            .then(|| "thinking_token_budget".to_owned())
    });
    let budget = effort.filter(|_| model.reasoning).and_then(|level| {
        let ceiling = params
            .get("max_tokens")
            .or_else(|| params.get("max_completion_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(model.max_tokens);
        let budget = clamp_thinking_budget_to_answer_room(
            thinking_budget_for_level(level, options.thinking_budgets.as_ref()),
            ceiling,
        );
        (budget > 0).then_some(budget)
    });
    thinking_params(&mut params, model, compat, effort, budget);
    if let (Some(field), Some(budget)) = (budget_field, budget) {
        params.insert(field, json!(budget));
    }
    if let Some(routing) = compat_value(model, "openRouterRouting").filter(|v| v.is_object()) {
        params.insert("provider".into(), routing.clone());
    }
    if let Some(routing) = compat_value(model, "vercelGatewayRouting") {
        let mut gateway = JsonObject::new();
        for key in ["only", "order"] {
            if let Some(value) = routing.get(key).filter(|v| is_set(v)) {
                gateway.insert(key.into(), value.clone());
            }
        }
        if !gateway.is_empty() {
            params.insert("providerOptions".into(), json!({"gateway": gateway}));
        }
    }
    let base_sampling = resolve_sampling_params(
        model,
        options.reasoning.unwrap_or(ThinkingLevel::Off),
        options.sampling_params.as_ref(),
    );
    if let Some(sampling) = resolve_sampling_params(
        model,
        effort.unwrap_or(ThinkingLevel::Off),
        base_sampling.as_ref(),
    ) {
        for (key, value) in sampling {
            params.insert(key, value);
        }
    }
    Ok(Value::Object(params))
}

fn is_set(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::String(s) => !s.is_empty(),
        Value::Bool(b) => *b,
        _ => true,
    }
}

#[allow(clippy::too_many_lines, reason = "one arm per pi thinkingFormat")]
fn thinking_params(
    params: &mut JsonObject,
    model: &Model,
    compat: &CompletionsCompat,
    effort: Option<ThinkingLevel>,
    budget: Option<u64>,
) {
    if !model.reasoning {
        return;
    }
    let mapped = |level: ThinkingLevel| mapped_level(model, level);
    match compat.thinking_format.as_str() {
        "zai" => {
            params.insert(
                "thinking".into(),
                if effort.is_some() {
                    json!({"type": "enabled", "clear_thinking": false})
                } else {
                    json!({"type": "disabled"})
                },
            );
            if let (Some(level), true) = (effort, compat.supports_reasoning_effort) {
                match mapped(level) {
                    MapEntry::Value(v) => {
                        params.insert("reasoning_effort".into(), json!(v));
                    }
                    MapEntry::Absent => {
                        params.insert("reasoning_effort".into(), json!(level.as_str()));
                    }
                    MapEntry::Null => {}
                }
            }
        }
        "qwen" => {
            params.insert("enable_thinking".into(), json!(effort.is_some()));
            if let (Some(level), true) = (effort, compat.supports_reasoning_effort) {
                params.insert("reasoning_effort".into(), mapped_or_level(model, level));
            }
        }
        "qwen-chat-template" => {
            params.insert(
                "chat_template_kwargs".into(),
                json!({"enable_thinking": effort.is_some(), "preserve_thinking": true}),
            );
        }
        "chat-template" => {
            if let Some(values) =
                chat_template_values(model, effort, &compat.chat_template_kwargs, budget)
            {
                params.insert("chat_template_kwargs".into(), values);
            }
        }
        "baseten" => {
            if let Some(values) =
                chat_template_values(model, effort, &compat.chat_template_args, budget)
            {
                params.insert("chat_template_args".into(), values);
            }
            if compat.supports_reasoning_effort {
                let entry = mapped(effort.unwrap_or(ThinkingLevel::Off));
                match (entry, effort) {
                    (MapEntry::Value(v), _) => {
                        params.insert("reasoning_effort".into(), json!(v));
                    }
                    (MapEntry::Absent, Some(level)) => {
                        params.insert("reasoning_effort".into(), json!(level.as_str()));
                    }
                    _ => {}
                }
            }
        }
        "deepseek" => {
            if effort.is_some() {
                params.insert("thinking".into(), json!({"type": "enabled"}));
            } else if off_not_null(model) {
                params.insert("thinking".into(), json!({"type": "disabled"}));
            }
            if let (Some(level), true) = (effort, compat.supports_reasoning_effort) {
                params.insert("reasoning_effort".into(), mapped_or_level(model, level));
            }
        }
        "openrouter" => {
            if let Some(level) = effort {
                params.insert(
                    "reasoning".into(),
                    json!({"effort": mapped_or_level(model, level)}),
                );
            } else if off_not_null(model) {
                let off = match mapped(ThinkingLevel::Off) {
                    MapEntry::Value(v) => v.to_owned(),
                    _ => "none".to_owned(),
                };
                params.insert("reasoning".into(), json!({"effort": off}));
            }
        }
        "ant-ling" => {
            if let Some(MapEntry::Value(v)) = effort.map(mapped) {
                params.insert("reasoning".into(), json!({"effort": v}));
            }
        }
        "together" => {
            params.insert("reasoning".into(), json!({"enabled": effort.is_some()}));
            if let (Some(level), true) = (effort, compat.supports_reasoning_effort) {
                params.insert("reasoning_effort".into(), mapped_or_level(model, level));
            }
        }
        "string-thinking" => {
            if let Some(level) = effort {
                params.insert("thinking".into(), mapped_or_level(model, level));
            } else if off_not_null(model) {
                let off = match mapped(ThinkingLevel::Off) {
                    MapEntry::Value(v) => v.to_owned(),
                    _ => "none".to_owned(),
                };
                params.insert("thinking".into(), json!(off));
            }
        }
        _ => {
            if let Some(level) = effort {
                if compat.supports_reasoning_effort {
                    params.insert("reasoning_effort".into(), mapped_or_level(model, level));
                }
            } else if compat.supports_reasoning_effort
                && let MapEntry::Value(off) = mapped(ThinkingLevel::Off)
            {
                params.insert("reasoning_effort".into(), json!(off));
            }
        }
    }
}

/// A prepared chat-completions request.
pub struct Prepared {
    /// Outbound request.
    pub http: HttpRequest,
    /// Streaming state.
    pub exchange: Exchange,
}

/// Builds the request (pi `streamSimple` + `stream` up to the HTTP call).
///
/// # Errors
/// Missing credentials or a tool that cannot be expressed.
pub fn prepare(
    request: &ProviderRequest,
    allow_missing_key: bool,
    now: i64,
) -> Result<Prepared, String> {
    let model = &request.model;
    let key = request.api_key.clone().filter(|k| !k.is_empty());
    let has_auth_header =
        request.has_header("authorization") || request.has_header("cf-aig-authorization");
    if key.is_none() && !has_auth_header && !allow_missing_key {
        return Err(format!("No API key for provider: {}", model.provider));
    }
    let compat = compat(model);
    let normalized = if compat.supports_mid_convo_system_messages {
        request.context.clone()
    } else {
        collapse_system_messages(request.context.clone())
    };
    let grammar = grammar_input_properties(
        &declared_tools(&normalized.messages),
        compat.supports_openai_grammar_tools,
    )?;
    let retention = resolve_retention(request);
    let params = build_params(request, &normalized, &compat, retention, &grammar, now)?;
    let mut headers = Headers::new();
    headers.set("accept", "application/json");
    headers.set("content-type", "application/json");
    match (&key, has_auth_header) {
        (Some(key), _) => headers.set("authorization", format!("Bearer {key}")),
        (None, true) => headers.set("authorization", "Bearer unused"),
        (None, false) => {}
    }
    headers.set("user-agent", USER_AGENT);
    for (name, value) in model.headers.iter().flatten() {
        headers.set(name, value);
    }
    for (name, value) in providers::copilot_dynamic_headers(request, &request.context.messages) {
        headers.set(&name, value);
    }
    let session = request
        .options
        .session_id
        .as_deref()
        .filter(|_| retention != CacheRetention::None);
    if let (Some(session), true) = (session, compat.send_session_affinity_headers) {
        if compat.session_affinity_format == "openrouter" {
            headers.set("x-session-id", session);
        } else {
            if compat.session_affinity_format == "openai" {
                headers.set("session_id", session);
            }
            headers.set("x-client-request-id", session);
            headers.set("x-session-affinity", session);
        }
    }
    for (name, value) in request.header_overrides() {
        headers.apply(&name, value.as_deref());
    }
    let parser = Parser {
        model: model.clone(),
        compat,
        grammar,
        text: None,
        thinking: None,
        tools_by_index: BTreeMap::new(),
        tools_by_id: BTreeMap::new(),
        scratch: BTreeMap::new(),
        reasoning_details: None,
        has_finish_reason: false,
    };
    Ok(Prepared {
        http: HttpRequest {
            url: join_url(&model.base_url, "/chat/completions"),
            headers,
            payload: params,
            timeout_ms: request.options.timeout_ms,
            signing: None,
        },
        exchange: Exchange::new(
            Emitter::new(model, now),
            Box::new(parser),
            ErrorStyle::Completions,
        ),
    })
}

/// Per-tool-call scratch (pi `partialArgs`, `customInput`).
#[derive(Default)]
struct ToolScratch {
    partial_args: Option<String>,
    custom: Option<(String, GrammarBuffer)>,
}

struct Parser {
    model: Model,
    compat: CompletionsCompat,
    grammar: BTreeMap<String, String>,
    text: Option<usize>,
    thinking: Option<usize>,
    tools_by_index: BTreeMap<i64, usize>,
    tools_by_id: BTreeMap<String, usize>,
    scratch: BTreeMap<usize, ToolScratch>,
    reasoning_details: Option<Vec<Value>>,
    has_finish_reason: bool,
}

fn parse_chunk_usage(raw: &Value, model: &Model) -> oino_ai::Usage {
    let prompt = raw["prompt_tokens"].as_u64().unwrap_or(0);
    let read = raw["prompt_tokens_details"]["cached_tokens"]
        .as_u64()
        .or_else(|| raw["prompt_cache_hit_tokens"].as_u64())
        .or_else(|| raw["cached_tokens"].as_u64())
        .unwrap_or(0);
    let write = raw["prompt_tokens_details"]["cache_write_tokens"]
        .as_u64()
        .unwrap_or(0);
    let input = prompt.saturating_sub(read).saturating_sub(write);
    let output = raw["completion_tokens"].as_u64().unwrap_or(0);
    let mut usage = oino_ai::Usage {
        input,
        output,
        cache_read: read,
        cache_write: write,
        cache_write_1h: None,
        reasoning: Some(
            raw["completion_tokens_details"]["reasoning_tokens"]
                .as_u64()
                .unwrap_or(0),
        ),
        total_tokens: input + output + read + write,
        cost: oino_ai::UsageCost::default(),
    };
    calculate_cost(model, &mut usage);
    usage
}

fn map_finish_reason(reason: &str) -> (StopReason, Option<String>) {
    match reason {
        "stop" | "end" => (StopReason::Stop, None),
        "length" => (StopReason::Length, None),
        "function_call" | "tool_calls" => (StopReason::ToolUse, None),
        other => (
            StopReason::Error,
            Some(format!("Provider finish_reason: {other}")),
        ),
    }
}

fn append_reasoning_detail(details: &mut Vec<Value>, detail: &Value) {
    let fill = |target: &mut Value, source: &Value| {
        if target.get("id").is_none_or(Value::is_null)
            && let Some(id) = source.get("id")
        {
            target["id"] = id.clone();
        }
        if target
            .get("format")
            .is_none_or(|f| f.as_str().is_none_or(str::is_empty))
            && let Some(format) = source.get("format")
        {
            target["format"] = format.clone();
        }
        if target.get("index").is_none_or(Value::is_null)
            && let Some(index) = source.get("index")
        {
            target["index"] = index.clone();
        }
    };
    let kind = detail["type"].as_str();
    if let Some(last) = details.last_mut() {
        if kind == Some("reasoning.text") && last["type"] == "reasoning.text" {
            let text = format!(
                "{}{}",
                last["text"].as_str().unwrap_or_default(),
                detail["text"].as_str().unwrap_or_default()
            );
            last["text"] = json!(text);
            if last
                .get("signature")
                .is_none_or(|s| s.as_str().is_none_or(str::is_empty))
                && let Some(signature) = detail.get("signature")
            {
                last["signature"] = signature.clone();
            }
            fill(last, detail);
            return;
        }
        if kind == Some("reasoning.summary") && last["type"] == "reasoning.summary" {
            let summary = format!(
                "{}{}",
                last["summary"].as_str().unwrap_or_default(),
                detail["summary"].as_str().unwrap_or_default()
            );
            last["summary"] = json!(summary);
            fill(last, detail);
            return;
        }
    }
    details.push(detail.clone());
}

impl Parser {
    fn ensure_text(&mut self, out: &mut Emitter) -> usize {
        if let Some(index) = self.text {
            return index;
        }
        let index = out.add(AssistantContent::Text(TextContent::new("")));
        self.text = Some(index);
        out.push(ProviderEvent::TextStart {
            content_index: index,
        });
        index
    }

    fn ensure_thinking(&mut self, out: &mut Emitter, signature: &str) -> usize {
        if let Some(index) = self.thinking {
            return index;
        }
        let index = out.add(AssistantContent::Thinking(ThinkingContent {
            thinking: String::new(),
            thinking_signature: Some(signature.to_owned()),
            redacted: None,
        }));
        self.thinking = Some(index);
        out.push(ProviderEvent::ThinkingStart {
            content_index: index,
        });
        index
    }

    fn apply_reasoning_details(&self, out: &mut Emitter, index: usize) {
        if let (Some(details), Some(AssistantContent::Thinking(block))) =
            (&self.reasoning_details, out.block(index))
        {
            block.thinking_signature = Some(Value::Array(details.clone()).to_string());
        }
    }

    fn ensure_tool(&mut self, call: &Value, out: &mut Emitter) -> usize {
        let stream_index = call["index"].as_i64();
        let id = call["id"].as_str().filter(|s| !s.is_empty());
        let is_custom = call.get("custom").is_some_and(|c| !c.is_null())
            && call.get("function").is_none_or(Value::is_null);
        let name = call["function"]["name"]
            .as_str()
            .or_else(|| call["custom"]["name"].as_str())
            .unwrap_or_default();
        let mut found = stream_index.and_then(|i| self.tools_by_index.get(&i).copied());
        if found.is_none() {
            found = id.and_then(|id| self.tools_by_id.get(id).copied());
        }
        let index = if let Some(index) = found {
            index
        } else {
            let property = is_custom.then(|| {
                self.grammar
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| "input".to_owned())
            });
            let mut arguments = JsonObject::new();
            if let Some(property) = &property {
                arguments.insert(property.clone(), json!(""));
            }
            let index = out.add(AssistantContent::ToolCall(ToolCall {
                id: id.unwrap_or_default().to_owned(),
                name: name.to_owned(),
                arguments,
                ..ToolCall::default()
            }));
            self.scratch.insert(
                index,
                ToolScratch {
                    partial_args: property.is_none().then(String::new),
                    custom: property.map(|p| (p, GrammarBuffer::default())),
                },
            );
            if let Some(stream_index) = stream_index {
                self.tools_by_index.insert(stream_index, index);
            }
            out.push(ProviderEvent::ToolcallStart {
                content_index: index,
                id: id.map(str::to_owned),
                tool_name: (!name.is_empty()).then(|| name.to_owned()),
            });
            index
        };
        if let Some(stream_index) = stream_index {
            self.tools_by_index.entry(stream_index).or_insert(index);
        }
        if let Some(id) = id {
            self.tools_by_id.insert(id.to_owned(), index);
        }
        if let Some(AssistantContent::ToolCall(block)) = out.block(index) {
            if block.name.is_empty() && !name.is_empty() {
                name.clone_into(&mut block.name);
            }
            if is_custom {
                let scratch = self.scratch.entry(index).or_default();
                if scratch.custom.is_none() {
                    let property = self
                        .grammar
                        .get(&block.name)
                        .cloned()
                        .unwrap_or_else(|| "input".to_owned());
                    block.arguments = JsonObject::new();
                    block.arguments.insert(property.clone(), json!(""));
                    scratch.custom = Some((property, GrammarBuffer::default()));
                    scratch.partial_args = None;
                }
            }
        }
        index
    }

    fn custom_input(out: &mut Emitter, index: usize, property: &str) -> String {
        match out.block(index) {
            Some(AssistantContent::ToolCall(block)) => block
                .arguments
                .get(property)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            _ => String::new(),
        }
    }

    fn append_custom(
        &mut self,
        out: &mut Emitter,
        index: usize,
        next: &str,
        close: bool,
    ) -> Result<Option<String>, StreamError> {
        let Some(ToolScratch {
            custom: Some((property, buffer)),
            ..
        }) = self.scratch.get_mut(&index)
        else {
            return Ok(None);
        };
        let delta = append_grammar_delta(buffer, property, next, close)?;
        let property = property.clone();
        if let Some(AssistantContent::ToolCall(block)) = out.block(index) {
            block.arguments = JsonObject::new();
            block.arguments.insert(property, json!(next));
        }
        Ok(delta)
    }

    fn tool_delta(&mut self, call: &Value, out: &mut Emitter) -> Result<(), StreamError> {
        let index = self.ensure_tool(call, out);
        if let Some(AssistantContent::ToolCall(block)) = out.block(index)
            && let Some(id) = call["id"].as_str().filter(|s| !s.is_empty())
            && block.id.is_empty()
        {
            id.clone_into(&mut block.id);
            self.tools_by_id.insert(id.to_owned(), index);
        }
        let mut delta = String::new();
        if let Some(arguments) = call["function"]["arguments"]
            .as_str()
            .filter(|a| !a.is_empty())
        {
            arguments.clone_into(&mut delta);
            let scratch = self.scratch.entry(index).or_default();
            let buffer = scratch.partial_args.get_or_insert_with(String::new);
            buffer.push_str(arguments);
            let parsed = parse_streaming_object(buffer);
            if let Some(AssistantContent::ToolCall(block)) = out.block(index) {
                block.arguments = parsed;
            }
        } else if let Some(input) = call["custom"]["input"].as_str().filter(|i| !i.is_empty()) {
            let property = self
                .scratch
                .get(&index)
                .and_then(|s| s.custom.as_ref())
                .map(|(p, _)| p.clone());
            if let Some(property) = property {
                let next = format!("{}{input}", Self::custom_input(out, index, &property));
                delta = self
                    .append_custom(out, index, &next, false)?
                    .unwrap_or_default();
            }
        }
        out.push(ProviderEvent::ToolcallDelta {
            content_index: index,
            delta,
        });
        Ok(())
    }

    fn chunk(&mut self, chunk: &Value, out: &mut Emitter) -> Result<(), StreamError> {
        if out.message.response_id.as_deref().is_none_or(str::is_empty)
            && let Some(id) = chunk["id"].as_str().filter(|s| !s.is_empty())
        {
            out.message.response_id = Some(id.to_owned());
        }
        if let Some(response_model) = chunk["model"].as_str()
            && !response_model.is_empty()
            && response_model != self.model.id
            && out.message.response_model.is_none()
        {
            out.message.response_model = Some(response_model.to_owned());
        }
        let has_usage = is_set(&chunk["usage"]);
        if has_usage {
            out.message.usage = parse_chunk_usage(&chunk["usage"], &self.model);
        }
        let Some(choice) = chunk["choices"].as_array().and_then(|c| c.first()) else {
            return Ok(());
        };
        if !has_usage && is_set(&choice["usage"]) {
            out.message.usage = parse_chunk_usage(&choice["usage"], &self.model);
        }
        if let Some(reason) = choice["finish_reason"].as_str().filter(|r| !r.is_empty()) {
            out.message.raw_stop_reason = Some(reason.to_owned());
            let (stop, message) = map_finish_reason(reason);
            out.message.stop_reason = stop;
            if let Some(message) = message {
                out.message.error_message = Some(message);
            }
            self.has_finish_reason = true;
        }
        let delta = &choice["delta"];
        if !delta.is_object() {
            return Ok(());
        }
        let field = ["reasoning_content", "reasoning", "reasoning_text"]
            .into_iter()
            .find(|f| delta[*f].as_str().is_some_and(|v| !v.is_empty()));
        if let Some(field) = field {
            let text = delta[field].as_str().unwrap_or_default().to_owned();
            let signature = if self.model.provider == "opencode-go" && field == "reasoning" {
                "reasoning_content"
            } else {
                field
            };
            let index = self.ensure_thinking(out, signature);
            if let Some(AssistantContent::Thinking(block)) = out.block(index) {
                block.thinking.push_str(&text);
            }
            out.push(ProviderEvent::ThinkingDelta {
                content_index: index,
                delta: text,
            });
        }
        if let Some(details) = delta["reasoning_details"].as_array() {
            for detail in details.iter().filter(|d| is_reasoning_detail(d)) {
                self.ensure_thinking(out, "");
                append_reasoning_detail(
                    self.reasoning_details.get_or_insert_with(Vec::new),
                    detail,
                );
            }
        }
        if let Some(content) = delta["content"].as_str().filter(|c| !c.is_empty()) {
            let index = self.ensure_text(out);
            if let Some(AssistantContent::Text(block)) = out.block(index) {
                block.text.push_str(content);
            }
            out.push(ProviderEvent::TextDelta {
                content_index: index,
                delta: content.to_owned(),
            });
        }
        if let Some(calls) = delta["tool_calls"].as_array() {
            for call in calls {
                self.tool_delta(call, out)?;
            }
        }
        Ok(())
    }

    fn finish_block(&mut self, out: &mut Emitter, index: usize) -> Result<(), StreamError> {
        match out.block(index) {
            Some(AssistantContent::Text(block)) => {
                let content = block.text.clone();
                out.push(ProviderEvent::TextEnd {
                    content_index: index,
                    content,
                });
            }
            Some(AssistantContent::Thinking(_)) => {
                self.apply_reasoning_details(out, index);
                if let Some(AssistantContent::Thinking(block)) = out.block(index) {
                    let content = block.thinking.clone();
                    out.push(ProviderEvent::ThinkingEnd {
                        content_index: index,
                        content,
                    });
                }
            }
            Some(AssistantContent::ToolCall(_)) => {
                let scratch = self.scratch.remove(&index).unwrap_or_default();
                if let Some((property, mut buffer)) = scratch.custom {
                    let input = Self::custom_input(out, index, &property);
                    if let Some(delta) = append_grammar_delta(&mut buffer, &property, &input, true)?
                    {
                        out.push(ProviderEvent::ToolcallDelta {
                            content_index: index,
                            delta,
                        });
                    }
                } else if let Some(AssistantContent::ToolCall(block)) = out.block(index) {
                    let parsed = parse_tool_call_arguments(
                        scratch.partial_args.as_deref().unwrap_or_default(),
                    );
                    block.arguments = parsed.arguments;
                    block.arguments_error = parsed.error;
                }
                if let Some(AssistantContent::ToolCall(block)) = out.block(index) {
                    let tool_call = block.clone();
                    out.push(ProviderEvent::ToolcallEnd {
                        content_index: index,
                        tool_call,
                    });
                }
            }
            None => {}
        }
        Ok(())
    }
}

impl StreamParser for Parser {
    fn event(&mut self, sse: &SseEvent, out: &mut Emitter) -> Result<Step, StreamError> {
        if sse.data == "[DONE]" {
            return Ok(Step::End);
        }
        let data: Value = serde_json::from_str(&sse.data).map_err(|_| {
            StreamError::from("Error reading response: malformed server-sent event JSON.")
        })?;
        if sse
            .event
            .as_deref()
            .is_some_and(|e| e.starts_with("thread."))
        {
            return Ok(Step::Event(data));
        }
        if sse.event.as_deref() == Some("error") || is_set(&data["error"]) {
            return Err(StreamError::Sdk(openai_stream_error(&data)));
        }
        if data.is_object() {
            self.chunk(&data, out)?;
        }
        Ok(Step::Event(data))
    }

    fn finish(&mut self, out: &mut Emitter) -> Result<(), StreamError> {
        for index in 0..out.message.content.len() {
            self.finish_block(out, index)?;
        }
        if out.message.stop_reason == StopReason::Aborted {
            return Err("Request was aborted".into());
        }
        if !self.has_finish_reason && !self.compat.supports_finish_reason {
            out.message.stop_reason = if has_tool_call(out) {
                StopReason::ToolUse
            } else {
                StopReason::Stop
            };
        }
        if out.message.stop_reason == StopReason::Error {
            return Err(out
                .message
                .error_message
                .clone()
                .unwrap_or_else(|| "Provider returned an error stop reason".to_owned())
                .into());
        }
        if (self.compat.supports_finish_reason && !self.has_finish_reason)
            || out.message.stop_reason == StopReason::Pending
        {
            return Err("Stream ended without finish_reason".into());
        }
        Ok(())
    }

    fn cleanup(&mut self, out: &mut Emitter) {
        if let Some(index) = self.thinking {
            self.apply_reasoning_details(out, index);
        }
    }
}
