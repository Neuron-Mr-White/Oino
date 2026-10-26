//! `oino-provider-kit`: the wire logic shared by the Oino provider components, ported from
//! pi-ai 1.4.0 onto `oino-ai` types. Pure and I/O-free, so it builds for `wasm32-wasip2` and
//! is tested natively against recorded pi fixtures.
//!
//! # API map
//!
//! | Module | pi-ai source | Contents |
//! |---|---|---|
//! | [`request`] | `SimpleStreamOptions`, SDK client setup | [`ProviderRequest`], [`Headers`], [`HttpRequest`] |
//! | [`anthropic`] | `api/anthropic-messages.js` | params, betas, cache control, OAuth identity, SSE parser |
//! | [`completions`] | `api/openai-completions.js` | compat detection, thinking formats, cache hints, chunk parser |
//! | [`responses`] | `api/openai-responses*.js` | input items, tool changes, prompt cache, event parser |
//! | [`azure`] | `api/azure-openai-responses.js`, `azure-openai-config.js` | Azure endpoints, deployments |
//! | [`codex`] | `api/openai-codex-responses.js` | `ChatGPT` Codex body, headers, errors |
//! | [`google`] | `api/google-shared.js`, `google-generative-ai.js`, `google-vertex.js` | Gemini contents, thinking, SDK body |
//! | [`bedrock`] | `api/bedrock-converse-stream.js` | Converse body, endpoints, auth, stream events |
//! | [`providers`] | `providers/{github-copilot,cloudflare-*,azure}.js` | per-provider request adaptations |
//! | [`eventstream`], [`sigv4`] | AWS SDK | event stream framing, Signature V4 |
//! | [`oauth`] | `auth/oauth/*.js` | PKCE, form encoding, device-code polling, JWT claims |
//! | [`exchange`] | `stream()` bodies | [`Exchange`]: response bytes → [`oino_ai::ProviderEvent`]s |
//! | [`retry`] | `utils/provider-retry.js` | retryable statuses and delays |
//! | [`catalog`] | `providers/*.js`, `models.generated.js` | bundled catalogs, registrations, `/models` parsing |
//! | [`errors`] | SDK `APIError`, `utils/error-body.js` | error texts |
//! | [`json`], [`sse`], [`options`], [`constrained`], [`util`] | shared helpers | partial JSON, SSE, budgets, strict schemas, cost |
//!
//! A component: parses [`ProviderRequest`], calls a wire's `prepare`, runs the
//! `provider-host.before-request` hook on [`HttpRequest::payload`], sends it, then feeds the
//! response into the [`Exchange`] and forwards the returned events.

#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

pub mod anthropic;
pub mod azure;
pub mod bedrock;
pub mod catalog;
pub mod codex;
pub mod completions;
pub mod constrained;
pub mod emit;
pub mod errors;
pub mod eventstream;
pub mod exchange;
pub mod google;
#[cfg(feature = "guest")]
pub mod guest;
pub mod json;
pub mod oauth;
pub mod options;
pub mod providers;
pub mod request;
pub mod responses;
pub mod retry;
pub mod sigv4;
pub mod sse;
pub mod util;

pub use emit::Emitter;
pub use exchange::{Exchange, Step, StreamError, StreamParser};
pub use request::{Headers, HttpRequest, ProviderRequest};

/// The wire APIs the kit implements.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Wire {
    /// `anthropic-messages`.
    Anthropic,
    /// `openai-completions`.
    Completions,
    /// `openai-responses`.
    Responses,
    /// `azure-openai-responses`.
    AzureResponses,
    /// `openai-codex-responses`.
    CodexResponses,
    /// `google-generative-ai`.
    Google,
    /// `google-vertex`.
    Vertex,
    /// `bedrock-converse-stream`.
    Bedrock,
}

