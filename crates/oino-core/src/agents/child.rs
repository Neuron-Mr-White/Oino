//! Spawning and driving one nested agent session.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

use futures::StreamExt;
use futures::stream::{BoxStream, empty, once};
use oino_ai::{AgentMessage, JsonValue, Model, ThinkingLevel};
use oino_session::{CreateOptions, SessionLease, SessionManager};
use serde_json::json;
use tokio::task::JoinHandle;

use super::spec::{
    AgentSpec, CLOSE_GRACE_CAP, DEFAULT_HIBERNATE_AFTER_SEC, DEFAULT_MAX_CONCURRENT,
    DEFAULT_MAX_DEPTH, ExtensionPolicy, HARD_MAX_CONCURRENT, HARD_MAX_DEPTH, Persistence, Seed,
};
use super::{ChildConfig, ChildLink, Tree};
use crate::builtin::{SessionToolContext, builtin_tool_entries};
use crate::events::{AgentSessionEvent, ClientEvent, OinoEvent, RunState};
use crate::extensions::api::CallContext;
use crate::extensions::host::HostError;
use crate::extensions::runner::ExtensionSet;
use crate::hub::EventHub;
use crate::runtime::{acquire_lease, process_lease_info};
use crate::session::{
    AgentSession, Inner, InputSource, IsolatedTask, PromptDisposition, PromptOptions,
    SessionConfig, StreamingBehavior, lock,
};
use oino_session::LeaseInfo;

/// How often the hibernation watcher re-examines an idle child (E-HIBERNATE), at most:
/// a quarter of the window, clamped to this range (perf-fix #9 — a 500 ms poll per child
/// was a steady idle-CPU cost with hundreds of children; the hibernation lands within
/// `window + window/4`).
const HIBERNATE_POLL_MIN: Duration = Duration::from_millis(500);
const HIBERNATE_POLL_MAX: Duration = Duration::from_secs(15);

fn hibernate_poll(window: Duration) -> Duration {
    (window / 4).clamp(HIBERNATE_POLL_MIN, HIBERNATE_POLL_MAX)
}

fn failed(error: impl std::fmt::Display) -> HostError {
    HostError::Failed(error.to_string())
}

/// A nested agent session owned by an extension (`agents.agent`).
///
/// A file-persisted child that stays idle hibernates (E-HIBERNATE): the live
/// [`AgentSession`] is disposed and this shell keeps `{id, label, owner, session_file,
/// spec}` plus the file lease; the next handle operation rebuilds the session from the
/// file (cold resume + D-REPAIR) with the same session id.
pub struct ChildAgent {
    id: String,
    label: String,
    owner: String,
    /// The live child session (`None` while hibernated).
    session: Mutex<Option<AgentSession>>,
    parent: Weak<Inner>,
    tree: Arc<Tree>,
    closed: AtomicBool,
    forward: Mutex<Option<JoinHandle<()>>>,
    /// Single-writer lease of the child's session file (D-LOCK); children hold their own.
    /// Hibernation keeps it: the shell owns the file until it closes.
    lease: Mutex<Option<SessionLease>>,
    /// Bumped whenever `lease` changes; a stale takeover watcher exits on a mismatch.
    lease_epoch: AtomicU64,
    /// Serializes hibernation and wake against handle operations; a tokio mutex because
    /// it is held across awaits.
    gate: tokio::sync::Mutex<()>,
    /// The spawn spec, stored for the wake rebuild (E-HIBERNATE).
    spec: AgentSpec,
    /// Model resolved at spawn; reused on every wake so the child keeps its model.
    model: Model,
    /// Thinking level; updated by `set_thinking` so a wake restores the latest level.
    thinking: Mutex<ThinkingLevel>,
    /// Working directory resolved at spawn.
    cwd: PathBuf,
    /// Nesting depth (feed cache while hibernated).
    depth: u8,
    /// Child role visible to extensions (feed cache while hibernated).
    role: Option<String>,
    /// The child session file (`None` for memory-persisted children).
    file: Option<PathBuf>,
    /// Last-known usage JSON and assistant text, cached at hibernation.
    last_usage: Mutex<Option<JsonValue>>,
    last_text: Mutex<Option<String>>,
    /// The child's `events()` fan-out, alive across hibernation (perf-fix #4): the forward
    /// task of each live session relays into it, so an extension's subscription neither
    /// keeps the child awake nor ends when it hibernates — it goes quiet and carries the
    /// woken session's events. Closed with the agent.
    relay: EventHub,
    /// When the hibernation preconditions first held (watcher bookkeeping).
    idle_since: Mutex<Option<Instant>>,
    /// Weak self reference for spawning per-child tasks (`Arc::new` + immediate set).
    watch_self: std::sync::OnceLock<Weak<ChildAgent>>,
}

impl std::fmt::Debug for ChildAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChildAgent")
            .field("id", &self.id)
            .field("label", &self.label)
            .finish_non_exhaustive()
    }
}

/// Lets a session's forward task drain into the relay after the session hub closed
/// (hibernate / close), then gives up on it.
async fn finish_forward(task: Option<JoinHandle<()>>) {
    const FORWARD_DRAIN: Duration = Duration::from_secs(1);
    if let Some(task) = task {
        let abort = task.abort_handle();
        if tokio::time::timeout(FORWARD_DRAIN, task).await.is_err() {
            abort.abort();
        }
    }
}

