//! Golden test: the scripted scenarios of `scripts/pi-oracle/core-runtime/agent/` produce the
//! same events and transcript as pi-agent-core's `Agent`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::{Arc, Mutex, OnceLock};

use futures::future::BoxFuture;
use futures::stream::{self, BoxStream};
use oino_agent::{
    AfterToolCall, AfterToolCallContext, AfterToolCallResult, Agent, AgentEvent, AgentHooks,
    AgentListener, AgentOptions, AgentTool, AgentToolResult, BeforeToolCall, BeforeToolCallContext,
    BeforeToolCallResult, ExecutionMode, FinishTurn, HookError, QueueMode, StreamFn, StreamRequest,
    ToolDefinitionMeta, ToolError, ToolInvocation, TurnDecision, UpdateSink,
};
use oino_ai::json::json_semantic_eq;
use oino_ai::{
    AgentMessage, AssistantContent, AssistantMessage, AssistantMessageEvent, DoneReason,
    ErrorReason, JsonValue, Model, PartialAssembler, ProviderEvent, StopReason, ThinkingLevel,
    UserContent, UserContentField, UserMessage,
};
use serde_json::json;
use tokio_util::sync::CancellationToken;

fn fixture(name: &str) -> JsonValue {
    let path = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

fn model() -> Model {
    serde_json::from_value(json!({
        "id": "faux-1", "name": "Faux", "api": "faux", "provider": "faux", "baseUrl": "",
        "reasoning": true, "input": ["text"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 1000, "maxTokens": 100
    }))
    .unwrap()
}

struct Faux {
    responses: Mutex<std::vec::IntoIter<JsonValue>>,
}

impl StreamFn for Faux {
    fn stream(&self, _request: StreamRequest) -> BoxStream<'static, AssistantMessageEvent> {
        let spec = self.responses.lock().unwrap().next().unwrap();
        let mut assembler =
            PartialAssembler::new(AssistantMessage::empty("faux", "faux", "faux-1", 0));
        let mut events = vec![assembler.apply(ProviderEvent::Start {})];
        for (index, block) in spec["content"].as_array().unwrap().iter().enumerate() {
            let provider_events = match block["type"].as_str().unwrap() {
                "text" => {
                    let text = block["text"].as_str().unwrap().to_owned();
                    vec![
                        ProviderEvent::TextStart {
                            content_index: index,
                        },
                        ProviderEvent::TextDelta {
                            content_index: index,
                            delta: text.clone(),
                        },
                        ProviderEvent::TextEnd {
                            content_index: index,
                            content: text,
                        },
                    ]
                }
                "thinking" => {
                    let text = block["thinking"].as_str().unwrap().to_owned();
                    vec![
                        ProviderEvent::ThinkingStart {
                            content_index: index,
                        },
                        ProviderEvent::ThinkingDelta {
                            content_index: index,
                            delta: text.clone(),
                        },
                        ProviderEvent::ThinkingEnd {
                            content_index: index,
                            content: text,
                        },
                    ]
                }
                _ => {
                    let call: oino_ai::ToolCall = serde_json::from_value(block.clone()).unwrap();
                    vec![
                        ProviderEvent::ToolcallStart {
                            content_index: index,
                            id: Some(call.id.clone()),
                            tool_name: Some(call.name.clone()),
                        },
                        ProviderEvent::ToolcallDelta {
                            content_index: index,
                            delta: serde_json::to_string(&call.arguments).unwrap(),
                        },
                        ProviderEvent::ToolcallEnd {
                            content_index: index,
                            tool_call: call,
                        },
                    ]
                }
            };
            events.extend(provider_events.into_iter().map(|e| assembler.apply(e)));
        }
        let mut message = assembler.partial().clone();
        message.stop_reason = serde_json::from_value(spec["stopReason"].clone()).unwrap();
        message.error_message = spec["errorMessage"].as_str().map(str::to_owned);
        let terminal = match message.stop_reason {
            StopReason::Error => ProviderEvent::Error {
                reason: ErrorReason::Error,
                error: message,
            },
            StopReason::Aborted => ProviderEvent::Error {
                reason: ErrorReason::Aborted,
                error: message,
            },
            StopReason::Length => ProviderEvent::Done {
                reason: DoneReason::Length,
                message,
            },
            StopReason::ToolUse => ProviderEvent::Done {
                reason: DoneReason::ToolUse,
                message,
            },
            _ => ProviderEvent::Done {
                reason: DoneReason::Stop,
                message,
            },
        };
        events.push(assembler.apply(terminal));
        Box::pin(stream::iter(events))
    }
}

struct ScriptedTool {
    meta: ToolDefinitionMeta,
    behaviour: JsonValue,
    agent: Arc<OnceLock<Agent>>,
}

fn user(text: &str) -> AgentMessage {
    AgentMessage::User(UserMessage {
        content: UserContentField::Blocks(vec![UserContent::text(text)]),
        timestamp: 0,
    })
}

impl AgentTool for ScriptedTool {
    fn meta(&self) -> &ToolDefinitionMeta {
        &self.meta
    }

    fn execute(
        &self,
        _call: ToolInvocation,
        updates: UpdateSink,
        _cancel: CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, ToolError>> {
        let behaviour = self.behaviour.clone();
        let agent = self.agent.get().cloned().unwrap();
        Box::pin(async move {
            for update in behaviour["updates"].as_array().into_iter().flatten() {
                updates.send(serde_json::from_value(update.clone()).unwrap());
            }
            for text in behaviour["steer"].as_array().into_iter().flatten() {
                agent.steer(user(text.as_str().unwrap()));
            }
            for text in behaviour["followUp"].as_array().into_iter().flatten() {
                agent.follow_up(user(text.as_str().unwrap()));
            }
            if let Some(message) = behaviour["throws"].as_str() {
                return Err(ToolError::new(message));
            }
            Ok(serde_json::from_value(behaviour["result"].clone()).unwrap())
        })
    }
}

struct Hooks(JsonValue, Mutex<usize>);

impl BeforeToolCall for Hooks {
    fn before<'a>(
        &'a self,
        cx: BeforeToolCallContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<BeforeToolCallResult>, HookError>> {
        let reason = self.0["block"][&cx.tool_call.name]
            .as_str()
            .map(str::to_owned);
        Box::pin(async move {
            Ok(reason.map(|reason| BeforeToolCallResult {
                block: true,
                reason: Some(reason),
                terminate: false,
            }))
        })
    }
}

impl AfterToolCall for Hooks {
    fn after<'a>(
        &'a self,
        cx: AfterToolCallContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<AfterToolCallResult>, HookError>> {
        let update = self.0["after"].get(&cx.tool_call.name).cloned();
        Box::pin(async move { Ok(update.map(|u| serde_json::from_value(u).unwrap())) })
    }
}

impl FinishTurn for Hooks {
    fn finish<'a>(
        &'a self,
        _turn: oino_agent::AgentTurnContext<'a>,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, Result<Option<TurnDecision>, HookError>> {
        let mut turn = self.1.lock().unwrap();
        let decision = self.0["finishTurn"][*turn].as_str().map(|d| match d {
            "continue" => TurnDecision::Continue,
            _ => TurnDecision::End,
        });
        *turn += 1;
        Box::pin(async move { Ok(decision) })
    }
}

struct Recorder(Mutex<Vec<JsonValue>>);

impl AgentListener for Recorder {
    fn on_event<'a>(
        &'a self,
        event: &'a AgentEvent,
        _cancel: &'a CancellationToken,
    ) -> BoxFuture<'a, ()> {
        self.0
            .lock()
            .unwrap()
            .push(serde_json::to_value(event).unwrap());
        Box::pin(async {})
    }
}

