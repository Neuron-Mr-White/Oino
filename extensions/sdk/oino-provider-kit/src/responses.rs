//! `OpenAI` Responses (pi-ai `api/openai-responses.js` + `openai-responses-shared.js`): input
//! items, tools, prompt-cache options and the response event stream parser.

use std::collections::{BTreeMap, BTreeSet};

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
use crate::options::{clamp_max_tokens_to_context, resolve_sampling_params};
use crate::request::{Headers, HttpRequest, ProviderRequest, USER_AGENT, join_url};
use crate::sse::SseEvent;
use crate::util::{
    calculate_cost, clamp_prompt_cache_key, clamp_thinking_level, compat_bool, mapped_level,
    mapped_or_level, short_hash,
};

const TOOL_CALL_PROVIDERS: [&str; 3] = ["openai", "openai-codex", "opencode"];
const MIN_OUTPUT_TOKENS: u64 = 16;
const CONFIGURATION_UPDATE_REQUEST_EFFORT: &str = "medium";

/// pi `getCompat(model)` for Responses.
#[allow(
    clippy::struct_excessive_bools,
    reason = "mirrors pi's compat flag record"
)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResponsesCompat {
    /// `developer` role.
    pub supports_developer_role: bool,
    /// Later system messages.
    pub supports_mid_convo_system_messages: bool,
    /// `openrouter` or `openai`.
    pub session_affinity_format: String,
    /// Long cache retention.
    pub supports_long_cache_retention: bool,
    /// Strict function tools.
    pub supports_strict_mode: bool,
    /// Grammar tools.
    pub supports_openai_grammar_tools: bool,
    /// `additional_tools` items.
    pub supports_additional_tools: bool,
    /// Synthetic tool search.
    pub supports_tool_search: bool,
    /// `prompt_cache_options`.
    pub supports_explicit_prompt_cache_mode: bool,
    /// `max_output_tokens`.
    pub supports_max_output_tokens: bool,
    /// `configuration_update` effort items.
    pub supports_mid_convo_effort: bool,
}

/// pi `getCompat(model)`.
#[must_use]
pub fn compat(model: &Model) -> ResponsesCompat {
    let flag = |key: &str, default: bool| compat_bool(model, key).unwrap_or(default);
    let detected = if model.provider == "openrouter" || model.base_url.contains("openrouter.ai") {
        "openrouter"
    } else {
        "openai"
    };
    ResponsesCompat {
        supports_developer_role: flag("supportsDeveloperRole", true),
        supports_mid_convo_system_messages: flag("supportsMidConvoSystemMessages", false),
        session_affinity_format: model
            .compat
            .as_ref()
            .and_then(|c| c.get("sessionAffinityFormat"))
            .and_then(Value::as_str)
            .unwrap_or(detected)
            .to_owned(),
        supports_long_cache_retention: flag("supportsLongCacheRetention", true),
        supports_strict_mode: flag("supportsStrictMode", false),
        supports_openai_grammar_tools: flag("supportsOpenAIGrammarTools", false),
        supports_additional_tools: flag("supportsAdditionalTools", false),
        supports_tool_search: flag("supportsToolSearch", false),
        supports_explicit_prompt_cache_mode: flag("supportsExplicitPromptCacheMode", false),
        supports_max_output_tokens: flag("supportsMaxOutputTokens", true),
        supports_mid_convo_effort: flag("supportsMidConvoEffort", false),
    }
}

/// pi `getResponsesToolSupport`.
fn tool_support(compat: &ResponsesCompat) -> ToolChangeSupport {
    if compat.supports_mid_convo_system_messages && compat.supports_additional_tools {
        ToolChangeSupport::Changes
    } else if compat.supports_tool_search {
        ToolChangeSupport::Additive
    } else {
        ToolChangeSupport::None
    }
}

/// pi `isChatGPTSignIn`.
fn is_chatgpt_sign_in(model: &Model, api_key: Option<&str>) -> bool {
    model.provider == "openai"
        && model.base_url == "https://api.openai.com/v1"
        && api_key.is_some_and(|k| !k.starts_with("sk-"))
}

#[derive(Clone, Copy, Debug, Default)]
struct ToolOptions {
    supports_strict_mode: bool,
    supports_grammar: bool,
    tool_search_result: bool,
}

/// pi `convertResponsesTools`.
fn convert_tools(tools: &[ToolDecl], options: ToolOptions) -> Result<Vec<Value>, String> {
    tools
        .iter()
        .map(|tool| {
            if let Some(grammar) = resolve_grammar(tool, options.supports_grammar)? {
                let mut out = json!({
                    "type": "custom",
                    "name": tool.name,
                    "description": tool.description,
                    "format": {"type": "grammar", "syntax": grammar.format, "definition": grammar.definition},
                });
                if options.tool_search_result {
                    out["defer_loading"] = json!(true);
                }
                return Ok(out);
            }
            let strict = resolve_json_schema_strict(tool, options.supports_strict_mode, None)?
                .unwrap_or(false);
            let mut out = JsonObject::new();
            out.insert("type".into(), json!("function"));
            out.insert("name".into(), json!(tool.name));
            out.insert("description".into(), json!(tool.description));
            out.insert("parameters".into(), json_schema_tool_parameters(tool, strict)?);
            if options.tool_search_result {
                out.insert("defer_loading".into(), json!(true));
            }
            if options.supports_strict_mode {
                out.insert("strict".into(), json!(strict));
            }
            Ok(Value::Object(out))
        })
        .collect()
}

fn normalize_id_part(part: &str) -> String {
    let sanitized: String = part
        .encode_utf16()
        .map(|unit| match char::from_u32(u32::from(unit)) {
            Some(c) if c.is_ascii_alphanumeric() || c == '_' || c == '-' => c,
            _ => '_',
        })
        .take(64)
        .collect();
    sanitized.trim_end_matches('_').to_owned()
}

