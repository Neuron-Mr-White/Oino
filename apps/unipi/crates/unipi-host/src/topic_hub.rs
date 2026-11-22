//! Topic subscriptions (v3-perf spec §4.6, P2): snapshot + seq-numbered deltas per topic,
//! resumable by `(epoch, seq)`.
//!
//! - Every topic has an `epoch` (unique for this process and this topic's numbering run)
//!   and a `seq` that grows by one per delta. The last [`RING`] deltas are kept so a
//!   reconnecting client that sends its cursor gets exactly what it missed.
//! - A cursor from another epoch (host restart, seq overflow, dropped state) or older than
//!   the ring is a **gap**: the client gets a fresh snapshot instead.
//! - [`TopicHub::attach`] subscribes to the live deltas and computes the resume answer under
//!   one lock, so no delta can fall between the replay and the live stream.
//! - A topic's change source (a watcher, a feed bridge) runs while at least one client is
//!   attached: the first attach starts it, the last detach drops it ([`SourceGuard`]).
//!
//! Deltas are invalidations with a small payload; the app re-reads through the existing
//! ops (which keep their own authorisation checks), so no data crosses this stream that a
//! paired device could not already read.
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::Value;
use tokio::sync::broadcast;
use unipi_proto::TopicCursor;

/// Deltas kept per topic for a resume.
pub const RING: usize = 64;
/// Live buffer per topic; a slower connection lags and gets a snapshot.
const LIVE_CAPACITY: usize = 128;
/// Idle (unsubscribed) topic states kept for a cheap resume before they are swept.
const IDLE_KEEP: usize = 256;

/// One delta on the live stream.
#[derive(Debug, Clone, PartialEq)]
pub struct TopicDelta {
    pub epoch: u64,
    pub seq: u64,
    pub delta: Value,
}

/// What a (re)subscribing client gets first.
#[derive(Debug, Clone, PartialEq)]
pub enum Resume {
    /// The deltas after the client's cursor (possibly none).
    Replay {
        epoch: u64,
        deltas: Vec<(u64, Value)>,
    },
    /// A snapshot is due; deltas continue after `seq`.
    Snapshot { epoch: u64, seq: u64 },
}

/// Keeps a topic's change source running; dropping it stops the source.
pub struct SourceGuard(Option<Box<dyn FnOnce() + Send>>);

impl SourceGuard {
    pub fn new(stop: impl FnOnce() + Send + 'static) -> Self {
        Self(Some(Box::new(stop)))
    }
    #[must_use]
    pub fn task(handle: tokio::task::JoinHandle<()>) -> Self {
        Self::new(move || handle.abort())
    }
}

impl Drop for SourceGuard {
    fn drop(&mut self) {
        if let Some(stop) = self.0.take() {
            stop();
        }
    }
}

struct TopicState {
    epoch: u64,
    seq: u64,
    ring: VecDeque<(u64, Value)>,
    live: broadcast::Sender<TopicDelta>,
    subscribers: usize,
    source: Option<SourceGuard>,
    source_gate: Arc<Mutex<()>>,
    /// Order of going idle (for the sweep).
    idle_since: u64,
}

pub struct TopicHub {
    next_epoch: Mutex<u64>,
    topics: Mutex<HashMap<String, TopicState>>,
    clock: Mutex<u64>,
}

static SHARED: OnceLock<TopicHub> = OnceLock::new();

impl Default for TopicHub {
    fn default() -> Self {
        Self::new()
    }
}

impl TopicHub {
    /// A hub whose epochs start from the wall clock, so they never repeat across restarts.
    #[must_use]
    pub fn new() -> Self {
        let base = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(1, |d| u64::try_from(d.as_micros()).unwrap_or(u64::MAX));
        Self {
            next_epoch: Mutex::new(base.max(1)),
            topics: Mutex::new(HashMap::new()),
            clock: Mutex::new(0),
        }
    }

