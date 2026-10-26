//! The agent loop (pi-agent-core `agent-loop.js`).
//!
//! A run emits `agent_start`, then turns: each turn adds queued messages, requests one
//! assistant response, executes its tool calls and ends with `turn_end`. The run continues
//! while tool calls or steering messages remain, then polls follow-ups, then honours an
//! explicit `finish_turn → Continue` with one context-only turn. There is no turn cap.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Instant;

use futures::StreamExt;
use futures::future::{BoxFuture, join_all};
use oino_ai::transcript::{SystemState, tool_state_changes};
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, AssistantMessageEvent, Context, JsonObject,
    StopReason, SystemMessage, ThinkingLevel, ToolCall, ToolDecl, ToolRef, ToolResultMessage,
    UserTextField,
};
use tokio_util::sync::CancellationToken;

use crate::config::{
    AfterToolCall, AfterToolCallContext, AgentContext, AgentLoopConfig, AgentTurnContext,
    BeforeToolCall, BeforeToolCallContext, HookError, PrepareRequestContext, StreamRequest,
    ToolExecution, TurnDecision,
};
use crate::event::{AgentEvent, EventSink};
use crate::tool::{AgentTool, AgentToolResult, ExecutionMode, ToolInvocation, UpdateSink};
use crate::validate::validate_tool_arguments;

/// The loop cannot start or a hook failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum AgentLoopError {
    /// `agent_loop_continue` without messages.
    #[error("Cannot continue: no messages in context")]
    NoMessages,
    /// `agent_loop_continue` after an assistant message.
    #[error("Cannot continue from message role: assistant")]
    ContinueFromAssistant,
    /// A loop hook failed; the run ends like a thrown pi error.
    #[error("{0}")]
    Hook(#[from] HookError),
}

/// Current wall-clock time in epoch milliseconds (pi `Date.now()`).
#[must_use]
pub fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

/// pi `runAgentLoop`: starts a run with `prompts`. Returns the messages the run added.
///
/// # Errors
///
/// [`AgentLoopError::Hook`] when a loop hook fails (events emitted so far stand).
pub async fn agent_loop(
    prompts: Vec<AgentMessage>,
    context: AgentContext,
    config: &AgentLoopConfig,
    sink: &dyn EventSink,
    cancel: CancellationToken,
) -> Result<Vec<AgentMessage>, AgentLoopError> {
    let initial = declare_tool_changes(&context, prompts);
    let mut new_messages = initial.clone();
    let mut current = context;
    current.messages.extend(initial.iter().cloned());
    sink.emit(AgentEvent::AgentStart).await;
    sink.emit(AgentEvent::TurnStart).await;
    for message in initial {
        sink.emit(AgentEvent::MessageStart {
            message: message.clone(),
        })
        .await;
        sink.emit(AgentEvent::MessageEnd { message }).await;
    }
    run_loop(current, &mut new_messages, config, sink, &cancel).await?;
    Ok(new_messages)
}

/// pi `runAgentLoopContinue`: continues from the current transcript.
///
/// # Errors
///
/// [`AgentLoopError::NoMessages`] / [`AgentLoopError::ContinueFromAssistant`] before anything
/// is emitted; [`AgentLoopError::Hook`] when a hook fails.
pub async fn agent_loop_continue(
    context: AgentContext,
    config: &AgentLoopConfig,
    sink: &dyn EventSink,
    cancel: CancellationToken,
) -> Result<Vec<AgentMessage>, AgentLoopError> {
    match context.messages.last() {
        None => return Err(AgentLoopError::NoMessages),
        Some(AgentMessage::Assistant(_)) => return Err(AgentLoopError::ContinueFromAssistant),
        Some(_) => {}
    }
    let mut new_messages = Vec::new();
    sink.emit(AgentEvent::AgentStart).await;
    sink.emit(AgentEvent::TurnStart).await;
    run_loop(context, &mut new_messages, config, sink, &cancel).await?;
    Ok(new_messages)
}