/// `oino_ai::ToolCall` serialises without `"type":"toolCall"` outside a content array
/// (`toolcall_end.toolCall`); pi includes it. Reported to core-types-session.
fn strip(value: &JsonValue) -> JsonValue {
    let mut value = strip_keys(value);
    for event in value.as_array_mut().into_iter().flatten() {
        if let Some(call) = event
            .get_mut("assistantMessageEvent")
            .and_then(|e| e.get_mut("toolCall"))
            .and_then(JsonValue::as_object_mut)
        {
            call.shift_remove("type");
        }
    }
    // Completion order of concurrent tools depends on the scheduler (JS microtasks vs tokio);
    // pi only promises "completion order". Runs of update/end events are compared as sets.
    if let Some(events) = value.as_array_mut() {
        let concurrent = |e: &JsonValue| {
            matches!(
                e["type"].as_str(),
                Some("tool_execution_update" | "tool_execution_end")
            )
        };
        let mut start = 0;
        while start < events.len() {
            let mut end = start;
            while end < events.len() && concurrent(&events[end]) {
                end += 1;
            }
            events[start..end]
                .sort_by_key(|e| (e["toolCallId"].to_string(), e["type"].to_string()));
            start = end + 1;
        }
    }
    value
}

fn strip_keys(value: &JsonValue) -> JsonValue {
    match value {
        JsonValue::Array(items) => JsonValue::Array(items.iter().map(strip_keys).collect()),
        JsonValue::Object(map) => JsonValue::Object(
            map.iter()
                .filter(|(k, _)| !matches!(k.as_str(), "timestamp" | "durationMs" | "partial"))
                .map(|(k, v)| (k.clone(), strip_keys(v)))
                .collect(),
        ),
        other => other.clone(),
    }
}