fn event_json(event: impl serde::Serialize) -> JsonValue {
    serde_json::to_value(&event).unwrap_or(JsonValue::Null)
}

/// A child event as an extension's `agent.events()` stream carries it (perf-fix #6): a
/// `message_update` keeps its `assistantMessageEvent` (type, block index, `delta`, the final
/// `content` / tool call) but not the two copies of the growing message (`message` and
/// `assistantMessageEvent.partial`) — every extension reads the delta, and the copies made
/// each streamed token cost O(message) to serialize into the guest (O(n²) per reply; 27 % of
/// the busy CPU in the 50-subagent bench). The full message arrives with `message_end`.
fn extension_event_json(event: ClientEvent) -> JsonValue {
    let mut value = event_json(event);
    if value.get("type").and_then(JsonValue::as_str) == Some("message_update")
        && let Some(object) = value.as_object_mut()
    {
        object.remove("message");
        if let Some(update) = object
            .get_mut("assistantMessageEvent")
            .and_then(JsonValue::as_object_mut)
        {
            update.remove("partial");
        }
    }
    value
}

impl ChildAgent {
    /// Agent id (the child's session id).
    #[must_use]
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Label from the spec.
    #[must_use]
    pub fn label(&self) -> &str {
        &self.label
    }

    /// Extension that spawned the agent.
    #[must_use]
    pub fn owner(&self) -> &str {
        &self.owner
    }

    /// The live child session (`None` while hibernated — E-HIBERNATE; the transcript
    /// lives in the session file and returns on wake). Cheap clone (`Arc` inside).
    #[must_use]
    pub fn session(&self) -> Option<AgentSession> {
        lock(&self.session).clone()
    }

    /// Clones the live child session out of the shell, if awake.
    fn session_slot(&self) -> Option<AgentSession> {
        lock(&self.session).clone()
    }

    /// The child's transcript (empty while hibernated; the next waking handle
    /// operation rebuilds it, or read the session file directly).
    #[must_use]
    pub fn messages(&self) -> Vec<AgentMessage> {
        self.session_slot()
            .map(|session| session.messages())
            .unwrap_or_default()
    }

    /// Child session file (`None` in memory; stable across hibernation).
    #[must_use]
    pub fn session_file(&self) -> Option<PathBuf> {
        match self.session_slot() {
            Some(session) => session.session_file(),
            None => self.file.clone(),
        }
    }

