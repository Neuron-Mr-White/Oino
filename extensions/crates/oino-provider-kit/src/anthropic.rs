//! Faithful port of pi `api/anthropic-messages.ts`: request building (system
//! blocks, betas, cache_control, thinking config, native mid-conversation
//! tool changes, tool/message conversion) and SSE event parsing.

use std::collections::BTreeMap;

use oino_types::{
    transcript, AssistantStreamEvent, ContentBlock, Message, Model, OinoId, ProviderMetadata,
    StopReason, Usage,
};
use serde_json::{json, Map, Value};

use crate::catalog::CatalogModel;
use crate::piutil;
use crate::{CacheRetention, KitError, StreamRequest};

const FINE_GRAINED_TOOL_STREAMING_BETA: &str = "fine-grained-tool-streaming-2025-05-14";
const INTERLEAVED_THINKING_BETA: &str = "interleaved-thinking-2025-05-14";
const SERVER_SIDE_FALLBACK_BETA: &str = "server-side-fallback-2026-07-01";
const MID_CONVERSATION_OUTPUT_CONFIG_BETA: &str = "mid-conversation-output-config-2026-07-01";
const THINKING_BINDING_CONTROLS_BETA: &str = "thinking-binding-controls-2026-08-01";
const MID_CONVERSATION_TOOL_CHANGES_BETA: &str = "mid-conversation-tool-changes-2026-07-01";

fn deferred_tool_placeholder() -> Value {
    json!({
        "name": "__pi_deferred_placeholder__",
        "description": "Reserved placeholder. Never available. Never call this.",
        "input_schema": {"type": "object", "properties": {}, "required": []},
        "defer_loading": true,
    })
}

/// Anthropic compat flags (pi `getAnthropicCompat`).
pub struct AnthropicCompat {
    pub supports_eager_tool_input_streaming: bool,
    pub supports_long_cache_retention: bool,
    pub send_session_affinity_headers: bool,
    pub session_affinity_format: Option<String>,
    pub supports_cache_control_on_tools: bool,
    pub supports_temperature: bool,
    pub allow_empty_signature: bool,
    pub supports_strict_tools: bool,
    pub supports_mid_convo_system_messages: bool,
    pub supports_mid_convo_tool_changes: bool,
    pub supports_mid_convo_effort: bool,
    pub force_adaptive_thinking: bool,
    pub allowed_fallback_models: Vec<FallbackModel>,
}

#[derive(Debug, Clone)]
pub struct FallbackModel {
    pub model: String,
    pub cost: crate::piutil::CostRates,
}

