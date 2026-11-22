//! `InProcessBackend` GUI surface: runtime-agnostic calls (GA-40) and the typed
//! `FrontendApi` additions (topics, agents, queue edits, checkpoints / rewind, extensions,
//! shutdown).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping; one scenario per test"
)]

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use indexmap::IndexMap;
use oino_ai::JsonObject;
use oino_client::protocol::{EditorEvent, EditorIntent, QueueKind};
use oino_client::{AgentCommand, FrontendApi, InProcessBackend, RewindOptions, SessionBackend};
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode, Registration};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse};
use serde_json::json;

struct Fixture {
    _dir: tempfile::TempDir,
    cwd: std::path::PathBuf,
    services: Arc<Services>,
    backend: Arc<InProcessBackend>,
}

async fn fixture(responses: Vec<FauxResponse>, extra: Vec<Arc<dyn ExtensionInstance>>) -> Fixture {
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    fixture_with(models, responses, extra).await
}

/// A fixture over `models` (a registry that can re-read a real `models.json`).
async fn fixture_with(
    models: oino_models::ModelRegistry,
    responses: Vec<FauxResponse>,
    extra: Vec<Arc<dyn ExtensionInstance>>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let mut instances: Vec<Arc<dyn ExtensionInstance>> =
        vec![Arc::new(FauxComponent(FauxProvider::new(responses)))];
    instances.extend(extra);
    let services = Services::create(ServicesOptions {
        cwd: cwd.clone(),
        agent_dir: cwd.join("agent"),
        docs_root: cwd.join("docs"),
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
        settings: Some(oino_settings::SettingsManager::in_memory(JsonObject::new())),
    })
    .await
    .unwrap();
    let context = Arc::new(SessionToolContext::default());
    let settings = services.settings.lock().unwrap().snapshot();
    let builtin_tools =
        builtin_tool_entries(&cwd, &services.agent_dir, &settings, Arc::clone(&context));
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source: SessionSource::New,
            builtin_tools,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    context.bind(&runtime.session());
    Fixture {
        _dir: dir,
        cwd,
        services,
        backend: Arc::new(InProcessBackend::new(Arc::new(runtime))),
    }
}

