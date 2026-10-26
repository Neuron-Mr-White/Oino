//! `AgentSession` behaviour against the faux provider and fake extensions (pi semantics).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use oino_agent::AgentToolResult;
use oino_ai::{AgentMessage, AssistantContent, JsonValue, StopReason, ThinkingLevel, Usage};
use oino_core::events::{AgentSessionEvent, ClientEvent};
use oino_core::extensions::api::{CommandSpec, Registration, ToolSpec};
use oino_core::session::{
    AgentSession, PromptDisposition, PromptOptions, SessionOpError, StreamingBehavior,
};
use oino_core::testing::{FakeExtension, FauxProvider, FauxResponse, TestSession};
use oino_session::EntryKind;
use serde_json::json;

fn kinds(events: &[ClientEvent]) -> Vec<String> {
    events
        .iter()
        .map(|e| {
            serde_json::to_value(e).unwrap()["type"]
                .as_str()
                .unwrap()
                .to_owned()
        })
        .collect()
}

async fn run(session: &AgentSession, text: &str) -> PromptDisposition {
    let disposition = session
        .prompt(text.into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
    disposition
}

fn args(value: JsonValue) -> oino_ai::JsonObject {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn prompt_runs_and_persists_pi_event_sequence() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("hello".into())]);
    let session = TestSession::default().build(Arc::clone(&provider)).await;
    let events = session.subscribe();
    assert_eq!(run(&session, "hi").await, PromptDisposition::Started);
    let kinds = kinds(&events.drain_now());
    let expected_prefix = [
        "agent_start",
        "turn_start",
        "message_start",
        "message_end",
        "message_start",
        "message_end",
        "message_start",
        "message_update",
        "message_update",
        "message_update",
        "message_end",
        "turn_end",
        "agent_end",
        "agent_settled",
    ];
    let filtered: Vec<&str> = kinds
        .iter()
        .map(String::as_str)
        .filter(|k| !matches!(*k, "tools_changed"))
        .collect();
    assert_eq!(filtered, expected_prefix, "{kinds:?}");
    assert_eq!(session.last_assistant_text().as_deref(), Some("hello"));
    let roles: Vec<&str> = session
        .entries()
        .iter()
        .filter_map(|e| e.message())
        .map(oino_core::extensions::runner::role_of)
        .collect();
    assert_eq!(roles, vec!["system", "user", "assistant"]);
    let request = &provider.requests()[0];
    assert_eq!(request["context"]["messages"][0]["role"], json!("system"));
    assert!(
        request["context"]["messages"][0]["sections"]["preamble"]
            .as_str()
            .unwrap()
            .contains("Oino")
    );
    assert_eq!(request["apiKey"], json!("test-key"));
    let stats = session.session_stats();
    assert_eq!((stats.user_messages, stats.assistant_messages), (1, 1));
}