#[must_use]
pub fn anthropic_compat(model: &CatalogModel) -> AnthropicCompat {
    let is_openrouter = model.provider == "openrouter" || model.base_url.contains("openrouter.ai");
    let compat = &model.compat;
    let flag = |k: &str, d: bool| compat.get(k).and_then(|v| v.as_bool()).unwrap_or(d);
    AnthropicCompat {
        supports_eager_tool_input_streaming: flag("supportsEagerToolInputStreaming", true),
        supports_long_cache_retention: flag("supportsLongCacheRetention", true),
        send_session_affinity_headers: flag("sendSessionAffinityHeaders", is_openrouter),
        session_affinity_format: compat
            .get("sessionAffinityFormat")
            .and_then(|v| v.as_str())
            .map(str::to_string)
            .or_else(|| is_openrouter.then(|| "openrouter".to_string())),
        supports_cache_control_on_tools: flag("supportsCacheControlOnTools", true),
        supports_temperature: flag("supportsTemperature", true),
        allow_empty_signature: flag("allowEmptySignature", false),
        supports_strict_tools: flag("supportsStrictTools", false),
        supports_mid_convo_system_messages: flag("supportsMidConvoSystemMessages", false),
        supports_mid_convo_tool_changes: flag("supportsMidConvoToolChanges", false),
        supports_mid_convo_effort: flag("supportsMidConvoEffort", false),
        force_adaptive_thinking: flag("forceAdaptiveThinking", false),
        allowed_fallback_models: compat
            .get("allowedFallbackModels")
            .and_then(|v| v.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| {
                        Some(FallbackModel {
                            model: item.get("model")?.as_str()?.to_string(),
                            cost: serde_json::from_value(
                                item.get("cost").cloned().unwrap_or_default(),
                            )
                            .unwrap_or_default(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default(),
    }
}

/// Options Oino maps onto pi's AnthropicOptions (no temperature/toolChoice/
/// metadata in Oino's request surface).
pub struct AnthropicRequestOptions<'a> {
    pub thinking_enabled: Option<bool>,
    pub thinking_budget_tokens: Option<u64>,
    pub effort: Option<&'a str>,
    pub thinking_display: Option<&'a str>,
    pub interleaved_thinking: bool,
    pub api_key: Option<&'a str>,
    pub max_tokens: Option<u64>,
}

impl Default for AnthropicRequestOptions<'_> {
    fn default() -> Self {
        Self {
            thinking_enabled: None,
            thinking_budget_tokens: None,
            effort: None,
            thinking_display: None,
            interleaved_thinking: true,
            api_key: None,
            max_tokens: None,
        }
    }
}

fn is_oauth_token(api_key: &str) -> bool {
    api_key.contains("sk-ant-oat")
}

fn cache_control(model: &CatalogModel, retention: CacheRetention) -> Option<Value> {
    if retention == CacheRetention::None {
        return None;
    }
    let compat = anthropic_compat(model);
    let ttl =
        (retention == CacheRetention::Long && compat.supports_long_cache_retention).then_some("1h");
    Some(match ttl {
        Some(ttl) => json!({"type": "ephemeral", "ttl": ttl}),
        None => json!({"type": "ephemeral"}),
    })
}

#[allow(dead_code)]
fn normalize_tool_call_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

fn convert_content_blocks(content: &[ContentBlock]) -> Value {
    let has_images = content
        .iter()
        .any(|b| matches!(b, ContentBlock::Image { .. }));
    if !has_images {
        return Value::String(piutil::sanitize_surrogates(
            &content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    let mut blocks: Vec<Value> = content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text, .. } => Some(json!({
                "type": "text",
                "text": piutil::sanitize_surrogates(text),
            })),
            ContentBlock::Image { media_type, data } => Some(json!({
                "type": "image",
                "source": {"type": "base64", "media_type": media_type, "data": data},
            })),
            _ => None,
        })
        .collect();
    let has_text = blocks
        .iter()
        .any(|b| b.get("type").and_then(|t| t.as_str()) == Some("text"));
    if !has_text {
        blocks.insert(0, json!({"type": "text", "text": "(see attached image)"}));
    }
    Value::Array(blocks)
}

fn convert_tool_result(message: &Message, native_ids: &BTreeMap<OinoId, String>) -> Value {
    let Message::ToolResult {
        tool_call_id,
        content,
        is_error,
        ..
    } = message
    else {
        return Value::Null;
    };
    let tool_use_id = native_ids
        .get(tool_call_id)
        .cloned()
        .unwrap_or_else(|| tool_call_id.to_string());
    json!({
        "type": "tool_result",
        "tool_use_id": tool_use_id,
        "content": convert_content_blocks(content),
        "is_error": is_error,
    })
}

fn convert_tools(
    tools: &[oino_types::ToolDeclaration],
    is_oauth: bool,
    supports_eager: bool,
    supports_strict: bool,
    cache_control: Option<&Value>,
) -> Vec<Value> {
    tools
        .iter()
        .enumerate()
        .map(|(index, tool)| {
            let schema = &tool.input_schema;
            let mut tool_json = json!({
                "name": tool.name,
                "description": tool.description,
            });
            if supports_eager {
                tool_json["eager_input_streaming"] = json!(true);
            }
            let _ = is_oauth;
            let _ = supports_strict;
            tool_json["input_schema"] = json!({
                "type": "object",
                "properties": schema.get("properties").cloned().unwrap_or_else(|| json!({})),
                "required": schema.get("required").cloned().unwrap_or_else(|| json!([])),
            });
            if let (Some(cc), true) = (cache_control, index == tools.len() - 1) {
                tool_json["cache_control"] = cc.clone();
            }
            tool_json
        })
        .collect()
}

/// Map assistant tool-call ids to their provider-native wire ids.
#[must_use]
pub fn native_id_map(messages: &[Message]) -> BTreeMap<OinoId, String> {
    let mut map = BTreeMap::new();
    for message in messages {
        let Message::Assistant { content, .. } = message else {
            continue;
        };
        for block in content {
            if let ContentBlock::ToolCall { id, native_id, .. } = block {
                map.insert(*id, native_id.clone().unwrap_or_else(|| id.to_string()));
            }
        }
    }
    map
}

fn is_anthropic_effort(value: &str) -> bool {
    matches!(value, "low" | "medium" | "high" | "xhigh" | "max")
}

fn assistant_provider(message: &Message) -> Option<(&str, &str)> {
    let Message::Assistant { provider, .. } = message else {
        return None;
    };
    provider
        .as_ref()?
        .model
        .as_ref()
        .map(|m| (m.provider.as_str(), m.name.as_str()))
}

fn convert_messages(
    transformed_messages: &[Message],
    _is_oauth: bool,
    cache_control: Option<&Value>,
    allow_empty_signature: bool,
    managed_provider: Option<&str>,
    native_tool_changes: bool,
) -> (Vec<Value>, BTreeMap<usize, String>) {
    let mut params: Vec<Value> = Vec::new();
    let native_ids = native_id_map(transformed_messages);
    let mut assistant_levels: BTreeMap<usize, String> = BTreeMap::new();
    let mut pending_system: Vec<Value> = Vec::new();

    let mut i = 0;
    while i < transformed_messages.len() {
        let msg = &transformed_messages[i];
        match msg {
            Message::System {
                tools_added,
                tools_removed,
                ..
            } => {
                let text = piutil::render_system_message_update(msg);
                let mut blocks: Vec<Value> = Vec::new();
                if !text.is_empty() {
                    blocks
                        .push(json!({"type": "text", "text": piutil::sanitize_surrogates(&text)}));
                }
                if native_tool_changes {
                    for name in tools_removed {
                        blocks.push(json!({
                            "type": "tool_removal",
                            "tool": {"type": "tool_reference", "name": name},
                        }));
                    }
                    for tool in tools_added {
                        blocks.push(json!({
                            "type": "tool_addition",
                            "tool": {"type": "tool_reference", "name": tool.name},
                        }));
                    }
                }
                if !blocks.is_empty() {
                    pending_system.push(json!({"role": "system", "content": blocks}));
                }
            }
            Message::User { content, .. } => {
                let text: String = content
                    .iter()
                    .filter_map(|b| match b {
                        ContentBlock::Text { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                let has_non_text = content
                    .iter()
                    .any(|b| !matches!(b, ContentBlock::Text { .. }));
                if !has_non_text {
                    if !text.trim().is_empty() {
                        params.push(json!({
                            "role": "user",
                            "content": piutil::sanitize_surrogates(&text),
                        }));
                    }
                } else {
                    let mut blocks: Vec<Value> = Vec::new();
                    for item in content {
                        match item {
                            ContentBlock::Text { text, .. } => {
                                if !text.trim().is_empty() {
                                    blocks.push(json!({
                                        "type": "text",
                                        "text": piutil::sanitize_surrogates(text),
                                    }));
                                }
                            }
                            ContentBlock::Image { media_type, data } => {
                                blocks.push(json!({
                                    "type": "image",
                                    "source": {"type": "base64", "media_type": media_type, "data": data},
                                }));
                            }
                            _ => {}
                        }
                    }
                    if !blocks.is_empty() {
                        params.push(json!({"role": "user", "content": blocks}));
                    }
                }
            }
            Message::Assistant { content, .. } => {
                params.append(&mut pending_system);
                let mut blocks: Vec<Value> = Vec::new();
                for block in content {
                    match block {
                        ContentBlock::Text { text, .. } => {
                            if text.trim().is_empty() {
                                continue;
                            }
                            blocks.push(json!({
                                "type": "text",
                                "text": piutil::sanitize_surrogates(text),
                            }));
                        }
                        ContentBlock::Thinking {
                            text,
                            redacted,
                            signature,
                            ..
                        } => {
                            if *redacted {
                                blocks.push(json!({
                                    "type": "redacted_thinking",
                                    "data": signature.clone().unwrap_or_default(),
                                }));
                                continue;
                            }
                            let has_signature =
                                signature.as_deref().is_some_and(|s| !s.trim().is_empty());
                            if text.trim().is_empty() && !has_signature {
                                continue;
                            }
                            if !has_signature {
                                if allow_empty_signature {
                                    blocks.push(json!({
                                        "type": "thinking",
                                        "thinking": piutil::sanitize_surrogates(text),
                                        "signature": "",
                                    }));
                                } else {
                                    blocks.push(json!({
                                        "type": "text",
                                        "text": piutil::sanitize_surrogates(text),
                                    }));
                                }
                            } else {
                                blocks.push(json!({
                                    "type": "thinking",
                                    "thinking": piutil::sanitize_surrogates(text),
                                    "signature": signature,
                                }));
                            }
                        }
                        ContentBlock::ToolCall {
                            id,
                            name,
                            arguments,
                            native_id,
                        } => {
                            blocks.push(json!({
                                "type": "tool_use",
                                "id": native_id.clone().unwrap_or_else(|| id.to_string()),
                                "name": name,
                                "input": arguments,
                            }));
                        }
                        ContentBlock::Image { .. } => {}
                    }
                }
                if !blocks.is_empty() {
                    let message_index = params.len();
                    params.push(json!({"role": "assistant", "content": blocks}));
                    if let Some(provider) = managed_provider {
                        if let Some((p, _)) = assistant_provider(msg) {
                            if p == provider {
                                if let Message::Assistant {
                                    provider_thinking_level: Some(level),
                                    ..
                                } = msg
                                {
                                    if is_anthropic_effort(level) {
                                        assistant_levels.insert(message_index, level.clone());
                                    }
                                }
                            }
                        }
                    }
                }
            }
            Message::ToolResult { .. } => {
                let mut tool_results: Vec<Value> = Vec::new();
                let mut j = i;
                while j < transformed_messages.len() {
                    if matches!(transformed_messages[j], Message::ToolResult { .. }) {
                        tool_results
                            .push(convert_tool_result(&transformed_messages[j], &native_ids));
                        j += 1;
                    } else {
                        break;
                    }
                }
                i = j - 1;
                params.push(json!({"role": "user", "content": tool_results}));
            }
            _ => {}
        }
        i += 1;
    }
    params.append(&mut pending_system);

    // cache_control on the last user/system message's last block.
    if let (Some(cc), Some(last)) = (cache_control, params.last_mut()) {
        if matches!(
            last.get("role").and_then(|r| r.as_str()),
            Some("user") | Some("system")
        ) {
            if let Some(blocks) = last.get_mut("content").and_then(|c| c.as_array_mut()) {
                if let Some(block) = blocks.last_mut() {
                    if matches!(
                        block.get("type").and_then(|t| t.as_str()),
                        Some("text")
                            | Some("image")
                            | Some("tool_result")
                            | Some("tool_addition")
                            | Some("tool_removal")
                    ) {
                        block["cache_control"] = cc.clone();
                    }
                }
            } else if let Some(text) = last.get("content").and_then(|c| c.as_str()) {
                last["content"] = json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": cc,
                }]);
            }
        }
    }
    (params, assistant_levels)
}

fn insert_thinking_level_messages(
    messages: &[Value],
    levels: &BTreeMap<usize, String>,
    active_effort: &str,
) -> Vec<Value> {
    let mut out = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        if let Some(effort) = levels.get(&index) {
            out.push(json!({
                "role": "system",
                "content": [],
                "output_config": {"effort": effort},
            }));
        }
        out.push(message.clone());
    }
    out.push(json!({
        "role": "system",
        "content": [],
        "output_config": {"effort": active_effort},
    }));
    out
}

fn should_use_fine_grained_tool_streaming(
    model: &CatalogModel,
    context_messages: &[Message],
) -> bool {
    !transcript::current_tools(context_messages).is_empty()
        && !anthropic_compat(model).supports_eager_tool_input_streaming
}

fn should_use_server_side_fallback(model: &CatalogModel) -> bool {
    !anthropic_compat(model).allowed_fallback_models.is_empty()
}

fn beta_features(
    model: &CatalogModel,
    context_messages: &[Message],
    is_oauth: bool,
    native_tool_changes: bool,
    options: &AnthropicRequestOptions<'_>,
    headers: &BTreeMap<String, String>,
) -> Vec<String> {
    // configured anthropic-beta header wins
    for headers in [headers] {
        for (name, value) in headers {
            if name.eq_ignore_ascii_case("anthropic-beta") {
                return value
                    .split(',')
                    .map(str::trim)
                    .filter(|f| !f.is_empty())
                    .map(str::to_string)
                    .collect::<std::collections::BTreeSet<_>>()
                    .into_iter()
                    .collect();
            }
        }
    }
    let mut features: Vec<String> = Vec::new();
    if is_oauth {
        features.push("claude-code-20250219".into());
        features.push("oauth-2025-04-20".into());
    }
    if should_use_fine_grained_tool_streaming(model, context_messages) {
        features.push(FINE_GRAINED_TOOL_STREAMING_BETA.into());
    }
    if model.reasoning
        && options.thinking_enabled == Some(true)
        && options.interleaved_thinking
        && !anthropic_compat(model).force_adaptive_thinking
    {
        features.push(INTERLEAVED_THINKING_BETA.into());
    }
    if should_use_server_side_fallback(model) {
        features.push(SERVER_SIDE_FALLBACK_BETA.into());
    }
    if anthropic_compat(model).supports_mid_convo_effort {
        features.push(MID_CONVERSATION_OUTPUT_CONFIG_BETA.into());
        features.push(THINKING_BINDING_CONTROLS_BETA.into());
    }
    if native_tool_changes {
        features.push(MID_CONVERSATION_TOOL_CHANGES_BETA.into());
    }
    // pi keeps first-occurrence order (`[...new Set(features)]`).
    let mut seen = std::collections::BTreeSet::new();
    features.retain(|f| seen.insert(f.clone()));
    features
}

/// Build the Anthropic `/v1/messages?beta=true` request body (pi
/// `buildParams`), plus the `betas` list for the `anthropic-beta` header.
pub fn build_params(
    model: &CatalogModel,
    request: &StreamRequest,
    headers: &BTreeMap<String, String>,
    options: &AnthropicRequestOptions<'_>,
) -> Result<(Value, Vec<String>), KitError> {
    let compat = anthropic_compat(model);
    let normalized = transcript::resolve(
        transcript::Transcript::new(request.transcript.messages.clone()),
        compat.supports_mid_convo_system_messages,
    );
    let context_messages = normalized.messages().to_vec();
    let is_oauth = options.api_key.is_some_and(is_oauth_token);
    let cc = cache_control(model, request.cache_retention);

    let initial = piutil::initial_system_message(&context_messages);
    let initial_system_text = initial.map(piutil::system_message_text).unwrap_or_default();
    let transformed = piutil::transform_messages(&context_messages, &model.facets(), None);
    let conversation_messages: &[Message] = if initial.is_some() {
        &transformed[1..]
    } else {
        &transformed
    };
    let initial_tools: Vec<oino_types::ToolDeclaration> = initial
        .map(|m| match m {
            Message::System { tools_added, .. } => tools_added.clone(),
            _ => Vec::new(),
        })
        .unwrap_or_default();
    let native_tool_changes = compat.supports_mid_convo_system_messages
        && compat.supports_mid_convo_tool_changes
        && !initial_tools.is_empty()
        && !piutil::has_tool_redefinitions(&context_messages);
    let managed_provider = compat
        .supports_mid_convo_effort
        .then_some(model.provider.as_str());
    let (converted, assistant_levels) = convert_messages(
        conversation_messages,
        is_oauth,
        cc.as_ref(),
        compat.allow_empty_signature,
        managed_provider,
        native_tool_changes,
    );
    let active_effort = options.effort.unwrap_or("high");
    let betas = beta_features(
        model,
        &context_messages,
        is_oauth,
        native_tool_changes,
        options,
        headers,
    );

    let mut params = Map::new();
    params.insert("model".into(), json!(model.id));
    params.insert(
        "messages".into(),
        json!(if compat.supports_mid_convo_effort {
            insert_thinking_level_messages(&converted, &assistant_levels, active_effort)
        } else {
            converted
        }),
    );
    let max_tokens = options
        .max_tokens
        .unwrap_or_else(|| clamp_max_tokens_to_context(model, &context_messages, model.max_tokens));
    params.insert("max_tokens".into(), json!(max_tokens));
    params.insert("stream".into(), json!(true));
    // `betas` rides the `anthropic-beta` header, not the JSON body (the SDK
    // strips the param into a header when serializing).
    if is_oauth {
        let mut system = vec![json!({
            "type": "text",
            "text": "You are Claude Code, Anthropic's official CLI for Claude.",
        })];
        if let Some(cc) = &cc {
            system[0]["cache_control"] = cc.clone();
        }
        if !initial_system_text.is_empty() {
            let mut block = json!({
                "type": "text",
                "text": piutil::sanitize_surrogates(&initial_system_text),
            });
            if let Some(cc) = &cc {
                block["cache_control"] = cc.clone();
            }
            system.push(block);
        }
        params.insert("system".into(), json!(system));
    } else if !initial_system_text.is_empty() {
        let mut block = json!({
            "type": "text",
            "text": piutil::sanitize_surrogates(&initial_system_text),
        });
        if let Some(cc) = &cc {
            block["cache_control"] = cc.clone();
        }
        params.insert("system".into(), json!([block]));
    }

    let tool_cache_control = if compat.supports_cache_control_on_tools {
        cc.as_ref()
    } else {
        None
    };
    if native_tool_changes {
        let initial_names: std::collections::BTreeSet<&str> =
            initial_tools.iter().map(|t| t.name.as_str()).collect();
        let later_tools: Vec<oino_types::ToolDeclaration> =
            piutil::declared_tools(&context_messages)
                .into_iter()
                .filter(|t| !initial_names.contains(t.name.as_str()))
                .collect();
        let mut tools = convert_tools(
            &initial_tools,
            is_oauth,
            compat.supports_eager_tool_input_streaming,
            compat.supports_strict_tools,
            tool_cache_control,
        );
        tools.push(deferred_tool_placeholder());
        for mut tool in convert_tools(
            &later_tools,
            is_oauth,
            compat.supports_eager_tool_input_streaming,
            compat.supports_strict_tools,
            None,
        ) {
            tool["defer_loading"] = json!(true);
            tools.push(tool);
        }
        params.insert("tools".into(), json!(tools));
    } else {
        let tools = transcript::current_tools(&context_messages);
        if !tools.is_empty() {
            params.insert(
                "tools".into(),
                json!(convert_tools(
                    &tools,
                    is_oauth,
                    compat.supports_eager_tool_input_streaming,
                    compat.supports_strict_tools,
                    tool_cache_control,
                )),
            );
        }
    }

    if compat.supports_mid_convo_effort {
        params.insert(
            "thinking".into(),
            json!({
                "type": "adaptive",
                "display": options.thinking_display.unwrap_or("summarized"),
                "block_binding": {"prefix_mismatch_behavior": "drop_block"},
            }),
        );
        params.insert("output_config".into(), json!({"effort": "high"}));
    } else if model.reasoning {
        if options.thinking_enabled == Some(true) {
            let display = options.thinking_display.unwrap_or("summarized");
            if compat.force_adaptive_thinking {
                params.insert(
                    "thinking".into(),
                    json!({"type": "adaptive", "display": display}),
                );
                if let Some(effort) = options.effort {
                    params.insert("output_config".into(), json!({"effort": effort}));
                }
            } else {
                params.insert(
                    "thinking".into(),
                    json!({
                        "type": "enabled",
                        "budget_tokens": options.thinking_budget_tokens.unwrap_or(1024),
                        "display": display,
                    }),
                );
            }
        } else if options.thinking_enabled == Some(false)
            && model.thinking_level_map.as_ref().and_then(|m| m.get("off")) != Some(&None)
        {
            params.insert("thinking".into(), json!({"type": "disabled"}));
        }
    }

    if !compat.allowed_fallback_models.is_empty() {
        params.insert(
            "fallbacks".into(),
            json!(compat
                .allowed_fallback_models
                .iter()
                .map(|f| json!({"model": f.model}))
                .collect::<Vec<_>>()),
        );
    }
    Ok((Value::Object(params), betas))
}

/// pi `mapThinkingLevelToEffort` for adaptive thinking models.
#[must_use]
pub fn thinking_level_to_effort(model: &CatalogModel, level: &str) -> String {
    let mapped = model
        .thinking_level_map
        .as_ref()
        .and_then(|m| m.get(level))
        .and_then(|v| v.clone());
    if let Some(mapped) = mapped {
        return mapped;
    }
    match level {
        "minimal" | "low" => "low".into(),
        "medium" => "medium".into(),
        _ => "high".into(),
    }
}

/// pi `clampMaxTokensToContext`.
#[must_use]
pub fn clamp_max_tokens_to_context(
    model: &CatalogModel,
    messages: &[Message],
    max_tokens: u64,
) -> u64 {
    if model.context_window == 0 {
        return max_tokens.max(1);
    }
    let available = model
        .context_window
        .saturating_sub(piutil::estimate_context_tokens(messages) as u64)
        .saturating_sub(4096);
    max_tokens.min(available.max(1))
}

/// Thinking options for a `ThinkingLevel`, mirroring `streamSimple`.
#[must_use]
pub fn thinking_options_for_level(
    model: &CatalogModel,
    level: oino_types::ThinkingLevel,
    messages: &[Message],
) -> AnthropicRequestOptions<'static> {
    let mut options = AnthropicRequestOptions::default();
    let base_max_tokens = clamp_max_tokens_to_context(model, messages, model.max_tokens);
    let compat = anthropic_compat(model);
    if level == oino_types::ThinkingLevel::Off {
        options.thinking_enabled = Some(false);
        options.max_tokens = Some(base_max_tokens);
        return options;
    }
    if compat.force_adaptive_thinking || compat.supports_mid_convo_effort {
        options.thinking_enabled = Some(true);
        options.effort = Some(Box::leak(
            thinking_level_to_effort(model, level_name(level)).into_boxed_str(),
        ));
        options.max_tokens = Some(base_max_tokens);
        return options;
    }
    // budget-based (pi adjustMaxTokensForThinking): budget by level, then
    // max_tokens = min(base + budget, model cap).
    let level_budget: u64 = match level {
        oino_types::ThinkingLevel::Minimal => 1024,
        oino_types::ThinkingLevel::Low => 2048,
        oino_types::ThinkingLevel::Medium => 8192,
        _ => 16384,
    };
    let mut thinking_budget = level_budget;
    let mut max_tokens = (base_max_tokens + thinking_budget).min(model.max_tokens);
    if max_tokens <= thinking_budget {
        thinking_budget = thinking_budget.min(max_tokens.saturating_sub(1024));
    }
    max_tokens = clamp_max_tokens_to_context(model, messages, max_tokens);
    options.thinking_enabled = Some(true);
    options.thinking_budget_tokens = Some(thinking_budget.min(max_tokens.saturating_sub(1024)));
    options.max_tokens = Some(max_tokens);
    options
}

fn level_name(level: oino_types::ThinkingLevel) -> &'static str {
    match level {
        oino_types::ThinkingLevel::Off => "off",
        oino_types::ThinkingLevel::Minimal => "minimal",
        oino_types::ThinkingLevel::Low => "low",
        oino_types::ThinkingLevel::Medium => "medium",
        oino_types::ThinkingLevel::High => "high",
        oino_types::ThinkingLevel::XHigh => "xhigh",
        oino_types::ThinkingLevel::Max => "max",
    }
}

