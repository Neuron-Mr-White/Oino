//! One ACP session: the runtime, its in-process backend, and the event pump that turns
//! Oino client events into `session/update` notifications and client requests
//! (modes §6.3, §6.4).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use futures::StreamExt;
use oino_agent::AgentToolResult;
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessageEvent, JsonObject, JsonValue, StopReason,
};
use oino_client::{InProcessBackend, SessionBackend};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent};
use oino_core::runtime::AgentSessionRuntime;
use oino_core::session::AgentSession;
use oino_core::ui::{UiAnswer, UiRequest, UiResponse, is_dialog};
use oino_tools::env::SpawnRequest;
use serde_json::json;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::client::ClientLink;
use crate::commands::available_commands;
use crate::config::config_options;
use crate::convert::{last_assistant_stop, stop_reason, tool_call_created};
use crate::env::TerminalObserver;
use crate::tools::{FileSnapshot, result_content, tool_kind, tool_locations, tool_title};
use crate::ui::{Presentation, fire_update};

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// A tool call the client knows about.
#[derive(Debug, Default)]
struct ToolState {
    name: String,
    args: JsonObject,
    executing: bool,
    snapshot: Option<FileSnapshot>,
    terminal: Option<String>,
}

/// A `session/prompt` waiting for `agent_settled`.
#[derive(Debug)]
struct Waiter {
    id: u64,
    /// Resolve only after a run started (the prompt started one).
    needs_start: bool,
    started: bool,
    tx: oneshot::Sender<&'static str>,
}

#[derive(Debug, Default)]
struct State {
    tools: HashMap<String, ToolState>,
    /// Tool-call ids in execution order (the latest one owns `confirm` permissions).
    executing: Vec<String>,
    waiters: Vec<Waiter>,
    next_waiter: u64,
    cancel_requested: bool,
    compactions: u64,
    compaction: Option<String>,
}

/// State shared by the pump, prompt handlers and the execution environment.
pub(crate) struct Shared {
    link: ClientLink,
    cwd: PathBuf,
    state: Mutex<State>,
}

impl std::fmt::Debug for Shared {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Shared")
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

/// A registered prompt; await [`PromptTicket::settled`] for the stop reason.
pub(crate) struct PromptTicket {
    id: u64,
    rx: oneshot::Receiver<&'static str>,
}

impl PromptTicket {
    /// The stop reason (`cancelled` if the session went away).
    pub(crate) async fn settled(self) -> &'static str {
        self.rx.await.unwrap_or("cancelled")
    }
}

impl Shared {
    /// State for a session at `cwd` talking through `link`.
    pub(crate) fn new(link: ClientLink, cwd: PathBuf) -> Arc<Self> {
        Arc::new(Self {
            link,
            cwd,
            state: Mutex::new(State::default()),
        })
    }

    pub(crate) fn link(&self) -> &ClientLink {
        &self.link
    }

    fn update(&self, update: JsonValue) {
        if let Err(error) = self.link.update(update) {
            tracing::debug!("session/update dropped: {error}");
        }
    }

    /// Registers a prompt before it is sent (so its `agent_settled` cannot be missed).
    pub(crate) fn register_prompt(&self) -> PromptTicket {
        let (tx, rx) = oneshot::channel();
        let mut state = lock(&self.state);
        if state.waiters.is_empty() {
            state.cancel_requested = false;
        }
        state.next_waiter += 1;
        let id = state.next_waiter;
        state.waiters.push(Waiter {
            id,
            needs_start: true,
            started: false,
            tx,
        });
        PromptTicket { id, rx }
    }

    /// The prompt was queued into a running turn: it settles with that turn.
    pub(crate) fn prompt_queued(&self, ticket: &PromptTicket) {
        if let Some(waiter) = lock(&self.state)
            .waiters
            .iter_mut()
            .find(|w| w.id == ticket.id)
        {
            waiter.needs_start = false;
        }
    }

    /// Resolves a prompt now (handled by a command, failed to start).
    pub(crate) fn resolve_prompt(&self, ticket: &PromptTicket, reason: &'static str) {
        let mut state = lock(&self.state);
        if let Some(index) = state.waiters.iter().position(|w| w.id == ticket.id) {
            let waiter = state.waiters.remove(index);
            let _ = waiter.tx.send(reason);
        }
    }