fn args(value: serde_json::Value) -> JsonObject {
    serde_json::from_value(value).unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn commands_work_from_a_non_tokio_thread() {
    let editor: Arc<dyn ExtensionInstance> = Arc::new(FakeExtension::new(
        "editor",
        Registration {
            events: vec!["oino_editor".into()],
            ..Registration::default()
        },
    ));
    let f = fixture(Vec::new(), vec![editor]).await;
    let backend = Arc::clone(&f.backend);
    // A GPUI-like executor: plain thread, no tokio context (GA-1 / GA-40).
    let consumed = std::thread::spawn(move || {
        assert!(tokio::runtime::Handle::try_current().is_err());
        futures::executor::block_on(async {
            let event = EditorEvent {
                event_id: "e1".into(),
                intent: EditorIntent::Changed,
                text: "/pl".into(),
                token_id: None,
            };
            let consumed = backend.editor_event(event).await.unwrap();
            let state = backend.send(AgentCommand::GetState.into()).await.unwrap();
            let topics = backend.topics(&["acme:*"], &[]).await.unwrap();
            (consumed, state.is_some(), topics.len())
        })
    })
    .join()
    .unwrap();
    assert_eq!(consumed, (false, true, 0));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frontend_api_covers_the_gui_feeds() {
    let f = fixture(
        vec![
            FauxResponse::ToolCalls(vec![(
                "w1".into(),
                "write".into(),
                args(json!({"path": "a.txt", "content": "new"})),
            )]),
            FauxResponse::Text("done".into()),
        ],
        Vec::new(),
    )
    .await;
    let backend = &f.backend;
    std::fs::write(f.cwd.join("a.txt"), "old").unwrap();
    // Topics.
    let session = backend.runtime().session();
    let cx = session.runner().call_context().clone();
    f.services
        .host
        .call(
            "acme.fusion",
            &cx,
            HostOp::BusPublish {
                topic: "acme:fusion:status".into(),
                data: json!({"sidekick": "m"}),
                sticky: true,
            },
        )
        .await
        .unwrap();
    let topics = backend.topics(&["acme:fusion:*"], &[]).await.unwrap();
    assert_eq!(topics[0].data, json!({"sidekick": "m"}));
    assert_eq!(
        backend.bus_get("acme:fusion:status").await.unwrap(),
        Some(json!({"sidekick": "m"}))
    );
    assert_eq!(backend.bus_get("nope").await.unwrap(), None);
    // Agents.
    let tree = backend.agents(Some(true)).await.unwrap();
    assert!(tree.agents.is_empty() && tree.follow);
    assert!(backend.agent_messages("missing").await.is_err());
    // Queue edits.
    for text in ["one", "two"] {
        backend
            .send(
                AgentCommand::FollowUp {
                    message: text.into(),
                    images: Vec::new(),
                }
                .into(),
            )
            .await
            .unwrap();
    }
    backend.queue_move(QueueKind::FollowUp, 1, 0).await.unwrap();
    backend
        .queue_set_text(QueueKind::FollowUp, 1, "ONE")
        .await
        .unwrap();
    backend.queue_remove(QueueKind::FollowUp, 0).await.unwrap();
    let queued = backend
        .send(AgentCommand::ClearQueue.into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(queued, json!({"steering": [], "followUp": ["ONE"]}));
    // Checkpoints and rewind (files + conversation fork).
    let mut events = backend.events();
    backend
        .send(
            AgentCommand::Prompt {
                message: "edit it".into(),
                images: Vec::new(),
                streaming_behavior: None,
            }
            .into(),
        )
        .await
        .unwrap();
    backend.runtime().session().wait_for_idle().await;
    assert_eq!(
        std::fs::read_to_string(f.cwd.join("a.txt")).unwrap(),
        "new\n"
    );
    let checkpoints = backend.checkpoints().await.unwrap();
    assert_eq!(checkpoints.len(), 1);
    let old_id = backend.session_id();
    let result = backend
        .rewind(&checkpoints[0].entry_id, RewindOptions::default())
        .await
        .unwrap();
    assert_eq!(std::fs::read_to_string(f.cwd.join("a.txt")).unwrap(), "old");
    assert_eq!(result.restored.len(), 1);
    assert_eq!(result.text.as_deref(), Some("edit it"));
    assert!(!result.cancelled);
    assert_ne!(backend.session_id(), old_id);
    let replaced = loop {
        let event = tokio::time::timeout(Duration::from_secs(10), events.next())
            .await
            .unwrap()
            .unwrap();
        if let ClientEvent::Oino(OinoEvent::SessionReplaced { reason, .. }) = event {
            break reason;
        }
    };
    assert_eq!(replaced, oino_core::events::ReplaceReason::Fork);
    let messages = backend
        .send(AgentCommand::GetMessages.into())
        .await
        .unwrap()
        .unwrap();
    assert!(
        !serde_json::to_string(&messages)
            .unwrap()
            .contains("edit it"),
        "{messages}"
    );
    // Following the agent tree carries over to the forked session.
    assert!(backend.agents(None).await.unwrap().follow);
    // Running bash, extensions.
    assert_eq!(backend.running_bash().await.unwrap().len(), 0);
    let extensions = backend.extensions().await.unwrap();
    let faux = extensions[0].id.clone();
    assert!(extensions[0].enabled && extensions[0].loaded);
    backend.set_extension_enabled(&faux, false).await.unwrap();
    assert!(
        backend
            .extensions()
            .await
            .unwrap()
            .iter()
            .any(|e| e.id == faux && !e.enabled)
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_requested_resolves_when_idle() {
    let f = fixture(Vec::new(), Vec::new()).await;
    let waiting = f.backend.shutdown_requested();
    let session = f.backend.runtime().session();
    let cx = session
        .runner()
        .call_context()
        .with_kind(oino_core::extensions::api::CallKind::Command);
    f.services
        .host
        .call("x", &cx, HostOp::Shutdown)
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), waiting)
        .await
        .unwrap();
    f.backend.shutdown().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reload_rereads_models_json_without_a_restart() {
    // A real `models.json` under the agent dir: the registry reloads it from disk.
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let write_models = |models: &str| {
        std::fs::write(
            agent_dir.join("models.json"),
            json!({"providers": {"ollama": {"baseUrl": "http://localhost:11434/v1",
                "api": "openai-completions", "apiKey": "ollama", "models": [{"id": models}]}}})
            .to_string(),
        )
        .unwrap();
    };
    write_models("qwen");
    let auth = Arc::new(oino_models::AuthStorage::in_memory(IndexMap::new()));
    let env = Arc::new(oino_models::MapEnv::default());
    let models = oino_models::ModelRegistry::new(
        auth,
        oino_models::ModelRegistryOptions {
            models_json_path: Some(agent_dir.join("models.json")),
            env,
            ..oino_models::ModelRegistryOptions::default()
        },
    );
    // A `models.json` provider is only offered once some implementation serves its `api`.
    models.register_api_implementor("openai-completions", "ext.openai");
    let f = fixture_with(models, Vec::new(), Vec::new()).await;
    let ids = |reply: Option<serde_json::Value>| {
        reply
            .and_then(|data| data.get("models").cloned())
            .and_then(|rows| rows.as_array().cloned())
            .unwrap_or_default()
            .iter()
            .map(|m| m["id"].as_str().unwrap_or_default().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(
        ids(f
            .backend
            .send(AgentCommand::GetAvailableModels.into())
            .await
            .unwrap()),
        vec!["qwen"]
    );

    write_models("llama");
    // `/reload` (the terminal's built-in and the app's) is this command.
    f.backend.send(AgentCommand::Reload.into()).await.unwrap();

    assert_eq!(
        ids(f
            .backend
            .send(AgentCommand::GetAvailableModels.into())
            .await
            .unwrap()),
        vec!["llama"]
    );
}

/// The app host parses typed commands, not `RpcServer`'s JSON-only fast paths.
#[tokio::test]
async fn typed_autonomy_and_approval_bridge() {
    use oino_client::protocol::{ClientInfo, OinoCommand, RpcRequest};
    let f = fixture(Vec::new(), vec![]).await;
    let set = RpcRequest::from_value(json!({"type":"set_autonomy","level":"low"})).unwrap();
    f.backend.send(set.command).await.unwrap();
    let state = f
        .backend
        .send(AgentCommand::GetState.into())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state["autonomy"], "low");
    let hello = |approvals| OinoCommand::Hello {
        client: ClientInfo {
            name: "unipi-app".into(),
            version: "test".into(),
        },
        ui: Some(json!({"approvals":approvals})),
        protocol: 2,
    };
    f.backend.send(hello(true).into()).await.unwrap();
    let mut events = f.backend.events();
    // Deny before execution: the shell command must never run.
    f.backend
        .send(
            OinoCommand::SetSetting {
                key: "permissions".into(),
                value: Some(json!({"version":1,"rules":[],"legacyDeny":[],"default":"ask"})),
                scope: oino_settings::SettingsScope::Global,
            }
            .into(),
        )
        .await
        .unwrap();
    let write = f.backend.clone();
    let prompt = tokio::spawn(async move {
        write
            .send(
                AgentCommand::Bash {
                    command: "git push".into(),
                    exclude_from_context: Some(false),
                }
                .into(),
            )
            .await
    });
    let approval = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(ClientEvent::Oino(OinoEvent::ApprovalRequest { request })) =
                events.next().await
            {
                break request;
            }
        }
    })
    .await
    .unwrap();
    let response = RpcRequest::from_value(
        json!({"type":"approval_response","requestId":approval.request_id,"decision":"deny"}),
    )
    .unwrap();
    f.backend.send(response.command.clone()).await.unwrap();
    assert!(prompt.await.unwrap().is_err());
    assert!(f.backend.send(response.command).await.is_err());
    f.backend.send(hello(false).into()).await.unwrap();
}

/// "Send as note": `append_note` writes an `oino.note` custom entry, publishes it, and the
/// model context never contains it (also after a run that happened before it).
#[tokio::test]
async fn append_note_saves_a_custom_entry_the_model_never_sees() {
    let f = fixture(vec![FauxResponse::Text("hello back".into())], Vec::new()).await;
    let backend = &f.backend;
    let mut events = backend.events();
    backend
        .send(
            AgentCommand::Prompt {
                message: "hi".into(),
                images: Vec::new(),
                streaming_behavior: None,
            }
            .into(),
        )
        .await
        .unwrap();
    backend.runtime().session().wait_for_idle().await;
    let reply = backend
        .send(
            AgentCommand::AppendNote {
                text: "PRIVATE-NOTE $skill /cmd".into(),
                images: vec![oino_ai::ImageContent {
                    data: "aGk=".into(),
                    mime_type: "image/png".into(),
                }],
            }
            .into(),
        )
        .await
        .unwrap()
        .unwrap();
    let entry_id = reply["entryId"].as_str().unwrap().to_owned();
    // Live clients see the entry.
    let mut published = None;
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(2), events.next()).await {
        if let ClientEvent::Session(oino_core::events::AgentSessionEvent::EntryAppended { entry }) =
            event
        {
            published = Some(entry);
            break;
        }
    }
    let published = serde_json::to_value(published.expect("EntryAppended")).unwrap();
    assert_eq!(published["id"], entry_id);
    assert_eq!(published["type"], "custom");
    assert_eq!(published["customType"], "oino.note");
    assert_eq!(published["data"]["text"], "PRIVATE-NOTE $skill /cmd");
    assert_eq!(published["data"]["images"][0]["mimeType"], "image/png");
    // Saved in the session …
    let entries = backend
        .send(AgentCommand::GetEntries { since: None }.into())
        .await
        .unwrap()
        .unwrap();
    assert!(
        entries["entries"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["id"] == entry_id.as_str() && e["customType"] == "oino.note")
    );
    // … and not part of the model context.
    let messages = backend
        .send(AgentCommand::GetMessages.into())
        .await
        .unwrap()
        .unwrap();
    let context = messages.to_string();
    assert!(context.contains("hello back"), "{context}");
    assert!(!context.contains("PRIVATE-NOTE"), "{context}");
    // An empty note is refused.
    assert!(
        backend
            .send(
                AgentCommand::AppendNote {
                    text: "  ".into(),
                    images: Vec::new()
                }
                .into()
            )
            .await
            .is_err()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn pending_request_snapshot_tracks_approval_wait_until_answered() {
    use oino_core::session::permission::{ApprovalDecision, ApprovalResponse, Level};
    let f = fixture(Vec::new(), Vec::new()).await;
    let session = f.backend.runtime().session();
    session.set_autonomy(Level::Low);
    session.set_approval_ui(true);
    let checking = session.clone();
    let wait = tokio::spawn(async move {
        checking
            .check_permission(
                "bash",
                &args(json!({"command":"git push", "riskLevel":"high"})),
            )
            .await
    });
    let pending = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let ui = f
                .backend
                .send(oino_client::protocol::RpcCommand::Oino(
                    oino_client::protocol::OinoCommand::UiSnapshot,
                ))
                .await
                .unwrap()
                .unwrap();
            if let Some(request) = ui["pending_requests"].as_array().unwrap().first() {
                break request.clone();
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert_eq!(pending["type"], "approval_request");
    assert_eq!(pending["tool"], "bash");
    assert!(!wait.is_finished());
    f.backend
        .approval_response(ApprovalResponse {
            request_id: pending["requestId"].as_str().unwrap().into(),
            decision: ApprovalDecision::AllowOnce,
        })
        .unwrap();
    assert!(wait.await.unwrap().is_ok());
    let ui = f
        .backend
        .send(oino_client::protocol::RpcCommand::Oino(
            oino_client::protocol::OinoCommand::UiSnapshot,
        ))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(ui["pending_requests"], json!([]));
}

#[tokio::test]
async fn retry_uses_original_durable_identity_and_consumes_failed_item() {
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let f = fixture_with(
        models.clone(),
        vec![FauxResponse::Text("done".into())],
        vec![],
    )
    .await;
    models.remove_runtime_api_key("faux");
    f.backend.restore_durable_queue().await.unwrap();
    let command = || {
        oino_client::protocol::RpcRequest::from_value(json!({
            "type":"prompt", "message":"one user intent"
        }))
        .unwrap()
        .command
    };
    assert!(
        f.backend
            .send_with_id(Some("failed-original".into()), command())
            .await
            .is_err()
    );
    let session = f.backend.runtime().session();
    let queue =
        oino_core::session::durable_queue::DurableQueue::new(&session.session_file().unwrap());
    assert_eq!(
        queue
            .load(&std::collections::HashSet::default())
            .unwrap()
            .len(),
        1
    );
    models.set_runtime_api_key("faux", "k");
    f.backend
        .send_with_id(Some("failed-original".into()), command())
        .await
        .unwrap();
    session.wait_for_idle().await;
    assert_eq!(
        queue.load(&std::collections::HashSet::default()).unwrap(),
        [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
    );
    let users: Vec<_> = session
        .entries()
        .into_iter()
        .filter(|entry| matches!(entry.message(), Some(oino_ai::AgentMessage::User(_))))
        .collect();
    assert_eq!(users.len(), 1);
    assert_eq!(users[0].id.as_str(), "failed-original");
    // A repeated click after success is a receipt hit, not another prompt.
    f.backend
        .send_with_id(Some("failed-original".into()), command())
        .await
        .unwrap();
    assert_eq!(
        queue.load(&std::collections::HashSet::default()).unwrap(),
        [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
    );
}

/// The parent kills only this owned helper, at four public Retry API checkpoints.
#[tokio::test]
async fn retry_kill_child() {
    let Ok(root) = std::env::var("OINO_RETRY_KILL_ROOT") else {
        return;
    };
    let stop: usize = std::env::var("OINO_RETRY_KILL_POINT")
        .unwrap()
        .parse()
        .unwrap();
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let f = fixture_with(
        models.clone(),
        vec![FauxResponse::Text("done".into())],
        vec![],
    )
    .await;
    f.backend.restore_durable_queue().await.unwrap();
    let session = f.backend.runtime().session();
    std::fs::write(
        std::path::Path::new(&root).join("session-path"),
        session.session_file().unwrap().to_string_lossy().as_bytes(),
    )
    .unwrap();
    std::fs::write(
        std::path::Path::new(&root).join("fixture-root"),
        f.cwd.to_string_lossy().as_bytes(),
    )
    .unwrap();
    let checkpoint = |point: usize| {
        if stop == point {
            std::fs::write(std::path::Path::new(&root).join("ready"), point.to_string()).unwrap();
            loop {
                std::thread::park();
            }
        }
    };
    let command = || {
        oino_client::protocol::RpcRequest::from_value(
            json!({"type":"prompt","message":"one user intent"}),
        )
        .unwrap()
        .command
    };
    models.remove_runtime_api_key("faux");
    assert!(
        f.backend
            .send_with_id(Some("failed-original".into()), command())
            .await
            .is_err()
    );
    checkpoint(0); // failed original is the only durable item
    models.set_runtime_api_key("faux", "k");
    checkpoint(1); // Retry owns the same item; no remove/re-admit window
    f.backend
        .send_with_id(Some("failed-original".into()), command())
        .await
        .unwrap();
    checkpoint(2); // successful Retry returned (receipt may already be persisted)
    session.wait_for_idle().await;
    checkpoint(3); // receipt persisted and queue cleaned
}

#[tokio::test]
async fn retry_kill_matrix_replays_no_extra_intent() {
    for point in 0..4 {
        let root = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "retry_kill_child", "--nocapture"])
            .env("OINO_RETRY_KILL_ROOT", root.path())
            .env("OINO_RETRY_KILL_POINT", point.to_string())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !root.path().join("ready").exists() {
            if std::time::Instant::now() > deadline {
                child.kill().unwrap();
                child.wait().unwrap();
                panic!("retry checkpoint {point} not reached");
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        child.kill().unwrap();
        child.wait().unwrap();
        let path = std::path::PathBuf::from(
            std::fs::read_to_string(root.path().join("session-path")).unwrap(),
        );
        let fixture_root = std::path::PathBuf::from(
            std::fs::read_to_string(root.path().join("fixture-root")).unwrap(),
        );
        let f = fixture(vec![FauxResponse::Text("recovered".into())], vec![]).await;
        let runtime = Arc::new(
            AgentSessionRuntime::create(
                Arc::clone(&f.services),
                SessionOptions {
                    source: SessionSource::Open(path.clone()),
                    ..SessionOptions::default()
                },
            )
            .await
            .unwrap(),
        );
        let backend = InProcessBackend::new(runtime.clone());
        backend.restore_durable_queue().await.unwrap();
        runtime.session().wait_for_idle().await;
        let entries = std::fs::read_to_string(&path).unwrap();
        let users: Vec<serde_json::Value> = entries
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|entry: &serde_json::Value| entry["message"]["role"] == "user")
            .collect();
        assert_eq!(users.len(), 1, "retry kill point {point}: {users:?}");
        assert_eq!(users[0]["id"], "failed-original");
        assert_eq!(
            oino_core::session::durable_queue::DurableQueue::new(&path)
                .load(&std::collections::HashSet::default())
                .unwrap(),
            [] as [oino_core::session::durable_queue::DurableQueueItem; 0]
        );
        std::fs::remove_dir_all(fixture_root).unwrap();
    }
}
