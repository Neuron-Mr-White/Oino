//! Per-client session streams (host.md §7.3–7.5, WP-H4): the snapshot / resume reply, the
//! per-session seq numbering and ring, delta coalescing, 16 ms batching, backpressure, and
//! the hub event stream (`events_open`).
//!
//! ```text
//! session_open{session, since_seq?} → ok{data: SessionSnapshot}
//! SessionCmd{id?, cmd}  →  ok{id?, data} | err{id?, error}   (correlated)
//!                       ←  events{items: [SessionEventDto…]} (≤ 1 frame / 16 ms)
//! ```
//!
//! One [`SessionService`] lives in the host process (installed by `serve`, the same way the
//! terminal registry is). It owns the hub plus one [`Feed`] per session id:
//!
//! - **Numbering.** Every event a session produces gets the next `seq` from the feed, from 1
//!   and never repeated, and is kept in a 4096-event ring (§7.4). The feed listens to the
//!   runtime with one *tap* task, not one per client, so two clients of one session see the
//!   same events and a client that goes away does not stop the recording: a run in flight is
//!   still numbered while nobody listens, and the tap lets go (so core can evict the idle
//!   session) only once it is idle with no subscribers.
//! - **Resume.** `session_open{since_seq}` answers with the ring's events after `since_seq`
//!   when the ring provably still holds all of them (`reset:false`, `replay`), and with a
//!   full snapshot otherwise (`reset:true`). A session whose runtime was disposed (evicted,
//!   taken over, closed, archived) always resets: the new runtime's state, not the old
//!   stream, is the truth then.
//! - **Coalescing.** Consecutive `message_update` text/thinking deltas for the same message
//!   and block merge into one event per ≤ [`FLUSH_WINDOW`] window; a non-delta event flushes
//!   at once. One `events` frame per flush tick per stream, never reordered, non-delta
//!   events never dropped. A slow client applies backpressure through its bounded channel:
//!   non-delta events wait for room (correctness first), deltas are cumulative-append and
//!   may be dropped when the channel is full (lossy by design, §7.4).
//! - **Host-only events** ride the same envelope with reserved `kind`s (§7.4): the tap adds
//!   `{kind:"crashed", error}` and `{kind:"taken_over", from}` next to the core event that
//!   caused them, so the app's banner has the resolved text (core events are tagged `type`,
//!   host-only ones `kind`). The *winning* side of a takeover gets one too: the stream that
//!   opened the session from another runner publishes `{kind:"session_acquired", from}` as its
//!   first event after the snapshot (§8.2).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::Instrument;

use oino_client::backend::SessionBackend;
use oino_client::protocol::{AgentCommand, RpcCommand, RpcRequest, RpcSessionState};
use oino_core::events::ClientEvent;
use oino_core::ui::UiResponse;
use oino_ui::slot::UiCapabilities;
use unipi_proto::{
    AppErrorCode, AppErrorDto, Control, HubEvent as WireHubEvent, NoticeLevel, Request, RunState,
    SessionCard, SessionEventDto, SessionReply, SessionSnapshot, SessionsListData,
};

use super::bound;
use super::card::{HubState, SessionCard as HostCard};
use super::hub::{takeover_from, HubError, HubEvent, HubSession, SessionHub};
use super::open::{self as fast, Live, Page, SessionFile};
use super::record_model;

/// The coalescing window of §7.4: consecutive deltas merge for at most this long.
pub const FLUSH_WINDOW: Duration = Duration::from_millis(16);
/// How many numbered events one session keeps for a resume (§7.4).
pub const RING_CAPACITY: usize = 4096;
/// perf-fix: the ring's byte budget per session. A resume replay must fit one frame
/// ([`bound::BATCH_BUDGET`], see `replay_fits`), so events older than that budget can never be
/// replayed anyway; keeping them only held ~1.3 MB per session ever opened (perf-baseline #8).
pub const RING_BYTES: usize = bound::BATCH_BUDGET;
/// Per-event framing overhead counted against [`RING_BYTES`] (the same figure `replay_fits` uses).
const RING_EVENT_OVERHEAD: usize = 96;
/// The per-session tap's queue depth (§7.4 `bounded broadcast channel (256)`).
pub const TAP_CAPACITY: usize = 256;
/// Consecutive idle polls after which a subscriber-less tap lets the runtime go.
const IDLE_POLLS: u32 = 3;
/// How often the tap checks whether it is still needed.
const IDLE_POLL: Duration = Duration::from_secs(1);
/// Reply budget of one `SessionCmd` (the app uses the same numbers, app spec §3.2).
const CMD_TIMEOUT: Duration = Duration::from_secs(30);
/// `_oino/render` answers fast or not at all: the timeline falls back to `null`.
const RENDER_TIMEOUT: Duration = Duration::from_secs(2);
/// [chat-tree] `_oino/navigate_tree` with `summarize` runs the branch summariser (a model
/// call) before it answers; the app waits the same budget.
const NAVIGATE_TIMEOUT: Duration = Duration::from_secs(300);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn internal(error: impl std::fmt::Display) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_INTERNAL, error.to_string(), false)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "Owned error/value adapter matches its consuming callback and avoids borrow-only wrappers at every call site."
)]
fn hub_error(error: HubError) -> AppErrorDto {
    let message = error.to_string();
    let code = match &error {
        HubError::Client(oino_client::backend::ClientError::Closed) => AppErrorCode::E_TRANSPORT,
        // An unknown session id: final, so the app forgets it instead of retrying.
        HubError::NotFound(_) => AppErrorCode::E_NOT_FOUND,
        _ => AppErrorCode::E_INTERNAL,
    };
    AppErrorDto::new(code, message, false)
}

/// One client's subscription to a [`Feed`]: bounded, dropped with the stream.
pub struct Subscription {
    feed: Arc<Feed>,
    id: u64,
    receiver: mpsc::Receiver<Arc<SessionEventDto>>,
}

impl Subscription {
    /// The next event, or `None` when the feed is gone.
    pub async fn recv(&mut self) -> Option<Arc<SessionEventDto>> {
        self.receiver.recv().await
    }

    /// Takes everything already queued, without waiting (the flush before a reply, §7.3).
    pub fn drain(&mut self) -> Vec<Arc<SessionEventDto>> {
        let mut items = Vec::new();
        while let Ok(event) = self.receiver.try_recv() {
            items.push(event);
        }
        items
    }
}

impl Drop for Subscription {
    fn drop(&mut self) {
        self.feed.unsubscribe(self.id);
    }
}

/// Receipt-ordered commands, with exactly one drain even across the wake boundary.
///
/// This queue serializes dispatch; it does not acknowledge commands itself.
/// Prompt ACKs wait for durable backend admission after wake/recovery. A durable
/// user JSONL receipt reconciles accepted commands after a host restart.
#[derive(Default)]
struct CommandQueue {
    pending: Mutex<(bool, VecDeque<QueuedCommand>)>,
}

type QueuedCommand = std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send>>;

impl CommandQueue {
    fn push(self: &Arc<Self>, job: impl std::future::Future<Output = ()> + Send + 'static) {
        let mut state = lock(&self.pending);
        state.1.push_back(Box::pin(job));
        if state.0 {
            return;
        }
        state.0 = true;
        let queue = Arc::clone(self);
        tokio::spawn(async move {
            loop {
                let next = {
                    let mut state = lock(&queue.pending);
                    if let Some(job) = state.1.pop_front() {
                        job
                    } else {
                        state.0 = false;
                        return;
                    }
                };
                next.await;
            }
        });
    }
}

/// The events of one session, numbered and kept for a resume (§7.4).
pub struct Feed {
    session_id: Mutex<String>,
    seq: AtomicU64,
    ring: Mutex<Ring>,
    subscribers: Mutex<Vec<(u64, mpsc::Sender<Arc<SessionEventDto>>)>>,
    next_subscriber: AtomicU64,
    tap: Mutex<Option<JoinHandle<()>>>,
    tap_gate: tokio::sync::Mutex<()>,
    /// Preserve arrival order for commands accepted while wake is in progress.
    wake_commands: Arc<CommandQueue>,
    /// The backend the running tap listens to (a repaired/reopened runtime is a new one).
    tap_backend: Mutex<Option<Arc<dyn SessionBackend>>>,
    /// The runtime this feed was numbering is gone (evicted, taken over, closed): a reopen is
    /// a new runtime, so every stream resets rather than replay.
    disposed: AtomicBool,
    /// Deltas dropped for a slow subscriber (metrics/debugging).
    dropped: AtomicU64,
    /// The host's blob store: big inline images of a live event move there (`mediaRef`).
    blobs_dir: Mutex<Option<std::path::PathBuf>>,
}

impl std::fmt::Debug for Feed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Feed")
            .field("session_id", &lock(&self.session_id))
            .field("seq", &self.seq.load(Ordering::Acquire))
            .field("subscribers", &lock(&self.subscribers).len())
            .field("disposed", &self.disposed.load(Ordering::Acquire))
            .finish_non_exhaustive()
    }
}

