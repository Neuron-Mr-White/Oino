#![doc = r#"OpenRouter provider adapter for Oino.

This crate converts Oino's provider-neutral `StreamRequest` into OpenRouter's
OpenAI-compatible chat-completions API and converts streaming SSE chunks back into typed
`AssistantStreamEvent`s for the pure agent loop.

## Boundary

`oino-provider-openrouter` owns only OpenRouter-specific HTTP and JSON details: the
base URL and endpoints, attribution headers, request serialization, model listing,
SSE parsing, finish-reason mapping, reasoning-parameter mapping, and tool-call chunk
assembly. It asks `oino-auth` for credentials but does not decide where secrets come
from, and it does not own session storage, model-cache persistence, UI state, or
prompt assembly.

## Public API map

- [`OpenRouterConfig`] carries the base URL, optional `HTTP-Referer` and
  `X-OpenRouter-Title` headers, and request timeout used by both chat and model-list
  calls.
- [`OpenRouterProvider`] implements the agent-loop streaming provider contract. It
  resolves the API key, builds a [`OpenRouterChatRequest`], posts to
  `/chat/completions`, and feeds response bytes into [`SseEventParser`].
- [`build_chat_request`] is the fixture-friendly conversion seam from Oino messages,
  tools, model ids, and thinking levels into OpenRouter chat-completions JSON.
- [`list_models`] fetches `/models` and returns [`OpenRouterModelInfo`] with
  `supported_parameters`; `oino-app` owns caching and settings integration.
- The `OpenRouter*` request structs are public so tests and documentation can assert
  exact JSON without making network calls.

## Contributor rules

Keep provider-neutral stream semantics in `oino-agent-loop` and Oino message shapes in
`oino-types`; add only OpenRouter protocol translations here. Prefer serialization
fixtures and SSE parser tests over live HTTP tests. When OpenRouter adds fields, map
them into existing `AssistantStreamEvent` or `StopReason` variants when possible, and
only extend core types when the concept is provider-neutral. Preserve abort checks,
sink-error propagation, and sanitized provider error bodies so credentials and large
HTML error pages do not leak into transcripts.
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