// ---- SSE parsing ----------------------------------------------------------

struct SseDecoderState {
    event: Option<String>,
    data: Vec<String>,
}

/// Incremental Anthropic SSE parser: `event:`/`data:` lines →
/// `AssistantStreamEvent`s. Mirrors `iterateAnthropicEvents` + the stream
/// loop's event mapping.
pub struct AnthropicSseParser {
    model: CatalogModel,
    buffer: String,
    state: SseDecoderState,
    saw_message_start: bool,
    saw_message_stop: bool,
    blocks: BTreeMap<usize, BlockState>,
    tool_blocks: BTreeMap<usize, (String, String)>,
    usage: Usage,
    cost: crate::piutil::CostRates,
    response_id: Option<String>,
    response_model: Option<String>,
    stop_reason: Option<StopReason>,
    error_message: Option<String>,
}

#[derive(Debug, Clone)]
enum BlockState {
    Text,
    Thinking,
    ToolCall { partial_json: String },
}

const ANTHROPIC_MESSAGE_EVENTS: &[&str] = &[
    "message_start",
    "message_delta",
    "message_stop",
    "content_block_start",
    "content_block_delta",
    "content_block_stop",
];

impl AnthropicSseParser {
    #[must_use]
    pub fn new(model: CatalogModel) -> Self {
        Self {
            cost: model.cost.clone(),
            model,
            buffer: String::new(),
            state: SseDecoderState {
                event: None,
                data: Vec::new(),
            },
            saw_message_start: false,
            saw_message_stop: false,
            blocks: BTreeMap::new(),
            tool_blocks: BTreeMap::new(),
            usage: Usage::default(),
            response_id: None,
            response_model: None,
            stop_reason: None,
            error_message: None,
        }
    }