struct CompletedTurn {
    message: AssistantMessage,
    tool_results: Vec<ToolResultMessage>,
}

#[allow(
    clippy::too_many_lines,
    reason = "one-to-one port of pi runLoop; splitting obscures the control flow"
)]
async fn run_loop(
    mut current: AgentContext,
    new_messages: &mut Vec<AgentMessage>,
    initial_config: &AgentLoopConfig,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> Result<(), AgentLoopError> {
    let mut model = initial_config.model.clone();
    let mut thinking = initial_config.thinking_level;
    let config = initial_config;
    let mut last_turn: Option<CompletedTurn> = None;
    let mut explicit_continuation = false;
    let mut pending = poll(config.get_steering_messages.as_ref()).await;

    loop {
        let mut has_more_tool_calls = true;
        while has_more_tool_calls || !pending.is_empty() {
            let mut prepared = Vec::new();
            if let Some(turn) = &last_turn {
                if let Some(hook) = &config.prepare_next_turn {
                    let cx = AgentTurnContext {
                        message: &turn.message,
                        tool_results: &turn.tool_results,
                        context: &current,
                        new_messages,
                    };
                    if let Some(update) = hook.prepare(cx, cancel).await? {
                        if let Some(context) = update.context {
                            current = context;
                        }
                        prepared = update.messages.unwrap_or_default();
                        model = update.model.unwrap_or(model);
                        thinking = update.thinking_level.unwrap_or(thinking);
                    }
                }
                if pending.is_empty() {
                    pending = poll(config.get_steering_messages.as_ref()).await;
                }
                sink.emit(AgentEvent::TurnStart).await;
            }

            prepared.append(&mut pending);
            for message in declare_tool_changes(&current, prepared) {
                sink.emit(AgentEvent::MessageStart {
                    message: message.clone(),
                })
                .await;
                sink.emit(AgentEvent::MessageEnd {
                    message: message.clone(),
                })
                .await;
                current.messages.push(message.clone());
                new_messages.push(message);
            }

            if let Some(hook) = &config.prepare_request {
                let cx = PrepareRequestContext {
                    context: &current,
                    model: &model,
                    thinking_level: thinking,
                };
                if let Some(update) = hook.prepare(cx, cancel).await? {
                    if let Some(context) = update.context {
                        current = context;
                    }
                    model = update.model.unwrap_or(model);
                    thinking = update.thinking_level.unwrap_or(thinking);
                }
            }

            let message =
                stream_assistant_response(&mut current, config, &model, thinking, sink, cancel)
                    .await?;
            new_messages.push(AgentMessage::Assistant(message.clone()));

            if matches!(message.stop_reason, StopReason::Error | StopReason::Aborted) {
                if let Some(hook) = &config.finish_turn {
                    let cx = AgentTurnContext {
                        message: &message,
                        tool_results: &[],
                        context: &current,
                        new_messages,
                    };
                    hook.finish(cx, cancel).await?;
                }
                sink.emit(AgentEvent::TurnEnd {
                    message: AgentMessage::Assistant(message),
                    tool_results: Vec::new(),
                })
                .await;
                sink.emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                })
                .await;
                return Ok(());
            }

            let tool_calls = tool_calls_of(&message);
            let mut tool_results = Vec::new();
            has_more_tool_calls = false;
            if !tool_calls.is_empty() {
                let batch = if message.stop_reason == StopReason::Length {
                    fail_truncated_tool_calls(&tool_calls, sink).await
                } else {
                    execute_tool_calls(&current, &message, tool_calls, config, sink, cancel).await
                };
                has_more_tool_calls = !batch.terminate;
                tool_results = batch.messages;
                for result in &tool_results {
                    current
                        .messages
                        .push(AgentMessage::ToolResult(result.clone()));
                    new_messages.push(AgentMessage::ToolResult(result.clone()));
                }
            }

            let decision = match &config.finish_turn {
                Some(hook) => {
                    let cx = AgentTurnContext {
                        message: &message,
                        tool_results: &tool_results,
                        context: &current,
                        new_messages,
                    };
                    hook.finish(cx, cancel).await?
                }
                None => None,
            };
            sink.emit(AgentEvent::TurnEnd {
                message: AgentMessage::Assistant(message.clone()),
                tool_results: tool_results.clone(),
            })
            .await;
            last_turn = Some(CompletedTurn {
                message,
                tool_results,
            });
            if decision == Some(TurnDecision::End) {
                sink.emit(AgentEvent::AgentEnd {
                    messages: new_messages.clone(),
                })
                .await;
                return Ok(());
            }
            explicit_continuation = decision == Some(TurnDecision::Continue);
            pending = poll(config.get_steering_messages.as_ref()).await;
            if has_more_tool_calls || !pending.is_empty() {
                explicit_continuation = false;
            }
        }

        let follow_ups = poll(config.get_follow_up_messages.as_ref()).await;
        if !follow_ups.is_empty() {
            explicit_continuation = false;
            pending = follow_ups;
            continue;
        }
        if explicit_continuation {
            explicit_continuation = false;
            continue;
        }
        break;
    }
    sink.emit(AgentEvent::AgentEnd {
        messages: new_messages.clone(),
    })
    .await;
    Ok(())
}