impl Feed {
    /// A fresh feed for `session_id`, numbering from 1.
    #[must_use]
    pub fn new(session_id: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            session_id: Mutex::new(session_id.into()),
            seq: AtomicU64::new(0),
            ring: Mutex::new(Ring::default()),
            subscribers: Mutex::new(Vec::new()),
            next_subscriber: AtomicU64::new(1),
            tap: Mutex::new(None),
            tap_gate: tokio::sync::Mutex::new(()),
            wake_commands: Arc::new(CommandQueue::default()),
            tap_backend: Mutex::new(None),
            disposed: AtomicBool::new(false),
            dropped: AtomicU64::new(0),
            blobs_dir: Mutex::new(None),
        })
    }

    /// Where the feed stores the big images of live events (the hub's blob store).
    pub fn set_blobs_dir(&self, dir: std::path::PathBuf) {
        *lock(&self.blobs_dir) = Some(dir);
    }

    /// The session id events are stamped with (it changes after a fork, §7.3).
    #[must_use]
    pub fn session_id(&self) -> String {
        lock(&self.session_id).clone()
    }

    /// The last seq handed out (0 before the first event).
    #[must_use]
    pub fn seq(&self) -> u64 {
        self.seq.load(Ordering::Acquire)
    }

    /// How many deltas were dropped for slow subscribers.
    #[must_use]
    pub fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Acquire)
    }

    /// Numbers one event and fans it out to every subscriber (`event` is `ClientEvent` JSON,
    /// or a host-only one). Returns its seq.
    ///
    /// A *delta* is cumulative-append and may be dropped for a subscriber whose queue is
    /// full (lossy by design, §7.4); any other event waits for room, so it is never lost or
    /// reordered — the client's queue is the backpressure.
    pub async fn publish(&self, mut event: Value) -> u64 {
        // fix-frames: every event is bounded once, here, so the ring (resume replays) and
        // every subscriber carry the frame-safe form — a screenshot becomes a `mediaRef`, a
        // multi-megabyte tool output is clipped and flagged `truncated`.
        let blobs_dir = lock(&self.blobs_dir).clone();
        let (bounded, bytes) =
            bound::bound_event_sized(&mut event, blobs_dir.as_deref(), bound::EVENT_BUDGET);
        if bounded {
            let kind = event
                .get("type")
                .or_else(|| event.get("kind"))
                .and_then(Value::as_str)
                .unwrap_or("?")
                .to_owned();
            tracing::info!(
                session = %self.session_id(),
                kind = %kind,
                bytes,
                "bounded an oversize live session event"
            );
        }
        self.publish_body(event.into(), bytes).await
    }

    /// Publish worker JSON through the same sequence, ring and backpressure contract.
    /// Oversize events use exactly the typed path's blob/truncation policy.
    /// # Panics
    /// Panics if previously validated raw JSON unexpectedly fails to deserialize.
    pub async fn publish_raw(&self, event: Box<serde_json::value::RawValue>) -> u64 {
        if event.get().len() > bound::EVENT_BUDGET {
            return self
                .publish(serde_json::from_str(event.get()).expect("validated raw JSON"))
                .await;
        }
        let bytes = event.get().len();
        self.publish_body(unipi_proto::session::EventBody::raw(event), bytes)
            .await
    }

    async fn publish_body(&self, event: unipi_proto::session::EventBody, bytes: usize) -> u64 {
        let delta = event.is_message_update() && is_delta(&event);
        let seq = self.seq.fetch_add(1, Ordering::AcqRel) + 1;
        let dto = Arc::new(SessionEventDto {
            session: self.session_id(),
            seq,
            event,
        });
        lock(&self.ring).push(Arc::clone(&dto), bytes + RING_EVENT_OVERHEAD);
        if delta {
            self.fan_out_lossy(&dto);
        } else {
            self.fan_out_awaiting(dto).await;
        }
        seq
    }

    fn fan_out_lossy(&self, event: &Arc<SessionEventDto>) {
        let mut subscribers = lock(&self.subscribers);
        subscribers.retain(|(_, sender)| match sender.try_send(Arc::clone(event)) {
            Err(mpsc::error::TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                true
            }
            Err(mpsc::error::TrySendError::Closed(_)) => false,
            Ok(()) => true,
        });
    }

    /// Fans one event out, waiting for room (§7.4: "non-delta events suspend the tap until
    /// sent").
    async fn fan_out_awaiting(&self, event: Arc<SessionEventDto>) {
        let subscribers: Vec<(u64, mpsc::Sender<Arc<SessionEventDto>>)> =
            lock(&self.subscribers).clone();
        for (id, sender) in subscribers {
            if sender.send(Arc::clone(&event)).await.is_err() {
                self.unsubscribe(id);
            }
        }
    }

    /// A new subscriber (one per open session stream).
    #[must_use]
    pub fn subscribe(self: &Arc<Self>) -> Subscription {
        let id = self.next_subscriber.fetch_add(1, Ordering::AcqRel);
        let (sender, receiver) = mpsc::channel(TAP_CAPACITY);
        lock(&self.subscribers).push((id, sender));
        Subscription {
            feed: Arc::clone(self),
            id,
            receiver,
        }
    }

    fn unsubscribe(&self, id: u64) {
        lock(&self.subscribers).retain(|(subscriber, _)| *subscriber != id);
    }

    /// How many open streams this feed serves.
    #[must_use]
    pub fn subscriber_count(&self) -> usize {
        lock(&self.subscribers).len()
    }

    /// Whether this feed belonged to a runtime that is gone. That generation's streams are
    /// ended; a reopen is a new runtime and gets a fresh feed from [`SessionService::feed`].
    #[must_use]
    pub fn is_disposed(&self) -> bool {
        self.disposed.load(Ordering::Acquire)
    }

    /// The runtime this feed was numbering is gone: the next open is a new runtime, so a
    /// resume cannot be trusted (§7.4).
    pub fn mark_disposed(&self) {
        self.disposed.store(true, Ordering::Release);
        // A disposed feed never resumes again: its ring is dead weight.
        self.forget_ring();
    }

    /// Ends every open stream on this feed: the runtime they were following is gone (evicted,
    /// taken over, closed). The app treats a clean end as a lost link and reopens (§7.4;
    /// `lib/session/stream.ts`), so the chat wakes again instead of sitting silent on a dead
    /// backend until the app is reloaded. Events already fanned out still reach the client
    /// first — dropping the senders only ends the stream after them.
    fn end_streams(&self) {
        let ended = {
            let mut subscribers = lock(&self.subscribers);
            let ended = subscribers.len();
            subscribers.clear();
            ended
        };
        tracing::info!(
            session = %self.session_id(),
            streams = ended,
            "session feed: runtime ended; ending its streams"
        );
    }

    /// Drops the buffered events (perf-fix #8): the feed keeps only its numbering. A later
    /// `session_open{since_seq}` with `since == seq` still resumes (nothing to replay), an
    /// older one resets to a snapshot — exactly what a ring that rolled over would do.
    pub fn forget_ring(&self) {
        *lock(&self.ring) = Ring::default();
    }

    /// Bytes held by the resume ring (tests, diagnostics).
    #[must_use]
    pub fn ring_bytes(&self) -> usize {
        lock(&self.ring).bytes
    }

    /// Can `session_open{since_seq}` be answered with the ring instead of a snapshot?
    #[must_use]
    pub fn can_resume(&self, since: u64) -> bool {
        let seq = self.seq();
        if since == 0 || since > seq || self.is_disposed() {
            return false;
        }
        if since == seq {
            return true;
        }
        let ring = lock(&self.ring);
        // The ring must still hold the event right after `since` (it holds every event from
        // its front to `seq` — numbering and eviction are contiguous).
        ring.events
            .front()
            .is_some_and(|(first, _)| first.seq <= since + 1)
    }

    /// The buffered events after `since`, in order (the resume `replay`, §7.4).
    #[must_use]
    pub fn events_after(&self, since: u64) -> Vec<SessionEventDto> {
        lock(&self.ring)
            .events
            .iter()
            .filter(|(event, _)| event.seq > since)
            .map(|(event, _)| (**event).clone())
            .collect()
    }

    /// Starts the session's tap unless one is already running (§7.4).
    pub fn ensure_tap(
        self: &Arc<Self>,
        backend: Arc<dyn SessionBackend>,
        service: Weak<SessionService>,
    ) {
        let mut tap = lock(&self.tap);
        let mut tapped = lock(&self.tap_backend);
        tracing::debug!(
            session = %self.session_id(),
            previous_runtime = ?tapped
                .as_ref()
                .and_then(|backend| backend.runtime_identity()),
            runtime = ?backend.runtime_identity(),
            tap_running = tap.as_ref().is_some_and(|task| !task.is_finished()),
            "session feed: ensure_tap"
        );
        if tap.as_ref().is_some_and(|task| !task.is_finished())
            && tapped.as_ref().is_some_and(|old| {
                match (old.runtime_identity(), backend.runtime_identity()) {
                    (Some(a), Some(b)) => a == b,
                    _ => Arc::ptr_eq(old, &backend),
                }
            })
        {
            tracing::debug!(
                session = %self.session_id(),
                "session feed: tap kept (same runtime)"
            );
            return;
        }
        tracing::info!(
            session = %self.session_id(),
            previous_runtime = ?tapped
                .as_ref()
                .and_then(|backend| backend.runtime_identity()),
            runtime = ?backend.runtime_identity(),
            "session feed: retapping for the new runtime"
        );
        // Subscribe before returning, but wait for the aborted tap to finish before
        // publishing from the new one: two runtimes must never number concurrently.
        let events = backend.events();
        let previous = tap.take();
        if let Some(task) = &previous {
            task.abort();
        }
        *tapped = Some(Arc::clone(&backend));
        let feed = Arc::clone(self);
        *tap = Some(tokio::spawn(async move {
            if let Some(task) = previous {
                let _ = task.await;
            }
            let _guard = feed.tap_gate.lock().await;
            Arc::clone(&feed).run_tap(backend, service, events).await;
        }));
    }

    /// Stops the tap (the runtime is gone; a later open restarts it).
    pub fn stop_tap(&self) {
        let mut tap = lock(&self.tap);
        *lock(&self.tap_backend) = None;
        if let Some(task) = tap.take() {
            task.abort();
        }
    }

    /// Follows the runtime: number its events until the runtime is gone or it is idle with
    /// no subscribers (so core can evict it, §4.2 lazy).
    async fn run_tap(
        self: Arc<Self>,
        backend: Arc<dyn SessionBackend>,
        service: Weak<SessionService>,
        mut events: futures_util::stream::BoxStream<'static, ClientEvent>,
    ) {
        tracing::info!(
            session = %self.session_id(),
            runtime = ?backend.runtime_identity(),
            "session feed: tap started"
        );
        let mut tick = tokio::time::interval(IDLE_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut idle_polls = 0;
        loop {
            tokio::select! {
                event = futures_util::StreamExt::next(&mut events) => {
                    let Some(event) = event else {
                        // The runtime ended (evicted, taken over, closed): a reopen is new,
                        // and the streams have nothing left to follow.
                        tracing::info!(
                            session = %self.session_id(),
                            runtime = ?backend.runtime_identity(),
                            "session feed: tap event stream ended"
                        );
                        self.mark_disposed();
                        self.end_streams();
                        return;
                    };
                    idle_polls = 0;
                    let Ok(value) = serde_json::to_value(&event) else {
                        continue;
                    };
                    let replaced = replaced_session(&value);
                    // v2 QA D-03: the agent names the session mid-run (`session_info_changed`,
                    // e.g. the auto-title after the first turn) — re-list the card so the
                    // Sessions list and the header pick the name up live, not after a restart.
                    let renamed = value.get("type").and_then(Value::as_str)
                        == Some("session_info_changed");
                    self.publish(value).await;
                    if renamed {
                        if let Some(service) = service.upgrade() {
                            service.hub().refresh(&self.session_id()).await;
                        }
                    }
                    // §7.4: the host adds its own kinds next to the core event that caused
                    // them, carrying the resolved text for the app's banner.
                    match event {
                        ClientEvent::Oino(oino_core::events::OinoEvent::SessionCrashed {
                            ref error,
                        }) => {
                            self.publish(json!({"kind": "crashed", "error": error}))
                                .await;
                        }
                        ClientEvent::Oino(oino_core::events::OinoEvent::SessionTakenOver {
                            ref by,
                        }) => {
                            let from = takeover_from(by);
                            self.publish(json!({"kind": "taken_over", "from": from}))
                                .await;
                            self.mark_disposed();
                            // §8.4: the card says who took it; a stream that reopens the
                            // record takes it back (and resets, this runtime is gone).
                            if let Some(service) = service.upgrade() {
                                service
                                    .hub()
                                    .mark_taken_over(&self.session_id(), &from)
                                    .await;
                            }
                            self.end_streams();
                            return;
                        }
                        ClientEvent::Oino(oino_core::events::OinoEvent::SessionEvicted {
                            ..
                        }) => {
                            self.mark_disposed();
                            self.end_streams();
                            return;
                        }
                        _ => {}
                    }
                    if let Some(new_id) = replaced {
                        let old_id = self.rename(&new_id);
                        if let Some(service) = service.upgrade() {
                            service.rekey(&old_id, &new_id);
                        }
                    }
                    // P0-06: the host owns phone push — dialogs flip the card's
                    // `needs_you` and push `unipi1:` E2E notices (deep link +
                    // UnifiedPush fan-out), settles push the run-finished notice
                    // and `notify_user` tool calls their message; everything not a
                    // notice returns immediately (`crate::notify::on_session_event`).
                    if let Some(service) = service.upgrade() {
                        crate::notify::on_session_event(
                            service.hub(),
                            &crate::data_dir::HostPaths::resolve(),
                            &event,
                            &self.session_id(),
                            crate::superagent::current()
                                .is_ok_and(|dispatcher| dispatcher.is_lead_session(&self.session_id())),
                        )
                        .await;
                    }
                }
                _ = tick.tick() => {
                    if self.subscriber_count() > 0 {
                        idle_polls = 0;
                        continue;
                    }
                    idle_polls += 1;
                    if idle_polls >= IDLE_POLLS && !is_busy(&backend).await {
                        // Idle and unwatched: stop numbering so the hub can evict (§4.2),
                        // and let the buffered events go (perf-fix #8: the soak leak).
                        self.forget_ring();
                        return;
                    }
                }
            }
        }
    }

    /// Renames the feed's stamp; returns the id it had before (the registry rekeys on it).
    fn rename(&self, new_id: &str) -> String {
        let mut id = lock(&self.session_id);
        let previous = id.clone();
        id.clone_from(&new_id.to_owned());
        previous
    }
}

/// The resume ring of one feed: shared events (no deep copy per event) bounded by count
/// ([`RING_CAPACITY`]) and bytes ([`RING_BYTES`]).
#[derive(Default)]
struct Ring {
    events: VecDeque<(Arc<SessionEventDto>, usize)>,
    bytes: usize,
}

impl Ring {
    fn push(&mut self, event: Arc<SessionEventDto>, bytes: usize) {
        self.bytes += bytes;
        self.events.push_back((event, bytes));
        while self.events.len() > RING_CAPACITY
            || (self.bytes > RING_BYTES && self.events.len() > 1)
        {
            if let Some((_, size)) = self.events.pop_front() {
                self.bytes -= size;
            }
        }
    }
}

/// The new session id of a `session_replaced` event (§7.3: a fork keeps the stream).
fn replaced_session(event: &Value) -> Option<String> {
    if event.get("type")?.as_str()? != "session_replaced" {
        return None;
    }
    event.get("sessionId")?.as_str().map(str::to_owned)
}

/// Is a run (or compaction) in flight? A disposed/evicted backend is not busy.
async fn is_busy(backend: &Arc<dyn SessionBackend>) -> bool {
    match backend
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await
    {
        // Waiting on work that will wake the session counts: the woken run's dialogs and
        // settle notices must still reach the card and the phone.
        Ok(Some(data)) => serde_json::from_value::<RpcSessionState>(data).is_ok_and(|state| {
            state.is_streaming || state.is_compacting || !state.waiting_on.is_empty()
        }),
        _ => false,
    }
}

/// The host's session streams: the hub plus one [`Feed`] per session id (§7.4).
pub struct SessionService {
    hub: Arc<SessionHub>,
    feeds: Mutex<HashMap<String, Arc<Feed>>>,
}

static SHARED: OnceLock<Arc<SessionService>> = OnceLock::new();

impl std::fmt::Debug for SessionService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionService")
            .field("hub", &self.hub)
            .field("feeds", &lock(&self.feeds).len())
            .finish()
    }
}

impl SessionService {
    /// The session service over `hub`.
    #[must_use]
    pub fn new(hub: Arc<SessionHub>) -> Arc<Self> {
        Arc::new(Self {
            hub,
            feeds: Mutex::new(HashMap::new()),
        })
    }

    /// Installs the process's session service (the `serve` daemon does this once); a second
    /// call returns the first, so the protocol handler always sees the same feeds.
    pub fn install(hub: Arc<SessionHub>) -> Arc<Self> {
        Arc::clone(SHARED.get_or_init(move || Self::new(hub)))
    }

    /// The installed service, when this process serves a host.
    #[must_use]
    pub fn shared() -> Option<Arc<Self>> {
        SHARED.get().cloned()
    }

    /// The session hub behind the streams.
    #[must_use]
    pub fn hub(&self) -> &Arc<SessionHub> {
        &self.hub
    }

    /// The feed of `session_id` (created on first use). A disposed feed is replaced: its
    /// runtime is gone, so a reopened chat must not share the dead numbering (the old tap's
    /// `end_streams` may still be clearing the old subscribers, which must never include the
    /// stream that opened for the new runtime).
    ///
    /// A feed whose runtime replaced its session id (a fork, §7.3) keeps numbering under the
    /// new id; the registry catches up with it here, so a forked stream shares one feed
    /// instead of tapping the runtime twice.
    #[must_use]
    pub fn feed(&self, session_id: &str) -> Arc<Feed> {
        let mut feeds = lock(&self.feeds);
        if let Some(feed) = feeds.get(session_id) {
            if !feed.is_disposed() {
                return Arc::clone(feed);
            }
            feeds.remove(session_id);
            tracing::info!(
                session = session_id,
                "session feed: replacing a disposed feed for a reopen"
            );
        }
        if let Some(feed) = feeds
            .values()
            .find(|feed| feed.session_id() == session_id && !feed.is_disposed())
            .map(Arc::clone)
        {
            feeds.insert(session_id.to_owned(), Arc::clone(&feed));
            return feed;
        }
        let feed = Feed::new(session_id);
        feed.set_blobs_dir(self.hub.blobs_dir());
        feeds.insert(session_id.to_owned(), Arc::clone(&feed));
        feed
    }

    /// Moves a feed when the runtime replaces its session (a fork keeps the stream, §7.3).
    fn rekey(&self, old_id: &str, new_id: &str) {
        if old_id == new_id {
            return;
        }
        let mut feeds = lock(&self.feeds);
        if let Some(feed) = feeds.remove(old_id) {
            feeds.insert(new_id.to_owned(), feed);
        }
    }
}

/// Maps a host card onto the wire card (§4.2 `run_state` + its detail).
fn wire_card(card: &HostCard) -> SessionCard {
    let (run_state, error, from) = match &card.state {
        HubState::Idle => (RunState::Idle, None, None),
        HubState::Working => (RunState::Working, None, None),
        HubState::Crashed { error } => (RunState::Crashed, Some(error.clone()), None),
        HubState::TakenOver { from } => (RunState::TakenOver, None, Some(from.clone())),
    };
    SessionCard {
        worker: card.worker.as_deref().cloned(),
        session_id: card.session_id.clone(),
        file: card.file.display().to_string(),
        name: card.name.clone(),
        cwd: card.cwd.clone(),
        updated_at: card.updated_at,
        message_count: card.message_count,
        run_state,
        needs_you: card.needs_you,
        model: card.model.clone(),
        error,
        from,
        pinned: card.pinned,
        unread: card.unread,
        title_locked: card.title_locked,
        source: card.source.clone(),
        cron_job_id: card.cron_job_id.clone(),
        cron_job_name: card.cron_job_name.clone(),
        changes: card.changes,
        last_activity: card.last_activity,
        pending_work: card.pending_work.clone(),
        background_work: card.background_work.clone(),
        draft_at: card.draft_at,
    }
}

