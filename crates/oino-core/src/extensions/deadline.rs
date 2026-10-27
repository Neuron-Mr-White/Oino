//! Handler deadlines that pause while the handler waits for the user (GA-30 / GA-42).
//!
//! A handler blocked on an approval, form or select dialog must not time out: unipi waits for
//! the user indefinitely. [`with_deadline`] lets `deadline` expire only while `waiting`
//! reports no open dialog; after the dialog closes the handler gets a fresh `deadline`.

use std::future::Future;
use std::time::Duration;

/// How often an expired deadline re-checks for open dialogs.
const RECHECK: Duration = Duration::from_millis(200);

/// Runs `fut` with `deadline`, paused while `waiting()` is true. `None` when it expired.
pub async fn with_deadline<T>(
    fut: impl Future<Output = T>,
    deadline: Duration,
    waiting: impl Fn() -> bool,
) -> Option<T> {
    let mut fut = std::pin::pin!(fut);
    let mut budget = deadline;
    loop {
        if let Ok(value) = tokio::time::timeout(budget, &mut fut).await {
            return Some(value);
        }
        if !waiting() {
            return None;
        }
        // Wait until the dialog closes, then grant a fresh deadline.
        loop {
            if let Ok(value) = tokio::time::timeout(RECHECK, &mut fut).await {
                return Some(value);
            }
            if !waiting() {
                break;
            }
        }
        budget = deadline;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    #[tokio::test(start_paused = true)]
    async fn expires_without_dialog() {
        let r = with_deadline(std::future::pending::<()>(), Duration::from_secs(1), || {
            false
        })
        .await;
        assert!(r.is_none());
    }

    #[tokio::test(start_paused = true)]
    async fn pauses_while_waiting() {
        let open = Arc::new(AtomicBool::new(true));
        let flag = Arc::clone(&open);
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(90)).await;
            flag.store(false, Ordering::SeqCst);
        });
        let fut = async {
            tokio::time::sleep(Duration::from_secs(95)).await;
            7
        };
        let r = with_deadline(fut, Duration::from_secs(30), || open.load(Ordering::SeqCst)).await;
        assert_eq!(r, Some(7));
        let open = AtomicBool::new(true);
        let never = with_deadline(std::future::pending::<()>(), Duration::from_secs(1), || {
            open.swap(false, Ordering::SeqCst)
        })
        .await;
        assert!(never.is_none());
    }
}