fn normalize_tool_call_id(model: &Model, id: &str, source: &AssistantMessage) -> String {
    if !TOOL_CALL_PROVIDERS.contains(&model.provider.as_str()) || !id.contains('|') {
        return normalize_id_part(id);
    }
    let mut parts = id.split('|');
    let call_id = normalize_id_part(parts.next().unwrap_or_default());
    let item_id = parts.next().unwrap_or_default();
    let foreign = source.provider != model.provider || source.api != model.api;
    let mut item = if foreign {
        let built = format!("fc_{}", short_hash(item_id));
        built.chars().take(64).collect()
    } else {
        normalize_id_part(item_id)
    };
    if !item.starts_with("fc_") {
        item = normalize_id_part(&format!("fc_{item}"));
    }
    format!("{call_id}|{item}")
}

fn parse_text_signature(signature: Option<&str>) -> Option<(String, Option<String>)> {
    let signature = signature.filter(|s| !s.is_empty())?;
    if signature.starts_with('{')
        && let Ok(parsed) = serde_json::from_str::<Value>(signature)
        && parsed["v"] == 1
        && let Some(id) = parsed["id"].as_str()
    {
        let phase = parsed["phase"]
            .as_str()
            .filter(|p| matches!(*p, "commentary" | "final_answer"))
            .map(str::to_owned);
        return Some((id.to_owned(), phase));
    }
    Some((signature.to_owned(), None))
}

fn input_image(data: &str, mime_type: &str) -> Value {
    json!({"type": "input_image", "detail": "auto", "image_url": format!("data:{mime_type};base64,{data}")})
}

fn tool_result_output(model: &Model, content: &[UserContent]) -> Value {
    let text: Vec<&str> = content
        .iter()
        .filter_map(|c| match c {
            UserContent::Text(t) => Some(t.text.as_str()),
            UserContent::Image(_) => None,
        })
        .collect();
    let text = text.join("\n");
    let images: Vec<&oino_ai::ImageContent> = content
        .iter()
        .filter_map(|c| match c {
            UserContent::Image(i) => Some(i),
            UserContent::Text(_) => None,
        })
        .collect();
    let has_text = !text.trim().is_empty();
    if images.is_empty() || !model.input.contains(&InputModality::Image) {
        return json!(if has_text {
            text
        } else if images.is_empty() {
            "(no tool output)".to_owned()
        } else {
            "(see attached image)".to_owned()
        });
    }
    let mut output = Vec::new();
    if has_text {
        output.push(json!({"type": "input_text", "text": text}));
    }
    output.extend(images.iter().map(|i| input_image(&i.data, &i.mime_type)));
    Value::Array(output)
}

fn incremental_tools_notice(unavailable: &BTreeSet<String>, order: &[String]) -> String {
    let notice = "This is an incremental tools update. Previously declared tools remain available unless explicitly marked unavailable. A tool declared again here is available again, and its latest definition replaces the earlier one.";
    let names: Vec<&str> = order
        .iter()
        .filter(|n| unavailable.contains(*n))
        .map(String::as_str)
        .collect();
    if names.is_empty() {
        notice.to_owned()
    } else {
        format!("{notice}\nStill unavailable: {}.", names.join(", "))
    }
}

struct MessageOptions<'a> {
    grammar: &'a BTreeMap<String, String>,
    compat: &'a ResponsesCompat,
    configuration_update_effort: Option<String>,
}