    /// Whether [`ChildAgent::close`] ran.
    #[must_use]
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst)
    }

    /// Whether the child hibernated (E-HIBERNATE): the live session was disposed and the
    /// next handle operation rebuilds it from the session file.
    #[must_use]
    pub fn is_hibernated(&self) -> bool {
        !self.is_closed() && self.session_slot().is_none()
    }

    /// `running`, `idle`, `hibernated` or `closed`.
    #[must_use]
    pub fn state(&self) -> &'static str {
        if self.is_closed() {
            "closed"
        } else if let Some(session) = self.session_slot() {
            if session.is_idle() { "idle" } else { "running" }
        } else {
            "hibernated"
        }
    }

    /// Nesting depth of the child.
    #[must_use]
    pub(crate) fn depth(&self) -> u8 {
        self.depth
    }

    /// Role visible to extensions (`AgentSpec.childRole`, else the owner).
    #[must_use]
    pub(crate) fn role(&self) -> Option<&str> {
        self.role.as_deref()
    }

    fn ensure_open(&self) -> Result<(), HostError> {
        if self.is_closed() {
            Err(HostError::Failed(format!("Agent {} is closed", self.id)))
        } else {
            Ok(())
        }
    }

    /// The live session, waking the child if it hibernated (E-HIBERNATE). Callers that
    /// hold `gate` get the no-lock variant.
    async fn session_or_wake(&self) -> Result<AgentSession, HostError> {
        let _gate = self.gate.lock().await;
        self.session_or_wake_locked().await
    }

    /// [`Self::session_or_wake`] for callers already holding `gate`.
    async fn session_or_wake_locked(&self) -> Result<AgentSession, HostError> {
        if let Some(session) = self.session_slot() {
            return Ok(session);
        }
        self.wake_locked().await
    }

    /// Rebuilds the hibernated session (E-HIBERNATE): exactly the `spawn_agent` build
    /// with `Seed::SessionFile(child_file)` plus D-REPAIR, reusing the lease the shell
    /// never dropped. The session id — hence the agent id — is stable.
    async fn wake_locked(&self) -> Result<AgentSession, HostError> {
        if self.is_closed() {
            return Err(HostError::Failed(format!("Agent {} is closed", self.id)));
        }
        let Some(parent_inner) = self.parent.upgrade() else {
            return Err(HostError::Failed(format!(
                "Agent {} cannot wake: its parent session is gone",
                self.id
            )));
        };
        let parent = AgentSession::from_inner(parent_inner);
        let Some(file) = self.file.clone() else {
            return Err(HostError::Failed(format!(
                "Agent {} has no session file to wake from",
                self.id
            )));
        };
        let mut spec = self.spec.clone();
        spec.seed = Seed::SessionFile(file);
        let thinking = *lock(&self.thinking);
        let built = parent
            .build_child(
                &self.owner,
                &spec,
                self.model.clone(),
                thinking,
                self.cwd.as_path(),
                self.depth,
                ChildOpen::Wake,
            )
            .await;
        let (session, _lease_replaced_none, _file) = match built {
            Ok(built) => built,
            Err(error) => {
                tracing::warn!(session = %self.id, "subagent wake failed: {error}");
                return Err(error);
            }
        };
        if session.session_id() != self.id {
            return Err(HostError::Failed(format!(
                "Agent {} woke to a different session ({})",
                self.id,
                session.session_id()
            )));
        }
        if self.is_closed() {
            // `close` raced the rebuild: do not install a session into a closed shell.
            session.dispose().await;
            return Err(HostError::Failed(format!("Agent {} is closed", self.id)));
        }
        if let Some(router) = parent.inner.agents.router() {
            router.attach(&session);
        }
        session.bind_extensions("resume", None).await;
        let forward = parent.spawn_isolated(
            IsolatedTask::ChildEvents,
            forward_child_events(
                session.subscribe(),
                self.relay.clone(),
                Arc::downgrade(&parent.inner),
                session.inner.agents.link.clone(),
                self.owner.clone(),
                session.session_id(),
                parent.runner().call_context().clone(),
            ),
        );
        *lock(&self.forward) = Some(forward);
        *lock(&self.session) = Some(session.clone());
        *lock(&self.idle_since) = None;
        if let Some(me) = self.watch_self.get().and_then(Weak::upgrade) {
            tokio::spawn(hibernate_watcher(Arc::downgrade(&me)));
        }
        tracing::info!(session = %self.id, "subagent woke from hibernation (E-HIBERNATE)");
        parent.publish_agents_changed();
        Ok(session)
    }

    /// Hibernates the child (E-HIBERNATE): emits `session_shutdown{hibernate}` to its
    /// extensions, makes the session log durable, disposes the live session (transcript
    /// projection, subscriptions, forward task) and keeps the shell with
    /// `state() == "hibernated"`. Called by [`hibernate_watcher`] after the idle window.
    async fn hibernate(&self) {
        let _gate = self.gate.lock().await;
        if self.is_closed() {
            return;
        }
        let Some(session) = lock(&self.session).take() else {
            return; // already hibernated
        };
        if !Self::hibernatable(&session) {
            *lock(&self.session) = Some(session);
            return;
        }
        // Serve read-only handle operations from the shell while hibernated.
        *lock(&self.last_usage) = Some(event_json(session.session_stats()));
        *lock(&self.last_text) = session.last_assistant_text();
        let runner = session.runner();
        if runner.has_handlers("session_shutdown") {
            runner
                .emit(json!({"type": "session_shutdown", "reason": "hibernate"}))
                .await;
        }
        // Wake resumes from this very file: make every appended line durable first
        // (best-effort — the lines are already in the page cache, fsync only guards a
        // machine crash while the child sleeps).
        if let Err(error) = session.with_manager(SessionManager::sync) {
            tracing::warn!(session = %self.id, "hibernate fsync failed: {error}");
        }
        if let Some(router) = session.inner.agents.router() {
            router.detach(&self.id);
        }
        session.dispose().await;
        session.hub().close();
        let forward = lock(&self.forward).take();
        finish_forward(forward).await;
        *lock(&self.idle_since) = None;
        tracing::info!(session = %self.id, "subagent hibernated (E-HIBERNATE)");
        if let Some(parent) = self.parent.upgrade() {
            AgentSession::from_inner(parent).publish_agents_changed();
        }
    }

    /// Whether `session` may hibernate now (E-HIBERNATE preconditions): file-persisted,
    /// no run, compaction, retry, wait or `!` command and no open dialog. An `events()`
    /// subscriber does not keep the child awake (it rides the relay across hibernation).
    fn hibernatable(session: &AgentSession) -> bool {
        session.session_file().is_some()
            && session.run_state() == RunState::Idle
            && !session.is_bash_running()
            && !session
                .ui()
                .snapshot()
                .iter()
                .any(|request| crate::ui::is_dialog(&request.method))
    }

    /// Prompts the child; while it runs the text is queued (`steer`, else follow-up).
    /// Wakes a hibernated child first (E-HIBERNATE). Returns the disposition
    /// (`started`, `queued`, `handled`).
    ///
    /// # Errors
    ///
    /// Closed agent, wake failures (session file, repair) and prompt errors.
    pub async fn prompt(&self, text: String, delivery: Option<&str>) -> Result<String, HostError> {
        self.ensure_open()?;
        let behavior = if delivery == Some("steer") {
            StreamingBehavior::Steer
        } else {
            StreamingBehavior::FollowUp
        };
        let _gate = self.gate.lock().await;
        let session = self.session_or_wake_locked().await?;
        // New activity restarts the hibernation window (the watcher only samples).
        *lock(&self.idle_since) = None;
        let disposition = session
            .prompt(
                text,
                PromptOptions {
                    streaming_behavior: Some(behavior),
                    source: InputSource::Extension,
                    ..PromptOptions::default()
                },
            )
            .await
            .map_err(failed)?;
        Ok(match disposition {
            PromptDisposition::Handled => "handled",
            PromptDisposition::Queued => "queued",
            PromptDisposition::Started => "started",
        }
        .to_owned())
    }

    /// Steering message while running; starts a run when idle.
    ///
    /// # Errors
    ///
    /// As [`ChildAgent::prompt`].
    pub async fn steer(&self, text: String) -> Result<(), HostError> {
        self.prompt(text, Some("steer")).await.map(|_| ())
    }

    /// Follow-up message while running; starts a run when idle (resume).
    ///
    /// # Errors
    ///
    /// As [`ChildAgent::prompt`].
    pub async fn follow_up(&self, text: String) -> Result<(), HostError> {
        self.prompt(text, Some("followUp")).await.map(|_| ())
    }

    /// Aborts the current run (returns at once). A hibernated child is idle by
    /// definition, so this is a no-op there.
    pub fn abort(&self) {
        if let Some(session) = self.session_slot() {
            tokio::spawn(async move { session.abort().await });
        }
    }

    /// Sets the thinking level; returns the level clamped to the child's model. On a
    /// hibernated child the level is stored and applied at wake.
    ///
    /// # Errors
    ///
    /// Unknown level.
    pub fn set_thinking(&self, level: &str) -> Result<String, HostError> {
        let level: ThinkingLevel = level
            .parse()
            .map_err(|_| HostError::Invalid(format!("thinking level `{level}`")))?;
        if let Some(session) = self.session_slot() {
            let clamped = level.clamp(&session.available_thinking_levels());
            *lock(&self.thinking) = clamped;
            tokio::spawn(async move {
                session.set_thinking_level(clamped, false).await;
            });
            Ok(clamped.to_string())
        } else {
            // Hibernated: clamp against the spawn model and store the level; the wake
            // build records it (`thinkingLevelChange`) and arms the woken session with it.
            let clamped = level.clamp(&self.model.available_thinking_levels());
            *lock(&self.thinking) = clamped;
            Ok(clamped.to_string())
        }
    }

    /// Text of the child's last assistant message (last-known while hibernated).
    #[must_use]
    pub fn last_assistant_text(&self) -> Option<String> {
        match self.session_slot() {
            Some(session) => session.last_assistant_text(),
            None => lock(&self.last_text).clone(),
        }
    }

    /// Resolves once the child is idle (immediately when closed or hibernated).
    pub async fn wait_idle(&self) {
        if let Some(session) = self.session_slot() {
            session.wait_for_idle().await;
        }
    }

    /// The child's `SessionStats` JSON (the shell's last-known stats while hibernated;
    /// child usage entries are mirrored into the parent at every `MessageEnd` anyway).
    #[must_use]
    pub fn usage(&self) -> JsonValue {
        match self.session_slot() {
            Some(session) => event_json(session.session_stats()),
            None => lock(&self.last_usage).clone().unwrap_or(JsonValue::Null),
        }
    }

    /// Child client events (`AgentSessionEvent` JSON and friends) from now until
    /// close, across hibernation (E-HIBERNATE): a subscription does not keep the child
    /// awake — it goes quiet while the child sleeps and carries the woken session's events.
    /// On a hibernated child the first poll wakes it.
    pub fn events(&self) -> BoxStream<'static, JsonValue> {
        if self.is_closed() {
            return Box::pin(empty());
        }
        // Subscribe now, so nothing published after this call is missed (a wake below
        // relays into this very subscription).
        let stream = self.relay.subscribe().map(extension_event_json);
        if self.session_slot().is_some() {
            return stream.boxed();
        }
        let me = self.watch_self.get().cloned();
        once(async move {
            if let Some(me) = me.as_ref().and_then(Weak::upgrade) {
                let _ = me.session_or_wake().await;
            }
        })
        .filter_map(|()| std::future::ready(None))
        .chain(stream)
        .boxed()
    }

    /// Closes the agent: waits up to `grace` (≤ 5 s) for idle, aborts, emits
    /// `session_shutdown{reason:"quit"}` to the child's extensions, disposes the session
    /// and ends its event streams. Idempotent; a hibernated shell closes without
    /// rebuilding its session.
    pub async fn close(&self, grace: Duration) {
        self.close_with_reason(grace, "quit").await;
    }

    /// [`ChildAgent::close`] with a `session_shutdown` reason (D-TAKEOVER uses
    /// `"takeover"`). Also releases the child's lease (D-LOCK).
    async fn close_with_reason(&self, grace: Duration, reason: &str) {
        if self.closed.swap(true, Ordering::SeqCst) {
            return;
        }
        // Serialize against hibernate/wake so a rebuild can never install a session into
        // a closed shell (and a close never disposes a session another path owns).
        let _gate = self.gate.lock().await;
        let grace = grace.min(CLOSE_GRACE_CAP);
        let live = lock(&self.session).take();
        if let Some(session) = live {
            if !session.is_idle()
                && tokio::time::timeout(grace, session.wait_for_idle())
                    .await
                    .is_err()
            {
                session.abort().await;
            }
            let runner = session.runner();
            if runner.has_handlers("session_shutdown") {
                runner
                    .emit(json!({"type": "session_shutdown", "reason": reason}))
                    .await;
            }
            if let Some(router) = session.inner.agents.router() {
                router.detach(&self.id);
            }
            session.dispose().await;
            session.hub().close();
        } else {
            // Hibernated: shutdown was emitted at hibernation and the session is
            // gone; only shell bookkeeping remains.
            if let Some(router) = self.parent.upgrade().and_then(|p| p.agents.router()) {
                router.detach(&self.id);
            }
        }
        let forward = lock(&self.forward).take();
        finish_forward(forward).await;
        self.relay.close();
        self.release_lease();
        if let Some(parent) = self.parent.upgrade() {
            parent.agents.remove_child(&self.id);
            AgentSession::from_inner(parent).publish_agents_changed();
        }
        self.tree.release();
        *lock(&self.idle_since) = None;
    }

    /// Releases the child's single-writer lease and retires its watcher (D-LOCK).
    fn release_lease(&self) {
        self.lease_epoch.fetch_add(1, Ordering::SeqCst);
        drop(lock(&self.lease).take());
    }

    /// D-TAKEOVER holder side: the child's session file was taken over by another
    /// runner; close the child here (releasing the lease) and notify its subscribers. The
    /// `epoch` + `session_file` guard ignores a stale watcher (one tick behind a lease
    /// change) closing the child that replaced its own session file.
    async fn on_taken_over(&self, epoch: u64, session_file: &Path, by: LeaseInfo) {
        {
            let held = lock(&self.lease);
            let still_current = self.lease_epoch.load(Ordering::SeqCst) == epoch
                && held
                    .as_ref()
                    .is_some_and(|lease| lease.session_file() == session_file);
            if !still_current {
                return;
            }
        }
        match self.session_slot() {
            Some(session) => session.hub().publish(OinoEvent::SessionTakenOver { by }),
            // Hibernated: the child hub is closed; surface the takeover on the parent
            // (and to the child's `events()` subscribers, which wait on the relay).
            None => {
                self.relay
                    .publish(OinoEvent::SessionTakenOver { by: by.clone() });
                if let Some(parent) = self.parent.upgrade() {
                    parent.hub.publish(OinoEvent::SessionTakenOver { by });
                }
            }
        }
        self.close_with_reason(Duration::ZERO, "takeover").await;
    }
}

