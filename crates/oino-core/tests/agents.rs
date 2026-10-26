//! Nested agents (runtime §12), the turn arbiter (§7.6) and command-context control ops.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::StreamExt;
use indexmap::IndexMap;
use oino_ai::{AgentMessage, JsonObject, JsonValue};
use oino_core::events::{ClientEvent, OinoEvent, RunState};
use oino_core::extensions::api::{CallContext, CallKind, ExtensionInstance, ExtensionMode};
use oino_core::extensions::host::{HostApi, HostError, HostOp};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::session::{AgentSession, PromptOptions};
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_settings::SettingsManager;
use serde_json::json;

async fn services(
    dir: &std::path::Path,
    provider: Arc<FauxProvider>,
    extra: Vec<Arc<dyn ExtensionInstance>>,
    settings: JsonObject,
) -> Arc<Services> {
    services_with_ui(dir, provider, extra, settings, false).await
}

async fn services_with_ui(
    dir: &std::path::Path,
    provider: Arc<FauxProvider>,
    extra: Vec<Arc<dyn ExtensionInstance>>,
    settings: JsonObject,
    has_ui: bool,
) -> Arc<Services> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let mut instances: Vec<Arc<dyn ExtensionInstance>> = vec![Arc::new(FauxComponent(provider))];
    instances.extend(extra);
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: dir.join("agent"),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Rpc,
        has_ui,
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(instances)),
        approve: Some(false),
        models: Some(models),
        settings: Some(SettingsManager::in_memory(settings)),
    })
    .await
    .unwrap()
}