/// pi `convertResponsesMessages`.
#[allow(
    clippy::too_many_lines,
    reason = "1:1 port of pi convertResponsesMessages"
)]
fn convert_messages(
    model: &Model,
    transcript: &Context,
    options: &MessageOptions<'_>,
    now: i64,
) -> Result<Vec<Value>, String> {
    let compat = options.compat;
    let normalized = if compat.supports_mid_convo_system_messages {
        transcript.clone()
    } else {
        collapse_system_messages(transcript.clone())
    };
    let transformed = transform_messages(
        normalized.messages.clone(),
        model,
        Some(&|id: &str, _: &Model, source: &AssistantMessage| {
            normalize_tool_call_id(model, id, source)
        }),
        now,
    );
    let support = tool_support(compat);
    let transcript_tools = resolve_transcript_tools(&normalized.messages, support);
    let tool_options = ToolOptions {
        supports_strict_mode: compat.supports_strict_mode,
        supports_grammar: compat.supports_openai_grammar_tools,
        tool_search_result: false,
    };
    let instruction_role = if model.reasoning
        && model
            .compat
            .as_ref()
            .and_then(|c| c.get("supportsDeveloperRole"))
            .and_then(Value::as_bool)
            != Some(false)
    {
        "developer"
    } else {
        "system"
    };
    let mut messages: Vec<Value> = Vec::new();
    let mut declared_additional = false;
    let mut unavailable: BTreeSet<String> = BTreeSet::new();
    let mut unavailable_order: Vec<String> = Vec::new();
    let mut msg_index = 0usize;
    let mut system_index = 0usize;
    for (source_index, message) in transformed.iter().enumerate() {
        let leading = source_index == 0 && matches!(message, Message::System(_));
        match message {
            Message::System(system) => {
                let anchors = system_index >= transcript_tools.folded_system_messages;
                system_index += 1;
                if anchors {
                    let tools = system.tools_added.clone().unwrap_or_default();
                    for tool in &tools {
                        unavailable.remove(&tool.name);
                    }
                    if !tools.is_empty() {
                        match support {
                            ToolChangeSupport::Changes => {
                                if declared_additional {
                                    messages.push(json!({"role": instruction_role, "content": incremental_tools_notice(&unavailable, &unavailable_order)}));
                                }
                                declared_additional = true;
                                messages.push(json!({"type": "additional_tools", "role": "developer", "tools": convert_tools(&tools, tool_options)?}));
                            }
                            ToolChangeSupport::Additive => {
                                let names: Vec<&str> =
                                    tools.iter().map(|t| t.name.as_str()).collect();
                                let call_id = format!(
                                    "pi_tool_load_{}",
                                    short_hash(&format!("system:{msg_index}:{}", names.join(",")))
                                );
                                messages.push(json!({
                                    "type": "tool_search_call", "call_id": call_id, "execution": "client", "status": "completed",
                                    "arguments": {"query": names.join(" "), "limit": names.len()},
                                }));
                                messages.push(json!({
                                    "type": "tool_search_output", "call_id": call_id, "execution": "client", "status": "completed",
                                    "tools": convert_tools(&tools, ToolOptions { tool_search_result: true, ..tool_options })?,
                                }));
                            }
                            ToolChangeSupport::None => {}
                        }
                    }
                }
                let mut parts = vec![if leading {
                    system_message_text(system)
                } else {
                    render_system_message_update(system)
                }];
                if anchors {
                    let added: BTreeSet<&str> = system
                        .tools_added
                        .iter()
                        .flatten()
                        .map(|t| t.name.as_str())
                        .collect();
                    let removed: Vec<String> = system
                        .tools_removed
                        .iter()
                        .flatten()
                        .map(|t| t.name.clone())
                        .filter(|n| !added.contains(n.as_str()))
                        .collect();
                    if !removed.is_empty() {
                        let lines: Vec<String> = removed.iter().map(|n| format!("- {n}")).collect();
                        parts.push(format!(
                            "The following tools are no longer available. Do not call them:\n{}",
                            lines.join("\n")
                        ));
                    }
                    for name in removed {
                        if unavailable.insert(name.clone()) {
                            unavailable_order.retain(|n| n != &name);
                            unavailable_order.push(name);
                        }
                    }
                }
                let text: Vec<String> = parts.into_iter().filter(|p| !p.is_empty()).collect();
                let text = text.join("\n\n");
                if !text.is_empty() {
                    messages.push(json!({"role": instruction_role, "content": text}));
                }
            }
            Message::User(user) => match &user.content {
                UserContentField::Text(text) => {
                    messages.push(
                        json!({"role": "user", "content": [{"type": "input_text", "text": text}]}),
                    );
                }
                UserContentField::Blocks(blocks) => {
                    let content: Vec<Value> = blocks
                        .iter()
                        .map(|b| match b {
                            UserContent::Text(t) => json!({"type": "input_text", "text": t.text}),
                            UserContent::Image(i) => input_image(&i.data, &i.mime_type),
                        })
                        .collect();
                    if content.is_empty() {
                        continue;
                    }
                    messages.push(json!({"role": "user", "content": content}));
                }
            },
            Message::Assistant(assistant) => {
                let output = assistant_items(model, assistant, options.grammar, msg_index)?;
                if output.is_empty() {
                    continue;
                }
                let same_model = assistant.provider == model.provider
                    && assistant.api == model.api
                    && assistant.model == model.id;
                if options.configuration_update_effort.is_some()
                    && same_model
                    && let Some(level) = &assistant.provider_thinking_level
                {
                    messages.push(
                        json!({"type": "configuration_update", "reasoning": {"effort": level}}),
                    );
                }
                messages.extend(output);
            }
            Message::ToolResult(result) => {
                let call_id = result.tool_call_id.split('|').next().unwrap_or_default();
                let output = tool_result_output(model, &result.content);
                let kind = if options.grammar.contains_key(&result.tool_name) {
                    "custom_tool_call_output"
                } else {
                    "function_call_output"
                };
                messages.push(json!({"type": kind, "call_id": call_id, "output": output}));
            }
        }
        if !leading {
            msg_index += 1;
        }
    }
    if let Some(effort) = &options.configuration_update_effort {
        messages.push(json!({"type": "configuration_update", "reasoning": {"effort": effort}}));
    }
    Ok(messages)
}

fn assistant_items(
    model: &Model,
    assistant: &AssistantMessage,
    grammar: &BTreeMap<String, String>,
    msg_index: usize,
) -> Result<Vec<Value>, String> {
    let same_provider_api = assistant.provider == model.provider && assistant.api == model.api;
    let same_model = same_provider_api && assistant.model == model.id;
    let different_model = same_provider_api && assistant.model != model.id;
    let mut output = Vec::new();
    let mut text_index = 0;
    for block in &assistant.content {
        match block {
            AssistantContent::Thinking(thinking) => {
                if let Some(signature) = thinking
                    .thinking_signature
                    .as_deref()
                    .filter(|s| !s.is_empty())
                {
                    let item: Value = serde_json::from_str(signature).map_err(|e| e.to_string())?;
                    output.push(item);
                }
            }
            AssistantContent::Text(text) => {
                let parsed = parse_text_signature(text.text_signature.as_deref());
                let fallback = if text_index == 0 {
                    format!("msg_pi_{msg_index}")
                } else {
                    format!("msg_pi_{msg_index}_{text_index}")
                };
                text_index += 1;
                let mut id = parsed
                    .as_ref()
                    .map_or(fallback.clone(), |(id, _)| id.clone());
                if id.is_empty() {
                    id = fallback;
                } else if id.encode_utf16().count() > 64 {
                    id = format!("msg_{}", short_hash(&id));
                }
                let mut item = json!({
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text.text, "annotations": []}],
                    "status": "completed",
                    "id": id,
                });
                if let Some((_, Some(phase))) = &parsed {
                    item["phase"] = json!(phase);
                }
                output.push(item);
            }
            AssistantContent::ToolCall(call) => {
                let mut parts = call.id.split('|');
                let call_id = parts.next().unwrap_or_default();
                let item_raw = parts.next();
                let property = grammar.get(&call.name);
                let prefix = if property.is_none() { "fc_" } else { "ctc_" };
                let item_id = item_raw.filter(|i| !different_model && i.starts_with(prefix));
                let mut item = JsonObject::new();
                if let Some(property) = property {
                    item.insert("type".into(), json!("custom_tool_call"));
                    if let Some(id) = item_id {
                        item.insert("id".into(), json!(id));
                    }
                    item.insert("call_id".into(), json!(call_id));
                    item.insert("name".into(), json!(call.name));
                    item.insert(
                        "input".into(),
                        json!(grammar_tool_input(&call.name, &call.arguments, property)?),
                    );
                } else {
                    item.insert("type".into(), json!("function_call"));
                    if let Some(id) = item_id {
                        item.insert("id".into(), json!(id));
                    }
                    item.insert("call_id".into(), json!(call_id));
                    item.insert("name".into(), json!(call.name));
                    item.insert(
                        "arguments".into(),
                        json!(Value::Object(call.arguments.clone()).to_string()),
                    );
                }
                if let (true, Some(namespace)) = (same_model, &call.namespace) {
                    item.insert("namespace".into(), json!(namespace));
                }
                output.push(Value::Object(item));
            }
        }
    }
    Ok(output)
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