    /// Feed a chunk; returns events parsed so far (no batching semantics —
    /// caller emits each returned event).
    pub fn push_str(&mut self, text: &str) -> Result<Vec<AssistantStreamEvent>, KitError> {
        self.buffer.push_str(text);
        let mut events = Vec::new();
        while let Some((line, rest)) = take_line(&self.buffer) {
            self.buffer = rest;
            events.extend(self.handle_line(&line)?);
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Vec<AssistantStreamEvent>, KitError> {
        let mut events = Vec::new();
        if !self.buffer.is_empty() {
            let line = std::mem::take(&mut self.buffer);
            events.extend(self.handle_line(&line)?);
        }
        events.extend(self.flush_sse_event()?);
        if self.saw_message_start && !self.saw_message_stop {
            return Err(KitError::Sse(
                "Anthropic stream ended before message_stop".into(),
            ));
        }
        if self.stop_reason.is_none() {
            return Err(KitError::Sse(
                "Anthropic stream ended without a stop reason".into(),
            ));
        }
        if matches!(
            self.stop_reason,
            Some(StopReason::Aborted) | Some(StopReason::Error)
        ) {
            return Err(KitError::Sse(
                self.error_message
                    .clone()
                    .unwrap_or_else(|| "An unknown error occurred".into()),
            ));
        }
        events.push(AssistantStreamEvent::Done {
            stop_reason: self.stop_reason.clone().unwrap_or(StopReason::Unknown),
            provider: provider_metadata(
                &self.model.provider,
                self.response_id.take(),
                self.response_model.take(),
            ),
        });
        Ok(events)
    }

    fn flush_sse_event(&mut self) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if self.state.event.is_none() && self.state.data.is_empty() {
            return Ok(Vec::new());
        }
        let event = self.state.event.take();
        let data = std::mem::take(&mut self.state.data).join("\n");
        self.dispatch_sse(event, &data)
    }

    fn handle_line(&mut self, line: &str) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if line.is_empty() {
            return self.flush_sse_event();
        }
        if line.starts_with(':') {
            return Ok(Vec::new());
        }
        let (field, value) = match line.find(':') {
            Some(index) => {
                let mut value = &line[index + 1..];
                if value.starts_with(' ') {
                    value = &value[1..];
                }
                (&line[..index], value)
            }
            None => (line, ""),
        };
        match field {
            "event" => self.state.event = Some(value.to_string()),
            "data" => self.state.data.push(value.to_string()),
            _ => {}
        }
        Ok(Vec::new())
    }