    /// The host's one hub.
    pub fn shared() -> &'static Self {
        SHARED.get_or_init(Self::new)
    }

    fn epoch(&self) -> u64 {
        let mut next = lock(&self.next_epoch);
        *next = next.wrapping_add(1).max(1);
        *next
    }

    fn fresh_state(&self) -> TopicState {
        TopicState {
            epoch: self.epoch(),
            seq: 0,
            ring: VecDeque::new(),
            live: broadcast::channel(LIVE_CAPACITY).0,
            subscribers: 0,
            source: None,
            source_gate: Arc::new(Mutex::new(())),
            idle_since: 0,
        }
    }

    /// Publishes one delta (no-op when nobody holds the topic: a later subscriber gets a
    /// snapshot anyway). A seq at `u64::MAX` starts a new epoch: subscribers see the epoch
    /// change and take a snapshot.
    pub fn publish(&self, topic: &str, delta: Value) {
        let mut topics = lock(&self.topics);
        let Some(state) = topics.get_mut(topic) else {
            return;
        };
        if state.seq == u64::MAX {
            state.epoch = self.epoch();
            state.seq = 0;
            state.ring.clear();
        }
        state.seq += 1;
        state.ring.push_back((state.seq, delta.clone()));
        while state.ring.len() > RING {
            state.ring.pop_front();
        }
        let _ = state.live.send(TopicDelta {
            epoch: state.epoch,
            seq: state.seq,
            delta,
        });
    }

    /// Subscribes to `topic`: the live receiver plus the resume answer for `since`.
    /// `start` builds the change source when this is the first subscriber.
    /// # Panics
    /// Panics if the reserved topic subscription is unexpectedly absent from the locked registry.
    pub fn attach(
        &self,
        topic: &str,
        since: Option<TopicCursor>,
        start: impl FnOnce() -> Option<SourceGuard>,
    ) -> (broadcast::Receiver<TopicDelta>, Resume) {
        let gate = {
            let mut topics = lock(&self.topics);
            let state = topics
                .entry(topic.to_owned())
                .or_insert_with(|| self.fresh_state());
            // Reserve the ref before releasing the map: detach/sweep cannot remove it.
            state.subscribers += 1;
            Arc::clone(&state.source_gate)
        };
        let _startup = lock(&gate);
        let needs_source = lock(&self.topics)
            .get(topic)
            .is_some_and(|s| s.source.is_none());
        // Watcher construction may do filesystem IO or publish. Never hold the map.
        let source = needs_source.then(start).flatten();
        let mut topics = lock(&self.topics);
        let state = topics.get_mut(topic).expect("reserved subscription");
        if needs_source {
            state.source = source;
        }
        let receiver = state.live.subscribe();
        let resume = resume_of(state, since);
        (receiver, resume)
    }

    /// One subscriber left; the last one stops the source (the numbering is kept a while
    /// so a quick resubscribe can still replay).
    pub fn detach(&self, topic: &str) {
        let stopped = {
            let mut topics = lock(&self.topics);
            let Some(state) = topics.get_mut(topic) else {
                return;
            };
            state.subscribers = state.subscribers.saturating_sub(1);
            if state.subscribers > 0 {
                return;
            }
            let mut clock = lock(&self.clock);
            *clock += 1;
            state.idle_since = *clock;
            let source = state.source.take();
            // sessions has a process-lifetime publisher. Watcher topics, including
            // failed starts, cannot replay changes missed while unsubscribed.
            if topic != "sessions" {
                state.epoch = self.epoch();
                state.seq = 0;
                state.ring.clear();
            }
            sweep(&mut topics);
            source
        };
        // Stop outside the lock (a source's stop may take other locks).
        drop(stopped);
    }

    /// The current `(epoch, seq)` of a topic (tests, diagnostics).
    #[must_use]
    pub fn cursor(&self, topic: &str) -> Option<TopicCursor> {
        lock(&self.topics).get(topic).map(|s| TopicCursor {
            epoch: s.epoch,
            seq: s.seq,
        })
    }

    /// Subscribers of a topic (tests).
    #[must_use]
    pub fn subscribers(&self, topic: &str) -> usize {
        lock(&self.topics).get(topic).map_or(0, |s| s.subscribers)
    }

    /// Whether the topic's source runs (tests).
    #[must_use]
    pub fn source_running(&self, topic: &str) -> bool {
        lock(&self.topics)
            .get(topic)
            .is_some_and(|s| s.source.is_some())
    }

    #[cfg(test)]
    fn set_seq(&self, topic: &str, seq: u64) {
        if let Some(state) = lock(&self.topics).get_mut(topic) {
            state.seq = seq;
            state.ring.clear();
        }
    }
}

