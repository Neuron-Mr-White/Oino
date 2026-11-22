//! A legacy dotted model call executes the renamed tool.
#![allow(clippy::unwrap_used, clippy::expect_used)]
use futures::future::BoxFuture;
use oino_agent::{
    AgentContext, AgentTool, AgentToolResult, RunToolCallOptions, ToolDefinitionMeta, ToolError,
    ToolInvocation, UpdateSink, run_tool_call,
};
use oino_ai::{AssistantMessage, ToolCall};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

struct Tool {
    meta: ToolDefinitionMeta,
    count: Arc<AtomicUsize>,
}
impl AgentTool for Tool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }
    fn execute(
        &self,
        _: ToolInvocation,
        _: UpdateSink,
        _: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        self.count.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(AgentToolResult::text("executed")) })
    }
}
#[tokio::test]
async fn dotted_start_executes_underscore_tool() {
    let count = Arc::new(AtomicUsize::new(0));
    let tool: Arc<dyn AgentTool> = Arc::new(Tool {
        meta: ToolDefinitionMeta::new(
            "superagent_start",
            "start",
            "start",
            serde_json::json!({"type":"object","properties":{},"additionalProperties":false}),
        ),
        count: count.clone(),
    });
    let tools = vec![tool];
    let context = AgentContext {
        tools: tools.clone(),
        ..Default::default()
    };
    let assistant = AssistantMessage::empty("a", "p", "m", 0);
    let call = ToolCall {
        id: "call_1".into(),
        name: "superagent.start".into(),
        ..Default::default()
    };
    let result = run_tool_call(
        call,
        RunToolCallOptions {
            assistant_message: &assistant,
            context: &context,
            tools: &tools,
            before_tool_call: None,
            after_tool_call: None,
            cancel: CancellationToken::new(),
            parent_tool_call_id: None,
            on_update: None,
        },
    )
    .await;
    assert!(!result.is_error, "{:?}", result.result);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
