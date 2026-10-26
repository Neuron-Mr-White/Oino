//! `oino-client` contract: the same scenario against `InProcessBackend` and against
//! `RpcBackend` (an `oino --mode rpc` child) must give the same observable results
//! (modes.md §8).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

mod support;

use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use futures::stream::BoxStream;
use indexmap::IndexMap;
use oino_ai::{AgentMessage, AssistantContent, JsonObject};
use oino_client::protocol::ClientInfo;
use oino_client::{ClientError, InProcessBackend, PiCommand, SessionBackend};
use oino_core::events::{AgentSessionEvent, ClientEvent, OinoEvent, ReplaceReason};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_rpc::{ChildCommand, RpcBackend, Transport};
use serde_json::json;
use support::Sandbox;

const SCRIPT: &str = r#"[{"text": "hello"}, {"text": "again"}]"#;

async fn in_process(sandbox: &Sandbox) -> Arc<dyn SessionBackend> {
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "faux-key");
    let faux: Arc<dyn ExtensionInstance> = Arc::new(oino::faux::FauxComponent(
        oino::faux::FauxProvider::from_script(SCRIPT),
    ));
    let services = Services::create(ServicesOptions {
        cwd: sandbox.cwd(),
        agent_dir: sandbox.agent(),
        docs_root: sandbox.agent(),
        mode: ExtensionMode::Rpc,
        has_ui: true,
        flags: ResourceFlags {
            no_skills: true,
            no_prompt_templates: true,
            no_context_files: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(vec![faux])),
        approve: Some(false),
        models: Some(models),
        settings: Some(oino_settings::SettingsManager::in_memory(JsonObject::new())),
    })
    .await
    .unwrap();
    let runtime = AgentSessionRuntime::create(
        services,
        SessionOptions {
            source: SessionSource::InMemory,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    Arc::new(InProcessBackend::new(Arc::new(runtime)))
}

async fn over_rpc(sandbox: &Sandbox) -> Arc<dyn SessionBackend> {
    let transport = Transport::StdioChild(ChildCommand {
        program: support::oino_bin(),
        args: [
            "-e",
            "builtin:faux",
            "-nc",
            "-ns",
            "-np",
            "--no-session",
            "--mode",
            "rpc",
        ]
        .map(str::to_owned)
        .to_vec(),
        cwd: Some(sandbox.cwd()),
        env: vec![
            ("HOME".into(), sandbox.home().display().to_string()),
            (
                "OINO_AGENT_DIR".into(),
                sandbox.agent().display().to_string(),
            ),
            ("OINO_FAUX_API_KEY".into(), "faux-key".into()),
            ("OINO_FAUX_RESPONSES".into(), SCRIPT.into()),
            ("OINO_BUNDLED".into(), "0".into()),
        ],
    });
    let backend = RpcBackend::connect(
        transport,
        ClientInfo {
            name: "contract".into(),
            version: "1".into(),
        },
        None,
    )
    .await
    .unwrap();
    Arc::new(backend)
}

async fn until_settled(events: &mut BoxStream<'static, ClientEvent>) -> Vec<ClientEvent> {
    let mut seen = Vec::new();
    loop {
        let event = tokio::time::timeout(Duration::from_secs(20), events.next())
            .await
            .unwrap()
            .unwrap();
        let settled = matches!(
            event,
            ClientEvent::Session(AgentSessionEvent::AgentSettled { .. })
        );
        seen.push(event);
        if settled {
            return seen;
        }
    }
}

fn kind(event: &ClientEvent) -> String {
    serde_json::to_value(event).unwrap()["type"]
        .as_str()
        .unwrap()
        .to_owned()
}

/// Observable results of the scenario.
#[derive(Debug, PartialEq)]
struct Observed {
    kinds: Vec<String>,
    streamed_text: String,
    disposition: serde_json::Value,
    last_text: serde_json::Value,
    model_error: ClientError,
    state_matches_id: bool,
    replaced: Option<ReplaceReason>,
    messages_after_new: serde_json::Value,
}

async fn scenario(backend: Arc<dyn SessionBackend>) -> Observed {
    let mut events = backend.events();
    let disposition = backend
        .send(
            PiCommand::Prompt {
                message: "hi".into(),
                images: Vec::new(),
                streaming_behavior: None,
            }
            .into(),
        )
        .await
        .unwrap()
        .unwrap();
    let run = until_settled(&mut events).await;
    let kinds = run
        .iter()
        .map(kind)
        .filter(|k| k != "message_update")
        .collect();
    let streamed_text = run
        .iter()
        .rev()
        .find_map(|e| match e {
            ClientEvent::Session(AgentSessionEvent::MessageUpdate {
                message: AgentMessage::Assistant(partial),
                ..
            }) => Some(
                partial
                    .content
                    .iter()
                    .filter_map(|c| match c {
                        AssistantContent::Text(t) => Some(t.text.clone()),
                        _ => None,
                    })
                    .collect::<String>(),
            ),
            _ => None,
        })
        .unwrap_or_default();
    let last_text = backend
        .send(PiCommand::GetLastAssistantText.into())
        .await
        .unwrap()
        .unwrap();
    let model_error = backend
        .send(
            PiCommand::SetModel {
                provider: "faux".into(),
                model_id: "missing".into(),
            }
            .into(),
        )
        .await
        .unwrap_err();
    let state = backend
        .send(PiCommand::GetState.into())
        .await
        .unwrap()
        .unwrap();
    let state_matches_id = state["sessionId"] == json!(backend.session_id());
    backend
        .send(
            PiCommand::NewSession {
                parent_session: None,
            }
            .into(),
        )
        .await
        .unwrap();
    let replaced = loop {
        let event = tokio::time::timeout(Duration::from_secs(20), events.next())
            .await
            .unwrap()
            .unwrap();
        if let ClientEvent::Oino(OinoEvent::SessionReplaced { reason, .. }) = event {
            break Some(reason);
        }
    };
    let messages_after_new = backend
        .send(PiCommand::GetMessages.into())
        .await
        .unwrap()
        .unwrap()["messages"]
        .clone();
    backend.shutdown().await;
    Observed {
        kinds,
        streamed_text,
        disposition,
        last_text,
        model_error,
        state_matches_id,
        replaced,
        messages_after_new,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn in_process_and_rpc_backends_behave_the_same() {
    let local = Sandbox::new();
    let remote = Sandbox::new();
    let backend = in_process(&local).await;
    let local = scenario(backend).await;
    let remote = scenario(over_rpc(&remote).await).await;
    assert_eq!(local.streamed_text, "hello");
    assert_eq!(local.disposition, json!({"disposition": "started"}));
    assert_eq!(local.last_text, json!({"text": "hello"}));
    assert_eq!(
        local.model_error,
        ClientError::Command("Model not found: faux/missing".into())
    );
    assert!(local.state_matches_id);
    assert_eq!(local.replaced, Some(ReplaceReason::New));
    assert_eq!(local, remote);
}

/// What the frontend surface (`FrontendApi`) reports, backend-independent.
#[derive(Debug, PartialEq)]
struct FrontendObserved {
    providers: Vec<(String, bool)>,
    paths: Vec<String>,
    shortcuts: usize,
    namespaces: usize,
    service_error: ClientError,
    shortcut_error: ClientError,
    consumed: bool,
}

async fn frontend_scenario(
    sandbox: &Sandbox,
    backend: Arc<dyn SessionBackend>,
) -> FrontendObserved {
    use oino_client::FrontendApi;
    std::fs::write(sandbox.cwd().join("alpha.txt"), "").unwrap();
    let providers = backend
        .auth_providers()
        .await
        .unwrap()
        .into_iter()
        .map(|p| (p.id, p.oauth))
        .collect();
    let paths = backend
        .completions("@alp", 4)
        .await
        .unwrap()
        .into_iter()
        .flat_map(|g| g.items.into_iter().map(|i| i.value))
        .collect();
    let event = oino_client::protocol::EditorEvent {
        event_id: "e".into(),
        intent: oino_client::protocol::EditorIntent::EmptyArrowDown,
        text: String::new(),
        token_id: None,
    };
    let observed = FrontendObserved {
        providers,
        paths,
        shortcuts: backend.shortcuts().await.unwrap().len(),
        namespaces: backend.settings_schema().await.unwrap().len(),
        service_error: backend
            .service_call(None, "nope", json!(null))
            .await
            .unwrap_err(),
        shortcut_error: backend.invoke_shortcut("nope").await.unwrap_err(),
        consumed: backend.editor_event(event).await.unwrap(),
    };
    backend.shutdown().await;
    observed
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn frontend_api_is_the_same_in_process_and_over_rpc() {
    let local = Sandbox::new();
    let remote = Sandbox::new();
    let local_observed = frontend_scenario(&local, in_process(&local).await).await;
    let remote_observed = frontend_scenario(&remote, over_rpc(&remote).await).await;
    assert_eq!(
        local_observed.providers,
        [("faux".to_owned(), false), ("faux-oauth".to_owned(), true)]
    );
    assert_eq!(local_observed.paths, ["@alpha.txt"]);
    assert_eq!(
        local_observed.service_error,
        ClientError::Command("Unknown service: nope".into())
    );
    assert!(!local_observed.consumed);
    assert_eq!(local_observed, remote_observed);
}