/// `extensionSettings.subagents.maxConcurrent` (default 8, hard cap 16).
fn max_concurrent(session: &AgentSession) -> usize {
    subagents_setting(session, "maxConcurrent")
        .and_then(|n| usize::try_from(n).ok())
        .unwrap_or(DEFAULT_MAX_CONCURRENT)
        .clamp(1, HARD_MAX_CONCURRENT)
}

/// `extensionSettings.subagents.hibernateAfterSec` (default 120, 0 = never; E-HIBERNATE).
fn hibernate_after_sec(session: &AgentSession) -> u64 {
    subagents_setting(session, "hibernateAfterSec").unwrap_or(DEFAULT_HIBERNATE_AFTER_SEC)
}

/// A u64 under `extensionSettings.subagents.<key>`.
fn subagents_setting(session: &AgentSession, key: &str) -> Option<u64> {
    session.with_settings(|s| {
        s.settings()
            .merged()
            .get("extensionSettings")
            .and_then(|e| e.get("subagents"))
            .and_then(|e| e.get(key))
            .and_then(JsonValue::as_u64)
    })
}

/// How [`AgentSession::build_child`] opens the child session.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ChildOpen {
    /// Fresh spawn (`agents.spawn`): acquire the file lease and record the model and
    /// thinking selection in the new session.
    Spawn,
    /// Hibernation wake (E-HIBERNATE): the shell still holds the lease of the very same
    /// file, so no lease is acquired (or returned); the session resumes the file in
    /// place with D-REPAIR.
    Wake,
}

