//! `AgentSession` behaviour against the faux provider and fake extensions.

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
    assert_eq!(roles, vec!["user", "assistant"]);
    let request = &provider.requests()[0];
    assert_eq!(request["context"]["messages"][0]["role"], json!("system"));
    assert!(
        request["context"]["messages"][0]["content"]
            .as_str()
            .unwrap()
            .contains("Oino")
    );
    assert!(request["context"]["messages"][0]["sections"].is_null());
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
        tools
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["description"].as_str().unwrap().contains("greet tool"))
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
    assert!(texts[1].contains("hi!"), "{texts:?}");
    assert!(texts[2].contains("injected"), "{texts:?}");
    let echo = session.entries().into_iter().any(|e| {
        matches!(&e.kind, EntryKind::Custom { custom_type, .. } if custom_type == "oino.command-echo")
    });
    assert!(echo);
    let commands = session.commands();
    assert_eq!(commands[0].name, "hello");
}

/// G3-2 (beqa): a host-admitted command's `oino.command-echo` carries the request's `messageId`, so a frontend pairs
/// its pending bubble with the echo by identity; a command run outside a host admission carries none (additive field).
#[tokio::test]
async fn command_echo_carries_the_originating_message_id() {
    let provider = FauxProvider::new(vec![]);
    let extension = FakeExtension::new(
        "ext",
        Registration {
            commands: vec![CommandSpec {
                name: "hello".into(),
                description: None,
                hidden: false,
                completions: false,
            }],
            ..Registration::default()
        },
    )
    .on_command(|_, _| Ok(None));
    let session = TestSession {
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    let id = "019b0000-0000-7000-8000-00000000000a";
    let disposition = oino_core::session::delivery::MESSAGE_ID
        .scope(
            id.into(),
            session.prompt("/hello one".into(), PromptOptions::default()),
        )
        .await
        .unwrap();
    assert_eq!(disposition, PromptDisposition::Handled);
    assert_eq!(
        run(&session, "/hello two").await,
        PromptDisposition::Handled
    );
    let echoes: Vec<JsonValue> = session
        .entries()
        .into_iter()
        .filter_map(|e| match e.kind {
            EntryKind::Custom {
                custom_type, data, ..
            } if custom_type == "oino.command-echo" => data,
            _ => None,
        })
        .collect();
    assert_eq!(echoes.len(), 2, "{echoes:?}");
    assert_eq!(
        echoes[0],
        json!({"name": "hello", "args": "one", "messageId": id})
    );
    assert_eq!(echoes[1], json!({"name": "hello", "args": "two"}));
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

/// E-19: `set_model(.., persist: false)` switches this session only — the PC default
/// (`defaultProvider`/`defaultModel`) and the cycle scope stay as they were; `persist:
/// true` keeps saving both (the TUI/CLI behaviour).
#[tokio::test]
async fn a_session_scoped_model_pick_does_not_persist_the_default() {
    let provider = FauxProvider::new(Vec::new());
    let session = TestSession::default().build(provider).await;
    let model = FauxProvider::model();

    session.set_model(model.clone(), false).await.unwrap();
    let snapshot = session.with_settings(|s| s.snapshot());
    assert_eq!(
        snapshot.default_provider(),
        None,
        "per-chat pick saves no default"
    );
    assert_eq!(snapshot.default_model(), None);
    assert_eq!(snapshot.enabled_models(), None, "no cycle-scope growth");
    // The session itself runs the new model.
    assert_eq!(session.model().map(|m| m.id), Some(model.id.clone()));

    session.set_model(model.clone(), true).await.unwrap();
    let snapshot = session.with_settings(|s| s.snapshot());
    assert_eq!(snapshot.default_provider(), Some("faux"));
    assert_eq!(snapshot.default_model(), Some("faux-1"));
}

/// `control.set-default-model` writes the fresh-session model into core settings without
/// touching the running session (the v1 port, R4).
#[tokio::test]
async fn set_default_model_persists_core_settings() {
    let provider = FauxProvider::new(Vec::new());
    let session = TestSession::default().build(provider).await;
    let cx = oino_core::extensions::api::CallContext {
        session_id: session.session_id().clone(),
        mode: oino_core::extensions::api::ExtensionMode::Print,
        has_ui: false,
        cwd: session.cwd().to_path_buf(),
        is_child: false,
        child_role: None,
        call_kind: oino_core::extensions::api::CallKind::Event,
    };
    let model = FauxProvider::model();
    let written = session
        .host_call(
            "acme.ext",
            &cx,
            oino_core::extensions::host::HostOp::SetDefaultModel {
                provider: model.provider.clone(),
                id: model.id.clone(),
                thinking: Some(ThinkingLevel::High),
            },
        )
        .await
        .unwrap();
    assert_eq!(written, json!(true));
    let (provider_id, model_id, thinking) = session.with_settings(|s| {
        let view = s.settings();
        (
            view.default_provider().map(ToOwned::to_owned),
            view.default_model().map(ToOwned::to_owned),
            view.model_thinking_level(&model.provider, &model.id),
        )
    });
    assert_eq!(provider_id.as_deref(), Some(model.provider.as_str()));
    assert_eq!(model_id.as_deref(), Some(model.id.as_str()));
    assert_eq!(thinking, Some(ThinkingLevel::High));
    // Unknown models are refused, like `set-model`.
    let refused = session
        .host_call(
            "acme.ext",
            &cx,
            oino_core::extensions::host::HostOp::SetDefaultModel {
                provider: "nokey".into(),
                id: "none".into(),
                thinking: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(refused, json!(false));
}

/// Render-only overrides for the built-in tools (`tools.override`, U1–U2): a spec for `read`
/// needs the permission, and `_oino/render` reaches the owner's `render` with the row data and
/// the frontend's density/width hints.
#[tokio::test]
async fn builtin_tool_render_overrides() {
    use oino_core::extensions::api::CallKind;
    use oino_core::extensions::host::HostOp;
    use oino_core::session::{RenderDensity, RenderRequest, RenderWidth};

    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = Arc::clone(&seen);
    let extension = Arc::new(
        FakeExtension::new("acme.render", Registration::default()).on_render(
            &["read", "bash"],
            move |request: JsonValue| {
                recorder.lock().unwrap().push(request);
                Some(json!({"t": "text", "value": "custom read row"}))
            },
        ),
    );
    let plain = Arc::new(FakeExtension::new("acme.plain", Registration::default()));
    let session = TestSession {
        extensions: vec![extension, plain],
        ..TestSession::default()
    }
    .build(FauxProvider::new(Vec::new()))
    .await;
    let cx = oino_core::extensions::api::CallContext {
        session_id: session.session_id().clone(),
        mode: oino_core::extensions::api::ExtensionMode::Print,
        has_ui: false,
        cwd: session.cwd().to_path_buf(),
        is_child: false,
        child_role: None,
        call_kind: CallKind::Event,
    };
    let spec = json!({"target": "tool", "key": "read"});
    assert!(
        session
            .host_call("acme.plain", &cx, HostOp::RegisterRenderer(spec.clone()))
            .await
            .is_err(),
        "tools.override is required for built-in tool rows"
    );
    session
        .host_call("acme.render", &cx, HostOp::RegisterRenderer(spec))
        .await
        .unwrap();
    // Extension tools and custom messages need no override.
    session
        .host_call(
            "acme.plain",
            &cx,
            HostOp::RegisterRenderer(json!({"target": "tool", "key": "sidekick"})),
        )
        .await
        .unwrap();
    let mut request = RenderRequest::tool(
        "read",
        Some("call-1".into()),
        json!({"args": {"path": "a.txt"}}),
    );
    request.density = RenderDensity::Advanced;
    request.expanded = true;
    request.width = RenderWidth::Wide;
    let node = session.render_spec(&request).unwrap().expect("render node");
    assert_eq!(node["value"], json!("custom read row"));
    let request = seen.lock().unwrap()[0].clone();
    assert_eq!(request["target"], json!("tool"));
    assert_eq!(request["key"], json!("read"));
    assert_eq!(request["entryId"], json!("call-1"));
    assert_eq!(request["density"], json!("advanced"));
    assert_eq!(request["width"], json!("wide"));
    assert_eq!(request["expanded"], json!(true));
    assert_eq!(request["data"]["args"]["path"], json!("a.txt"));
    // No renderer for this key: the frontend renders it itself.
    assert!(
        session
            .render_spec(&RenderRequest::tool("write", None, json!({})))
            .unwrap()
            .is_none()
    );
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
    assert!(messages.iter().any(
        |m| matches!(m, AgentMessage::User(u) if u.content.text("").contains(&result.summary))
    ));
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

struct Outer {
    meta: oino_agent::ToolDefinitionMeta,
    session: Arc<std::sync::OnceLock<AgentSession>>,
}

impl oino_agent::AgentTool for Outer {
    fn meta(&self) -> &oino_agent::ToolDefinitionMeta {
        &self.meta
    }

    fn execute(
        &self,
        call: oino_agent::ToolInvocation,
        _updates: oino_agent::UpdateSink,
        cancel: tokio_util::sync::CancellationToken,
    ) -> futures::future::BoxFuture<'static, Result<AgentToolResult, oino_agent::ToolError>> {
        let session = self.session.get().cloned().unwrap();
        Box::pin(async move {
            let inner = session
                .execute_nested_tool(&call.tool_call_id, "inner", args(json!({"x": 1})), cancel)
                .await;
            Ok(AgentToolResult::text(format!(
                "outer saw {}",
                serde_json::to_string(&inner.result.content).unwrap()
            )))
        })
    }
}

#[tokio::test]
async fn nested_tool_calls_are_recorded() {
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![("c1".into(), "outer".into(), args(json!({})))]),
        FauxResponse::Text("done".into()),
    ]);
    let handle = Arc::new(std::sync::OnceLock::new());
    let schema = json!({"type": "object"});
    let outer = Arc::new(Outer {
        meta: oino_agent::ToolDefinitionMeta::new("outer", "outer", "calls inner", schema.clone()),
        session: Arc::clone(&handle),
    });
    let inner_extension = FakeExtension::new(
        "ext",
        Registration {
            tools: vec![ToolSpec {
                name: "inner".into(),
                label: "inner".into(),
                description: "inner".into(),
                parameters: schema,
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
    .on_tool(|_, args| Ok(AgentToolResult::text(format!("inner got {}", args["x"]))));
    let session = TestSession {
        extensions: vec![Arc::new(inner_extension)],
        tools: vec![oino_core::session::ToolEntry::builtin(
            outer,
            None,
            Vec::new(),
        )],
        ..TestSession::default()
    }
    .build(provider)
    .await;
    handle.set(session.clone()).ok();
    let events = session.subscribe();
    run(&session, "go").await;
    let result = session
        .messages()
        .into_iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) => Some(r),
            _ => None,
        })
        .unwrap();
    let nested = result.nested_calls.unwrap();
    assert_eq!(nested.calls.len(), 1);
    assert_eq!(nested.calls[0].id, "c1/1");
    assert!(nested.complete);
    let parents: Vec<_> = events
        .drain_now()
        .into_iter()
        .filter_map(|e| match e {
            ClientEvent::Session(AgentSessionEvent::ToolExecutionEnd {
                tool_call_id,
                parent_tool_call_id,
                ..
            }) => Some((tool_call_id, parent_tool_call_id)),
            _ => None,
        })
        .collect();
    assert_eq!(
        parents,
        vec![
            ("c1/1".to_owned(), Some("c1".to_owned())),
            ("c1".to_owned(), None)
        ]
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_settles_with_aborted_and_holds_queued_steering() {
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![("c1".into(), "slow".into(), args(json!({})))]),
        FauxResponse::Text("never".into()),
        FauxResponse::Text("resumed".into()),
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
        std::thread::sleep(Duration::from_millis(100));
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
        .prompt("go".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(20)).await;
    session
        .steer(
            "queued".into(),
            Vec::new(),
            oino_core::session::InputSource::Interactive,
        )
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), session.abort())
        .await
        .unwrap();
    assert!(session.is_idle());
    let settled: Vec<bool> = events
        .drain_now()
        .into_iter()
        .filter_map(|e| match e {
            ClientEvent::Session(AgentSessionEvent::AgentSettled { aborted }) => Some(aborted),
            _ => None,
        })
        .collect();
    assert_eq!(settled, vec![true]);
    // OIN-75: aborting a run before a steer point must NOT auto-send the queued steer;
    // it remains queued (held) so the owner can send, edit, or discard it.
    assert_eq!(session.steering_messages(), vec!["queued".to_owned()]);
    assert!(
        !session
            .messages()
            .iter()
            .any(|m| matches!(m, AgentMessage::User(u) if u.content.text("") == "queued")),
        "queued steer must not be sent into the transcript on abort"
    );
    let last = session.last_assistant_message().unwrap();
    assert_eq!(last.stop_reason, StopReason::Aborted);

    // Sending the held steer explicitly delivers it as a normal turn.
    session
        .edit_queue(oino_core::session::QueueEdit::SendNow {
            queue: oino_core::session::QueueKind::Steering,
            index: 0,
        })
        .await
        .unwrap();
    session.wait_for_idle().await;
    assert_eq!(session.steering_messages(), Vec::<String>::new());
    assert!(
        session
            .messages()
            .iter()
            .any(|m| matches!(m, AgentMessage::User(u) if u.content.text("") == "queued"))
    );
    assert_eq!(session.last_assistant_text().as_deref(), Some("resumed"));
}

/// E-04: Stop must always end a turn. A dialog the user never answers (the approval
/// card whose "Allow once" never reached the tool) must not hold the run: abort
/// cancels pending dialogs, answers them `Cancelled`, and returns even when a run is
/// wedged (the 15 s force-idle bound).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn abort_cancels_pending_dialogs_and_always_ends_the_turn() {
    let provider = FauxProvider::new(Vec::new());
    let session = TestSession {
        has_ui: true,
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;

    // The wedged approval: a tool/handler waiting for an answer that never comes
    // (no timeout, no cancel token — exactly the stuck card).
    let bridge = session.ui();
    let dialog = tokio::spawn(async move {
        bridge
            .dialog(
                None,
                "approval",
                serde_json::from_value(json!({"title": "Allow bg_run once?"})).unwrap(),
                None,
            )
            .await
    });
    tokio::time::sleep(Duration::from_millis(50)).await;

    let started = std::time::Instant::now();
    session.abort().await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "abort must not wait on an unanswered dialog"
    );
    assert_eq!(
        tokio::time::timeout(Duration::from_secs(5), dialog)
            .await
            .unwrap()
            .unwrap(),
        oino_core::ui::UiAnswer::Cancelled,
        "the pending dialog is cancelled"
    );
}

