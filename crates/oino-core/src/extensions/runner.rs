//! Extension runner: registries and event dispatch with composition semantics
//! (runtime.md §7.2).
//!
//! Handlers run sequentially in load order. Notification and transform handlers that fail
//! are reported as `extension_error` and skipped (the chain keeps the previous value);
//! `tool_call` and `user_bash` failures block. Every call has a deadline.

use std::collections::{HashMap, HashSet};
use std::panic::AssertUnwindSafe;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use futures::FutureExt;
use indexmap::{IndexMap, IndexSet};
use oino_ai::{AgentMessage, ImageContent, JsonObject, JsonValue};
use serde_json::json;

use super::api::{
    ArbiterRoles, CallContext, CommandSpec, ExtensionCallError, ExtensionId, ExtensionInstance,
    FlagSpec, Registration, ShortcutSpec, ToolSpec,
};
use crate::events::{ExtensionDisabledReason, ExtensionErrorEvent, OinoEvent};
use crate::session::panic_message;

/// Default handler deadline.
pub const HOOK_DEADLINE: Duration = Duration::from_secs(5);
/// `tool_call` deadline.
pub const TOOL_CALL_DEADLINE: Duration = Duration::from_secs(30);
/// `session_before_*` deadline.
pub const SESSION_BEFORE_DEADLINE: Duration = Duration::from_secs(60);
/// Upper bound of a manifest-raised handler deadline (runtime §7.2).
pub const MAX_MANIFEST_DEADLINE: Duration = Duration::from_secs(300);
/// Deadline of one `wait-reason` / `propose-nudge` call (runtime §7.6).
pub const ARBITER_DEADLINE: Duration = Duration::from_secs(2);

/// Deadline for one handler of `kind`.
#[must_use]
pub fn deadline_for(kind: &str) -> Duration {
    if kind == "tool_call" {
        TOOL_CALL_DEADLINE
    } else if kind.starts_with("session_before_") {
        SESSION_BEFORE_DEADLINE
    } else {
        HOOK_DEADLINE
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// Receives extension failures (published as `extension_error`).
pub trait ErrorSink: Send + Sync {
    /// Reports one failure.
    fn report(&self, error: ExtensionErrorEvent);

    /// Reports that an extension was disabled for the session (X-TRAP), published as
    /// `extension_disabled`.
    fn disabled(&self, event: OinoEvent);
}

/// Mutable registrations of one extension (static + dynamic).
#[derive(Clone, Debug, Default)]
pub struct ExtensionState {
    /// Tools by name.
    pub tools: IndexMap<String, ToolSpec>,
    /// Commands in registration order.
    pub commands: Vec<CommandSpec>,
    /// Shortcuts by id.
    pub shortcuts: IndexMap<String, ShortcutSpec>,
    /// Flags by name.
    pub flags: IndexMap<String, FlagSpec>,
    /// Subscribed event kinds.
    pub events: IndexSet<String>,
    /// Render specs.
    pub renderers: Vec<JsonValue>,
    /// Implemented provider APIs.
    pub apis: Vec<String>,
    /// Turn arbiter roles.
    pub arbiter: ArbiterRoles,
}

impl From<Registration> for ExtensionState {
    fn from(registration: Registration) -> Self {
        Self {
            tools: registration
                .tools
                .into_iter()
                .map(|tool| (tool.name.clone(), tool))
                .collect(),
            commands: registration.commands,
            shortcuts: registration
                .shortcuts
                .into_iter()
                .map(|s| (s.id.clone(), s))
                .collect(),
            flags: registration
                .flags
                .into_iter()
                .map(|f| (f.name.clone(), f))
                .collect(),
            events: registration.events.into_iter().collect(),
            renderers: registration.renderers,
            apis: registration.apis,
            arbiter: registration.arbiter,
        }
    }
}

/// A loaded extension and its registrations.
pub struct LoadedExtension {
    /// The instance.
    pub instance: Arc<dyn ExtensionInstance>,
    state: Mutex<ExtensionState>,
}

impl std::fmt::Debug for LoadedExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedExtension")
            .field("id", &self.instance.id())
            .finish_non_exhaustive()
    }
}

impl LoadedExtension {
    /// Wraps an initialised instance.
    #[must_use]
    pub fn new(instance: Arc<dyn ExtensionInstance>, registration: Registration) -> Self {
        Self {
            instance,
            state: Mutex::new(registration.into()),
        }
    }

    /// Copy of the registrations.
    #[must_use]
    pub fn state(&self) -> ExtensionState {
        lock(&self.state).clone()
    }

    /// Edits the registrations.
    pub fn update<R>(&self, f: impl FnOnce(&mut ExtensionState) -> R) -> R {
        f(&mut lock(&self.state))
    }

    fn subscribed(&self, kind: &str) -> bool {
        lock(&self.state).events.contains(kind)
    }
}

/// Every loaded extension in load order (shared by the sessions of one cwd).
#[derive(Debug, Default)]
pub struct ExtensionSet {
    extensions: Vec<Arc<LoadedExtension>>,
}

impl ExtensionSet {
    /// A set in load order.
    #[must_use]
    pub fn new(extensions: Vec<Arc<LoadedExtension>>) -> Self {
        Self { extensions }
    }

    /// Extensions in load order.
    #[must_use]
    pub fn extensions(&self) -> &[Arc<LoadedExtension>] {
        &self.extensions
    }

    /// Registered commands before any session exists (same collision rules as a runner).
    #[must_use]
    pub fn commands(&self) -> Vec<ResolvedCommand> {
        resolve_commands(self.extensions.iter())
    }

    /// Extension by id.
    #[must_use]
    pub fn get(&self, id: &str) -> Option<&Arc<LoadedExtension>> {
        self.extensions.iter().find(|e| e.instance.id() == id)
    }

    /// Number of extensions.
    #[must_use]
    pub fn len(&self) -> usize {
        self.extensions.len()
    }

    /// Whether no extension is loaded.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.extensions.is_empty()
    }
}

/// Resolve invocation names identically for session and sessionless catalogues.
fn resolve_commands<'a>(
    extensions: impl Iterator<Item = &'a Arc<LoadedExtension>>,
) -> Vec<ResolvedCommand> {
    let mut all = Vec::new();
    for extension in extensions {
        for spec in extension.state().commands {
            all.push((Arc::clone(extension), spec));
        }
    }
    let mut counts: IndexMap<String, usize> = IndexMap::new();
    for (_, spec) in &all {
        *counts.entry(spec.name.clone()).or_default() += 1;
    }
    let mut seen: IndexMap<String, usize> = IndexMap::new();
    let mut taken = HashSet::new();
    all.into_iter()
        .map(|(extension, spec)| {
            let occurrence = {
                let slot = seen.entry(spec.name.clone()).or_default();
                *slot += 1;
                *slot
            };
            let mut invocation_name = if counts.get(&spec.name).copied().unwrap_or(0) > 1 {
                format!("{}:{occurrence}", spec.name)
            } else {
                spec.name.clone()
            };
            let mut suffix = occurrence;
            while taken.contains(&invocation_name) {
                suffix += 1;
                invocation_name = format!("{}:{suffix}", spec.name);
            }
            taken.insert(invocation_name.clone());
            ResolvedCommand {
                extension,
                spec,
                invocation_name,
            }
        })
        .collect()
}

/// A command with its resolved invocation name.
#[derive(Clone, Debug)]
pub struct ResolvedCommand {
    /// Owning extension.
    pub extension: Arc<LoadedExtension>,
    /// Spec.
    pub spec: CommandSpec,
    /// `name`, or `name:N` on collisions.
    pub invocation_name: String,
}

