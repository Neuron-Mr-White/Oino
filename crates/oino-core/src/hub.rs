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
/// perf-fix #3: once a subscriber has this many events queued (it lags), consecutive text /
/// thinking deltas of the same block merge into the queued one (deltas appended, the newer
/// cumulative message wins), so a slow reader holds a handful of message snapshots instead
/// of up to [`SUBSCRIBER_CAPACITY`] full copies of a growing message (583 MB of a 646 MB
/// heap peak in the 50-subagent bench).
pub const MERGE_BACKLOG: usize = 16;

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
        let event = if self.events.len() >= MERGE_BACKLOG.min(capacity) {
            match self.merge_delta(event) {
                Ok(()) => {
                    self.wake();
                    return;
                }
                Err(event) => event,
            }
        } else {
            event
        };
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

    /// Appends `event`'s text/thinking delta to the last queued event when both are deltas
    /// of the same block of the same (agent's) message; gives `event` back otherwise.
    fn merge_delta(&mut self, event: ClientEvent) -> Result<(), ClientEvent> {
        let Some(last) = self.events.back_mut() else {
            return Err(event);
        };
        let Some((last_agent, last_message, last_update)) = update_parts(last) else {
            return Err(event);
        };
        let (agent, message, update) = match into_update_parts(event) {
            Ok(parts) => parts,
            Err(event) => return Err(event),
        };
        if last_agent != agent.as_deref() {
            return Err(rebuild_update(agent, message, update));
        }
        use oino_ai::AssistantMessageEvent as E;
        match (last_update, update) {
            (
                E::TextDelta {
                    content_index: a,
                    delta: existing,
                    partial: old,
                },
                E::TextDelta {
                    content_index: b,
                    delta,
                    partial,
                },
            )
            | (
                E::ThinkingDelta {
                    content_index: a,
                    delta: existing,
                    partial: old,
                },
                E::ThinkingDelta {
                    content_index: b,
                    delta,
                    partial,
                },
            ) if *a == b => {
                existing.push_str(&delta);
                *old = partial;
                *last_message = message;
                self.dropped += 1;
                Ok(())
            }
            (_, update) => Err(rebuild_update(agent, message, update)),
        }
    }

    fn wake(&mut self) {
        if let Some(waker) = self.waker.take() {
            waker.wake();
        }
    }
}

type UpdateParts<'a> = (
    Option<&'a str>,
    &'a mut oino_ai::AgentMessage,
    &'a mut oino_ai::AssistantMessageEvent,
);

/// The parts of a (possibly agent-wrapped) `message_update`, borrowed.
fn update_parts(event: &mut ClientEvent) -> Option<UpdateParts<'_>> {
    match event {
        ClientEvent::Session(AgentSessionEvent::MessageUpdate {
            message,
            assistant_message_event,
        }) => Some((None, message, assistant_message_event)),
        ClientEvent::Oino(OinoEvent::AgentEvent { agent_id, event }) => match &mut **event {
            ClientEvent::Session(AgentSessionEvent::MessageUpdate {
                message,
                assistant_message_event,
            }) => Some((Some(agent_id.as_str()), message, assistant_message_event)),
            _ => None,
        },
        _ => None,
    }
}

type OwnedUpdate = (
    Option<String>,
    oino_ai::AgentMessage,
    oino_ai::AssistantMessageEvent,
);

/// The parts of a (possibly agent-wrapped) `message_update`, moved out; the event otherwise.
fn into_update_parts(event: ClientEvent) -> Result<OwnedUpdate, ClientEvent> {
    match event {
        ClientEvent::Session(AgentSessionEvent::MessageUpdate {
            message,
            assistant_message_event,
        }) => Ok((None, message, assistant_message_event)),
        ClientEvent::Oino(OinoEvent::AgentEvent { agent_id, event }) => match *event {
            ClientEvent::Session(AgentSessionEvent::MessageUpdate {
                message,
                assistant_message_event,
            }) => Ok((Some(agent_id), message, assistant_message_event)),
            inner => Err(ClientEvent::Oino(OinoEvent::AgentEvent {
                agent_id,
                event: Box::new(inner),
            })),
        },
        other => Err(other),
    }
}

fn rebuild_update(
    agent: Option<String>,
    message: oino_ai::AgentMessage,
    assistant_message_event: oino_ai::AssistantMessageEvent,
) -> ClientEvent {
    let update = ClientEvent::Session(AgentSessionEvent::MessageUpdate {
        message,
        assistant_message_event,
    });
    match agent {
        Some(agent_id) => ClientEvent::Oino(OinoEvent::AgentEvent {
            agent_id,
            event: Box::new(update),
        }),
        None => update,
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
        let queues: Vec<Arc<Mutex<Queue>>> = subscribers.iter().filter_map(Weak::upgrade).collect();
        // perf-fix #3: the last subscriber takes the event itself — a hub with one
        // subscriber (the common case) no longer deep-copies every streamed message.
        let mut event = Some(event);
        let count = queues.len();
        for (index, queue) in queues.iter().enumerate() {
            let item = if index + 1 == count {
                event.take()
            } else {
                event.clone()
            };
            if let Some(item) = item {
                lock(queue).push(item, capacity);
            }
        }
        subscribers.retain(|weak| weak.upgrade().is_some_and(|queue| !lock(&queue).closed));
    }

    /// [`Self::publish`] for a borrowed event: one clone per live subscriber, none when
    /// nobody listens (relays that are usually unwatched).
    pub fn publish_ref(&self, event: &ClientEvent) {
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

    fn text_delta(index: usize, delta: &str) -> ClientEvent {
        let partial = oino_ai::AssistantMessage::empty("a", "p", "m", 0);
        ClientEvent::Session(AgentSessionEvent::MessageUpdate {
            message: oino_ai::AgentMessage::Assistant(partial.clone()),
            assistant_message_event: oino_ai::AssistantMessageEvent::TextDelta {
                content_index: index,
                delta: delta.into(),
                partial,
            },
        })
    }

    fn delta_of(event: &ClientEvent) -> Option<(usize, String)> {
        match event {
            ClientEvent::Session(AgentSessionEvent::MessageUpdate {
                assistant_message_event:
                    oino_ai::AssistantMessageEvent::TextDelta {
                        content_index,
                        delta,
                        ..
                    },
                ..
            }) => Some((*content_index, delta.clone())),
            _ => None,
        }
    }

    #[tokio::test]
    async fn a_lagging_subscriber_merges_deltas_without_losing_text() {
        let hub = EventHub::new();
        let mut stream = hub.subscribe();
        for i in 0..MERGE_BACKLOG {
            hub.publish(bash(&i.to_string()));
        }
        for part in ["a", "b", "c"] {
            hub.publish(text_delta(0, part));
        }
        hub.publish(text_delta(1, "x"));
        hub.publish(text_delta(1, "y"));
        hub.close();
        let events: Vec<_> = (&mut stream).collect().await;
        let deltas: Vec<_> = events.iter().filter_map(delta_of).collect();
        // The first delta after the backlog is queued; the next ones of block 0 merge into
        // it, block 1 starts a new queued delta (merged again). No text is lost.
        assert_eq!(deltas, vec![(0, "abc".to_owned()), (1, "xy".to_owned())]);
    }

    #[tokio::test]
    async fn a_keeping_up_subscriber_sees_every_delta() {
        let hub = EventHub::new();
        let mut stream = hub.subscribe();
        for part in ["a", "b", "c"] {
            hub.publish(text_delta(0, part));
        }
        hub.close();
        let events: Vec<_> = (&mut stream).collect().await;
        assert_eq!(events.len(), 3);
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
