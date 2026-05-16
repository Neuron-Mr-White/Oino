#![doc = r#"OpenRouter provider adapter for Oino.

This crate converts Oino's provider-neutral `StreamRequest` into OpenRouter's
OpenAI-compatible chat-completions API and converts streaming SSE chunks back into typed
`AssistantStreamEvent`s for the pure agent loop.
"#]
#![forbid(unsafe_code)]

use async_trait::async_trait;
use futures::StreamExt;
use oino_agent_loop::{
    AbortSignal, BoxFuture, LoopError, LoopResult, StreamEventSink, StreamProvider, StreamRequest,
    ToolDefinition,
};
use oino_auth::{AuthError, AuthStorage, ProviderAuthSpec};
use oino_types::{
    AssistantStreamEvent, ContentBlock, Message, Model, OinoId, ProviderMetadata, StopReason,
    ThinkingLevel, Usage,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex},
    time::Duration,
};
use thiserror::Error;
use uuid::Uuid;

const DEFAULT_BASE_URL: &str = "https://openrouter.ai/api/v1";
const CHAT_COMPLETIONS_PATH: &str = "/chat/completions";
const MODELS_PATH: &str = "/models";

#[derive(Debug, Error)]
pub enum OpenRouterError {
    #[error(transparent)]
    Auth(#[from] AuthError),
    #[error("unsupported OpenRouter request content: {0}")]
    UnsupportedContent(String),
    #[error("OpenRouter serialization error: {0}")]
    Serialization(String),
    #[error("OpenRouter SSE parse error: {0}")]
    Sse(String),
    #[error("OpenRouter HTTP error: {0}")]
    Http(String),
    #[error("OpenRouter request aborted")]
    Aborted,
    #[error("OpenRouter stream sink error: {0}")]
    StreamSink(String),
}

impl From<OpenRouterError> for LoopError {
    fn from(value: OpenRouterError) -> Self {
        match value {
            OpenRouterError::Aborted => Self::Aborted,
            other => Self::Stream(other.to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub base_url: String,
    pub referer: Option<String>,
    pub title: Option<String>,
    pub timeout: Duration,
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_BASE_URL.into(),
            referer: None,
            title: None,
            timeout: Duration::from_secs(120),
        }
    }
}

impl OpenRouterConfig {
    #[must_use]
    pub fn endpoint(&self) -> String {
        format!(
            "{}{}",
            self.base_url.trim_end_matches('/'),
            CHAT_COMPLETIONS_PATH
        )
    }

    #[must_use]
    pub fn models_endpoint(&self) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), MODELS_PATH)
    }
}

#[derive(Debug, Clone)]
pub struct OpenRouterProvider {
    auth: AuthStorage,
    client: reqwest::Client,
    config: OpenRouterConfig,
}

impl OpenRouterProvider {
    pub fn new(auth: AuthStorage, config: OpenRouterConfig) -> Result<Self, OpenRouterError> {
        let client = reqwest::Client::builder()
            .timeout(config.timeout)
            .build()
            .map_err(|err| OpenRouterError::Http(err.to_string()))?;
        Ok(Self {
            auth,
            client,
            config,
        })
    }

    #[must_use]
    pub fn config(&self) -> &OpenRouterConfig {
        &self.config
    }

