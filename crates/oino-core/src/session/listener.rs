//! Agent event handling: queues, extension events, client events and persistence
//! (pi `_handleAgentEvent`, `_emitExtensionEvent`).

use std::sync::{Arc, Weak};

use futures::future::BoxFuture;
use oino_agent::{AgentEvent, AgentListener};
use oino_ai::{AgentMessage, StopReason};
use serde_json::json;
use tokio_util::sync::CancellationToken;

use super::{AgentSession, Inner, lock};
use crate::events::AgentSessionEvent;

/// Subscribes the session to its agent.
pub(crate) struct Listener {
    inner: Weak<Inner>,
}

impl Listener {
    pub(crate) fn new(inner: &Arc<Inner>) -> Self {
        Self {
            inner: Arc::downgrade(inner),
        }
    }
}

impl AgentListener for Listener {
    fn on_event<'a>(
        &'a self,
        event: &'a AgentEvent,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, ()> {
        Box::pin(async move {
            if let Some(inner) = self.inner.upgrade() {
                AgentSession::from_inner(inner)
                    .handle_agent_event(event)
                    .await;
            }
        })
    }
}

/// Joined text of a user message (pi `contentText(content, "")`).
pub(crate) fn user_text(message: &AgentMessage) -> String {
    match message {
        AgentMessage::User(user) => user.content.text(""),
        AgentMessage::Custom(custom) => custom.content.text(""),
        _ => String::new(),
    }
}

impl AgentSession {
    pub(crate) async fn handle_agent_event(&self, event: &AgentEvent) {
        if matches!(event, AgentEvent::AgentStart) && lock(&self.inner.book).abort_requested {
            self.inner.agent.abort();
        }
        if let AgentEvent::MessageStart {
            message: message @ AgentMessage::User(_),
        } = event
        {
            self.dequeue_delivered(&user_text(message));
        }
        match event {
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => self.note_tool_start(tool_call_id, tool_name, args),
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => self.note_tool_end(tool_call_id),
            _ => {}
        }

        let nested = match event {
            AgentEvent::MessageStart {
                message: AgentMessage::ToolResult(result),
            } => self
                .attach_nested_record(result, false)
                .map(|m| AgentEvent::MessageStart {
                    message: AgentMessage::ToolResult(m),
                }),
            AgentEvent::MessageEnd {
                message: AgentMessage::ToolResult(result),
            } => self.attach_nested_record(result, true).map(|m| {
                self.inner
                    .agent
                    .replace_last_message(AgentMessage::ToolResult(m.clone()));
                AgentEvent::MessageEnd {
                    message: AgentMessage::ToolResult(m),
                }
            }),
            AgentEvent::AgentEnd { .. } => {
                self.clear_nested();
                None
            }
            _ => None,
        };
        let event = nested.as_ref().unwrap_or(event);
        let replacement = self.emit_extension_event(event).await;
        if let Some(message) = &replacement {
            self.inner.agent.replace_last_message(message.clone());
        }
        let event = match (event, replacement) {
            (AgentEvent::MessageEnd { .. }, Some(message)) => AgentEvent::MessageEnd { message },
            (event, _) => event.clone(),
        };
        let will_retry = match &event {
            AgentEvent::AgentEnd { messages } => self.will_retry_after_agent_end(messages),
            _ => false,
        };
        self.emit(AgentSessionEvent::from_agent(event.clone(), will_retry));

        if let AgentEvent::MessageEnd { message } = &event {
            self.persist_message(message);
        }
        if matches!(event, AgentEvent::TurnEnd { .. }) {
            self.flush_pending_custom_messages();
        }
    }

    fn dequeue_delivered(&self, text: &str) {
        let removed = {
            let mut book = lock(&self.inner.book);
            book.overflow_recovery_attempted = false;
            if text.is_empty() {
                false
            } else if let Some(index) = book.steering.iter().position(|t| t == text) {
                book.steering.remove(index);
                true
            } else if let Some(index) = book.follow_up.iter().position(|t| t == text) {
                book.follow_up.remove(index);
                true
            } else {
                false
            }
        };
        if removed {
            self.emit_queue_update();
        }
    }