/// A registered tool and its owner.
#[derive(Clone, Debug)]
pub struct RegisteredTool {
    /// Owning extension.
    pub extension: Arc<LoadedExtension>,
    /// Spec.
    pub spec: ToolSpec,
}

/// Result of [`ExtensionRunner::emit_input`].
#[derive(Clone, Debug, PartialEq)]
pub enum InputOutcome {
    /// Continue with the original input.
    Continue,
    /// Continue with replaced input.
    Transform {
        /// New text.
        text: String,
        /// New images.
        images: Option<Vec<ImageContent>>,
    },
    /// An extension handled the input.
    Handled,
}

/// Combined `before_agent_start` result.
#[derive(Clone, Debug, PartialEq)]
pub struct BeforeAgentStartOutcome {
    /// Messages to inject (`{customType, content, display, details?}`), in order.
    pub messages: Vec<JsonValue>,
    /// Final `systemPromptOptions`.
    pub system_prompt_options: JsonValue,
    /// Forced system prompt (last wins).
    pub system_prompt: Option<String>,
}

/// Result of a boundary dispatch (`turn_end`, `agent_before_settle`).
#[derive(Clone, Debug, PartialEq)]
pub struct BoundaryOutcome {
    /// Drafts to commit.
    pub entries: Vec<JsonValue>,
    /// Ensure one more request.
    pub continue_run: bool,
}

/// Collected `resources_discover` paths with their extension.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiscoveredResources {
    /// `(path, extension id)` skill paths.
    pub skill_paths: Vec<(String, String)>,
    /// Prompt template paths.
    pub prompt_paths: Vec<(String, String)>,
    /// Theme paths.
    pub theme_paths: Vec<(String, String)>,
}

/// Per-session dispatcher over the shared [`ExtensionSet`].
#[derive(Clone)]
pub struct ExtensionRunner {
    set: Arc<ExtensionSet>,
    cx: CallContext,
    errors: Arc<dyn ErrorSink>,
    flag_values: Arc<Mutex<IndexMap<String, JsonValue>>>,
    stale: Arc<Mutex<Option<String>>>,
    dialogs: Option<crate::ui::UiBridge>,
    approval_pending: Option<Arc<dyn Fn() -> bool + Send + Sync>>,
    /// Extensions disabled for this session (X-TRAP); shared by every clone of this
    /// session's runner.
    disabled: Arc<Mutex<IndexSet<ExtensionId>>>,
    /// perf-fix #7: each extension's last `before_agent_start` prompt contribution
    /// (`systemPromptOptions` in → out, forced `systemPrompt`). A handler that misses its
    /// deadline on a later turn with the same input keeps its section instead of dropping
    /// it — a dropped section broke the provider's prompt cache on ~35 % of graph root
    /// turns. One entry per extension (bounded by the set).
    sticky_prompt: Arc<Mutex<HashMap<String, StickyPrompt>>>,
    deliveries: Arc<Mutex<HashMap<String, tokio::sync::mpsc::Sender<Delivery>>>>,
    /// N6: handlers blocked on a process-approval card (`HostOp::CheckProcess`), per extension — the user's
    /// answer time is not handler time (the deadline pauses like it does for an open dialog).
    approval_waits: Arc<Mutex<HashMap<String, usize>>>,
}

/// Held while an extension's handler waits for the user to answer a process approval.
pub struct ApprovalWait {
    waits: Arc<Mutex<HashMap<String, usize>>>,
    extension: String,
}

impl Drop for ApprovalWait {
    fn drop(&mut self) {
        let mut waits = lock(&self.waits);
        if let Some(n) = waits.get_mut(&self.extension) {
            *n = n.saturating_sub(1);
            if *n == 0 {
                waits.remove(&self.extension);
            }
        }
    }
}

// One FIFO per extension and session. Bounded admission applies backpressure even
// to stream deltas: never discard a delta or a lifecycle boundary.
const DELIVERY_CAPACITY: usize = 32;
tokio::task_local! {
    // Inherit the whole stack for A -> B -> A calls, not just direct recursion.
    static ACTIVE_EXTENSIONS: Vec<usize>;
}

/// Reentrant hook stack carried across an extension runtime's task boundary.
#[doc(hidden)]
#[derive(Clone, Default)]
pub struct HookContext(Vec<usize>, Option<CommandHandoff>);

type CommandHandoff = (
    String,
    Arc<Mutex<HashMap<String, usize>>>,
    Arc<std::sync::atomic::AtomicBool>,
);

impl HookContext {
    /// Captures the caller's active extension stack.
    pub fn capture() -> Self {
        Self(
            ACTIVE_EXTENSIONS.try_with(Clone::clone).unwrap_or_default(),
            crate::session::delivery::COMMAND_HANDOFF
                .try_with(Clone::clone)
                .ok(),
        )
    }

    /// Restores the caller's stack while polling runtime/import work.
    pub async fn scope<T>(&self, future: impl Future<Output = T>) -> T {
        let future = ACTIVE_EXTENSIONS.scope(self.0.clone(), future);
        if let Some(context) = &self.1 {
            crate::session::delivery::COMMAND_HANDOFF
                .scope(context.clone(), future)
                .await
        } else {
            future.await
        }
    }
}

enum Delivery {
    Observer(Box<ObserverDelivery>),
    Fence(tokio::sync::oneshot::Sender<()>),
}

struct ObserverDelivery {
    runner: ExtensionRunner,
    extension: Arc<LoadedExtension>,
    kind: String,
    payload: Option<JsonValue>,
    encoded: Option<Arc<str>>,
    reply: tokio::sync::oneshot::Sender<Result<Option<JsonValue>, ExtensionCallError>>,
}

/// One extension's last successful `before_agent_start` prompt result.
#[derive(Clone, Debug)]
struct StickyPrompt {
    input: JsonValue,
    options: Option<JsonValue>,
    forced: Option<String>,
}

impl std::fmt::Debug for ExtensionRunner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ExtensionRunner")
            .field("extensions", &self.set.len())
            .field("session", &self.cx.session_id)
            .finish_non_exhaustive()
    }
}

/// The `invalidate` default message (verbatim, product name substituted).
pub const STALE_CONTEXT_MESSAGE: &str = "This extension ctx is stale after session replacement or reload. Do not use a captured Oino or command ctx after ctx.newSession(), ctx.fork(), ctx.switchSession(), or ctx.reload(). For newSession, fork, and switchSession, move post-replacement work into withSession and use the ctx passed to withSession. For reload, do not use the old ctx after await ctx.reload().";

impl ExtensionRunner {
    /// A runner for one session.
    #[must_use]
    pub fn new(
        set: Arc<ExtensionSet>,
        cx: CallContext,
        errors: Arc<dyn ErrorSink>,
        flag_values: IndexMap<String, JsonValue>,
    ) -> Self {
        Self {
            set,
            cx,
            errors,
            flag_values: Arc::new(Mutex::new(flag_values)),
            stale: Arc::new(Mutex::new(None)),
            dialogs: None,
            approval_pending: None,
            approval_waits: Arc::default(),
            disabled: Arc::new(Mutex::new(IndexSet::new())),
            sticky_prompt: Arc::default(),
            deliveries: Arc::default(),
        }
    }

    /// Pauses handler deadlines while the handler's extension has a dialog open on `ui`,
    /// and withdraws its dialogs when a handler times out (GA-42).
    #[must_use]
    pub fn with_dialogs(mut self, ui: crate::ui::UiBridge) -> Self {
        self.dialogs = Some(ui);
        self
    }

