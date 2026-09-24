//! Faithful port of pi `api/openai-responses.ts` + `api/openai-responses-shared.ts`
//! + `api/openai-prompt-cache.ts`.

use std::collections::BTreeMap;

use oino_types::{
    transcript, AssistantStreamEvent, ContentBlock, Message, OinoId, ProviderMetadata, StopReason,
    Usage,
};
use serde_json::{json, Map, Value};

use crate::catalog::CatalogModel;
use crate::piutil;
use crate::{CacheRetention, KitError, StreamRequest};

const OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH: usize = 64;

/// pi `clampOpenAIPromptCacheKey` (counts Unicode scalar values).
#[must_use]
pub fn clamp_openai_prompt_cache_key(key: Option<&str>) -> Option<String> {
    let key = key?;
    if key.chars().count() <= OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH {
        return Some(key.to_string());
    }
    Some(
        key.chars()
            .take(OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH)
            .collect(),
    )
}

/// OpenAI Responses compat flags (pi `getCompat`).
pub struct ResponsesCompat {
    pub supports_developer_role: bool,
    pub supports_mid_convo_system_messages: bool,
    pub session_affinity_format: String,
    pub supports_long_cache_retention: bool,
    pub supports_strict_mode: bool,
    pub supports_openai_grammar_tools: bool,
    pub supports_additional_tools: bool,
    pub supports_tool_search: bool,
    pub supports_explicit_prompt_cache_mode: bool,
    pub supports_max_output_tokens: bool,
}

#[must_use]
pub fn responses_compat(model: &CatalogModel) -> ResponsesCompat {
    let flag = |k: &str, d: bool| model.compat_flag(k, d);
    let affinity = model
        .compat_str("sessionAffinityFormat")
        .map(str::to_string)
        .unwrap_or_else(|| {
            if model.provider == "openrouter" || model.base_url.contains("openrouter.ai") {
                "openrouter".into()
            } else {
                "openai".into()
            }
        });
    ResponsesCompat {
        supports_developer_role: flag("supportsDeveloperRole", true),
        supports_mid_convo_system_messages: flag("supportsMidConvoSystemMessages", false),
        session_affinity_format: affinity,
        supports_long_cache_retention: flag("supportsLongCacheRetention", true),
        supports_strict_mode: flag("supportsStrictMode", false),
        supports_openai_grammar_tools: flag("supportsOpenAIGrammarTools", false),
        supports_additional_tools: flag("supportsAdditionalTools", false),
        supports_tool_search: flag("supportsToolSearch", false),
        supports_explicit_prompt_cache_mode: flag("supportsExplicitPromptCacheMode", false),
        supports_max_output_tokens: flag("supportsMaxOutputTokens", true),
    }
}

fn prompt_cache_retention(
    compat: &ResponsesCompat,
    retention: CacheRetention,
) -> Option<&'static str> {
    (retention == CacheRetention::Long
        && compat.supports_long_cache_retention
        && !compat.supports_explicit_prompt_cache_mode)
        .then_some("24h")
}

fn prompt_cache_options(compat: &ResponsesCompat, retention: CacheRetention) -> Option<Value> {
    if !compat.supports_explicit_prompt_cache_mode {
        return None;
    }
    if retention == CacheRetention::None {
        return Some(json!({"mode": "explicit"}));
    }
    if retention == CacheRetention::Long && compat.supports_long_cache_retention {
        return Some(json!({"ttl": "30m"}));
    }
    None
}

const OPENAI_TOOL_CALL_PROVIDERS: &[&str] = &["openai", "openai-codex", "opencode"];

// ---- text signatures (encodeTextSignatureV1 / parseTextSignature) ---------

fn parse_text_signature(signature: Option<&str>) -> Option<(String, Option<String>)> {
    let signature = signature?;
    if signature.starts_with('{') {
        if let Ok(parsed) = serde_json::from_str::<Value>(signature) {
            if parsed["v"].as_i64() == Some(1) {
                if let Some(id) = parsed["id"].as_str() {
                    let phase = parsed["phase"].as_str().and_then(|p| {
                        (p == "commentary" || p == "final_answer").then(|| p.to_string())
                    });
                    return Some((id.to_string(), phase));
                }
            }
        }
    }
    Some((signature.to_string(), None))
}

