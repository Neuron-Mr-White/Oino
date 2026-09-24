//! OpenAI-compatible provider wire logic: chat request building with pi's
//! cache hints (prompt_cache_key, Anthropic `cache_control` breakpoints),
//! session-affinity header computation, SSE chunk parsing into
//! `AssistantStreamEvent`s, and models-list parsing.
//!
//! This crate is a faithful port of the wire logic in
//! `crates/oino-provider-openrouter` so the same request/response bytes are
//! produced inside a `wasm32-wasip2` component. `oino-agent-loop` cannot
//! target wasm (tokio), so `StreamRequest`/`CacheRetention` are mirrored here
//! with the identical serde shape — the mirror is covered by a host-side
//! round-trip test.

pub mod anthropic;
pub mod catalog;
pub mod piutil;
pub mod responses;

use std::collections::BTreeMap;

use oino_types::{
    transcript, AssistantStreamEvent, ContentBlock, Message, Model, OinoId, ProviderMetadata,
    StopReason, ThinkingLevel, ToolDeclaration, Usage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

#[derive(Debug)]
pub enum KitError {
    Serialization(String),
    UnsupportedContent(String),
    Sse(String),
    Http(String),
}

impl std::fmt::Display for KitError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Serialization(e) | Self::UnsupportedContent(e) | Self::Sse(e) | Self::Http(e) => {
                f.write_str(e)
            }
        }
    }
}
impl std::error::Error for KitError {}

/// Mirror of `oino_agent_loop::CacheRetention` (same serde shape).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CacheRetention {
    None,
    #[default]
    Short,
    Long,
}

/// Transcript carrier in the wire request (`{"messages": [Message]}`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RequestTranscript {
    pub messages: Vec<Message>,
}

/// Mirror of `oino_agent_loop::StreamRequest` with the identical JSON shape
/// the host sends over the WIT boundary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StreamRequest {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub transcript: RequestTranscript,
    pub session_id: Option<String>,
    pub cache_retention: CacheRetention,
}

/// Resolved component configuration (auth excluded — resolved via
/// `host.secret("api_key")`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompatConfig {
    pub provider_id: String,
    pub display_name: String,
    pub base_url: String,
    #[serde(default)]
    pub models_url: Option<String>,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// `required` | `optional` | `none`.
    #[serde(default = "default_auth_kind")]
    pub auth: String,
    /// Header name for api-key auth styles other than bearer.
    #[serde(default)]
    pub api_key_header: Option<String>,
    #[serde(default)]
    pub model_id_policy: Option<String>,
    #[serde(default)]
    pub compat: CompatFlags,
}

fn default_auth_kind() -> String {
    "none".into()
}

/// Cache/affinity behavior flags (`None` = auto per pi detection).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CompatFlags {
    pub send_prompt_cache_key: Option<bool>,
    /// `openai` | `openrouter`.
    pub session_affinity: Option<String>,
    /// `anthropic`.
    pub cache_control_format: Option<String>,
}