    fn dispatch_sse(
        &mut self,
        event: Option<String>,
        data: &str,
    ) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if event.as_deref() == Some("error") {
            return Err(KitError::Sse(data.to_string()));
        }
        if !event
            .as_deref()
            .is_some_and(|e| ANTHROPIC_MESSAGE_EVENTS.contains(&e))
        {
            return Ok(Vec::new());
        }
        let parsed: Value = piutil::parse_json_with_repair(data).map_err(|err| {
            KitError::Sse(format!(
                "Could not parse Anthropic SSE event {}: {err}; data={data}",
                event.as_deref().unwrap_or_default()
            ))
        })?;
        let kind = parsed["type"].as_str().unwrap_or_default();
        match kind {
            "message_start" => {
                self.saw_message_start = true;
                let message = &parsed["message"];
                self.response_id = message["id"].as_str().map(str::to_string);
                if let Some(response_model) = message["model"].as_str() {
                    if response_model != self.model.id {
                        self.response_model = Some(response_model.to_string());
                        // fallback cost attribution
                        let compat = anthropic_compat(&self.model);
                        if let Some(fallback) = compat
                            .allowed_fallback_models
                            .iter()
                            .find(|f| f.model == response_model)
                        {
                            self.cost = fallback.cost.clone();
                        }
                    }
                }
                let usage = &message["usage"];
                self.usage.input_tokens = usage["input_tokens"].as_u64().unwrap_or(0);
                self.usage.output_tokens = usage["output_tokens"].as_u64().unwrap_or(0);
                self.usage.cache_read_tokens =
                    usage["cache_read_input_tokens"].as_u64().unwrap_or(0);
                self.usage.cache_write_tokens =
                    usage["cache_creation_input_tokens"].as_u64().unwrap_or(0);
                let one_h = usage["cache_creation"]["ephemeral_1h_input_tokens"].as_u64();
                self.usage.cache_write_1h_tokens = one_h.map(|v| if v > 0 { v } else { 0 });
                self.apply_cost();
                Ok(vec![AssistantStreamEvent::Start {
                    response_id: self.response_id.clone(),
                    response_model: self.response_model.clone(),
                }])
            }
            "content_block_start" => {
                let index = parsed["index"].as_u64().unwrap_or(0) as usize;
                let block = &parsed["content_block"];
                match block["type"].as_str() {
                    Some("fallback") => {
                        if !self.blocks.is_empty() {
                            return Err(KitError::Sse(
                                "Anthropic performed an unsupported mid-output model fallback"
                                    .into(),
                            ));
                        }
                        Ok(Vec::new())
                    }
                    Some("text") => {
                        self.blocks.insert(index, BlockState::Text);
                        Ok(Vec::new())
                    }
                    Some("thinking") => {
                        self.blocks.insert(index, BlockState::Thinking);
                        Ok(Vec::new())
                    }
                    Some("redacted_thinking") => {
                        self.blocks.insert(index, BlockState::Thinking);
                        Ok(vec![AssistantStreamEvent::ThinkingSignature {
                            signature: block["data"].as_str().unwrap_or_default().into(),
                        }])
                    }
                    Some("tool_use") => {
                        self.blocks.insert(
                            index,
                            BlockState::ToolCall {
                                partial_json: String::new(),
                            },
                        );
                        self.tool_blocks.insert(
                            index,
                            (
                                block["id"].as_str().unwrap_or_default().to_string(),
                                block["name"].as_str().unwrap_or_default().to_string(),
                            ),
                        );
                        Ok(Vec::new())
                    }
                    _ => Ok(Vec::new()),
                }
            }
            "content_block_delta" => {
                let index = parsed["index"].as_u64().unwrap_or(0) as usize;
                let delta = &parsed["delta"];
                match (delta["type"].as_str(), self.blocks.get(&index)) {
                    (Some("text_delta"), Some(BlockState::Text)) => {
                        Ok(vec![AssistantStreamEvent::TextDelta {
                            delta: delta["text"].as_str().unwrap_or_default().into(),
                        }])
                    }
                    (Some("thinking_delta"), Some(BlockState::Thinking)) => {
                        Ok(vec![AssistantStreamEvent::ThinkingDelta {
                            delta: delta["thinking"].as_str().unwrap_or_default().into(),
                        }])
                    }
                    (Some("input_json_delta"), Some(BlockState::ToolCall { .. })) => {
                        let partial = delta["partial_json"].as_str().unwrap_or_default();
                        let index_block = self.blocks.get_mut(&index);
                        if let Some(BlockState::ToolCall { partial_json }) = index_block {
                            partial_json.push_str(partial);
                        }
                        Ok(vec![AssistantStreamEvent::ToolCallDelta {
                            id: OinoId::new_v4(),
                            name: None,
                            native_id: None,
                            arguments_delta: partial.into(),
                        }])
                    }
                    (Some("signature_delta"), Some(BlockState::Thinking)) => {
                        Ok(vec![AssistantStreamEvent::ThinkingSignature {
                            signature: delta["signature"].as_str().unwrap_or_default().into(),
                        }])
                    }
                    _ => Ok(Vec::new()),
                }
            }
            "content_block_stop" => {
                let index = parsed["index"].as_u64().unwrap_or(0) as usize;
                if let Some(BlockState::ToolCall { partial_json }) = self.blocks.remove(&index) {
                    let arguments = piutil::parse_streaming_json(&partial_json);
                    let (native_id, name) = self.tool_blocks.remove(&index).unwrap_or_default();
                    return Ok(vec![AssistantStreamEvent::ToolCallDone {
                        id: OinoId::new_v4(),
                        name,
                        native_id: (!native_id.is_empty()).then_some(native_id),
                        arguments,
                    }]);
                }
                self.tool_blocks.remove(&index);
                Ok(Vec::new())
            }
            "message_delta" => {
                let mut events = Vec::new();
                if let Some(reason) = parsed["delta"]["stop_reason"].as_str() {
                    let (stop, error) = map_stop_reason(reason, &parsed["delta"]["stop_details"])?;
                    self.stop_reason = Some(stop.clone());
                    self.error_message = error.clone();
                    if let Some(message) = error {
                        events.push(AssistantStreamEvent::Error { message });
                    }
                }
                let usage = &parsed["usage"];
                if !usage.is_null() {
                    if let Some(v) = usage["input_tokens"].as_u64() {
                        self.usage.input_tokens = v;
                    }
                    if let Some(v) = usage["output_tokens"].as_u64() {
                        self.usage.output_tokens = v;
                    }
                    if let Some(v) = usage["cache_read_input_tokens"].as_u64() {
                        self.usage.cache_read_tokens = v;
                    }
                    if let Some(v) = usage["cache_creation_input_tokens"].as_u64() {
                        self.usage.cache_write_tokens = v;
                    }
                    if let Some(v) = usage["cache_creation"]["ephemeral_1h_input_tokens"].as_u64() {
                        self.usage.cache_write_1h_tokens = Some(v);
                    }
                    if let Some(v) = usage["output_tokens_details"]["thinking_tokens"].as_u64() {
                        self.usage.reasoning_tokens = v;
                    }
                    self.apply_cost();
                    events.push(AssistantStreamEvent::Usage {
                        usage: self.usage.clone(),
                    });
                }
                Ok(events)
            }
            "message_stop" => {
                self.saw_message_stop = true;
                Ok(Vec::new())
            }
            _ => Ok(Vec::new()),
        }
    }

    fn apply_cost(&mut self) {
        piutil::calculate_cost(&self.cost, &mut self.usage);
    }
}