impl Wire {
    /// The pi `api` string.
    #[must_use]
    pub const fn api(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic-messages",
            Self::Completions => "openai-completions",
            Self::Responses => "openai-responses",
            Self::AzureResponses => "azure-openai-responses",
            Self::CodexResponses => "openai-codex-responses",
            Self::Google => "google-generative-ai",
            Self::Vertex => "google-vertex",
            Self::Bedrock => "bedrock-converse-stream",
        }
    }

    /// Wire for a pi `api` string.
    #[must_use]
    pub fn from_api(api: &str) -> Option<Self> {
        match api {
            "anthropic-messages" => Some(Self::Anthropic),
            "openai-completions" => Some(Self::Completions),
            "openai-responses" => Some(Self::Responses),
            "azure-openai-responses" => Some(Self::AzureResponses),
            "openai-codex-responses" => Some(Self::CodexResponses),
            "google-generative-ai" => Some(Self::Google),
            "google-vertex" => Some(Self::Vertex),
            "bedrock-converse-stream" => Some(Self::Bedrock),
            _ => None,
        }
    }
}

/// A prepared request of any wire.
pub struct Prepared {
    /// Outbound request (payload before hooks).
    pub http: HttpRequest,
    /// Streaming state.
    pub exchange: Exchange,
    /// Anthropic: a strict-limit 400 may be retried without preferred strict tools.
    pub strict_fallback: bool,
}

/// Options of [`prepare`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrepareOptions {
    /// Allow requests without a credential (local OpenAI-compatible servers).
    pub allow_missing_key: bool,
    /// Anthropic: drop preferred strict tools (after a strict-limit 400).
    pub disable_preferred_strict: bool,
}

/// Builds `request` for `wire`, after the provider adaptations of [`providers::adapt`].
///
/// # Errors
/// Missing credentials, an unusable provider configuration or a tool that cannot be expressed.
pub fn prepare(
    wire: Wire,
    request: &ProviderRequest,
    options: PrepareOptions,
    now: i64,
) -> Result<Prepared, String> {
    let mut adapted = request.clone();
    providers::adapt(&mut adapted)?;
    let mut prepared = prepare_wire(wire, &adapted, options, now)?;
    providers::finish(&adapted, &mut prepared.http.payload);
    Ok(prepared)
}

fn simple(http: HttpRequest, exchange: Exchange) -> Prepared {
    Prepared {
        http,
        exchange,
        strict_fallback: false,
    }
}

fn prepare_wire(
    wire: Wire,
    request: &ProviderRequest,
    options: PrepareOptions,
    now: i64,
) -> Result<Prepared, String> {
    match wire {
        Wire::Anthropic => {
            anthropic::prepare(request, options.disable_preferred_strict, now).map(|p| Prepared {
                http: p.http,
                exchange: p.exchange,
                strict_fallback: p.has_preferred_strict_tools,
            })
        }
        Wire::Completions => {
            completions::prepare(request, options.allow_missing_key, now).map(|p| Prepared {
                http: p.http,
                exchange: p.exchange,
                strict_fallback: false,
            })
        }
        Wire::Responses => responses::prepare(request, now).map(|p| simple(p.http, p.exchange)),
        Wire::AzureResponses => azure::prepare(request, now).map(|p| simple(p.http, p.exchange)),
        Wire::CodexResponses => codex::prepare(request, now).map(|p| simple(p.http, p.exchange)),
        Wire::Google => google::prepare(request, google::GoogleApi::Generative, now)
            .map(|p| simple(p.http, p.exchange)),
        Wire::Vertex => google::prepare(request, google::GoogleApi::Vertex, now)
            .map(|p| simple(p.http, p.exchange)),
        Wire::Bedrock => bedrock::prepare(request, now).map(|p| simple(p.http, p.exchange)),
    }
}

/// The wire body for a (possibly hook-replaced) payload.
#[must_use]
pub fn finalize(
    wire: Wire,
    payload: serde_json::Value,
    headers: &mut Headers,
) -> serde_json::Value {
    match wire {
        Wire::Anthropic => anthropic::finalize(payload, headers),
        Wire::Google | Wire::Vertex => google::finalize(payload),
        Wire::Bedrock => bedrock::finalize(payload),
        Wire::Completions | Wire::Responses | Wire::AzureResponses | Wire::CodexResponses => {
            payload
        }
    }
}
