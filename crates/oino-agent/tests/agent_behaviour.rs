//! Behaviour tests for `Agent`: busy errors, continue, abort, tool-loadout patches.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::future::BoxFuture;
use futures::stream::{self, BoxStream};
use oino_agent::{
    Agent, AgentError, AgentOptions, AgentTool, AgentToolResult, StreamFn, StreamRequest,
    ToolDefinitionMeta, ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, AssistantMessageEvent, DoneReason,
    ErrorReason, StopReason, ToolCall,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

/// Answers with a tool call to `slow` on the first request, text afterwards; honours abort.
struct Script;

impl StreamFn for Script {
    fn stream(&self, request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent> {
        let calls_tool = !request
            .context
            .messages
            .iter()
            .any(|m| matches!(m, oino_ai::Message::ToolResult(_)));
        let mut message = AssistantMessage::empty("faux", "faux", "faux-1", 0);
        if calls_tool {
            message.content = vec![AssistantContent::ToolCall(ToolCall {
                id: "s1".into(),
                name: "slow".into(),
                ..ToolCall::default()
            })];
            message.stop_reason = StopReason::ToolUse;
        } else {
            message.content = vec![AssistantContent::text("done")];
            message.stop_reason = StopReason::Stop;
        }
        let cancel = request.cancel;
        Box::pin(stream::once(async move {
            if cancel.is_cancelled() {
                message.stop_reason = StopReason::Aborted;
                message.error_message = Some("aborted".into());
                return AssistantMessageEvent::Error {
                    reason: ErrorReason::Aborted,
                    error: message,
                };
            }
            let reason = if message.stop_reason == StopReason::ToolUse {
                DoneReason::ToolUse
            } else {
                DoneReason::Stop
            };
            AssistantMessageEvent::Done { reason, message }
        }))
        .boxed()
    }
}

struct Slow {
    meta: ToolDefinitionMeta,
}

impl AgentTool for Slow {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn execute(
        &self,
        _call: ToolInvocation,
        _updates: UpdateSink,
        cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        Box::pin(async move {
            tokio::select! {
                () = cancel.cancelled() => Err(ToolError::new("Operation aborted")),
                () = tokio::time::sleep(Duration::from_millis(200)) => Ok(AgentToolResult::text("slept")),
            }
        })
    }
}

fn slow_tool(name: &str) -> Arc<dyn AgentTool> {
    Arc::new(Slow {
        meta: ToolDefinitionMeta::new(
            name,
            name,
            "sleeps",
            json!({"type":"object","properties":{}}),
        ),
    })
}

fn agent() -> Agent {
    let mut options = AgentOptions::new(Arc::new(Script));
    options.system_prompt = "S".into();
    options.tools = vec![slow_tool("slow")];
    Agent::new(options)
}

#[tokio::test]
async fn busy_and_continue_errors() {
    let agent = agent();
    assert_eq!(agent.continue_run().await, Err(AgentError::NoMessages));
    let running = agent.clone();
    let task = tokio::spawn(async move { running.prompt_text("go", Vec::new()).await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(agent.is_streaming());
    assert_eq!(
        agent.prompt_text("again", Vec::new()).await,
        Err(AgentError::PromptWhileBusy)
    );
    assert_eq!(agent.reset(), Err(AgentError::ResetWhileBusy));
    task.await.unwrap().unwrap();
    agent.wait_for_idle().await;
    assert!(!agent.is_streaming());
    assert_eq!(agent.continue_run().await, Err(AgentError::CannotContinue));
    // A queued follow-up lets `continue` run after an assistant message.
    agent.follow_up(AgentMessage::User(oino_ai::UserMessage {
        content: "more".into(),
        timestamp: 0,
    }));
    agent.continue_run().await.unwrap();
    assert!(matches!(
        agent.messages().last(),
        Some(AgentMessage::Assistant(_))
    ));
}

#[tokio::test]
async fn abort_cancels_running_tool() {
    let agent = agent();
    let running = agent.clone();
    let task = tokio::spawn(async move { running.prompt_text("go", Vec::new()).await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    agent.abort();
    task.await.unwrap().unwrap();
    let messages = agent.messages();
    let result = messages
        .iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) => Some(r.clone()),
            _ => None,
        })
        .unwrap();
    assert!(result.is_error);
    // The follow-up request sees the abort and ends the run with an aborted message.
    match messages.last() {
        Some(AgentMessage::Assistant(last)) => assert_eq!(last.stop_reason, StopReason::Aborted),
        other => panic!("unexpected last message {other:?}"),
    }
}

#[tokio::test]
async fn tool_changes_are_declared_before_next_request() {
    let agent = agent();
    agent.prompt_text("one", Vec::new()).await.unwrap();
    agent.set_tools(vec![slow_tool("slow"), slow_tool("other")]);
    agent.follow_up(AgentMessage::User(oino_ai::UserMessage {
        content: "two".into(),
        timestamp: 0,
    }));
    agent.continue_run().await.unwrap();
    let patch = agent
        .messages()
        .into_iter()
        .filter_map(|m| match m {
            AgentMessage::System(system) => Some(system),
            _ => None,
        })
        .nth(1)
        .unwrap();
    let added: Vec<_> = patch
        .tools_added
        .unwrap()
        .into_iter()
        .map(|t| t.name)
        .collect();
    assert_eq!(added, vec!["other".to_owned()]);
    assert!(patch.tools_removed.is_none());
    assert_eq!(agent.system_prompt(), "S");
}