pub const OPENROUTER_PROVIDER_ID: &str = "openrouter";
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

    #[must_use]
    pub fn openai_compatible_config(&self) -> OpenAiCompatibleConfig {
        let mut config = OpenAiCompatibleConfig::new(
            oino_auth::OPENROUTER_PROVIDER_ID,
            "OpenRouter",
            self.base_url.clone(),
        )
        .with_auth(OpenAiCompatibleAuth::Bearer {
            spec: ProviderAuthSpec::openrouter(),
        })
        .with_timeout(self.timeout);
        if let Some(referer) = &self.referer {
            config = config.with_header("HTTP-Referer", referer.clone());
        }
        if let Some(title) = &self.title {
            config = config.with_header("X-OpenRouter-Title", title.clone());
        }
        config
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAiCompatibleAuth {
    None,
    Bearer {
        spec: ProviderAuthSpec,
    },
    OptionalBearer {
        spec: ProviderAuthSpec,
    },
    ApiKeyHeader {
        spec: ProviderAuthSpec,
        header_name: String,
    },
}

impl OpenAiCompatibleAuth {
    #[must_use]
    pub const fn requires_credential(&self) -> bool {
        matches!(self, Self::Bearer { .. } | Self::ApiKeyHeader { .. })
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleConfig {
    pub provider_id: String,
    pub display_name: String,
    pub base_url: String,
    pub timeout: Duration,
    pub auth: OpenAiCompatibleAuth,
    pub headers: BTreeMap<String, String>,
    pub chat_endpoint: Option<String>,
}

impl OpenAiCompatibleConfig {
    #[must_use]
    pub fn new(
        provider_id: impl Into<String>,
        display_name: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            display_name: display_name.into(),
            base_url: base_url.into(),
            timeout: Duration::from_secs(120),
            auth: OpenAiCompatibleAuth::None,
            headers: BTreeMap::new(),
            chat_endpoint: None,
        }
    }

    #[must_use]
    pub fn from_profile(profile: oino_provider_catalog::OpenAiCompatibleProfile) -> Self {
        let credential = profile.credential_spec();
        let auth = credential
            .env_var
            .map(|env_var| {
                ProviderAuthSpec::new(credential.provider_id, credential.auth_key, env_var)
            })
            .map(|spec| {
                if profile.requires_api_key {
                    OpenAiCompatibleAuth::Bearer { spec }
                } else {
                    OpenAiCompatibleAuth::OptionalBearer { spec }
                }
            })
            .unwrap_or(OpenAiCompatibleAuth::None);
        Self::new(profile.id, profile.display_name, profile.api_base).with_auth(auth)
    }

    #[must_use]
    pub fn from_provider(provider: oino_provider_catalog::ProviderDescriptor) -> Option<Self> {
        match provider.target {
            oino_provider_catalog::ProviderTarget::OpenRouter => {
                Some(OpenRouterConfig::default().openai_compatible_config())
            }
            oino_provider_catalog::ProviderTarget::OpenAiApiKey => {
                let credential = provider.credential_spec();
                credential.env_var.map(|env_var| {
                    Self::new(
                        provider.id,
                        provider.display_name,
                        "https://api.openai.com/v1",
                    )
                    .with_auth(OpenAiCompatibleAuth::Bearer {
                        spec: ProviderAuthSpec::new(
                            credential.provider_id,
                            credential.auth_key,
                            env_var,
                        ),
                    })
                })
            }
            oino_provider_catalog::ProviderTarget::OpenAiCompatible { profile_id } => {
                oino_provider_catalog::openai_compatible_profile_by_id(profile_id)
                    .map(|profile| Self::from_profile(*profile))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn endpoint(&self) -> String {
        self.chat_endpoint.clone().unwrap_or_else(|| {
            format!(
                "{}{}",
                self.base_url.trim_end_matches('/'),
                CHAT_COMPLETIONS_PATH
            )
        })
    }

    #[must_use]
    pub fn models_endpoint(&self) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), MODELS_PATH)
    }

    #[must_use]
    pub fn with_auth(mut self, auth: OpenAiCompatibleAuth) -> Self {
        self.auth = auth;
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }

    #[must_use]
    pub fn with_chat_endpoint(mut self, endpoint: impl Into<String>) -> Self {
        self.chat_endpoint = Some(endpoint.into());
        self
    }
}

#[derive(Debug, Clone)]
pub struct OpenAiCompatibleProvider {
    auth: AuthStorage,
    client: reqwest::Client,
    config: OpenAiCompatibleConfig,
}

impl OpenAiCompatibleProvider {
    pub fn new(auth: AuthStorage, config: OpenAiCompatibleConfig) -> Result<Self, OpenRouterError> {
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
    pub fn config(&self) -> &OpenAiCompatibleConfig {
        &self.config
    }

    pub fn build_chat_request(
        &self,
        request: &StreamRequest,
    ) -> Result<OpenRouterChatRequest, OpenRouterError> {
        build_openai_compatible_chat_request(request, &self.config.provider_id)
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
        let body = self.build_chat_request(&request)?;
        let mut builder = self.client.post(self.config.endpoint()).json(&body);
        builder = self.apply_auth(builder).await?;
        for (name, value) in &self.config.headers {
            builder = builder.header(name, value);
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
        let mut parser = SseEventParser::for_provider(self.config.provider_id.clone());
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

    async fn apply_auth(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, OpenRouterError> {
        match &self.config.auth {
            OpenAiCompatibleAuth::None => Ok(builder),
            OpenAiCompatibleAuth::Bearer { spec } => {
                let api_key = self.auth.resolve_api_key(spec).await?;
                Ok(builder.bearer_auth(api_key))
            }
            OpenAiCompatibleAuth::OptionalBearer { spec } => match self.auth.resolve(spec).await? {
                Some(credential) => {
                    let Some(api_key) = credential.as_api_key() else {
                        return Err(OpenRouterError::Auth(AuthError::NotApiKey {
                            provider: spec.provider_id.clone(),
                        }));
                    };
                    Ok(builder.bearer_auth(api_key))
                }
                None => Ok(builder),
            },
            OpenAiCompatibleAuth::ApiKeyHeader { spec, header_name } => {
                let api_key = self.auth.resolve_api_key(spec).await?;
                Ok(builder.header(header_name.as_str(), api_key))
            }
        }
    }
}

#[async_trait]
impl StreamProvider for OpenAiCompatibleProvider {
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

#[derive(Debug, Clone)]
pub struct OpenRouterProvider {
    inner: OpenAiCompatibleProvider,
    config: OpenRouterConfig,
}

impl OpenRouterProvider {
    pub fn new(auth: AuthStorage, config: OpenRouterConfig) -> Result<Self, OpenRouterError> {
        let inner = OpenAiCompatibleProvider::new(auth, config.openai_compatible_config())?;
        Ok(Self { inner, config })
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
        self.inner.stream_events_inner(request, signal, sink).await
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
    pub owned_by: Option<String>,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub context_length: Option<usize>,
    #[serde(default)]
    pub pricing: Option<OpenRouterModelPricing>,
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OpenRouterModelPricing {
    #[serde(default)]
    pub prompt: Option<String>,
    #[serde(default)]
    pub completion: Option<String>,
    #[serde(default)]
    pub input_cache_read: Option<String>,
    #[serde(default)]
    pub input_cache_write: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct OpenRouterModelsResponse {
    data: Vec<OpenRouterModelInfo>,
}

pub async fn list_models(
    config: &OpenRouterConfig,
) -> Result<Vec<OpenRouterModelInfo>, OpenRouterError> {
    list_openai_compatible_models(&config.openai_compatible_config()).await
}

pub async fn list_openai_compatible_models(
    config: &OpenAiCompatibleConfig,
) -> Result<Vec<OpenRouterModelInfo>, OpenRouterError> {
    let client = reqwest::Client::builder()
        .timeout(config.timeout)
        .build()
        .map_err(|err| OpenRouterError::Http(err.to_string()))?;
    let mut builder = client.get(config.models_endpoint());
    for (name, value) in &config.headers {
        builder = builder.header(name, value);
    }
    let response = builder
        .send()
        .await
        .map_err(|err| OpenRouterError::Http(err.to_string()))?;
    if !response.status().is_success() {
        return Err(OpenRouterError::Http(format!(
            "{} models request failed with status {}",
            config.display_name,
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
    Text { text: String },
    ImageUrl { image_url: OpenRouterImageUrl },
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
    build_openai_compatible_chat_request(request, oino_auth::OPENROUTER_PROVIDER_ID)
}

pub fn build_openai_compatible_chat_request(
    request: &StreamRequest,
    provider_id: &str,
) -> Result<OpenRouterChatRequest, OpenRouterError> {
    if request.model.provider != provider_id {
        return Err(OpenRouterError::Serialization(format!(
            "model provider `{}` does not match OpenAI-compatible provider `{provider_id}`",
            request.model.provider
        )));
    }
    let mut messages = Vec::new();
    if let Some(system) = &request.system_prompt {
        messages.push(OpenRouterChatMessage {
            role: "system".into(),
            content: Some(system.clone().into()),
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
        stream_options: Some(OpenRouterStreamOptions {
            include_usage: true,
        }),
        tools,
        reasoning: openrouter_reasoning(request.thinking_level),
    })
}

fn openrouter_reasoning(level: ThinkingLevel) -> Option<OpenRouterReasoning> {
    let (effort, exclude) = match level {
        ThinkingLevel::Off => ("none", Some(true)),
        ThinkingLevel::Minimal => ("minimal", None),
        ThinkingLevel::Low => ("low", None),
        ThinkingLevel::Medium => ("medium", None),
        ThinkingLevel::High => ("high", None),
        ThinkingLevel::XHigh => ("xhigh", None),
    };
    Some(OpenRouterReasoning {
        effort: Some(effort.into()),
        exclude,
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
        } if *model_visible => Err(OpenRouterError::UnsupportedContent(format!(
            "custom message `{name}` needs app-level conversion before OpenRouter"
        ))),
        Message::Custom { .. } => Err(OpenRouterError::UnsupportedContent(
            "runtime-only custom message cannot be sent to OpenRouter".into(),
        )),
    }
}

fn user_content(content: &[ContentBlock]) -> Result<OpenRouterMessageContent, OpenRouterError> {
    let mut parts = Vec::new();
    for block in content {
        match block {
            ContentBlock::Text { text } => {
                parts.push(OpenRouterContentPart::Text { text: text.clone() })
            }
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
            OpenRouterContentPart::Text { text } => Ok(OpenRouterMessageContent::Text(text)),
            image => Ok(OpenRouterMessageContent::Parts(vec![image])),
        }
    } else {
        Ok(OpenRouterMessageContent::Parts(parts))
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

#[derive(Debug)]
pub struct SseEventParser {
    provider_id: String,
    buffer: String,
    tool_states: BTreeMap<u32, PartialToolState>,
}

impl Default for SseEventParser {
    fn default() -> Self {
        Self::for_provider(oino_auth::OPENROUTER_PROVIDER_ID)
    }
}

impl SseEventParser {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn for_provider(provider_id: impl Into<String>) -> Self {
        Self {
            provider_id: provider_id.into(),
            buffer: String::new(),
            tool_states: BTreeMap::new(),
        }
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
    prompt_tokens_details: Option<OpenRouterPromptTokensDetails>,
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

fn sanitize_error_body(text: &str) -> String {
    if text.trim().is_empty() {
        return "<empty response body>".into();
    }
    text.chars().take(500).collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterUsageReport {
    pub limits: Vec<OpenRouterUsageLimit>,
    pub extra_info: Vec<(String, String)>,
    pub balance: Option<OpenRouterCreditBalance>,
    pub hard_limit_reached: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterUsageLimit {
    pub name: String,
    pub usage_percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterCreditBalance {
    pub amount: f64,
    pub currency: String,
    pub total_credits: f64,
    pub total_usage: f64,
}

pub fn parse_openrouter_usage_payloads(
    key_payload: Option<&str>,
    credits_payload: Option<&str>,
) -> Result<OpenRouterUsageReport, OpenRouterError> {
    let key = key_payload
        .map(|raw| serde_json::from_str::<Value>(raw))
        .transpose()
        .map_err(|err| {
            OpenRouterError::Serialization(format!("invalid key usage fixture: {err}"))
        })?;
    let credits = credits_payload
        .map(|raw| serde_json::from_str::<Value>(raw))
        .transpose()
        .map_err(|err| {
            OpenRouterError::Serialization(format!("invalid credits usage fixture: {err}"))
        })?;
    Ok(parse_openrouter_usage_values(
        key.as_ref(),
        credits.as_ref(),
    ))
}

pub fn parse_openrouter_usage_values(
    key_payload: Option<&Value>,
    credits_payload: Option<&Value>,
) -> OpenRouterUsageReport {
    let mut report = OpenRouterUsageReport {
        limits: Vec::new(),
        extra_info: Vec::new(),
        balance: None,
        hard_limit_reached: false,
    };

    if let Some(data) = credits_payload.and_then(openrouter_data_object) {
        let total_credits = data
            .get("total_credits")
            .and_then(openrouter_f64)
            .unwrap_or(0.0);
        let total_usage = data
            .get("total_usage")
            .and_then(openrouter_f64)
            .unwrap_or(0.0);
        let balance = total_credits - total_usage;
        if total_credits > 0.0 {
            report.limits.push(OpenRouterUsageLimit {
                name: "Credits".into(),
                usage_percent: usage_percent_from_used_limit(total_usage, total_credits),
                resets_at: None,
            });
            report.balance = Some(OpenRouterCreditBalance {
                amount: balance,
                currency: "USD".into(),
                total_credits,
                total_usage,
            });
            report.extra_info.push((
                "Balance".into(),
                format!("${balance:.2} / ${total_credits:.2}"),
            ));
        }
    }

    if let Some(data) = key_payload.and_then(openrouter_data_object) {
        for (label, key) in [
            ("Today", "usage_daily"),
            ("This week", "usage_weekly"),
            ("This month", "usage_monthly"),
        ] {
            if let Some(value) = data.get(key).and_then(openrouter_f64) {
                report
                    .extra_info
                    .push((label.into(), format!("${value:.2}")));
            }
        }
        if let Some(limit) = data
            .get("limit")
            .and_then(openrouter_f64)
            .filter(|value| *value > 0.0)
        {
            let remaining = data
                .get("limit_remaining")
                .and_then(openrouter_f64)
                .unwrap_or(0.0);
            let used = (limit - remaining).max(0.0);
            report.hard_limit_reached = remaining <= 0.0;
            report.limits.push(OpenRouterUsageLimit {
                name: "Key limit".into(),
                usage_percent: usage_percent_from_used_limit(used, limit),
                resets_at: None,
            });
            report.extra_info.push((
                "Key limit".into(),
                format!("${remaining:.2} remaining / ${limit:.2}"),
            ));
        }
    }

    report
}

fn openrouter_data_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.get("data").unwrap_or(value).as_object()
}

fn openrouter_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn usage_percent_from_used_limit(used: f64, limit: f64) -> f64 {
    if !used.is_finite() || !limit.is_finite() || limit <= 0.0 {
        return 0.0;
    }
    ((used.max(0.0) / limit) * 100.0).clamp(0.0, 100.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_agent_loop::{StreamRequest, ToolDefinition};
    use oino_types::{ContentBlock, Message, Model, StopReason, ThinkingLevel};
    use serde_json::json;

    fn request(messages: Vec<Message>) -> StreamRequest {
        request_for_provider("openrouter", "openai/gpt-4o-mini", messages)
    }

    fn request_for_provider(
        provider: impl Into<String>,
        model: impl Into<String>,
        messages: Vec<Message>,
    ) -> StreamRequest {
        StreamRequest {
            model: Model::new(provider, model),
            thinking_level: ThinkingLevel::Off,
            system_prompt: Some("be kind".into()),
            messages,
            tools: Vec::new(),
        }
    }

    #[test]
    fn parses_openrouter_usage_key_and_credit_fixtures() {
        let report = parse_openrouter_usage_payloads(
            Some(
                r#"{
                    "data": {
                        "usage_daily": 1.25,
                        "usage_weekly": "2.50",
                        "usage_monthly": 3.75,
                        "limit": 10.0,
                        "limit_remaining": 4.0
                    }
                }"#,
            ),
            Some(
                r#"{
                    "data": {
                        "total_credits": 20.0,
                        "total_usage": 5.0
                    }
                }"#,
            ),
        )
        .unwrap_or_else(|err| panic!("usage fixtures should parse: {err}"));

        assert_eq!(
            report.balance.as_ref().map(|balance| balance.amount),
            Some(15.0)
        );
        assert_eq!(report.limits.len(), 2);
        assert_eq!(report.limits[0].name, "Credits");
        assert_eq!(report.limits[0].usage_percent, 25.0);
        assert_eq!(report.limits[1].name, "Key limit");
        assert_eq!(report.limits[1].usage_percent, 60.0);
        assert!(report
            .extra_info
            .iter()
            .any(|(key, value)| key == "This week" && value == "$2.50"));
        assert!(!report.hard_limit_reached);
    }

    #[test]
    fn parses_openrouter_exhausted_key_limit_as_hard_limit() {
        let report = parse_openrouter_usage_payloads(
            Some(r#"{"data":{"limit":5,"limit_remaining":0}}"#),
            None,
        )
        .unwrap_or_else(|err| panic!("usage fixture should parse: {err}"));

        assert!(report.hard_limit_reached);
        assert_eq!(report.limits[0].usage_percent, 100.0);
    }

    #[test]
    fn serializes_openai_compatible_profile_request() {
        let profile = match oino_provider_catalog::openai_compatible_profile_by_id("deepseek") {
            Some(profile) => *profile,
            None => panic!("deepseek profile missing"),
        };
        let config = OpenAiCompatibleConfig::from_profile(profile);
        let built = match build_openai_compatible_chat_request(
            &request_for_provider(
                "deepseek",
                "deepseek-chat",
                vec![Message::user_text("hello")],
            ),
            &config.provider_id,
        ) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        assert_eq!(built.model, "deepseek-chat");
        assert_eq!(
            built.stream_options,
            Some(OpenRouterStreamOptions {
                include_usage: true
            })
        );
        assert_eq!(
            config.endpoint(),
            "https://api.deepseek.com/chat/completions"
        );
        assert!(config.auth.requires_credential());
    }

    #[test]
    fn azure_style_config_uses_custom_endpoint_and_api_key_header() {
        let spec = ProviderAuthSpec::new("azure", "azure", "AZURE_OPENAI_API_KEY");
        let config = OpenAiCompatibleConfig::new(
            "azure",
            "Azure OpenAI",
            "https://resource.openai.azure.com",
        )
        .with_chat_endpoint("https://resource.openai.azure.com/openai/deployments/gpt4/chat/completions?api-version=2024-10-21")
        .with_auth(OpenAiCompatibleAuth::ApiKeyHeader {
            spec: spec.clone(),
            header_name: "api-key".into(),
        });

        assert_eq!(
            config.endpoint(),
            "https://resource.openai.azure.com/openai/deployments/gpt4/chat/completions?api-version=2024-10-21"
        );
        assert!(config.auth.requires_credential());
        match &config.auth {
            OpenAiCompatibleAuth::ApiKeyHeader { spec, header_name } => {
                assert_eq!(spec.env_var, "AZURE_OPENAI_API_KEY");
                assert_eq!(header_name, "api-key");
            }
            other => panic!("expected API-key header auth, got {other:?}"),
        }
    }

    #[test]
    fn local_profile_can_build_without_required_auth() {
        let profile = match oino_provider_catalog::openai_compatible_profile_by_id("ollama") {
            Some(profile) => *profile,
            None => panic!("ollama profile missing"),
        };
        let config = OpenAiCompatibleConfig::from_profile(profile);
        let auth = AuthStorage::new(
            oino_auth::AuthConfig::new(std::env::temp_dir().join("oino-ollama-auth.json"))
                .with_process_env(false),
        );
        let provider = match OpenAiCompatibleProvider::new(auth, config) {
            Ok(provider) => provider,
            Err(err) => panic!("provider init failed: {err}"),
        };
        let built = match provider.build_chat_request(&request_for_provider(
            "ollama",
            "llama3.2",
            vec![Message::user_text("hello")],
        )) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        assert_eq!(built.model, "llama3.2");
        assert!(!provider.config().auth.requires_credential());
    }

    #[test]
    fn rejects_model_for_wrong_openai_compatible_provider() {
        match build_openai_compatible_chat_request(
            &request_for_provider(
                "openrouter",
                "openai/gpt-4o-mini",
                vec![Message::user_text("hi")],
            ),
            "deepseek",
        ) {
            Err(OpenRouterError::Serialization(message)) => {
                assert!(message.contains("does not match"));
            }
            other => panic!("expected provider mismatch, got {other:?}"),
        }
    }

    #[test]
    fn sse_metadata_uses_configured_provider_id() {
        let mut parser = SseEventParser::for_provider("deepseek");
        let events = match parser.push_str(
            "data: {\"id\":\"req-1\",\"model\":\"deepseek-chat\",\"choices\":[{\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        ) {
            Ok(events) => events,
            Err(err) => panic!("parse failed: {err}"),
        };
        let provider = events.iter().find_map(|event| match event {
            AssistantStreamEvent::Done { provider, .. } => provider.as_ref(),
            _ => None,
        });
        assert_eq!(
            provider
                .and_then(|metadata| metadata.model.as_ref())
                .map(|model| model.provider.as_str()),
            Some("deepseek")
        );
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
                "reasoning": {"effort": "none", "exclude": true},
                "stream": true,
                "stream_options": {"include_usage": true}
            })
        );
    }

    #[test]
    fn serializes_reasoning_none_when_off() {
        let built = match build_chat_request(&request(vec![Message::user_text("hello")])) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        let json = match serde_json::to_value(built) {
            Ok(value) => value,
            Err(err) => panic!("serialize failed: {err}"),
        };
        assert_eq!(json["reasoning"]["effort"], "none");
        assert_eq!(json["reasoning"]["exclude"], true);
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
        assert!(json["reasoning"].get("exclude").is_none());
    }

    #[test]
    fn deserializes_model_catalog_response() {
        let response = match serde_json::from_value::<OpenRouterModelsResponse>(json!({
            "data": [{
                "id": "openai/gpt-4o-mini",
                "name": "GPT 4o Mini",
                "supported_parameters": ["tools", "reasoning"],
                "context_length": 128000,
                "pricing": {
                    "prompt": "0.00000015",
                    "completion": "0.0000006",
                    "input_cache_read": "0.000000075",
                    "input_cache_write": "0.0000003"
                }
            }]
        })) {
            Ok(value) => value,
            Err(err) => panic!("deserialize failed: {err}"),
        };
        assert_eq!(response.data[0].id, "openai/gpt-4o-mini");
        assert!(response.data[0]
            .supported_parameters
            .contains(&"reasoning".to_string()));
        assert_eq!(response.data[0].context_length, Some(128_000));
        let pricing = response.data[0]
            .pricing
            .as_ref()
            .expect("pricing should parse");
        assert_eq!(pricing.prompt.as_deref(), Some("0.00000015"));
        assert_eq!(pricing.completion.as_deref(), Some("0.0000006"));
        assert_eq!(pricing.input_cache_read.as_deref(), Some("0.000000075"));
        assert_eq!(pricing.input_cache_write.as_deref(), Some("0.0000003"));
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
    fn user_image_content_serializes_as_data_url_parts() {
        let message = Message::User {
            id: Uuid::new_v4(),
            content: vec![
                ContentBlock::Text {
                    text: "describe".into(),
                },
                ContentBlock::Image {
                    media_type: "image/png".into(),
                    data: "aGVsbG8=".into(),
                },
            ],
        };
        let built = match build_chat_request(&request(vec![message])) {
            Ok(value) => value,
            Err(err) => panic!("build failed: {err}"),
        };
        let json = match serde_json::to_value(built) {
            Ok(value) => value,
            Err(err) => panic!("serialize failed: {err}"),
        };
        assert_eq!(json["messages"][1]["content"][0]["type"], "text");
        assert_eq!(json["messages"][1]["content"][0]["text"], "describe");
        assert_eq!(json["messages"][1]["content"][1]["type"], "image_url");
        assert_eq!(
            json["messages"][1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,aGVsbG8="
        );
    }

    #[test]
    fn parses_sse_text_usage_done() {
        let mut parser = SseEventParser::new();
        let input = concat!(
            "data: {\"id\":\"req-1\",\"model\":\"m\",\"choices\":[{\"delta\":{\"content\":\"hel\"},\"finish_reason\":null}]}\n\n",
            "data: {\"choices\":[{\"delta\":{\"content\":\"lo\"},\"finish_reason\":null}],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":2,\"total_tokens\":12,\"prompt_tokens_details\":{\"cached_tokens\":4,\"cache_write_tokens\":3},\"cost\":0.00042}}\n\n",
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
        assert!(events.iter().any(|event| matches!(event, AssistantStreamEvent::Usage { usage } if usage.input_tokens == 10 && usage.output_tokens == 2 && usage.cache_read_tokens == 4 && usage.cache_write_tokens == 3 && usage.cost.as_ref().is_some_and(|cost| cost.amount == 0.00042 && cost.currency == "USD"))));
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