fn resume_of(state: &TopicState, since: Option<TopicCursor>) -> Resume {
    let snapshot = Resume::Snapshot {
        epoch: state.epoch,
        seq: state.seq,
    };
    let Some(since) = since else { return snapshot };
    if since.epoch != state.epoch || since.seq > state.seq {
        return snapshot;
    }
    if since.seq == state.seq {
        return Resume::Replay {
            epoch: state.epoch,
            deltas: Vec::new(),
        };
    }
    // The ring must hold every delta after `since`.
    match state.ring.front() {
        Some((first, _)) if *first <= since.seq + 1 => Resume::Replay {
            epoch: state.epoch,
            deltas: state
                .ring
                .iter()
                .filter(|(seq, _)| *seq > since.seq)
                .cloned()
                .collect(),
        },
        _ => snapshot,
    }
}

fn sweep(topics: &mut HashMap<String, TopicState>) {
    let idle: Vec<(u64, String)> = topics
        .iter()
        .filter(|(_, s)| s.subscribers == 0)
        .map(|(k, s)| (s.idle_since, k.clone()))
        .collect();
    if idle.len() <= IDLE_KEEP {
        return;
    }
    let excess = idle.len() - IDLE_KEEP;
    let mut idle = idle;
    idle.sort();
    for (_, key) in idle.into_iter().take(excess) {
        topics.remove(&key);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    fn none() -> Option<SourceGuard> {
        None
    }

    #[test]
    fn startup_can_publish_without_holding_global_map() {
        let hub = TopicHub::new();
        let (_rx, resume) = hub.attach("fs:/test", None, || {
            hub.publish("fs:/test", serde_json::json!({"invalidate": true}));
            Some(SourceGuard::new(|| {}))
        });
        assert!(matches!(resume, Resume::Snapshot { seq: 1, .. }));
        assert!(hub.source_running("fs:/test"));
        hub.detach("fs:/test");
    }

    #[test]
    fn failed_source_detach_invalidates_cursor() {
        let hub = TopicHub::new();
        let _ = hub.attach("fs:/missing", None, || None);
        let old = hub.cursor("fs:/missing").unwrap();
        assert!(!hub.source_running("fs:/missing"));
        hub.detach("fs:/missing");
        assert_ne!(old.epoch, hub.cursor("fs:/missing").unwrap().epoch);
    }

    #[test]
    fn first_subscribe_gets_a_snapshot_then_numbered_deltas() {
        let hub = TopicHub::new();
        let (mut rx, resume) = hub.attach("cron", None, none);
        let Resume::Snapshot { epoch, seq: 0 } = resume else {
            panic!("{resume:?}")
        };
        hub.publish("cron", json!({"n": 1}));
        hub.publish("cron", json!({"n": 2}));
        assert_eq!(
            rx.try_recv().unwrap(),
            TopicDelta {
                epoch,
                seq: 1,
                delta: json!({"n": 1})
            }
        );
        assert_eq!(rx.try_recv().unwrap().seq, 2);
    }

    #[test]
    fn reconnect_resumes_from_the_last_seq() {
        let hub = TopicHub::new();
        let (_rx, Resume::Snapshot { epoch, .. }) = hub.attach("sessions", None, none) else {
            panic!()
        };
        for n in 1..=5 {
            hub.publish("sessions", json!(n));
        }
        // The client saw seq 2, then its connection dropped.
        hub.detach("sessions");
        let (_rx, resume) = hub.attach("sessions", Some(TopicCursor { epoch, seq: 2 }), none);
        assert_eq!(
            resume,
            Resume::Replay {
                epoch,
                deltas: vec![(3, json!(3)), (4, json!(4)), (5, json!(5))]
            }
        );
        // Up to date: nothing to replay, no snapshot.
        let (_rx, resume) = hub.attach("sessions", Some(TopicCursor { epoch, seq: 5 }), none);
        assert_eq!(
            resume,
            Resume::Replay {
                epoch,
                deltas: vec![]
            }
        );
    }

    #[test]
    fn a_gap_or_a_foreign_epoch_gets_a_fresh_snapshot() {
        let hub = TopicHub::new();
        let (_rx, Resume::Snapshot { epoch, .. }) = hub.attach("board:p", None, none) else {
            panic!()
        };
        for n in 0..(RING as u64 + 10) {
            hub.publish("board:p", json!(n));
        }
        let seq = hub.cursor("board:p").unwrap().seq;
        // Older than the ring: a gap.
        let (_rx, resume) = hub.attach("board:p", Some(TopicCursor { epoch, seq: 3 }), none);
        assert_eq!(resume, Resume::Snapshot { epoch, seq });
        // The oldest seq the ring still covers replays.
        let (_rx, resume) = hub.attach(
            "board:p",
            Some(TopicCursor {
                epoch,
                seq: seq - RING as u64,
            }),
            none,
        );
        assert!(matches!(resume, Resume::Replay { ref deltas, .. } if deltas.len() == RING));
        // Another epoch (a host restart) or a seq from the future.
        for bad in [
            TopicCursor {
                epoch: epoch + 99,
                seq: 1,
            },
            TopicCursor {
                epoch,
                seq: seq + 1,
            },
        ] {
            let (_rx, resume) = hub.attach("board:p", Some(bad), none);
            assert_eq!(resume, Resume::Snapshot { epoch, seq });
        }
    }

    #[test]
    fn seq_overflow_starts_a_new_epoch() {
        let hub = TopicHub::new();
        let (mut rx, Resume::Snapshot { epoch, .. }) = hub.attach("models", None, none) else {
            panic!()
        };
        hub.set_seq("models", u64::MAX);
        hub.publish("models", json!("wrapped"));
        let delta = rx.try_recv().unwrap();
        assert_ne!(delta.epoch, epoch);
        assert_eq!(delta.seq, 1);
        // The old cursor is from another epoch now: snapshot.
        let (_rx, resume) = hub.attach(
            "models",
            Some(TopicCursor {
                epoch,
                seq: u64::MAX,
            }),
            none,
        );
        assert!(matches!(resume, Resume::Snapshot { seq: 1, .. }));
    }

    #[test]
    fn a_lagging_subscriber_sees_lagged_not_silence() {
        let hub = TopicHub::new();
        let (mut rx, _) = hub.attach("ports", None, none);
        for n in 0..(LIVE_CAPACITY + 5) {
            hub.publish("ports", json!(n));
        }
        assert!(matches!(
            rx.try_recv(),
            Err(broadcast::error::TryRecvError::Lagged(_))
        ));
    }

    #[test]
    fn the_source_runs_while_someone_is_subscribed() {
        let hub = TopicHub::new();
        let started = Arc::new(AtomicUsize::new(0));
        let stopped = Arc::new(AtomicUsize::new(0));
        let start = || {
            started.fetch_add(1, Ordering::SeqCst);
            let stopped = Arc::clone(&stopped);
            Some(SourceGuard::new(move || {
                stopped.fetch_add(1, Ordering::SeqCst);
            }))
        };
        let _a = hub.attach("fs:/d", None, start);
        let _b = hub.attach("fs:/d", None, start);
        assert_eq!(
            (started.load(Ordering::SeqCst), hub.subscribers("fs:/d")),
            (1, 2)
        );
        hub.detach("fs:/d");
        assert!(hub.source_running("fs:/d"));
        hub.detach("fs:/d");
        assert!(!hub.source_running("fs:/d"));
        assert_eq!(stopped.load(Ordering::SeqCst), 1);
        // Publishing to an unheld topic is a no-op, not a new state.
        hub.publish("nobody", json!(1));
        assert!(hub.cursor("nobody").is_none());
    }

    #[test]
    fn stopping_a_watcher_invalidates_the_resume_cursor() {
        let hub = TopicHub::new();
        let (_, Resume::Snapshot { epoch, seq }) =
            hub.attach("fs:/d", None, || Some(SourceGuard::new(|| {})))
        else {
            panic!()
        };
        hub.detach("fs:/d");
        let (_, resume) = hub.attach("fs:/d", Some(TopicCursor { epoch, seq }), none);
        assert!(matches!(resume, Resume::Snapshot { .. }));
    }

    #[test]
    fn idle_states_are_swept_beyond_the_cap() {
        let hub = TopicHub::new();
        for i in 0..(IDLE_KEEP + 20) {
            let topic = format!("fs:/d{i}");
            let _ = hub.attach(&topic, None, none);
            hub.detach(&topic);
        }
        assert!(lock(&hub.topics).len() <= IDLE_KEEP);
        // The newest idle state survives for a resume.
        assert!(hub.cursor(&format!("fs:/d{}", IDLE_KEEP + 19)).is_some());
    }
}