/// Maps a hub event onto `events_open`'s wire event (§7.4).
fn wire_hub_event(event: &HubEvent) -> WireHubEvent {
    match event {
        HubEvent::SessionChanged { card } => WireHubEvent::SessionChanged {
            card: wire_card(card),
        },
        HubEvent::SessionRemoved { session_id } => WireHubEvent::SessionRemoved {
            session: session_id.clone(),
        },
        HubEvent::SessionsRemoved {
            session_ids,
            archived,
        } => WireHubEvent::SessionsRemoved {
            sessions: session_ids.clone(),
            archived: *archived,
        },
        HubEvent::SessionsChanged { cards } => WireHubEvent::SessionsChanged {
            cards: cards.iter().map(wire_card).collect(),
        },
        HubEvent::HostNotice { level, text } => WireHubEvent::HostNotice {
            level: match level {
                super::NoticeLevel::Info => NoticeLevel::Info,
                super::NoticeLevel::Warning => NoticeLevel::Warning,
            },
            text: text.clone(),
        },
        HubEvent::Ui { snapshot, patch } => WireHubEvent::Ui {
            snapshot: snapshot.clone(),
            patch: patch.clone(),
        },
        HubEvent::BoardChanged { project } => WireHubEvent::BoardChanged {
            project: project.clone(),
        },
    }
}