    /// Session-scoped approval wait; unrelated cards must not consume hook deadlines.
    pub fn set_approval_pending(&mut self, pending: Arc<dyn Fn() -> bool + Send + Sync>) {
        self.approval_pending = Some(pending);
    }

    /// The extension set.
    #[must_use]
    pub fn set(&self) -> &Arc<ExtensionSet> {
        &self.set
    }

    /// Base call context of this session.
    #[must_use]
    pub fn call_context(&self) -> &CallContext {
        &self.cx
    }

    /// Replaces the base call context (UI binding, mode).
    pub fn set_call_context(&mut self, cx: CallContext) {
        self.cx = cx;
    }

    /// Marks contexts of this runner stale.
    pub fn invalidate(&self, message: Option<&str>) {
        let mut stale = lock(&self.stale);
        if stale.is_none() {
            *stale = Some(message.unwrap_or(STALE_CONTEXT_MESSAGE).to_owned());
        }
    }

    /// The stale message once invalidated.
    #[must_use]
    pub fn stale_message(&self) -> Option<String> {
        lock(&self.stale).clone()
    }

    /// Reports an error as `extension_error`.
    pub fn emit_error(&self, extension_path: &str, event: &str, error: impl Into<String>) {
        self.errors.report(ExtensionErrorEvent {
            extension_path: extension_path.to_owned(),
            event: event.to_owned(),
            error: error.into(),
            stack: None,
        });
    }

    fn active(&self) -> impl Iterator<Item = &Arc<LoadedExtension>> {
        self.set
            .extensions()
            .iter()
            .filter(|extension| !self.is_disabled(extension.instance.id()))
    }

    fn handlers(&self, kind: &str) -> Vec<Arc<LoadedExtension>> {
        self.active()
            .filter(|extension| extension.subscribed(kind))
            .cloned()
            .collect()
    }

    /// Returns whether any handler is registered.
    #[must_use]
    pub fn has_handlers(&self, kind: &str) -> bool {
        self.active().any(|extension| extension.subscribed(kind))
    }

    /// Whether `id` is disabled for this session (X-TRAP).
    #[must_use]
    pub fn is_disabled(&self, id: &str) -> bool {
        lock(&self.disabled).contains(id)
    }

    /// Extensions disabled for this session (X-TRAP), in disable order.
    #[must_use]
    pub fn disabled_extensions(&self) -> Vec<ExtensionId> {
        lock(&self.disabled).iter().cloned().collect()
    }

    /// Disables `extension` for this session (X-TRAP): one trap or panic is enough — cheap
    /// and reversible, since the runner is per session and a session replacement starts
    /// with a fresh one. The instance-level circuit breaker (3 restarts / 10 min → disabled
    /// for the process, instance.rs) stays as the last resort.
    fn disable(&self, extension: &LoadedExtension, reason: ExtensionDisabledReason) {
        let id = extension.instance.id().to_owned();
        if !lock(&self.disabled).insert(id.clone()) {
            return; // already disabled for this session; the event was emitted
        }
        if let Some(ui) = &self.dialogs {
            ui.cancel_for(&id);
        }
        self.errors.disabled(OinoEvent::ExtensionDisabled {
            session_id: self.cx.session_id.clone(),
            extension_id: id,
            reason,
        });
    }

    async fn call(
        &self,
        extension: &LoadedExtension,
        kind: &str,
        payload: JsonValue,
    ) -> Result<Option<JsonValue>, ExtensionCallError> {
        self.call_payload(extension, kind, Some(payload), None)
            .await
    }

    async fn call_payload(
        &self,
        extension: &LoadedExtension,
        kind: &str,
        payload: Option<JsonValue>,
        encoded: Option<Arc<str>>,
    ) -> Result<Option<JsonValue>, ExtensionCallError> {
        let key = std::ptr::from_ref(extension) as usize;
        let nested = ACTIVE_EXTENSIONS
            .try_with(|active| active.contains(&key))
            .unwrap_or(false);
        if !nested {
            let (done, drained) = tokio::sync::oneshot::channel();
            let _ = self
                .delivery_sender(extension.instance.id())
                .send(Delivery::Fence(done))
                .await;
            let _ = drained.await;
        }
        // Keep the handler future owned by the caller: cancellation drops it,
        // propagating to the wasm host's CancelOnDrop guard.
        self.deliver(extension, kind, payload, encoded).await
    }

    fn delivery_sender(&self, id: &str) -> tokio::sync::mpsc::Sender<Delivery> {
        let mut deliveries = lock(&self.deliveries);
        deliveries
            .entry(id.to_owned())
            .or_insert_with(|| {
                let (tx, mut rx) = tokio::sync::mpsc::channel::<Delivery>(DELIVERY_CAPACITY);
                tokio::spawn(async move {
                    while let Some(delivery) = rx.recv().await {
                        match delivery {
                            Delivery::Fence(done) => {
                                let _ = done.send(());
                            }
                            Delivery::Observer(mut job) => {
                                if job.reply.is_closed() {
                                    continue;
                                }
                                tokio::select! {
                                    biased;
                                    () = job.reply.closed() => {},
                                    result = job.runner.deliver(&job.extension, &job.kind, job.payload, job.encoded) => {
                                        let _ = job.reply.send(result);
                                    }
                                }
                            }
                        }
                    }
                });
                tx
            })
            .clone()
    }

    /// Marks `extension`'s handler as waiting for the user's approval until the guard drops.
    #[must_use]
    pub fn approval_wait(&self, extension: &str) -> ApprovalWait {
        *lock(&self.approval_waits)
            .entry(extension.to_owned())
            .or_default() += 1;
        ApprovalWait {
            waits: Arc::clone(&self.approval_waits),
            extension: extension.to_owned(),
        }
    }

    async fn enqueue(
        &self,
        extension: Arc<LoadedExtension>,
        kind: &str,
        payload: Option<JsonValue>,
        encoded: Option<Arc<str>>,
    ) -> tokio::sync::oneshot::Receiver<Result<Option<JsonValue>, ExtensionCallError>> {
        let tx = self.delivery_sender(extension.instance.id());
        let (reply, rx) = tokio::sync::oneshot::channel();
        let _ = tx
            .send(Delivery::Observer(Box::new(ObserverDelivery {
                runner: self.clone(),
                extension,
                kind: kind.to_owned(),
                payload,
                encoded,
                reply,
            })))
            .await;
        rx
    }

