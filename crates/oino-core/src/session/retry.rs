//! Automatic retry of transient provider errors (pi `_prepareRetry`, `_finishCancelledRetry`).

use oino_ai::AssistantMessage;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, lock};
use crate::compaction::retry_delay_ms;
use crate::events::AgentSessionEvent;

impl AgentSession {
    /// pi `_finishCancelledRetry`.
    pub(crate) fn finish_cancelled_retry(&self) {
        let attempt = std::mem::take(&mut lock(&self.inner.book).retry_attempt);
        if attempt == 0 {
            return;
        }
        self.emit(AgentSessionEvent::AutoRetryEnd {
            success: false,
            attempt,
            final_error: Some("Retry cancelled".to_owned()),
        });
    }

    /// pi `_prepareRetry`: schedules a retry with exponential backoff; `false` when the budget
    /// is spent, retries are disabled or the wait was cancelled.
    pub(crate) async fn prepare_retry(&self, message: &AssistantMessage) -> bool {
        let settings = lock(&self.inner.settings).settings().retry_settings();
        if !settings.enabled {
            return false;
        }
        let attempt = {
            let mut book = lock(&self.inner.book);
            book.retry_attempt += 1;
            if u64::from(book.retry_attempt) > settings.max_retries {
                book.retry_attempt -= 1;
                return false;
            }
            book.retry_attempt
        };
        let delay_ms = retry_delay_ms(
            settings.base_delay_ms,
            Some(settings.max_agent_delay_ms),
            attempt,
        );
        self.emit(AgentSessionEvent::AutoRetryStart {
            attempt,
            max_attempts: u32::try_from(settings.max_retries).unwrap_or(u32::MAX),
            delay_ms,
            error_message: message
                .error_message
                .clone()
                .filter(|m| !m.is_empty())
                .unwrap_or_else(|| "Unknown error".to_owned()),
        });
        self.omit_recovery_attempt(message);
        let cancel = CancellationToken::new();
        lock(&self.inner.cancels).retry = Some(cancel.clone());
        let slept = tokio::select! {
            () = cancel.cancelled() => false,
            () = tokio::time::sleep(std::time::Duration::from_millis(delay_ms)) => true,
        };
        lock(&self.inner.cancels).retry = None;
        if !slept {
            self.finish_cancelled_retry();
        }
        slept
    }

    /// Enables or disables automatic retry (settings).
    pub fn set_auto_retry_enabled(&self, enabled: bool) {
        if let Err(error) = self.with_settings(|s| s.set_retry_enabled(enabled)) {
            tracing::warn!("saving retry.enabled failed: {error}");
        }
    }
}