/// E-04's hard bound: a run whose provider stream never settles cannot keep the turn
/// on "Working…" — abort forces the run idle after the bound. Virtual time: the 15 s
/// bound auto-advances on the paused clock while real time stays tiny.
#[tokio::test(flavor = "current_thread", start_paused = true)]
async fn abort_forces_a_wedged_run_idle_within_the_bound() {
    let provider = FauxProvider::new(vec![FauxResponse::Never]);
    let session = TestSession::default().build(Arc::clone(&provider)).await;
    session
        .prompt("hi".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(!session.is_idle(), "the run is wedged (sanity)");

    let started = std::time::Instant::now();
    session.abort().await;
    // Virtual time: the 15 s bound auto-advances; real time stays tiny.
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "abort returned (bounded wait)"
    );
    assert!(session.is_idle(), "the wedged run was forced idle");
}

/// A command whose body sends a user message through the host while the command runs
/// (long-horizon's `/swarm <task>` → `send_user_message`).
struct ForwardingCommand {
    session: std::sync::OnceLock<AgentSession>,
}

impl oino_core::extensions::api::ExtensionInstance for ForwardingCommand {
    fn id(&self) -> &'static str {
        "fwd"
    }

    fn init(
        &self,
        _cx: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<Registration, oino_core::extensions::api::ExtensionCallError>,
    > {
        Box::pin(async {
            Ok(Registration {
                commands: vec![CommandSpec {
                    name: "task".into(),
                    description: None,
                    hidden: false,
                    completions: false,
                }],
                ..Registration::default()
            })
        })
    }

    fn on_event(
        &self,
        _kind: &str,
        _payload: JsonValue,
        _cx: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<Option<JsonValue>, oino_core::extensions::api::ExtensionCallError>,
    > {
        Box::pin(async { Ok(None) })
    }

    fn run_tool(
        &self,
        tool: &str,
        _invocation: oino_agent::ToolInvocation,
        _updates: oino_agent::UpdateSink,
        _cancel: tokio_util::sync::CancellationToken,
        _cx: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<AgentToolResult, oino_core::extensions::api::ExtensionCallError>,
    > {
        let tool = tool.to_owned();
        Box::pin(async move {
            Err(oino_core::extensions::api::ExtensionCallError::new(
                format!("unknown tool {tool}"),
            ))
        })
    }

    fn run_command(
        &self,
        _name: &str,
        args: &str,
        _cancel: tokio_util::sync::CancellationToken,
        cx: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<Option<JsonValue>, oino_core::extensions::api::ExtensionCallError>,
    > {
        let args = args.to_owned();
        Box::pin(async move {
            let session = self.session.get().expect("session bound");
            session
                .host_call(
                    "fwd",
                    &cx,
                    oino_core::extensions::host::HostOp::SendUserMessage {
                        content: oino_ai::UserContentField::Text(args),
                        deliver_as: None,
                    },
                )
                .await
                .map_err(|e| oino_core::extensions::api::ExtensionCallError::new(e.to_string()))?;
            Ok(None)
        })
    }
}

#[tokio::test]
async fn command_prompt_images_reach_the_message_the_command_sends() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    let extension = Arc::new(ForwardingCommand {
        session: std::sync::OnceLock::new(),
    });
    let session = TestSession {
        extensions: vec![
            Arc::clone(&extension) as Arc<dyn oino_core::extensions::api::ExtensionInstance>
        ],
        ..TestSession::default()
    }
    .build(Arc::clone(&provider))
    .await;
    extension.session.set(session.clone()).ok().unwrap();
    let image = oino_ai::ImageContent {
        data: "aGVsbG8=".into(),
        mime_type: "image/png".into(),
    };
    let disposition = session
        .prompt(
            "/task fix [image 1]".into(),
            PromptOptions {
                images: vec![image],
                ..PromptOptions::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(disposition, PromptDisposition::Handled);
    for _ in 0..100 {
        if !provider.requests().is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
    let requests = provider.requests();
    assert_eq!(requests.len(), 1, "the command's message ran once");
    let user = requests[0]["context"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .rev()
        .find(|m| m["role"] == "user")
        .cloned()
        .unwrap();
    let content = user["content"].to_string();
    assert!(content.contains("fix [image 1]"), "{content}");
    assert!(
        content.contains("aGVsbG8="),
        "the image rode along: {content}"
    );
}

#[tokio::test]
async fn message_origin_is_persisted_for_extensions_only() {
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("one".into()),
        FauxResponse::Text("two".into()),
    ]);
    let session = TestSession::default().build(Arc::clone(&provider)).await;
    run(&session, "human").await;
    session
        .send_user_message("extension prompt".into(), None, false)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
    let users: Vec<_> = session
        .entries()
        .iter()
        .filter_map(|entry| match entry.message() {
            Some(AgentMessage::User(user)) => Some(serde_json::to_value(user).unwrap()),
            _ => None,
        })
        .collect();
    assert!(users[0].get("author").is_none());
    assert_eq!(users[1]["author"], "extension");
    assert!(
        provider.requests()[1]["context"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .all(|m| m.get("author").is_none())
    );
}

#[tokio::test]
async fn last_assistant_snapshot_ignores_trailing_non_assistant_messages() {
    let provider = FauxProvider::new(vec![FauxResponse::Text("answer".into())]);
    let session = TestSession::default().build(provider).await;
    assert!(session.last_assistant_message().is_none());
    run(&session, "question").await;
    let expected = session.last_assistant_message().unwrap();
    session.with_manager(|manager| {
        manager
            .append_message(AgentMessage::User(oino_ai::UserMessage {
                author: None,
                content: oino_ai::UserContentField::Text("trailing context".repeat(1000)),
                timestamp: 0,
            }))
            .unwrap();
    });
    session.refresh_context();
    assert_eq!(session.last_assistant_message(), Some(expected));
}

/// Never must keep the stream open after Start, rather than ending at EOF.
#[tokio::test]
async fn faux_never_stream_stays_pending_after_start() {
    use futures::StreamExt;
    use oino_core::extensions::api::ProviderInstance;
    let provider = FauxProvider::new(vec![FauxResponse::Never]);
    let mut stream = provider
        .stream(
            json!({"model": FauxProvider::model()}),
            tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap();
    assert!(matches!(
        stream.next().await,
        Some(Ok(oino_ai::ProviderEvent::Start {}))
    ));
    assert!(
        tokio::time::timeout(Duration::from_millis(50), stream.next())
            .await
            .is_err()
    );
}

#[tokio::test]
async fn refused_extension_command_does_not_consume_text_or_images() {
    let provider = FauxProvider::new(vec![]);
    let extension = FakeExtension::new(
        "refusal",
        Registration {
            commands: vec![CommandSpec {
                name: "swarm".into(),
                description: None,
                hidden: false,
                completions: false,
            }],
            ..Registration::default()
        },
    )
    .on_command(|_, _| {
        Ok(Some(
            json!({"disposition":"refused", "reason":"live graph"}),
        ))
    });
    let session = TestSession {
        extensions: vec![Arc::new(extension)],
        ..Default::default()
    }
    .build(Arc::clone(&provider))
    .await;
    let images = vec![
        oino_ai::ImageContent {
            data: "aW1hZ2Ux".into(),
            mime_type: "image/png".into(),
        },
        oino_ai::ImageContent {
            data: "aW1hZ2Uy".into(),
            mime_type: "image/png".into(),
        },
    ];
    let result = session
        .prompt(
            "/swarm keep this".into(),
            PromptOptions {
                images,
                ..Default::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(result, PromptDisposition::Refused("live graph".into()));
    assert_eq!(provider.requests(), [] as [serde_json::Value; 0]);
    assert!(!session.entries().iter().any(|e| matches!(&e.kind,
        EntryKind::Message { message } if matches!(message, AgentMessage::User(_)))));
}

struct SlowModelObserver {
    wake: bool,
    called: std::sync::atomic::AtomicBool,
}
impl oino_core::extensions::api::ExtensionInstance for SlowModelObserver {
    fn id(&self) -> &'static str {
        "slow-model-observer"
    }
    fn init(
        &self,
        _: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<Registration, oino_core::extensions::api::ExtensionCallError>,
    > {
        Box::pin(async {
            Ok(Registration {
                events: vec!["model_select".into()],
                ..Registration::default()
            })
        })
    }
    fn prepare_event(
        &self,
        _: &str,
    ) -> futures::future::BoxFuture<'_, Result<(), oino_core::extensions::api::ExtensionCallError>>
    {
        Box::pin(async move {
            if self.wake {
                tokio::time::sleep(Duration::from_secs(6)).await;
            }
            Ok(())
        })
    }
    fn on_event(
        &self,
        _: &str,
        _: JsonValue,
        _: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<Option<JsonValue>, oino_core::extensions::api::ExtensionCallError>,
    > {
        Box::pin(async move {
            self.called.store(true, std::sync::atomic::Ordering::SeqCst);
            if self.wake {
                Ok(None)
            } else {
                std::future::pending().await
            }
        })
    }
    fn run_tool(
        &self,
        _: &str,
        _: oino_agent::ToolInvocation,
        _: oino_agent::UpdateSink,
        _: oino_extension_api::CancellationToken,
        _: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<AgentToolResult, oino_core::extensions::api::ExtensionCallError>,
    > {
        Box::pin(async { Ok(AgentToolResult::text("unused")) })
    }
    fn run_command(
        &self,
        _: &str,
        _: &str,
        _: oino_extension_api::CancellationToken,
        _: oino_core::extensions::api::CallContext,
    ) -> futures::future::BoxFuture<
        '_,
        Result<Option<JsonValue>, oino_core::extensions::api::ExtensionCallError>,
    > {
        Box::pin(async { Ok(None) })
    }
}

#[tokio::test(start_paused = true)]
async fn slow_model_select_observer_cannot_fail_selection_or_show_a_toast() {
    let session = TestSession {
        extensions: vec![Arc::new(SlowModelObserver {
            wake: false,
            called: std::sync::atomic::AtomicBool::new(false),
        })],
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![]))
    .await;
    let events = session.subscribe();
    let mut next = FauxProvider::model();
    next.id = "new-model".into();
    assert_eq!(
        session.set_model(next.clone(), false).await.unwrap().id,
        next.id
    );
    assert_eq!(session.model().unwrap().id, next.id);
    assert!(session.entries().iter().any(|entry| matches!(&entry.kind, EntryKind::ModelChange { model_id, .. } if model_id == &next.id)));
    let all = kinds(&events.drain_now());
    assert!(all.iter().any(|kind| kind == "model_changed"));
    assert!(!all.iter().any(|kind| kind == "extension_error"), "{all:?}");
}

#[tokio::test(start_paused = true)]
async fn model_select_wake_does_not_consume_core_handler_deadline() {
    let observer = Arc::new(SlowModelObserver {
        wake: true,
        called: std::sync::atomic::AtomicBool::new(false),
    });
    let session = TestSession {
        extensions: vec![observer.clone()],
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![]))
    .await;
    let events = session.subscribe();
    let mut next = FauxProvider::model();
    next.id = "after-wake".into();
    session.set_model(next.clone(), false).await.unwrap();
    assert_eq!(session.model().unwrap().id, next.id);
    assert!(
        observer.called.load(std::sync::atomic::Ordering::SeqCst),
        "wake finished and handler ran"
    );
    assert!(
        !kinds(&events.drain_now())
            .iter()
            .any(|kind| kind == "extension_error")
    );
}

/// An extension's synchronous `set_model` import must not re-enter its busy actor.
/// Other observers still run, and direct user selections still notify everyone.
#[tokio::test(start_paused = true)]
async fn self_originated_model_select_skips_busy_actor_but_not_other_observers() {
    let observer = Arc::new(SlowModelObserver {
        wake: false,
        called: std::sync::atomic::AtomicBool::new(false),
    });
    let session = TestSession {
        extensions: vec![observer.clone()],
        ..Default::default()
    }
    .build(FauxProvider::new(vec![]))
    .await;
    let mut next = FauxProvider::model();
    next.id = "previous".into();
    session.set_model(next, false).await.unwrap();
    observer
        .called
        .store(false, std::sync::atomic::Ordering::SeqCst);
    let cx = oino_core::extensions::api::CallContext {
        session_id: session.session_id().clone(),
        mode: oino_core::extensions::api::ExtensionMode::Print,
        has_ui: false,
        cwd: session.cwd().to_path_buf(),
        is_child: false,
        child_role: None,
        call_kind: oino_core::extensions::api::CallKind::Event,
    };
    let model = FauxProvider::model();
    let start = tokio::time::Instant::now();
    let reply = session
        .host_call(
            "slow-model-observer",
            &cx,
            oino_core::extensions::host::HostOp::SetModel {
                provider: model.provider.clone(),
                id: model.id.clone(),
            },
        )
        .await
        .unwrap();
    assert_eq!(reply, json!(true));
    assert_eq!(session.model().unwrap().id, model.id);
    assert!(!observer.called.load(std::sync::atomic::Ordering::SeqCst));
    assert!(
        start.elapsed() < Duration::from_secs(1),
        "no five-second recursive wait"
    );

    let mut other = model.clone();
    other.id = "another".into();
    session.set_model(other, false).await.unwrap();
    assert!(
        observer.called.load(std::sync::atomic::Ordering::SeqCst),
        "user switch still notifies"
    );
}

/// A denied/failed slash command must leave more than its input echo behind.
#[tokio::test]
async fn denied_command_has_a_persistent_outcome_and_next_command_is_not_lost() {
    let extension = FakeExtension::new(
        "ext",
        Registration {
            commands: vec![CommandSpec {
                name: "denied".into(),
                description: None,
                hidden: false,
                completions: false,
            }],
            ..Registration::default()
        },
    )
    .on_command(|_, _| Err("Process approval denied".into()));
    let session = TestSession {
        extensions: vec![Arc::new(extension)],
        ..TestSession::default()
    }
    .build(FauxProvider::new(vec![]))
    .await;
    for args in ["first", "follow-up"] {
        session
            .prompt(format!("/denied {args}"), PromptOptions::default())
            .await
            .unwrap();
    }
    let entries = session.with_manager(|m| m.entries().to_vec());
    let failures: Vec<_> = entries
        .iter()
        .filter(|e| {
            let v = serde_json::to_value(e).unwrap();
            v["type"] == "custom_message"
                && v["customType"] == "oino.command-error"
                && v["display"] == true
                && v["content"]
                    .as_str()
                    .is_some_and(|s| s.contains("Process approval denied"))
        })
        .collect();
    assert_eq!(
        failures.len(),
        2,
        "each command has its own durable failure"
    );
}