    /// `session/cancel` arrived.
    pub(crate) fn request_cancel(&self) {
        lock(&self.state).cancel_requested = true;
    }

    fn latest_executing(&self) -> Option<String> {
        lock(&self.state).executing.last().cloned()
    }

    /// The `toolCall` (`ToolCallUpdate`) object of a permission request.
    fn permission_tool_call(&self, id: &str, title: &str, detail: Option<&str>) -> JsonValue {
        let state = lock(&self.state);
        let mut call = json!({ "toolCallId": id });
        if let Some(tool) = state.tools.get(id) {
            call["title"] = json!(tool_title(&tool.name, &tool.args));
            call["kind"] = json!(tool_kind(&tool.name));
            call["rawInput"] = JsonValue::Object(tool.args.clone());
            call["locations"] = json!(tool_locations(&tool.name, &tool.args, &self.cwd));
        }
        if !title.is_empty() {
            call["title"] = json!(title);
        }
        if let Some(detail) = detail {
            call["content"] =
                json!([{ "type": "content", "content": {"type": "text", "text": detail} }]);
        }
        call
    }
}

impl TerminalObserver for Shared {
    fn terminal_started(&self, request: &SpawnRequest, terminal_id: &str) {
        let script = request.args.last().map(String::as_str).unwrap_or_default();
        let owner = {
            let mut state = lock(&self.state);
            let owner = state
                .executing
                .iter()
                .rev()
                .find(|id| {
                    state.tools.get(*id).is_some_and(|tool| {
                        tool.terminal.is_none()
                            && tool
                                .args
                                .get("command")
                                .and_then(JsonValue::as_str)
                                .is_some_and(|command| script.contains(command))
                    })
                })
                .cloned();
            if let Some(tool) = owner.as_ref().and_then(|id| state.tools.get_mut(id)) {
                tool.terminal = Some(terminal_id.to_owned());
            }
            owner
        };
        if let Some(id) = owner {
            self.update(json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": id,
                "content": [{ "type": "terminal", "terminalId": terminal_id }],
            }));
        }
    }
}

/// One live ACP session.
pub(crate) struct AcpSession {
    /// ACP session id (the Oino id of the first session of the runtime).
    pub(crate) id: String,
    /// Session cwd.
    pub(crate) cwd: PathBuf,
    /// Commands and events.
    pub(crate) backend: Arc<InProcessBackend>,
    /// Pump/prompt state.
    pub(crate) shared: Arc<Shared>,
    pump: JoinHandle<()>,
}

impl std::fmt::Debug for AcpSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AcpSession")
            .field("id", &self.id)
            .field("cwd", &self.cwd)
            .finish_non_exhaustive()
    }
}

impl AcpSession {
    /// Wraps `runtime` and starts the event pump.
    pub(crate) fn start(runtime: Arc<AgentSessionRuntime>, shared: Arc<Shared>) -> Self {
        let id = runtime.session().session_id();
        shared.link.bind(&id);
        let cwd = shared.cwd.clone();
        let backend = Arc::new(InProcessBackend::new(runtime));
        let events = backend.events();
        let pump = tokio::spawn(pump(events, Arc::clone(&backend), Arc::clone(&shared)));
        Self {
            id,
            cwd,
            backend,
            shared,
            pump,
        }
    }

    /// The current session of the runtime.
    pub(crate) fn session(&self) -> AgentSession {
        self.backend.runtime().session()
    }

    /// Sends the updates that follow `session/new` / `session/load` responses.
    pub(crate) fn announce(&self) {
        self.shared.update(available_commands(&self.session()));
    }

    /// Disposes the runtime.
    pub(crate) async fn close(&self) {
        self.backend.shutdown().await;
        self.pump.abort();
        let waiters = std::mem::take(&mut lock(&self.shared.state).waiters);
        for waiter in waiters {
            let _ = waiter.tx.send("cancelled");
        }
    }
}

async fn pump(
    mut events: futures::stream::BoxStream<'static, ClientEvent>,
    backend: Arc<InProcessBackend>,
    shared: Arc<Shared>,
) {
    while let Some(event) = events.next().await {
        match event {
            ClientEvent::Session(event) => on_session_event(event, &backend, &shared).await,
            ClientEvent::Ui(request) => on_ui(request, &backend, &shared),
            ClientEvent::ExtensionError(error) => shared.update(json!({
                "sessionUpdate": "notice",
                "severity": "error",
                "title": format!("Extension error ({})", error.error.extension_path),
                "description": error.error.error,
            })),
            ClientEvent::Oino(event) => on_oino_event(&event, &backend, &shared),
        }
    }
}

