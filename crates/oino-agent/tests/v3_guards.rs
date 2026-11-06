//! v3 loop guards, exercised through the real loop with a scripted provider.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "test assertions"
)]

use futures::{
    future::BoxFuture,
    stream::{self, BoxStream},
};
use oino_agent::{
    Agent, AgentOptions, AgentTool, AgentToolResult, StreamFn, StreamRequest, ToolDefinitionMeta,
    ToolError, ToolInvocation, UpdateSink,
};
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, AssistantMessageEvent, DoneReason,
    StopReason, ToolCall, UserMessage,
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use tokio_util::sync::CancellationToken;

const MISSING: &str = "TodoWrite has not been called. If the task requires 3 or more distinct actions or tool calls, create a list and keep it current; otherwise ignore this reminder. Performance tip: call the TodoWrite tool in parallel to the main flow related tool calls to save user's time and tokens.";
const STALE: &str = "IMPORTANT: Your todo list has pending items but hasn't been updated in a while. Update it with todowrite to reflect your current progress: mark finished items completed and keep exactly one item in_progress.";
const SETTLE: &str = "IMPORTANT: You may be ending the turn, but the TodoWrite plan still has open items. Continue working on every actionable pending or in-progress item before ending the turn. Do not stop merely because the plan is accurate. Call TodoWrite as work finishes. Once no actionable work remains, end the turn by replying with no text and no tool calls.\nDo not reference this reminder to the user.";
const STEERING: &str = "A follow-up user message arrived while you were working. Treat it as steering, not implicit cancellation. Continue all earlier actionable instructions unless the user explicitly canceled or replaced them. Respect blockers that require user input.";

struct Faux {
    responses: Mutex<std::collections::VecDeque<AssistantMessage>>,
    requests: Mutex<Vec<StreamRequest>>,
}
impl StreamFn for Faux {
    fn stream(&self, request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent> {
        self.requests.lock().unwrap().push(request);
        let message = self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .expect("unexpected extra request");
        Box::pin(stream::iter([AssistantMessageEvent::Done {
            reason: DoneReason::Stop,
            message,
        }]))
    }
}
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
        Box::pin(async { Ok(AgentToolResult::text("ok")) })
    }
}
fn response(content: Vec<AssistantContent>, stop_reason: StopReason) -> AssistantMessage {
    AssistantMessage {
        content,
        stop_reason,
        ..AssistantMessage::empty("faux", "faux", "faux", 0)
    }
}
fn text(text: &str) -> AssistantMessage {
    response(vec![AssistantContent::text(text)], StopReason::Stop)
}
fn call(id: usize, name: &str, args: Value) -> AssistantMessage {
    response(
        vec![AssistantContent::ToolCall(ToolCall {
            id: id.to_string(),
            name: name.into(),
            arguments: args.as_object().unwrap().clone(),
            ..ToolCall::default()
        })],
        StopReason::ToolUse,
    )
}
fn todo(id: usize, status: &str) -> AssistantMessage {
    call(
        id,
        "todowrite",
        json!({"todos":[{"content":"work", "status":status}]}),
    )
}
fn setup(responses: Vec<AssistantMessage>) -> (Agent, Arc<Faux>, Arc<AtomicUsize>) {
    let faux = Arc::new(Faux {
        responses: Mutex::new(responses.into()),
        requests: Mutex::new(Vec::new()),
    });
    let count = Arc::new(AtomicUsize::new(0));
    let mut options = AgentOptions::new(faux.clone());
    options.tools = ["work", "todowrite"]
        .map(|name| {
            Arc::new(Tool {
                meta: ToolDefinitionMeta::new(name, name, name, json!({"type":"object"})),
                count: count.clone(),
            }) as Arc<dyn AgentTool>
        })
        .to_vec();
    // The host's custom-message projection is represented explicitly here.
    options.convert_to_llm = Arc::new(|messages| {
        messages
            .iter()
            .filter_map(|message| match message {
                AgentMessage::Custom(custom) => Some(oino_ai::Message::User(UserMessage {
                    content: custom.content.clone(),
                    timestamp: custom.timestamp,
                })),
                _ => message.as_llm(),
            })
            .collect()
    });
    (Agent::new(options), faux, count)
}
fn reminders(agent: &Agent) -> Vec<String> {
    agent
        .messages()
        .iter()
        .filter_map(|m| match m {
            AgentMessage::Custom(c) if c.custom_type == "oino.system-reminder" => {
                assert!(!c.display);
                assert!(
                    c.details.as_ref().unwrap()["key"]
                        .as_str()
                        .unwrap()
                        .starts_with("oino/")
                );
                Some(c.content.text(""))
            }
            _ => None,
        })
        .collect()
}
fn envelope(text: &str) -> String {
    format!("<system-reminder>\n{text}\n</system-reminder>")
}
fn has_notice(agent: &Agent, part: &str) -> bool {
    agent.messages().iter().any(
        |m| matches!(m, AgentMessage::Custom(c) if c.display && c.content.text("").contains(part)),
    )
}