impl AgentSession {
    fn resolve_child_model(&self, spec: &AgentSpec) -> Result<Model, HostError> {
        match &spec.model {
            Some(key) => key
                .split_once('/')
                .and_then(|(provider, id)| self.inner.models.find(provider, id))
                .ok_or_else(|| HostError::Invalid(format!("Unknown model {key}"))),
            None => self
                .model()
                .ok_or_else(|| HostError::Failed("No model selected".into())),
        }
    }

    fn child_manager(&self, spec: &AgentSpec, cwd: &Path) -> Result<SessionManager, HostError> {
        let parent_file = self.session_file();
        let options = CreateOptions {
            id: None,
            parent_session: parent_file.clone(),
        };
        let dir = match &spec.persistence {
            Persistence::Memory => None,
            Persistence::File { session_dir } => session_dir.clone().or_else(|| {
                parent_file
                    .as_deref()
                    .and_then(Path::parent)
                    .map(|d| d.join("children"))
            }),
        };
        if let Seed::SessionFile(path) = &spec.seed {
            if let Some(dir) = dir {
                return SessionManager::open(path, &dir, Some(cwd)).map_err(failed);
            }
            let root = path.parent().unwrap_or(cwd);
            let source = SessionManager::open(path, root, Some(cwd)).map_err(failed)?;
            return Ok(SessionManager::in_memory(
                cwd,
                options,
                source.entries().to_vec(),
            ));
        }
        let mut manager = match &spec.seed {
            Seed::BranchCopy => {
                let branch = self.with_manager(|m| {
                    m.branch(None)
                        .into_iter()
                        .filter(|e| !matches!(e.message(), Some(AgentMessage::System(_))))
                        .cloned()
                        .collect()
                });
                SessionManager::in_memory(cwd, options, branch)
            }
            _ => SessionManager::in_memory(cwd, options, Vec::new()),
        };
        if let Seed::Messages(messages) = &spec.seed {
            for message in messages {
                manager.append_message(message.clone()).map_err(failed)?;
            }
        }
        let Some(dir) = dir else {
            return Ok(manager);
        };
        // `--session-dir` semantics: the file lives directly in the child sessions dir.
        let path = dir.join(oino_session::manager::session_file_name(manager.header()));
        let mut text = serde_json::to_string(manager.header()).map_err(failed)?;
        text.push('\n');
        for entry in manager.entries() {
            text.push_str(&serde_json::to_string(entry).map_err(failed)?);
            text.push('\n');
        }
        std::fs::create_dir_all(&dir).map_err(failed)?;
        std::fs::write(&path, text).map_err(failed)?;
        let manager = SessionManager::open(&path, &dir, Some(cwd)).map_err(failed)?;
        Ok(manager)
    }

