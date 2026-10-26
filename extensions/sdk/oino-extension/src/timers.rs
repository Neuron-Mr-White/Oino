//! Timers: `oino_timer {timerId}` events (subscribe to `EventKind::OinoTimer`), sleep, clock.

use std::time::Duration;

use crate::bindings::oino::extension::timers as wt;

/// Sets (or replaces) timer `id`.
pub fn set(id: &str, delay: Duration, repeat: bool) {
    wt::set(
        id,
        u64::try_from(delay.as_millis()).unwrap_or(u64::MAX),
        repeat,
    );
}
/// Cancels timer `id`.
pub fn cancel(id: &str) {
    wt::cancel(id);
}
/// Sleeps without blocking other calls of this extension.
pub async fn sleep(delay: Duration) {
    wt::sleep(u64::try_from(delay.as_millis()).unwrap_or(u64::MAX)).await;
}
/// Wall clock (epoch ms).
#[must_use]
pub fn now_ms() -> i64 {
    wt::now_ms()
}
