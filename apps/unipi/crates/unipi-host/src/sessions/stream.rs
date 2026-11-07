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
//!   opened the session from another runner publishes `{kind:"taken_over", from}` as its
//!   first event after the snapshot (§8.2).

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock, PoisonError, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

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

/// The events of one session, numbered and kept for a resume (§7.4).
pub struct Feed {
    session_id: Mutex<String>,
    seq: AtomicU64,
    ring: Mutex<Ring>,
    subscribers: Mutex<Vec<(u64, mpsc::Sender<Arc<SessionEventDto>>)>>,
    next_subscriber: AtomicU64,
    tap: Mutex<Option<JoinHandle<()>>>,
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
            .finish()
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
        let seq = self.seq.fetch_add(1, Ordering::AcqRel) + 1;
        let dto = Arc::new(SessionEventDto::new(self.session_id(), seq, event));
        lock(&self.ring).push(Arc::clone(&dto), bytes + RING_EVENT_OVERHEAD);
        if is_delta(&dto.event) {
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

    fn is_disposed(&self) -> bool {
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
        lock(&self.subscribers).clear();
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
        if tap.as_ref().is_some_and(|task| !task.is_finished()) {
            return;
        }
        // Subscribe synchronously: commands sent immediately after ensure_tap must
        // not race the spawned task and lose non-replayed events such as renames.
        let events = backend.events();
        let feed = Arc::clone(self);
        *tap = Some(tokio::spawn(async move {
            feed.run_tap(backend, service, events).await
        }));
    }

    /// Stops the tap (the runtime is gone; a later open restarts it).
    pub fn stop_tap(&self) {
        *lock(&self.tap_backend) = None;
        if let Some(task) = lock(&self.tap).take() {
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
        let mut tick = tokio::time::interval(IDLE_POLL);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut idle_polls = 0;
        loop {
            tokio::select! {
                event = futures_util::StreamExt::next(&mut events) => {
                    let Some(event) = event else {
                        // The runtime ended (evicted, taken over, closed): a reopen is new,
                        // and the streams have nothing left to follow.
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
        Ok(Some(data)) => serde_json::from_value::<RpcSessionState>(data)
            .map(|state| state.is_streaming || state.is_compacting || !state.waiting_on.is_empty())
            .unwrap_or(false),
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

    /// The feed of `session_id` (created on first use).
    ///
    /// A feed whose runtime replaced its session id (a fork, §7.3) keeps numbering under the
    /// new id; the registry catches up with it here, so a forked stream shares one feed
    /// instead of tapping the runtime twice.
    #[must_use]
    pub fn feed(&self, session_id: &str) -> Arc<Feed> {
        let mut feeds = lock(&self.feeds);
        if let Some(feed) = feeds.get(session_id) {
            return Arc::clone(feed);
        }
        if let Some(feed) = feeds
            .values()
            .find(|feed| feed.session_id() == session_id)
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
            let name = name.as_ref().map(|name| name.as_str());
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
            if let Some((card, file, page)) = if crate::superagent_session::runtime(session.as_str()).is_some() { None } else { file_open(&hub, session.as_str()).await } {
                tracing::info!(
                    session = %card.session_id,
                    entries = page.entries.len(),
                    total = page.page.total,
                    took_ms = started.elapsed().as_millis() as u64,
                    "session open: tail page read from the file"
                );
                if !crate::superagent_session::is_superagent_file(&file.path) {
                    return open_cold(service, card, file, page, read, write).await;
                }
            }
            let opened = hub.open(session.as_str()).await.map_err(hub_error)?;
            tracing::info!(
                session = %opened.session_id(),
                took_ms = started.elapsed().as_millis() as u64,
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
                    name.as_ref().map(|name| name.as_str()),
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
            tracing::info!(
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
    let mut bytes = serde_json::to_vec(value).map_err(internal)?;
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
    write
        .write_all(&unipi_proto::frame::encode(
            unipi_proto::frame::FrameKind::Control,
            &bytes,
        ))
        .await
        .map_err(internal)?;
    write.flush().await.map_err(internal)
}

/// The error that stands in for an oversize `ok` reply: the same `id` (a session command's)
/// or the bare hello reply's `err`. `None` for anything else (events, hub events).
fn oversize_replacement(bytes: &[u8]) -> Option<Value> {
    let value: Value = serde_json::from_slice(bytes).ok()?;
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
    for notice in hub.notices() {
        write_json(write, &wire_hub_event(&notice)).await?;
    }
    let focus = hub.focus_connection();
    let mut events = hub.subscribe();
    // WP-A15 (app.md §3.14): the git watcher rides this stream too — a
    // `GitChanged{root}` per repo change, after the watcher's debounce.
    let mut git = crate::git_watch::GitWatchers::shared().subscribe();
    loop {
        tokio::select! {
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
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                // The global watcher registry outlives every connection; a
                // closed channel means the host is shutting down.
                Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                    hub.release_focus(focus).await;
                    return Ok(());
                }
            },
            frame = crate::protocol::read_frame(read) => {
                let frame = match frame {
                    Ok(frame) => frame,
                    Err(_) => {
                        hub.release_focus(focus).await;
                        return Ok(());
                    }
                };
                if frame.0 != 0 {
                    continue;
                }
                if let Ok(Control::Focus { session }) = serde_json::from_slice::<Control>(&frame.1) {
                    hub.set_focus(focus, session.as_deref()).await;
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
    // §8.2: an open that took the session over from another runner is a new runtime here, so
    // the previous stream's numbering is not resumable (the other runner wrote in between).
    let taken_over = session.taken_over().map(str::to_owned);
    let data = match since_seq {
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
    if let Some(from) = taken_over {
        // §8.2: the first session event after the snapshot tells the app where it came from.
        feed.publish(json!({"kind": "taken_over", "from": from}))
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
    let card = hub.resolve_quick(target).await.ok()?;
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
    snapshot.state = Some(provisional);
    snapshot.leaf_id = page.leaf_id.clone().map(Value::String);
    snapshot.entries = Some(page.entries);
    snapshot.page = Some(page.page);
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
            match runtime_parts(&session).await {
                Ok(parts) => {
                    // §8.2: where the session came from is the first event after the
                    // snapshot, as on the runtime path.
                    if let Some(from) = session.taken_over() {
                        feed.publish(json!({"kind": "taken_over", "from": from}))
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
            took_ms = started.elapsed().as_millis() as u64,
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
    let mut page = match page {
        Some(page) => page,
        None => {
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
    let reply = serde_json::to_value(&data).map_err(internal)?;
    write_control(write, &Control::Ok { data: Some(reply) }).await?;
    // Everything up to `data.seq` is in the snapshot (or the replay): the subscription may
    // still hold some of it, and applying it twice must not happen.
    let mut seen_through = data.seq;
    let mut batch: Vec<SessionEventDto> = Vec::new();
    let mut flush_at: Option<tokio::time::Instant> = None;
    let (replies, mut answered) = mpsc::channel::<SessionReply>(16);
    loop {
        let deadline =
            flush_at.unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(3600));
        tokio::select! {
            received = subscription.recv() => {
                let Some(event) = received else { return Ok(()) };
                if event.seq <= seen_through {
                    continue;
                }
                let delta = is_delta(&event.event);
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
            _ = tokio::time::sleep_until(deadline) => {
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
                        let cmd = app_model_semantics(cmd);
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
                                PageAsk { before, limit, ids },
                                id,
                                replies.clone(),
                            ));
                            continue;
                        }
                        match RpcRequest::from_value(cmd) {
                            Ok(request) => {
                                dispatch(&service.upgrade().expect("service gone"), session.clone(), Arc::clone(&feed), id, request.command, replies.clone());
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
    const DATA_URL_CAP: usize = unipi_proto::frame::MAX_FRAME * 3 / 4 - 4096;
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
            if ask.ids.is_some() {
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

fn dispatch(
    service: &Arc<SessionService>,
    session: Live,
    feed: Arc<Feed>,
    id: Option<Value>,
    command: RpcCommand,
    replies: mpsc::Sender<SessionReply>,
) {
    let tap_service = Arc::downgrade(service);
    let navigate = command.kind() == "_oino/navigate_tree";
    let limit = if command.kind() == "_oino/render" {
        RENDER_TIMEOUT
    } else if navigate {
        NAVIGATE_TIMEOUT
    } else {
        CMD_TIMEOUT
    };
    let request_id = id.as_ref().map(|value| match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
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
    tokio::spawn(async move {
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
        let is_models = matches!(command, RpcCommand::Agent(AgentCommand::GetAvailableModels));
        let reply =
            match tokio::time::timeout(limit, session.send_with_id(request_id, command)).await {
                Ok(Ok(mut data)) => {
                    if is_models {
                        // F-02: per-provider readiness on every row (the app path only).
                        if let Some(data) = data.as_mut() {
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
                    AppErrorDto::new(AppErrorCode::E_INTERNAL, error.to_string(), false),
                ),
                Err(_) => SessionReply::failed(
                    id,
                    AppErrorDto::new(
                        AppErrorCode::E_TIMEOUT,
                        "The session did not answer in time",
                        true,
                    ),
                ),
            };
        let _ = replies.send(reply).await;
    });
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