    async fn deliver(
        &self,
        extension: &LoadedExtension,
        kind: &str,
        payload: Option<JsonValue>,
        encoded: Option<Arc<str>>,
    ) -> Result<Option<JsonValue>, ExtensionCallError> {
        // A queued notification may have trapped while this one was waiting.
        if self.is_disabled(extension.instance.id()) {
            return Ok(None);
        }
        let deadline = extension.instance.hook_deadline(kind).map_or_else(
            || deadline_for(kind),
            |d| d.min(MAX_MANIFEST_DEADLINE).max(deadline_for(kind)),
        );
        let id = extension.instance.id();
        let waiting = || {
            self.dialogs
                .as_ref()
                .is_some_and(|ui| ui.pending_for(id) > 0)
                || lock(&self.approval_waits).contains_key(id)
                || self
                    .approval_pending
                    .as_ref()
                    .is_some_and(|pending| pending())
        };
        // X-TRAP: a Rust-side panic is contained to this call instead of taking the task
        // down; it becomes an `ExtensionCallError` and disables the extension for the
        // session. The async block covers the synchronous prefix too — some instances
        // run handler bodies while building the call future.
        let mut active = ACTIVE_EXTENSIONS.try_with(Clone::clone).unwrap_or_default();
        active.push(std::ptr::from_ref(extension) as usize);
        let outcome = ACTIVE_EXTENSIONS
            .scope(
                active,
                AssertUnwindSafe(async {
                    if let Err(error) = extension.instance.prepare_event(kind).await {
                        return Some(Err(error));
                    }
                    let call = if let Some(encoded) = encoded {
                        extension
                            .instance
                            .on_event_json(kind, encoded, self.cx.clone())
                    } else {
                        extension.instance.on_event(
                            kind,
                            payload.unwrap_or(JsonValue::Null),
                            self.cx.clone(),
                        )
                    };
                    crate::extensions::deadline::with_deadline(call, deadline, waiting).await
                })
                .catch_unwind(),
            )
            .await;
        match outcome {
            Ok(Some(result)) => {
                if let Err(error) = &result
                    && error.trap
                {
                    self.disable(extension, ExtensionDisabledReason::Trap);
                }
                result
            }
            Ok(None) => {
                if let Some(ui) = &self.dialogs {
                    ui.cancel_for(id);
                }
                Err(ExtensionCallError::timeout(format!(
                    "Extension handler for {kind} timed out after {}ms",
                    deadline.as_millis()
                )))
            }
            Err(panic) => {
                let error = ExtensionCallError {
                    message: format!("extension panicked: {}", panic_message(&*panic)),
                    trap: true,
                    timed_out: false,
                };
                self.disable(extension, ExtensionDisabledReason::Panic);
                Err(error)
            }
        }
    }

    fn report(&self, extension: &LoadedExtension, kind: &str, error: &ExtensionCallError) {
        // Selection is already committed. Observer failures must not look like a
        // failed user action (or expose runtime cancellation errors in a red toast).
        if kind == "model_select" {
            tracing::warn!(extension = %extension.instance.id(), event = kind, error = %error.message, "model selection observer failed; selection retained");
            return;
        }
        let message = if error.message.contains("component call cancelled") {
            "An extension stopped while handling this action. Please try again.".to_owned()
        } else {
            error.message.clone()
        };
        self.emit_error(&extension.instance.path(), kind, message);
    }

    /// Generic dispatch: notification events, plus the `session_before_*` gates
    /// where the first result with `cancel: true` stops dispatch and the last result wins.
    pub async fn emit(&self, payload: JsonValue) -> Option<JsonValue> {
        self.emit_except(payload, None).await
    }

