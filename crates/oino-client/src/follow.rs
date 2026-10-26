//! Event streams that follow the runtime's current session (pi `rebindSession`).
//!
//! Each stream subscribes to the *session* hub, not the runtime hub: session events are
//! queued synchronously when published, so once a command returns every event it caused is
//! already in the stream (a frontend can drain it before writing the reply, pi ordering).
//! When the backend replaces the session it hands every live stream a subscription to the
//! new session plus the `session_replaced` event; a stream switches after draining the old
//! session.

use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Waker};

use futures::Stream;
use oino_core::events::ClientEvent;
use oino_core::hub::EventStream;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Slot {
    /// Subscription to the replacement session and the event announcing it.
    next: Option<(EventStream, ClientEvent)>,
    /// The backend shut down: end after draining.
    closed: bool,
    waker: Option<Waker>,
}

/// Live streams of one backend.
#[derive(Default)]
pub(crate) struct Followers {
    slots: Mutex<Vec<Weak<Mutex<Slot>>>>,
}

impl Followers {
    /// A stream starting with `current`.
    pub(crate) fn stream(&self, current: EventStream) -> FollowStream {
        let slot = Arc::new(Mutex::new(Slot::default()));
        lock(&self.slots).push(Arc::downgrade(&slot));
        FollowStream {
            current,
            slot,
            announce: None,
        }
    }

    /// Ends every live stream once it is drained.
    pub(crate) fn close(&self) {
        for weak in lock(&self.slots).drain(..) {
            if let Some(slot) = weak.upgrade() {
                let mut slot = lock(&slot);
                slot.closed = true;
                if let Some(waker) = slot.waker.take() {
                    waker.wake();
                }
            }
        }
    }

    /// Moves every live stream to the next session; `subscribe` is called once per stream.
    pub(crate) fn rebind(&self, subscribe: impl Fn() -> EventStream, announce: &ClientEvent) {
        lock(&self.slots).retain(|weak| {
            let Some(slot) = weak.upgrade() else {
                return false;
            };
            let mut slot = lock(&slot);
            slot.next = Some((subscribe(), announce.clone()));
            if let Some(waker) = slot.waker.take() {
                waker.wake();
            }
            true
        });
    }
}

/// A [`Stream`] of the current session's client events.
pub struct FollowStream {
    current: EventStream,
    slot: Arc<Mutex<Slot>>,
    announce: Option<ClientEvent>,
}

impl std::fmt::Debug for FollowStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FollowStream").finish_non_exhaustive()
    }
}

impl Stream for FollowStream {
    type Item = ClientEvent;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ClientEvent>> {
        loop {
            if let Some(event) = self.announce.take() {
                return Poll::Ready(Some(event));
            }
            if let Poll::Ready(Some(event)) = Pin::new(&mut self.current).poll_next(cx) {
                return Poll::Ready(Some(event));
            }
            // The old session is drained (or closed): switch if a replacement is waiting.
            let next = {
                let mut slot = lock(&self.slot);
                let next = slot.next.take();
                if next.is_none() && slot.closed {
                    return Poll::Ready(None);
                }
                if next.is_none() {
                    slot.waker = Some(cx.waker().clone());
                }
                next
            };
            match next {
                Some((stream, announce)) => {
                    self.current = stream;
                    self.announce = Some(announce);
                }
                None => return Poll::Pending,
            }
        }
    }
}