/// pi `buildParams` for Responses.
#[allow(clippy::too_many_lines, reason = "1:1 port of pi buildParams")]
fn build_params(
    request: &ProviderRequest,
    context: &Context,
    compat: &ResponsesCompat,
    grammar: &BTreeMap<String, String>,
    effort: Option<ThinkingLevel>,
    configuration_update_effort: Option<&str>,
    now: i64,
) -> Result<Value, String> {
    let model = &request.model;
    let options = &request.options;
    let transcript_tools = resolve_transcript_tools(&context.messages, tool_support(compat));
    let input = convert_messages(
        model,
        context,
        &MessageOptions {
            grammar,
            compat,
            configuration_update_effort: configuration_update_effort.map(str::to_owned),
        },
        now,
    )?;
    let retention = resolve_retention(request);
    let omit = is_chatgpt_sign_in(model, request.api_key.as_deref());
    let mut params = JsonObject::new();
    params.insert("model".into(), json!(model.id));
    params.insert("input".into(), Value::Array(input));
    params.insert("stream".into(), json!(true));
    if retention != CacheRetention::None
        && let Some(session) = &options.session_id
    {
        params.insert(
            "prompt_cache_key".into(),
            json!(clamp_prompt_cache_key(session)),
        );
    }
    if !omit {
        if retention == CacheRetention::Long
            && compat.supports_long_cache_retention
            && !compat.supports_explicit_prompt_cache_mode
        {
            params.insert("prompt_cache_retention".into(), json!("24h"));
        }
        if compat.supports_explicit_prompt_cache_mode {
            if retention == CacheRetention::None {
                params.insert("prompt_cache_options".into(), json!({"mode": "explicit"}));
            } else if retention == CacheRetention::Long && compat.supports_long_cache_retention {
                params.insert("prompt_cache_options".into(), json!({"ttl": "30m"}));
            }
        }
    }
    params.insert("store".into(), json!(false));
    let max_tokens = clamp_max_tokens_to_context(
        model,
        &request.context.messages,
        options.max_tokens.unwrap_or(model.max_tokens),
    );
    if max_tokens > 0 && compat.supports_max_output_tokens && !omit {
        params.insert(
            "max_output_tokens".into(),
            json!(max_tokens.max(MIN_OUTPUT_TOKENS)),
        );
    }
    if let (Some(temperature), false) = (options.temperature, omit) {
        params.insert("temperature".into(), json!(temperature));
    }
    if let Some(tier) = &request.extra.service_tier {
        params.insert("service_tier".into(), json!(tier));
    }
    if !transcript_tools.request_tools.is_empty() {
        params.insert(
            "tools".into(),
            Value::Array(convert_tools(
                &transcript_tools.request_tools,
                ToolOptions {
                    supports_strict_mode: compat.supports_strict_mode,
                    supports_grammar: compat.supports_openai_grammar_tools,
                    tool_search_result: false,
                },
            )?),
        );
    }
    if let Some(choice) = options.tool_choice.as_ref().filter(|c| !c.is_null()) {
        params.insert("tool_choice".into(), choice.clone());
    }
    let summary = request.extra.reasoning_summary.clone();
    let reasoning_effort: Option<Value> = effort
        .map(|level| mapped_or_level(model, level))
        .or_else(|| summary.as_ref().map(|_| json!("medium")));
    if model.reasoning {
        if let Some(requested) = &reasoning_effort {
            let effort_value = if configuration_update_effort.is_some() {
                json!(CONFIGURATION_UPDATE_REQUEST_EFFORT)
            } else {
                requested.clone()
            };
            params.insert(
                "reasoning".into(),
                json!({"effort": effort_value, "summary": summary.clone().filter(|s| !s.is_empty()).unwrap_or_else(|| "auto".to_owned())}),
            );
            params.insert("include".into(), json!(["reasoning.encrypted_content"]));
        } else if model.provider != "github-copilot"
            && mapped_level(model, ThinkingLevel::Off) != MapEntry::Null
        {
            let off = match mapped_level(model, ThinkingLevel::Off) {
                MapEntry::Value(v) => v.to_owned(),
                _ => "none".to_owned(),
            };
            params.insert("reasoning".into(), json!({"effort": off}));
        }
        if model.provider == "xai" {
            params.insert("include".into(), json!(["reasoning.encrypted_content"]));
        }
    }
    let sampling_level = effort.unwrap_or(if summary.is_some() {
        ThinkingLevel::Medium
    } else {
        ThinkingLevel::Off
    });
    let base = resolve_sampling_params(
        model,
        options.reasoning.unwrap_or(ThinkingLevel::Off),
        options.sampling_params.as_ref(),
    );
    if let Some(sampling) = resolve_sampling_params(model, sampling_level, base.as_ref()) {
        for (key, value) in sampling {
            params.insert(key, value);
        }
    }
    Ok(Value::Object(params))
}

