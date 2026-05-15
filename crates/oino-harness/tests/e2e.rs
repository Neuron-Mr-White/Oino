use oino_agent::Agent;
use oino_agent_loop::{
    AgentEvent, AgentLoopConfig, FakeTool, FauxStream, Tool, ToolExecutionMode, VecEventSink,
};
use oino_env::{CommandOptions, ExecutionEnv, LocalExecutionEnv};
use oino_harness::{Harness, HarnessConfig, NotificationHook};
use oino_session::{SessionHeader, SessionManager};
use oino_types::{AssistantStreamEvent, Message, Model, StopReason, ThinkingLevel};
use std::{collections::BTreeMap, path::PathBuf, sync::Arc};
use tokio::sync::Mutex;
use uuid::Uuid;

fn model() -> Model {
    Model::new("test", "faux")
}

#[tokio::test]
async fn full_prompt_lifecycle_streams_and_saves() {
    let sink = VecEventSink::new();
    let session = SessionManager::new(SessionHeader::new("e2e", PathBuf::from("/tmp")));
    let stream = Arc::new(FauxStream::once(vec![
        AssistantStreamEvent::TextDelta { delta: "he".into() },
        AssistantStreamEvent::TextDelta {
            delta: "llo".into(),
        },
        AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        },
    ]));
    let mut cfg = HarnessConfig::new(model(), stream, session);
    cfg.event_sink = Arc::new(sink.clone());
    let harness = Harness::new(cfg);
    let output = harness.prompt(Message::user_text("hi")).await;
    assert!(output.is_ok());
    let context = harness.build_context().await;
    let context = match context {
        Ok(context) => context,
        Err(err) => panic!("context failed: {err}"),
    };
    assert!(context
        .iter()
        .any(|msg| matches!(msg, Message::Assistant { .. })));
    let events = sink.events().await;
    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::MessageUpdate { .. })));
}

#[tokio::test]
async fn tool_execution_supports_parallel_and_sequential_modes() {
    let call_a = Uuid::new_v4();
    let call_b = Uuid::new_v4();
    let stream = Arc::new(FauxStream::turns(vec![
        vec![
            AssistantStreamEvent::ToolCallDone {
                id: call_a,
                name: "a".into(),
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::ToolCallDone {
                id: call_b,
                name: "b".into(),
                arguments: serde_json::json!({}),
            },
            AssistantStreamEvent::Done {
                stop_reason: StopReason::ToolUse,
                provider: None,
            },
        ],
        vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }],
    ]));
    let mut cfg = AgentLoopConfig::new(model(), stream);
    let mut a = FakeTool::new("a", "A");
    a.mode = ToolExecutionMode::Sequential;
    cfg.tools.insert("a".into(), Arc::new(a));
    cfg.tools.insert(
        "b".into(),
        Arc::new(FakeTool::new("b", "B")) as Arc<dyn Tool>,
    );
    let output = oino_agent_loop::run_agent_loop(Message::user_text("tools"), cfg).await;
    let output = match output {
        Ok(output) => output,
        Err(err) => panic!("loop failed: {err}"),
    };
    let names: Vec<String> = output
        .messages
        .into_iter()
        .filter_map(|msg| match msg {
            Message::ToolResult { tool_name, .. } => Some(tool_name),
            _ => None,
        })
        .collect();
    assert_eq!(names, vec!["a".to_string(), "b".to_string()]);
}

#[tokio::test]
async fn steering_followup_and_abort_are_exercised() {
    let stream = Arc::new(FauxStream::turns(vec![
        vec![AssistantStreamEvent::Done {
            stop_reason: StopReason::EndTurn,
            provider: None,
        }],
        vec![AssistantStreamEvent::Aborted],
    ]));
    let mut cfg = AgentLoopConfig::new(model(), stream);
    cfg.max_turns = 2;
    let agent = Agent::new(cfg);
    let queued = agent.follow_up(Message::user_text("next turn")).await;
    assert!(queued.is_ok());
    let output = agent.prompt(Message::user_text("start")).await;
    assert!(output.is_ok());
    agent.abort().await;
    assert!(agent.is_idle().await);
}

#[tokio::test]
async fn sessions_jsonl_compaction_and_branch_summaries_roundtrip() {
    let dir = match tempfile::tempdir() {
        Ok(dir) => dir,
        Err(err) => panic!("tempdir failed: {err}"),
    };
    let path = dir.path().join("session.jsonl");
    let mut session = SessionManager::new(SessionHeader::new("tree", PathBuf::from("/tmp")));
    let root = session.append_message(Message::user_text("root"));
    let branch = session.branch_with_summary(root, "branch summary");
    assert!(branch.is_ok());
    let replaced = session.append_compaction("compact summary", vec![root]);
    session.append_message(Message::Custom {
        id: Uuid::new_v4(),
        name: "custom".into(),
        payload: serde_json::json!({"ok": true}),
        model_visible: false,
    });
    let context = session.build_session_context();
    let context = match context {
        Ok(context) => context,
        Err(err) => panic!("context failed: {err}"),
    };
    assert!(matches!(
        context.messages.first(),
        Some(Message::CompactionSummary { .. })
    ));
    assert!(session.reset_leaf(Some(replaced)).is_ok());
    assert!(session.save_jsonl(&path).await.is_ok());
    let loaded = SessionManager::load_jsonl(&path).await;
    let loaded = match loaded {
        Ok(loaded) => loaded,
        Err(err) => panic!("load failed: {err}"),
    };
    assert_eq!(loaded.get_entries().len(), session.get_entries().len());
}

#[tokio::test]
async fn hooks_env_model_and_resources_are_available() {
    let stream = Arc::new(FauxStream::once(vec![AssistantStreamEvent::Done {
        stop_reason: StopReason::EndTurn,
        provider: None,
    }]));
    let session = SessionManager::new(SessionHeader::new("hooks", PathBuf::from("/tmp")));
    let harness = Harness::new(HarnessConfig::new(model(), stream, session));
    let seen = Arc::new(Mutex::new(0usize));
    let seen_clone = Arc::clone(&seen);
    harness
        .hooks()
        .on_notification(
            NotificationHook::SavePoint,
            Arc::new(move |_event| {
                let seen = Arc::clone(&seen_clone);
                Box::pin(async move {
                    *seen.lock().await += 1;
                })
            }),
        )
        .await;
    assert!(harness.set_model(Model::new("test", "other")).await.is_ok());
    assert!(harness.set_thinking_level(ThinkingLevel::Low).await.is_ok());
    harness
        .set_resources(vec!["skills".into(), "themes".into()])
        .await;
    assert_eq!(harness.resources().await.len(), 2);
    let mut tools: BTreeMap<String, Arc<dyn Tool>> = BTreeMap::new();
    tools.insert("fake".into(), Arc::new(FakeTool::new("fake", "ok")));
    harness.set_tools(tools).await;
    assert!(harness.prompt(Message::user_text("hi")).await.is_ok());
    assert_eq!(*seen.lock().await, 1);

    let env = LocalExecutionEnv;
    let output = env.shell("printf env", CommandOptions::default()).await;
    let output = match output {
        Ok(output) => output,
        Err(err) => panic!("env failed: {err}"),
    };
    assert_eq!(output.stdout, "env");
}