fn normalize_id_part(part: &str) -> String {
    let sanitized: String = part
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect();
    sanitized.trim_end_matches('_').to_string()
}

fn build_foreign_responses_item_id(item_id: &str) -> String {
    let normalized = format!("fc_{}", piutil::short_hash(item_id));
    normalized.chars().take(64).collect()
}

fn normalize_tool_call_id(
    id: &str,
    model: &CatalogModel,
    source_provider: &str,
    source_api: &str,
) -> String {
    if !OPENAI_TOOL_CALL_PROVIDERS.contains(&model.provider.as_str()) {
        return normalize_id_part(id);
    }
    if !id.contains('|') {
        return normalize_id_part(id);
    }
    let mut parts = id.splitn(2, '|');
    let call_id = parts.next().unwrap_or_default();
    let item_id = parts.next().unwrap_or_default();
    let normalized_call_id = normalize_id_part(call_id);
    let is_foreign = source_provider != model.provider || source_api != model.api;
    let mut normalized_item_id = if is_foreign {
        build_foreign_responses_item_id(item_id)
    } else {
        normalize_id_part(item_id)
    };
    if !normalized_item_id.starts_with("fc_") {
        normalized_item_id = normalize_id_part(&format!("fc_{normalized_item_id}"));
    }
    format!("{normalized_call_id}|{normalized_item_id}")
}

