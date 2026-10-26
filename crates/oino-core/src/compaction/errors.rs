//! Assistant error classification (pi-ai `utils/overflow.js` and `utils/retry.js`).
//!
//! Pattern tables are copied verbatim from pi 1.4.0 (JS regex source, matched
//! case-insensitively by a small built-in matcher, see [`pattern_matches`]); a golden test compares them with pi's sources.

use std::sync::LazyLock;

use oino_ai::{AssistantMessage, StopReason};

use super::pattern::Pattern;

/// pi `OVERFLOW_PATTERNS` (context overflow error texts per provider).
pub const OVERFLOW_PATTERNS: &[&str] = &[
    r"prompt (?:is )?too long",
    r"prompt exceeds max length",
    r"request_too_large",
    r"input is too long for requested model",
    r"exceeds the context window",
    r"exceeds (?:the )?(?:model'?s )?maximum context length(?: of [\d,]+ tokens?|\s*\([\d,]+\))",
    r"input token count.*exceeds the maximum",
    r"maximum prompt length is \d+",
    r"reduce the length of the messages",
    r"maximum context length is \d+ tokens",
    r"exceeds (?:the )?maximum allowed input length of [\d,]+ tokens?",
    r"input \(\d+ tokens\) is longer than the model'?s context length \(\d+ tokens\)",
    r"exceeds the limit of \d+",
    r"exceeds the available context size",
    r"greater than the context length",
    r"context window exceeds limit",
    r"exceeded model token limit",
    r"too large for model with \d+ maximum context length",
    r"prompt has [\d,]+ tokens?, but the configured context size is [\d,]+ tokens?",
    r"model_context_window_exceeded",
    r"prompt too long; exceeded (?:max )?context length",
    r"range of input length should be",
    r"context[_ ]length[_ ]exceeded",
    r"too many tokens",
    r"token limit exceeded",
    r"^(?:[\w -]+ API error \(413\):|413(?:\s|:|$))",
];

/// pi `CEREBRAS_BODYLESS_OVERFLOW_PATTERN` (only for provider `cerebras`).
pub const CEREBRAS_BODYLESS_OVERFLOW_PATTERN: &str =
    r"^4(?:00|13)\s*(?:status code)?\s*\(no body\)";

/// pi `NON_OVERFLOW_PATTERNS`: errors that are never overflow even if they match above.
pub const NON_OVERFLOW_PATTERNS: &[&str] = &[
    r"^(Throttling error|Service unavailable):",
    r"rate limit",
    r"too many requests",
];

/// pi `NON_RETRYABLE_PROVIDER_LIMIT_ERROR_PATTERN` alternatives (quota/billing exhaustion).
pub const NON_RETRYABLE_PROVIDER_LIMIT_PATTERNS: &[&str] = &[
    "GoUsageLimitError",
    "FreeUsageLimitError",
    "Monthly usage limit reached",
    "available balance",
    "insufficient_quota",
    "out of budget",
    "quota exceeded",
    "billing",
    "subscription_sharing_usage_limit_exceeded",
    "subscription_sharing_workplace_individual_usage_limit_exceeded",
];

/// pi `RETRYABLE_PROVIDER_ERROR_PATTERN` alternatives (transient provider/transport errors).
pub const RETRYABLE_PROVIDER_ERROR_PATTERNS: &[&str] = &[
    "overloaded",
    "server_busy",
    "servers are currently busy",
    "currently experiencing high demand",
    "model is at capacity",
    "rate.?limit",
    "too many requests",
    "(?<![a-z0-9])(?:429|500|502|503|504|520|524)(?![a-z0-9])",
    "service.?(?:is.?)?(?:temporarily.?)?unavailable",
    "server.?error",
    "internal.?error",
    "provider.?returned.?(?:an.?)?error",
    "exceeded request buffer limit while retrying upstream",
    "unable to forward upstream",
    "upstream.?request.?failed",
    "upstream_stream_read_error",
    "stream was interrupted",
    "http/?2 stream failed",
    "network.?error",
    "connection.?error",
    "connection.?refused",
    "connection.?lost",
    "other side closed",
    "fetch failed",
    r"request to \S+ failed, reason:",
    "getaddrinfo",
    "ENOTFOUND",
    "EAI_AGAIN",
    "upstream.?connect",
    "reset before headers",
    "socket hang up",
    "socket connection was closed",
    "timed? out",
    "timeout",
    "terminated",
    "websocket.?closed",
    "websocket.?error",
    "ended without",
    "stream ended before message_stop",
    "stream ended before a terminal response event",
    "http2 request did not get a response",
    "stream disconnected before completion",
    "pending stream has been canceled",
    "retry delay",
    "you can retry your request",
    "try your request again",
    "please retry your request",
    "ResourceExhausted",
    "subscription_sharing_usage_unavailable",
    "subscription_sharing_user_unavailable",
];

