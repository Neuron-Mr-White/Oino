//! One request/response exchange, independent of I/O: response bytes in, [`ProviderEvent`]s
//! out, with pi's stream rules (start first; failures end with `error`; usage and stop reason
//! in the final message).

use oino_ai::{AssistantContent, ProviderEvent};
use serde_json::Value;

use crate::emit::Emitter;
use crate::errors::{
    SdkError, anthropic_status_error, connection_error_message, format_provider_error,
    openai_status_error,
};
use crate::eventstream::EventStreamDecoder;
use crate::sse::{SseDecoder, SseEvent};

/// What a parser did with one SSE event.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A provider event was processed; the parsed JSON is reported to `stream-event` hooks.
    Event(Value),
    /// Ignored (keep-alive, unknown event).
    Skip,
    /// The stream is complete (`[DONE]`); stop reading.
    End,
}

/// A failure inside the stream (pi `throw` inside the stream loop).
#[derive(Clone, Debug, PartialEq)]
pub enum StreamError {
    /// Plain `Error(message)`.
    Message(String),
    /// SDK `APIError` from an in-stream error payload.
    Sdk(SdkError),
}

impl From<String> for StreamError {
    fn from(message: String) -> Self {
        Self::Message(message)
    }
}

impl From<&str> for StreamError {
    fn from(message: &str) -> Self {
        Self::Message(message.to_owned())
    }
}

/// Wire-specific streaming parser.
pub trait StreamParser {
    /// Handles one SSE event.
    ///
    /// # Errors
    /// A stream failure; the exchange ends with `error`.
    fn event(&mut self, sse: &SseEvent, out: &mut Emitter) -> Result<Step, StreamError>;

    /// End of the response body: finalise blocks and validate the stop reason.
    ///
    /// # Errors
    /// A stream failure; the exchange ends with `error`.
    fn finish(&mut self, out: &mut Emitter) -> Result<(), StreamError>;

    /// pi catch-block cleanup before the error event (finalise scratch state).
    fn cleanup(&mut self, _out: &mut Emitter) {}

    /// The wire emits `start` itself (Bedrock: on `messageStart`) instead of on a 2xx status.
    fn emits_start(&self) -> bool {
        false
    }

    /// Response headers of the streaming response (request ids for diagnostics).
    fn response_headers(&mut self, _headers: &[(String, String)]) {}

    /// Diagnostics pi appends to a failed message (Bedrock `bedrock_response_failure`).
    fn failure_diagnostics(&mut self, _error: &StreamFailure<'_>, _out: &mut Emitter) {}
}

/// What failed, for [`StreamParser::failure_diagnostics`].
#[derive(Clone, Copy, Debug)]
pub enum StreamFailure<'a> {
    /// Non-2xx response.
    Status {
        /// HTTP status.
        status: u16,
        /// Response headers.
        headers: &'a [(String, String)],
        /// Response body.
        body: &'a str,
    },
    /// A failure inside the stream.
    Stream(&'a StreamError),
    /// Transport failure or local error.
    Other,
}

/// Response body framing.
#[derive(Debug)]
pub enum Framing {
    /// Server-sent events.
    Sse(SseDecoder),
    /// AWS event stream (Bedrock).
    EventStream(EventStreamDecoder),
}

impl Framing {
    fn feed(&mut self, bytes: &[u8]) -> Vec<SseEvent> {
        match self {
            Self::Sse(d) => d.feed(bytes),
            Self::EventStream(d) => d.feed(bytes),
        }
    }

    fn finish(&mut self) -> Vec<SseEvent> {
        match self {
            Self::Sse(d) => d.finish(),
            Self::EventStream(d) => d.finish(),
        }
    }
}

/// How errors are rendered for a wire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ErrorStyle {
    /// Anthropic: SDK message verbatim.
    Anthropic,
    /// `OpenAI` chat completions: `formatProviderError` without prefix, plus `OpenRouter`
    /// `metadata.raw`.
    Completions,
    /// `OpenAI` Responses: `formatProviderError` with `"<provider> API error"`.
    Responses {
        /// `OpenAI` for provider `openai`, else the provider id.
        prefix: String,
    },
    /// `OpenAI` SDK errors with a fixed `formatProviderError` prefix (Azure `OpenAI`).
    Prefixed {
        /// Prefix, e.g. `Azure OpenAI API error`.
        prefix: String,
    },
    /// `@google/genai` `ApiError`: the message is the JSON error body.
    Google,
    /// pi Codex `parseErrorResponse` (plain `Error`, `ChatGPT` usage-limit text).
    Codex {
        /// Wall clock for `resets_at` (ms).
        now: i64,
    },
    /// AWS SDK service exceptions with pi's Bedrock prefixes.
    Bedrock,
}