/// Serves one session-layer request (host.md §7.2–7.4): the streams (`session_new` /
/// `session_open` / `session_fork`, `events_open`) and the one-shot session ops.
/// # Errors
/// Returns an error for invalid session requests or session transport and backend failures.
#[allow(
    clippy::too_many_lines,
    reason = "One arm per protocol operation or state transition; keep exhaustive dispatch and resource lifetimes visible together."
)]
pub async fn serve_request<R, W>(
    service: &Arc<SessionService>,
    req: Request,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let hub = Arc::clone(service.hub());
    match req {
        Request::SessionsList { archived } => {
            // `took_ms`: a slow first list (a big sessions folder) is visible in the log
            // instead of "the sidebar took 10 s at startup" (owner-fix-5).
            let started = std::time::Instant::now();
            tracing::info!(op = "sessions_list", archived, "request");
            let cards: SessionsListData = if archived {
                hub.archived().await.map_err(hub_error)?
            } else {
                hub.list().await.map_err(hub_error)?
            }
            .iter()
            .map(wire_card)
            .collect();
            let data = serde_json::to_value(&cards).map_err(internal)?;
            tracing::info!(
                op = "sessions_list",
                archived,
                took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                cards = cards.len(),
                "done"
            );
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionSearch {
            query,
            limit,
            cursor,
            archived,
        } => {
            tracing::info!(op = "session_search", archived, "request");
            let cards: Vec<SessionCard> = if archived {
                hub.archived().await.map_err(hub_error)?
            } else {
                hub.list().await.map_err(hub_error)?
            }
            .iter()
            .map(wire_card)
            .collect();
            let page = unipi_proto::session_search_page(&cards, &query, limit, cursor.as_deref());
            let data = serde_json::to_value(&page).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::MemoryBrowse {
            project,
            query,
            limit,
            cursor,
        } => {
            tracing::info!(op = "memory_browse", "request");
            let root = crate::memory_browse::store_root(hub.agent_dir());
            let page = tokio::task::spawn_blocking(move || {
                crate::memory_browse::browse(
                    &root,
                    &crate::memory_browse::Browse {
                        project: project.as_deref(),
                        query: &query,
                        limit,
                        cursor: cursor.as_deref(),
                    },
                )
            })
            .await
            .map_err(internal)?
            .map_err(|_| {
                AppErrorDto::new(
                    AppErrorCode::E_SCHEMA,
                    "memory_browse: `project` must be one folder name",
                    false,
                )
            })?;
            let data = serde_json::to_value(&page).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::UsageStats {
            days,
            tz_offset_min,
        } => {
            tracing::info!(op = "usage_stats", days, "request");
            let root = hub.sessions_root().to_path_buf();
            let data = tokio::task::spawn_blocking(move || {
                crate::usage_stats::usage_stats(&root, days, tz_offset_min)
            })
            .await
            .map_err(internal)?;
            let data = serde_json::to_value(&data).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::EventsOpen => {
            tracing::info!(op = "events_open", "request");
            pump_events(&hub, read, write).await
        }
        Request::SuperagentSessions => {
            let data = hub.superagent_sessions().await.map_err(hub_error)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SuperagentFleet => {
            let data = hub.superagent_fleet().await.map_err(hub_error)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SuperagentSession => {
            let session = hub.create_superagent().await.map_err(hub_error)?;
            open_stream(service, Arc::new(session), None, read, write).await
        }
        Request::SessionNew { cwd, name } => {
            tracing::info!(op = "session_new", "request");
            let cwd = match cwd {
                Some(path) => {
                    let home = crate::fs::home_dir_opt().ok_or_else(|| {
                        internal("the host has no home directory for this workspace")
                    })?;
                    Some(crate::fs::resolve_request_path(path.as_str(), &home)?)
                }
                None => None,
            };
            let name = name.as_ref().map(unipi_proto::SessionName::as_str);
            // No cwd = "No workspace": the chat's own private scratch folder, never the
            // user's home (scratch.rs).
            let session = match cwd.as_deref() {
                Some(cwd) => hub.create_session(Some(cwd), name).await,
                None => hub.create_scratch_session(name).await,
            }
            .map_err(hub_error)?;
            open_stream(service, Arc::new(session), None, read, write).await
        }
        Request::SessionOpen { session, since_seq } => {
            tracing::info!(
                op = "session_open",
                session = session.as_str(),
                ?since_seq,
                "request"
            );
            let started = std::time::Instant::now();
            if let Some((card, file, page)) =
                if crate::superagent_session::runtime(session.as_str()).is_some() {
                    None
                } else {
                    file_open(&hub, session.as_str()).await
                }
            {
                tracing::info!(
                    session = %card.session_id,
                    entries = page.entries.len(),
                    total = page.page.total,
                    took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                    "session open: tail page read from the file"
                );
                if !crate::superagent_session::is_superagent_file(&file.path) {
                    return open_cold(service, card, file, page, read, write).await;
                }
            }
            let opened = hub.open(session.as_str()).await.map_err(hub_error)?;
            tracing::info!(
                session = %opened.session_id(),
                took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
                "session open: runtime ready"
            );
            open_stream(service, Arc::new(opened), since_seq, read, write).await
        }
        Request::SessionFork {
            session,
            entry_id,
            name,
        } => {
            tracing::info!(op = "session_fork", session = session.as_str(), "request");
            let forked = hub
                .fork(
                    session.as_str(),
                    entry_id.as_deref(),
                    name.as_ref().map(unipi_proto::SessionName::as_str),
                )
                .await
                .map_err(hub_error)?;
            open_stream(service, Arc::new(forked), None, read, write).await
        }
        Request::SessionArchive { session } => {
            tracing::info!(
                op = "session_archive",
                session = session.as_str(),
                "request"
            );
            let card = hub.archive(session.as_str()).await.map_err(hub_error)?;
            // §7.4: the old runtime is gone, so a later open of the same id resets.
            service.feed(&card.session_id).mark_disposed();
            let data = serde_json::to_value(wire_card(&card)).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionRestore { session } => {
            tracing::info!(
                op = "session_restore",
                session = session.as_str(),
                "request"
            );
            let card = hub.restore(session.as_str()).await.map_err(hub_error)?;
            let data = serde_json::to_value(wire_card(&card)).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionDelete { session } => {
            tracing::info!(op = "session_delete", session = session.as_str(), "request");
            hub.delete_archived(session.as_str())
                .await
                .map_err(hub_error)?;
            write_control(write, &Control::Ok { data: None }).await
        }
        Request::TrashEmpty => {
            tracing::info!(op = "trash_empty", "request");
            let data = serde_json::to_value(unipi_proto::session::TrashEmptyData {
                deleted: hub.empty_trash().await.map_err(hub_error)?,
            })
            .map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionsArchive { sessions } => {
            unipi_proto::session_bulk::check_batch(&sessions)
                .map_err(|message| AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false))?;
            tracing::info!(op = "sessions_archive", count = sessions.len(), "request");
            let targets: Vec<String> = sessions.iter().map(|s| s.as_str().to_owned()).collect();
            let data = hub.archive_many(&targets).await.map_err(hub_error)?;
            // §7.4: each old runtime is gone, so a later open of the same id resets.
            for id in &data.done {
                service.feed(id).mark_disposed();
            }
            let data = serde_json::to_value(&data).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionsRestore { sessions } => {
            unipi_proto::session_bulk::check_batch(&sessions)
                .map_err(|message| AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false))?;
            tracing::info!(op = "sessions_restore", count = sessions.len(), "request");
            let targets: Vec<String> = sessions.iter().map(|s| s.as_str().to_owned()).collect();
            let data = hub.restore_many(&targets).await.map_err(hub_error)?;
            let data = serde_json::to_value(&data).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionsQuery {
            query,
            scope,
            filters,
            sort,
            limit,
            cursor,
        } => {
            tracing::info!(op = "sessions_query", ?scope, "request");
            let page = hub
                .query(&super::hub::SessionsQuery {
                    text: query,
                    scope,
                    filters: filters.bounded(),
                    sort,
                    limit,
                    cursor,
                })
                .await
                .map_err(hub_error)?;
            let data = unipi_proto::session_bulk::SessionsQueryData {
                hits: page
                    .hits
                    .iter()
                    .map(|hit| unipi_proto::session_bulk::SessionsQueryHit {
                        card: wire_card(&hit.card),
                        archived: hit.archived,
                        snippet: hit.snippet.clone(),
                    })
                    .collect(),
                total: page.total,
                next_cursor: page.next_cursor,
            };
            let data = serde_json::to_value(&data).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionRename { session, name } => {
            tracing::info!(op = "session_rename", session = session.as_str(), "request");
            let opened = hub.open(session.as_str()).await.map_err(hub_error)?;
            opened
                .send(RpcCommand::Agent(AgentCommand::SetSessionName {
                    name: name.as_str().to_owned(),
                }))
                .await
                .map_err(hub_error)?;
            hub.refresh(&opened.session_id()).await;
            write_control(write, &Control::Ok { data: None }).await
        }
        Request::SessionMove {
            session,
            cwd,
            copy_files,
        } => {
            tracing::info!(op = "session_move", session = session.as_str(), "request");
            let home = crate::fs::home_dir_opt()
                .ok_or_else(|| internal("the host has no home directory for this workspace"))?;
            let cwd = crate::fs::resolve_request_path(cwd.as_str(), &home)?;
            let moved = hub
                .move_to_workspace(session.as_str(), &cwd, copy_files)
                .await
                .map_err(hub_error)?;
            let data = serde_json::to_value(unipi_proto::session::SessionMoveData {
                card: wire_card(&moved.card),
                copied: moved.copied,
                skipped: moved.skipped,
            })
            .map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionClose { session } => {
            tracing::info!(op = "session_close", session = session.as_str(), "request");
            let card = hub.resolve(session.as_str()).await.map_err(hub_error)?;
            hub.close(&card.session_id).await.map_err(hub_error)?;
            service.feed(&card.session_id).mark_disposed();
            write_control(write, &Control::Ok { data: None }).await
        }
        Request::DraftGet { session } => {
            let draft = hub.draft_get(session.as_str()).await.map_err(hub_error)?;
            write_control(
                write,
                &Control::Ok {
                    data: Some(json!({ "draft": draft })),
                },
            )
            .await
        }
        Request::DraftSet { session, draft } => {
            tracing::debug!(
                op = "draft_set",
                session = session.as_str(),
                clear = draft.is_none(),
                "request"
            );
            let card = hub
                .draft_set(session.as_str(), draft)
                .await
                .map_err(hub_error)?;
            let data = serde_json::to_value(wire_card(&card)).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        Request::SessionMetaSet { session, patch } => {
            tracing::info!(
                op = "session_meta_set",
                session = session.as_str(),
                "request"
            );
            let card = hub
                .meta_set(session.as_str(), patch)
                .await
                .map_err(hub_error)?;
            let data = serde_json::to_value(wire_card(&card)).map_err(internal)?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        // App spec §3.14 (WP-A24): hub-level `_oino/*` subset, no session runtime.
        Request::HostCmd { cmd } => {
            let data = crate::host_cmd::run(&hub, cmd).await?;
            write_control(write, &Control::Ok { data: Some(data) }).await
        }
        _ => Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "not a session-layer request",
            false,
        )),
    }
}

/// Writes one JSON control frame (the `ok` reply, an `events` batch or a bare hub event).
async fn write_json<W>(write: &mut W, value: &impl serde::Serialize) -> Result<(), AppErrorDto>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    use tokio::io::AsyncWriteExt;
    let started = std::time::Instant::now();
    let mut bytes = serde_json::to_vec(value).map_err(internal)?;
    tracing::debug!(
        phase = "serialize",
        elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        bytes = bytes.len(),
        "control frame phase"
    );
    if bytes.len() > unipi_proto::frame::MAX_FRAME {
        // fix-frames: never put an oversize frame on the wire (the app cannot read it). A
        // correlated reply becomes its error; anything else is dropped (the app resyncs on
        // the seq gap).
        let Some(replacement) = oversize_replacement(&bytes) else {
            tracing::error!(bytes = bytes.len(), "dropping an oversize control frame");
            return Ok(());
        };
        tracing::error!(bytes = bytes.len(), "oversize reply replaced by an error");
        bytes = serde_json::to_vec(&replacement).map_err(internal)?;
    }
    // A caller-supplied correlation id can itself exceed the budget. The final
    // serialized replacement must pass the same guard as every other frame.
    if bytes.len() > unipi_proto::frame::MAX_FRAME {
        bytes = br#"{"type":"err","error":{"code":"E_INTERNAL","message":"Frame too large","retryable":false}}"#.to_vec();
    }
    let started = std::time::Instant::now();
    write
        .write_all(&unipi_proto::frame::encode(
            unipi_proto::frame::FrameKind::Control,
            &bytes,
        ))
        .await
        .map_err(internal)?;
    write.flush().await.map_err(internal)?;
    tracing::debug!(
        phase = "send",
        elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        "control frame phase"
    );
    Ok(())
}

/// The error that stands in for an oversize `ok` reply: the same `id` (a session command's)
/// or the bare hello reply's `err`. `None` for anything else (events, hub events).
fn oversize_replacement(bytes: &[u8]) -> Option<Value> {
    let mut value: Value = serde_json::from_slice(bytes).ok()?;
    // Last-resort serialized-envelope guard, independent of the shape and of
    // upstream per-field estimators. Retain entry identities for full refetch.
    if let Some(entries) = value
        .pointer_mut("/data/entries")
        .and_then(Value::as_array_mut)
    {
        for entry in entries {
            let id = entry
                .get("id")
                .filter(|id| bound::json_len(*id) <= 1024)
                .cloned();
            *entry = json!({"truncated":true});
            if let Some(id) = id {
                entry["id"] = id;
            }
        }
        if bound::json_len(&value) <= unipi_proto::frame::MAX_FRAME {
            return Some(value);
        }
    }
    if value.get("type").and_then(Value::as_str) != Some("ok") {
        return None;
    }
    let error = AppErrorDto::new(
        AppErrorCode::E_INTERNAL,
        "The answer was too large to send",
        true,
    );
    match value.get("id") {
        Some(id) => serde_json::to_value(SessionReply::failed(Some(id.clone()), error)).ok(),
        None => serde_json::to_value(Control::Err { error }).ok(),
    }
}

async fn write_control<W>(write: &mut W, control: &Control) -> Result<(), AppErrorDto>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    write_json(write, control).await
}

/// `events_open` (§7.4): the retained notices first (the safe-mode banner fires before any
/// client connects), then every hub event until the connection goes away. The connection
/// reports its focused session with `Control::Focus` frames (app spec §3.14, WP-A4): the
/// host counts it for Z-86's unread rule and drops it when the stream ends.
// RAII releases both sources and forwarders on every exit, including write failures.
struct Topics(std::collections::HashMap<String, tokio::task::JoinHandle<()>>);
impl Topics {
    fn remove(&mut self, topic: &str) {
        if let Some(task) = self.0.remove(topic) {
            task.abort();
            crate::topic_hub::TopicHub::shared().detach(topic);
        }
    }
}
impl Drop for Topics {
    fn drop(&mut self) {
        for (topic, task) in self.0.drain() {
            task.abort();
            crate::topic_hub::TopicHub::shared().detach(&topic);
        }
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "One select loop owns all client subscriptions and RAII guards; splitting would obscure cancellation and teardown order."
)]
async fn pump_events<R, W>(
    hub: &Arc<SessionHub>,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    write_control(write, &Control::Ok { data: None }).await?;
    write_json(
        write,
        &WireHubEvent::Hello {
            capabilities: vec![
                unipi_proto::TOPICS_CAPABILITY.into(),
                "topic:sessions".into(),
                "topic:board".into(),
                "topic:git".into(),
                "topic:fs".into(),
            ],
        },
    )
    .await?;
    // Subscribe before taking retained state: a concurrent state change must be
    // either in the snapshot or queued on this receiver (duplicates are harmless).
    let mut events = hub.subscribe();
    for notice in hub.notices() {
        write_json(write, &wire_hub_event(&notice)).await?;
    }
    // Reconnects must see startup recovery even though it preceded subscription.
    for card in hub.recovered_run_cards().await {
        write_json(
            write,
            &WireHubEvent::SessionChanged {
                card: wire_card(&card),
            },
        )
        .await?;
    }
    let (topic_tx, mut topic_rx) = tokio::sync::mpsc::channel::<WireHubEvent>(128);
    let mut topics = Topics(std::collections::HashMap::new());
    let focus = hub.focus_connection();
    // WP-A15 (app.md §3.14): the git watcher rides this stream too — a
    // `GitChanged{root}` per repo change, after the watcher's debounce.
    let mut git = crate::git_watch::GitWatchers::shared().subscribe();
    loop {
        tokio::select! {
            Some(event) = topic_rx.recv() => write_json(write, &event).await?,
            event = events.recv() => match event {
                Ok(event) => write_json(write, &wire_hub_event(&event)).await?,
                // This connection fell behind and events were dropped (a burst of card
                // changes): tell the app to re-list instead of leaving it stale.
                Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
                    tracing::warn!(skipped, "events_open lagged: asking the app to resync");
                    write_json(write, &WireHubEvent::Resync).await?;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    hub.release_focus(focus).await;
                    return Ok(());
                }
            },
            changed = git.recv() => match changed {
                Ok(changed) => write_json(
                    write,
                    &WireHubEvent::GitChanged {
                        root: changed.root,
                    },
                )
                .await?,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                // The global watcher registry outlives every connection; a
                // closed channel means the host is shutting down.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    hub.release_focus(focus).await;
                    return Ok(());
                }
            },
            frame = crate::protocol::read_frame(read) => {
                let Ok(frame) = frame else {
                    hub.release_focus(focus).await;
                    return Ok(());
                };
                if frame.0 != 0 {
                    continue;
                }
                match serde_json::from_slice::<Control>(&frame.1) {
                    Ok(Control::Focus { session }) => hub.set_focus(focus, session.as_deref()).await,
                    Ok(Control::Unsubscribe { topics: names }) => {
                        for name in names { topics.remove(&name); }
                    }
                    Ok(Control::Subscribe { topics: requested }) => {
                        for sub in requested.into_iter().take(128) {
                            if !crate::topic_sources::supported(&sub.topic) { continue; }
                            if !topics.0.contains_key(&sub.topic) && topics.0.len() >= 128 { continue; }
                            topics.remove(&sub.topic);
                            let name = sub.topic;
                            let topic_hub = crate::topic_hub::TopicHub::shared();
                            let start_name = name.clone();
                            let (mut rx, resume) = tokio::task::spawn_blocking(move || {
                                let hub = crate::topic_hub::TopicHub::shared();
                                hub.attach(&start_name, sub.since, || crate::topic_sources::start(&start_name))
                            }).await.map_err(internal)?;
                            // Own the hub ref before any fallible write below.
                            topics.0.insert(name.clone(), tokio::spawn(async {}));
                            if name != "sessions" && !topic_hub.source_running(&name) {
                                let cursor = topic_hub.cursor(&name).expect("attached topic");
                                write_json(write, &WireHubEvent::Topic {
                                    topic: name.clone(), epoch: cursor.epoch, seq: cursor.seq,
                                    snapshot: Some(json!({"status": "error", "error": "watcher_start_failed"})), delta: None,
                                }).await?;
                                continue;
                            }
                            match resume {
                                crate::topic_hub::Resume::Snapshot { epoch, seq } => {
                                    // An invalidation snapshot deliberately uses existing authorized
                                    // bounded read ops, rather than duplicating their data here.
                                    write_json(write, &WireHubEvent::Topic {
                                        topic: name.clone(), epoch, seq,
                                        snapshot: Some(json!({"invalidate": true})), delta: None,
                                    }).await?;
                                }
                                crate::topic_hub::Resume::Replay { epoch, deltas } => {
                                    for (seq, delta) in deltas {
                                        write_json(write, &WireHubEvent::Topic {
                                            topic: name.clone(), epoch, seq, snapshot: None, delta: Some(delta),
                                        }).await?;
                                    }
                                }
                            }
                            let tx = topic_tx.clone();
                            let topic = name.clone();
                            let task = tokio::spawn(async move {
                                loop {
                                    let frame = match rx.recv().await {
                                        Ok(d) => WireHubEvent::Topic { topic: topic.clone(), epoch: d.epoch,
                                            seq: d.seq, snapshot: None, delta: Some(d.delta) },
                                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                                            let Some(cursor) = topic_hub.cursor(&topic) else { break; };
                                            WireHubEvent::Topic { topic: topic.clone(), epoch: cursor.epoch,
                                                seq: cursor.seq, snapshot: Some(json!({"invalidate": true})), delta: None }
                                        }
                                        Err(_) => break,
                                    };
                                    if tx.send(frame).await.is_err() { break; }
                                }
                            });
                            topics.0.insert(name, task);
                        }
                    }
                    _ => {}
                }
                // Anything else on an events stream is meaningless: ignore it.
            }
        }
    }
}

/// Opens one session stream: tap on, subscribe **before** the reply is built (so no event
/// can fall between the snapshot and the first batch), then snapshot or resume (§7.3–7.4).
async fn open_stream<R, W>(
    service: &Arc<SessionService>,
    session: Arc<HubSession>,
    since_seq: Option<u64>,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let feed = service.feed(&session.session_id());
    feed.ensure_tap(Arc::clone(session.backend()), Arc::downgrade(service));
    let subscription = feed.subscribe();
    let restore_error = restore_queue(&session, &feed).await.err();
    // §8.2: an open that took the session over from another runner is a new runtime here, so
    // the previous stream's numbering is not resumable (the other runner wrote in between).
    let taken_over = session.taken_over().map(str::to_owned);
    let mut data = match since_seq {
        Some(since)
            if taken_over.is_none()
                && feed.can_resume(since)
                && replay_fits(&feed.events_after(since)) =>
        {
            let replay = feed.events_after(since);
            let seq = replay.last().map_or(since, |event| event.seq);
            tracing::debug!(session = %feed.session_id(), since, seq, "session resume");
            SessionSnapshot::resume(session.session_id(), seq, replay)
        }
        _ => snapshot(&session, &feed, Arc::downgrade(service)).await?,
    };
    // Read waiters AFTER the snapshot watermark: a request created during the
    // transcript read must not be covered by seq without appearing in the snapshot.
    // Later transitions are also delivered through the already-active subscription.
    let ui = send_data(
        &session,
        RpcCommand::Oino(oino_client::protocol::OinoCommand::UiSnapshot),
    )
    .await?;
    data.pending_requests = field_array(ui.as_ref(), "pending_requests");
    if let Some(error) = restore_error {
        // The initial notice may be covered by the snapshot seq. Re-publish after
        // the snapshot without attempting restoration a second time.
        let queue = oino_core::session::durable_queue::DurableQueue::new(&session.card().file);
        feed.publish(json!({"kind": "queue_restore_failed", "items": queue.recover_quarantined(), "error": error})).await;
    }
    if let Some(from) = taken_over {
        // §8.2: the first session event after the snapshot tells the app where it came from.
        feed.publish(json!({"kind": "session_acquired", "from": from}))
            .await;
    }
    let file = session_file(service.hub(), session.card());
    serve_stream(
        feed,
        Live::Ready(session),
        file,
        subscription,
        data,
        Arc::downgrade(service),
        read,
        write,
    )
    .await
}

/// The card's session file, when it exists (a fresh chat has none until its first reply).
fn session_file(hub: &SessionHub, card: &HostCard) -> Option<SessionFile> {
    card.file
        .is_file()
        .then(|| SessionFile::new(&card.file, hub.sessions_root()))
}

/// perf-open: a session that is not live here opens from its file — the tail page through
/// the offset index — instead of waking first. `None` = take the runtime path (live
/// already, unknown target, a pre-v3 or missing file).
async fn file_open(hub: &Arc<SessionHub>, target: &str) -> Option<(HostCard, SessionFile, Page)> {
    let started = std::time::Instant::now();
    let card = hub.resolve_quick(target).await.ok()?;
    tracing::info!(
        phase = "resolve",
        elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        "session open phase"
    );
    // A live runtime answers its own snapshot — unless another session's wake holds the
    // hub lock (the runtime path would wait for it): then the file serves this one too
    // (a live idle session's file is current) and the "wake" just reattaches.
    // A running turn always takes the runtime path (its in-flight message is not in the
    // file yet).
    if hub.is_live(&card.session_id).await && (!hub.core_busy() || card.state.is_working()) {
        return None;
    }
    let file = session_file(hub, &card)?;
    let reader = file.clone();
    let page = tokio::task::spawn_blocking(move || reader.page(None, fast::PAGE_LIMIT))
        .await
        .ok()?
        .map_err(|error| tracing::info!(%error, "session open: no file page, waking first"))
        .ok()?;
    Some((card, file, page))
}

/// Serves an open read from the file (`waking` snapshot) and wakes the runtime behind it.
async fn open_cold<R, W>(
    service: &Arc<SessionService>,
    card: HostCard,
    file: SessionFile,
    mut page: Page,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let hub = Arc::clone(service.hub());
    let feed = service.feed(&card.session_id);
    // Subscribe before the snapshot's seq is read: `woken` and every runtime event after
    // it are numbered above the snapshot and reach this stream.
    let subscription = feed.subscribe();
    let blobs_dir = hub.blobs_dir();
    page.bound(Some(&blobs_dir));
    let mut snapshot = SessionSnapshot::full(card.session_id.clone(), feed.seq());
    // owner-fix-6 P1-2: a cron run still in flight owns this chat (its `oino -p`
    // subprocess holds the lease): the open joins it — streaming, followed from the file —
    // and wakes only once the run is over, never taking the session over mid-run.
    let follow = hub
        .cron_run_in_flight(&card)
        .then(|| (card.clone(), file.clone()));
    let mut provisional = fast::provisional_state(&card, &page.entries);
    if follow.is_some() {
        provisional["isStreaming"] = json!(true);
    }
    snapshot.project_trust = Some(
        oino_resources::trust::project_trust_snapshot(
            std::path::Path::new(&card.cwd),
            hub.agent_dir(),
        )
        .map_err(internal)?,
    );
    snapshot.state = Some(provisional);
    snapshot.leaf_id = page.leaf_id.clone().map(Value::String);
    snapshot.entries = Some(page.entries);
    snapshot.page = Some(page.page);
    snapshot.pending_requests = Some(Vec::new());
    snapshot.waking = true;
    snapshot.validate().map_err(internal)?;
    let (tx, rx) = tokio::sync::watch::channel(None);
    tokio::spawn(wake_behind(
        Arc::downgrade(service),
        card.session_id.clone(),
        Arc::clone(&feed),
        tx,
        follow,
    ));
    serve_stream(
        feed,
        Live::Waking(rx),
        Some(file),
        subscription,
        snapshot,
        Arc::downgrade(service),
        read,
        write,
    )
    .await
}

/// The background wake of a file-opened session: the runtime comes up (takeover and
/// crash disposal included, [`SessionHub::open`]), its tap starts, and `{kind:"woken"}`
/// carries what the provisional snapshot lacked — or `{kind:"wake_failed", error}`.
async fn wake_behind(
    service: Weak<SessionService>,
    session_id: String,
    feed: Arc<Feed>,
    tx: tokio::sync::watch::Sender<Option<Result<Arc<HubSession>, String>>>,
    follow: Option<(HostCard, SessionFile)>,
) {
    let Some(strong) = service.upgrade() else {
        let _ = tx.send(Some(Err("the host is shutting down".to_owned())));
        return;
    };
    let hub = Arc::clone(strong.hub());
    drop(strong);
    if let Some((card, file)) = follow {
        if !follow_cron_run(&hub, &card, &file, &feed).await {
            let _ = tx.send(Some(Err("the chat was closed".to_owned())));
            return;
        }
    }
    let started = std::time::Instant::now();
    let outcome = match hub.open(&session_id).await {
        Ok(session) => {
            let session = Arc::new(session);
            feed.ensure_tap(Arc::clone(session.backend()), service.clone());
            let _ = restore_queue(&session, &feed).await;
            match runtime_parts(&session).await {
                Ok(mut parts) => {
                    if let Some(topics) = parts.topics.as_mut() {
                        let blobs_dir = service.upgrade().map(|service| service.hub().blobs_dir());
                        let _ = bound::bound_topics(topics, blobs_dir.as_deref());
                    }
                    // §8.2: where the session came from is the first event after the
                    // snapshot, as on the runtime path.
                    if let Some(from) = session.taken_over() {
                        feed.publish(json!({"kind": "session_acquired", "from": from}))
                            .await;
                    }
                    let leaf = session
                        .send(RpcCommand::Oino(
                            oino_client::protocol::OinoCommand::EntriesPage {
                                before: None,
                                limit: Some(1),
                                ids: None,
                            },
                        ))
                        .await
                        .ok()
                        .flatten()
                        .and_then(|reply| reply.get("leafId").cloned());
                    feed.publish(json!({
                        "kind": "woken",
                        "state": parts.state,
                        "commands": parts.commands,
                        "ui": parts.ui,
                        "topics": parts.topics,
                        "leafId": leaf,
                    }))
                    .await;
                    Ok(session)
                }
                Err(error) => Err(error.message.to_string()),
            }
        }
        Err(error) => Err(error.to_string()),
    };
    match &outcome {
        Ok(_) => tracing::info!(
            session = %session_id,
            took_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
            "session open: runtime woke behind the history"
        ),
        Err(error) => {
            tracing::warn!(session = %session_id, %error, "session open: wake failed");
            feed.publish(json!({"kind": "wake_failed", "error": error}))
                .await;
        }
    }
    let _ = tx.send(Some(outcome));
}

/// How often an open follows a running cron run's file.
const FOLLOW_POLL: std::time::Duration = std::time::Duration::from_millis(400);
/// How long a run record may still say `running` with nobody holding the session before
/// the open wakes it anyway (a run whose process died without a final record).
const FOLLOW_STALE: std::time::Duration = std::time::Duration::from_secs(15);

/// owner-fix-6 P1-2: follows the file of a chat a cron run is writing (its `oino -p`
/// subprocess holds the lease) until the run is over and the session free — each new leaf
/// goes out as `session_tree` (the app reloads its tail page from the file) — so the wake
/// after it never takes the session over mid-run. `false` = every stream on the chat went
/// away (nothing to wake for).
async fn follow_cron_run(
    hub: &Arc<SessionHub>,
    card: &HostCard,
    file: &SessionFile,
    feed: &Arc<Feed>,
) -> bool {
    let leaf_of = |file: &SessionFile| {
        let reader = file.clone();
        async move {
            tokio::task::spawn_blocking(move || reader.page(None, 1).ok().and_then(|p| p.leaf_id))
                .await
                .ok()
                .flatten()
        }
    };
    tracing::info!(
        session = %card.session_id,
        "session open: a cron run is writing this chat; following its file until it ends"
    );
    let mut leaf = leaf_of(file).await;
    let mut free_since: Option<std::time::Instant> = None;
    loop {
        tokio::time::sleep(FOLLOW_POLL).await;
        if feed.subscriber_count() == 0 {
            return false;
        }
        let now = leaf_of(file).await;
        if now.is_some() && now != leaf {
            feed.publish(json!({"type": "session_tree", "leafId": now}))
                .await;
            leaf = now;
        }
        if SessionHub::lease_held_elsewhere(&card.file) {
            free_since = None;
            continue;
        }
        if !hub.cron_run_in_flight(card) {
            return true;
        }
        if free_since
            .get_or_insert_with(std::time::Instant::now)
            .elapsed()
            >= FOLLOW_STALE
        {
            return true;
        }
    }
}

/// The full snapshot of §7.3: `state`, the current branch's `entries` + `leaf_id`,
/// `commands`, the registry `ui` snapshot and the sticky `topics`.
///
/// `_oino/hello` goes first (the host sends the app's `UiCapabilities` implicitly), and the
/// snapshot's `seq` is read **after** the data, so every event it does not contain is
/// numbered above it and reaches the client through its subscription.
async fn snapshot(
    session: &Arc<HubSession>,
    feed: &Arc<Feed>,
    service: Weak<SessionService>,
) -> Result<SessionSnapshot, AppErrorDto> {
    let parts = runtime_parts(session).await?;
    // perf-open: the tail page of the branch, never the whole history (`_oino/entries_page`;
    // a backend without it answers the whole list, bounded and cut to a page below).
    let page = match session
        .send(RpcCommand::Oino(
            oino_client::protocol::OinoCommand::EntriesPage {
                before: None,
                limit: Some(fast::PAGE_LIMIT),
                ids: None,
            },
        ))
        .await
    {
        Ok(Some(reply)) => Page::from_reply(&reply),
        _ => None,
    };
    let mut page = if let Some(page) = page {
        page
    } else {
        let entries = send_data(
            session,
            RpcCommand::Agent(AgentCommand::GetEntries { since: None }),
        )
        .await?;
        let leaf_id = entries
            .as_ref()
            .and_then(|value| value.get("leafId"))
            .and_then(Value::as_str)
            .map(str::to_owned);
        let entries = field_array(entries.as_ref(), "entries").unwrap_or_default();
        Page {
            page: unipi_proto::session::HistoryPage {
                total: entries.len() as u64,
                has_more: false,
                cursor: entries
                    .first()
                    .and_then(|entry| entry.get("id"))
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            },
            leaf_id,
            entries,
        }
    };
    // v2 QA: a session bigger than the 1 MiB frame must still open — big text is
    // truncated (flagged for the app's full-output refetch), big inline images
    // become blob `mediaRef`s the stream's `media` command serves; a page still too big
    // starts later (its older entries load with the next page).
    let blobs_dir = service.upgrade().map(|service| service.hub().blobs_dir());
    page.bound(blobs_dir.as_deref());
    let mut snapshot = SessionSnapshot::full(session.session_id(), feed.seq());
    snapshot.state = parts.state;
    snapshot.leaf_id = page.leaf_id.map(Value::String);
    snapshot.entries = Some(page.entries);
    snapshot.page = Some(page.page);
    snapshot.commands = parts.commands;
    snapshot.ui = parts.ui;
    snapshot.topics = parts.topics;
    if let Some(topics) = snapshot.topics.as_mut() {
        let _ = bound::bound_topics(topics, blobs_dir.as_deref());
    }
    snapshot.validate().map_err(internal)?;
    Ok(snapshot)
}

/// What a snapshot needs from the runtime besides entries.
struct RuntimeParts {
    state: Option<Value>,
    commands: Option<Vec<Value>>,
    ui: Option<Value>,
    topics: Option<Vec<Value>>,
}

/// `_oino/hello` first (the host sends the app's `UiCapabilities` implicitly), then
/// `get_state`, `get_commands`, the sticky topics and the ui registry snapshot.
async fn runtime_parts(session: &Arc<HubSession>) -> Result<RuntimeParts, AppErrorDto> {
    session
        .send(RpcCommand::Oino(
            oino_client::protocol::OinoCommand::Hello {
                client: oino_client::protocol::ClientInfo {
                    name: "unipi-app".to_owned(),
                    version: unipi_proto::PROTO_VERSION.to_owned(),
                },
                ui: Some(serde_json::to_value(UiCapabilities::full()).map_err(internal)?),
                protocol: 1,
            },
        ))
        .await
        .map_err(hub_error)?;
    let state = send_data(session, RpcCommand::Agent(AgentCommand::GetState)).await?;
    let commands = send_data(session, RpcCommand::Agent(AgentCommand::GetCommands)).await?;
    let topics = send_data(
        session,
        RpcCommand::Oino(oino_client::protocol::OinoCommand::Topics {
            subscribe: Vec::new(),
            unsubscribe: Vec::new(),
        }),
    )
    .await?;
    let ui = send_data(
        session,
        RpcCommand::Oino(oino_client::protocol::OinoCommand::UiSnapshot),
    )
    .await?;
    Ok(RuntimeParts {
        state,
        commands: field_array(commands.as_ref(), "commands"),
        ui,
        topics: field_array(topics.as_ref(), "topics"),
    })
}

fn field_array(value: Option<&Value>, field: &str) -> Option<Vec<Value>> {
    value?.get(field).and_then(Value::as_array).cloned()
}

async fn send_data(
    session: &Arc<HubSession>,
    command: RpcCommand,
) -> Result<Option<Value>, AppErrorDto> {
    session.send(command).await.map_err(hub_error)
}

/// Pumps one open session stream until the client goes away (§7.3): batches events out,
/// takes commands in, answers them in order (events first, replies after).
#[allow(clippy::too_many_arguments)]
#[allow(
    clippy::too_many_lines,
    reason = "One arm per protocol operation or state transition; keep exhaustive dispatch and resource lifetimes visible together."
)]
async fn serve_stream<R, W>(
    feed: Arc<Feed>,
    session: Live,
    file: Option<SessionFile>,
    mut subscription: Subscription,
    data: SessionSnapshot,
    service: Weak<SessionService>,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    let started = std::time::Instant::now();
    let mut data = data;
    if let Some(strong) = service.upgrade() {
        let cwd = match &session {
            Live::Ready(session) => session.card().cwd.clone(),
            Live::Waking(_) => String::new(),
        };
        if !cwd.is_empty() {
            data.project_trust = Some(
                oino_resources::trust::project_trust_snapshot(
                    std::path::Path::new(&cwd),
                    strong.hub().agent_dir(),
                )
                .map_err(internal)?,
            );
        }
    }
    let reply = serde_json::to_value(&data).map_err(internal)?;
    tracing::info!(
        phase = "snapshot_value",
        elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        "session open phase"
    );
    let started = std::time::Instant::now();
    write_control(write, &Control::Ok { data: Some(reply) }).await?;
    tracing::info!(
        phase = "serialize_send",
        elapsed_us = u64::try_from(started.elapsed().as_micros()).unwrap_or(u64::MAX),
        "session open phase"
    );
    if let Some(file) = &file {
        oino_session::manager::OffsetIndex::persist_after_reply(&file.path);
    }
    // Everything up to `data.seq` is in the snapshot (or the replay): the subscription may
    // still hold some of it, and applying it twice must not happen.
    let mut seen_through = data.seq;
    let mut batch: Vec<SessionEventDto> = Vec::new();
    let mut flush_at: Option<tokio::time::Instant> = None;
    let (replies, mut answered) = mpsc::channel::<SessionReply>(16);
    let mut model_catalogue: Option<Value> = None;
    loop {
        let deadline =
            flush_at.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            received = subscription.recv() => {
                let Some(event) = received else { return Ok(()) };
                if event.seq <= seen_through {
                    continue;
                }
                let delta = event.event.is_message_update() && is_delta(&event.event);
                if let Some(last) = batch.last_mut() {
                    if merge_delta(last, &event) {
                        if delta && flush_at.is_none() {
                            flush_at = Some(tokio::time::Instant::now() + FLUSH_WINDOW);
                        }
                        continue;
                    }
                }
                batch.push((*event).clone());
                if delta {
                    flush_at.get_or_insert_with(|| tokio::time::Instant::now() + FLUSH_WINDOW);
                } else {
                    // A non-delta event flushes at once: it is never held back (§7.4).
                    flush_batch(write, &mut batch, &mut seen_through).await?;
                    flush_at = None;
                }
            }
            () = tokio::time::sleep_until(deadline) => {
                flush_batch(write, &mut batch, &mut seen_through).await?;
                flush_at = None;
            }
            reply = answered.recv() => {
                let Some(reply) = reply else { continue };
                // §7.3 ordering: whatever the command caused and the stream already holds
                // goes out before its reply.
                flush_batch(write, &mut batch, &mut seen_through).await?;
                flush_at = None;
                for event in subscription.drain() {
                    if event.seq > seen_through {
                        batch.push((*event).clone());
                    }
                }
                flush_batch(write, &mut batch, &mut seen_through).await?;
                write_json(write, &reply).await?;
            }
            frame = crate::protocol::read_frame(read) => {
                let Ok((kind, bytes)) = frame else { return Ok(()) };
                if kind != 0 {
                    continue;
                }
                match serde_json::from_slice::<Control>(&bytes) {
                    Ok(Control::Exit { .. }) => return Ok(()),
                    Ok(Control::SessionCmd { id, cmd }) => {
                        // The app's image fetch (`sessionRefChat.media`): a bounded
                        // snapshot's `mediaRef` images load through this command, from
                        // the same blob store the reference points into.
                        if cmd.get("type").and_then(Value::as_str) == Some("media") {
                            let reply = media_reply(service.upgrade().as_deref(), &cmd);
                            flush_batch(write, &mut batch, &mut seen_through).await?;
                            flush_at = None;
                            write_json(write, &SessionReply::ok(id, Some(reply))).await?;
                            continue;
                        }
                        // E-19/F-02: an in-chat model pick is per-chat; the catalogue rows
                        // carry the hub's per-provider readiness. Both are app-path
                        // semantics, applied at this boundary only (the TUI/CLI keep their
                        // behaviour).
                        let mut cmd = app_model_semantics(cmd);
                        if let Err(message) = resolve_prompt_images(&mut cmd, service.upgrade().map(|s| s.hub().blobs_dir()).as_deref()) {
                            let error = AppErrorDto::new(AppErrorCode::E_SCHEMA, message, false);
                            write_json(write, &SessionReply::failed(id, error)).await?;
                            continue;
                        }
                        // The picker is a cache read even on a live chat. Do not send it
                        // through the engine command queue or refresh providers on this path.
                        if cmd.get("type").and_then(Value::as_str) == Some("get_available_models") {
                            if let Some(strong) = service.upgrade() {
                                if model_catalogue.is_none() {
                                    let dir = strong.hub().agent_dir().to_path_buf();
                                    model_catalogue = tokio::task::spawn_blocking(move || record_model::cached_catalogue(&dir))
                                        .await.ok().flatten().map(|models| json!({"models": models}));
                                }
                                if let Some(mut data) = model_catalogue.clone() {
                                    enrich_models_reply(&strong.hub().readiness().snapshot(), &mut data);
                                    write_json(write, &SessionReply::ok(id, Some(data))).await?;
                                    continue;
                                }
                            }
                        }
                        // v3-perf §4.5 (T6/T7): while the runtime is not up, the picker's
                        // catalogue comes from the on-disk snapshot and a chat-only pick is
                        // appended to the record — neither waits for the wake. A live runtime
                        // (or a pick the record path cannot serve) takes the runtime path.
                        if session.ready().is_none() {
                            if let (Some(strong), Some(_file)) = (service.upgrade(), file.as_ref()) {
                                let agent_dir = strong.hub().agent_dir().to_path_buf();
                                let ty = cmd.get("type").and_then(Value::as_str).unwrap_or("");
                                let reply = if ty == "get_available_models" {
                                    let dir = agent_dir.clone();
                                    tokio::task::spawn_blocking(move || record_model::cached_catalogue(&dir))
                                        .await
                                        .ok()
                                        .flatten()
                                        .map(|rows| {
                                            let mut data = json!({ "models": rows });
                                            enrich_models_reply(&strong.hub().readiness().snapshot(), &mut data);
                                            data
                                        })
                                } else {
                                    None
                                };
                                if let Some(data) = reply {
                                    // Through the reply channel: §7.3 ordering puts the
                                    // `model_changed` event before its reply.
                                    if let Err(error) = replies.try_send(SessionReply::ok(id, Some(data))) {
                                        let reply = match error {
                                            mpsc::error::TrySendError::Full(r) | mpsc::error::TrySendError::Closed(r) => r,
                                        };
                                        write_json(write, &reply).await?;
                                    }
                                    continue;
                                }
                            }
                        }
                        // E-04: a dialog answer (`extension_ui_response`) is not an RPC
                        // command — it rides the same stream as one and must reach the
                        // waiting tool instead of dying as a schema rejection (the
                        // approval card's "Allow once" then never ran and the turn hung
                        // on "Working…" until restart).
                        if cmd.get("type").and_then(Value::as_str) == Some("extension_ui_response")
                        {
                            let reply = match serde_json::from_value::<UiResponse>(cmd) {
                                Ok(response) => match session.ready() {
                                    Some(live) => {
                                        live.backend().ui_respond(response);
                                        SessionReply::ok(id, None)
                                    }
                                    // No runtime yet: nothing can be waiting for an answer.
                                    None => SessionReply::failed(
                                        id,
                                        AppErrorDto::new(
                                            AppErrorCode::E_UNSUPPORTED,
                                            "The chat is still starting",
                                            true,
                                        ),
                                    ),
                                },
                                Err(error) => SessionReply::failed(
                                    id,
                                    AppErrorDto::new(AppErrorCode::E_SCHEMA, error.to_string(), false),
                                ),
                            };
                            flush_batch(write, &mut batch, &mut seen_through).await?;
                            flush_at = None;
                            write_json(write, &reply).await?;
                            continue;
                        }
                        // perf-open: "load earlier" (`before`/`limit`) and the full-output
                        // refetch (`ids`) are pages, served bounded — never core's
                        // whole-history `get_entries`.
                        if let Some((before, limit, ids)) = fast::paged_request(&cmd) {
                            let blobs_dir = service.upgrade().map(|service| service.hub().blobs_dir());
                            tokio::spawn(answer_page(
                                session.clone(),
                                file.clone(),
                                blobs_dir,
                                PageAsk { before, limit, ids, chunk_offset: cmd.get("chunkOffset").and_then(Value::as_u64).and_then(|n| usize::try_from(n).ok()) },
                                id,
                                replies.clone(),
                            ));
                            continue;
                        }
                        let retry_id = cmd.get("retryMessageId").and_then(Value::as_str)
                            .filter(|id| uuid::Uuid::parse_str(id).is_ok()).map(str::to_owned);
                        match RpcRequest::from_value(cmd) {
                            Ok(request) => {
                                dispatch(&service.upgrade().expect("service gone"), session.clone(), Arc::clone(&feed), file.clone(), id, request.command, retry_id, replies.clone());
                            }
                            Err(rejected) => {
                                let error = AppErrorDto::new(
                                    AppErrorCode::E_SCHEMA,
                                    rejected.error,
                                    false,
                                );
                                write_json(write, &SessionReply::failed(id, error)).await?;
                            }
                        }
                    }
                    // Anything else on a session stream is meaningless: ignore it.
                    Ok(_) | Err(_) => {}
                }
            }
        }
    }
}

/// Writes one `events` frame for whatever is batched (§7.4: one frame per flush tick).
async fn flush_batch<W>(
    write: &mut W,
    batch: &mut Vec<SessionEventDto>,
    seen_through: &mut u64,
) -> Result<(), AppErrorDto>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if batch.is_empty() {
        return Ok(());
    }
    let items = std::mem::take(batch);
    *seen_through = items
        .last()
        .map_or(*seen_through, |event| event.seq.max(*seen_through));
    for items in split_batch(items) {
        write_control(write, &Control::Events { items }).await?;
    }
    Ok(())
}

/// A resume replay small enough for the open reply's frame; a bigger one reopens with a
/// (bounded) full snapshot instead.
fn replay_fits(replay: &[SessionEventDto]) -> bool {
    replay
        .iter()
        .map(|event| bound::json_len(&event.event) + 96)
        .sum::<usize>()
        <= bound::BATCH_BUDGET
}

/// Splits one flush's events into frames of at most [`bound::BATCH_BUDGET`] (order kept,
/// every event in exactly one frame). An event still too big for a frame on its own (a
/// coalesced delta that grew) is bounded again — to a stub if need be — never sent oversize.
fn split_batch(items: Vec<SessionEventDto>) -> Vec<Vec<SessionEventDto>> {
    let mut frames: Vec<Vec<SessionEventDto>> = Vec::new();
    let mut current: Vec<SessionEventDto> = Vec::new();
    let mut size = 0usize;
    for mut item in items {
        let mut len = bound::json_len(&item.event) + 96 + item.session.len();
        if len > bound::BATCH_BUDGET {
            let _ = bound::bound_event(&mut item.event, None, bound::EVENT_BUDGET);
            len = bound::json_len(&item.event) + 96 + item.session.len();
        }
        if !current.is_empty() && size + len > bound::BATCH_BUDGET {
            frames.push(std::mem::take(&mut current));
            size = 0;
        }
        size += len;
        current.push(item);
    }
    if !current.is_empty() {
        frames.push(current);
    }
    frames
}

/// Answers the `media{mediaRef}` session command (`sessionRefChat.media`): the blob
/// a bounded snapshot referenced, as `{mime, dataUrl}` in one frame. `mediaRef` is a
/// sha256 hex into the host's blob store (validated: no path traversal); an image too
/// large for one frame answers `null` — the app falls back to `blob_get{sha256}`.
fn media_reply(service: Option<&SessionService>, cmd: &Value) -> Value {
    const DATA_URL_CAP: usize = unipi_proto::frame::MAX_FRAME * 3 / 4 - 4096;
    let Some(service) = service else {
        return Value::Null;
    };
    let Some(reference) = cmd
        .get("mediaRef")
        .and_then(Value::as_str)
        .filter(|r| r.len() == 64 && r.bytes().all(|b| b.is_ascii_hexdigit()))
    else {
        return Value::Null;
    };
    let path = service.hub().blobs_dir().join(reference);
    let Ok(bytes) = std::fs::read(&path) else {
        tracing::warn!(%reference, "media: no such blob");
        return Value::Null;
    };
    // Keep the reply under the frame cap: a data URL costs 4/3 the bytes.
    if bytes.len() > DATA_URL_CAP {
        tracing::warn!(%reference, bytes = bytes.len(), "media: image larger than one frame");
        return Value::Null;
    }
    let mime = sniff_mime(&bytes);
    let encoded = base64::Engine::encode(&base64::engine::general_purpose::STANDARD, &bytes);
    json!({ "mime": mime, "dataUrl": format!("data:{mime};base64,{encoded}") })
}

/// Image mime from magic bytes (the stored blob carries no sidecar metadata).
fn sniff_mime(bytes: &[u8]) -> &'static str {
    match bytes {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [0xff, 0xd8, ..] => "image/jpeg",
        [b'G', b'I', b'F', b'8', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "application/octet-stream",
    }
}

/// Runs one `SessionCmd` off the stream loop (a long `prompt` must not stall the writer) and
/// answers it with the command's correlation id (§7.3).
/// App-path command semantics (the session stream is the app's only command entry; the
/// TUI/CLI never ride it):
///
/// - E-19: `set_model` without an explicit `persist` picks the model **for this chat**
///   — the PC's default changes only through Settings or an explicit `persist: true`
///   ("Use at startup").
#[must_use]
fn app_model_semantics(mut cmd: Value) -> Value {
    if cmd.get("type").and_then(Value::as_str) == Some("set_model") && cmd.get("persist").is_none()
    {
        cmd["persist"] = Value::Bool(false);
    }
    cmd
}

/// Resolve only content-addressed blobs from this host's blob store before the core sees
/// the unchanged image protocol. Never accept client-supplied filesystem paths.
fn resolve_prompt_images(
    cmd: &mut Value,
    blobs_dir: Option<&std::path::Path>,
) -> Result<(), String> {
    use base64::Engine as _;
    if !matches!(
        cmd.get("type").and_then(Value::as_str),
        Some("prompt" | "append_note")
    ) {
        return Ok(());
    }
    let Some(images) = cmd.get_mut("images").and_then(Value::as_array_mut) else {
        return Ok(());
    };
    for image in images {
        let Some(reference) = image.get("mediaRef") else {
            continue;
        };
        let digest = reference.as_str().ok_or("Invalid image blob reference")?;
        if digest.len() != 64
            || !digest
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err("Invalid image blob reference".into());
        }
        let dir = blobs_dir.ok_or("Host unavailable")?;
        let bytes = std::fs::read(dir.join(digest)).map_err(|_| "Image blob not found")?;
        image["data"] = Value::String(base64::engine::general_purpose::STANDARD.encode(bytes));
        image
            .as_object_mut()
            .ok_or("Invalid image")?
            .remove("mediaRef");
    }
    Ok(())
}

/// Enriches a `get_available_models` reply with the hub's per-provider readiness (F-02):
/// each model row gains `ready: true` (the provider's credential validated), or
/// `ready: false` + a human `reason` (`invalid_key` / `unreachable`). An unknown
/// provider stays unmarked — the UI treats it as usable, nothing says otherwise.
pub(crate) fn enrich_models_reply(
    readiness: &std::collections::HashMap<String, crate::models_readiness::Readiness>,
    data: &mut Value,
) {
    let Some(rows) = data.get_mut("models").and_then(Value::as_array_mut) else {
        return;
    };
    for row in rows {
        let Some(provider) = row.get("provider").and_then(Value::as_str) else {
            continue;
        };
        let Some(entry) = readiness.get(provider) else {
            continue;
        };
        match entry.state {
            crate::models_readiness::ReadinessState::Ready => {
                row["ready"] = Value::Bool(true);
            }
            state => {
                row["ready"] = Value::Bool(false);
                row["reason"] = Value::String(readiness_reason(state, entry.detail.as_deref()));
            }
        }
    }
}

/// The human line for a readiness state (mirrors the host's `auth_providers` wording).
fn readiness_reason(
    state: crate::models_readiness::ReadinessState,
    detail: Option<&str>,
) -> String {
    let detail = detail.map(|d| format!(" ({d})")).unwrap_or_default();
    match state {
        crate::models_readiness::ReadinessState::InvalidKey => {
            format!("the key was rejected{detail}")
        }
        crate::models_readiness::ReadinessState::Unreachable => {
            format!("the provider is unreachable{detail}")
        }
        crate::models_readiness::ReadinessState::Ready => "the key works".to_owned(),
    }
}

/// One `get_entries{before|limit|ids}` ask.
struct PageAsk {
    before: Option<String>,
    limit: usize,
    ids: Option<Vec<String>>,
    chunk_offset: Option<usize>,
}

/// Answers a paged `get_entries`: from the runtime when it is up (`_oino/entries_page`),
/// else — or when the runtime cannot — from the session file; bounded for one frame.
async fn answer_page(
    session: Live,
    file: Option<SessionFile>,
    blobs_dir: Option<std::path::PathBuf>,
    ask: PageAsk,
    id: Option<Value>,
    replies: mpsc::Sender<SessionReply>,
) {
    let from_runtime = match session.ready() {
        Some(live) => live
            .send(RpcCommand::Oino(
                oino_client::protocol::OinoCommand::EntriesPage {
                    before: ask.before.clone(),
                    limit: Some(ask.limit),
                    ids: ask.ids.clone(),
                },
            ))
            .await
            .ok()
            .flatten(),
        None => None,
    };
    let reply = match (from_runtime, file) {
        (Some(reply), _) => Ok(reply),
        (None, Some(file)) => {
            let ask_ids = ask.ids.clone();
            tokio::task::spawn_blocking(move || match ask_ids {
                Some(ids) => file
                    .by_ids(&ids)
                    .map(|entries| json!({ "entries": entries })),
                None => file.page(ask.before, ask.limit).map(|page| page.reply()),
            })
            .await
            .map_err(|error| error.to_string())
            .and_then(|result| result)
        }
        (None, None) => Err("this chat has no saved history yet".to_owned()),
    };
    let reply = match reply {
        Ok(mut data) => {
            if let Some(offset) = ask.chunk_offset {
                // Full-output escape hatch: the UTF-8 JSON for a single entry can
                // be fetched incrementally without an oversized response. The
                // chunks concatenate to the original entry, including every field.
                let entry = data
                    .get("entries")
                    .and_then(Value::as_array)
                    .filter(|entries| entries.len() == 1)
                    .and_then(|entries| entries.first());
                match entry {
                    Some(entry) => SessionReply::ok(id, Some(bound::entry_chunk(entry, offset))),
                    None => SessionReply::failed(
                        id,
                        AppErrorDto::new(
                            AppErrorCode::E_SCHEMA,
                            "chunkOffset requires exactly one entry",
                            false,
                        ),
                    ),
                }
            } else if ask.ids.is_some() {
                if let Some(entries) = data.get_mut("entries").and_then(Value::as_array_mut) {
                    fast::bound_full(entries, blobs_dir.as_deref());
                }
                SessionReply::ok(id, Some(data))
            } else {
                match Page::from_reply(&data) {
                    Some(mut page) => {
                        page.bound(blobs_dir.as_deref());
                        SessionReply::ok(id, Some(page.reply()))
                    }
                    None => SessionReply::ok(id, Some(data)),
                }
            }
        }
        Err(error) => {
            SessionReply::failed(id, AppErrorDto::new(AppErrorCode::E_INTERNAL, error, false))
        }
    };
    let _ = replies.send(reply).await;
}

/// Recovery runs after the event tap subscribes, before the snapshot or new commands.
async fn restore_queue(session: &HubSession, feed: &Feed) -> Result<(), String> {
    match session.backend().restore_durable_queue().await {
        Ok(()) => Ok(()),
        Err(error) => {
            let error = error.to_string();
            let items = {
                let queue =
                    oino_core::session::durable_queue::DurableQueue::new(&session.card().file);
                queue.recover_quarantined()
            };
            feed.publish(json!({"kind": "queue_restore_failed", "items": items, "error": error}))
                .await;
            Err(error)
        }
    }
}

/// Mutations whose receipt order determines the model used for a prompt.
fn ordered_command(command: &RpcCommand) -> bool {
    matches!(
        command,
        RpcCommand::Agent(
            AgentCommand::Prompt { .. }
                | AgentCommand::Steer { .. }
                | AgentCommand::FollowUp { .. }
                | AgentCommand::ClearQueue
                | AgentCommand::SetModel { .. }
                | AgentCommand::CycleModel { .. }
                | AgentCommand::SetThinkingLevel { .. }
                | AgentCommand::CycleThinkingLevel
        ) | RpcCommand::Oino(oino_client::protocol::OinoCommand::QueueEdit(_))
    )
}

#[allow(
    clippy::too_many_lines,
    reason = "One arm per protocol operation or state transition; keep exhaustive dispatch and resource lifetimes visible together."
)]
#[allow(
    clippy::too_many_arguments,
    reason = "Dispatch captures one immutable admitted-command envelope; grouping it would only add a single-use transport wrapper."
)]
fn dispatch(
    service: &Arc<SessionService>,
    session: Live,
    feed: Arc<Feed>,
    file: Option<SessionFile>,
    id: Option<Value>,
    command: RpcCommand,
    retry_id: Option<String>,
    replies: mpsc::Sender<SessionReply>,
) {
    // Trust is user/host state, never a model tool. Handle both session and no-chat
    // transports at the host so changes reach every project, not just this chat.
    let host_cmd = match &command {
        RpcCommand::Oino(oino_client::protocol::OinoCommand::TrustGet { cwd }) => {
            Some(unipi_proto::settings::HostCmd::TrustGet { cwd: cwd.clone() })
        }
        RpcCommand::Oino(oino_client::protocol::OinoCommand::TrustSet { path, trusted }) => {
            Some(unipi_proto::settings::HostCmd::TrustSet {
                path: path.clone(),
                trusted: *trusted,
            })
        }
        RpcCommand::Oino(oino_client::protocol::OinoCommand::McpReload) => {
            Some(unipi_proto::settings::HostCmd::McpReload)
        }
        _ => None,
    };
    if let Some(cmd) = host_cmd {
        let hub = Arc::clone(service.hub());
        tokio::spawn(async move {
            let reply = match crate::host_cmd::run(&hub, cmd).await {
                Ok(data) => SessionReply::ok(id, Some(data)),
                Err(error) => SessionReply::failed(id, error),
            };
            let _ = replies.send(reply).await;
        });
        return;
    }
    let record_dir = service.hub().agent_dir().to_path_buf();
    let tap_service = Arc::downgrade(service);
    let mut command = command;
    if let RpcCommand::Agent(AgentCommand::Prompt {
        streaming_behavior, ..
    }) = &mut command
    {
        // App clients (including 1.0.6) send plain prompts. Core starts a run for
        // a follow-up prompt when the session is idle and queues it behind the
        // current run otherwise, so this makes a plain prompt unconditional: never
        // rejected as already-processing, and never turned into a steer. A wake may
        // already have started a legitimate autonomous continuation or restored
        // delivery; explicit steering/follow-up commands keep their behaviour.
        if streaming_behavior.is_none() {
            *streaming_behavior = Some(oino_core::session::StreamingBehavior::FollowUp);
        }
    }
    let reload_global_mcp = match &command {
        RpcCommand::Oino(oino_client::protocol::OinoCommand::ServiceCall {
            name, payload, ..
        }) => {
            matches!(name.as_str(), "mcp.save" | "mcp.remove" | "mcp.set-enabled")
                && payload.get("scope").and_then(Value::as_str) == Some("global")
                || name == "mcp.server-action"
                    && payload.get("action").and_then(Value::as_str) == Some("reload")
                    && payload.get("scope").and_then(Value::as_str) != Some("project")
        }
        RpcCommand::Agent(AgentCommand::Prompt { message, .. }) => message.trim() == "/mcp-reload",
        _ => false,
    };
    let reload_hub = Arc::clone(service.hub());
    let queued_text = match &command {
        RpcCommand::Agent(
            AgentCommand::Prompt { message, .. }
            | AgentCommand::Steer { message, .. }
            | AgentCommand::FollowUp { message, .. },
        ) => Some(message.clone()),
        _ => None,
    };
    let navigate = command.kind() == "_oino/navigate_tree";
    let limit = if command.kind() == "_oino/render" {
        RENDER_TIMEOUT
    } else if navigate {
        NAVIGATE_TIMEOUT
    } else {
        CMD_TIMEOUT
    };
    // Wire command ids correlate replies only. Never admit them as receipt identities.
    let message_id = queued_text
        .as_ref()
        .map(|_| retry_id.unwrap_or_else(|| uuid::Uuid::now_v7().to_string()));
    let request_id = message_id.clone().or_else(|| {
        id.as_ref()
            .map(|id| id.as_str().map_or_else(|| id.to_string(), str::to_owned))
    });
    // F-02: snapshot the readiness cache before the send (a mutex read, never a probe).
    let readiness = std::sync::Arc::new(service.hub().readiness().snapshot());
    // Core's whole `get_entries{since?}` (the app's run-end reconcile): a long turn's
    // subagent steps alone can pass the frame limit, and an oversize reply turns into an
    // error the app drops — the chat then keeps stale provisional state ("Subagent
    // running" on a finished turn, owner 2026-11-06). Bound it like a snapshot: big text
    // is cut with `truncated: true` (the app refetches it on expand).
    let is_entries = matches!(command, RpcCommand::Agent(AgentCommand::GetEntries { .. }));
    let blobs_dir = is_entries.then(|| service.hub().blobs_dir());
    let agent_dir = matches!(command, RpcCommand::Agent(AgentCommand::GetAvailableModels))
        .then(|| service.hub().agent_dir().to_path_buf());
    // v3-perf P0: send → accepted (`prompt`; core sends the provider request next) and
    // model switch timings — `wait_ms` is the wake a file-opened chat needed first.
    let kind = command.kind();
    let traced = matches!(kind, "prompt" | "steer" | "follow_up" | "set_model");
    let received = std::time::Instant::now();
    let span = tracing::info_span!("host_command", cmd = kind, request_id = ?request_id);
    let prompt_queue = ordered_command(&command).then(|| Arc::clone(&feed.wake_commands));
    let job = async move {
        // The record-only optimization belongs inside the receipt FIFO too. A
        // preceding prompt may acquire the runtime lease while this job waits.
        if session.ready().is_none() {
            if let (
                Some(file),
                RpcCommand::Agent(AgentCommand::SetModel {
                    provider,
                    model_id,
                    persist: Some(false),
                }),
            ) = (file, &command)
            {
                let provider = provider.clone();
                let model_id = model_id.clone();
                if let Ok(Some(model)) = tokio::task::spawn_blocking(move || {
                    record_model::append_model_change(&record_dir, &file, &provider, &model_id)
                })
                .await
                {
                    feed.publish(record_model::model_changed_event(&model))
                        .await;
                    let _ = replies.send(SessionReply::ok(id, Some(model))).await;
                    return;
                }
            }
        }
        // A file-opened session wakes behind its history: the command waits for it.
        let session = match session.get().await {
            Ok(session) => session,
            Err(error) => {
                let _ = replies.send(SessionReply::failed(id, error)).await;
                return;
            }
        };
        // A command may wake a session whose tap let go while it was idle: number what it
        // produces again (§4.2 lazy).
        feed.ensure_tap(Arc::clone(session.backend()), tap_service);
        if let Err(error) = restore_queue(&session, &feed).await {
            let _ = replies
                .send(SessionReply::failed(id, internal(error)))
                .await;
            return;
        }
        let is_models = matches!(command, RpcCommand::Agent(AgentCommand::GetAvailableModels));
        let sent_at = std::time::Instant::now();
        let reply =
            match tokio::time::timeout(limit, session.send_with_id(request_id.clone(), command))
                .await
            {
                Ok(Ok(mut data)) => {
                    if reload_global_mcp
                        && !data
                            .as_ref()
                            .is_some_and(|v| v.get("ok") == Some(&Value::Bool(false)))
                    {
                        if let Err(error) = crate::host_cmd::run(
                            &reload_hub,
                            unipi_proto::settings::HostCmd::McpReload,
                        )
                        .await
                        {
                            let _ = replies.send(SessionReply::failed(id, error)).await;
                            return;
                        }
                    }

                    if let Some(message_id) = message_id {
                        let value = data.get_or_insert_with(|| json!({}));
                        value["messageId"] = json!(message_id);
                    }
                    if is_models {
                        // F-02: per-provider readiness on every row (the app path only).
                        if let Some(data) = data.as_mut() {
                            // v3-perf §4.5: the live answer refreshes the picker's snapshot.
                            if let Some(dir) = agent_dir.clone() {
                                let reply = data.clone();
                                tokio::task::spawn_blocking(move || {
                                    record_model::store_catalogue(&dir, &reply);
                                });
                            }
                            enrich_models_reply(readiness.as_ref(), data);
                        }
                    }
                    if let (Some(dir), Some(entries)) = (
                        blobs_dir.as_deref(),
                        data.as_mut()
                            .and_then(|data| data.get_mut("entries"))
                            .and_then(Value::as_array_mut),
                    ) {
                        let _ = bound::bound_entries(entries, dir);
                    }
                    if navigate && !navigate_cancelled(data.as_ref()) {
                        publish_branch_moved(&session, &feed).await;
                    }
                    SessionReply::ok(id, data)
                }
                Ok(Err(error)) => SessionReply::failed(
                    id,
                    AppErrorDto::new(AppErrorCode::E_INTERNAL, error.to_string(), false)
                        .with_detail(json!({"messageId": message_id})),
                ),
                Err(_) => SessionReply::failed(
                    id,
                    AppErrorDto::new(
                        AppErrorCode::E_TIMEOUT,
                        "The session did not answer in time",
                        true,
                    )
                    .with_detail(json!({"messageId": message_id})),
                ),
            };
        if traced {
            tracing::info!(
                session = %session.session_id(),
                cmd = kind,
                wait_ms = u64::try_from((sent_at - received).as_millis()).unwrap_or(u64::MAX),
                cmd_ms = u64::try_from(sent_at.elapsed().as_millis()).unwrap_or(u64::MAX),
                ok = matches!(reply, SessionReply::Ok { .. }),
                "session command done"
            );
        }
        let _ = replies.send(reply).await;
    };
    if let Some(queue) = prompt_queue {
        queue.push(job.instrument(span));
    } else {
        tokio::spawn(job.instrument(span));
    }
}