    /// Notify observers in load order, excluding the extension that made the change.
    /// A synchronous host import must not call back into its still-busy guest actor.
    pub(crate) async fn emit_except(
        &self,
        payload: JsonValue,
        origin: Option<&str>,
    ) -> Option<JsonValue> {
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        // Only notification dispatch is detached. In particular, turn_end's
        // result-bearing emit_boundary path must keep its draft/continue chain.
        if matches!(
            kind.as_str(),
            "message_update" | "turn_end" | "oino_timer" | "oino_bus"
        ) {
            for extension in self.handlers(&kind) {
                // Enqueue before returning, so the next lifecycle call cannot overtake it.
                let reply = self
                    .enqueue(extension.clone(), &kind, Some(payload.clone()), None)
                    .await;
                let runner = self.clone();
                let kind = kind.clone();
                tokio::spawn(async move {
                    if let Ok(Err(error)) = reply.await {
                        runner.report(&extension, &kind, &error);
                    }
                });
            }
            return None;
        }
        let gate = kind.starts_with("session_before_");
        let mut result = None;
        for extension in self.handlers(&kind) {
            if origin == Some(extension.instance.id()) {
                continue;
            }
            match self.call(&extension, &kind, payload.clone()).await {
                Ok(Some(value)) if gate && !value.is_null() => {
                    let cancel = value.get("cancel") == Some(&JsonValue::Bool(true));
                    result = Some(value);
                    if cancel {
                        return result;
                    }
                }
                Ok(_) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
        }
        result
    }

    /// Consumable dispatch (`oino_editor` intents, ui-protocol §7.2): `true` once a handler
    /// returns `{consume: true}`; later handlers are not called.
    pub async fn emit_consumable(&self, payload: JsonValue) -> bool {
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        for extension in self.handlers(&kind) {
            match self.call(&extension, &kind, payload.clone()).await {
                Ok(Some(value)) if value.get("consume") == Some(&JsonValue::Bool(true)) => {
                    return true;
                }
                Ok(_) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
        }
        false
    }

    /// Chain over one payload field: each handler's returned `{field: value}` replaces the
    /// field for the next handler (`before_provider_headers`, `before_provider_request`).
    /// Returns the final field value.
    pub async fn emit_chain(&self, mut payload: JsonValue, field: &str) -> JsonValue {
        let kind = payload["type"].as_str().unwrap_or_default().to_owned();
        for extension in self.handlers(&kind) {
            match self.call(&extension, &kind, payload.clone()).await {
                Ok(Some(value)) => {
                    if let Some(next) = value.get(field) {
                        payload[field] = next.clone();
                    } else if !value.is_object() {
                        payload[field] = value;
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
        }
        payload[field].take()
    }

    /// `tool_call` gate: the first `block` result wins; a failing handler blocks with
    /// `Extension error: {message}`. Returned `input` replaces the arguments (Oino).
    pub async fn emit_tool_call(&self, mut payload: JsonValue) -> Option<JsonValue> {
        let mut result: Option<JsonValue> = None;
        for extension in self.handlers("tool_call") {
            match self.call(&extension, "tool_call", payload.clone()).await {
                Ok(Some(value)) if value.is_object() => {
                    if let Some(input) = value.get("input").filter(|v| v.is_object()) {
                        payload["input"] = input.clone();
                    }
                    let block = value.get("block") == Some(&JsonValue::Bool(true));
                    result = Some(value);
                    if block {
                        return result;
                    }
                }
                Ok(_) => {}
                Err(error) => {
                    self.report(&extension, "tool_call", &error);
                    return Some(json!({
                        "block": true,
                        "reason": format!("Extension error: {}", error.message),
                    }));
                }
            }
        }
        if let (Some(result), Some(input)) = (result.as_mut(), payload.get("input"))
            && result.get("input").is_some()
        {
            result["input"] = input.clone();
        }
        result
    }

    /// `tool_result` chain: returns the combined override, if any.
    pub async fn emit_tool_result(&self, mut event: JsonObject) -> Option<JsonObject> {
        let mut modified = false;
        for extension in self.handlers("tool_result") {
            let result = self
                .call(&extension, "tool_result", JsonValue::Object(event.clone()))
                .await;
            let update = match result {
                Ok(Some(JsonValue::Object(update))) => update,
                Ok(_) => continue,
                Err(error) => {
                    self.report(&extension, "tool_result", &error);
                    continue;
                }
            };
            if let Some(content) = update.get("content").filter(|v| !v.is_null()) {
                event.insert("content".into(), content.clone());
                if update
                    .get("structuredContent")
                    .is_none_or(JsonValue::is_null)
                {
                    event.shift_remove("structuredContent");
                }
                modified = true;
            }
            for key in ["details", "structuredContent", "isError", "usage"] {
                if let Some(value) = update.get(key).filter(|v| !v.is_null()) {
                    event.insert(key.into(), value.clone());
                    modified = true;
                }
            }
        }
        modified.then(|| {
            [
                "content",
                "details",
                "structuredContent",
                "isError",
                "usage",
            ]
            .into_iter()
            .filter_map(|key| event.get(key).map(|v| (key.to_owned(), v.clone())))
            .collect()
        })
    }

    /// `message_end` chain: handlers may return a replacement with the same role.
    pub async fn emit_message_end(&self, message: &AgentMessage) -> Option<AgentMessage> {
        let mut current = message.clone();
        let mut modified = false;
        for extension in self.handlers("message_end") {
            let payload = json!({"type": "message_end", "message": current});
            match self.call(&extension, "message_end", payload).await {
                Ok(Some(value)) => {
                    let Some(replacement) = value.get("message").filter(|m| !m.is_null()) else {
                        continue;
                    };
                    let mut replacement = replacement.clone();
                    let role = replacement.get("role").and_then(JsonValue::as_str);
                    if role != Some(role_of(&current)) {
                        self.emit_error(
                            &extension.instance.path(),
                            "message_end",
                            "message_end handlers must return a message with the same role",
                        );
                        continue;
                    }
                    if replacement.get("content").is_none_or(JsonValue::is_null)
                        && matches!(role, Some("user" | "assistant" | "toolResult" | "custom"))
                    {
                        replacement["content"] = json!([]);
                    }
                    match serde_json::from_value::<AgentMessage>(replacement) {
                        Ok(parsed) => {
                            current = parsed;
                            modified = true;
                        }
                        Err(error) => {
                            self.emit_error(
                                &extension.instance.path(),
                                "message_end",
                                error.to_string(),
                            );
                        }
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "message_end", &error),
            }
        }
        modified.then_some(current)
    }

    /// `input` chain: `transform` replaces text/images for the next handler, `handled` stops.
    pub async fn emit_input(
        &self,
        text: &str,
        images: Option<&[ImageContent]>,
        source: &str,
        streaming_behavior: Option<&str>,
    ) -> InputOutcome {
        let mut current_text = text.to_owned();
        let mut current_images: Option<Vec<ImageContent>> = images.map(<[ImageContent]>::to_vec);
        let original_images = current_images.clone();
        for extension in self.handlers("input") {
            let mut payload = json!({"type": "input", "text": current_text, "source": source});
            if let Some(images) = &current_images {
                payload["images"] = images_json(images);
            }
            if let Some(behavior) = streaming_behavior {
                payload["streamingBehavior"] = json!(behavior);
            }
            match self.call(&extension, "input", payload).await {
                Ok(Some(value)) => match value.get("action").and_then(JsonValue::as_str) {
                    Some("handled") => return InputOutcome::Handled,
                    Some("transform") => {
                        if let Some(text) = value.get("text").and_then(JsonValue::as_str) {
                            text.clone_into(&mut current_text);
                        }
                        if let Some(images) = value
                            .get("images")
                            .filter(|v| !v.is_null())
                            .and_then(|v| serde_json::from_value(v.clone()).ok())
                        {
                            current_images = Some(images);
                        }
                    }
                    _ => {}
                },
                Ok(None) => {}
                Err(error) => self.report(&extension, "input", &error),
            }
        }
        if current_text != text || current_images != original_images {
            InputOutcome::Transform {
                text: current_text,
                images: current_images,
            }
        } else {
            InputOutcome::Continue
        }
    }

    /// `context` then `context_with_system`, on the unsent tail supplied by the
    /// session. The session protects the sent prefix and persists accepted edits
    /// before advancing its durable sent frontier.
    pub async fn emit_context(&self, messages: Vec<AgentMessage>) -> Vec<AgentMessage> {
        let mut current = messages;
        let mut encoded: Option<Arc<str>> = None;
        for extension in self.handlers("context") {
            let payload = encoded
                .get_or_insert_with(|| {
                    let visible: Vec<_> = current
                        .iter()
                        .filter(|m| !matches!(m, AgentMessage::System(_)))
                        .collect();
                    serde_json::to_string(&json!({"type": "context", "messages": visible}))
                        .unwrap_or_default()
                        .into()
                })
                .clone();
            match self
                .call_payload(&extension, "context", None, Some(payload))
                .await
            {
                Ok(Some(value)) => {
                    let Some(returned) = value
                        .get("messages")
                        .filter(|v| !v.is_null())
                        .and_then(|v| serde_json::from_value::<Vec<AgentMessage>>(v.clone()).ok())
                    else {
                        continue;
                    };
                    if returned.iter().eq(current
                        .iter()
                        .filter(|m| !matches!(m, AgentMessage::System(_))))
                    {
                        continue;
                    }
                    let head = crate::session::current_system_message(&current);
                    current = head
                        .map(AgentMessage::System)
                        .into_iter()
                        .chain(returned)
                        .collect();
                    encoded = None;
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "context", &error),
            }
        }
        encoded = None;
        for extension in self.handlers("context_with_system") {
            let had_head = matches!(current.first(), Some(AgentMessage::System(_)));
            let payload = encoded
                .get_or_insert_with(|| {
                    serde_json::to_string(
                        &json!({"type": "context_with_system", "messages": current}),
                    )
                    .unwrap_or_default()
                    .into()
                })
                .clone();
            match self
                .call_payload(&extension, "context_with_system", None, Some(payload))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(returned) = value
                        .get("messages")
                        .filter(|v| !v.is_null())
                        .and_then(|v| serde_json::from_value::<Vec<AgentMessage>>(v.clone()).ok())
                        && current != returned
                    {
                        current = returned;
                        encoded = None;
                    }
                    if had_head && !matches!(current.first(), Some(AgentMessage::System(_))) {
                        self.emit_error(&extension.instance.path(), "context_with_system", "Handler removed the leading system message; the request has no prompt or initial tool declarations. Keep it at index 0 or replace a dropped prefix with getCurrentSystemMessage().");
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "context_with_system", &error),
            }
        }
        current
    }

    /// `before_agent_start`: `systemPromptOptions` chain (returned options replace the
    /// current ones), `message`s accumulate, `systemPrompt` last wins. `render` renders the
    /// current options for the event's `systemPrompt` field.
    pub async fn emit_before_agent_start(
        &self,
        trigger: JsonObject,
        options: JsonValue,
        render: &(dyn Fn(&JsonValue) -> String + Send + Sync),
    ) -> BeforeAgentStartOutcome {
        let mut current = options;
        let mut messages = Vec::new();
        let mut forced = None;
        for extension in self.handlers("before_agent_start") {
            let mut payload = trigger.clone();
            payload.insert("type".into(), json!("before_agent_start"));
            payload.insert(
                "systemPrompt".into(),
                json!(forced.clone().unwrap_or_else(|| render(&current))),
            );
            payload.insert("systemPromptOptions".into(), current.clone());
            let input = current.clone();
            let id = extension.instance.id().to_owned();
            match self
                .call(&extension, "before_agent_start", JsonValue::Object(payload))
                .await
            {
                Ok(result) => {
                    let value = result.unwrap_or(JsonValue::Null);
                    if let Some(message) = value.get("message").filter(|m| m.is_object()) {
                        messages.push(message.clone());
                    }
                    let options = value
                        .get("systemPromptOptions")
                        .filter(|o| o.is_object())
                        .cloned();
                    if let Some(options) = &options {
                        current = options.clone();
                    }
                    let prompt = value
                        .get("systemPrompt")
                        .and_then(JsonValue::as_str)
                        .map(str::to_owned);
                    if let Some(prompt) = &prompt {
                        forced = Some(prompt.clone());
                    }
                    lock(&self.sticky_prompt).insert(
                        id,
                        StickyPrompt {
                            input,
                            options,
                            forced: prompt,
                        },
                    );
                }
                Err(error) => {
                    self.report(&extension, "before_agent_start", &error);
                    let sticky = lock(&self.sticky_prompt).get(&id).cloned();
                    if error.timed_out
                        && let Some(sticky) = sticky.filter(|s| s.input == input)
                    {
                        if let Some(options) = sticky.options {
                            current = options;
                        }
                        if let Some(prompt) = sticky.forced {
                            forced = Some(prompt);
                        }
                    }
                }
            }
        }
        BeforeAgentStartOutcome {
            messages,
            system_prompt_options: current,
            system_prompt: forced,
        }
    }

    /// `resources_discover`: every result merged in order.
    pub async fn emit_resources_discover(&self, cwd: &str, reason: &str) -> DiscoveredResources {
        let mut found = DiscoveredResources::default();
        for extension in self.handlers("resources_discover") {
            let payload = json!({"type": "resources_discover", "cwd": cwd, "reason": reason});
            match self.call(&extension, "resources_discover", payload).await {
                Ok(Some(value)) => {
                    let id = extension.instance.id().to_owned();
                    let collect = |key: &str, into: &mut Vec<(String, String)>| {
                        for path in value
                            .get(key)
                            .and_then(JsonValue::as_array)
                            .into_iter()
                            .flatten()
                        {
                            if let Some(path) = path.as_str() {
                                into.push((path.to_owned(), id.clone()));
                            }
                        }
                    };
                    collect("skillPaths", &mut found.skill_paths);
                    collect("promptPaths", &mut found.prompt_paths);
                    collect("themePaths", &mut found.theme_paths);
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "resources_discover", &error),
            }
        }
        found
    }

    /// Boundary dispatch. `context` rebuilds the preview after each
    /// handler; an invalid draft list discards everything.
    pub async fn emit_boundary(
        &self,
        base: JsonObject,
        context: &(dyn Fn(&[JsonValue]) -> Result<JsonValue, String> + Send + Sync),
    ) -> BoundaryOutcome {
        let kind = base
            .get("type")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        let mut entries: Vec<JsonValue> = Vec::new();
        let mut continue_run = false;
        let mut preview = context(&entries).unwrap_or(JsonValue::Null);
        let mut valid = true;
        for extension in self.handlers(&kind) {
            let mut payload = base.clone();
            payload.insert("entries".into(), JsonValue::Array(entries.clone()));
            payload.insert("continue".into(), JsonValue::Bool(continue_run));
            payload.insert("context".into(), preview.clone());
            match self
                .call(&extension, &kind, JsonValue::Object(payload))
                .await
            {
                Ok(Some(value)) => {
                    if let Some(list) = value.get("entries").and_then(JsonValue::as_array) {
                        entries.clone_from(list);
                    }
                    if let Some(flag) = value.get("continue").and_then(JsonValue::as_bool) {
                        continue_run = flag;
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, &kind, &error),
            }
            match context(&entries) {
                Ok(next) => {
                    preview = next;
                    valid = true;
                }
                Err(error) => {
                    valid = false;
                    self.emit_error(
                        &extension.instance.path(),
                        &kind,
                        format!("Invalid boundary entries: {error}"),
                    );
                }
            }
        }
        if valid {
            BoundaryOutcome {
                entries,
                continue_run,
            }
        } else {
            BoundaryOutcome {
                entries: Vec::new(),
                continue_run: false,
            }
        }
    }

    /// `user_bash` gate: the first `{result}` wins; failures block (`Err`). `{operations}`
    /// cannot be supplied by WASM extensions and is treated as no result (Oino delta).
    ///
    /// # Errors
    ///
    /// The failing handler's error text.
    pub async fn emit_user_bash(&self, payload: JsonValue) -> Result<Option<JsonValue>, String> {
        for extension in self.handlers("user_bash") {
            match self.call(&extension, "user_bash", payload.clone()).await {
                Ok(Some(value)) => {
                    if let Some(result) = value.get("result").filter(|r| r.is_object()) {
                        return Ok(Some(result.clone()));
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    self.report(&extension, "user_bash", &error);
                    return Err(error.message);
                }
            }
        }
        Ok(None)
    }

    /// `cache_warming_decision`: the last returned `action` wins.
    pub async fn emit_cache_warming_decision(&self, payload: JsonValue, action: String) -> String {
        let mut action = action;
        for extension in self.handlers("cache_warming_decision") {
            match self
                .call(&extension, "cache_warming_decision", payload.clone())
                .await
            {
                Ok(Some(value)) => {
                    if let Some(next) = value.get("action").and_then(JsonValue::as_str) {
                        next.clone_into(&mut action);
                    }
                }
                Ok(None) => {}
                Err(error) => self.report(&extension, "cache_warming_decision", &error),
            }
        }
        action
    }

    /// Arbiter wait sources (runtime §7.6): asks every `wait-source` concurrently (2 s each)
    /// and returns the reasons in load order.
    pub async fn arbiter_wait_reasons(&self) -> Vec<String> {
        let sources: Vec<Arc<LoadedExtension>> = self
            .active()
            .filter(|e| e.state().arbiter.wait_source)
            .cloned()
            .collect();
        let calls = sources.iter().map(|extension| {
            let cx = self.cx.clone();
            async move {
                tokio::time::timeout(ARBITER_DEADLINE, extension.instance.wait_reason(cx))
                    .await
                    .ok()
                    .flatten()
            }
        });
        futures::future::join_all(calls)
            .await
            .into_iter()
            .flatten()
            .filter(|reason| !reason.trim().is_empty())
            .collect()
    }

    /// Arbiter nudge (runtime §7.6): asks every `nudge-provider` concurrently (2 s each),
    /// picks the highest `priority` (ties: earliest loaded), tells the winner
    /// `nudge-delivered(id)` and returns its proposal.
    pub async fn arbiter_nudge(&self) -> Option<JsonValue> {
        let providers: Vec<Arc<LoadedExtension>> = self
            .active()
            .filter(|e| e.state().arbiter.nudge_provider)
            .cloned()
            .collect();
        let calls = providers.iter().map(|extension| {
            let cx = self.cx.clone();
            async move {
                tokio::time::timeout(ARBITER_DEADLINE, extension.instance.propose_nudge(cx))
                    .await
                    .ok()
                    .flatten()
                    .filter(|proposal| proposal.get("customType").is_some_and(JsonValue::is_string))
            }
        });
        let proposals = futures::future::join_all(calls).await;
        let priority = |p: &JsonValue| p.get("priority").and_then(JsonValue::as_f64).unwrap_or(0.0);
        let mut winner: Option<(usize, JsonValue)> = None;
        for (index, proposal) in proposals.into_iter().enumerate() {
            let Some(proposal) = proposal else { continue };
            if winner
                .as_ref()
                .is_none_or(|(_, best)| priority(&proposal) > priority(best))
            {
                winner = Some((index, proposal));
            }
        }
        let (index, proposal) = winner?;
        let id = proposal
            .get("id")
            .and_then(JsonValue::as_str)
            .unwrap_or_default()
            .to_owned();
        providers[index]
            .instance
            .nudge_delivered(&id, self.cx.clone());
        Some(proposal)
    }

    /// Registered tools across extensions; the first registration per name wins.
    #[must_use]
    pub fn tools(&self) -> Vec<RegisteredTool> {
        let mut seen = HashSet::new();
        let mut tools = Vec::new();
        for extension in self.active() {
            for spec in extension.state().tools.into_values() {
                if seen.insert(spec.name.clone()) {
                    tools.push(RegisteredTool {
                        extension: Arc::clone(extension),
                        spec,
                    });
                }
            }
        }
        tools
    }

    /// Commands with invocation names.
    #[must_use]
    pub fn commands(&self) -> Vec<ResolvedCommand> {
        resolve_commands(self.active())
    }

    /// Returns the command matching an invocation name.
    #[must_use]
    pub fn command(&self, invocation_name: &str) -> Option<ResolvedCommand> {
        self.commands()
            .into_iter()
            .find(|command| command.invocation_name == invocation_name)
    }

    /// Flags (first registration per name wins).
    #[must_use]
    pub fn flags(&self) -> IndexMap<String, FlagSpec> {
        let mut flags = IndexMap::new();
        for extension in self.active() {
            for (name, flag) in extension.state().flags {
                flags.entry(name).or_insert(flag);
            }
        }
        flags
    }

    /// Flag value: CLI value, else the flag default.
    #[must_use]
    pub fn flag_value(&self, name: &str) -> Option<JsonValue> {
        lock(&self.flag_values)
            .get(name)
            .cloned()
            .or_else(|| self.flags().get(name).and_then(|f| f.default.clone()))
    }

    /// Sets a flag value.
    pub fn set_flag_value(&self, name: &str, value: JsonValue) {
        lock(&self.flag_values).insert(name.to_owned(), value);
    }

    /// Every flag value set on the CLI.
    #[must_use]
    pub fn flag_values(&self) -> IndexMap<String, JsonValue> {
        lock(&self.flag_values).clone()
    }

    /// Shortcuts with their owners (later registrations of the same id win, like map keys).
    #[must_use]
    pub fn shortcuts(&self) -> Vec<(String, ShortcutSpec)> {
        let mut shortcuts: IndexMap<String, (String, ShortcutSpec)> = IndexMap::new();
        for extension in self.active() {
            for (id, spec) in extension.state().shortcuts {
                shortcuts.insert(id, (extension.instance.id().to_owned(), spec));
            }
        }
        shortcuts.into_values().collect()
    }

    /// Every render spec in load order.
    #[must_use]
    pub fn renderers(&self) -> Vec<(String, JsonValue)> {
        self.active()
            .flat_map(|extension| {
                let id = extension.instance.id().to_owned();
                extension
                    .state()
                    .renderers
                    .into_iter()
                    .map(move |spec| (id.clone(), spec))
            })
            .collect()
    }

    /// The provider component implementing `api` (first in load order).
    #[must_use]
    pub fn provider_for_api(&self, api: &str) -> Option<Arc<LoadedExtension>> {
        self.set
            .extensions()
            .iter()
            .find(|extension| {
                extension.instance.provider().is_some()
                    && extension.state().apis.iter().any(|a| a == api)
            })
            .cloned()
    }
}

/// Role string of a message.
#[must_use]
pub fn role_of(message: &AgentMessage) -> &'static str {
    match message {
        AgentMessage::System(_) => "system",
        AgentMessage::User(_) => "user",
        AgentMessage::Assistant(_) => "assistant",
        AgentMessage::ToolResult(_) => "toolResult",
        AgentMessage::BashExecution(_) => "bashExecution",
        AgentMessage::Custom(_) => "custom",
        AgentMessage::BranchSummary(_) => "branchSummary",
        AgentMessage::CompactionSummary(_) => "compactionSummary",
    }
}

fn images_json(images: &[ImageContent]) -> JsonValue {
    JsonValue::Array(
        images
            .iter()
            .map(|image| json!({"type": "image", "data": image.data, "mimeType": image.mime_type}))
            .collect(),
    )
}

#[cfg(test)]
mod observer_tests {
    use super::super::api::*;
    use super::*;
    use futures::future::BoxFuture;
    use oino_agent::{AgentToolResult, ToolInvocation, UpdateSink};
    use tokio_util::sync::CancellationToken;

    struct Sink;
    impl ErrorSink for Sink {
        fn report(&self, _: ExtensionErrorEvent) {}
        fn disabled(&self, _: OinoEvent) {}
    }

    struct Probe {
        id: String,
        seen: Arc<Mutex<Vec<String>>>,
        barrier: Arc<tokio::sync::Barrier>,
        lifecycle: bool,
        nested: Option<Arc<Mutex<Option<ExtensionRunner>>>>,
        slow: bool,
    }
    impl ExtensionInstance for Probe {
        fn id(&self) -> &str {
            &self.id
        }
        fn init(&self, _: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>> {
            Box::pin(async { Ok(Registration::default()) })
        }
        fn on_event(
            &self,
            kind: &str,
            payload: JsonValue,
            _: CallContext,
        ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
            let observer = kind == "message_update";
            let reenter = kind == "message_start" || observer;
            let runner = self.nested.as_ref().and_then(|r| lock(r).clone());
            Box::pin(async move {
                if observer {
                    self.barrier.wait().await;
                }
                if reenter && let Some(runner) = runner {
                    let context = HookContext::capture();
                    // Model the actor task hop made by the wasm host.
                    tokio::spawn(async move {
                        context
                            .scope(runner.emit(json!({"type":"ui_prompt_start", "seq":0})))
                            .await;
                    })
                    .await
                    .unwrap();
                }
                if self.slow {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                if self.lifecycle {
                    lock(&self.seen).push(format!(
                        "{}:{}",
                        payload["seq"],
                        payload["type"].as_str().unwrap()
                    ));
                } else {
                    lock(&self.seen).push(self.id.clone());
                }
                Ok(Some(
                    json!({"value": format!("{}{}", payload["value"].as_str().unwrap_or_default(), self.id)}),
                ))
            })
        }
        fn run_tool(
            &self,
            _: &str,
            _: ToolInvocation,
            _: UpdateSink,
            _: CancellationToken,
            _: CallContext,
        ) -> BoxFuture<'_, Result<AgentToolResult, ExtensionCallError>> {
            Box::pin(async { Ok(AgentToolResult::text("ok")) })
        }
        fn run_command(
            &self,
            _: &str,
            _: &str,
            _: CancellationToken,
            _: CallContext,
        ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
            Box::pin(async { Ok(None) })
        }
    }

    #[tokio::test(start_paused = true)]
    async fn unrelated_approval_pauses_a_queued_tool_call_hook() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let seen = Arc::new(Mutex::new(Vec::new()));
        let instance = Arc::new(Probe {
            id: "ask-user-probe".into(),
            seen: seen.clone(),
            barrier: Arc::new(tokio::sync::Barrier::new(2)),
            lifecycle: false,
            nested: None,
            slow: true,
        });
        let extension = Arc::new(LoadedExtension::new(
            instance.clone(),
            Registration::default(),
        ));
        let mut runner = ExtensionRunner::new(
            Arc::new(ExtensionSet::new(vec![extension.clone()])),
            CallContext::default(),
            Arc::new(Sink),
            IndexMap::new(),
        );
        let pending = Arc::new(AtomicBool::new(true));
        let flag = pending.clone();
        runner.set_approval_pending(Arc::new(move || flag.load(Ordering::SeqCst)));
        // Model a hook awaiting an actor/observer behind another extension's approval.
        let r = runner.clone();
        let ext = extension.clone();
        let task = tokio::spawn(async move {
            r.deliver(&ext, "message_update", Some(json!({})), None)
                .await
        });
        tokio::task::yield_now().await;
        tokio::time::advance(Duration::from_secs(90)).await;
        tokio::task::yield_now().await;
        assert!(!task.is_finished());
        pending.store(false, Ordering::SeqCst);
        instance.barrier.wait().await;
        assert!(task.await.unwrap().is_ok());
        assert_eq!(*lock(&seen), ["ask-user-probe"]);
    }

    #[tokio::test]
    async fn observers_return_before_completion_and_run_concurrently_but_chains_stay_ordered() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let barrier = Arc::new(tokio::sync::Barrier::new(3));
        let extensions = ["a", "b"]
            .into_iter()
            .map(|id| {
                let instance = Arc::new(Probe {
                    id: id.into(),
                    seen: seen.clone(),
                    barrier: barrier.clone(),
                    lifecycle: false,
                    nested: None,
                    slow: false,
                });
                Arc::new(LoadedExtension::new(
                    instance,
                    Registration {
                        events: vec!["message_update".into(), "before_provider_request".into()],
                        ..Registration::default()
                    },
                ))
            })
            .collect();
        let runner = ExtensionRunner::new(
            Arc::new(ExtensionSet::new(extensions)),
            CallContext::default(),
            Arc::new(Sink),
            IndexMap::new(),
        );
        runner.emit(json!({"type": "message_update"})).await;
        assert!(lock(&seen).is_empty());
        // Neither observer can finish unless both are in flight together.
        tokio::time::timeout(Duration::from_secs(1), barrier.wait())
            .await
            .unwrap();
        tokio::time::timeout(Duration::from_secs(1), async {
            while lock(&seen).len() != 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        lock(&seen).clear();
        assert_eq!(
            runner
                .emit_chain(
                    json!({"type": "before_provider_request", "value": ""}),
                    "value"
                )
                .await,
            json!("ab")
        );
        assert_eq!(*lock(&seen), vec!["a", "b"]);
    }
    fn single_probe(
        slow: bool,
        nested: Option<Arc<Mutex<Option<ExtensionRunner>>>>,
    ) -> (ExtensionRunner, Arc<Mutex<Vec<String>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let instance = Arc::new(Probe {
            id: "probe".into(),
            seen: seen.clone(),
            barrier: Arc::new(tokio::sync::Barrier::new(1)),
            lifecycle: true,
            nested,
            slow,
        });
        let runner = ExtensionRunner::new(
            Arc::new(ExtensionSet::new(vec![Arc::new(LoadedExtension::new(
                instance,
                Registration {
                    events: vec![
                        "message_start".into(),
                        "message_update".into(),
                        "message_end".into(),
                        "ui_prompt_start".into(),
                    ],
                    ..Default::default()
                },
            ))])),
            CallContext::default(),
            Arc::new(Sink),
            IndexMap::new(),
        );
        (runner, seen)
    }

    #[tokio::test]
    #[allow(clippy::print_stderr, reason = "regression timing diagnostic")]
    async fn reentrant_inline_and_observer_hooks_do_not_fence_themselves() {
        let slot = Arc::new(Mutex::new(None));
        let (runner, seen) = single_probe(false, Some(slot.clone()));
        *lock(&slot) = Some(runner.clone());
        let started = std::time::Instant::now();
        tokio::time::timeout(Duration::from_millis(500), async {
            runner.emit(json!({"type":"message_start", "seq":1})).await;
            runner.emit(json!({"type":"message_update", "seq":2})).await;
            runner.emit(json!({"type":"message_end", "seq":3})).await;
        })
        .await
        .unwrap();
        assert_eq!(
            *lock(&seen),
            vec![
                "0:ui_prompt_start",
                "1:message_start",
                "0:ui_prompt_start",
                "2:message_update",
                "3:message_end"
            ]
        );
        eprintln!(
            "reentrant_inline_and_observer_us={}",
            started.elapsed().as_micros()
        );
        *lock(&slot) = None;
    }

    #[tokio::test]
    async fn model_selection_origin_skips_only_caller_and_preserves_observer_order() {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let extensions = ["caller", "a", "b"]
            .into_iter()
            .map(|id| {
                Arc::new(LoadedExtension::new(
                    Arc::new(Probe {
                        id: id.into(),
                        seen: seen.clone(),
                        barrier: Arc::new(tokio::sync::Barrier::new(1)),
                        lifecycle: false,
                        nested: None,
                        slow: false,
                    }),
                    Registration {
                        events: vec!["model_select".into()],
                        ..Default::default()
                    },
                ))
            })
            .collect();
        let runner = ExtensionRunner::new(
            Arc::new(ExtensionSet::new(extensions)),
            CallContext::default(),
            Arc::new(Sink),
            IndexMap::new(),
        );
        runner
            .emit_except(json!({"type":"model_select"}), Some("caller"))
            .await;
        assert_eq!(*lock(&seen), ["a", "b"]);
        lock(&seen).clear();
        runner.emit(json!({"type":"model_select"})).await;
        assert_eq!(*lock(&seen), ["caller", "a", "b"]);
    }

    #[tokio::test]
    async fn cancelling_result_hook_drops_handler_without_later_effects() {
        let (runner, seen) = single_probe(true, None);
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                runner.emit(json!({"type":"message_end", "seq":1}))
            )
            .await
            .is_err()
        );
        drop(runner);
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert!(
            lock(&seen).is_empty(),
            "handler continued after its caller was cancelled"
        );
    }

    #[tokio::test]
    async fn cancelled_hook_waiting_for_observers_never_starts() {
        let (runner, seen) = single_probe(true, None);
        runner.emit(json!({"type":"message_update", "seq":1})).await;
        assert!(
            tokio::time::timeout(
                Duration::from_millis(10),
                runner.emit(json!({"type":"message_end", "seq":2}))
            )
            .await
            .is_err()
        );
        tokio::time::sleep(Duration::from_millis(150)).await;
        assert_eq!(*lock(&seen), vec!["1:message_update"]);
    }

    #[tokio::test]
    async fn queued_observer_without_requester_is_skipped() {
        let (runner, seen) = single_probe(true, None);
        let extension = runner.set.extensions()[0].clone();
        let reply = runner
            .enqueue(
                extension,
                "message_update",
                Some(json!({"type":"message_update", "seq":1})),
                None,
            )
            .await;
        drop(reply);
        runner.emit(json!({"type":"message_end", "seq":2})).await;
        assert_eq!(*lock(&seen), vec!["2:message_end"]);
    }

    async fn lifecycle_stress(count: usize) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let instance = Arc::new(Probe {
            id: "ordered".into(),
            seen: seen.clone(),
            barrier: Arc::new(tokio::sync::Barrier::new(1)),
            lifecycle: true,
            nested: None,
            slow: false,
        });
        let runner = ExtensionRunner::new(
            Arc::new(ExtensionSet::new(vec![Arc::new(LoadedExtension::new(
                instance,
                Registration {
                    events: vec![
                        "message_update".into(),
                        "message_end".into(),
                        "session_shutdown".into(),
                    ],
                    ..Registration::default()
                },
            ))])),
            CallContext::default(),
            Arc::new(Sink),
            IndexMap::new(),
        );
        for seq in 0..count {
            runner
                .emit(json!({"type":"message_update", "seq":seq}))
                .await;
        }
        runner
            .emit(json!({"type":"message_end", "seq":count}))
            .await;
        runner
            .emit(json!({"type":"session_shutdown", "seq":count+1}))
            .await;
        let mut expected: Vec<_> = (0..count)
            .map(|seq| format!("{seq}:message_update"))
            .collect();
        expected.push(format!("{count}:message_end"));
        expected.push(format!("{}:session_shutdown", count + 1));
        assert_eq!(*lock(&seen), expected);
    }

    #[tokio::test]
    async fn update_cannot_be_overtaken_by_end_or_shutdown() {
        lifecycle_stress(1).await;
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn saturated_delivery_preserves_all_deltas_and_lifecycle_order() {
        tokio::time::timeout(Duration::from_secs(10), lifecycle_stress(4096))
            .await
            .unwrap();
    }
}