    async fn stream_events_inner(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
        sink: StreamEventSink,
    ) -> Result<(), OpenRouterError> {
        if signal.is_aborted() {
            emit_to_sink(&sink, AssistantStreamEvent::Aborted).await?;
            return Ok(());
        }
        let api_key = self
            .auth
            .resolve_api_key(&ProviderAuthSpec::openrouter())
            .await?;
        let body = build_chat_request(&request)?;
        let mut builder = self
            .client
            .post(self.config.endpoint())
            .bearer_auth(api_key)
            .json(&body);
        if let Some(referer) = &self.config.referer {
            builder = builder.header("HTTP-Referer", referer);
        }
        if let Some(title) = &self.config.title {
            builder = builder.header("X-OpenRouter-Title", title);
        }
        let response = builder
            .send()
            .await
            .map_err(|err| OpenRouterError::Http(err.to_string()))?;
        let status = response.status();
        if !status.is_success() {
            let text = response.text().await.unwrap_or_else(|_| String::new());
            return Err(OpenRouterError::Http(format!(
                "status {status}: {}",
                sanitize_error_body(&text)
            )));
        }
        let mut parser = SseEventParser::new();
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream.next().await {
            if signal.is_aborted() {
                emit_to_sink(&sink, AssistantStreamEvent::Aborted).await?;
                return Ok(());
            }
            let chunk = chunk.map_err(|err| OpenRouterError::Http(err.to_string()))?;
            let text =
                std::str::from_utf8(&chunk).map_err(|err| OpenRouterError::Sse(err.to_string()))?;
            for event in parser.push_str(text)? {
                emit_to_sink(&sink, event).await?;
            }
        }
        for event in parser.finish()? {
            emit_to_sink(&sink, event).await?;
        }
        Ok(())
    }
}

async fn emit_to_sink(
    sink: &StreamEventSink,
    event: AssistantStreamEvent,
) -> Result<(), OpenRouterError> {
    sink(event)
        .await
        .map_err(|err| OpenRouterError::StreamSink(err.to_string()))
}

#[async_trait]
impl StreamProvider for OpenRouterProvider {
    async fn stream(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
    ) -> LoopResult<Vec<AssistantStreamEvent>> {
        let events = Arc::new(Mutex::new(Vec::new()));
        let sink_events = Arc::clone(&events);
        let sink = Arc::new(move |event| {
            let sink_events = Arc::clone(&sink_events);
            let fut: BoxFuture<'static, LoopResult<()>> = Box::pin(async move {
                let mut events = sink_events
                    .lock()
                    .map_err(|err| LoopError::Stream(format!("event sink lock poisoned: {err}")))?;
                events.push(event);
                Ok(())
            });
            fut
        });
        self.stream_events(request, signal, sink).await?;
        let events = events
            .lock()
            .map_err(|err| LoopError::Stream(format!("event sink lock poisoned: {err}")))?
            .clone();
        Ok(events)
    }

    async fn stream_events(
        &self,
        request: StreamRequest,
        signal: AbortSignal,
        sink: StreamEventSink,
    ) -> LoopResult<()> {
        self.stream_events_inner(request, signal, sink)
            .await
            .map_err(Into::into)
    }
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OpenRouterModelInfo {
    pub id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModelInfo>,
}