fn convert_tool_result_output(model: &CatalogModel, content: &[ContentBlock]) -> Value {
    let text: String = content
        .iter()
        .filter_map(|b| match b {
            ContentBlock::Text { text, .. } => Some(text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("\n");
    let images: Vec<&ContentBlock> = content
        .iter()
        .filter(|b| matches!(b, ContentBlock::Image { .. }))
        .collect();
    if images.is_empty() || !model.accepts_images() {
        return json!(if !text.is_empty() {
            piutil::sanitize_surrogates(&text)
        } else if !images.is_empty() {
            "(see attached image)".to_string()
        } else {
            "(no tool output)".to_string()
        });
    }
    let mut out = Vec::new();
    if !text.is_empty() {
        out.push(json!({"type": "input_text", "text": piutil::sanitize_surrogates(&text)}));
    }
    for image in images {
        if let ContentBlock::Image { media_type, data } = image {
            out.push(json!({
                "type": "input_image",
                "detail": "auto",
                "image_url": format!("data:{media_type};base64,{data}"),
            }));
        }
    }
    json!(out)
}

fn convert_responses_tools(tools: &[oino_types::ToolDeclaration], strict: bool) -> Vec<Value> {
    tools
        .iter()
        .map(|tool| {
            let mut t = json!({
                "type": "function",
                "name": tool.name,
                "description": tool.description,
                "parameters": tool.input_schema,
            });
            if strict {
                t["strict"] = json!(false);
            }
            t
        })
        .collect()
}

fn convert_responses_messages(
    model: &CatalogModel,
    context_messages: &[Message],
    compat: &ResponsesCompat,
) -> Vec<Value> {
    let transcript_tools = piutil::resolve_transcript_tools(
        context_messages,
        compat.supports_additional_tools || compat.supports_tool_search,
    );
    let facets = model.facets();
    let normalize = |id: &str, _facets: &piutil::ModelFacets<'_>, source: &Message| {
        let (provider, api) = match source {
            Message::Assistant { provider, .. } => provider
                .as_ref()
                .and_then(|p| p.model.as_ref())
                .map(|m| (m.provider.as_str(), model.api.as_str()))
                .unwrap_or(("", "")),
            _ => ("", ""),
        };
        normalize_tool_call_id(id, model, provider, api)
    };
    let transformed = piutil::transform_messages(context_messages, &facets, Some(&normalize));

    let mut messages: Vec<Value> = Vec::new();
    let instruction_role = if model.reasoning && compat.supports_developer_role {
        "developer"
    } else {
        "system"
    };

    let append_tool_additions = |messages: &mut Vec<Value>, msg: &Message, _seed: usize| {
        let Message::System { tools_added, .. } = msg else {
            return;
        };
        let tools: &[oino_types::ToolDeclaration] = if transcript_tools.anchors_additions {
            tools_added
        } else {
            &[]
        };
        if tools.is_empty() {
            return;
        }
        if compat.supports_additional_tools {
            messages.push(json!({
                "type": "additional_tools",
                "role": "developer",
                "tools": convert_responses_tools(tools, compat.supports_strict_mode),
            }));
            return;
        }
        if !compat.supports_tool_search {
            return;
        }
        let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
        let call_id = format!(
            "pi_tool_load_{}",
            piutil::short_hash(&format!("system:{}:{}", _seed, names.join(",")))
        );
        messages.push(json!({
            "type": "tool_search_call",
            "call_id": call_id,
            "execution": "client",
            "status": "completed",
            "arguments": {"query": names.join(" "), "limit": names.len()},
        }));
        let mut results = convert_responses_tools(tools, compat.supports_strict_mode);
        for t in &mut results {
            t["defer_loading"] = json!(true);
        }
        messages.push(json!({
            "type": "tool_search_output",
            "call_id": call_id,
            "execution": "client",
            "status": "completed",
            "tools": results,
        }));
    };

    let native_ids = crate::anthropic::native_id_map(&transformed);
    let mut msg_index = 0usize;
    for (source_index, msg) in transformed.iter().enumerate() {
        let is_leading_system = source_index == 0 && matches!(msg, Message::System { .. });
        match msg {
            Message::System { .. } => {
                if !is_leading_system {
                    append_tool_additions(&mut messages, msg, msg_index);
                }
                let text = if is_leading_system {
                    piutil::system_message_text(msg)
                } else {
                    piutil::render_system_message_update(msg)
                };
                if !text.is_empty() {
                    messages.push(json!({
                        "role": instruction_role,
                        "content": piutil::sanitize_surrogates(&text),
                    }));
                }
            }
            Message::User { content, .. } => {
                let all_text = content
                    .iter()
                    .all(|b| matches!(b, ContentBlock::Text { .. }));
                if all_text {
                    let text: String = content
                        .iter()
                        .filter_map(|b| match b {
                            ContentBlock::Text { text, .. } => Some(text.as_str()),
                            _ => None,
                        })
                        .collect::<Vec<_>>()
                        .join("\n");
                    messages.push(json!({
                        "role": "user",
                        "content": [{"type": "input_text", "text": piutil::sanitize_surrogates(&text)}],
                    }));
                } else {
                    let blocks: Vec<Value> = content
                        .iter()
                        .filter_map(|item| match item {
                            ContentBlock::Text { text, .. } => Some(json!({
                                "type": "input_text",
                                "text": piutil::sanitize_surrogates(text),
                            })),
                            ContentBlock::Image { media_type, data } => Some(json!({
                                "type": "input_image",
                                "detail": "auto",
                                "image_url": format!("data:{media_type};base64,{data}"),
                            })),
                            _ => None,
                        })
                        .collect();
                    if !blocks.is_empty() {
                        messages.push(json!({"role": "user", "content": blocks}));
                    }
                }
            }
            Message::Assistant {
                content, provider, ..
            } => {
                let mut output: Vec<Value> = Vec::new();
                let (src_provider, src_model) = provider
                    .as_ref()
                    .and_then(|p| p.model.as_ref())
                    .map(|m| (m.provider.as_str(), m.name.as_str()))
                    .unwrap_or_default();
                let is_same_provider_and_api = src_provider == model.provider;
                let is_same_model = is_same_provider_and_api && src_model == model.id;
                let _ = is_same_model;
                let is_different_model = is_same_provider_and_api && src_model != model.id;
                let mut text_block_index = 0usize;
                for block in content {
                    match block {
                        ContentBlock::Thinking { signature, .. } => {
                            if let Some(sig) = signature {
                                if let Ok(item) = serde_json::from_str::<Value>(sig) {
                                    output.push(item);
                                }
                            }
                        }
                        ContentBlock::Text { text, signature } => {
                            let parsed = parse_text_signature(signature.as_deref());
                            let fallback_id = if text_block_index == 0 {
                                format!("msg_pi_{msg_index}")
                            } else {
                                format!("msg_pi_{msg_index}_{text_block_index}")
                            };
                            text_block_index += 1;
                            let mut msg_id = parsed
                                .as_ref()
                                .map(|(id, _)| id.clone())
                                .unwrap_or(fallback_id);
                            if msg_id.len() > 64 {
                                msg_id = format!("msg_{}", piutil::short_hash(&msg_id));
                            }
                            let mut item = json!({
                                "type": "message",
                                "role": "assistant",
                                "content": [{"type": "output_text", "text": piutil::sanitize_surrogates(text), "annotations": []}],
                                "status": "completed",
                                "id": msg_id,
                            });
                            if let Some((_, Some(phase))) = parsed {
                                item["phase"] = json!(phase);
                            }
                            output.push(item);
                        }
                        ContentBlock::ToolCall {
                            name,
                            arguments,
                            native_id,
                            ..
                        } => {
                            let raw = native_id.clone().unwrap_or_default();
                            let mut parts = raw.splitn(2, '|');
                            let call_id = parts.next().unwrap_or_default().to_string();
                            let mut item_id = parts.next().map(str::to_string);
                            if (is_different_model
                                && item_id.as_deref().is_some_and(|i| i.starts_with("fc_")))
                                || !item_id.as_deref().is_some_and(|i| i.starts_with("fc_"))
                            {
                                item_id = None;
                            }
                            output.push(json!({
                                "type": "function_call",
                                "id": item_id,
                                "call_id": call_id,
                                "name": name,
                                "arguments": serde_json::to_string(arguments).unwrap_or_else(|_| "{}".into()),
                            }));
                        }
                        ContentBlock::Image { .. } => {}
                    }
                }
                if !output.is_empty() {
                    messages.extend(output);
                }
            }
            Message::ToolResult {
                tool_call_id,
                content,
                ..
            } => {
                let raw = native_ids
                    .get(tool_call_id)
                    .cloned()
                    .unwrap_or_else(|| tool_call_id.to_string());
                let call_id = raw.split('|').next().unwrap_or_default();
                let out = convert_tool_result_output(model, content);
                messages.push(json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": out,
                }));
            }
            _ => {}
        }
        if !is_leading_system {
            msg_index += 1;
        }
    }
    messages
}