/// A `navigate_tree` reply that did not move the leaf (an extension or the user cancelled).
fn navigate_cancelled(data: Option<&Value>) -> bool {
    data.and_then(|data| data.get("cancelled"))
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

/// [chat-tree] `_oino/navigate_tree` moved the leaf: the current branch every client shows is
/// stale. Core tells only extensions (`session_tree`), so the host numbers a
/// `{type: "session_tree", leafId}` event for every subscriber of the session (another
/// window or the phone reloads its branch page). It goes out before the command's reply
/// (§7.3 ordering). The leaf comes from a one-entry branch page (never the whole history).
async fn publish_branch_moved(session: &HubSession, feed: &Feed) {
    let leaf = session
        .send(RpcCommand::Oino(
            oino_client::protocol::OinoCommand::EntriesPage {
                before: None,
                limit: Some(1),
                ids: None,
            },
        ))
        .await
        .ok()
        .flatten()
        .and_then(|reply| reply.get("leafId").cloned())
        .unwrap_or(Value::Null);
    feed.publish(json!({"type": "session_tree", "leafId": leaf}))
        .await;
}

/// The identity of a streamed text/thinking delta: same message, same block, same kind
/// (§7.4 coalescing). `None` for every other event.
fn delta_key(event: &Value) -> Option<(String, u64, bool)> {
    if event.get("type")?.as_str()? != "message_update" {
        return None;
    }
    let provider_event = event.get("assistantMessageEvent")?;
    let thinking = match provider_event.get("type")?.as_str()? {
        "text_delta" => false,
        "thinking_delta" => true,
        _ => return None,
    };
    let message = event.get("message")?;
    let stamp = message.get("timestamp")?.to_string();
    let identity = format!(
        "{}:{}:{}:{}",
        stamp,
        message
            .get("provider")
            .and_then(Value::as_str)
            .unwrap_or(""),
        message.get("model").and_then(Value::as_str).unwrap_or(""),
        message.get("api").and_then(Value::as_str).unwrap_or("")
    );
    Some((
        identity,
        provider_event.get("contentIndex")?.as_u64()?,
        thinking,
    ))
}

fn is_delta(event: &Value) -> bool {
    delta_key(event).is_some()
}

/// Merges `next` into `into` when both are deltas of the same block: the text is appended,
/// the newer partial message replaces the old one and the seq becomes the newest (§7.4).
fn merge_delta(into: &mut SessionEventDto, next: &SessionEventDto) -> bool {
    if !into.event.is_message_update() || !next.event.is_message_update() {
        return false;
    }
    match (delta_key(&into.event), delta_key(&next.event)) {
        (Some(a), Some(b)) if a == b => {}
        _ => return false,
    }
    let Some(appended) = next
        .event
        .pointer("/assistantMessageEvent/delta")
        .and_then(Value::as_str)
    else {
        return false;
    };
    let appended = appended.to_owned();
    if let Some(Value::String(text)) = into.event.pointer_mut("/assistantMessageEvent/delta") {
        text.push_str(&appended);
    } else {
        return false;
    }
    if let Some(partial) = next
        .event
        .pointer("/assistantMessageEvent/partial")
        .cloned()
    {
        into.event["assistantMessageEvent"]["partial"] = partial;
    }
    if let Some(message) = next.event.get("message").cloned() {
        into.event["message"] = message;
    }
    into.seq = next.seq;
    true
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod app_model_tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn receipt_queue_keeps_abc_across_ready_boundary() {
        let queue = Arc::new(CommandQueue::default());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (ready, wake) = tokio::sync::oneshot::channel();
        let (done, drained) = tokio::sync::oneshot::channel();
        let first = Arc::clone(&seen);
        queue.push(async move {
            wake.await.unwrap();
            lock(&first).push("A");
        });
        let second = Arc::clone(&seen);
        queue.push(async move {
            lock(&second).push("B");
        });
        // C arrives after runtime readiness but before the old queue drains.
        ready.send(()).unwrap();
        let third = Arc::clone(&seen);
        queue.push(async move {
            lock(&third).push("C");
            done.send(()).unwrap();
        });
        drained.await.unwrap();
        assert_eq!(*lock(&seen), ["A", "B", "C"]);
    }

    #[tokio::test]
    async fn model_and_thinking_changes_share_prompt_fifo() {
        let queue = Arc::new(CommandQueue::default());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (release, wait) = tokio::sync::oneshot::channel();
        queue.push(async move {
            wait.await.unwrap();
        });
        let (done, drained) = tokio::sync::oneshot::channel();
        for (i, cmd) in [
            json!({"type":"set_model", "provider":"test", "modelId":"one"}),
            json!({"type":"prompt", "message":"A"}),
            json!({"type":"set_thinking_level", "level":"high"}),
            json!({"type":"prompt", "message":"B"}),
            json!({"type":"set_model", "provider":"test", "modelId":"two"}),
        ]
        .into_iter()
        .enumerate()
        {
            let command = RpcRequest::from_value(cmd).unwrap().command;
            assert!(ordered_command(&command));
            let seen = Arc::clone(&seen);
            queue.push(async move {
                lock(&seen).push(i);
            });
        }
        queue.push(async move {
            done.send(()).unwrap();
        });
        assert!(lock(&seen).is_empty());
        release.send(()).unwrap();
        drained.await.unwrap();
        assert_eq!(*lock(&seen), [0, 1, 2, 3, 4]);
    }

    #[test]
    fn prompt_blob_images_are_resolved_in_order_and_bad_refs_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let digest = "a".repeat(64);
        std::fs::write(dir.path().join(&digest), b"image bytes").unwrap();
        let mut cmd = json!({"type":"prompt", "message":"images", "images": (0..20).map(|_| json!({"type":"image", "mimeType":"image/png", "mediaRef":digest})).collect::<Vec<_>>()});
        resolve_prompt_images(&mut cmd, Some(dir.path())).unwrap();
        let images = cmd["images"].as_array().unwrap();
        assert_eq!(images.len(), 20);
        assert!(images
            .iter()
            .all(|image| image["data"] == "aW1hZ2UgYnl0ZXM=" && image.get("mediaRef").is_none()));
        assert!(RpcRequest::from_value(cmd).is_ok());
        let mut bad = json!({"type":"prompt", "images":[{"mediaRef":"../secret"}]});
        assert!(resolve_prompt_images(&mut bad, Some(dir.path())).is_err());
        let mut missing = json!({"type":"prompt", "images":[{"mediaRef":"b".repeat(64)}]});
        assert!(resolve_prompt_images(&mut missing, Some(dir.path())).is_err());
    }

    #[test]
    fn note_blob_images_are_resolved_like_prompt_images() {
        let dir = tempfile::tempdir().unwrap();
        let digest = "c".repeat(64);
        std::fs::write(dir.path().join(&digest), b"image bytes").unwrap();
        let mut cmd = json!({"type":"append_note", "text":"look", "images":[{"type":"image", "mimeType":"image/png", "mediaRef":digest}]});
        resolve_prompt_images(&mut cmd, Some(dir.path())).unwrap();
        assert_eq!(cmd["images"][0]["data"], "aW1hZ2UgYnl0ZXM=");
        assert!(matches!(
            RpcRequest::from_value(cmd).unwrap().command,
            RpcCommand::Agent(AgentCommand::AppendNote { .. })
        ));
    }

    #[test]
    fn an_app_model_pick_without_a_flag_is_per_chat() {
        // E-19: the app's plain `set_model` (the picker) switches THIS chat only.
        let cmd = serde_json::json!({"type": "set_model", "provider": "faux", "modelId": "faux-1"});
        let cmd = app_model_semantics(cmd);
        assert_eq!(cmd["persist"], false);
        // An explicit `persist: true` ("Use at startup") is kept.
        let cmd = app_model_semantics(serde_json::json!(
            {"type": "set_model", "provider": "faux", "modelId": "faux-1", "persist": true}
        ));
        assert_eq!(cmd["persist"], true);
        // Every other command passes through untouched.
        let cmd = app_model_semantics(serde_json::json!({"type": "prompt", "text": "hi"}));
        assert!(cmd.get("persist").is_none());
    }

    #[test]
    fn model_rows_carry_the_provider_readiness() {
        let cache = crate::models_readiness::ReadinessCache::new();
        cache.store("live", crate::models_readiness::ReadinessState::Ready, None);
        cache.store(
            "dead",
            crate::models_readiness::ReadinessState::InvalidKey,
            Some("HTTP 401 Unauthorized".to_owned()),
        );
        cache.store(
            "far",
            crate::models_readiness::ReadinessState::Unreachable,
            None,
        );
        let mut data = serde_json::json!({"models": [
            {"provider": "live", "id": "a", "kind": "chat"},
            {"provider": "dead", "id": "b", "kind": "chat"},
            {"provider": "far", "id": "c", "kind": "chat"},
            {"provider": "unknown", "id": "d", "kind": "chat"},
            {"id": "providerless"}
        ]});
        let snapshot = cache.snapshot();
        enrich_models_reply(&snapshot, &mut data);
        let rows = data["models"].as_array().unwrap();
        assert_eq!(rows[0]["ready"], true);
        assert_eq!(rows[1]["ready"], false);
        assert!(
            rows[1]["reason"]
                .as_str()
                .unwrap()
                .starts_with("the key was rejected"),
            "{}",
            rows[1]["reason"]
        );
        assert_eq!(rows[2]["ready"], false);
        assert!(rows[2]["reason"]
            .as_str()
            .unwrap()
            .starts_with("the provider is unreachable"));
        // Unknown / absent providers stay unmarked (usable, nothing says otherwise).
        assert!(rows[3].get("ready").is_none());
        assert!(rows[4].get("ready").is_none());
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "test assertions")]
mod ring_tests {
    use super::*;