    fn child_extensions(&self, policy: &ExtensionPolicy) -> Arc<ExtensionSet> {
        let all = &self.inner.agents.all_extensions;
        match policy {
            ExtensionPolicy::None => Arc::new(ExtensionSet::default()),
            ExtensionPolicy::Inherit => Arc::clone(all),
            ExtensionPolicy::Only(ids) => Arc::new(ExtensionSet::new(
                all.extensions()
                    .iter()
                    .filter(|e| ids.iter().any(|id| id == e.instance.id()))
                    .cloned()
                    .collect(),
            )),
        }
    }

    /// `agents.spawn` (runtime §12): creates a nested session owned by `extension`.
    ///
    /// # Errors
    ///
    /// Depth or concurrency limit, unknown model, session file failures.
    #[allow(clippy::too_many_lines, reason = "one step per AgentSpec field")]
    pub async fn spawn_agent(
        &self,
        extension: &str,
        spec: AgentSpec,
    ) -> Result<Arc<ChildAgent>, HostError> {
        let agents = &self.inner.agents;
        let depth = agents.depth.saturating_add(1);
        if depth > agents.max_depth {
            return Err(HostError::Failed(format!(
                "Nested agent depth limit reached ({} levels)",
                agents.max_depth
            )));
        }
        let model = self.resolve_child_model(&spec)?;
        let thinking = spec
            .thinking
            .unwrap_or_else(|| self.thinking_level())
            .clamp(&model.available_thinking_levels());
        let cwd = spec.cwd.clone().unwrap_or_else(|| self.work_cwd());
        let max = max_concurrent(self);
        if !agents.tree.try_acquire(max) {
            return Err(HostError::Failed(format!(
                "Too many nested agents running (max {max})"
            )));
        }
        let built = self
            .build_child(
                extension,
                &spec,
                model.clone(),
                thinking,
                &cwd,
                depth,
                ChildOpen::Spawn,
            )
            .await;
        let (child, child_lease, child_file) = match built {
            Ok(built) => built,
            Err(error) => {
                agents.tree.release();
                return Err(error);
            }
        };
        if let Some(router) = agents.router() {
            router.attach(&child);
        }
        child.bind_extensions("startup", None).await;
        let relay = EventHub::new();
        let forward = self.spawn_isolated(
            IsolatedTask::ChildEvents,
            forward_child_events(
                child.subscribe(),
                relay.clone(),
                Arc::downgrade(&self.inner),
                child.inner.agents.link.clone(),
                extension.to_owned(),
                child.session_id(),
                self.runner().call_context().clone(),
            ),
        );
        let handle = Arc::new_cyclic(|watch_self| ChildAgent {
            id: child.session_id(),
            label: spec.label.clone(),
            owner: extension.to_owned(),
            session: Mutex::new(Some(child.clone())),
            parent: Arc::downgrade(&self.inner),
            tree: Arc::clone(&agents.tree),
            closed: AtomicBool::new(false),
            forward: Mutex::new(Some(forward)),
            lease: Mutex::new(child_lease),
            lease_epoch: AtomicU64::new(0),
            gate: tokio::sync::Mutex::new(()),
            spec,
            model,
            thinking: Mutex::new(thinking),
            cwd,
            depth,
            role: child
                .inner
                .agents
                .link
                .as_ref()
                .and_then(|link| link.role.clone()),
            file: child_file.clone(),
            last_usage: Mutex::new(None),
            last_text: Mutex::new(None),
            relay,
            idle_since: Mutex::new(None),
            watch_self: std::sync::OnceLock::from(watch_self.clone()),
        });
        if let Some(file) = child_file {
            let epoch = handle.lease_epoch.fetch_add(1, Ordering::SeqCst) + 1;
            tokio::spawn(child_lease_watcher(Arc::downgrade(&handle), epoch, file));
            // E-HIBERNATE: file-persisted children hibernate after the idle window.
            tokio::spawn(hibernate_watcher(Arc::downgrade(&handle)));
        }
        lock(&agents.children).push(Arc::clone(&handle));
        self.publish_agents_changed();
        Ok(handle)
    }