/// Options Oino maps onto pi's OpenAIResponsesOptions.
#[derive(Default)]
pub struct ResponsesRequestOptions<'a> {
    pub reasoning_effort: Option<&'a str>,
    pub reasoning_summary: Option<&'a str>,
    pub service_tier: Option<&'a str>,
    pub max_tokens: Option<u64>,
    pub temperature: Option<f64>,
}

/// Build the `/responses` request body (pi `buildParams`).
#[must_use]
pub fn build_params(
    model: &CatalogModel,
    request: &StreamRequest,
    options: &ResponsesRequestOptions<'_>,
) -> Value {
    let compat = responses_compat(model);
    let normalized = transcript::resolve(
        transcript::Transcript::new(request.transcript.messages.clone()),
        compat.supports_mid_convo_system_messages,
    );
    let context_messages = normalized.messages().to_vec();
    let transcript_tools = piutil::resolve_transcript_tools(
        &context_messages,
        compat.supports_additional_tools || compat.supports_tool_search,
    );
    let input = convert_responses_messages(model, &context_messages, &compat);
    let retention = request.cache_retention;

    let mut params = Map::new();
    params.insert("model".into(), json!(model.id));
    params.insert("input".into(), json!(input));
    params.insert("stream".into(), json!(true));
    if retention != CacheRetention::None {
        params.insert(
            "prompt_cache_key".into(),
            json!(clamp_openai_prompt_cache_key(request.session_id.as_deref())),
        );
    }
    if let Some(v) = prompt_cache_retention(&compat, retention) {
        params.insert("prompt_cache_retention".into(), json!(v));
    }
    if let Some(v) = prompt_cache_options(&compat, retention) {
        params.insert("prompt_cache_options".into(), v);
    }
    params.insert("store".into(), json!(false));

    if compat.supports_max_output_tokens {
        let clamped = crate::anthropic::clamp_max_tokens_to_context(
            model,
            &context_messages,
            options.max_tokens.unwrap_or(model.max_tokens),
        );
        params.insert("max_output_tokens".into(), json!(clamped.max(16)));
    }
    if let Some(t) = options.temperature {
        params.insert("temperature".into(), json!(t));
    }
    if let Some(t) = options.service_tier {
        params.insert("service_tier".into(), json!(t));
    }
    if !transcript_tools.request_tools.is_empty() {
        params.insert(
            "tools".into(),
            json!(convert_responses_tools(
                &transcript_tools.request_tools,
                compat.supports_strict_mode
            )),
        );
    }
    if model.reasoning {
        if options.reasoning_effort.is_some() || options.reasoning_summary.is_some() {
            let effort = options
                .reasoning_effort
                .map(|e| {
                    model
                        .thinking_level_map
                        .as_ref()
                        .and_then(|m| m.get(e))
                        .cloned()
                        .flatten()
                        .unwrap_or_else(|| e.to_string())
                })
                .unwrap_or_else(|| "medium".into());
            params.insert(
                "reasoning".into(),
                json!({
                    "effort": effort,
                    "summary": options.reasoning_summary.unwrap_or("auto"),
                }),
            );
            params.insert("include".into(), json!(["reasoning.encrypted_content"]));
        } else if model.provider != "github-copilot"
            && model.thinking_level_map.as_ref().and_then(|m| m.get("off")) != Some(&None)
        {
            let effort = model
                .thinking_level_map
                .as_ref()
                .and_then(|m| m.get("off"))
                .cloned()
                .flatten()
                .unwrap_or_else(|| "none".into());
            params.insert("reasoning".into(), json!({"effort": effort}));
        }
        if model.provider == "xai" {
            params.insert("include".into(), json!(["reasoning.encrypted_content"]));
        }
    }
    Value::Object(params)
}