async fn runtime(services: &Arc<Services>, source: SessionSource) -> AgentSessionRuntime {
    AgentSessionRuntime::create(
        Arc::clone(services),
        SessionOptions {
            source,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap()
}

fn cx(session: &AgentSession, kind: CallKind) -> CallContext {
    session.runner().call_context().with_kind(kind)
}

async fn idle(session: &AgentSession) {
    tokio::time::timeout(Duration::from_secs(10), session.wait_for_idle())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_agent_runs_streams_and_records_usage() {
    let dir = tempfile::tempdir().unwrap();
    let usage = oino_ai::Usage {
        input: 7,
        output: 3,
        ..oino_ai::Usage::default()
    };
    let provider = FauxProvider::new(vec![FauxResponse::Message {
        content: vec![oino_ai::AssistantContent::text("child answer")],
        stop_reason: oino_ai::StopReason::Stop,
        usage,
        error_message: None,
    }]);
    let probe = Arc::new(FakeExtension::new(
        "probe",
        oino_core::extensions::api::Registration {
            events: vec!["agent_start".into()],
            ..Default::default()
        },
    ));
    let services = services(
        dir.path(),
        Arc::clone(&provider),
        vec![Arc::clone(&probe) as Arc<dyn ExtensionInstance>],
        JsonObject::new(),
    )
    .await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let parent = rt.session();
    let spec = json!({
        "label": "helper", "childRole": "subagent", "extensions": "inherit",
        "tools": {"builtin": ["read"]}, "systemPrompt": {"append": "CHILD-ONLY"},
    });
    let child = services
        .host
        .spawn_agent("probe", &cx(&parent, CallKind::Tool), spec)
        .await
        .unwrap();
    assert!(child.session().is_child());
    assert_eq!(child.session().active_tool_names(), vec!["read"]);
    assert!(child.session().system_prompt().contains("CHILD-ONLY"));
    assert!(!parent.system_prompt().contains("CHILD-ONLY"));
    let mut events = child.events();
    assert_eq!(child.prompt("go".into(), None).await.unwrap(), "started");
    child.wait_idle().await;
    assert_eq!(child.last_assistant_text().as_deref(), Some("child answer"));
    let mut kinds = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(2), events.next()).await {
        let kind = event["type"].as_str().unwrap_or_default().to_owned();
        let done = kind == "agent_settled";
        kinds.push(kind);
        if done {
            break;
        }
    }
    for expected in ["agent_start", "message_end", "agent_end", "agent_settled"] {
        assert!(kinds.iter().any(|k| k == expected), "{expected}: {kinds:?}");
    }
    // The inherited extension saw the child's run with a child context.
    let child_cx = probe
        .received_contexts()
        .into_iter()
        .find(|(_, c)| c.session_id == child.id())
        .map(|(_, c)| c)
        .unwrap();
    assert!(child_cx.is_child);
    assert_eq!(child_cx.child_role.as_deref(), Some("subagent"));
    // Usage mirrored into the parent.
    tokio::time::sleep(Duration::from_millis(100)).await;
    let entry = parent
        .entries()
        .into_iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .find(|e| e["type"] == "usage")
        .unwrap();
    assert_eq!(entry["kind"], "child:helper");
    assert_eq!(entry["usage"]["input"], 7);
    assert_eq!(parent.session_stats().tokens.input, 7);
    // agents.list and close.
    let list = services
        .host
        .call("probe", &cx(&parent, CallKind::Event), HostOp::ListAgents)
        .await
        .unwrap();
    assert_eq!(list[0]["label"], "helper");
    assert_eq!(list[0]["state"], "idle");
    child.close(Duration::from_millis(10)).await;
    assert!(child.is_closed());
    assert_eq!(parent.agents_list(), json!([]));
    assert!(child.prompt("again".into(), None).await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn spawn_limits_depth_and_concurrency() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(Vec::new());
    let mut settings = JsonObject::new();
    settings.insert(
        "extensionSettings".into(),
        json!({"subagents": {"maxConcurrent": 2}}),
    );
    let services = services(dir.path(), provider, Vec::new(), settings).await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let parent = rt.session();
    let spawn = |session: &AgentSession, spec: JsonValue| {
        let cx = cx(session, CallKind::Tool);
        let host = Arc::clone(&services.host);
        async move { host.spawn_agent("x", &cx, spec).await }
    };
    let a = spawn(&parent, json!({"label": "a", "maxDepth": 1}))
        .await
        .unwrap();
    let nested = spawn(a.session(), json!({"label": "a1"})).await;
    assert!(
        matches!(&nested, Err(HostError::Failed(m)) if m.contains("depth")),
        "{nested:?}"
    );
    let b = spawn(&parent, json!({"label": "b"})).await.unwrap();
    let b1 = spawn(b.session(), json!({"label": "b1"})).await;
    assert!(
        matches!(&b1, Err(HostError::Failed(m)) if m.contains("Too many")),
        "{b1:?}"
    );
    a.close(Duration::ZERO).await;
    let b1 = spawn(b.session(), json!({"label": "b1"})).await.unwrap();
    // Closing a parent closes its children.
    b.close(Duration::ZERO).await;
    assert!(b1.is_closed());
    let unknown = spawn(&parent, json!({"label": "m", "model": "nope/none"})).await;
    assert!(matches!(unknown, Err(HostError::Invalid(_))));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_sessions_persist_under_the_parent_and_resume() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("parent reply".into()),
        FauxResponse::Text("first".into()),
        FauxResponse::Text("second".into()),
        FauxResponse::Text("aside".into()),
    ]);
    let services = services(
        dir.path(),
        Arc::clone(&provider),
        Vec::new(),
        JsonObject::new(),
    )
    .await;
    let rt = runtime(&services, SessionSource::New).await;
    let parent = rt.session();
    parent
        .prompt("parent question".into(), PromptOptions::default())
        .await
        .unwrap();
    idle(&parent).await;
    let parent_dir = parent
        .session_file()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf();
    let child = services
        .host
        .spawn_agent(
            "x",
            &cx(&parent, CallKind::Tool),
            json!({"label": "p", "persistence": {"file": {}}, "tools": {"builtin": []}}),
        )
        .await
        .unwrap();
    let file = child.session_file().unwrap();
    assert_eq!(file.parent().unwrap(), parent_dir.join("children"));
    child.prompt("task one".into(), None).await.unwrap();
    child.wait_idle().await;
    child.close(Duration::ZERO).await;
    assert!(file.is_file());
    let header: JsonValue = serde_json::from_str(
        std::fs::read_to_string(&file)
            .unwrap()
            .lines()
            .next()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        header["parentSession"],
        json!(parent.session_file().unwrap().display().to_string())
    );
    // Resume the same file and continue the conversation.
    let resumed = services
        .host
        .spawn_agent(
            "x",
            &cx(&parent, CallKind::Tool),
            json!({"label": "p", "persistence": {"file": {}}, "seed": {"sessionFile": file}}),
        )
        .await
        .unwrap();
    assert_eq!(resumed.session_file().unwrap(), file);
    resumed.follow_up("task two".into()).await.unwrap();
    resumed.wait_idle().await;
    let texts: Vec<String> = resumed
        .messages()
        .iter()
        .filter_map(|m| match m {
            AgentMessage::User(u) => Some(u.content.text("")),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["task one", "task two"]);
    resumed.close(Duration::ZERO).await;
    // A branch copy sees the parent's conversation without writing a file.
    let aside = services
        .host
        .spawn_agent(
            "x",
            &cx(&parent, CallKind::Tool),
            json!({"label": "btw", "seed": "branchCopy"}),
        )
        .await
        .unwrap();
    assert!(aside.session_file().is_none());
    aside.prompt("side".into(), None).await.unwrap();
    aside.wait_idle().await;
    let request = provider.requests().last().cloned().unwrap();
    let wire = request["context"]["messages"].to_string();
    assert!(
        wire.contains("parent question") && wire.contains("side"),
        "{wire}"
    );
    aside.close(Duration::ZERO).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn child_dialogs_follow_the_ui_policy() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(Vec::new());
    let services =
        services_with_ui(dir.path(), provider, Vec::new(), JsonObject::new(), true).await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let parent = rt.session();
    let approval = |session: &AgentSession| {
        let mut params = JsonObject::new();
        params.insert("title".into(), json!("Run rm?"));
        params.insert(
            "options".into(),
            json!([{"optionId": "y", "kind": "allow_once"}, {"optionId": "n", "kind": "reject_once"}]),
        );
        (
            cx(session, CallKind::Tool),
            HostOp::UiDialog {
                method: "approval".into(),
                params,
                cancel: None,
            },
        )
    };
    let deny = services
        .host
        .spawn_agent(
            "x",
            &cx(&parent, CallKind::Tool),
            json!({"label": "bg", "ui": {"autoDeny": {"note": "background"}}}),
        )
        .await
        .unwrap();
    assert!(deny.session().runner().call_context().has_ui);
    let (c, op) = approval(deny.session());
    let answer = services.host.call("x", &c, op).await.unwrap();
    assert_eq!(
        answer,
        json!({"value": {"optionId": "n", "feedback": "background"}})
    );
    let silent = services
        .host
        .spawn_agent("x", &cx(&parent, CallKind::Tool), json!({"label": "s"}))
        .await
        .unwrap();
    assert!(!silent.session().runner().call_context().has_ui);
    let (c, op) = approval(silent.session());
    assert_eq!(
        services.host.call("x", &c, op).await.unwrap(),
        json!({"cancelled": true})
    );
    // Forward: the request reaches the parent's UI with the label prefix and is answered there.
    let forward = services
        .host
        .spawn_agent(
            "x",
            &cx(&parent, CallKind::Tool),
            json!({"label": "fg", "ui": "forward"}),
        )
        .await
        .unwrap();
    let mut parent_events = parent.subscribe();
    let (c, op) = approval(forward.session());
    let host = Arc::clone(&services.host);
    let pending = tokio::spawn(async move { host.call("x", &c, op).await });
    let request = loop {
        match tokio::time::timeout(Duration::from_secs(5), parent_events.next())
            .await
            .unwrap()
        {
            Some(ClientEvent::Ui(request)) if request.method == "approval" => break request,
            Some(_) => {}
            None => panic!("parent stream ended"),
        }
    };
    assert_eq!(request.params["title"], "[fg] Run rm?");
    parent.ui().respond(oino_core::ui::UiResponse {
        id: request.id,
        answer: oino_core::ui::UiAnswer::Value(json!({"optionId": "y"})),
    });
    assert_eq!(
        pending.await.unwrap().unwrap(),
        json!({"value": {"optionId": "y"}})
    );
}

fn nudging_extension(count: Arc<AtomicUsize>) -> FakeExtension {
    FakeExtension::new("nudger", oino_core::extensions::api::Registration::default()).on_nudge(
        move |cx| {
            if cx.is_child || count.fetch_add(1, Ordering::SeqCst) > 0 {
                return None;
            }
            Some(json!({"id": "n1", "priority": 10, "customType": "test:continue", "content": "keep going", "display": true}))
        },
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arbiter_nudges_continue_the_run() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first".into()),
        FauxResponse::Text("continued".into()),
    ]);
    let count = Arc::new(AtomicUsize::new(0));
    let nudger = Arc::new(nudging_extension(Arc::clone(&count)));
    let low_count = AtomicUsize::new(0);
    let low = Arc::new(
        FakeExtension::new("low", oino_core::extensions::api::Registration::default()).on_nudge(
            move |_| {
                (low_count.fetch_add(1, Ordering::SeqCst) < 2).then(
                    || json!({"id": "low", "priority": 1, "customType": "test:low", "content": "x"}),
                )
            },
        ),
    );
    let services = services(
        dir.path(),
        Arc::clone(&provider),
        vec![
            Arc::clone(&low) as Arc<dyn ExtensionInstance>,
            Arc::clone(&nudger) as Arc<dyn ExtensionInstance>,
        ],
        JsonObject::new(),
    )
    .await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let session = rt.session();
    session
        .prompt("start".into(), PromptOptions::default())
        .await
        .unwrap();
    idle(&session).await;
    // One nudge per settle: the higher priority wins first, the lower one at the next settle.
    assert_eq!(nudger.delivered(), vec!["n1"]);
    assert_eq!(low.delivered(), vec!["low"]);
    let custom = session
        .messages()
        .into_iter()
        .find_map(|m| match m {
            AgentMessage::Custom(c) if c.custom_type == "test:continue" => Some(c),
            _ => None,
        })
        .unwrap();
    assert_eq!(custom.content.text(""), "keep going");
    assert_eq!(provider.requests().len(), 3);
    assert_eq!(
        provider.requests()[1]["context"]["messages"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|m| m.to_string().contains("keep going"))
            .count(),
        1
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arbiter_wait_sources_hold_the_session_waiting() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("done".into())]);
    let busy = Arc::new(Mutex::new(Some("background task t1".to_owned())));
    let source = {
        let busy = Arc::clone(&busy);
        FakeExtension::new(
            "waiter",
            oino_core::extensions::api::Registration::default(),
        )
        .on_wait(move |_| busy.lock().unwrap().clone())
    };
    let count = Arc::new(AtomicUsize::new(0));
    let services = services(
        dir.path(),
        provider,
        vec![
            Arc::new(source) as Arc<dyn ExtensionInstance>,
            Arc::new(nudging_extension(Arc::clone(&count))),
        ],
        JsonObject::new(),
    )
    .await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let session = rt.session();
    let mut events = session.subscribe();
    session
        .prompt("start".into(), PromptOptions::default())
        .await
        .unwrap();
    idle(&session).await;
    assert_eq!(count.load(Ordering::SeqCst), 0, "no nudge while waiting");
    assert_eq!(session.run_state(), RunState::Waiting);
    assert_eq!(session.waiting_on(), vec!["background task t1"]);
    let state = services
        .host
        .call(
            "waiter",
            &cx(&session, CallKind::Event),
            HostOp::GetRunState,
        )
        .await
        .unwrap();
    assert_eq!(
        state,
        json!({"state": "waiting", "waitingOn": ["background task t1"]})
    );
    *busy.lock().unwrap() = None;
    let mut saw_waiting = false;
    let mut saw_idle = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while !saw_idle && tokio::time::Instant::now() < deadline {
        if let Ok(Some(ClientEvent::Oino(OinoEvent::RunState { state, .. }))) =
            tokio::time::timeout(Duration::from_secs(1), events.next()).await
        {
            saw_waiting |= state == RunState::Waiting;
            saw_idle |= state == RunState::Idle;
        }
    }
    assert!(saw_waiting && saw_idle);
    assert_eq!(session.run_state(), RunState::Idle);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn arbiter_is_disabled_in_children() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("child".into())]);
    let calls = Arc::new(AtomicUsize::new(0));
    let ext = {
        let calls = Arc::clone(&calls);
        FakeExtension::new("n", oino_core::extensions::api::Registration::default()).on_nudge(
            move |_| {
                calls.fetch_add(1, Ordering::SeqCst);
                Some(json!({"id": "x", "priority": 1, "customType": "t", "content": "again"}))
            },
        )
    };
    let services = services(
        dir.path(),
        Arc::clone(&provider),
        vec![Arc::new(ext)],
        JsonObject::new(),
    )
    .await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let child = services
        .host
        .spawn_agent(
            "n",
            &cx(&rt.session(), CallKind::Tool),
            json!({"label": "c", "extensions": "inherit"}),
        )
        .await
        .unwrap();
    child.prompt("go".into(), None).await.unwrap();
    child.wait_idle().await;
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    assert_eq!(provider.requests().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn command_contexts_replace_sessions_and_request_shutdown() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(Vec::new());
    let services = services(dir.path(), provider, Vec::new(), JsonObject::new()).await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let first = rt.session();
    let event_cx = cx(&first, CallKind::Event);
    let denied = services
        .host
        .call(
            "x",
            &event_cx,
            HostOp::NewSession {
                parent_session: None,
            },
        )
        .await;
    assert_eq!(denied, Err(HostError::CommandContextRequired));
    let options = services
        .host
        .call(
            "x",
            &cx(&first, CallKind::Command),
            HostOp::GetSystemPromptOptions,
        )
        .await
        .unwrap();
    assert!(options.get("selectedTools").is_some(), "{options}");
    let replaced = services
        .host
        .call(
            "x",
            &cx(&first, CallKind::Command),
            HostOp::NewSession {
                parent_session: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(replaced, json!({"cancelled": false}));
    assert_ne!(rt.session().session_id(), first.session_id());
    let stale = services.host.call("x", &event_cx, HostOp::IsIdle).await;
    assert!(stale.is_err(), "{stale:?}");
    let reloaded = services
        .host
        .call("x", &cx(&rt.session(), CallKind::Command), HostOp::Reload)
        .await;
    assert_eq!(reloaded, Ok(JsonValue::Null));
    let shutdown = rt.shutdown_requested();
    assert!(!*shutdown.borrow());
    services
        .host
        .call("x", &cx(&rt.session(), CallKind::Event), HostOp::Shutdown)
        .await
        .unwrap();
    assert!(*shutdown.borrow());
}