fn usage_update(session: &AgentSession) -> Option<JsonValue> {
    let usage = session.context_usage()?;
    let used = usage.tokens?;
    Some(json!({
        "sessionUpdate": "usage_update",
        "used": used,
        "size": usage.context_window,
        "cost": { "amount": session.session_stats().cost, "currency": "USD" },
    }))
}

fn config_update(session: &AgentSession) -> JsonValue {
    json!({ "sessionUpdate": "config_option_update", "configOptions": config_options(session) })
}

#[allow(
    clippy::too_many_lines,
    reason = "one arm per AgentSessionEvent, mirroring the modes §6.3 table"
)]
async fn on_session_event(event: AgentSessionEvent, backend: &InProcessBackend, shared: &Shared) {
    let session = backend.runtime().session();
    match event {
        AgentSessionEvent::MessageUpdate {
            assistant_message_event,
            ..
        } => on_stream_event(assistant_message_event, shared),
        AgentSessionEvent::ToolExecutionStart {
            tool_call_id,
            tool_name,
            args,
            ..
        } => {
            let snapshot = FileSnapshot::take(&tool_name, &args, &shared.cwd).await;
            let known = {
                let mut state = lock(&shared.state);
                state.executing.push(tool_call_id.clone());
                let known = state.tools.contains_key(&tool_call_id);
                let tool = state.tools.entry(tool_call_id.clone()).or_default();
                tool.name.clone_from(&tool_name);
                tool.args.clone_from(&args);
                tool.executing = true;
                tool.snapshot = snapshot;
                known
            };
            if !known {
                shared.update(tool_call_created(
                    &tool_call_id,
                    &tool_name,
                    &args,
                    "in_progress",
                    &shared.cwd,
                ));
            }
            shared.update(json!({
                "sessionUpdate": "tool_call_update",
                "toolCallId": tool_call_id,
                "status": "in_progress",
            }));
        }
        AgentSessionEvent::ToolExecutionUpdate {
            tool_call_id,
            partial_result,
            ..
        } => {
            let terminal = lock(&shared.state)
                .tools
                .get(&tool_call_id)
                .is_some_and(|t| t.terminal.is_some());
            if !terminal {
                shared.update(json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": tool_call_id,
                    "content": result_content(&partial_result),
                }));
            }
        }
        AgentSessionEvent::ToolExecutionEnd {
            tool_call_id,
            result,
            is_error,
            ..
        } => on_tool_end(&tool_call_id, &result, is_error, shared).await,
        AgentSessionEvent::TurnEnd { .. } => {
            if let Some(update) = usage_update(&session) {
                shared.update(update);
            }
        }
        AgentSessionEvent::AgentStart => {
            for waiter in &mut lock(&shared.state).waiters {
                waiter.started = true;
            }
        }
        AgentSessionEvent::AgentSettled { aborted } => on_settled(aborted, &session, shared),
        AgentSessionEvent::CompactionStart { .. } => {
            let id = {
                let mut state = lock(&shared.state);
                state.compactions += 1;
                let id = format!("compaction-{}", state.compactions);
                state.compaction = Some(id.clone());
                id
            };
            shared.update(json!({
                "sessionUpdate": "compaction_update",
                "compactionId": id,
                "status": "in_progress",
            }));
        }
        AgentSessionEvent::CompactionEnd {
            result,
            aborted,
            error_message,
            ..
        } => {
            let id = lock(&shared.state)
                .compaction
                .take()
                .unwrap_or_else(|| "compaction".to_owned());
            if let Some(result) = &result {
                shared.update(json!({
                    "sessionUpdate": "compaction_summary_chunk",
                    "compactionId": id,
                    "content": {"type": "text", "text": result.summary},
                }));
            }
            let mut update = json!({ "sessionUpdate": "compaction_update", "compactionId": id });
            if aborted {
                update["status"] = json!("cancelled");
            } else if let Some(error) = error_message.filter(|_| result.is_none()) {
                update["status"] = json!("failed");
                update["error"] = json!(error);
            } else {
                update["status"] = json!("completed");
            }
            shared.update(update);
            if let Some(update) = usage_update(&session) {
                shared.update(update);
            }
        }
        AgentSessionEvent::SessionInfoChanged { name } => shared.update(json!({
            "sessionUpdate": "session_info_update",
            "title": name,
        })),
        AgentSessionEvent::ThinkingLevelChanged { .. } => shared.update(config_update(&session)),
        AgentSessionEvent::AutoRetryStart {
            attempt,
            max_attempts,
            delay_ms,
            error_message,
        } => shared.update(json!({
            "sessionUpdate": "notice",
            "severity": "warning",
            "title": format!(
                "Retrying in {}s (attempt {attempt}/{max_attempts})",
                delay_ms.div_ceil(1000)
            ),
            "description": error_message,
        })),
        AgentSessionEvent::AutoRetryEnd {
            success: false,
            final_error,
            attempt,
        } => shared.update(json!({
            "sessionUpdate": "notice",
            "severity": "error",
            "title": format!("Retry failed after {attempt} attempts"),
            "description": final_error,
        })),
        AgentSessionEvent::QueueUpdate {
            steering,
            follow_up,
        } => shared.link.ext_notification(
            "_oino/queue_update",
            json!({ "steering": steering, "followUp": follow_up }),
        ),
        _ => {}
    }
}

