//! Ordered client event fan-out with bounded, coalescing subscriber queues
//! (architecture.md §4.2).
//!
//! Publishing never blocks the agent loop. When a subscriber's queue is full, consecutive
//! `message_update` / `tool_execution_update` events of the same id are coalesced (latest
//! cumulative state wins) and `bash_execution_update` deltas are concatenated. Other events
//! are never dropped; a subscriber that stays over capacity for 5 s is disconnected with a
//! final `lagged` event.

use std::collections::VecDeque;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, Weak};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use futures::Stream;

use crate::events::{AgentSessionEvent, ClientEvent, OinoEvent};

/// Per-subscriber capacity before coalescing.
pub const SUBSCRIBER_CAPACITY: usize = 1024;
/// How long a subscriber may stay over capacity.
pub const LAG_LIMIT: Duration = Duration::from_secs(5);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

#[derive(Default)]
struct Queue {
    events: VecDeque<ClientEvent>,
    waker: Option<Waker>,
    over_since: Option<Instant>,
    dropped: u64,
    closed: bool,
}

impl Queue {
    fn push(&mut self, event: ClientEvent, capacity: usize) {
        if self.closed {
            return;
        }
        if self.events.len() >= capacity {
            if self.coalesce(&event) {
                self.dropped += 1;
                self.wake();
                return;
            }
            let since = *self.over_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= LAG_LIMIT {
                self.events.push_back(ClientEvent::Oino(OinoEvent::Lagged {
                    dropped: self.dropped,
                }));
                self.closed = true;
                self.wake();
                return;
            }
        } else {
            self.over_since = None;
        }
        self.events.push_back(event);
        self.wake();
    }

    /// Merges `event` into the last queued event when both update the same item.
    fn coalesce(&mut self, event: &ClientEvent) -> bool {
        let Some(last) = self.events.back_mut() else {
            return false;
        };
        let replace = match (&*last, event) {
            (
                ClientEvent::Session(AgentSessionEvent::MessageUpdate { .. }),
                ClientEvent::Session(AgentSessionEvent::MessageUpdate { .. }),
            ) => true,
            (
                ClientEvent::Session(AgentSessionEvent::ToolExecutionUpdate {
                    tool_call_id: a,
                    ..
                }),
                ClientEvent::Session(AgentSessionEvent::ToolExecutionUpdate {
                    tool_call_id: b,
                    ..
                }),
            ) => a == b,
            (
                ClientEvent::Oino(OinoEvent::AgentEvent {
                    agent_id: a,
                    event: first,
                }),
                ClientEvent::Oino(OinoEvent::AgentEvent {
                    agent_id: b,
                    event: second,
                }),
            ) => {
                a == b
                    && matches!(
                        (&**first, &**second),
                        (
                            ClientEvent::Session(AgentSessionEvent::MessageUpdate { .. }),
                            ClientEvent::Session(AgentSessionEvent::MessageUpdate { .. })
                        )
                    )
            }
            _ => false,
        };
        if replace {
            *last = event.clone();
            return true;
        }
        if let (
            ClientEvent::Session(AgentSessionEvent::BashExecutionUpdate {
                id: a,
                delta: existing,
            }),
            ClientEvent::Session(AgentSessionEvent::BashExecutionUpdate { id: b, delta }),
        ) = (last, event)
            && a == b
        {
            existing.push_str(delta);
            return true;
        }
        false
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

/// Fan-out point. Cheap to clone.
#[derive(Clone, Default)]
pub struct EventHub {
    subscribers: Arc<Mutex<Vec<Weak<Mutex<Queue>>>>>,
    capacity: Option<usize>,
}

impl std::fmt::Debug for EventHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventHub")
            .field("subscribers", &lock(&self.subscribers).len())
            .finish_non_exhaustive()
    }
}

impl EventHub {
    /// A hub with [`SUBSCRIBER_CAPACITY`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A hub with a custom per-subscriber capacity (tests).
    #[must_use]
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            subscribers: Arc::default(),
            capacity: Some(capacity.max(1)),
        }
    }

    /// A new subscription receiving every event published from now on.
    #[must_use]
    pub fn subscribe(&self) -> EventStream {
        let queue = Arc::new(Mutex::new(Queue::default()));
        lock(&self.subscribers).push(Arc::downgrade(&queue));
        EventStream { queue }
    }

    /// Publishes one event to every live subscriber, in order.
    pub fn publish(&self, event: impl Into<ClientEvent>) {
        let event = event.into();
        let capacity = self.capacity.unwrap_or(SUBSCRIBER_CAPACITY);
        let mut subscribers = lock(&self.subscribers);
        subscribers.retain(|weak| {
            let Some(queue) = weak.upgrade() else {
                return false;
            };
            let mut queue = lock(&queue);
            queue.push(event.clone(), capacity);
            !queue.closed
        });
    }

    /// Closes every subscription (streams end after draining).
    pub fn close(&self) {
        for weak in lock(&self.subscribers).drain(..) {
            if let Some(queue) = weak.upgrade() {
                let mut queue = lock(&queue);
                queue.closed = true;
                queue.wake();
            }
        }
    }

    /// Number of live subscribers.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        lock(&self.subscribers)
            .iter()
            .filter(|w| w.strong_count() > 0)
            .count()
    }
}

/// One subscription; ends when the hub closes or the subscriber lagged out.
pub struct EventStream {
    queue: Arc<Mutex<Queue>>,
}

impl std::fmt::Debug for EventStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EventStream")
            .field("queued", &lock(&self.queue).events.len())
            .finish()
    }
}

impl EventStream {
    /// Takes every queued event without waiting.
    #[must_use]
    pub fn drain_now(&self) -> Vec<ClientEvent> {
        lock(&self.queue).events.drain(..).collect()
    }
}

impl Stream for EventStream {
    type Item = ClientEvent;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<ClientEvent>> {
        let mut queue = lock(&self.queue);
        if let Some(event) = queue.events.pop_front() {
            return Poll::Ready(Some(event));
        }
        if queue.closed {
            return Poll::Ready(None);
        }
        queue.waker = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    fn bash(delta: &str) -> ClientEvent {
        ClientEvent::Session(AgentSessionEvent::BashExecutionUpdate {
            id: None,
            delta: delta.into(),
        })
    }

    #[tokio::test]
    async fn delivers_in_order_and_coalesces_when_full() {
        let hub = EventHub::with_capacity(2);
        let mut stream = hub.subscribe();
        hub.publish(AgentSessionEvent::AgentStart);
        hub.publish(bash("a"));
        hub.publish(bash("b"));
        hub.publish(bash("c"));
        hub.close();
        let events: Vec<_> = (&mut stream).collect().await;
        assert_eq!(
            events,
            vec![
                ClientEvent::Session(AgentSessionEvent::AgentStart),
                bash("abc")
            ]
        );
    }

    #[test]
    fn dropped_subscriptions_are_pruned() {
        let hub = EventHub::new();
        let stream = hub.subscribe();
        assert_eq!(hub.subscriber_count(), 1);
        drop(stream);
        hub.publish(AgentSessionEvent::AgentStart);
        assert_eq!(hub.subscriber_count(), 0);
    }
}
