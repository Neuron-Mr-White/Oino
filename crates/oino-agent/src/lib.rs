//! `oino-agent`: the pi-agent-core runtime for Oino.
//!
//! Provider-agnostic agent loop: turns, steering and follow-up queues, sequential and parallel
//! tool execution, abort, and every loop hook with pi 1.4.0 semantics. It knows nothing about
//! sessions, extensions or providers; `oino-core` supplies those through the hook traits and a
//! [`StreamFn`].
//!
//! # API map
//!
//! | Module | pi equivalent | Contents |
//! |---|---|---|
//! | [`tool`] | `AgentTool`, `AgentToolResult` | [`AgentTool`], [`ToolDefinitionMeta`], [`AgentToolResult`], [`UpdateSink`], [`ToolError`] |
//! | [`event`] | `AgentEvent` | [`AgentEvent`], [`EventSink`] |
//! | [`config`] | `AgentLoopConfig` + hooks | [`AgentLoopConfig`], [`AgentContext`], [`StreamFn`], hook traits |
//! | [`mod@agent_loop`] | `agent-loop.js` | [`agent_loop()`], [`agent_loop_continue`], [`run_tool_call`], [`declare_tool_changes`] |
//! | [`agent`] | `agent.js` | [`Agent`], [`AgentOptions`], [`AgentListener`], [`QueueMode`] |
//! | [`validate`] | pi-ai `validateToolArguments` | [`validate_tool_arguments`] |

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        reason = "tests assert by unwrapping"
    )
)]

pub mod agent;
pub mod agent_loop;
pub mod config;
pub mod event;
pub mod tool;
pub mod validate;

pub use agent::{
    Agent, AgentError, AgentHooks, AgentListener, AgentOptions, AgentState, QueueMode, Subscription,
};
pub use agent_loop::{
    AgentLoopError, RunToolCallOptions, ToolCallOutcome, agent_loop, agent_loop_continue,
    declare_tool_changes, now_ms, run_tool_call, tool_result_message,
};
pub use config::{
    AfterToolCall, AfterToolCallContext, AfterToolCallResult, AgentContext, AgentLoopConfig,
    AgentLoopTurnUpdate, AgentRequestUpdate, AgentTurnContext, BeforeToolCall,
    BeforeToolCallContext, BeforeToolCallResult, ConvertToLlm, FinishTurn, GetApiKey, HookError,
    PrepareNextTurn, PrepareRequest, PrepareRequestContext, QueueSource, StreamFn, StreamRequest,
    ToolExecution, TransformContext, TurnDecision, default_convert_to_llm,
};
pub use event::{AgentEvent, EventSink};
pub use tool::{
    AgentTool, AgentToolResult, ExecutionMode, Replay, ToolDefinitionMeta, ToolError,
    ToolInvocation, ToolUpdate, UpdateSink,
};
pub use validate::{ValidationError, check, validate_tool_arguments};