fn on_stream_event(event: AssistantMessageEvent, shared: &Shared) {
    match event {
        AssistantMessageEvent::TextDelta { delta, .. } => shared.update(json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": delta},
        })),
        AssistantMessageEvent::ThinkingDelta { delta, .. } => shared.update(json!({
            "sessionUpdate": "agent_thought_chunk",
            "content": {"type": "text", "text": delta},
        })),
        AssistantMessageEvent::ToolcallStart {
            content_index,
            partial,
        } => {
            let Some(AssistantContent::ToolCall(call)) = partial.content.get(content_index) else {
                return;
            };
            if call.id.is_empty() {
                return;
            }
            let fresh = {
                let mut state = lock(&shared.state);
                let fresh = !state.tools.contains_key(&call.id);
                state
                    .tools
                    .entry(call.id.clone())
                    .or_default()
                    .name
                    .clone_from(&call.name);
                fresh
            };
            if fresh {
                shared.update(tool_call_created(
                    &call.id,
                    &call.name,
                    &JsonObject::new(),
                    "pending",
                    &shared.cwd,
                ));
            }
        }
        AssistantMessageEvent::ToolcallEnd { tool_call, .. } => {
            let fresh = {
                let mut state = lock(&shared.state);
                let fresh = !state.tools.contains_key(&tool_call.id);
                let tool = state.tools.entry(tool_call.id.clone()).or_default();
                tool.name.clone_from(&tool_call.name);
                tool.args.clone_from(&tool_call.arguments);
                fresh
            };
            if fresh {
                shared.update(tool_call_created(
                    &tool_call.id,
                    &tool_call.name,
                    &tool_call.arguments,
                    "pending",
                    &shared.cwd,
                ));
            } else {
                shared.update(json!({
                    "sessionUpdate": "tool_call_update",
                    "toolCallId": tool_call.id,
                    "title": tool_title(&tool_call.name, &tool_call.arguments),
                    "rawInput": tool_call.arguments,
                    "locations": tool_locations(&tool_call.name, &tool_call.arguments, &shared.cwd),
                }));
            }
        }
        _ => {}
    }
}

async fn on_tool_end(id: &str, result: &AgentToolResult, is_error: bool, shared: &Shared) {
    let tool = {
        let mut state = lock(&shared.state);
        state.executing.retain(|e| e != id);
        state.tools.remove(id)
    };
    let mut content = match tool.as_ref().and_then(|t| t.terminal.clone()) {
        Some(terminal) => vec![json!({ "type": "terminal", "terminalId": terminal })],
        None => result_content(result),
    };
    if !is_error
        && let Some(snapshot) = tool.as_ref().and_then(|t| t.snapshot.as_ref())
        && let Some(diff) = snapshot.diff().await
    {
        content.push(diff);
    }
    shared.update(json!({
        "sessionUpdate": "tool_call_update",
        "toolCallId": id,
        "status": if is_error { "failed" } else { "completed" },
        "content": content,
        "rawOutput": serde_json::to_value(result).unwrap_or(JsonValue::Null),
    }));
}