async fn poll(source: Option<&Arc<dyn crate::config::QueueSource>>) -> Vec<AgentMessage> {
    match source {
        Some(source) => source.drain().await,
        None => Vec::new(),
    }
}

fn tool_calls_of(message: &AssistantMessage) -> Vec<ToolCall> {
    message
        .content
        .iter()
        .filter_map(|block| match block {
            AssistantContent::ToolCall(call) => Some(call.clone()),
            _ => None,
        })
        .collect()
}

/// pi `declareToolChanges`: announces loadout changes with a system message before the next
/// request, or rewrites the tool fields of a pending system message.
#[must_use]
pub fn declare_tool_changes(
    context: &AgentContext,
    mut pending: Vec<AgentMessage>,
) -> Vec<AgentMessage> {
    let system_index = pending
        .iter()
        .rposition(|message| matches!(message, AgentMessage::System(_)));
    // The pending system message counts with its tool fields cleared, i.e. not at all.
    let previous = SystemState::fold(
        context
            .messages
            .iter()
            .chain(
                pending
                    .iter()
                    .enumerate()
                    .filter(|(index, _)| Some(*index) != system_index)
                    .map(|(_, message)| message),
            )
            .filter_map(|message| match message {
                AgentMessage::System(system) => Some(system),
                _ => None,
            }),
    );
    let current: Vec<ToolDecl> = context
        .tools
        .iter()
        .map(|tool| tool.meta().declaration())
        .collect();
    let (added, removed) = tool_state_changes(&previous.tools, &current);
    let unchanged = added.is_empty() && removed.is_empty();
    if let Some(index) = system_index {
        if let AgentMessage::System(system) = &pending[index] {
            let declares_nothing = system.tools_added.as_ref().is_none_or(Vec::is_empty)
                && system.tools_removed.as_ref().is_none_or(Vec::is_empty);
            if unchanged && declares_nothing {
                return pending;
            }
            pending[index] = AgentMessage::System(with_tool_changes(system, added, removed));
        }
        return pending;
    }
    if unchanged {
        return pending;
    }
    let update = SystemMessage {
        content: UserTextField::Text(String::new()),
        sections: None,
        tools_added: None,
        tools_removed: None,
        timestamp: now_ms(),
    };
    let update = AgentMessage::System(with_tool_changes(&update, added, removed));
    let insert_at = pending
        .iter()
        .position(|message| !matches!(message, AgentMessage::System(_)))
        .unwrap_or(pending.len());
    pending.insert(insert_at, update);
    pending
}