    #[allow(
        clippy::type_complexity,
        reason = "(session, its lease, the leased file)"
    )]
    #[allow(
        clippy::too_many_arguments,
        reason = "one parameter per spawn input; mirrors createAgentSession"
    )]
    #[allow(clippy::too_many_lines, reason = "one step per SessionConfig field")]
    async fn build_child(
        &self,
        extension: &str,
        spec: &AgentSpec,
        model: Model,
        thinking: ThinkingLevel,
        cwd: &Path,
        depth: u8,
        open: ChildOpen,
    ) -> Result<(AgentSession, Option<SessionLease>, Option<PathBuf>), HostError> {
        let agents = &self.inner.agents;
        let mut manager = self.child_manager(spec, cwd)?;
        // D-LOCK: children acquire the lease of their own file when `Persistence::File`
        // (fresh files are uniquely named, so this only contends on a `SessionFile` seed) —
        // before the child writes anything to it. A hibernation wake (E-HIBERNATE) keeps
        // the shell's lease instead: the child never stopped owning its file.
        let lease = match open {
            ChildOpen::Spawn => match manager.session_file().filter(|_| manager.is_persisted()) {
                Some(file) => {
                    let holder = process_lease_info(self.agent_dir(), self.mode());
                    Some(
                        acquire_lease(file, &holder, manager.session_id())
                            .await
                            .map_err(failed)?,
                    )
                }
                None => None,
            },
            ChildOpen::Wake => None,
        };
        let file = lease
            .as_ref()
            .map(|lease| lease.session_file().to_path_buf());
        // D-REPAIR: a resumed child file is repaired under the lease before use, so the
        // woken child (or a child spawned from a `SessionFile` seed) always starts from a
        // balanced transcript.
        if matches!(spec.seed, Seed::SessionFile(_)) && manager.is_persisted() {
            let report = manager.repair_interrupted_turn().map_err(failed)?;
            if report.torn_tail || !report.closed_tool_calls.is_empty() {
                tracing::info!(
                    session = manager.session_id(),
                    closed_tool_calls = ?report.closed_tool_calls,
                    torn_tail = report.torn_tail,
                    "child resume repair appended closers (D-REPAIR)"
                );
            }
        }
        manager
            .append_model_change(&model.provider, &model.id)
            .map_err(failed)?;
        manager
            .append_thinking_level_change(&thinking.to_string())
            .map_err(failed)?;
        let settings = self.with_settings(|s| s.snapshot());
        let wanted: Vec<String> = spec.tools.builtin.clone().unwrap_or_else(|| {
            settings.default_tools().unwrap_or_else(|| {
                oino_settings::tools::DEFAULT_TOOL_NAMES
                    .iter()
                    .map(|s| (*s).to_owned())
                    .collect()
            })
        });
        let context = Arc::new(SessionToolContext::default());
        let mut builtin =
            builtin_tool_entries(cwd, self.agent_dir(), &settings, Arc::clone(&context));
        builtin.retain(|entry| wanted.iter().any(|w| w == entry.name()));
        let mut allowed = wanted.clone();
        allowed.extend(spec.tools.extension_tools.iter().cloned());
        let parent_cx = self.runner().call_context().clone();
        let has_ui = match spec.ui {
            super::UiPolicy::Forward => parent_cx.has_ui,
            super::UiPolicy::AutoDeny { .. } => true,
            super::UiPolicy::None => false,
        };
        let child_role = spec
            .child_role
            .clone()
            .or_else(|| (!extension.is_empty()).then(|| extension.to_owned()));
        let max_depth = spec
            .max_depth
            .unwrap_or(DEFAULT_MAX_DEPTH)
            .min(agents.max_depth)
            .min(HARD_MAX_DEPTH);
        let session = AgentSession::new(SessionConfig {
            cwd: cwd.to_path_buf(),
            agent_dir: self.agent_dir().to_path_buf(),
            docs_root: self.inner.docs_root.clone(),
            manager,
            settings: Arc::clone(&self.inner.settings),
            resources: Arc::clone(&self.inner.resources),
            models: self.inner.models.clone(),
            extensions: self.child_extensions(&spec.extensions),
            stream: Arc::clone(&agents.base_stream),
            mode: self.mode(),
            has_ui,
            builtin_tools: builtin,
            custom_tools: Vec::new(),
            initial_active_tools: wanted,
            allowed_tools: Some(allowed),
            excluded_tools: spec.tools.deny.clone(),
            model: Some(model),
            thinking_level: thinking,
            scoped_models: Vec::new(),
            flag_values: self.runner().flag_values(),
            child: Some(ChildConfig {
                link: ChildLink {
                    parent: Arc::downgrade(&self.inner),
                    label: spec.label.clone(),
                    role: child_role,
                    ui: spec.ui.clone(),
                    prompt: spec.system_prompt.clone(),
                },
                all_extensions: Arc::clone(&agents.all_extensions),
                base_stream: Arc::clone(&agents.base_stream),
                tree: Arc::clone(&agents.tree),
                depth,
                max_depth,
                router: agents.router.get().cloned(),
            }),
        });
        context.bind(&session);
        Ok((session, lease, file))
    }

    /// Nested agents of this session (`agents.list`): `[{id, label, state, sessionFile}]`.
    #[must_use]
    pub fn agents_list(&self) -> JsonValue {
        let children = lock(&self.inner.agents.children).clone();
        JsonValue::Array(
            children
                .iter()
                .map(|c| {
                    json!({
                        "id": c.id(),
                        "label": c.label(),
                        "state": c.state(),
                        "owner": c.owner(),
                        "sessionFile": c.session_file().map(|p| p.display().to_string()),
                    })
                })
                .collect(),
        )
    }

    /// Open nested agents of this session.
    #[must_use]
    pub fn child_agents(&self) -> Vec<Arc<ChildAgent>> {
        lock(&self.inner.agents.children).clone()
    }

    /// Closes every nested agent (session disposal); hibernated shells close without
    /// rebuilding.
    pub(crate) async fn close_children(&self) {
        let children = std::mem::take(&mut *lock(&self.inner.agents.children));
        for child in children {
            child.close(Duration::ZERO).await;
        }
    }

    /// Whether this is a nested (child) session.
    #[must_use]
    pub fn is_child(&self) -> bool {
        self.inner.agents.link.is_some()
    }
}

