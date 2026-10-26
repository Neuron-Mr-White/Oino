//! Bounded retry of one assistant call (pi-ai `RetryPolicy`, `retryDelayMs`,
//! `retryAssistantCall`) with observer callbacks (pi `RetryCallbacks`).

use std::future::Future;

use oino_ai::{AssistantMessage, StopReason};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;

use super::errors::is_retryable_assistant_error;

/// Default cap of one retry delay (pi `DEFAULT_MAX_AGENT_RETRY_DELAY_MS`).
pub const DEFAULT_MAX_AGENT_RETRY_DELAY_MS: u64 = 60_000;

/// `Number.MAX_SAFE_INTEGER`.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;

/// pi `RetryPolicy` (`settings.retry`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RetryPolicy {
    /// Retries enabled.
    pub enabled: bool,
    /// Maximum retries (the initial call is not a retry).
    pub max_retries: u32,
    /// Base delay; attempt `n` waits `base · 2^(n−1)`.
    pub base_delay_ms: u64,
    /// Cap of one delay (default [`DEFAULT_MAX_AGENT_RETRY_DELAY_MS`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_agent_delay_ms: Option<u64>,
}

/// pi `retryDelayMs`: `baseDelayMs · 2^max(0, attempt−1)` (saturating at
/// `Number.MAX_SAFE_INTEGER`), capped by `maxAgentDelayMs`.
#[must_use]
pub fn retry_delay_ms(base_delay_ms: u64, max_agent_delay_ms: Option<u64>, attempt: u32) -> u64 {
    let exponent = attempt.saturating_sub(1);
    let delay = 2u64
        .checked_pow(exponent)
        .and_then(|factor| base_delay_ms.checked_mul(factor))
        .filter(|delay| *delay <= MAX_SAFE_INTEGER)
        .unwrap_or(MAX_SAFE_INTEGER);
    delay.min(max_agent_delay_ms.unwrap_or(DEFAULT_MAX_AGENT_RETRY_DELAY_MS))
}

/// pi `RetryCallbacks` for summarisation calls; core maps them to the
/// `summarization_retry_scheduled` / `_attempt_start` / `_finished` session events.
pub trait SummarizationRetryObserver: Send + Sync {
    /// Before the backoff sleep of retry `attempt` (1-based).
    fn retry_scheduled(&self, attempt: u32, max_attempts: u32, delay_ms: u64, error_message: &str);
    /// After the sleep, right before the retried call.
    fn retry_attempt_start(&self);
    /// Once when a loop that scheduled a retry ends.
    fn retry_finished(&self, success: bool, attempt: u32, final_error: Option<&str>);
}

/// pi `retryAssistantCall`: runs `produce` and retries transient errors (per
/// [`is_retryable_assistant_error`]) with exponential backoff. Aborted responses are terminal;
/// cancellation during a backoff returns the last response as `aborted` (without its error
/// message).
pub async fn retry_assistant_call<F, Fut>(
    mut produce: F,
    policy: Option<&RetryPolicy>,
    cancel: &CancellationToken,
    observer: Option<&dyn SummarizationRetryObserver>,
) -> AssistantMessage
where
    F: FnMut() -> Fut,
    Fut: Future<Output = AssistantMessage>,
{
    let max_attempts = policy
        .filter(|policy| policy.enabled)
        .map_or(0, |policy| policy.max_retries);
    let mut attempt = 0;
    let mut last_retry: Option<String> = None;
    loop {
        let mut response = produce().await;
        let finished = |success: bool, final_error: Option<&str>| {
            if let (Some(observer), Some(_)) = (observer, &last_retry) {
                observer.retry_finished(success, attempt, final_error);
            }
        };
        match response.stop_reason {
            StopReason::Aborted => {
                finished(false, None);
                return response;
            }
            StopReason::Error => {}
            _ => {
                finished(true, None);
                return response;
            }
        }
        let Some(policy) =
            policy.filter(|_| attempt < max_attempts && is_retryable_assistant_error(&response))
        else {
            finished(false, response.error_message.as_deref());
            return response;
        };
        attempt += 1;
        let error_message = response
            .error_message
            .clone()
            .filter(|text| !text.is_empty())
            .unwrap_or_else(|| "Unknown error".to_owned());
        let delay_ms = retry_delay_ms(policy.base_delay_ms, policy.max_agent_delay_ms, attempt);
        if let Some(observer) = observer {
            observer.retry_scheduled(attempt, max_attempts, delay_ms, &error_message);
        }
        let cancelled = tokio::select! {
            biased;
            () = cancel.cancelled() => true,
            () = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => false,
        };
        if cancelled {
            if let Some(observer) = observer {
                observer.retry_finished(false, attempt, Some(&error_message));
            }
            response.error_message = None;
            response.stop_reason = StopReason::Aborted;
            return response;
        }
        last_retry = Some(error_message);
        if let Some(observer) = observer {
            observer.retry_attempt_start();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_doubles_and_caps() {
        assert_eq!(retry_delay_ms(2000, None, 0), 2000);
        assert_eq!(retry_delay_ms(2000, None, 1), 2000);
        assert_eq!(retry_delay_ms(2000, None, 3), 8000);
        assert_eq!(retry_delay_ms(2000, None, 10), 60_000);
        assert_eq!(retry_delay_ms(2000, Some(u64::MAX), 80), MAX_SAFE_INTEGER);
    }
}
