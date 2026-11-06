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
    assert_eq!(std::fs::read_to_string(f.cwd.join("a.txt")).unwrap(), "new");
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
        ids(f.backend.send(AgentCommand::GetAvailableModels.into()).await.unwrap()),
        vec!["qwen"]
    );

    write_models("llama");
    // `/reload` (the terminal's built-in and the app's) is this command.
    f.backend.send(AgentCommand::Reload.into()).await.unwrap();

    assert_eq!(
        ids(f.backend.send(AgentCommand::GetAvailableModels.into()).await.unwrap()),
        vec!["llama"]
    );
}

/// The app host parses typed commands, not RpcServer's JSON-only fast paths.
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