#[tokio::test]
async fn extension_tools_hooks_and_results() {
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![
            ("c1".into(), "greet".into(), args(json!({"name": "x"}))),
            ("c2".into(), "danger".into(), args(json!({}))),
        ]),
        FauxResponse::Text("done".into()),
    ]);
    let schema = json!({"type": "object", "properties": {"name": {"type": "string"}}});
    let tool = |name: &str| ToolSpec {
        name: name.into(),
        label: name.into(),
        description: format!("{name} tool"),
        parameters: schema.clone(),
        prompt_snippet: Some(format!("{name} things")),
        prompt_guidelines: Vec::new(),
        exposure: oino_core::extensions::api::ToolExposure::Direct,
        default_active: None,
        execution_mode: None,
        namespace: None,
        annotations: None,
        reaches_tools: None,
        output_schema: None,
        prepare_arguments: false,
    };
    let extension = FakeExtension::new(
        "ext",
        Registration {
            tools: vec![tool("greet"), tool("danger")],
            events: vec!["tool_call".into(), "tool_result".into()],
            ..Registration::default()
        },
    )
    .on_event(|kind, payload| match kind {
        "tool_call" if payload["toolName"] == "danger" => {
            Ok(Some(json!({"block": true, "reason": "Nope"})))
        }
        "tool_result" => Ok(Some(
            json!({"content": [{"type": "text", "text": "rewritten"}]}),
        )),
        _ => Ok(None),
    })
    .on_tool(|name, args| Ok(AgentToolResult::text(format!("{name}:{}", args["name"]))));
    let session = TestSession {
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    assert!(session.active_tool_names().contains(&"greet".to_owned()));
    run(&session, "go").await;
    let results: Vec<_> = session
        .messages()
        .into_iter()
        .filter_map(|m| match m {
            AgentMessage::ToolResult(r) => Some(r),
            _ => None,
        })
        .collect();
    assert_eq!(results.len(), 2);
    assert_eq!(
        results[0].content,
        vec![oino_ai::UserContent::text("rewritten")]
    );
    assert!(results[1].is_error);
    let tools = provider.requests()[0]["context"]["messages"][0]["toolsAdded"].clone();
    assert_eq!(tools.as_array().unwrap().len(), 2);
    assert!(
        provider.requests()[0]["context"]["messages"][0]["sections"]["tools"]
            .as_str()
            .unwrap()
            .contains("greet things")
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queueing_and_streaming_errors() {
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![("c1".into(), "slow".into(), args(json!({})))]),
        FauxResponse::Text("after tool".into()),
        FauxResponse::Text("follow".into()),
    ]);
    let extension = FakeExtension::new(
        "ext",
        Registration {
            tools: vec![ToolSpec {
                name: "slow".into(),
                label: "slow".into(),
                description: "slow".into(),
                parameters: json!({"type": "object"}),
                prompt_snippet: None,
                prompt_guidelines: Vec::new(),
                exposure: oino_core::extensions::api::ToolExposure::Direct,
                default_active: None,
                execution_mode: None,
                namespace: None,
                annotations: None,
                reaches_tools: None,
                output_schema: None,
                prepare_arguments: false,
            }],
            ..Registration::default()
        },
    )
    .on_tool(|_, _| {
        std::thread::sleep(Duration::from_millis(150));
        Ok(AgentToolResult::text("slept"))
    });
    let session = TestSession {
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(provider)
    .await;
    let events = session.subscribe();
    session
        .prompt("start".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(session.is_streaming());
    assert_eq!(
        session.prompt("x".into(), PromptOptions::default()).await,
        Err(SessionOpError::StreamingWithoutBehavior)
    );
    let queued = session
        .prompt(
            "later".into(),
            PromptOptions {
                streaming_behavior: Some(StreamingBehavior::FollowUp),
                ..PromptOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(queued, PromptDisposition::Queued);
    assert_eq!(session.follow_up_messages(), vec!["later".to_owned()]);
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
    assert_eq!(session.follow_up_messages().len(), 0);
    let queue_updates: Vec<_> = events
        .drain_now()
        .into_iter()
        .filter_map(|e| match e {
            ClientEvent::Session(AgentSessionEvent::QueueUpdate { follow_up, .. }) => {
                Some(follow_up)
            }
            _ => None,
        })
        .collect();
    assert_eq!(queue_updates, vec![vec!["later".to_owned()], vec![]]);
    assert_eq!(session.last_assistant_text().as_deref(), Some("follow"));
}

#[tokio::test]
async fn input_handlers_and_extension_commands() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    let extension = FakeExtension::new(
        "ext",
        Registration {
            events: vec!["input".into(), "before_agent_start".into()],
            commands: vec![CommandSpec {
                name: "hello".into(),
                description: Some("Say hello".into()),
                hidden: false,
                completions: false,
            }],
            ..Registration::default()
        },
    )
    .on_event(|kind, payload| match kind {
        "input" if payload["text"] == "swallow" => Ok(Some(json!({"action": "handled"}))),
        "input" => Ok(Some(json!({"action": "transform", "text": format!("{}!", payload["text"].as_str().unwrap())}))),
        "before_agent_start" => Ok(Some(json!({"message": {"customType": "note", "content": "injected", "display": false}}))),
        _ => Ok(None),
    })
    .on_command(|name, args| {
        assert_eq!((name, args), ("hello", "world"));
        Ok(None)
    });
    let session = TestSession {
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    assert_eq!(
        run(&session, "/hello world").await,
        PromptDisposition::Handled
    );
    assert_eq!(run(&session, "swallow").await, PromptDisposition::Handled);
    assert_eq!(provider.requests(), [] as [serde_json::Value; 0]);
    assert_eq!(run(&session, "hi").await, PromptDisposition::Started);
    let messages = &provider.requests()[0]["context"]["messages"];
    let texts: Vec<String> = messages
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user")
        .map(|m| m["content"].to_string())
        .collect();
    assert!(texts[0].contains("hi!"), "{texts:?}");
    assert!(texts[1].contains("injected"), "{texts:?}");
    let echo = session.entries().into_iter().any(|e| {
        matches!(&e.kind, EntryKind::Custom { custom_type, .. } if custom_type == "oino.command-echo")
    });
    assert!(echo);
    let commands = session.commands();
    assert_eq!(commands[0].name, "hello");
}

#[tokio::test]
async fn auto_retry_recovers_from_transient_errors() {
    let provider = FauxProvider::new(vec![
        FauxResponse::Error("529 overloaded_error: Overloaded".into()),
        FauxResponse::Text("recovered".into()),
    ]);
    let session = TestSession {
        settings: json!({"retry": {"baseDelayMs": 1}})
            .as_object()
            .cloned()
            .unwrap(),
        ..TestSession::default()
    }
    .build(provider)
    .await;
    let events = session.subscribe();
    run(&session, "hi").await;
    let kinds = kinds(&events.drain_now());
    assert!(kinds.contains(&"auto_retry_start".to_owned()), "{kinds:?}");
    assert!(kinds.contains(&"auto_retry_end".to_owned()), "{kinds:?}");
    assert_eq!(session.last_assistant_text().as_deref(), Some("recovered"));
    let omitted = session
        .entries()
        .into_iter()
        .filter(|e| matches!(e.kind, EntryKind::ContextEdit { .. }))
        .count();
    assert_eq!(omitted, 1);
    let errors_in_context = session
        .messages()
        .iter()
        .filter(|m| matches!(m, AgentMessage::Assistant(a) if a.stop_reason == StopReason::Error))
        .count();
    assert_eq!(errors_in_context, 0);
}

#[tokio::test]
async fn thinking_levels_and_models() {
    let provider = FauxProvider::new(Vec::new());
    let session = TestSession::default().build(provider).await;
    assert_eq!(
        session.set_thinking_level(ThinkingLevel::High, false).await,
        ThinkingLevel::High
    );
    assert_eq!(
        session.set_thinking_level(ThinkingLevel::Max, false).await,
        ThinkingLevel::High
    );
    let changes = session
        .entries()
        .into_iter()
        .filter(|e| matches!(e.kind, EntryKind::ThinkingLevelChange { .. }))
        .count();
    assert_eq!(changes, 2, "initial entry plus one change");
    let mut other = FauxProvider::model();
    other.provider = "nokey".into();
    let error = session.set_model(other, false).await.unwrap_err();
    assert_eq!(error.to_string(), "No API key for nokey/faux-1");
}

#[tokio::test]
async fn manual_compaction_and_tree_navigation() {
    let mut responses = Vec::new();
    for i in 0..4 {
        responses.push(FauxResponse::Message {
            content: vec![AssistantContent::text(format!(
                "answer {i} {}",
                "x".repeat(2000)
            ))],
            stop_reason: StopReason::Stop,
            usage: Usage::default(),
            error_message: None,
        });
    }
    responses.push(FauxResponse::Text("## Goal\nSummary".into()));
    let provider = FauxProvider::new(responses);
    let session = TestSession {
        settings: json!({"compaction": {"keepRecentTokens": 600}})
            .as_object()
            .cloned()
            .unwrap(),
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    for i in 0..4 {
        run(&session, &format!("question {i}")).await;
    }
    let result = session.compact(None).await.unwrap();
    assert!(result.summary.starts_with("## Goal"));
    let messages = session.messages();
    assert!(
        messages
            .iter()
            .any(|m| matches!(m, AgentMessage::CompactionSummary(_)))
    );
    let forks = session.user_messages_for_forking();
    assert_eq!(forks.len(), 4);
    let target = oino_session::EntryId::new(forks[1].entry_id.clone());
    let navigated = session
        .navigate_tree(&target, oino_core::session::NavigateTreeOptions::default())
        .await
        .unwrap();
    assert_eq!(navigated.editor_text.as_deref(), Some("question 1"));
    assert!(!navigated.cancelled);
}