#[allow(
    clippy::too_many_lines,
    reason = "one scenario driver mirrors the oracle script"
)]
#[tokio::test]
async fn agent_matches_pi_scenarios() {
    let scenarios = fixture("agent-scenarios.json");
    let expected = fixture("agent-loop.json");
    for (scenario, golden) in scenarios
        .as_array()
        .unwrap()
        .iter()
        .zip(expected.as_array().unwrap())
    {
        let name = scenario["name"].as_str().unwrap();
        let handle = Arc::new(OnceLock::new());
        let tools: Vec<Arc<dyn AgentTool>> = scenario["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|spec| {
                let tool_name = spec["name"].as_str().unwrap();
                let mut meta = ToolDefinitionMeta::new(
                    tool_name,
                    tool_name,
                    format!("{tool_name} tool"),
                    spec["parameters"].clone(),
                );
                if spec["executionMode"] == "sequential" {
                    meta.execution_mode = ExecutionMode::Sequential;
                }
                Arc::new(ScriptedTool {
                    meta,
                    behaviour: spec["behaviour"].clone(),
                    agent: Arc::clone(&handle),
                }) as Arc<dyn AgentTool>
            })
            .collect();
        let responses: Vec<JsonValue> = scenario["responses"].as_array().unwrap().clone();
        let mut options = AgentOptions::new(Arc::new(Faux {
            responses: Mutex::new(responses.into_iter()),
        }));
        options.system_prompt = scenario["systemPrompt"].as_str().unwrap().to_owned();
        options.model = model();
        options.thinking_level = scenario["thinkingLevel"]
            .as_str()
            .map_or(ThinkingLevel::Off, |l| l.parse().unwrap());
        options.tools = tools;
        if scenario["steeringMode"] == "all" {
            options.steering_mode = QueueMode::All;
        }
        let hooks = Arc::new(Hooks(scenario["hooks"].clone(), Mutex::new(0)));
        options.hooks = AgentHooks {
            before_tool_call: scenario["hooks"]["block"]
                .is_object()
                .then(|| Arc::clone(&hooks) as _),
            after_tool_call: scenario["hooks"]["after"]
                .is_object()
                .then(|| Arc::clone(&hooks) as _),
            finish_turn: scenario["hooks"]["finishTurn"]
                .is_array()
                .then(|| Arc::clone(&hooks) as _),
            ..AgentHooks::default()
        };
        let agent = Agent::new(options);
        handle.set(agent.clone()).ok();
        let recorder = Arc::new(Recorder(Mutex::new(Vec::new())));
        let _subscription = agent.subscribe(Arc::clone(&recorder) as _);
        agent
            .prompt_text(scenario["prompt"].as_str().unwrap(), Vec::new())
            .await
            .unwrap();

        let events = strip(&JsonValue::Array(recorder.0.lock().unwrap().clone()));
        let golden_events = &strip(&golden["events"]);
        let actual_types: Vec<_> = events
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["type"].clone())
            .collect();
        let golden_types: Vec<_> = golden_events
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["type"].clone())
            .collect();
        assert_eq!(actual_types, golden_types, "{name}: event order");
        for (index, (actual, wanted)) in events
            .as_array()
            .unwrap()
            .iter()
            .zip(golden_events.as_array().unwrap())
            .enumerate()
        {
            assert!(
                json_semantic_eq(actual, wanted),
                "{name}: event {index} differs\n rust: {actual}\n   pi: {wanted}"
            );
        }
        let messages = strip(&serde_json::to_value(agent.messages()).unwrap());
        assert!(
            json_semantic_eq(&messages, &golden["messages"]),
            "{name}: transcript differs\n rust: {messages}\n   pi: {}",
            golden["messages"]
        );
        let error = agent.state().error_message;
        assert_eq!(
            error.as_deref(),
            golden["errorMessage"].as_str(),
            "{name}: errorMessage"
        );
        let _ = AssistantContent::text("");
    }
}