fn with_tool_changes(
    message: &SystemMessage,
    added: Vec<ToolDecl>,
    removed: Vec<ToolRef>,
) -> SystemMessage {
    SystemMessage {
        tools_added: (!added.is_empty()).then_some(added),
        tools_removed: (!removed.is_empty()).then_some(removed),
        ..message.clone()
    }
}

async fn stream_assistant_response(
    current: &mut AgentContext,
    config: &AgentLoopConfig,
    model: &oino_ai::Model,
    thinking: ThinkingLevel,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> Result<AssistantMessage, AgentLoopError> {
    let messages = match &config.transform_context {
        Some(hook) => hook.transform(current.messages.clone(), cancel).await?,
        None => current.messages.clone(),
    };
    let llm_messages = (config.convert_to_llm)(&messages);
    let api_key = match &config.get_api_key {
        Some(hook) => hook.api_key(&model.provider).await?,
        None => None,
    }
    .filter(|key| !key.is_empty())
    .or_else(|| config.api_key.clone());
    let reasoning = (thinking != ThinkingLevel::Off).then_some(thinking);
    let mut options = config.stream_options.clone();
    options.reasoning = reasoning;
    let mut events = config.stream.stream(StreamRequest {
        model: model.clone(),
        context: Context {
            messages: llm_messages,
        },
        options,
        api_key,
        cancel: cancel.clone(),
    });

    let mut started = false;
    let mut last_partial: Option<AssistantMessage> = None;
    while let Some(event) = events.next().await {
        match &event {
            AssistantMessageEvent::Start { partial } => {
                started = true;
                last_partial = Some(partial.clone());
                sink.emit(AgentEvent::MessageStart {
                    message: AgentMessage::Assistant(partial.clone()),
                })
                .await;
            }
            AssistantMessageEvent::Done { message, .. }
            | AssistantMessageEvent::Error { error: message, .. } => {
                let mut final_message = message.clone();
                final_message.thinking_level = Some(thinking);
                return Ok(finish_message(current, final_message, started, sink).await);
            }
            other => {
                if started {
                    let partial = partial_of(other).cloned();
                    if let Some(partial) = partial {
                        last_partial = Some(partial.clone());
                        sink.emit(AgentEvent::MessageUpdate {
                            message: AgentMessage::Assistant(partial),
                            assistant_message_event: event,
                        })
                        .await;
                    }
                }
            }
        }
    }
    // The stream ended without a terminal event: finish with an error message.
    let mut final_message = last_partial.unwrap_or_else(|| {
        AssistantMessage::empty(&model.api, &model.provider, &model.id, now_ms())
    });
    final_message.stop_reason = if cancel.is_cancelled() {
        StopReason::Aborted
    } else {
        StopReason::Error
    };
    final_message
        .error_message
        .get_or_insert_with(|| "Stream ended without a final event".to_owned());
    final_message.thinking_level = Some(thinking);
    Ok(finish_message(current, final_message, started, sink).await)
}

fn partial_of(event: &AssistantMessageEvent) -> Option<&AssistantMessage> {
    match event {
        AssistantMessageEvent::Start { partial }
        | AssistantMessageEvent::TextStart { partial, .. }
        | AssistantMessageEvent::TextDelta { partial, .. }
        | AssistantMessageEvent::TextEnd { partial, .. }
        | AssistantMessageEvent::ThinkingStart { partial, .. }
        | AssistantMessageEvent::ThinkingDelta { partial, .. }
        | AssistantMessageEvent::ThinkingEnd { partial, .. }
        | AssistantMessageEvent::ToolcallStart { partial, .. }
        | AssistantMessageEvent::ToolcallDelta { partial, .. }
        | AssistantMessageEvent::ToolcallEnd { partial, .. } => Some(partial),
        AssistantMessageEvent::Done { .. } | AssistantMessageEvent::Error { .. } => None,
    }
}

async fn finish_message(
    current: &mut AgentContext,
    message: AssistantMessage,
    started: bool,
    sink: &dyn EventSink,
) -> AssistantMessage {
    current
        .messages
        .push(AgentMessage::Assistant(message.clone()));
    if !started {
        sink.emit(AgentEvent::MessageStart {
            message: AgentMessage::Assistant(message.clone()),
        })
        .await;
    }
    sink.emit(AgentEvent::MessageEnd {
        message: AgentMessage::Assistant(message.clone()),
    })
    .await;
    message
}

/// A settled tool call (pi `AgentToolCallOutcome`).
#[derive(Clone, Debug, PartialEq)]
pub struct ToolCallOutcome {
    /// The call as issued.
    pub tool_call: ToolCall,
    /// Final result.
    pub result: AgentToolResult,
    /// Error flag.
    pub is_error: bool,
    /// Execution time (absent when the tool never ran).
    pub duration_ms: Option<u64>,
}

struct ToolBatch {
    messages: Vec<ToolResultMessage>,
    terminate: bool,
}

async fn fail_truncated_tool_calls(tool_calls: &[ToolCall], sink: &dyn EventSink) -> ToolBatch {
    let mut messages = Vec::new();
    for call in tool_calls {
        emit_start(call, sink).await;
        let outcome = ToolCallOutcome {
            tool_call: call.clone(),
            result: AgentToolResult::error(format!(
                "Tool call \"{}\" was not executed: the response hit the output token limit, so its arguments may be truncated. Re-issue the tool call with complete arguments.",
                call.name
            )),
            is_error: true,
            duration_ms: None,
        };
        emit_end(&outcome, sink).await;
        let message = tool_result_message(&outcome);
        emit_result_message(&message, sink).await;
        messages.push(message);
    }
    ToolBatch {
        messages,
        terminate: false,
    }
}

async fn execute_tool_calls(
    current: &AgentContext,
    assistant: &AssistantMessage,
    tool_calls: Vec<ToolCall>,
    config: &AgentLoopConfig,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> ToolBatch {
    let sequential = config.tool_execution == ToolExecution::Sequential
        || tool_calls.iter().any(|call| {
            current
                .tool(&call.name)
                .is_some_and(|tool| tool.meta().execution_mode == ExecutionMode::Sequential)
        });
    if sequential {
        execute_sequential(current, assistant, tool_calls, config, sink, cancel).await
    } else {
        execute_parallel(current, assistant, tool_calls, config, sink, cancel).await
    }
}

async fn execute_sequential(
    current: &AgentContext,
    assistant: &AssistantMessage,
    tool_calls: Vec<ToolCall>,
    config: &AgentLoopConfig,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> ToolBatch {
    let mut outcomes = Vec::new();
    let mut messages = Vec::new();
    for call in tool_calls {
        emit_start(&call, sink).await;
        let hooks = Hooks::from_config(config);
        let outcome = match prepare_tool_call(
            current,
            assistant,
            &call,
            &current.tools,
            &hooks,
            cancel,
        )
        .await
        {
            Preparation::Immediate { result } => ToolCallOutcome {
                tool_call: call.clone(),
                result,
                is_error: true,
                duration_ms: None,
            },
            Preparation::Prepared { tool, args } => {
                let executed =
                    execute_prepared(&tool, &call, args.clone(), None, cancel, Some(sink), None)
                        .await;
                finalize(current, assistant, &call, &args, executed, &hooks, cancel).await
            }
        };
        emit_end(&outcome, sink).await;
        let message = tool_result_message(&outcome);
        emit_result_message(&message, sink).await;
        outcomes.push(outcome);
        messages.push(message);
        if cancel.is_cancelled() {
            break;
        }
    }
    ToolBatch {
        messages,
        terminate: should_terminate(&outcomes),
    }
}

async fn execute_parallel(
    current: &AgentContext,
    assistant: &AssistantMessage,
    tool_calls: Vec<ToolCall>,
    config: &AgentLoopConfig,
    sink: &dyn EventSink,
    cancel: &CancellationToken,
) -> ToolBatch {
    let hooks = Hooks::from_config(config);
    let mut slots: Vec<BoxFuture<'_, ToolCallOutcome>> = Vec::new();
    for call in tool_calls {
        emit_start(&call, sink).await;
        match prepare_tool_call(current, assistant, &call, &current.tools, &hooks, cancel).await {
            Preparation::Immediate { result } => {
                let outcome = ToolCallOutcome {
                    tool_call: call,
                    result,
                    is_error: true,
                    duration_ms: None,
                };
                emit_end(&outcome, sink).await;
                slots.push(Box::pin(std::future::ready(outcome)));
            }
            Preparation::Prepared { tool, args } => {
                let hooks = hooks.clone();
                slots.push(Box::pin(async move {
                    if cancel.is_cancelled() {
                        let outcome = ToolCallOutcome {
                            tool_call: call,
                            result: AgentToolResult::error("Operation aborted"),
                            is_error: true,
                            duration_ms: None,
                        };
                        emit_end(&outcome, sink).await;
                        return outcome;
                    }
                    let executed = execute_prepared(
                        &tool,
                        &call,
                        args.clone(),
                        None,
                        cancel,
                        Some(sink),
                        None,
                    )
                    .await;
                    let outcome =
                        finalize(current, assistant, &call, &args, executed, &hooks, cancel).await;
                    emit_end(&outcome, sink).await;
                    outcome
                }));
            }
        }
        if cancel.is_cancelled() {
            break;
        }
    }
    let outcomes = join_all(slots).await;
    let mut messages = Vec::new();
    for outcome in &outcomes {
        let message = tool_result_message(outcome);
        emit_result_message(&message, sink).await;
        messages.push(message);
    }
    ToolBatch {
        messages,
        terminate: should_terminate(&outcomes),
    }
}

fn should_terminate(outcomes: &[ToolCallOutcome]) -> bool {
    !outcomes.is_empty() && outcomes.iter().all(|outcome| outcome.result.terminate)
}

#[derive(Clone)]
struct Hooks {
    before: Option<Arc<dyn BeforeToolCall>>,
    after: Option<Arc<dyn AfterToolCall>>,
}

impl Hooks {
    fn from_config(config: &AgentLoopConfig) -> Self {
        Self {
            before: config.before_tool_call.clone(),
            after: config.after_tool_call.clone(),
        }
    }
}

enum Preparation {
    Immediate {
        result: AgentToolResult,
    },
    Prepared {
        tool: Arc<dyn AgentTool>,
        args: JsonObject,
    },
}

async fn prepare_tool_call(
    current: &AgentContext,
    assistant: &AssistantMessage,
    call: &ToolCall,
    tools: &[Arc<dyn AgentTool>],
    hooks: &Hooks,
    cancel: &CancellationToken,
) -> Preparation {
    let Some(tool) = tools.iter().find(|tool| tool.meta().name == call.name) else {
        return Preparation::Immediate {
            result: AgentToolResult::error(format!("Tool {} not found", call.name)),
        };
    };
    let mut prepared_call = call.clone();
    if call.arguments_error.is_none() {
        prepared_call.arguments = tool.prepare_arguments(call.arguments.clone());
    }
    let args = match validate_tool_arguments(&tool.meta().parameters, &prepared_call) {
        Ok(args) => args,
        Err(message) => {
            return Preparation::Immediate {
                result: AgentToolResult::error(message),
            };
        }
    };
    if let Some(before) = &hooks.before {
        let cx = BeforeToolCallContext {
            assistant_message: assistant,
            tool_call: call,
            args: &args,
            context: current,
        };
        let decision = match before.before(cx, cancel).await {
            Ok(decision) => decision,
            Err(error) => {
                return Preparation::Immediate {
                    result: AgentToolResult::error(error.0),
                };
            }
        };
        if cancel.is_cancelled() {
            return Preparation::Immediate {
                result: AgentToolResult::error("Operation aborted"),
            };
        }
        if let Some(decision) = decision.filter(|d| d.block) {
            let mut result = AgentToolResult::error(
                decision
                    .reason
                    .filter(|reason| !reason.is_empty())
                    .unwrap_or_else(|| "Tool execution was blocked".to_owned()),
            );
            result.terminate = decision.terminate;
            return Preparation::Immediate { result };
        }
    }
    if cancel.is_cancelled() {
        return Preparation::Immediate {
            result: AgentToolResult::error("Operation aborted"),
        };
    }
    Preparation::Prepared {
        tool: Arc::clone(tool),
        args,
    }
}

struct Executed {
    result: AgentToolResult,
    is_error: bool,
    duration_ms: u64,
}

/// Where partial results of a running tool go.
type UpdateCallback<'a> = &'a (dyn Fn(AgentToolResult) -> BoxFuture<'a, ()> + Send + Sync);

async fn execute_prepared(
    tool: &Arc<dyn AgentTool>,
    call: &ToolCall,
    args: JsonObject,
    parent_tool_call_id: Option<String>,
    cancel: &CancellationToken,
    sink: Option<&dyn EventSink>,
    on_update: Option<UpdateCallback<'_>>,
) -> Executed {
    let (updates, mut receiver, open) = UpdateSink::channel();
    let invocation = ToolInvocation {
        tool_call_id: call.id.clone(),
        args,
        parent_tool_call_id,
    };
    let started = Instant::now();
    let mut running = tool.execute(invocation, updates, cancel.clone());
    let forward = |partial: AgentToolResult| async move {
        if let Some(sink) = sink {
            sink.emit(AgentEvent::ToolExecutionUpdate {
                tool_call_id: call.id.clone(),
                tool_name: call.name.clone(),
                args: call.arguments.clone(),
                partial_result: partial.clone(),
            })
            .await;
        }
        if let Some(on_update) = on_update {
            on_update(partial).await;
        }
    };
    let outcome = loop {
        tokio::select! {
            biased;
            result = &mut running => break result,
            Some(partial) = receiver.recv() => forward(partial).await,
        }
    };
    let duration_ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    open.store(false, Ordering::Release);
    while let Ok(partial) = receiver.try_recv() {
        forward(partial).await;
    }
    match outcome {
        Ok(result) => Executed {
            is_error: result.is_error,
            result,
            duration_ms,
        },
        Err(error) => Executed {
            result: AgentToolResult::error(error.0),
            is_error: true,
            duration_ms,
        },
    }
}

async fn finalize(
    current: &AgentContext,
    assistant: &AssistantMessage,
    call: &ToolCall,
    args: &JsonObject,
    executed: Executed,
    hooks: &Hooks,
    cancel: &CancellationToken,
) -> ToolCallOutcome {
    let mut result = executed.result;
    let mut is_error = executed.is_error;
    if let Some(after) = &hooks.after {
        let cx = AfterToolCallContext {
            assistant_message: assistant,
            tool_call: call,
            args,
            result: &result,
            is_error,
            context: current,
        };
        match after.after(cx, cancel).await {
            Ok(Some(update)) => {
                let structured = match (&update.structured_content, &update.content) {
                    (Some(structured), _) => Some(structured.clone()),
                    (None, Some(_)) => None,
                    (None, None) => result.structured_content.clone(),
                };
                if let Some(content) = update.content {
                    result.content = content;
                }
                if let Some(details) = update.details {
                    result.details = Some(details);
                }
                if let Some(usage) = update.usage {
                    result.usage = Some(usage);
                }
                if let Some(terminate) = update.terminate {
                    result.terminate = terminate;
                }
                result.structured_content = structured;
                is_error = update.is_error.unwrap_or(is_error);
            }
            Ok(None) => {}
            Err(error) => {
                result = AgentToolResult::error(error.0);
                is_error = true;
            }
        }
    }
    ToolCallOutcome {
        tool_call: call.clone(),
        result,
        is_error,
        duration_ms: Some(executed.duration_ms),
    }
}

async fn emit_start(call: &ToolCall, sink: &dyn EventSink) {
    sink.emit(AgentEvent::ToolExecutionStart {
        tool_call_id: call.id.clone(),
        tool_name: call.name.clone(),
        args: call.arguments.clone(),
    })
    .await;
}

async fn emit_end(outcome: &ToolCallOutcome, sink: &dyn EventSink) {
    sink.emit(AgentEvent::ToolExecutionEnd {
        tool_call_id: outcome.tool_call.id.clone(),
        tool_name: outcome.tool_call.name.clone(),
        result: outcome.result.clone(),
        is_error: outcome.is_error,
        duration_ms: outcome.duration_ms,
    })
    .await;
}

/// pi `createToolResultMessage`.
#[must_use]
pub fn tool_result_message(outcome: &ToolCallOutcome) -> ToolResultMessage {
    ToolResultMessage {
        tool_call_id: outcome.tool_call.id.clone(),
        tool_name: outcome.tool_call.name.clone(),
        content: outcome.result.content.clone(),
        details: outcome.result.details.clone(),
        usage: outcome.result.usage,
        nested_calls: None,
        is_error: outcome.is_error,
        timestamp: now_ms(),
        duration_ms: outcome.duration_ms,
    }
}

async fn emit_result_message(message: &ToolResultMessage, sink: &dyn EventSink) {
    sink.emit(AgentEvent::MessageStart {
        message: AgentMessage::ToolResult(message.clone()),
    })
    .await;
    sink.emit(AgentEvent::MessageEnd {
        message: AgentMessage::ToolResult(message.clone()),
    })
    .await;
}

/// Options of [`run_tool_call`] (pi `RunToolCallOptions`).
pub struct RunToolCallOptions<'a> {
    /// Assistant message the call belongs to.
    pub assistant_message: &'a AssistantMessage,
    /// Context the hooks see.
    pub context: &'a AgentContext,
    /// Tools to resolve the call against.
    pub tools: &'a [Arc<dyn AgentTool>],
    /// Tool gate.
    pub before_tool_call: Option<Arc<dyn BeforeToolCall>>,
    /// Result post-processing.
    pub after_tool_call: Option<Arc<dyn AfterToolCall>>,
    /// Abort signal.
    pub cancel: CancellationToken,
    /// Calling tool for nested calls (pi 1.4 `ctx.executeTool`).
    pub parent_tool_call_id: Option<String>,
    /// Partial-result callback.
    pub on_update: Option<UpdateCallback<'a>>,
}

/// pi `runToolCall`: one call through preparation, validation, hooks and execution. Emits no
/// events and never fails (errors become `is_error` outcomes).
pub async fn run_tool_call(call: ToolCall, options: RunToolCallOptions<'_>) -> ToolCallOutcome {
    let hooks = Hooks {
        before: options.before_tool_call.clone(),
        after: options.after_tool_call.clone(),
    };
    match prepare_tool_call(
        options.context,
        options.assistant_message,
        &call,
        options.tools,
        &hooks,
        &options.cancel,
    )
    .await
    {
        Preparation::Immediate { result } => ToolCallOutcome {
            tool_call: call,
            result,
            is_error: true,
            duration_ms: None,
        },
        Preparation::Prepared { tool, args } => {
            let executed = execute_prepared(
                &tool,
                &call,
                args.clone(),
                options.parent_tool_call_id.clone(),
                &options.cancel,
                None,
                options.on_update,
            )
            .await;
            finalize(
                options.context,
                options.assistant_message,
                &call,
                &args,
                executed,
                &hooks,
                &options.cancel,
            )
            .await
        }
    }
}