/// Holder-side takeover watcher of a child's lease (D-TAKEOVER): polls the child session
/// file's `.takeover` request while the lease identified by `epoch` is current; on a
/// request the child is closed here (releasing the lease) so the other runner reloads it.
/// Runs through hibernation — the shell keeps the lease while it sleeps.
async fn child_lease_watcher(child: Weak<ChildAgent>, epoch: u64, session_file: PathBuf) {
    loop {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let Some(child) = child.upgrade() else {
            return; // child gone
        };
        if child.lease_epoch.load(Ordering::SeqCst) != epoch {
            return; // lease replaced or released
        }
        if let Some(request) = oino_session::read_takeover_request(&session_file) {
            child
                .on_taken_over(epoch, &session_file, request.requester)
                .await;
            return; // close released the lease; the epoch moved on
        }
    }
}

/// Hibernation watcher (E-HIBERNATE): polls a file-persisted child; once it stayed
/// hibernatable for `hibernateAfterSec` (watcher re-armed at every wake), the child
/// hibernates. Exits when the child closes or hibernates.
async fn hibernate_watcher(child: Weak<ChildAgent>) {
    let mut poll = HIBERNATE_POLL_MIN;
    loop {
        tokio::time::sleep(poll).await;
        let Some(child) = child.upgrade() else {
            return; // child gone
        };
        if child.is_closed() {
            return;
        }
        let Some(session) = child.session_slot() else {
            return; // hibernated (this watcher's job is done) or closing
        };
        let after = Duration::from_secs(hibernate_after_sec(&session));
        poll = hibernate_poll(after);
        if after.is_zero() || !ChildAgent::hibernatable(&session) {
            *lock(&child.idle_since) = None;
            continue;
        }
        let due = {
            let mut since = lock(&child.idle_since);
            let start = *since.get_or_insert_with(Instant::now);
            start.elapsed() >= after
        };
        if due {
            child.hibernate().await;
            return;
        }
    }
}

/// Mirrors child usage into the parent (`usage{kind:"child:<label>"}`) and delivers
/// `oino_agent` events to the owner when it subscribed.
async fn forward_child_events(
    mut events: crate::hub::EventStream,
    relay: EventHub,
    parent: Weak<Inner>,
    link: Option<ChildLink>,
    owner: String,
    agent_id: String,
    owner_cx: CallContext,
) {
    let label = link.map(|l| l.label).unwrap_or_default();
    let kind = format!("child:{label}");
    while let Some(event) = events.next().await {
        relay.publish_ref(&event);
        let Some(parent) = parent.upgrade().map(AgentSession::from_inner) else {
            return;
        };
        super::forward_to_root(&parent.inner, &agent_id, &event);
        if super::changes_state(&event) {
            parent.publish_agents_changed();
        }
        if let ClientEvent::Session(AgentSessionEvent::MessageEnd {
            message: AgentMessage::Assistant(message),
        }) = &event
        {
            let appended = parent.with_manager(|m| {
                m.append_usage(
                    &kind,
                    &message.provider,
                    &message.model,
                    message.usage,
                    None,
                )
            });
            match appended {
                Ok(id) => parent.publish_entry(&id),
                Err(error) => tracing::warn!("recording child usage failed: {error}"),
            }
        }
        let owner_ext = parent
            .runner()
            .set()
            .get(&owner)
            .cloned()
            .or_else(|| parent.inner.agents.all_extensions.get(&owner).cloned());
        if let Some(ext) = owner_ext
            && ext.state().events.contains("oino_agent")
        {
            let payload = json!({"type": "oino_agent", "agentId": agent_id, "event": event});
            if let Err(error) = ext
                .instance
                .on_event("oino_agent", payload, owner_cx.clone())
                .await
            {
                tracing::debug!("oino_agent delivery failed: {}", error.message);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{FauxProvider, FauxResponse, TestSession};
    use serde_json::json;

    /// E-HIBERNATE acceptance core: an idle file-persisted child hibernates within the
    /// window and the live `AgentSession` (its `Inner`) is actually dropped — observed
    /// through a weak reference, the strongest drop evidence available.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn hibernation_drops_the_child_session_inner() {
        let dir = tempfile::tempdir().unwrap();
        let provider = FauxProvider::new(vec![FauxResponse::Text("child reply".into())]);
        let parent = TestSession {
            settings: json!({"extensionSettings": {"subagents": {"hibernateAfterSec": 1}}})
                .as_object()
                .unwrap()
                .clone(),
            cwd: dir.path().to_path_buf(),
            session_dir: Some(dir.path().join("sessions")),
            ..TestSession::default()
        }
        .build(Arc::clone(&provider))
        .await;
        let spec = serde_json::from_value(json!({
            "label": "sleeper",
            "persistence": {"file": {}},
            "tools": {"builtin": []},
        }))
        .unwrap();
        let child = parent.spawn_agent("test", spec).await.unwrap();
        assert_eq!(
            child.prompt("task one".into(), None).await.unwrap(),
            "started"
        );
        child.wait_idle().await;
        let inner: Weak<Inner> = Arc::downgrade(&child.session().unwrap().inner);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
        while !child.is_hibernated() {
            assert!(tokio::time::Instant::now() < deadline, "never hibernated");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(child.state(), "hibernated");
        assert!(child.session().is_none());
        // The only strong reference was the shell's: the whole AgentSession deallocated.
        assert!(
            inner.upgrade().is_none(),
            "child Inner survived hibernation"
        );
    }
}