/// Map an Oino ThinkingLevel to pi's reasoningEffort path (`streamSimple`).
#[must_use]
pub fn reasoning_effort_for_level(
    model: &CatalogModel,
    level: oino_types::ThinkingLevel,
) -> Option<String> {
    if level == oino_types::ThinkingLevel::Off {
        return None;
    }
    let clamped = piutil::clamp_thinking_level(
        model.reasoning,
        model.thinking_level_map.as_ref(),
        match level {
            oino_types::ThinkingLevel::Minimal => "minimal",
            oino_types::ThinkingLevel::Low => "low",
            oino_types::ThinkingLevel::Medium => "medium",
            oino_types::ThinkingLevel::High => "high",
            oino_types::ThinkingLevel::XHigh => "xhigh",
            oino_types::ThinkingLevel::Max => "max",
            oino_types::ThinkingLevel::Off => "off",
        },
    );
    (clamped != "off").then_some(clamped)
}

/// Session affinity headers (pi `createClient`).
#[must_use]
pub fn session_headers(model: &CatalogModel, session_id: Option<&str>) -> Vec<(String, String)> {
    let compat = responses_compat(model);
    let Some(session_id) = session_id else {
        return Vec::new();
    };
    let mut headers = Vec::new();
    if compat.session_affinity_format == "openrouter" {
        headers.push(("x-session-id".into(), session_id.into()));
    } else {
        if compat.session_affinity_format == "openai" {
            headers.push(("session_id".into(), session_id.into()));
        }
        headers.push(("x-client-request-id".into(), session_id.into()));
    }
    headers
}

// ---- stream processing (pi processResponsesStream) -------------------------