fn take_line(buffer: &str) -> Option<(String, String)> {
    let r = buffer.find('\r');
    let n = buffer.find('\n');
    let index = match (r, n) {
        (Some(r), Some(n)) => r.min(n),
        (Some(r), None) => r,
        (None, Some(n)) => n,
        (None, None) => return None,
    };
    let mut next = index + 1;
    if buffer.as_bytes()[index] == b'\r' && buffer.as_bytes().get(next) == Some(&b'\n') {
        next += 1;
    }
    Some((buffer[..index].to_string(), buffer[next..].to_string()))
}

fn provider_metadata(
    provider_id: &str,
    id: Option<String>,
    model: Option<String>,
) -> Option<ProviderMetadata> {
    if id.is_none() && model.is_none() {
        return None;
    }
    let mut values = BTreeMap::new();
    if let Some(id) = id {
        values.insert("id".into(), Value::String(id));
    }
    Some(ProviderMetadata {
        request_id: None,
        model: model.map(|name| Model::new(provider_id, name)),
        values,
    })
}

fn map_stop_reason(
    reason: &str,
    stop_details: &Value,
) -> Result<(StopReason, Option<String>), KitError> {
    match reason {
        "end_turn" | "pause_turn" | "stop_sequence" => Ok((StopReason::EndTurn, None)),
        "max_tokens" => Ok((StopReason::Length, None)),
        "tool_use" => Ok((StopReason::ToolUse, None)),
        "refusal" => Ok((
            StopReason::Error,
            Some(
                stop_details["explanation"]
                    .as_str()
                    .unwrap_or("The model refused to complete the request")
                    .to_string(),
            ),
        )),
        "sensitive" => Ok((
            StopReason::Error,
            Some("Provider stopped with: sensitive".into()),
        )),
        other => Err(KitError::Sse(format!("Unhandled stop reason: {other}"))),
    }
}
