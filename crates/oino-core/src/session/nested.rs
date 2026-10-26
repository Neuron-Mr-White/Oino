//! Nested tool calls (pi 1.4 `ctx.executeTool`, `nested-tool-calls.ts`): a tool runs another
//! callable tool through the full pipeline (validation, `tool_call`/`tool_result` hooks) with
//! id `<parent>/<n>`; calls and usage are recorded on the parent's result message.

use std::collections::HashMap;
use std::time::Instant;

use oino_agent::{
    AfterToolCall, AfterToolCallContext, AfterToolCallResult, AgentContext, BeforeToolCall,
    BeforeToolCallContext, BeforeToolCallResult, HookError, RunToolCallOptions, ToolCallOutcome,
    run_tool_call,
};
use oino_ai::{
    AssistantMessage, JsonObject, NestedStatus, NestedToolCallRecord, NestedToolCalls, ToolCall,
    Usage, UserContent,
};
use tokio_util::sync::CancellationToken;

use super::{AgentSession, lock};
use crate::events::AgentSessionEvent;

/// pi `NESTED_CALL_LIMITS`.
const MAX_CALLS: usize = 256;
const MAX_ARGUMENT_BYTES_PER_CALL: u64 = 8 * 1024;
const MAX_ARGUMENT_BYTES_TOTAL: u64 = 32 * 1024;
const MAX_ERROR_CHARS: usize = 500;

/// pi `NestedCallRecorder` for one top-level call.
pub(crate) struct Recorder {
    calls: Vec<(NestedToolCallRecord, Instant)>,
    complete: bool,
    argument_bytes: u64,
    usage: Option<Usage>,
    next_id: u64,
}

impl Default for Recorder {
    fn default() -> Self {
        Self {
            calls: Vec::new(),
            complete: true,
            argument_bytes: 0,
            usage: None,
            next_id: 1,
        }
    }
}

impl Recorder {
    fn start(&mut self, call: &ToolCall) -> Option<usize> {
        if self.calls.len() >= MAX_CALLS {
            self.complete = false;
            return None;
        }
        let json = serde_json::to_string(&call.arguments).unwrap_or_default();
        let bytes = json.len() as u64;
        let mut record = NestedToolCallRecord {
            id: call.id.clone(),
            name: call.name.clone(),
            arguments: None,
            arguments_bytes: None,
            status: NestedStatus::Unfinished,
            duration_ms: None,
            error: None,
        };
        if bytes > MAX_ARGUMENT_BYTES_PER_CALL
            || self.argument_bytes + bytes > MAX_ARGUMENT_BYTES_TOTAL
        {
            record.arguments_bytes = Some(bytes);
            self.complete = false;
        } else {
            record.arguments = Some(call.arguments.clone());
            self.argument_bytes += bytes;
        }
        self.calls.push((record, Instant::now()));
        Some(self.calls.len() - 1)
    }

    fn finish(&mut self, index: Option<usize>, outcome: &ToolCallOutcome) {
        let Some((record, started)) = index.and_then(|i| self.calls.get_mut(i)) else {
            return;
        };
        record.status = if outcome.is_error {
            NestedStatus::Error
        } else {
            NestedStatus::Ok
        };
        record.duration_ms = Some(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX));
        let text: Vec<&str> = outcome
            .result
            .content
            .iter()
            .filter_map(|block| match block {
                UserContent::Text(text) => Some(text.text.as_str()),
                UserContent::Image(_) => None,
            })
            .collect();
        let text = text.join("\n");
        if outcome.is_error && !text.is_empty() {
            record.error = Some(text.chars().take(MAX_ERROR_CHARS).collect());
        }
        if let Some(usage) = &outcome.result.usage {
            match &mut self.usage {
                Some(total) => total.accumulate(usage),
                None => self.usage = Some(*usage),
            }
        }
    }

    fn snapshot(&self) -> Option<NestedToolCalls> {
        if self.calls.is_empty() && self.complete {
            return None;
        }
        let calls: Vec<NestedToolCallRecord> = self.calls.iter().map(|(r, _)| r.clone()).collect();
        let complete = self.complete && calls.iter().all(|c| c.status != NestedStatus::Unfinished);
        Some(NestedToolCalls { calls, complete })
    }
}

/// Recorders by top-level tool-call id, plus nested-id → top-level id.
#[derive(Default)]
pub(crate) struct NestedState {
    recorders: HashMap<String, Recorder>,
    roots: HashMap<String, String>,
    /// Records taken at the result's `message_start`, applied again at `message_end`.
    taken: HashMap<String, (Option<NestedToolCalls>, Option<Usage>)>,
}

struct NestedHooks {
    session: AgentSession,
    parent: String,
}

