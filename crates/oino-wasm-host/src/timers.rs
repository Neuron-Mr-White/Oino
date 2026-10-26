//! `timers`: per-instance timer wheel on tokio; firing calls back into the host
//! (`oino_timer` events through `HostServices::deliver_event`).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::task::JoinHandle;

use crate::state::lock;

type Fire = Arc<dyn Fn(&str) + Send + Sync>;

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
            loop {
                tokio::time::sleep(delay).await;
                fire(&timer_id);
                if !repeat {
                    break;
                }
            }
        });
        if let Some(old) = lock(&self.handles).insert(id.to_owned(), handle) {
            old.abort();
        }
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