/// A prepared Responses request.
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
pub fn prepare(request: &ProviderRequest, now: i64) -> Result<Prepared, String> {
    let model = &request.model;
    let key = request.api_key.clone().filter(|k| !k.is_empty());
    let has_auth_header =
        request.has_header("authorization") || request.has_header("cf-aig-authorization");
    if key.is_none() && !has_auth_header {
        return Err(format!("No API key for provider: {}", model.provider));
    }
    let compat = compat(model);
    let effort = request
        .options
        .reasoning
        .map(|level| clamp_thinking_level(model, level))
        .filter(|level| *level != ThinkingLevel::Off);
    let configuration_update_effort = effort
        .filter(|_| compat.supports_mid_convo_effort && !is_chatgpt_sign_in(model, key.as_deref()))
        .map(|level| match mapped_level(model, level) {
            MapEntry::Value(v) => v.to_owned(),
            _ => level.as_str().to_owned(),
        });
    let normalized = if compat.supports_mid_convo_system_messages {
        request.context.clone()
    } else {
        collapse_system_messages(request.context.clone())
    };
    let grammar = grammar_input_properties(
        &declared_tools(&normalized.messages),
        compat.supports_openai_grammar_tools,
    )?;
    let params = build_params(
        request,
        &normalized,
        &compat,
        &grammar,
        effort,
        configuration_update_effort.as_deref(),
        now,
    )?;
    let retention = resolve_retention(request);
    let mut headers = Headers::new();
    headers.set("accept", "application/json");
    headers.set("content-type", "application/json");
    headers.set(
        "authorization",
        format!("Bearer {}", key.as_deref().unwrap_or("unused")),
    );
    headers.set("user-agent", USER_AGENT);
    for (name, value) in model.headers.iter().flatten() {
        headers.set(name, value);
    }
    if let Some(session) = request
        .options
        .session_id
        .as_deref()
        .filter(|_| retention != CacheRetention::None)
    {
        if compat.session_affinity_format == "openrouter" {
            headers.set("x-session-id", session);
        } else {
            if compat.session_affinity_format == "openai" {
                headers.set("session_id", session);
            }
            headers.set("x-client-request-id", session);
        }
    }
    for (name, value) in request.header_overrides() {
        headers.apply(&name, value.as_deref());
    }
    let mut out = Emitter::new(model, now);
    out.message.provider_thinking_level = configuration_update_effort;
    let prefix = if model.provider == "openai" {
        "OpenAI".to_owned()
    } else {
        model.provider.clone()
    };
    let parser = Parser {
        model: model.clone(),
        grammar,
        service_tier: request.extra.service_tier.clone(),
        slots: BTreeMap::new(),
        reasoning_by_id: BTreeMap::new(),
        saw_terminal: false,
    };
    Ok(Prepared {
        http: HttpRequest {
            url: join_url(&model.base_url, "/responses"),
            headers,
            payload: params,
            timeout_ms: request.options.timeout_ms,
        },
        exchange: Exchange::new(out, Box::new(parser), ErrorStyle::Responses { prefix }),
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SlotKind {
    Thinking,
    Text,
    ToolCall,
}

struct Slot {
    kind: SlotKind,
    index: usize,
    partial_json: Option<String>,
    custom: Option<(String, GrammarBuffer)>,
}

struct Parser {
    model: Model,
    grammar: BTreeMap<String, String>,
    service_tier: Option<String>,
    slots: BTreeMap<i64, Slot>,
    reasoning_by_id: BTreeMap<String, usize>,
    saw_terminal: bool,
}

/// Unfinished tool calls (scratch still present) by content index.
fn unfinished(slots: &BTreeMap<i64, Slot>) -> BTreeSet<usize> {
    slots
        .values()
        .filter(|s| {
            s.kind == SlotKind::ToolCall && (s.partial_json.is_some() || s.custom.is_some())
        })
        .map(|s| s.index)
        .collect()
}

fn map_status(
    status: Option<&str>,
    incomplete: Option<&str>,
) -> Result<(StopReason, Option<String>), StreamError> {
    let Some(status) = status.filter(|s| !s.is_empty()) else {
        return Ok((StopReason::Stop, None));
    };
    Ok(match status {
        "completed" | "in_progress" | "queued" => (StopReason::Stop, None),
        "incomplete" if incomplete == Some("max_output_tokens") => (StopReason::Length, None),
        "incomplete" => (
            StopReason::Error,
            Some(incomplete.map_or_else(
                || "Response incomplete without a provider reason".to_owned(),
                |r| format!("Response incomplete: {r}"),
            )),
        ),
        "failed" | "cancelled" => (StopReason::Error, None),
        other => return Err(format!("Unhandled stop reason: {other}").into()),
    })
}

fn service_tier_multiplier(model: &Model, tier: Option<&str>) -> f64 {
    match tier {
        Some("flex") => 0.5,
        Some("priority" | "fast") => {
            if model.id == "gpt-5.5" {
                2.5
            } else {
                2.0
            }
        }
        _ => 1.0,
    }
}

impl Parser {
    fn create_slot(&mut self, output_index: i64, item: &Value, out: &mut Emitter) {
        let (kind, block, custom, partial) = match item["type"].as_str() {
            Some("reasoning") => (
                SlotKind::Thinking,
                AssistantContent::Thinking(ThinkingContent::default()),
                None,
                None,
            ),
            Some("message") => {
                if item["phase"] == "final_answer" {
                    out.message.stop_reason = StopReason::Stop;
                }
                (
                    SlotKind::Text,
                    AssistantContent::Text(TextContent::new("")),
                    None,
                    None,
                )
            }
            Some("function_call") => (
                SlotKind::ToolCall,
                AssistantContent::ToolCall(ToolCall {
                    id: format!(
                        "{}|{}",
                        item["call_id"].as_str().unwrap_or_default(),
                        item["id"].as_str().unwrap_or_default()
                    ),
                    name: item["name"].as_str().unwrap_or_default().to_owned(),
                    namespace: item["namespace"].as_str().map(str::to_owned),
                    ..ToolCall::default()
                }),
                None,
                Some(item["arguments"].as_str().unwrap_or_default().to_owned()),
            ),
            Some("custom_tool_call") => {
                let name = item["name"].as_str().unwrap_or_default();
                let property = self
                    .grammar
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| "input".to_owned());
                let mut arguments = JsonObject::new();
                arguments.insert(
                    property.clone(),
                    json!(item["input"].as_str().unwrap_or_default()),
                );
                (
                    SlotKind::ToolCall,
                    AssistantContent::ToolCall(ToolCall {
                        id: format!(
                            "{}|{}",
                            item["call_id"].as_str().unwrap_or_default(),
                            item["id"].as_str().unwrap_or_default()
                        ),
                        name: name.to_owned(),
                        arguments,
                        namespace: item["namespace"].as_str().map(str::to_owned),
                        ..ToolCall::default()
                    }),
                    Some((property, GrammarBuffer::default())),
                    None,
                )
            }
            _ => return,
        };
        let (id, name) = match &block {
            AssistantContent::ToolCall(call) => (Some(call.id.clone()), Some(call.name.clone())),
            _ => (None, None),
        };
        let index = out.add(block);
        self.slots.insert(
            output_index,
            Slot {
                kind,
                index,
                partial_json: partial,
                custom,
            },
        );
        out.push(match kind {
            SlotKind::Thinking => ProviderEvent::ThinkingStart {
                content_index: index,
            },
            SlotKind::Text => ProviderEvent::TextStart {
                content_index: index,
            },
            SlotKind::ToolCall => ProviderEvent::ToolcallStart {
                content_index: index,
                id,
                tool_name: name,
            },
        });
    }

    fn slot(&mut self, event: &Value, kind: SlotKind) -> Option<&mut Slot> {
        let output_index = event["output_index"].as_i64()?;
        self.slots.get_mut(&output_index).filter(|s| s.kind == kind)
    }

    fn custom_input(out: &mut Emitter, index: usize, property: &str) -> String {
        match out.block(index) {
            Some(AssistantContent::ToolCall(call)) => call
                .arguments
                .get(property)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned(),
            _ => String::new(),
        }
    }

    fn append_custom(
        slot: &mut Slot,
        out: &mut Emitter,
        next: &str,
        close: bool,
    ) -> Result<(), StreamError> {
        let Some((property, buffer)) = slot.custom.as_mut() else {
            return Ok(());
        };
        let delta = append_grammar_delta(buffer, property, next, close)?;
        if let Some(AssistantContent::ToolCall(call)) = out.block(slot.index) {
            call.arguments = JsonObject::new();
            call.arguments.insert(property.clone(), json!(next));
        }
        if let Some(delta) = delta {
            out.push(ProviderEvent::ToolcallDelta {
                content_index: slot.index,
                delta,
            });
        }
        Ok(())
    }

    fn thinking_delta(&mut self, event: &Value, out: &mut Emitter, text: &str) {
        let Some(index) = self.slot(event, SlotKind::Thinking).map(|s| s.index) else {
            return;
        };
        if let Some(AssistantContent::Thinking(block)) = out.block(index) {
            block.thinking.push_str(text);
        }
        out.push(ProviderEvent::ThinkingDelta {
            content_index: index,
            delta: text.to_owned(),
        });
    }

    fn text_delta(&mut self, event: &Value, out: &mut Emitter) {
        let Some(index) = self.slot(event, SlotKind::Text).map(|s| s.index) else {
            return;
        };
        let text = event["delta"].as_str().unwrap_or_default();
        if let Some(AssistantContent::Text(block)) = out.block(index) {
            block.text.push_str(text);
        }
        out.push(ProviderEvent::TextDelta {
            content_index: index,
            delta: text.to_owned(),
        });
    }

    fn arguments(&mut self, event: &Value, out: &mut Emitter, done: bool) {
        let Some(slot) = self.slot(event, SlotKind::ToolCall) else {
            return;
        };
        let Some(previous) = slot.partial_json.clone() else {
            return;
        };
        let (next, delta) = if done {
            let full = event["arguments"].as_str().unwrap_or_default().to_owned();
            let delta = full
                .strip_prefix(previous.as_str())
                .filter(|d| !d.is_empty())
                .map(str::to_owned);
            (full, delta)
        } else {
            let delta = event["delta"].as_str().unwrap_or_default().to_owned();
            (format!("{previous}{delta}"), Some(delta))
        };
        let index = slot.index;
        slot.partial_json = Some(next.clone());
        if let Some(AssistantContent::ToolCall(call)) = out.block(index) {
            call.arguments = parse_streaming_object(&next);
        }
        if let Some(delta) = delta {
            out.push(ProviderEvent::ToolcallDelta {
                content_index: index,
                delta,
            });
        }
    }

    #[allow(clippy::too_many_lines, reason = "one arm per output item type")]
    fn item_done(&mut self, event: &Value, out: &mut Emitter) -> Result<(), StreamError> {
        let item = &event["item"];
        if item["type"] == "message" && item["phase"] == "final_answer" {
            out.message.stop_reason = StopReason::Stop;
        }
        let Some(output_index) = event["output_index"].as_i64() else {
            return Ok(());
        };
        if !self.slots.contains_key(&output_index) {
            self.create_slot(output_index, item, out);
        }
        let Some(mut slot) = self.slots.remove(&output_index) else {
            return Ok(());
        };
        let index = slot.index;
        match (item["type"].as_str(), slot.kind) {
            (Some("reasoning"), SlotKind::Thinking) => {
                let join = |key: &str, field: &str| -> String {
                    item[key]
                        .as_array()
                        .map(|list| {
                            list.iter()
                                .map(|s| s[field].as_str().unwrap_or_default())
                                .collect::<Vec<_>>()
                                .join("\n\n")
                        })
                        .unwrap_or_default()
                };
                let summary = join("summary", "text");
                let content = join("content", "text");
                if let Some(AssistantContent::Thinking(block)) = out.block(index) {
                    if !summary.is_empty() {
                        block.thinking = summary;
                    } else if !content.is_empty() {
                        block.thinking = content;
                    }
                    block.thinking_signature = Some(item.to_string());
                    let content = block.thinking.clone();
                    if let Some(id) = item["id"].as_str() {
                        self.reasoning_by_id.insert(id.to_owned(), index);
                    }
                    out.push(ProviderEvent::ThinkingEnd {
                        content_index: index,
                        content,
                    });
                }
            }
            (Some("message"), SlotKind::Text) => {
                let text: String = item["content"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|c| {
                        if c["type"] == "output_text" {
                            c["text"].as_str().unwrap_or_default()
                        } else {
                            c["refusal"].as_str().unwrap_or_default()
                        }
                    })
                    .collect();
                let mut signature = json!({"v": 1, "id": item["id"]});
                if let Some(phase) = item["phase"].as_str() {
                    signature["phase"] = json!(phase);
                }
                if let Some(AssistantContent::Text(block)) = out.block(index) {
                    block.text.clone_from(&text);
                    block.text_signature = Some(signature.to_string());
                }
                out.push(ProviderEvent::TextEnd {
                    content_index: index,
                    content: text,
                });
            }
            (Some("function_call"), SlotKind::ToolCall) if slot.partial_json.is_some() => {
                let source = item["arguments"]
                    .as_str()
                    .filter(|a| !a.is_empty())
                    .map_or_else(
                        || slot.partial_json.clone().unwrap_or_default(),
                        str::to_owned,
                    );
                let parsed = parse_tool_call_arguments(&source);
                if let Some(AssistantContent::ToolCall(call)) = out.block(index) {
                    call.arguments = parsed.arguments;
                    call.arguments_error = parsed.error;
                    if let Some(namespace) = item["namespace"].as_str() {
                        call.namespace = Some(namespace.to_owned());
                    }
                    let tool_call = call.clone();
                    out.push(ProviderEvent::ToolcallEnd {
                        content_index: index,
                        tool_call,
                    });
                }
            }
            (Some("custom_tool_call"), SlotKind::ToolCall) if slot.custom.is_some() => {
                let property = slot
                    .custom
                    .as_ref()
                    .map(|(p, _)| p.clone())
                    .unwrap_or_default();
                let input = item["input"]
                    .as_str()
                    .map_or_else(|| Self::custom_input(out, index, &property), str::to_owned);
                Self::append_custom(&mut slot, out, &input, true)?;
                if let Some(AssistantContent::ToolCall(call)) = out.block(index) {
                    if let Some(namespace) = item["namespace"].as_str() {
                        call.namespace = Some(namespace.to_owned());
                    }
                    let tool_call = call.clone();
                    out.push(ProviderEvent::ToolcallEnd {
                        content_index: index,
                        tool_call,
                    });
                }
            }
            _ => {
                // Not finalised: keep the slot (pi only deletes finished slots).
                self.slots.insert(output_index, slot);
            }
        }
        Ok(())
    }

    fn finalize_response(
        &mut self,
        response: &Value,
        out: &mut Emitter,
    ) -> Result<(), StreamError> {
        self.saw_terminal = true;
        for item in response["output"].as_array().into_iter().flatten() {
            let Some(encrypted) = item["encrypted_content"].as_str().filter(|e| !e.is_empty())
            else {
                continue;
            };
            if item["type"] != "reasoning" {
                continue;
            }
            let Some(index) = item["id"]
                .as_str()
                .and_then(|id| self.reasoning_by_id.get(id))
            else {
                continue;
            };
            if let Some(AssistantContent::Thinking(block)) = out.block(*index) {
                let Some(stored) = block
                    .thinking_signature
                    .as_deref()
                    .and_then(|s| serde_json::from_str::<Value>(s).ok())
                else {
                    continue;
                };
                if stored["encrypted_content"]
                    .as_str()
                    .is_some_and(|e| !e.is_empty())
                {
                    continue;
                }
                let mut stored = stored;
                stored["encrypted_content"] = json!(encrypted);
                block.thinking_signature = Some(stored.to_string());
            }
        }
        if let Some(id) = response["id"].as_str().filter(|i| !i.is_empty()) {
            out.message.response_id = Some(id.to_owned());
        }
        let usage = &response["usage"];
        if usage.is_object() {
            let cached = usage["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0);
            let write = usage["input_tokens_details"]["cache_write_tokens"]
                .as_u64()
                .unwrap_or(0);
            let u = &mut out.message.usage;
            u.input = usage["input_tokens"]
                .as_u64()
                .unwrap_or(0)
                .saturating_sub(cached)
                .saturating_sub(write);
            u.output = usage["output_tokens"].as_u64().unwrap_or(0);
            u.cache_read = cached;
            u.cache_write = write;
            u.cache_write_1h = None;
            u.reasoning = Some(
                usage["output_tokens_details"]["reasoning_tokens"]
                    .as_u64()
                    .unwrap_or(0),
            );
            u.total_tokens = usage["total_tokens"].as_u64().unwrap_or(0);
        }
        calculate_cost(&self.model, &mut out.message.usage);
        let tier = response["service_tier"]
            .as_str()
            .map(str::to_owned)
            .or_else(|| self.service_tier.clone());
        let multiplier = service_tier_multiplier(&self.model, tier.as_deref());
        #[allow(
            clippy::float_cmp,
            reason = "pi compares the multiplier with 1 exactly"
        )]
        if multiplier != 1.0 {
            let cost = &mut out.message.usage.cost;
            cost.input *= multiplier;
            cost.output *= multiplier;
            cost.cache_read *= multiplier;
            cost.cache_write *= multiplier;
            cost.total = cost.input + cost.output + cost.cache_read + cost.cache_write;
        }
        let status = response["status"].as_str();
        let incomplete = response["incomplete_details"]["reason"].as_str();
        out.message.raw_stop_reason = match (status, incomplete) {
            (Some(s), Some(r)) => Some(format!("{s}.{r}")),
            (Some(s), None) => Some(s.to_owned()),
            (None, _) => None,
        };
        let (stop, message) = map_status(status, incomplete)?;
        out.message.stop_reason = stop;
        out.message.error_message = message;
        if has_tool_call(out) && out.message.stop_reason == StopReason::Stop {
            out.message.stop_reason = StopReason::ToolUse;
        }
        Ok(())
    }
}

impl StreamParser for Parser {
    fn event(&mut self, sse: &SseEvent, out: &mut Emitter) -> Result<Step, StreamError> {
        if sse.data == "[DONE]" {
            return Ok(Step::End);
        }
        let event: Value = serde_json::from_str(&sse.data).map_err(|_| {
            StreamError::from("Error reading response: malformed server-sent event JSON.")
        })?;
        if sse.event.as_deref() == Some("error")
            || event
                .get("error")
                .is_some_and(|e| !e.is_null() && e != &json!(false))
                && event["type"] != "response.failed"
        {
            return Err(StreamError::Sdk(openai_stream_error(&event)));
        }
        match event["type"].as_str().unwrap_or_default() {
            "response.created" => {
                out.message.response_id = event["response"]["id"].as_str().map(str::to_owned);
            }
            "response.output_item.added" => {
                if let Some(index) = event["output_index"].as_i64() {
                    self.create_slot(index, &event["item"], out);
                }
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                let text = event["delta"].as_str().unwrap_or_default().to_owned();
                self.thinking_delta(&event, out, &text);
            }
            "response.reasoning_summary_part.done" => self.thinking_delta(&event, out, "\n\n"),
            "response.output_text.delta" | "response.refusal.delta" => self.text_delta(&event, out),
            "response.function_call_arguments.delta" => self.arguments(&event, out, false),
            "response.function_call_arguments.done" => self.arguments(&event, out, true),
            "response.custom_tool_call_input.delta" | "response.custom_tool_call_input.done" => {
                let done = event["type"] == "response.custom_tool_call_input.done";
                if let Some(slot) = self.slot(&event, SlotKind::ToolCall)
                    && let Some(property) = slot.custom.as_ref().map(|(p, _)| p.clone())
                {
                    let next = if done {
                        event["input"].as_str().unwrap_or_default().to_owned()
                    } else {
                        format!(
                            "{}{}",
                            Self::custom_input(out, slot.index, &property),
                            event["delta"].as_str().unwrap_or_default()
                        )
                    };
                    Self::append_custom(slot, out, &next, done)?;
                }
            }
            "response.output_item.done" => self.item_done(&event, out)?,
            "response.completed" | "response.incomplete" => {
                self.finalize_response(&event["response"], out)?;
            }
            "error" => {
                return Err(format!(
                    "Error Code {}: {}",
                    event["code"].as_str().unwrap_or("undefined"),
                    event["message"].as_str().unwrap_or("undefined")
                )
                .into());
            }
            "response.failed" => {
                self.saw_terminal = true;
                let response = &event["response"];
                out.message.raw_stop_reason = response["status"].as_str().map(str::to_owned);
                let error = &response["error"];
                let message = if error.is_object() {
                    format!(
                        "{}: {}",
                        error["code"]
                            .as_str()
                            .filter(|c| !c.is_empty())
                            .unwrap_or("unknown"),
                        error["message"]
                            .as_str()
                            .filter(|m| !m.is_empty())
                            .unwrap_or("no message")
                    )
                } else if let Some(reason) = response["incomplete_details"]["reason"].as_str() {
                    format!("incomplete: {reason}")
                } else {
                    "Unknown error (no error details in response)".to_owned()
                };
                return Err(message.into());
            }
            _ => {}
        }
        Ok(Step::Event(event))
    }

    fn finish(&mut self, out: &mut Emitter) -> Result<(), StreamError> {
        if !self.saw_terminal {
            return Err("OpenAI Responses stream ended before a terminal response event".into());
        }
        if out.message.stop_reason == StopReason::ToolUse {
            let open = unfinished(&self.slots);
            for (index, block) in out.message.content.iter().enumerate() {
                if let AssistantContent::ToolCall(call) = block
                    && open.contains(&index)
                {
                    return Err(format!(
                        "OpenAI Responses stream completed with an unfinished tool call: {} ({})",
                        call.name, call.id
                    )
                    .into());
                }
            }
        }
        match out.message.stop_reason {
            StopReason::Pending => {
                Err("OpenAI Responses stream ended without a stop reason".into())
            }
            StopReason::Aborted | StopReason::Error => Err(out
                .message
                .error_message
                .clone()
                .unwrap_or_else(|| "An unknown error occurred".to_owned())
                .into()),
            _ => Ok(()),
        }
    }
}