impl BeforeToolCall for NestedHooks {
    fn before<'a>(
        &'a self,
        cx: BeforeToolCallContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, Result<Option<BeforeToolCallResult>, HookError>> {
        Box::pin(async move {
            self.session
                .before_tool_call(cx.tool_call, cx.args, Some(&self.parent))
                .await
        })
    }
}

impl AfterToolCall for NestedHooks {
    fn after<'a>(
        &'a self,
        cx: AfterToolCallContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> futures::future::BoxFuture<'a, Result<Option<AfterToolCallResult>, HookError>> {
        Box::pin(async move { Ok(self.session.after_tool_call(&cx, Some(&self.parent)).await) })
    }
}

impl AgentSession {
    /// pi `ctx.executeTool(name, args)` for the running call `parent`.
    pub async fn execute_nested_tool(
        &self,
        parent: &str,
        name: &str,
        args: JsonObject,
        cancel: CancellationToken,
    ) -> ToolCallOutcome {
        let (call, index) = {
            let mut state = lock(&self.inner.nested);
            let root = state
                .roots
                .get(parent)
                .cloned()
                .unwrap_or_else(|| parent.to_owned());
            let recorder = state.recorders.entry(root.clone()).or_default();
            let call = ToolCall {
                id: format!("{parent}/{}", recorder.next_id),
                name: name.to_owned(),
                arguments: args,
                ..ToolCall::default()
            };
            recorder.next_id += 1;
            let index = recorder.start(&call);
            state.roots.insert(call.id.clone(), root);
            (call, index)
        };
        self.emit(AgentSessionEvent::ToolExecutionStart {
            tool_call_id: call.id.clone(),
            tool_name: name.to_owned(),
            args: call.arguments.clone(),
            parent_tool_call_id: Some(parent.to_owned()),
        });
        let outcome = match self.last_assistant_message() {
            None => ToolCallOutcome {
                tool_call: call.clone(),
                result: oino_agent::AgentToolResult::error("No assistant message issued this call"),
                is_error: true,
                duration_ms: None,
            },
            Some(assistant) => {
                self.run_nested(call.clone(), parent, &assistant, cancel)
                    .await
            }
        };
        {
            let mut state = lock(&self.inner.nested);
            let root = state.roots.remove(&call.id);
            if let Some(recorder) = root.and_then(|r| state.recorders.get_mut(&r)) {
                recorder.finish(index, &outcome);
            }
        }
        self.emit(AgentSessionEvent::ToolExecutionEnd {
            tool_call_id: call.id.clone(),
            tool_name: name.to_owned(),
            result: outcome.result.clone(),
            is_error: outcome.is_error,
            duration_ms: outcome.duration_ms,
            parent_tool_call_id: Some(parent.to_owned()),
        });
        outcome
    }

    async fn run_nested(
        &self,
        call: ToolCall,
        parent: &str,
        assistant: &AssistantMessage,
        cancel: CancellationToken,
    ) -> ToolCallOutcome {
        let active = self.active_tool_names().into_iter().collect();
        let tools = self.callable_tools(&active);
        let hooks = std::sync::Arc::new(NestedHooks {
            session: self.clone(),
            parent: parent.to_owned(),
        });
        let context = AgentContext {
            messages: self.messages(),
            tools: self.inner.agent.with_state(|s| s.tools.clone()),
        };
        run_tool_call(
            call,
            RunToolCallOptions {
                assistant_message: assistant,
                context: &context,
                tools: &tools,
                before_tool_call: Some(std::sync::Arc::clone(&hooks) as _),
                after_tool_call: Some(hooks as _),
                cancel,
                parent_tool_call_id: Some(parent.to_owned()),
                on_update: None,
            },
        )
        .await
    }

    /// Adds the nested calls and usage of a finished top-level call to its result message
    /// (pi `takeRecord` at the result's `message_start`; the same record again at `message_end`).
    pub(crate) fn attach_nested_record(
        &self,
        message: &oino_ai::ToolResultMessage,
        end: bool,
    ) -> Option<oino_ai::ToolResultMessage> {
        let mut state = lock(&self.inner.nested);
        let record = if end {
            state.taken.remove(&message.tool_call_id)?
        } else {
            let recorder = state.recorders.remove(&message.tool_call_id)?;
            let record = (recorder.snapshot(), recorder.usage);
            state
                .taken
                .insert(message.tool_call_id.clone(), record.clone());
            record
        };
        let mut message = message.clone();
        if let Some(calls) = record.0 {
            message.nested_calls = Some(calls);
        }
        if let Some(usage) = record.1 {
            match &mut message.usage {
                Some(total) => total.accumulate(&usage),
                None => message.usage = Some(usage),
            }
        }
        Some(message)
    }

    /// Clears nested-call state at `agent_end`.
    pub(crate) fn clear_nested(&self) {
        let mut state = lock(&self.inner.nested);
        state.recorders.clear();
        state.roots.clear();
        state.taken.clear();
    }
}