pub async fn list_models(
    config: &OpenRouterConfig,
) -> Result<Vec<OpenRouterModelInfo>, OpenRouterError> {
    let client = reqwest::Client::builder()
        .timeout(config.timeout)
        .build()
        .map_err(|err| OpenRouterError::Http(err.to_string()))?;
    let mut builder = client.get(config.models_endpoint());
    if let Some(referer) = &config.referer {
        builder = builder.header("HTTP-Referer", referer);
    }
    if let Some(title) = &config.title {
        builder = builder.header("X-OpenRouter-Title", title);
    }
    let response = builder
        .send()
        .await
        .map_err(|err| OpenRouterError::Http(err.to_string()))?;
    if !response.status().is_success() {
        return Err(OpenRouterError::Http(format!(
            "OpenRouter models request failed with status {}",
            response.status()
        )));
    }
    let body = response
        .json::<OpenRouterModelsResponse>()
        .await
        .map_err(|err| OpenRouterError::Http(err.to_string()))?;
    Ok(body.data)
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterReasoning {
    pub effort: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterChatRequest {
    pub model: String,
    pub messages: Vec<OpenRouterChatMessage>,
    pub stream: bool,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tools: Vec<OpenRouterTool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<OpenRouterReasoning>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterChatMessage {
    pub role: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub tool_calls: Vec<OpenRouterToolCall>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OpenRouterTool {
    #[serde(rename = "type")]
    pub kind: String,
    pub function: OpenRouterFunction,
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

pub fn build_chat_request(
    request: &StreamRequest,
) -> Result<OpenRouterChatRequest, OpenRouterError> {
    if request.model.provider != oino_auth::OPENROUTER_PROVIDER_ID {
        return Err(OpenRouterError::Serialization(format!(
            "model provider `{}` is not openrouter",
            request.model.provider
        )));
    }
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt {
        messages.push(OpenRouterChatMessage {
            role: "system".into(),
            content: Some(system.clone()),
            tool_call_id: None,
            name: None,
            tool_calls: Vec::new(),
        });
    }
    for message in &request.messages {
        messages.push(convert_message(message)?);
    }
    let tools = request.tools.iter().map(convert_tool).collect();
    Ok(OpenRouterChatRequest {
        model: request.model.name.clone(),
        messages,
        stream: true,
        tools,
        reasoning: openrouter_reasoning(request.thinking_level),
    })
}

fn openrouter_reasoning(level: ThinkingLevel) -> Option<OpenRouterReasoning> {
    let effort = match level {
        ThinkingLevel::Off => return None,
        ThinkingLevel::Minimal => "minimal",
        ThinkingLevel::Low => "low",
        ThinkingLevel::Medium => "medium",
        ThinkingLevel::High => "high",
        ThinkingLevel::XHigh => "xhigh",
    };
    Some(OpenRouterReasoning {
        effort: effort.into(),
    })
}

fn convert_tool(tool: &ToolDefinition) -> OpenRouterTool {
    OpenRouterTool {
        kind: "function".into(),
        function: OpenRouterFunction {
            name: tool.name.clone(),
            description: tool.description.clone(),
            parameters: tool.input_schema.clone(),
        },
    }
}

fn convert_message(message: &Message) -> Result<OpenRouterChatMessage, OpenRouterError> {
    match message {
        Message::User { content, .. } => Ok(OpenRouterChatMessage {
            role: "user".into(),
            content: Some(text_content(content)?),
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
                    } => Some(OpenRouterToolCall {
                        id: id.to_string(),
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
                content: text,
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
            content: Some(text_content(content)?),
            tool_call_id: Some(tool_call_id.to_string()),
            name: Some(tool_name.clone()),
            tool_calls: Vec::new(),
        }),
        Message::CompactionSummary { summary, .. } | Message::BranchSummary { summary, .. } => {
            Ok(OpenRouterChatMessage {
                role: "system".into(),
                content: Some(summary.clone()),
                tool_call_id: None,
                name: None,
                tool_calls: Vec::new(),
            })
        }
        Message::Custom {
            name,
            model_visible,
            ..
        } if *model_visible => Err(OpenRouterError::UnsupportedContent(format!(
            "custom message `{name}` needs app-level conversion before OpenRouter"
        ))),
        Message::Custom { .. } => Err(OpenRouterError::UnsupportedContent(
            "runtime-only custom message cannot be sent to OpenRouter".into(),
        )),
    }
}

fn text_content(content: &[ContentBlock]) -> Result<String, OpenRouterError> {
    let Some(text) = optional_text_content(content)? else {
        return Ok(String::new());
    };
    Ok(text)
}

fn optional_text_content(content: &[ContentBlock]) -> Result<Option<String>, OpenRouterError> {
    let mut text = String::new();
    for block in content {
        match block {
            ContentBlock::Text { text: value } => text.push_str(value),
            ContentBlock::ToolCall { .. } => {}
            ContentBlock::Image { .. } => {
                return Err(OpenRouterError::UnsupportedContent(
                    "image content is not supported in the first OpenRouter adapter".into(),
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

#[derive(Debug, Default)]
pub struct SseEventParser {
    buffer: String,
    tool_states: BTreeMap<u32, PartialToolState>,
}

impl SseEventParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push_str(&mut self, text: &str) -> Result<Vec<AssistantStreamEvent>, OpenRouterError> {
        self.buffer.push_str(text);
        let mut events = Vec::new();
        while let Some(index) = self.buffer.find("\n\n") {
            let frame = self.buffer[..index].to_string();
            self.buffer = self.buffer[index + 2..].to_string();
            events.extend(self.parse_frame(&frame)?);
        }
        Ok(events)
    }

    pub fn finish(&mut self) -> Result<Vec<AssistantStreamEvent>, OpenRouterError> {
        if self.buffer.trim().is_empty() {
            self.buffer.clear();
            return Ok(Vec::new());
        }
        let frame = std::mem::take(&mut self.buffer);
        self.parse_frame(&frame)
    }

    fn parse_frame(&mut self, frame: &str) -> Result<Vec<AssistantStreamEvent>, OpenRouterError> {
        let mut events = Vec::new();
        for payload in data_payloads(frame) {
            if payload == "[DONE]" {
                continue;
            }
            let chunk: OpenRouterStreamChunk = serde_json::from_str(&payload)
                .map_err(|err| OpenRouterError::Sse(err.to_string()))?;
            events.extend(self.chunk_to_events(chunk)?);
        }
        Ok(events)
    }

    fn chunk_to_events(
        &mut self,
        chunk: OpenRouterStreamChunk,
    ) -> Result<Vec<AssistantStreamEvent>, OpenRouterError> {
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
                    provider: provider_metadata(chunk.id.clone(), chunk.model.clone()),
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
            arguments_delta,
        }
    }

    fn finish_tool_calls(&mut self) -> Result<Vec<AssistantStreamEvent>, OpenRouterError> {
        let states = std::mem::take(&mut self.tool_states);
        let mut events = Vec::new();
        for (_, state) in states {
            if let Some(name) = state.name {
                let arguments = if state.arguments.trim().is_empty() {
                    Value::Object(Default::default())
                } else {
                    serde_json::from_str(&state.arguments).map_err(|err| {
                        OpenRouterError::Sse(format!("tool arguments are not JSON: {err}"))
                    })?
                };
                events.push(AssistantStreamEvent::ToolCallDone {
                    id: state.id,
                    name,
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
    total_tokens: u64,
}

impl From<OpenRouterUsage> for Usage {
    fn from(value: OpenRouterUsage) -> Self {
        Self {
            input_tokens: value.prompt_tokens,
            output_tokens: value.completion_tokens,
            cache_read_tokens: 0,
            cache_write_tokens: value
                .total_tokens
                .saturating_sub(value.prompt_tokens + value.completion_tokens),
            cost: None,
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

fn provider_metadata(id: Option<String>, model: Option<String>) -> Option<ProviderMetadata> {
    if id.is_none() && model.is_none() {
        return None;
    }
    let mut values = BTreeMap::new();
    if let Some(id) = id {
        values.insert("id".into(), Value::String(id));
    }
    Some(ProviderMetadata {
        request_id: None,
        model: model.map(|name| Model::new(oino_auth::OPENROUTER_PROVIDER_ID, name)),
        values,
    })
}

fn sanitize_error_body(text: &str) -> String {
    if text.trim().is_empty() {
        return "<empty response body>".into();
    }
    text.chars().take(500).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::{StreamRequest, ToolDefinition};
    use oino_types::{ContentBlock, Message, Model, StopReason, ThinkingLevel};
    use serde_json::json;

    fn request(messages: Vec<Message>) -> StreamRequest {
        StreamRequest {
            model: Model::new("openrouter", "openai/gpt-4o-mini"),
            thinking_level: ThinkingLevel::Off,
            system_prompt: Some("be kind".into()),
            messages,
            tools: Vec::new(),
        }
    }

    #[test]
    fn serializes_text_messages() {
        let built = match build_chat_request(&request(vec![Message::user_text("hello")])) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        let json = match serde_json::to_value(built) {
            Ok(value) => value,
            Err(err) => panic!("serialize failed: {err}"),
        };
        assert_eq!(
            json,
            json!({
                "model": "openai/gpt-4o-mini",
                "messages": [
                    {"role": "system", "content": "be kind"},
                    {"role": "user", "content": "hello"}
                ],
                "stream": true
            })
        );
    }

    #[test]
    fn serializes_assistant_messages_without_replaying_thinking() {
        let built = match build_chat_request(&request(vec![Message::Assistant {
            id: Uuid::new_v4(),
            content: vec![
                ContentBlock::Thinking {
                    text: "private reasoning".into(),
                    redacted: false,
                },
                ContentBlock::Text {
                    text: "public answer".into(),
                },
            ],
            stop_reason: Some(StopReason::EndTurn),
            usage: None,
            provider: None,
        }])) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        let json = match serde_json::to_value(built) {
            Ok(value) => value,
            Err(err) => panic!("serialize failed: {err}"),
        };
        assert_eq!(json["messages"][1]["role"], "assistant");
        assert_eq!(json["messages"][1]["content"], "public answer");
    }

    #[test]
    fn serializes_reasoning_effort_when_enabled() {
        let mut req = request(vec![Message::user_text("think")]);
        req.thinking_level = ThinkingLevel::High;
        let built = match build_chat_request(&req) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        let json = match serde_json::to_value(built) {
            Ok(value) => value,
            Err(err) => panic!("serialize failed: {err}"),
        };
        assert_eq!(json["reasoning"]["effort"], "high");
    }

    #[test]
    fn deserializes_model_catalog_response() {
        let response = match serde_json::from_value::<OpenRouterModelsResponse>(json!({
            "data": [{
                "id": "openai/gpt-4o-mini",
                "name": "GPT 4o Mini",
                "supported_parameters": ["tools", "reasoning"]
            }]
        })) {
            Ok(value) => value,
            Err(err) => panic!("deserialize failed: {err}"),
        };
        assert_eq!(response.data[0].id, "openai/gpt-4o-mini");
        assert!(response.data[0]
            .supported_parameters
            .contains(&"reasoning".to_string()));
    }

    #[test]
    fn serializes_tools() {
        let mut req = request(vec![Message::user_text("use tool")]);
        req.tools.push(ToolDefinition {
            name: "read".into(),
            description: "Read a file".into(),
            input_schema: json!({"type":"object","required":["path"],"properties":{"path":{"type":"string"}}}),
        });
        let built = match build_chat_request(&req) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        let json = match serde_json::to_value(built) {
            Ok(value) => value,
            Err(err) => panic!("serialize failed: {err}"),
        };
        assert_eq!(json["tools"][0]["type"], "function");
        assert_eq!(json["tools"][0]["function"]["name"], "read");
        assert_eq!(json["tools"][0]["function"]["parameters"]["type"], "object");
    }

    #[test]
    fn unsupported_content_is_error() {
        let message = Message::User {
            id: Uuid::new_v4(),
            content: vec![ContentBlock::Image {
                media_type: "image/png".into(),
                data: "...".into(),
            }],
        };
        match build_chat_request(&request(vec![message])) {
            Err(OpenRouterError::UnsupportedContent(_)) => {}
            other => panic!("expected unsupported content, got {other:?}"),
        }
    }

    #[test]
    fn parses_sse_text_usage_done() {
        let mut parser = SseEventParser::new();
        let input = concat!(
            "data: {\"id\":\"req-1\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":1,\"completion_tokens\":2,\"total_tokens\":3}}\n\n",
            "data: {\"id\":\"req-1\",\"model\":\"m\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let events = match parser.push_str(input) {
            Ok(events) => events,
            Err(err) => panic!("parse failed: {err}"),
        };
        assert!(events.contains(&AssistantStreamEvent::TextDelta {
            delta: "hel".into()
        }));
        assert!(events.contains(&AssistantStreamEvent::TextDelta { delta: "lo".into() }));
        assert!(events.iter().any(|event| matches!(event, AssistantStreamEvent::Usage { usage } if usage.input_tokens == 1 && usage.output_tokens == 2)));
        assert!(events.iter().any(|event| matches!(
            event,
            AssistantStreamEvent::Done {
                stop_reason: StopReason::EndTurn,
                ..
            }
        )));
    }

    #[test]
    fn parses_partial_sse_frames() {
        let mut parser = SseEventParser::new();
        let first = match parser.push_str("data: {\"choices\":[{\"delta\":{\"content\":\"a\"},") {
            Ok(events) => events,
            Err(err) => panic!("parse failed: {err}"),
        };
        assert!(first.is_empty());
        let second = match parser.push_str("\"finish_reason\":null}]}\n\n") {
            Ok(events) => events,
            Err(err) => panic!("parse failed: {err}"),
        };
        assert_eq!(
            second,
            vec![AssistantStreamEvent::TextDelta { delta: "a".into() }]
        );
    }

    #[test]
    fn parses_tool_call_delta_and_done() {
        let mut parser = SseEventParser::new();
        let input = concat!(
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"path\\\"\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\":\\\"README.md\\\"}\"}}]},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n"
        );
        let events = match parser.push_str(input) {
            Ok(events) => events,
            Err(err) => panic!("parse failed: {err}"),
        };
        assert!(events.iter().any(|event| matches!(event, AssistantStreamEvent::ToolCallDelta { name: Some(name), .. } if name == "read")));
        assert!(events.iter().any(|event| matches!(event, AssistantStreamEvent::ToolCallDone { name, arguments, .. } if name == "read" && arguments["path"] == "README.md")));
        assert!(events.iter().any(|event| matches!(
            event,
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                ..
            }
        )));
    }

    #[test]
    fn parses_error_payload() {
        let mut parser = SseEventParser::new();
        let events = match parser.push_str("data: {\"error\":{\"message\":\"bad auth\"}}\n\n") {
            Ok(events) => events,
            Err(err) => panic!("parse failed: {err}"),
        };
        assert_eq!(
            events,
            vec![AssistantStreamEvent::Error {
                message: "bad auth".into()
            }]
        );
    }

    #[tokio::test]
    async fn provider_reports_missing_auth_before_http() {
        let auth = AuthStorage::new(
            oino_auth::AuthConfig::new(
                std::env::temp_dir().join("oino-provider-missing-auth.json"),
            )
            .with_process_env(false),
        );
        let provider = match OpenRouterProvider::new(auth, OpenRouterConfig::default()) {
            Ok(provider) => provider,
            Err(err) => panic!("provider init failed: {err}"),
        };
        let sink = Arc::new(|_event| {
            let fut: BoxFuture<'static, LoopResult<()>> = Box::pin(async { Ok(()) });
            fut
        });
        match provider
            .stream_events_inner(
                request(vec![Message::user_text("hi")]),
                AbortSignal::new(),
                sink,
            )
            .await
        {
            Err(OpenRouterError::Auth(AuthError::MissingCredential { provider, .. })) => {
                assert_eq!(provider, "openrouter");
            }
            other => panic!("expected missing auth, got {other:?}"),
        }
    }

    #[test]
    fn maps_finish_reasons() {
        assert_eq!(map_finish_reason(Some("stop")), StopReason::EndTurn);
        assert_eq!(map_finish_reason(Some("length")), StopReason::Length);
        assert_eq!(map_finish_reason(Some("tool_calls")), StopReason::ToolUse);
        assert_eq!(map_finish_reason(Some("error")), StopReason::Error);
        assert_eq!(map_finish_reason(None), StopReason::Unknown);
    }
}
