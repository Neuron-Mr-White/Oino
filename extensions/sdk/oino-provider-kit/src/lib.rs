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
pub mod catalog;
pub mod completions;
pub mod constrained;
pub mod emit;
pub mod errors;
pub mod exchange;
#[cfg(feature = "guest")]
pub mod guest;
pub mod json;
pub mod options;
pub mod request;
pub mod responses;
pub mod retry;
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
}

impl Wire {
    /// The pi `api` string.
    #[must_use]
    pub const fn api(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic-messages",
            Self::Completions => "openai-completions",
            Self::Responses => "openai-responses",
        }
    }

    /// Wire for a pi `api` string.
    #[must_use]
    pub fn from_api(api: &str) -> Option<Self> {
        match api {
            "anthropic-messages" => Some(Self::Anthropic),
            "openai-completions" => Some(Self::Completions),
            "openai-responses" => Some(Self::Responses),
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

/// Builds `request` for `wire`.
///
/// # Errors
/// Missing credentials or a tool that cannot be expressed.
pub fn prepare(
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
        Wire::Responses => responses::prepare(request, now).map(|p| Prepared {
            http: p.http,
            exchange: p.exchange,
            strict_fallback: false,
        }),
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
        Wire::Completions | Wire::Responses => payload,
    }
}
