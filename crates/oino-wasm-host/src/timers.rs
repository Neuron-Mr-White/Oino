//! `timers`: per-instance timer wheel on tokio; firing calls back into the host
//! (`oino_timer` events through `HostServices::deliver_event`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::state::lock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    Delivered,
    TimedOut,
    Failed,
}

type Fire = Arc<dyn Fn(String) -> futures::future::BoxFuture<'static, Outcome> + Send + Sync>;

#[derive(Default)]
struct Backoff {
    timeouts: u32,
    warned: bool,
}

impl Backoff {
    fn record(&mut self, outcome: Outcome, delay: Duration) -> (Duration, bool) {
        if outcome != Outcome::TimedOut {
            self.timeouts = 0;
            return (delay, false);
        }
        self.timeouts = self.timeouts.saturating_add(1);
        if self.timeouts < 3 {
            return (delay, false);
        }
        let warn = !self.warned;
        self.warned = true;
        let multiplier = 1_u32 << (self.timeouts - 2).min(30);
        (
            delay
                .saturating_mul(multiplier)
                .min(Duration::from_secs(600)),
            warn,
        )
    }
}

/// Timers of one extension; same id replaces, dropped timers are aborted.
#[derive(Default)]
pub(crate) struct Timers {
    handles: Mutex<HashMap<String, JoinHandle<()>>>,
}

impl Timers {
    /// Schedules `fire(id)` after `delay` (repeating when `repeat`).
    pub(crate) fn set(&self, id: &str, delay: Duration, repeat: bool, fire: Fire) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            tracing::warn!(timer = id, "timers.set outside a tokio runtime; ignored");
            return;
        };
        let timer_id = id.to_owned();
        let delay = delay.max(Duration::from_millis(1));
        let handle = runtime.spawn(async move {
            let mut backoff = Backoff::default();
            let mut next = delay;
            loop {
                tokio::time::sleep(next).await;
                let outcome = fire(timer_id.clone()).await;
                let (wait, warn) = backoff.record(outcome, delay);
                next = wait;
                if warn {
                    tracing::warn!(timer = %timer_id, "timer handler repeatedly timed out; applying exponential backoff (cap 10 minutes)");
                }
                if !repeat {
                    break;
                }
            }
        });
        if let Some(old) = lock(&self.handles).insert(id.to_owned(), handle) {
            old.abort();
        }
    }

    /// Completed one-shot timers do not pin an idle services set.
    pub(crate) fn is_active(&self) -> bool {
        let mut handles = lock(&self.handles);
        handles.retain(|_, handle| !handle.is_finished());
        !handles.is_empty()
    }

    /// Cancels `id`.
    pub(crate) fn cancel(&self, id: &str) {
        if let Some(handle) = lock(&self.handles).remove(id) {
            handle.abort();
        }
    }

    /// Cancels everything.
    pub(crate) fn cancel_all(&self) {
        for (_, handle) in lock(&self.handles).drain() {
            handle.abort();
        }
    }
}

impl Drop for Timers {
    fn drop(&mut self) {
        self.cancel_all();
    }
}

/// Wall clock in epoch milliseconds.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn timeout_backoff_is_capped_resets_and_warns_once() {
        let mut backoff = Backoff::default();
        let delay = Duration::from_secs(5);
        assert_eq!(backoff.record(Outcome::TimedOut, delay), (delay, false));
        assert_eq!(backoff.record(Outcome::TimedOut, delay), (delay, false));
        assert_eq!(
            backoff.record(Outcome::TimedOut, delay),
            (Duration::from_secs(10), true)
        );
        assert_eq!(
            backoff.record(Outcome::TimedOut, delay),
            (Duration::from_secs(20), false)
        );
        for _ in 0..100 {
            assert!(!backoff.record(Outcome::TimedOut, delay).1);
        }
        assert_eq!(
            backoff.record(Outcome::TimedOut, delay).0,
            Duration::from_secs(600)
        );
        assert_eq!(backoff.record(Outcome::Delivered, delay), (delay, false));
        for _ in 0..4 {
            assert!(!backoff.record(Outcome::TimedOut, delay).1);
        }
    }

    #[tokio::test]
    async fn only_live_timers_pin_services() {
        let timers = Timers::default();
        assert!(!timers.is_active());
        let fire: Fire = Arc::new(|_| Box::pin(async { Outcome::Delivered }));
        timers.set("one-shot", Duration::from_millis(1), false, fire.clone());
        assert!(timers.is_active());
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert!(!timers.is_active());
        timers.set("repeating", Duration::from_secs(60), true, fire);
        assert!(timers.is_active());
        timers.cancel_all();
        assert!(!timers.is_active());
    }

    #[tokio::test]
    async fn fires_replaces_and_cancels() {
        let timers = Timers::default();
        let count = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&count);
        timers.set(
            "a",
            Duration::from_millis(5),
            true,
            Arc::new(move |_| {
                c.fetch_add(1, Ordering::SeqCst);
                Box::pin(async { Outcome::Delivered })
            }),
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
        timers.cancel("a");
        let seen = count.load(Ordering::SeqCst);
        assert!(seen >= 2, "{seen}");
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(count.load(Ordering::SeqCst), seen);
    }
}