struct Tables {
    overflow: Vec<Pattern>,
    cerebras: Vec<Pattern>,
    non_overflow: Vec<Pattern>,
    non_retryable: Vec<Pattern>,
    retryable: Vec<Pattern>,
}

fn compile(sources: &[&str]) -> Vec<Pattern> {
    sources
        .iter()
        .filter_map(|source| {
            Pattern::new(source)
                .inspect_err(
                    |error| tracing::error!(%error, "error-classification pattern skipped"),
                )
                .ok()
        })
        .collect()
}

static TABLES: LazyLock<Tables> = LazyLock::new(|| Tables {
    overflow: compile(OVERFLOW_PATTERNS),
    cerebras: compile(&[CEREBRAS_BODYLESS_OVERFLOW_PATTERN]),
    non_overflow: compile(NON_OVERFLOW_PATTERNS),
    non_retryable: compile(NON_RETRYABLE_PROVIDER_LIMIT_PATTERNS),
    retryable: compile(RETRYABLE_PROVIDER_ERROR_PATTERNS),
});

fn any_match(patterns: &[Pattern], text: &str) -> bool {
    patterns.iter().any(|pattern| pattern.is_match(text))
}

/// Whether `text` matches the JS-syntax `pattern` case-insensitively (`RegExp#test` with the
/// `i` flag), or `None` when the pattern is outside the supported subset.
#[must_use]
pub fn pattern_matches(pattern: &str, text: &str) -> Option<bool> {
    Pattern::new(pattern)
        .ok()
        .map(|pattern| pattern.is_match(text))
}

fn error_text(message: &AssistantMessage) -> Option<&str> {
    (message.stop_reason == StopReason::Error)
        .then_some(message.error_message.as_deref())
        .flatten()
        .filter(|text| !text.is_empty())
}

/// pi `isContextOverflow(message, contextWindow)`. `context_window == 0` disables the
/// silent-overflow (usage above the window) and length-stop (input fills the window) checks.
#[must_use]
pub fn is_context_overflow(message: &AssistantMessage, context_window: u64) -> bool {
    let tables = &*TABLES;
    if let Some(text) = error_text(message)
        && !any_match(&tables.non_overflow, text)
        && (any_match(&tables.overflow, text)
            || (message.provider == "cerebras" && any_match(&tables.cerebras, text)))
    {
        return true;
    }
    let input = message.usage.input + message.usage.cache_read;
    if context_window > 0 && message.stop_reason == StopReason::Stop && input > context_window {
        return true;
    }
    #[allow(
        clippy::cast_precision_loss,
        reason = "pi compares token counts as JS doubles"
    )]
    let fills_window = input as f64 >= context_window as f64 * 0.99;
    context_window > 0
        && message.stop_reason == StopReason::Length
        && message.usage.output == 0
        && fills_window
}

/// pi `isRecoverableLength`: a length stop below the intended output limit (`desired_max_output`
/// is the limit before context clamping).
#[must_use]
pub fn is_recoverable_length(message: &AssistantMessage, desired_max_output: u64) -> bool {
    message.stop_reason == StopReason::Length
        && desired_max_output > 0
        && message.usage.output < desired_max_output
}

/// pi `isRetryableAssistantError`: a transient provider or transport error (callers handle
/// context overflow first).
#[must_use]
pub fn is_retryable_assistant_error(message: &AssistantMessage) -> bool {
    let tables = &*TABLES;
    error_text(message).is_some_and(|text| {
        !any_match(&tables.non_retryable, text) && any_match(&tables.retryable, text)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_table_entry_compiles() {
        let tables = &*TABLES;
        assert_eq!(tables.overflow.len(), OVERFLOW_PATTERNS.len());
        assert_eq!(tables.cerebras.len(), 1);
        assert_eq!(tables.non_overflow.len(), NON_OVERFLOW_PATTERNS.len());
        assert_eq!(
            tables.non_retryable.len(),
            NON_RETRYABLE_PROVIDER_LIMIT_PATTERNS.len()
        );
        assert_eq!(
            tables.retryable.len(),
            RETRYABLE_PROVIDER_ERROR_PATTERNS.len()
        );
    }

    #[test]
    fn recoverable_length() {
        let mut message = AssistantMessage::empty("a", "p", "m", 0);
        message.stop_reason = StopReason::Length;
        message.usage.output = 10;
        assert!(is_recoverable_length(&message, 11));
        assert!(!is_recoverable_length(&message, 10));
        assert!(!is_recoverable_length(&message, 0));
    }
}
