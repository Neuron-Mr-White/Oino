//! GUI feeds and controls (gui-audit area E): topics, nested-agent feed, single-item queue
//! edits, file checkpoints, bash detach, `disabledExtensions`, shortcuts, frontends and
//! shutdown.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use indexmap::IndexMap;
use oino_ai::{AgentMessage, JsonObject};
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::{
    CallContext, CallKind, ExtensionInstance, ExtensionMode, Registration, ShortcutSpec,
};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::hub::EventStream;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::session::{AgentSession, InputSource, QueueEdit, QueueKind};
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
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let mut instances: Vec<Arc<dyn ExtensionInstance>> = vec![Arc::new(FauxComponent(provider))];
    instances.extend(extra);
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: dir.join("agent"),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Desktop,
        has_ui: true,
        flags: ResourceFlags {
            no_context_files: true,
            no_skills: true,
            no_prompt_templates: true,
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

/// A runtime with the built-in tools bound to the session (as the desktop does).
async fn runtime(services: &Arc<Services>, source: SessionSource) -> AgentSessionRuntime {
    let context = Arc::new(SessionToolContext::default());
    let settings = services.settings.lock().unwrap().snapshot();
    let builtin_tools = builtin_tool_entries(
        &services.cwd,
        &services.agent_dir,
        &settings,
        Arc::clone(&context),
    );
    let rt = AgentSessionRuntime::create(
        Arc::clone(services),
        SessionOptions {
            source,
            builtin_tools,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    context.bind(&rt.session());
    rt
}

fn cx(session: &AgentSession, kind: CallKind) -> CallContext {
    session.runner().call_context().with_kind(kind)
}

async fn idle(session: &AgentSession) {
    tokio::time::timeout(Duration::from_secs(20), session.wait_for_idle())
        .await
        .unwrap();
}

/// The next Oino event matching `pick`.
async fn next_oino<T>(events: &mut EventStream, pick: impl Fn(&OinoEvent) -> Option<T>) -> T {
    loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .expect("timed out waiting for an event")
            .expect("stream ended");
        if let ClientEvent::Oino(event) = &event
            && let Some(found) = pick(event)
        {
            return found;
        }
    }
}

fn args(value: serde_json::Value) -> JsonObject {
    serde_json::from_value(value).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn topics_feed_snapshots_and_streams_matching_publishes() {
    let dir = tempfile::tempdir().unwrap();
    let services = services(
        dir.path(),
        FauxProvider::new(Vec::new()),
        Vec::new(),
        JsonObject::new(),
    )
    .await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let session = rt.session();
    let publish = |topic: &str, data: serde_json::Value, sticky: bool| {
        let host = Arc::clone(&services.host);
        let cx = cx(&session, CallKind::Event);
        let topic = topic.to_owned();
        async move {
            host.call(
                "unipi.workflow",
                &cx,
                HostOp::BusPublish {
                    topic,
                    data,
                    sticky,
                },
            )
            .await
            .unwrap();
        }
    };
    // Published before anyone subscribed: kept as sticky state.
    publish("unipi:workflow:status", json!({"planMode": true}), true).await;
    publish("unipi:bg:tasks", json!({"tasks": []}), true).await;
    let snapshot = session
        .subscribe_topics(&["unipi:workflow:*".into()], &[])
        .await;
    assert_eq!(snapshot.len(), 1);
    assert_eq!(snapshot[0].topic, "unipi:workflow:status");
    assert_eq!(snapshot[0].publisher, "unipi.workflow");
    assert_eq!(snapshot[0].data, json!({"planMode": true}));
    let mut events = session.subscribe();
    publish("unipi:bg:tasks", json!({"tasks": [1]}), true).await;
    publish("unipi:workflow:status", json!({"planMode": false}), true).await;
    // Unchanged sticky values do not repeat.
    publish("unipi:workflow:status", json!({"planMode": false}), true).await;
    publish("unipi:workflow:ping", json!(1), false).await;
    let first = next_oino(&mut events, |e| match e {
        OinoEvent::Topic { topic, data, .. } => Some((topic.clone(), data.clone())),
        _ => None,
    })
    .await;
    assert_eq!(
        first,
        ("unipi:workflow:status".into(), json!({"planMode": false}))
    );
    let second = next_oino(&mut events, |e| match e {
        OinoEvent::Topic { topic, sticky, .. } => Some((topic.clone(), *sticky)),
        _ => None,
    })
    .await;
    assert_eq!(second, ("unipi:workflow:ping".into(), false));
    // bus.get for hosts without their own bus and `_oino/bus_get`.
    let value = services
        .host
        .call(
            "x",
            &cx(&session, CallKind::Event),
            HostOp::BusGet("unipi:bg:tasks".into()),
        )
        .await
        .unwrap();
    assert_eq!(value, json!({"tasks": [1]}));
    assert_eq!(
        session.bus_get("unipi:bg:tasks").unwrap().data,
        json!({"tasks": [1]})
    );
    // Unsubscribing stops the feed; the git topic is always a valid shape.
    session
        .subscribe_topics(&[], &["unipi:workflow:*".into()])
        .await;
    assert_eq!(session.topic_patterns().len(), 0);
    let git = session.subscribe_topics(&["oino:git".into()], &[]).await;
    assert_eq!(git.len(), 1);
    assert_eq!(git[0].publisher, "oino.core");
    // Subscriptions follow session replacement.
    rt.new_session(None).await.unwrap();
    assert_eq!(rt.session().topic_patterns(), vec!["oino:git".to_owned()]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agents_feed_reports_tree_changes_and_forwards_child_events() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("child says hi".into())]);
    let services = services(dir.path(), provider, Vec::new(), JsonObject::new()).await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let parent = rt.session();
    let mut events = parent.subscribe();
    let child = services
        .host
        .spawn_agent(
            "unipi.subagents",
            &cx(&parent, CallKind::Tool),
            json!({"label": "Subagent scout", "childRole": "subagent"}),
        )
        .await
        .unwrap();
    let agents = next_oino(&mut events, |e| match e {
        OinoEvent::AgentsChanged { agents } => Some(agents.clone()),
        _ => None,
    })
    .await;
    assert_eq!(agents.len(), 1);
    assert_eq!(agents[0].label, "Subagent scout");
    assert_eq!(agents[0].owner, "unipi.subagents");
    assert_eq!(agents[0].role.as_deref(), Some("subagent"));
    assert_eq!(agents[0].depth, 1);
    assert_eq!(parent.agent_tree(), agents);
    assert!(parent.find_agent(child.id()).is_some());
    parent.follow_agents(true);
    child.prompt("go".into(), None).await.unwrap();
    child.wait_idle().await;
    let forwarded = next_oino(&mut events, |e| match e {
        OinoEvent::AgentEvent { agent_id, event } => {
            let value = serde_json::to_value(event).unwrap();
            (value["type"] == "agent_settled").then(|| agent_id.clone())
        }
        _ => None,
    })
    .await;
    assert_eq!(forwarded, child.id());
    assert!(matches!(
        child.messages().last(),
        Some(AgentMessage::Assistant(_))
    ));
    child.close(Duration::ZERO).await;
    let after = next_oino(&mut events, |e| match e {
        OinoEvent::AgentsChanged { agents } if agents.is_empty() => Some(()),
        _ => None,
    })
    .await;
    assert_eq!(after, ());
    assert_eq!(parent.agent_tree().len(), 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn single_item_queue_edits_keep_texts_and_messages_in_step() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("done".into())]);
    let services = services(dir.path(), provider, Vec::new(), JsonObject::new()).await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let session = rt.session();
    for text in ["a", "b", "c"] {
        session
            .follow_up(text.into(), Vec::new(), InputSource::Rpc)
            .await
            .unwrap();
    }
    let agent_texts = |session: &AgentSession| -> Vec<String> {
        session
            .agent()
            .follow_up_queue()
            .iter()
            .map(|m| match m {
                AgentMessage::User(u) => u.content.text(""),
                _ => String::new(),
            })
            .collect()
    };
    let queue = QueueKind::FollowUp;
    session
        .edit_queue(QueueEdit::Move {
            queue,
            from: 2,
            to: 0,
        })
        .await
        .unwrap();
    assert_eq!(session.follow_up_messages(), ["c", "a", "b"]);
    assert_eq!(agent_texts(&session), ["c", "a", "b"]);
    session
        .edit_queue(QueueEdit::Edit {
            queue,
            index: 1,
            text: "A!".into(),
        })
        .await
        .unwrap();
    session
        .edit_queue(QueueEdit::Remove { queue, index: 2 })
        .await
        .unwrap();
    assert_eq!(session.follow_up_messages(), ["c", "A!"]);
    assert_eq!(agent_texts(&session), ["c", "A!"]);
    let error = session
        .edit_queue(QueueEdit::Remove { queue, index: 9 })
        .await
        .unwrap_err();
    assert!(error.to_string().contains("out of range"), "{error}");
    // Send-now while idle prompts the item.
    session
        .edit_queue(QueueEdit::SendNow { queue, index: 1 })
        .await
        .unwrap();
    idle(&session).await;
    let users: Vec<String> = session
        .messages()
        .iter()
        .filter_map(|m| match m {
            AgentMessage::User(u) => Some(u.content.text("")),
            _ => None,
        })
        .collect();
    assert_eq!(users.first().map(String::as_str), Some("A!"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn checkpoints_restore_files_and_rewind_forks() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "v0").unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![(
            "w1".into(),
            "write".into(),
            args(json!({"path": "notes.txt", "content": "v1"})),
        )]),
        FauxResponse::Text("wrote".into()),
        FauxResponse::ToolCalls(vec![
            (
                "w2".into(),
                "write".into(),
                args(json!({"path": "notes.txt", "content": "v2"})),
            ),
            (
                "w3".into(),
                "write".into(),
                args(json!({"path": "new.txt", "content": "fresh"})),
            ),
        ]),
        FauxResponse::Text("wrote again".into()),
    ]);
    let services = services(dir.path(), provider, Vec::new(), JsonObject::new()).await;
    let rt = runtime(&services, SessionSource::New).await;
    let session = rt.session();
    session
        .prompt("first".into(), oino_core::session::PromptOptions::default())
        .await
        .unwrap();
    idle(&session).await;
    session
        .prompt(
            "second".into(),
            oino_core::session::PromptOptions::default(),
        )
        .await
        .unwrap();
    idle(&session).await;
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "v2");
    let checkpoints = session.checkpoints();
    assert_eq!(checkpoints.len(), 2, "{checkpoints:?}");
    assert_eq!(checkpoints[1].files.len(), 2);
    assert!(checkpoints[1].files[0].existed);
    assert!(!checkpoints[1].files[1].existed);
    // Persisted next to the agent dir.
    assert!(
        dir.path()
            .join("agent/checkpoints")
            .join(session.session_id())
            .join("index.json")
            .is_file()
    );
    // Undo the last turn's files, then rewind the conversation to the first prompt.
    let second = checkpoints[1].entry_id.clone();
    let undone = session.rewind_files(&second).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "v1");
    assert!(!dir.path().join("new.txt").exists());
    assert_eq!(undone.deleted.len(), 1);
    let first = checkpoints[0].entry_id.clone();
    session.rewind_files(&first).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "v0");
    assert_eq!(session.checkpoints().len(), 0);
    assert!(session.rewind_files(&first).is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_bash_calls_can_be_detached() {
    if cfg!(windows) {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![(
            "b1".into(),
            "bash".into(),
            args(json!({"command": "echo early; sleep 20; echo late"})),
        )]),
        FauxResponse::Text("ok".into()),
    ]);
    let services = services(dir.path(), provider, Vec::new(), JsonObject::new()).await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let session = rt.session();
    session
        .prompt("run".into(), oino_core::session::PromptOptions::default())
        .await
        .unwrap();
    let running = loop {
        let list = session.running_bash().await;
        if !list.is_empty() {
            break list;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(running[0].tool_call_id, "b1");
    assert!(running[0].command.contains("sleep 20"));
    assert!(running[0].pid.is_some());
    tokio::time::sleep(Duration::from_millis(200)).await;
    let detached = services
        .host
        .detach_bash("x", &cx(&session, CallKind::Tool), "b1", "t9", "too slow")
        .await
        .unwrap();
    assert_eq!(detached.pid, running[0].pid);
    assert!(detached.command.contains("sleep 20"));
    idle(&session).await;
    let result = session
        .messages()
        .into_iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) => Some(serde_json::to_string(&r.content).unwrap()),
            _ => None,
        })
        .unwrap();
    assert!(
        result.contains("[Detached to background task t9: too slow]"),
        "{result}"
    );
    assert_eq!(session.running_bash().await.len(), 0);
    detached.kill.cancel();
    let code = tokio::time::timeout(Duration::from_secs(5), detached.exit)
        .await
        .unwrap();
    assert_ne!(code, Some(0));
    assert!(
        services
            .host
            .detach_bash("x", &cx(&session, CallKind::Tool), "b1", "t10", "again")
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn disabled_extensions_are_not_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let mut settings = JsonObject::new();
    settings.insert("disabledExtensions".into(), json!(["off"]));
    let on: Arc<dyn ExtensionInstance> =
        Arc::new(FakeExtension::new("on", Registration::default()));
    let off: Arc<dyn ExtensionInstance> =
        Arc::new(FakeExtension::new("off", Registration::default()));
    let services = services(
        dir.path(),
        FauxProvider::new(Vec::new()),
        vec![on, off],
        settings,
    )
    .await;
    assert!(services.extensions.get("on").is_some());
    assert!(services.extensions.get("off").is_none());
    let rt = runtime(&services, SessionSource::InMemory).await;
    let infos = rt.session().extension_infos();
    let off = infos.iter().find(|i| i.id == "off").unwrap();
    assert!(!off.enabled && !off.loaded);
    assert!(infos.iter().any(|i| i.id == "on" && i.enabled && i.loaded));
    rt.session().set_extension_enabled("off", true).unwrap();
    assert!(rt.session().extension_infos().iter().all(|i| i.id != "off"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shortcuts_frontends_and_shutdown_reach_frontends() {
    let dir = tempfile::tempdir().unwrap();
    let keys: Arc<dyn ExtensionInstance> = Arc::new(FakeExtension::new(
        "unipi.input-shortcuts",
        Registration {
            shortcuts: vec![ShortcutSpec {
                id: "plan-toggle".into(),
                default_keys: vec!["alt+p".into()],
                description: "Toggle plan mode".into(),
            }],
            ..Registration::default()
        },
    ));
    let services = services(
        dir.path(),
        FauxProvider::new(Vec::new()),
        vec![keys],
        JsonObject::new(),
    )
    .await;
    let rt = runtime(&services, SessionSource::InMemory).await;
    let session = rt.session();
    let shortcuts = session
        .ui()
        .snapshot()
        .into_iter()
        .find(|r| r.method == "setShortcuts")
        .expect("setShortcuts retained");
    assert_eq!(
        shortcuts.params["shortcuts"],
        json!([{"id": "plan-toggle", "keys": ["alt+p"], "description": "Toggle plan mode", "extensionId": "unipi.input-shortcuts"}])
    );
    session.register_frontend("oino-desktop");
    let frontends = services
        .host
        .call("x", &cx(&session, CallKind::Event), HostOp::GetFrontends)
        .await
        .unwrap();
    assert_eq!(frontends, json!(["desktop", "oino-desktop"]));
    let mut events = session.subscribe();
    let mut requested = rt.shutdown_requested();
    services
        .host
        .call("x", &cx(&session, CallKind::Command), HostOp::Shutdown)
        .await
        .unwrap();
    next_oino(&mut events, |e| {
        matches!(e, OinoEvent::ShutdownRequested).then_some(())
    })
    .await;
    assert!(*requested.borrow_and_update());
}