    #[tokio::test]
    #[ignore = "manual feed microbenchmark; no release or worker IPC claim"]
    async fn raw_feed_microbenchmark() {
        let value =
            json!({"type":"tool_execution_update", "result":{"text":"x".repeat(16 * 1024)}});
        let json = serde_json::to_string(&value).unwrap();
        let mut typed_times = Vec::new();
        let mut raw_times = Vec::new();
        for round in 0..50 {
            for raw_first in [round % 2 == 0] {
                for raw in [raw_first, !raw_first] {
                    let feed = Feed::new("bench");
                    let start = std::time::Instant::now();
                    for _ in 0..200 {
                        if raw {
                            feed.publish_raw(
                                serde_json::value::RawValue::from_string(json.clone()).unwrap(),
                            )
                            .await;
                        } else {
                            feed.publish(serde_json::from_str(&json).unwrap()).await;
                        }
                    }
                    let elapsed = start.elapsed().as_secs_f64() * 1000.0;
                    if raw {
                        raw_times.push(elapsed);
                    } else {
                        typed_times.push(elapsed);
                    }
                    assert!(feed.ring_bytes() <= RING_BYTES);
                }
            }
        }
        typed_times.sort_by(f64::total_cmp);
        raw_times.sort_by(f64::total_cmp);
        println!(
            "200 events of {} bytes, n=50: typed median={:.3}ms p95={:.3}ms; raw median={:.3}ms p95={:.3}ms",
            json.len(),
            typed_times[25],
            typed_times[47],
            raw_times[25],
            raw_times[47]
        );
    }