    fn persist_message(&self, message: &AgentMessage) {
        let appended = {
            let mut manager = lock(&self.inner.manager);
            match message {
                AgentMessage::Custom(custom) => Some(manager.append_custom_message_entry(
                    &custom.custom_type,
                    custom.content.clone(),
                    custom.display,
                    custom.details.clone(),
                )),
                AgentMessage::System(_)
                | AgentMessage::User(_)
                | AgentMessage::Assistant(_)
                | AgentMessage::ToolResult(_) => Some(manager.append_message(message.clone())),
                _ => None,
            }
        };
        let entry_id = match appended {
            Some(Ok(id)) => Some(id),
            Some(Err(error)) => {
                tracing::error!("persisting message failed: {error}");
                None
            }
            None => None,
        };
        let mut retry_success = None;
        {
            let mut book = lock(&self.inner.book);
            match message {
                AgentMessage::Assistant(assistant) => {
                    book.last_assistant = Some(assistant.clone());
                    book.last_assistant_entry = entry_id;
                    book.tool_result_entries.clear();
                    if !matches!(
                        assistant.stop_reason,
                        StopReason::Error | StopReason::Length
                    ) {
                        book.overflow_recovery_attempted = false;
                    }
                    if assistant.stop_reason != StopReason::Error && book.retry_attempt > 0 {
                        retry_success = Some(book.retry_attempt);
                        book.retry_attempt = 0;
                    }
                }
                AgentMessage::ToolResult(result) => {
                    if let Some(id) = entry_id {
                        book.tool_result_entries
                            .push((result.tool_call_id.clone(), id));
                    }
                }
                _ => {}
            }
        }
        if let Some(attempt) = retry_success {
            self.emit(AgentSessionEvent::AutoRetryEnd {
                success: true,
                attempt,
                final_error: None,
            });
        }
    }

    fn will_retry_after_agent_end(&self, messages: &[AgentMessage]) -> bool {
        let (abort_requested, attempt) = {
            let book = lock(&self.inner.book);
            (book.abort_requested, book.retry_attempt)
        };
        if abort_requested {
            return false;
        }
        let settings = lock(&self.inner.settings).settings().retry_settings();
        if !settings.enabled || u64::from(attempt) >= settings.max_retries {
            return false;
        }
        messages
            .iter()
            .rev()
            .find_map(|message| match message {
                AgentMessage::Assistant(assistant) => Some(self.is_retryable_error(assistant)),
                _ => None,
            })
            .unwrap_or(false)
    }

    /// Emits the extension event for a loop event; returns a `message_end` replacement.
    async fn emit_extension_event(&self, event: &AgentEvent) -> Option<AgentMessage> {
        let runner = self.runner();
        match event {
            AgentEvent::AgentStart => {
                lock(&self.inner.book).turn_index = 0;
                if runner.has_handlers("agent_start") {
                    runner.emit(json!({"type": "agent_start"})).await;
                }
            }
            AgentEvent::AgentEnd { messages } => {
                if runner.has_handlers("agent_end") {
                    runner
                        .emit(json!({"type": "agent_end", "messages": messages}))
                        .await;
                }
            }
            AgentEvent::TurnStart => {
                if runner.has_handlers("turn_start") {
                    let turn_index = lock(&self.inner.book).turn_index;
                    runner
                        .emit(json!({"type": "turn_start", "turnIndex": turn_index, "timestamp": oino_agent::now_ms()}))
                        .await;
                }
            }
            AgentEvent::TurnEnd {
                message,
                tool_results,
            } => {
                let dispatched = std::mem::take(&mut lock(&self.inner.book).boundary_dispatched);
                if let (AgentMessage::Assistant(assistant), false) = (message, dispatched) {
                    self.dispatch_turn_end_boundary(assistant, tool_results)
                        .await;
                }
                lock(&self.inner.book).turn_index += 1;
            }
            AgentEvent::MessageStart { message } => {
                if runner.has_handlers("message_start") {
                    runner
                        .emit(json!({"type": "message_start", "message": message}))
                        .await;
                }
            }
            AgentEvent::MessageUpdate {
                message,
                assistant_message_event,
            } => {
                if runner.has_handlers("message_update") {
                    runner
                        .emit(json!({"type": "message_update", "message": message, "assistantMessageEvent": assistant_message_event}))
                        .await;
                }
            }
            AgentEvent::MessageEnd { message } => {
                if runner.has_handlers("message_end") {
                    return runner.emit_message_end(message).await;
                }
            }
            AgentEvent::ToolExecutionStart {
                tool_call_id,
                tool_name,
                args,
            } => {
                if runner.has_handlers("tool_execution_start") {
                    runner
                        .emit(json!({"type": "tool_execution_start", "toolCallId": tool_call_id, "toolName": tool_name, "args": args}))
                        .await;
                }
            }
            AgentEvent::ToolExecutionUpdate {
                tool_call_id,
                tool_name,
                args,
                partial_result,
            } => {
                if runner.has_handlers("tool_execution_update") {
                    runner
                        .emit(json!({"type": "tool_execution_update", "toolCallId": tool_call_id, "toolName": tool_name, "args": args, "partialResult": partial_result}))
                        .await;
                }
            }
            AgentEvent::ToolExecutionEnd {
                tool_call_id,
                tool_name,
                result,
                is_error,
                duration_ms,
            } => {
                if runner.has_handlers("tool_execution_end") {
                    let mut payload = json!({"type": "tool_execution_end", "toolCallId": tool_call_id, "toolName": tool_name, "result": result, "isError": is_error});
                    if let Some(duration) = duration_ms {
                        payload["durationMs"] = json!(duration);
                    }
                    runner.emit(payload).await;
                }
            }
        }
        None
    }
}