const CHATGPT_USAGE_URL: &str = "https://chatgpt.com/settings/usage";
const CHATGPT_WORKSPACE_LIMIT_HINT: &str = "You've reached your ChatGPT workspace's usage limit. Ask your workspace admin to increase it at admin.openai.com → Workspace → Usage limits.";

impl ErrorStyle {
    /// SDK error for a non-2xx response.
    #[must_use]
    pub fn status_error(&self, status: u16, body: &str) -> SdkError {
        self.status_error_with(status, &[], body)
    }

    /// [`Self::status_error`] with the response headers (Bedrock reads `x-amzn-errortype`).
    #[must_use]
    pub fn status_error_with(
        &self,
        status: u16,
        headers: &[(String, String)],
        body: &str,
    ) -> SdkError {
        match self {
            Self::Anthropic => anthropic_status_error(status, body),
            Self::Completions | Self::Responses { .. } | Self::Prefixed { .. } => {
                openai_status_error(status, body)
            }
            Self::Google => crate::google::status_error(status, headers, body),
            Self::Codex { now } => crate::codex::status_error(status, body, *now),
            Self::Bedrock => crate::bedrock::status_error(status, headers, body),
        }
    }

    /// Final `errorMessage` text for an SDK error.
    #[must_use]
    pub fn render(&self, error: &SdkError) -> String {
        match self {
            Self::Anthropic | Self::Google | Self::Codex { .. } | Self::Bedrock => {
                error.message.clone()
            }
            Self::Prefixed { prefix } => format_provider_error(error, Some(prefix)),
            Self::Completions => {
                let mut text = format_provider_error(error, None);
                let raw = error
                    .error
                    .as_ref()
                    .and_then(|e| e.pointer("/metadata/raw"))
                    .filter(|raw| !raw.is_null());
                if let Some(raw) = raw {
                    let raw = raw.as_str().map_or_else(|| raw.to_string(), str::to_owned);
                    if !raw.is_empty() && !text.contains(&raw) {
                        text.push('\n');
                        text.push_str(&raw);
                    }
                }
                text
            }
            Self::Responses { prefix } => {
                let text = format_provider_error(error, Some(&format!("{prefix} API error")));
                if text.contains("subscription_sharing_usage_limit_exceeded") {
                    format!("{text}\nCheck your ChatGPT usage: {CHATGPT_USAGE_URL}")
                } else if text
                    .contains("subscription_sharing_workplace_individual_usage_limit_exceeded")
                {
                    format!("{text}\n{CHATGPT_WORKSPACE_LIMIT_HINT}")
                } else {
                    text
                }
            }
        }
    }

    /// Message for a plain thrown error (`Error(message)`): Responses/completions pass it
    /// through `formatProviderError`, which leaves it unchanged without a status.
    #[must_use]
    pub fn render_message(&self, message: &str) -> String {
        message.to_owned()
    }
}

/// A streaming exchange.
pub struct Exchange {
    out: Emitter,
    parser: Box<dyn StreamParser>,
    decoder: Framing,
    style: ErrorStyle,
    raw: Vec<Value>,
    finished: bool,
}

impl Exchange {
    /// New exchange over `out` (the initial output message) and `parser`.
    #[must_use]
    pub fn new(out: Emitter, parser: Box<dyn StreamParser>, style: ErrorStyle) -> Self {
        Self {
            out,
            parser,
            decoder: Framing::Sse(SseDecoder::new()),
            style,
            raw: Vec::new(),
            finished: false,
        }
    }

    /// Uses `framing` for the response body instead of SSE.
    #[must_use]
    pub fn with_framing(mut self, framing: Framing) -> Self {
        self.decoder = framing;
        self
    }

    /// Error style of the wire.
    #[must_use]
    pub const fn style(&self) -> &ErrorStyle {
        &self.style
    }

    /// Mutable output (e.g. for diagnostics added before the response).
    pub fn output(&mut self) -> &mut Emitter {
        &mut self.out
    }

    /// Whether a terminal event was produced.
    #[must_use]
    pub const fn is_finished(&self) -> bool {
        self.finished
    }