fn on_settled(aborted: bool, session: &AgentSession, shared: &Shared) {
    if let Some(update) = usage_update(session) {
        shared.update(update);
    }
    let last = last_assistant_stop(&session.messages());
    let (waiters, cancelled) = {
        let mut state = lock(&shared.state);
        let (ready, waiting): (Vec<_>, Vec<_>) = state
            .waiters
            .drain(..)
            .partition(|w| !w.needs_start || w.started);
        state.waiters = waiting;
        let cancelled = state.cancel_requested;
        if state.waiters.is_empty() {
            state.cancel_requested = false;
        }
        (ready, cancelled)
    };
    if waiters.is_empty() {
        return;
    }
    if let Some((StopReason::Error, Some(message))) = &last
        && !aborted
    {
        shared.update(json!({
            "sessionUpdate": "notice",
            "severity": "error",
            "title": message,
        }));
    }
    let reason = stop_reason(aborted || cancelled, last.map(|(reason, _)| reason));
    for waiter in waiters {
        let _ = waiter.tx.send(reason);
    }
}

fn on_oino_event(event: &OinoEvent, backend: &InProcessBackend, shared: &Shared) {
    let session = backend.runtime().session();
    match event {
        OinoEvent::ModelChanged { .. } => shared.update(config_update(&session)),
        OinoEvent::CommandsChanged => shared.update(available_commands(&session)),
        OinoEvent::SessionReplaced { .. } => {
            shared.update(available_commands(&session));
            shared.update(config_update(&session));
            shared.update(json!({
                "sessionUpdate": "session_info_update",
                "title": session.session_name(),
            }));
        }
        OinoEvent::RunState { .. } => shared.link.ext_notification(
            "_oino/run_state",
            serde_json::to_value(event).unwrap_or(JsonValue::Null),
        ),
        OinoEvent::ExtensionUiDismiss { .. } => shared.link.ext_notification(
            "_oino/extension_ui",
            serde_json::to_value(event).unwrap_or(JsonValue::Null),
        ),
        _ => {}
    }
}

fn on_ui(request: UiRequest, backend: &Arc<InProcessBackend>, shared: &Arc<Shared>) {
    if !is_dialog(&request.method) {
        match fire_update(&request) {
            Some(update) => shared.update(update),
            None => shared.link.ext_notification(
                "_oino/extension_ui",
                serde_json::to_value(&request).unwrap_or(JsonValue::Null),
            ),
        }
        return;
    }
    let backend = Arc::clone(backend);
    let shared = Arc::clone(shared);
    tokio::spawn(async move {
        let answer = answer_dialog(&request, &shared).await;
        backend.ui_respond(UiResponse {
            id: request.id,
            answer,
        });
    });
}

async fn answer_dialog(request: &UiRequest, shared: &Shared) -> UiAnswer {
    let running = shared.latest_executing();
    let caps = shared.link.caps();
    match Presentation::of(request, running.is_some(), caps.elicitation_form) {
        Presentation::Permission {
            tool_call_id,
            title,
            detail,
            options,
            confirm,
        } => {
            let Some(id) = tool_call_id.or(running) else {
                return Presentation::permission_answer(confirm, None);
            };
            let tool_call = shared.permission_tool_call(&id, &title, detail.as_deref());
            match shared
                .link
                .request_permission(tool_call, JsonValue::Array(options))
                .await
            {
                Ok(selected) => Presentation::permission_answer(confirm, selected),
                Err(error) => {
                    tracing::warn!("{error}");
                    Presentation::permission_answer(confirm, None)
                }
            }
        }
        Presentation::Elicitation {
            message,
            schema,
            confirm,
        } => match shared
            .link
            .elicit_form(&message, schema, running.as_deref())
            .await
        {
            Ok(content) => Presentation::elicitation_answer(confirm, content),
            Err(error) => {
                tracing::warn!("{error}");
                Presentation::elicitation_answer(confirm, None)
            }
        },
        Presentation::Immediate(answer) => answer,
    }
}

/// `session/load` replay of the current branch.
pub(crate) fn replay(session: &AgentSession, cwd: &Path, link: &ClientLink) {
    let messages: Vec<AgentMessage> = session.messages();
    for update in crate::convert::replay(&messages, cwd) {
        if let Err(error) = link.update(update) {
            tracing::debug!("replay dropped: {error}");
        }
    }
}