    #[tokio::test]
    async fn raw_feed_matches_typed_bounding_replay_and_coalescing() {
        let typed = Feed::new("s1");
        let raw = Feed::new("s1");
        let mut typed_sub = typed.subscribe();
        let mut raw_sub = raw.subscribe();
        let delta = |text: &str| {
            json!({"type":"message_update",
            "message":{"timestamp":1,"provider":"faux","model":"one","api":"faux"},
            "assistantMessageEvent":{"type":"text_delta","contentIndex":0,"delta":text}})
        };
        let corpus = vec![
            json!({"type":"agent_start"}),
            delta("hello"),
            delta(" world"),
            json!({"type":"tool_execution_end","result":{"content":[{"type":"text","text":"x".repeat(2 * bound::EVENT_BUDGET)}]}}),
            json!({"type":"session_crashed","error":"boom"}),
            json!({"kind":"crashed","error":"boom"}),
            json!({"type":"session_taken_over","by":"tui"}),
            json!({"kind":"taken_over","from":"tui"}),
            json!({"type":"session_replaced","sessionId":"s2"}),
        ];
        for event in corpus {
            let bytes = serde_json::to_string(&event).unwrap();
            typed.publish(event).await;
            raw.publish_raw(serde_json::value::RawValue::from_string(bytes).unwrap())
                .await;
        }
        let typed_events = typed_sub.drain();
        let raw_events = raw_sub.drain();
        assert_eq!(typed_events, raw_events);
        assert_eq!(typed.events_after(0), raw.events_after(0));
        assert_eq!(typed.ring_bytes(), raw.ring_bytes());
        let mut a = (*typed_events[1]).clone();
        let mut b = (*raw_events[1]).clone();
        assert!(merge_delta(&mut a, &typed_events[2]));
        assert!(merge_delta(&mut b, &raw_events[2]));
        assert_eq!(a, b);
        assert_eq!(split_batch(vec![a]), split_batch(vec![b]));
        assert_eq!(replaced_session(&raw_events[8].event), Some("s2".into()));
        typed.mark_disposed();
        raw.mark_disposed();
        assert!(!raw.can_resume(1));
        assert_eq!(typed.events_after(0), raw.events_after(0));
    }