impl CompatConfig {
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }
    #[must_use]
    pub fn models_endpoint(&self) -> String {
        self.models_url
            .clone()
            .unwrap_or_else(|| format!("{}/models", self.base_url.trim_end_matches('/')))
    }
    #[must_use]
    pub fn host(&self) -> String {
        self.base_url
            .trim_start_matches("https://")
            .trim_start_matches("http://")
            .split('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    }
}

fn cache_key_enabled(config: &CompatConfig) -> bool {
    match config.compat.send_prompt_cache_key {
        Some(value) => value,
        None => matches!(config.host().as_str(), "openrouter.ai" | "api.openai.com"),
    }
}

fn resolved_affinity(config: &CompatConfig) -> Option<&str> {
    config
        .compat
        .session_affinity
        .as_deref()
        .or_else(|| (config.host() == "openrouter.ai").then_some("openrouter"))
}

/// Session-affinity headers mirroring pi `sessionAffinityFormat`.
#[must_use]
pub fn affinity_headers(config: &CompatConfig, session_id: Option<&str>) -> Vec<(String, String)> {
    let Some(session_id) = session_id else {
        return Vec::new();
    };
    match resolved_affinity(config) {
        Some("openrouter") => vec![("x-session-id".into(), session_id.to_string())],
        Some("openai") => vec![
            ("session_id".into(), session_id.to_string()),
            ("x-client-request-id".into(), session_id.to_string()),
            ("x-session-affinity".into(), session_id.to_string()),
        ],
        _ => Vec::new(),
    }
}

fn cache_control_active(request: &StreamRequest, config: &CompatConfig) -> bool {
    match config.compat.cache_control_format.as_deref() {
        Some(format) => format == "anthropic",
        None => {
            let name = request.model.name.as_str();
            name.starts_with("anthropic/") || name.starts_with("~anthropic/")
        }
    }
}

// ---- request types -------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterReasoning {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exclude: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterChatRequest {
    pub model: String,
    pub messages: Vec<OpenRouterChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stream_options: Option<OpenRouterStreamOptions>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenRouterTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<OpenRouterReasoning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterCacheControl {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterStreamOptions {
    pub include_usage: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<OpenRouterMessageContent>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<OpenRouterToolCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(untagged)]
pub enum OpenRouterMessageContent {
    Text(String),
    Parts(Vec<OpenRouterContentPart>),
}

impl From<String> for OpenRouterMessageContent {
    fn from(value: String) -> Self {
        Self::Text(value)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum OpenRouterContentPart {
    Text {
        text: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        cache_control: Option<OpenRouterCacheControl>,
    },
    ImageUrl {
        image_url: OpenRouterImageUrl,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterImageUrl {
    pub url: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: OpenRouterFunction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cache_control: Option<OpenRouterCacheControl>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterFunction {
    pub name: String,
    pub description: String,
    pub parameters: Value,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterToolCall {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: String,
    pub function: OpenRouterToolCallFunction,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterToolCallFunction {
    pub name: String,
    pub arguments: String,
}

/// Build a chat request applying the configured cache hints: a
/// `prompt_cache_key` derived from `session_id` (when enabled and retention is
/// not `None`), and Anthropic-style `cache_control` breakpoints on the system
/// message, the last tool, and the last text part of the last
/// user/assistant/tool message.
pub fn build_openai_compatible_chat_request_with_cache(
    request: &StreamRequest,
    config: &CompatConfig,
) -> Result<OpenRouterChatRequest, KitError> {
    if request.model.provider != config.provider_id {
        return Err(KitError::Serialization(format!(
            "model provider `{}` does not match OpenAI-compatible provider `{}`",
            request.model.provider, config.provider_id
        )));
    }
    let transcript = transcript::Transcript::new(request.transcript.messages.clone());
    let collapsed = transcript::collapse(&transcript);
    let current_system = transcript::current_system(collapsed.messages());
    let current_tools = transcript::current_tools(collapsed.messages());
    let mut messages = Vec::new();
    if let Some(system) = &current_system {
        let prompt = transcript::render_system_prompt(system);
        if !prompt.is_empty() {
            messages.push(OpenRouterChatMessage {
                role: "system".into(),
                content: Some(prompt.into()),
                tool_call_id: None,
                name: None,
                tool_calls: Vec::new(),
            });
        }
    }
    for message in collapsed.messages() {
        if matches!(message, Message::System { .. }) {
            continue;
        }
        messages.push(convert_message(message)?);
    }
    let mut tools: Vec<OpenRouterTool> = current_tools.iter().map(convert_tool).collect();
    let retention_enabled = request.cache_retention != CacheRetention::None;
    if retention_enabled && cache_control_active(request, config) {
        let cache_control = OpenRouterCacheControl {
            kind: "ephemeral".into(),
            ttl: (request.cache_retention == CacheRetention::Long).then(|| "1h".to_string()),
        };
        apply_anthropic_cache_control(&mut messages, &mut tools, cache_control);
    }
    let prompt_cache_key = if retention_enabled && cache_key_enabled(config) {
        request
            .session_id
            .as_deref()
            .map(|id| id.chars().take(64).collect())
    } else {
        None
    };
    Ok(OpenRouterChatRequest {
        model: request.model.name.clone(),
        messages,
        stream: true,
        stream_options: Some(OpenRouterStreamOptions {
            include_usage: true,
        }),
        tools,
        reasoning: openrouter_reasoning(request.thinking_level),
        prompt_cache_key,
    })
}

fn apply_anthropic_cache_control(
    messages: &mut [OpenRouterChatMessage],
    tools: &mut [OpenRouterTool],
    cache_control: OpenRouterCacheControl,
) {
    for message in messages.iter_mut() {
        if message.role == "system" || message.role == "developer" {
            add_cache_control_to_message(message, &cache_control);
            break;
        }
    }
    if let Some(tool) = tools.last_mut() {
        tool.cache_control = Some(cache_control.clone());
    }
    for message in messages.iter_mut().rev() {
        if matches!(message.role.as_str(), "user" | "assistant" | "tool")
            && add_cache_control_to_message(message, &cache_control)
        {
            break;
        }
    }
}

/// Mirrors pi `addCacheControlToTextContent`: string content becomes a single
/// text part carrying the marker; array content gets the marker on its last
/// text part; returns false when no text part exists so the caller continues
/// scanning backward.
fn add_cache_control_to_message(
    message: &mut OpenRouterChatMessage,
    cache_control: &OpenRouterCacheControl,
) -> bool {
    match &mut message.content {
        Some(OpenRouterMessageContent::Text(text)) => {
            if text.is_empty() {
                return false;
            }
            message.content = Some(OpenRouterMessageContent::Parts(vec![
                OpenRouterContentPart::Text {
                    text: std::mem::take(text),
                    cache_control: Some(cache_control.clone()),
                },
            ]));
            true
        }
        Some(OpenRouterMessageContent::Parts(parts)) => {
            for part in parts.iter_mut().rev() {
                if let OpenRouterContentPart::Text {
                    cache_control: slot,
                    ..
                } = part
                {
                    *slot = Some(cache_control.clone());
                    return true;
                }
            }
            false
        }
        None => false,
    }
}

fn openrouter_reasoning(level: ThinkingLevel) -> Option<OpenRouterReasoning> {
    let (effort, exclude) = match level {
        ThinkingLevel::Off => ("none", Some(true)),
        ThinkingLevel::Minimal => ("minimal", None),
        ThinkingLevel::Low => ("low", None),
        ThinkingLevel::Medium => ("medium", None),
        ThinkingLevel::High => ("high", None),
        ThinkingLevel::XHigh | ThinkingLevel::Max => ("xhigh", None),
    };
    Some(OpenRouterReasoning {
        effort: Some(effort.into()),
        exclude,
    })
}

fn convert_tool(tool: &ToolDeclaration) -> OpenRouterTool {
    OpenRouterTool {
        kind: "function".into(),
        function: OpenRouterFunction {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        },
        cache_control: None,
    }
}

fn convert_message(message: &Message) -> Result<OpenRouterChatMessage, KitError> {
    match message {
        Message::User { content, .. } => Ok(OpenRouterChatMessage {
            role: "user".into(),
            content: Some(user_content(content)?),
            tool_call_id: None,
            name: None,
            tool_calls: Vec::new(),
        }),
        Message::Assistant { content, .. } => {
            let text = optional_text_content(content)?;
            let tool_calls = content
                .iter()
                .filter_map(|block| match block {
                    ContentBlock::ToolCall {
                        id,
                        name,
                        arguments,
                        native_id,
                    } => Some(OpenRouterToolCall {
                        id: native_id.clone().unwrap_or_else(|| id.to_string()),
                        kind: "function".into(),
                        function: OpenRouterToolCallFunction {
                            name: name.clone(),
                            arguments: serde_json::to_string(arguments)
                                .unwrap_or_else(|_| arguments.to_string()),
                        },
                    }),
                    _ => None,
                })
                .collect();
            Ok(OpenRouterChatMessage {
                role: "assistant".into(),
                content: text.map(Into::into),
                tool_call_id: None,
                name: None,
                tool_calls,
            })
        }
        Message::ToolResult {
            tool_call_id,
            tool_name,
            content,
            ..
        } => Ok(OpenRouterChatMessage {
            role: "tool".into(),
            content: Some(text_content(content)?.into()),
            tool_call_id: Some(tool_call_id.to_string()),
            name: Some(tool_name.clone()),
            tool_calls: Vec::new(),
        }),
        Message::System { .. } => Err(KitError::UnsupportedContent(
            "system message must be collapsed before OpenRouter conversion".into(),
        )),
        Message::CompactionSummary { summary, .. } | Message::BranchSummary { summary, .. } => {
            Ok(OpenRouterChatMessage {
                role: "system".into(),
                content: Some(summary.clone().into()),
                tool_call_id: None,
                name: None,
                tool_calls: Vec::new(),
            })
        }
        Message::Custom {
            name,
            model_visible,
            ..
        } if *model_visible => Err(KitError::UnsupportedContent(format!(
            "custom message `{name}` needs app-level conversion before OpenRouter"
        ))),
        Message::Custom { .. } => Err(KitError::UnsupportedContent(
            "runtime-only custom message cannot be sent to OpenRouter".into(),
        )),
    }
}

fn user_content(content: &[ContentBlock]) -> Result<OpenRouterMessageContent, KitError> {
    let mut parts = Vec::new();
    for block in content {
        match block {
            ContentBlock::Text { text, .. } => parts.push(OpenRouterContentPart::Text {
                text: text.clone(),
                cache_control: None,
            }),
            ContentBlock::Image { media_type, data } => {
                parts.push(OpenRouterContentPart::ImageUrl {
                    image_url: OpenRouterImageUrl {
                        url: format!("data:{media_type};base64,{data}"),
                    },
                })
            }
            ContentBlock::Thinking { .. } | ContentBlock::ToolCall { .. } => {}
        }
    }
    if parts.is_empty() {
        Ok(OpenRouterMessageContent::Text(String::new()))
    } else if parts.len() == 1 {
        match parts.remove(0) {
            OpenRouterContentPart::Text { text, .. } => Ok(OpenRouterMessageContent::Text(text)),
            image => Ok(OpenRouterMessageContent::Parts(vec![image])),
        }
    } else {
        Ok(OpenRouterMessageContent::Parts(parts))
    }
}

fn text_content(content: &[ContentBlock]) -> Result<String, KitError> {
    let Some(text) = optional_text_content(content)? else {
        return Ok(String::new());
    };
    Ok(text)
}

fn optional_text_content(content: &[ContentBlock]) -> Result<Option<String>, KitError> {
    let mut text = String::new();
    for block in content {
        match block {
            ContentBlock::Text { text: value, .. } => text.push_str(value),
            ContentBlock::ToolCall { .. } => {}
            ContentBlock::Image { .. } => {
                return Err(KitError::UnsupportedContent(
                    "image content is only supported for user messages".into(),
                ));
            }
            ContentBlock::Thinking { .. } => {}
        }
    }
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

// ---- SSE parsing ---------------------------------------------------------

#[derive(Debug)]
pub struct SseEventParser {
    provider_id: String,
    buffer: String,
    tool_states: BTreeMap<u32, PartialToolState>,
}

impl SseEventParser {
    #[must_use]
    pub fn for_provider(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            buffer: String::new(),
            tool_states: BTreeMap::new(),
        }
    }

    pub fn push_str(&mut self, text: &str) -> Result<Vec<AssistantStreamEvent>, KitError> {
        self.buffer.push_str(text);
        let mut events = Vec::new();
        while let Some(index) = self.buffer.find("\n\n") {
            let frame = self.buffer[..index].to_string();
            self.buffer = self.buffer[index + 2..].to_string();
            events.extend(self.parse_frame(&frame)?);
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return Ok(Vec::new());
        }
        let frame = std::mem::take(&mut self.buffer);
        self.parse_frame(&frame)
    }

    fn parse_frame(&mut self, frame: &str) -> Result<Vec<AssistantStreamEvent>, KitError> {
        let mut events = Vec::new();
        for payload in data_payloads(frame) {
            if payload == "[DONE]" {
                continue;
            }
            let chunk: OpenRouterStreamChunk =
                serde_json::from_str(&payload).map_err(|err| KitError::Sse(err.to_string()))?;
            events.extend(self.chunk_to_events(chunk)?);
        }
        Ok(events)
    }

    fn chunk_to_events(
        &mut self,
        chunk: OpenRouterStreamChunk,
    ) -> Result<Vec<AssistantStreamEvent>, KitError> {
        if let Some(error) = chunk.error {
            return Ok(vec![AssistantStreamEvent::Error {
                message: error.message,
            }]);
        }
        let mut events = Vec::new();
        if let Some(usage) = chunk.usage {
            events.push(AssistantStreamEvent::Usage {
                usage: usage.into(),
            });
        }
        for choice in chunk.choices {
            if let Some(delta) = choice.delta {
                if let Some(content) = delta.content {
                    events.push(AssistantStreamEvent::TextDelta { delta: content });
                }
                if let Some(reasoning) = delta.reasoning.or(delta.reasoning_content) {
                    events.push(AssistantStreamEvent::ThinkingDelta { delta: reasoning });
                }
                for tool_delta in delta.tool_calls {
                    events.push(self.tool_delta_to_event(tool_delta));
                }
            }
            if let Some(reason) = choice.finish_reason {
                events.extend(self.finish_tool_calls()?);
                events.push(AssistantStreamEvent::Done {
                    stop_reason: map_finish_reason(reason.as_deref()),
                    provider: provider_metadata(
                        &self.provider_id,
                        chunk.id.clone(),
                        chunk.model.clone(),
                    ),
                });
            }
        }
        Ok(events)
    }

    fn tool_delta_to_event(&mut self, delta: OpenRouterToolCallDelta) -> AssistantStreamEvent {
        let state = self
            .tool_states
            .entry(delta.index)
            .or_insert_with(|| PartialToolState {
                id: delta
                    .id
                    .as_deref()
                    .and_then(|id| Uuid::parse_str(id).ok())
                    .unwrap_or_else(Uuid::new_v4),
                // Only non-UUID provider ids (e.g. `call_`, `toolu_`, custom
                // ids) need native replay; UUIDs round-trip as `id` already.
                native_id: delta.id.clone().filter(|id| Uuid::parse_str(id).is_err()),
                name: None,
                arguments: String::new(),
            });
        if let Some(id) = delta.id.as_deref().and_then(|id| Uuid::parse_str(id).ok()) {
            state.id = id;
        }
        let mut name = None;
        let mut arguments_delta = String::new();
        if let Some(function) = delta.function {
            if let Some(value) = function.name {
                state.name = Some(value.clone());
                name = Some(value);
            }
            if let Some(value) = function.arguments {
                state.arguments.push_str(&value);
                arguments_delta = value;
            }
        }
        AssistantStreamEvent::ToolCallDelta {
            id: state.id,
            name,
            native_id: state.native_id.clone(),
            arguments_delta,
        }
    }

    fn finish_tool_calls(&mut self) -> Result<Vec<AssistantStreamEvent>, KitError> {
        let states = std::mem::take(&mut self.tool_states);
        let mut events = Vec::new();
        for (_, state) in states {
            if let Some(name) = state.name {
                let arguments = if state.arguments.trim().is_empty() {
                    Value::Object(Default::default())
                } else {
                    serde_json::from_str(&state.arguments).map_err(|err| {
                        KitError::Sse(format!("tool arguments are not JSON: {err}"))
                    })?
                };
                events.push(AssistantStreamEvent::ToolCallDone {
                    id: state.id,
                    name,
                    native_id: state.native_id,
                    arguments,
                });
            }
        }
        Ok(events)
    }
}

fn data_payloads(frame: &str) -> Vec<String> {
    frame
        .lines()
        .filter_map(|line| line.strip_prefix("data:"))
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToOwned::to_owned)
        .collect()
}

#[derive(Debug, Default)]
struct PartialToolState {
    id: OinoId,
    native_id: Option<String>,
    name: Option<String>,
    arguments: String,
}

#[derive(Debug, Deserialize)]
struct OpenRouterStreamChunk {
    id: Option<String>,
    model: Option<String>,
    #[serde(default)]
    choices: Vec<OpenRouterChoice>,
    usage: Option<OpenRouterUsage>,
    error: Option<OpenRouterErrorPayload>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterErrorPayload {
    message: String,
}

#[derive(Debug, Deserialize)]
struct OpenRouterChoice {
    delta: Option<OpenRouterDelta>,
    finish_reason: Option<Option<String>>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterDelta {
    content: Option<String>,
    reasoning: Option<String>,
    reasoning_content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenRouterToolCallDelta>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterToolCallDelta {
    index: u32,
    id: Option<String>,
    function: Option<OpenRouterToolFunctionDelta>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterToolFunctionDelta {
    name: Option<String>,
    arguments: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterUsage {
    #[serde(default)]
    prompt_tokens: u64,
    #[serde(default)]
    completion_tokens: u64,
    #[serde(default)]
    prompt_tokens_details: Option<OpenRouterPromptTokensDetails>,
    #[serde(default)]
    completion_tokens_details: Option<OpenRouterCompletionTokensDetails>,
    cost: Option<f64>,
    #[serde(default)]
    cost_currency: Option<String>,
}

#[derive(Debug, Deserialize)]
struct OpenRouterPromptTokensDetails {
    #[serde(default, alias = "cached_tokens")]
    cache_read_tokens: u64,
    #[serde(default)]
    cache_write_tokens: u64,
}

#[derive(Debug, Deserialize)]
struct OpenRouterCompletionTokensDetails {
    #[serde(default)]
    reasoning_tokens: u64,
}

impl From<OpenRouterUsage> for Usage {
    fn from(value: OpenRouterUsage) -> Self {
        let cost = value.cost.map(|amount| oino_types::UsageCost {
            amount,
            currency: value.cost_currency.unwrap_or_else(|| "USD".into()),
        });
        let (cache_read_tokens, cache_write_tokens) = value
            .prompt_tokens_details
            .as_ref()
            .map_or((0, 0), |details| {
                (details.cache_read_tokens, details.cache_write_tokens)
            });
        Self {
            input_tokens: value.prompt_tokens,
            output_tokens: value.completion_tokens,
            cache_read_tokens,
            cache_write_tokens,
            cache_write_1h_tokens: None,
            reasoning_tokens: value
                .completion_tokens_details
                .as_ref()
                .map_or(0, |details| details.reasoning_tokens),
            cost,
        }
    }
}

fn map_finish_reason(reason: Option<&str>) -> StopReason {
    match reason {
        Some("stop") => StopReason::EndTurn,
        Some("length") => StopReason::Length,
        Some("tool_calls") | Some("function_call") => StopReason::ToolUse,
        Some("error") => StopReason::Error,
        _ => StopReason::Unknown,
    }
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

#[must_use]
pub fn sanitize_error_body(text: &str) -> String {
    if text.trim().is_empty() {
        return "<empty response body>".into();
    }
    text.chars().take(500).collect()
}

// ---- models --------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OpenRouterModelInfo {
    pub id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub context_length: Option<usize>,
}

#[derive(Debug, Deserialize)]
pub struct OpenRouterModelsResponse {
    pub data: Vec<OpenRouterModelInfo>,
}