    /// Raw provider events parsed since the last call (for `stream-event` hooks).
    pub fn take_raw(&mut self) -> Vec<Value> {
        std::mem::take(&mut self.raw)
    }

    /// The request failed before streaming: non-2xx status with `body`.
    pub fn http_error(&mut self, status: u16, body: &[u8]) -> Vec<ProviderEvent> {
        self.http_error_with(status, &[], body)
    }

    /// [`Self::http_error`] with the response headers.
    pub fn http_error_with(
        &mut self,
        status: u16,
        headers: &[(String, String)],
        body: &[u8],
    ) -> Vec<ProviderEvent> {
        let body = String::from_utf8_lossy(body);
        let error = self.style.status_error_with(status, headers, &body);
        let text = self.style.render(&error);
        self.parser.failure_diagnostics(
            &StreamFailure::Status {
                status,
                headers,
                body: &body,
            },
            &mut self.out,
        );
        self.fail(text, false)
    }

    /// The request failed in transport.
    pub fn connection_error(&mut self, detail: &str) -> Vec<ProviderEvent> {
        self.fail(connection_error_message(detail), false)
    }

    /// The caller aborted.
    pub fn abort(&mut self) -> Vec<ProviderEvent> {
        self.fail("Request was aborted".to_owned(), true)
    }

    /// Fails with an already rendered message.
    pub fn fail(&mut self, message: String, aborted: bool) -> Vec<ProviderEvent> {
        if !self.finished {
            self.parser.cleanup(&mut self.out);
            self.out.fail(message, aborted);
            self.finished = true;
        }
        self.out.drain()
    }

    /// Response headers arrived with a 2xx status: emits `start` (unless the wire emits it).
    pub fn start(&mut self) -> Vec<ProviderEvent> {
        if !self.parser.emits_start() {
            self.out.start();
        }
        self.out.drain()
    }

    /// [`Self::start`] with the response headers.
    pub fn start_with(&mut self, headers: &[(String, String)]) -> Vec<ProviderEvent> {
        self.parser.response_headers(headers);
        self.start()
    }

    /// Feeds response body bytes.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ProviderEvent> {
        if self.finished {
            return Vec::new();
        }
        let events = self.decoder.feed(bytes);
        self.handle(&events)
    }

    /// End of body.
    pub fn finish(&mut self) -> Vec<ProviderEvent> {
        if self.finished {
            return self.out.drain();
        }
        let events = self.decoder.finish();
        let mut out = self.handle(&events);
        if !self.finished {
            match self.parser.finish(&mut self.out) {
                Ok(()) => {
                    self.out.done();
                    self.finished = true;
                    out.extend(self.out.drain());
                }
                Err(error) => out.extend(self.stream_failure(&error)),
            }
        }
        out
    }

    fn handle(&mut self, events: &[SseEvent]) -> Vec<ProviderEvent> {
        for sse in events {
            if self.finished {
                break;
            }
            match self.parser.event(sse, &mut self.out) {
                Ok(Step::Event(raw)) => self.raw.push(raw),
                Ok(Step::Skip) => {}
                Ok(Step::End) => {
                    let mut out = self.out.drain();
                    out.extend(self.finish_now());
                    return out;
                }
                Err(error) => {
                    let mut out = self.out.drain();
                    out.extend(self.stream_failure(&error));
                    return out;
                }
            }
        }
        self.out.drain()
    }

    fn finish_now(&mut self) -> Vec<ProviderEvent> {
        match self.parser.finish(&mut self.out) {
            Ok(()) => {
                self.out.done();
                self.finished = true;
                self.out.drain()
            }
            Err(error) => self.stream_failure(&error),
        }
    }

    fn stream_failure(&mut self, error: &StreamError) -> Vec<ProviderEvent> {
        let text = match error {
            StreamError::Message(message) => self.style.render_message(message),
            StreamError::Sdk(error) => self.style.render(error),
        };
        self.parser
            .failure_diagnostics(&StreamFailure::Stream(error), &mut self.out);
        let mut out = self.out.drain();
        out.extend(self.fail(text, false));
        out
    }
}

/// Whether the message contains a tool call.
#[must_use]
pub fn has_tool_call(out: &Emitter) -> bool {
    out.message
        .content
        .iter()
        .any(|block| matches!(block, AssistantContent::ToolCall(_)))
}