    #[tokio::test]
    async fn raw_feed_preserves_opaque_bytes_and_bounds_its_ring() {
        let feed = Feed::new("s1");
        let json = r#"{ "type": "future_event", "n": 1e2, "unknown": [true, null] }"#;
        feed.publish_raw(serde_json::value::RawValue::from_string(json.into()).unwrap())
            .await;
        let events = feed.events_after(0);
        assert_eq!(serde_json::to_string(&events[0].event).unwrap(), json);
        assert!(matches!(
            events[0].event,
            unipi_proto::session::EventBody::Raw(_)
        ));
        let text = "x".repeat(16 * 1024);
        for _ in 0..200 {
            let json =
                serde_json::to_string(&json!({"type":"tool_execution_end","result":text})).unwrap();
            feed.publish_raw(serde_json::value::RawValue::from_string(json).unwrap())
                .await;
        }
        assert_eq!(feed.seq(), 201);
        assert!(feed.ring_bytes() <= RING_BYTES);
        assert!(!feed.can_resume(1));
        assert!(feed.can_resume(200));
    }

    #[tokio::test]
    async fn blocked_raw_feed_does_not_stall_a_sibling() {
        let blocked = Feed::new("blocked");
        let _slow = blocked.subscribe();
        for _ in 0..TAP_CAPACITY {
            blocked
                .publish_raw(
                    serde_json::value::RawValue::from_string(r#"{"type":"agent_end"}"#.into())
                        .unwrap(),
                )
                .await;
        }
        let publisher = Arc::clone(&blocked);
        let task = tokio::spawn(async move {
            publisher
                .publish_raw(
                    serde_json::value::RawValue::from_string(r#"{"type":"agent_end"}"#.into())
                        .unwrap(),
                )
                .await
        });
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        let sibling = Feed::new("sibling");
        tokio::time::timeout(
            Duration::from_millis(100),
            sibling.publish(json!({"type":"agent_end"})),
        )
        .await
        .unwrap();
        task.abort();
        let _ = task.await;
    }

    #[tokio::test]
    async fn the_ring_is_byte_bounded_and_resume_stays_contiguous() {
        let feed = Feed::new("s1");
        let text = "x".repeat(16 * 1024);
        for _ in 0..200 {
            feed.publish(json!({"type": "tool_execution_end", "result": text}))
                .await;
        }
        assert_eq!(feed.seq(), 200);
        assert!(feed.ring_bytes() <= RING_BYTES, "{}", feed.ring_bytes());
        // The oldest events rolled over: resuming from them resets, the recent ones replay.
        assert!(!feed.can_resume(1));
        assert!(feed.can_resume(199));
        assert_eq!(feed.events_after(199).len(), 1);
        assert!(replay_fits(&feed.events_after(feed.seq() - 20)));
    }

    #[tokio::test]
    async fn a_forgotten_ring_keeps_the_numbering() {
        let feed = Feed::new("s1");
        for _ in 0..3 {
            feed.publish(json!({"type": "turn_start"})).await;
        }
        feed.forget_ring();
        assert_eq!(feed.ring_bytes(), 0);
        assert!(
            feed.can_resume(3),
            "nothing missed: resumes with an empty replay"
        );
        assert!(!feed.can_resume(2), "a missed event: snapshot");
        assert_eq!(feed.publish(json!({"type": "turn_start"})).await, 4);
    }

    /// owner 2026-11-07 00:33Z: the hub retired a chat's runtime (`maxServices`) and the app's
    /// stream stayed open and silent, so the chat never loaded again (no `session_open` until a
    /// reload). A runtime that goes away now ends every stream: the app reconnects on a clean end.
    #[tokio::test]
    async fn a_gone_runtime_ends_every_open_stream_after_what_it_queued() {
        let feed = Feed::new("s1");
        let mut first = feed.subscribe();
        let mut second = feed.subscribe();
        assert_eq!(feed.subscriber_count(), 2);
        feed.publish(json!({"type": "turn_start"})).await;
        feed.mark_disposed();
        feed.end_streams();
        assert_eq!(first.recv().await.unwrap().seq, 1, "queued first");
        assert!(first.recv().await.is_none(), "then the stream ends");
        assert!(second.recv().await.is_some());
        assert!(second.recv().await.is_none());
        assert_eq!(feed.subscriber_count(), 0);
        // The disposed feed makes the reopen a fresh snapshot, never a replay.
        assert!(!feed.can_resume(1));
    }
}