#[tokio::test]
async fn todo_thresholds() {
    let mut script = (0..6)
        .map(|i| call(i, "work", json!({"n":i})))
        .collect::<Vec<_>>();
    script.push(text("done"));
    let (agent, faux, _) = setup(script);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(reminders(&agent), [envelope(MISSING)]);
    let requests = faux.requests.lock().unwrap();
    assert!(
        !requests[4]
            .context
            .messages
            .iter()
            .any(|m| format!("{m:?}").contains(MISSING))
    );
    assert!(
        requests[5]
            .context
            .messages
            .iter()
            .any(|m| format!("{m:?}").contains(MISSING))
    );
    drop(requests);

    let mut script = vec![todo(0, "pending")];
    script.extend((1..=19).map(|i| call(i, "work", json!({"n":i}))));
    // Reset immediately before the threshold; only the next 20 calls count.
    script.push(todo(20, "in_progress"));
    script.extend((21..=40).map(|i| call(i, "work", json!({"n":i}))));
    script.extend([text("done"), text("still done")]);
    let (agent, faux, _) = setup(script);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(reminders(&agent), [envelope(STALE), envelope(SETTLE)]);
    let requests = faux.requests.lock().unwrap();
    assert!(
        !requests[40]
            .context
            .messages
            .iter()
            .any(|m| format!("{m:?}").contains(STALE))
    );
    assert!(
        requests[41]
            .context
            .messages
            .iter()
            .any(|m| format!("{m:?}").contains(STALE))
    );
    drop(requests);

    let (agent, _, _) = setup(vec![
        todo(0, "pending"),
        text("done"),
        todo(1, "completed"),
        text("done"),
    ]);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(reminders(&agent), [envelope(SETTLE)]);
}

#[tokio::test]
async fn empty_and_identical_calls() {
    let empty = response(vec![], StopReason::Stop);
    let (agent, faux, count) = setup(vec![empty.clone(), empty.clone(), empty]);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(faux.requests.lock().unwrap().len(), 3);
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert!(has_notice(&agent, "three consecutive empty"));

    // Nested object order is irrelevant, array order is not. IDs do not matter.
    let (agent, faux, count) = setup(vec![
        call(1, "work", json!({"a":{"x":1,"y":2},"b":[1,2]})),
        call(2, "work", json!({"b":[1,2],"a":{"y":2,"x":1}})),
        call(3, "work", json!({"a":{"x":1,"y":2},"b":[1,2]})),
    ]);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(faux.requests.lock().unwrap().len(), 3);
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert!(has_notice(&agent, "identical tool calls"));
    assert_eq!(
        agent
            .messages()
            .iter()
            .filter(|m| matches!(m, AgentMessage::ToolResult(_)))
            .count(),
        3
    );

    let (agent, _, count) = setup(vec![
        call(1, "work", json!({"a":[1,2]})),
        call(2, "work", json!({"a":[2,1]})),
        call(3, "work", json!({"a":[1,2]})),
        text("done"),
    ]);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(count.load(Ordering::SeqCst), 3);
    assert!(!has_notice(&agent, "identical"));
}

#[tokio::test]
async fn resample_before_tools() {
    let mut bad = call(0, "work", json!({"n":0}));
    bad.content
        .insert(0, AssistantContent::text("loop\n".repeat(16)));
    let (agent, faux, count) = setup(vec![
        bad.clone(),
        bad.clone(),
        bad.clone(),
        text("recovered"),
    ]);
    agent.prompt_text("go", vec![]).await.unwrap();
    // Rejected samples must not contribute to identical *executed* call history.
    assert_eq!(count.load(Ordering::SeqCst), 0);
    assert_eq!(faux.requests.lock().unwrap().len(), 4);
    assert!(!has_notice(&agent, "identical"));

    let (agent, faux, _) = setup(vec![bad.clone(), bad.clone(), bad.clone(), bad]);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(faux.requests.lock().unwrap().len(), 4);
    assert!(has_notice(&agent, "three resamples"));

    let (agent, faux, count) = setup(vec![
        response(vec![], StopReason::Length),
        call(1, "work", json!({})),
        text("done"),
    ]);
    agent.prompt_text("go", vec![]).await.unwrap();
    assert!(reminders(&agent).contains(&envelope("Continue.")));
    assert!(
        faux.requests.lock().unwrap()[1]
            .context
            .messages
            .iter()
            .any(|m| format!("{m:?}").contains("Continue."))
    );
    assert_eq!(count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn steering() {
    let (agent, _, count) = setup(vec![
        call(1, "work", json!({})),
        text("done"),
        text("steered"),
    ]);
    agent.follow_up(AgentMessage::User(UserMessage {
        content: "also do this".into(),
        timestamp: 0,
    }));
    agent.prompt_text("go", vec![]).await.unwrap();
    assert_eq!(reminders(&agent), [envelope(STEERING)]);
    assert_eq!(count.load(Ordering::SeqCst), 1);
}
