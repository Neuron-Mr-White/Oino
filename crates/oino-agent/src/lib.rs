//! `oino-agent`: the agent runtime for Oino.
//!
//! Provider-agnostic agent loop: turns, steering and follow-up queues, sequential and parallel
//! tool execution, abort, and every loop hook. It knows nothing about
//! sessions, extensions or providers; `oino-core` supplies those through the hook traits and a
//! [`StreamFn`].
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`tool`] | [`AgentTool`], [`ToolDefinitionMeta`], [`AgentToolResult`], [`UpdateSink`], [`ToolError`] |
//! | [`event`] | [`AgentEvent`], [`EventSink`] |
//! | [`config`] | [`AgentLoopConfig`], [`AgentContext`], [`StreamFn`], hook traits |
//! | [`mod@agent_loop`] | [`agent_loop()`], [`agent_loop_continue`], [`run_tool_call`], [`declare_tool_changes`] |
//! | [`agent`] | [`Agent`], [`AgentOptions`], [`AgentListener`], [`QueueMode`] |
//! | [`validate`] | [`validate_tool_arguments`] |

#![doc(
    html_logo_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-256.png",
    html_favicon_url = "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/oino.ico"
)]
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