#[derive(Debug, Clone)]
enum OutputSlot {
    Thinking,
    Text,
    ToolCall { partial_json: String },
}

/// Incremental Responses SSE → `AssistantStreamEvent` translator.
pub struct ResponsesSseParser {
    model: CatalogModel,
    buffer: String,
    data: Vec<String>,
    output_slots: BTreeMap<u64, OutputSlot>,
    block_count: usize,
    reasoning_signatures: BTreeMap<String, String>,
    response_id: Option<String>,
    usage: Usage,
    stop_reason: Option<StopReason>,
    error_message: Option<String>,
    saw_terminal: bool,
}

impl ResponsesSseParser {
    #[must_use]
    pub fn new(model: CatalogModel) -> Self {
        Self {
            model,
            buffer: String::new(),
            data: Vec::new(),
            output_slots: BTreeMap::new(),
            block_count: 0,
            reasoning_signatures: BTreeMap::new(),
            response_id: None,
            usage: Usage::default(),
            stop_reason: None,
            error_message: None,
            saw_terminal: false,
        }
    }

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
        events.extend(self.flush_event()?);
        if !self.saw_terminal {
            return Err(KitError::Sse(
                "OpenAI Responses stream ended before a terminal response event".into(),
            ));
        }
        if self.stop_reason.is_none() {
            return Err(KitError::Sse(
                "OpenAI Responses stream ended without a stop reason".into(),
            ));
        }
        match self.stop_reason {
            Some(StopReason::Error) | Some(StopReason::Aborted) => {
                return Err(KitError::Sse(
                    self.error_message
                        .clone()
                        .unwrap_or_else(|| "An unknown error occurred".into()),
                ))
            }
            _ => {}
        }
        events.push(AssistantStreamEvent::Done {
            stop_reason: self.stop_reason.clone().unwrap_or(StopReason::Unknown),
            provider: self.response_id.take().map(|id| ProviderMetadata {
                request_id: None,
                model: None,
                values: {
                    let mut v = BTreeMap::new();
                    v.insert("id".into(), Value::String(id));
                    v
                },
            }),
        });
        Ok(events)
    }

    fn flush_event(&mut self) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if self.data.is_empty() {
            return Ok(Vec::new());
        }
        let data = std::mem::take(&mut self.data).join("\n");
        self.dispatch(&data)
    }

    fn handle_line(&mut self, line: &str) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if line.is_empty() {
            return self.flush_event();
        }
        if line.starts_with(':') {
            return Ok(Vec::new());
        }
        if let Some(data) = line.strip_prefix("data:") {
            self.data
                .push(data.strip_prefix(' ').unwrap_or(data).to_string());
        }
        Ok(Vec::new())
    }

    fn slot_events_at(&mut self, output_index: u64, item: &Value) -> Vec<AssistantStreamEvent> {
        if self.output_slots.contains_key(&output_index) {
            return Vec::new();
        }
        match item["type"].as_str() {
            Some("reasoning") => {
                self.output_slots.insert(output_index, OutputSlot::Thinking);
                self.block_count += 1;
                Vec::new()
            }
            Some("message") => {
                if item["phase"].as_str() == Some("final_answer") {
                    self.stop_reason = Some(StopReason::EndTurn);
                }
                self.output_slots.insert(output_index, OutputSlot::Text);
                self.block_count += 1;
                Vec::new()
            }
            Some("function_call") => {
                self.output_slots.insert(
                    output_index,
                    OutputSlot::ToolCall {
                        partial_json: item["arguments"].as_str().unwrap_or_default().into(),
                    },
                );
                self.block_count += 1;
                Vec::new()
            }
            _ => Vec::new(),
        }
    }

    fn dispatch(&mut self, data: &str) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if data == "[DONE]" {
            return Ok(Vec::new());
        }
        let event: Value = piutil::parse_json_with_repair(data)
            .map_err(|err| KitError::Sse(format!("responses sse: {err}")))?;
        let kind = event["type"].as_str().unwrap_or_default();
        let mut events = Vec::new();
        match kind {
            "response.created" => {
                self.response_id = event["response"]["id"].as_str().map(str::to_string);
                events.push(AssistantStreamEvent::Start {
                    response_id: self.response_id.clone(),
                    response_model: None,
                });
            }
            "response.output_item.added" => {
                events.extend(
                    self.slot_events_at(
                        event["output_index"].as_u64().unwrap_or(0),
                        &event["item"],
                    ),
                );
            }
            "response.reasoning_summary_text.delta" | "response.reasoning_text.delta" => {
                if let Some(OutputSlot::Thinking) = self
                    .output_slots
                    .get(&event["output_index"].as_u64().unwrap_or(0))
                {
                    events.push(AssistantStreamEvent::ThinkingDelta {
                        delta: event["delta"].as_str().unwrap_or_default().into(),
                    });
                }
            }
            "response.reasoning_summary_part.done" => {
                if let Some(OutputSlot::Thinking) = self
                    .output_slots
                    .get(&event["output_index"].as_u64().unwrap_or(0))
                {
                    events.push(AssistantStreamEvent::ThinkingDelta {
                        delta: "\n\n".into(),
                    });
                }
            }
            "response.output_text.delta" | "response.refusal.delta" => {
                if let Some(OutputSlot::Text) = self
                    .output_slots
                    .get(&event["output_index"].as_u64().unwrap_or(0))
                {
                    events.push(AssistantStreamEvent::TextDelta {
                        delta: event["delta"].as_str().unwrap_or_default().into(),
                    });
                }
            }
            "response.function_call_arguments.delta" => {
                if let Some(OutputSlot::ToolCall { partial_json, .. }) = self
                    .output_slots
                    .get_mut(&event["output_index"].as_u64().unwrap_or(0))
                {
                    let delta = event["delta"].as_str().unwrap_or_default();
                    partial_json.push_str(delta);
                    events.push(AssistantStreamEvent::ToolCallDelta {
                        id: OinoId::new_v4(),
                        name: None,
                        native_id: None,
                        arguments_delta: delta.into(),
                    });
                }
            }
            "response.function_call_arguments.done" => {
                if let Some(OutputSlot::ToolCall { partial_json, .. }) = self
                    .output_slots
                    .get_mut(&event["output_index"].as_u64().unwrap_or(0))
                {
                    let args = event["arguments"].as_str().unwrap_or_default();
                    if args.starts_with(partial_json.as_str()) {
                        let delta = &args[partial_json.len()..];
                        if !delta.is_empty() {
                            events.push(AssistantStreamEvent::ToolCallDelta {
                                id: OinoId::new_v4(),
                                name: None,
                                native_id: None,
                                arguments_delta: delta.into(),
                            });
                        }
                    }
                    *partial_json = args.to_string();
                }
            }
            "response.output_item.done" => {
                let output_index = event["output_index"].as_u64().unwrap_or(0);
                let item = event["item"].clone();
                events.extend(self.slot_events_at(output_index, &item));
                if item["phase"].as_str() == Some("final_answer") {
                    self.stop_reason = Some(StopReason::EndTurn);
                }
                match self.output_slots.remove(&output_index) {
                    Some(OutputSlot::Thinking) if item["type"].as_str() == Some("reasoning") => {
                        if let Some(id) = item["id"].as_str() {
                            self.reasoning_signatures
                                .insert(id.to_string(), item.to_string());
                        }
                        events.push(AssistantStreamEvent::ThinkingSignature {
                            signature: item.to_string(),
                        });
                    }
                    Some(OutputSlot::ToolCall { partial_json, .. })
                        if item["type"].as_str() == Some("function_call") =>
                    {
                        let arguments = piutil::parse_streaming_json(
                            item["arguments"].as_str().unwrap_or(&partial_json),
                        );
                        let call_id = item["call_id"].as_str().unwrap_or_default();
                        let item_id = item["id"].as_str().unwrap_or_default();
                        events.push(AssistantStreamEvent::ToolCallDone {
                            id: OinoId::new_v4(),
                            name: item["name"].as_str().unwrap_or_default().into(),
                            native_id: Some(format!("{call_id}|{item_id}")),
                            arguments,
                        });
                    }
                    _ => {}
                }
            }
            "response.completed" | "response.incomplete" => {
                self.finalize_response(&event["response"]);
                events.push(AssistantStreamEvent::Usage {
                    usage: self.usage.clone(),
                });
            }
            "error" => {
                return Err(KitError::Sse(format!(
                    "Error Code {}: {}",
                    event["code"].as_str().unwrap_or_default(),
                    event["message"].as_str().unwrap_or("Unknown error")
                )));
            }
            "response.failed" => {
                self.saw_terminal = true;
                let error = &event["response"]["error"];
                let details = &event["response"]["incomplete_details"];
                let message = if !error.is_null() {
                    format!(
                        "{}: {}",
                        error["code"].as_str().unwrap_or("unknown"),
                        error["message"].as_str().unwrap_or("no message")
                    )
                } else if let Some(reason) = details["reason"].as_str() {
                    format!("incomplete: {reason}")
                } else {
                    "Unknown error (no error details in response)".into()
                };
                return Err(KitError::Sse(message));
            }
            _ => {}
        }
        Ok(events)
    }

    fn finalize_response(&mut self, response: &Value) {
        self.saw_terminal = true;
        if let Some(id) = response["id"].as_str() {
            self.response_id = Some(id.to_string());
        }
        // reasoning signature backfill (Azure-style late encrypted_content)
        if let Some(output) = response["output"].as_array() {
            for item in output {
                if item["type"].as_str() == Some("reasoning") {
                    if let (Some(id), Some(enc)) =
                        (item["id"].as_str(), item["encrypted_content"].as_str())
                    {
                        if let Some(sig) = self.reasoning_signatures.get_mut(id) {
                            if let Ok(mut stored) = serde_json::from_str::<Value>(sig) {
                                if stored["encrypted_content"].is_null() {
                                    stored["encrypted_content"] = json!(enc);
                                    *sig = stored.to_string();
                                }
                            }
                        }
                    }
                }
            }
        }
        let usage = &response["usage"];
        if !usage.is_null() {
            let cached = usage["input_tokens_details"]["cached_tokens"]
                .as_u64()
                .unwrap_or(0);
            let write = usage["input_tokens_details"]["cache_write_tokens"]
                .as_u64()
                .unwrap_or(0);
            self.usage = Usage {
                input_tokens: usage["input_tokens"]
                    .as_u64()
                    .unwrap_or(0)
                    .saturating_sub(cached + write),
                output_tokens: usage["output_tokens"].as_u64().unwrap_or(0),
                cache_read_tokens: cached,
                cache_write_tokens: write,
                cache_write_1h_tokens: None,
                reasoning_tokens: usage["output_tokens_details"]["reasoning_tokens"]
                    .as_u64()
                    .unwrap_or(0),
                cost: None,
            };
            piutil::calculate_cost(&self.model.cost, &mut self.usage);
        }
        let status = response["status"].as_str();
        let incomplete_reason = response["incomplete_details"]["reason"].as_str();
        let (stop, error) = map_stop_reason(status, incomplete_reason);
        self.stop_reason = Some(stop);
        self.error_message = error;
        let _ = &self.model;
    }
}

fn map_stop_reason(
    status: Option<&str>,
    incomplete_reason: Option<&str>,
) -> (StopReason, Option<String>) {
    match status {
        None | Some("completed") | Some("in_progress") | Some("queued") => {
            (StopReason::EndTurn, None)
        }
        Some("incomplete") => {
            if incomplete_reason == Some("max_output_tokens") {
                (StopReason::Length, None)
            } else {
                (
                    StopReason::Error,
                    Some(match incomplete_reason {
                        Some(reason) => format!("Response incomplete: {reason}"),
                        None => "Response incomplete without a provider reason".into(),
                    }),
                )
            }
        }
        Some("failed") | Some("cancelled") => (StopReason::Error, None),
        _ => (StopReason::Error, None),
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
