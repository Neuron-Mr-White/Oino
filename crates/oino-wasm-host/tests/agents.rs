//! Nested agents and the turn arbiter through the real ABI: the `agent-probe` component,
//! loaded with `core_bridge::CoreLoader` into an `oino-core` session answered by the native
//! faux provider, spawns child agents, proposes nudges, reports wait reasons and asks for
//! shutdown.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]
#![cfg(feature = "core")]

use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oino_ai::{AgentMessage, JsonObject, JsonValue};
use oino_core::events::RunState;
use oino_core::extensions::api::{CallKind, ExtensionInstance, ExtensionMode};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::session::{AgentSession, PromptOptions};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_extension_api::{ComponentSource, LoadedPackage};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::testing::{build_fixtures, fixture_manifest, fixture_wasm};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::json;

const PROBE: &str = "oino.fixture.agent-probe";

struct Loader {
    faux: Arc<FauxProvider>,
    wasm: CoreLoader,
}

impl ExtensionLoader for Loader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        Box::pin(async move {
            let mut out: Vec<Arc<dyn ExtensionInstance>> =
                vec![Arc::new(FauxComponent(Arc::clone(&self.faux)))];
            out.extend(self.wasm.load(host, trusted).await);
            out
        })
    }
}

struct Env {
    _dir: tempfile::TempDir,
    services: Arc<Services>,
    runtime: AgentSessionRuntime,
}

async fn env(responses: Vec<FauxResponse>) -> (Env, Arc<FauxProvider>) {
    build_fixtures().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(responses);
    let wasm = Arc::new(
        WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
            dir.path().join("agent"),
            dir.path(),
        )))
        .unwrap(),
    );
    let package = LoadedPackage {
        manifest: fixture_manifest("agent-probe", None).unwrap(),
        component: ComponentSource::Path(fixture_wasm("agent-probe")),
        root: None,
        bundled: false,
    };
    let loader = Loader {
        faux: Arc::clone(&provider),
        wasm: CoreLoader::new(
            wasm,
            vec![SourcedPackage {
                package,
                project: false,
            }],
        ),
    };
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let services = Services::create(ServicesOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir: dir.path().join("agent"),
        docs_root: dir.path().join("docs"),
        mode: ExtensionMode::Print,
        has_ui: false,
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(loader),
        approve: Some(false),
        models: Some(models),
        settings: Some(oino_settings::SettingsManager::in_memory(JsonObject::new())),
    })
    .await
    .unwrap();
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source: SessionSource::New,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    (
        Env {
            _dir: dir,
            services,
            runtime,
        },
        provider,
    )
}

async fn command(env: &Env, name: &str, args: &str) -> JsonValue {
    let session = env.runtime.session();
    let cx = session.runner().call_context().with_kind(CallKind::Command);
    env.services
        .host
        .call(
            PROBE,
            &cx,
            HostOp::InvokeCommand {
                name: name.into(),
                args: args.into(),
            },
        )
        .await
        .unwrap()
}

async fn idle(session: &AgentSession) {
    tokio::time::timeout(Duration::from_secs(20), session.wait_for_idle())
        .await
        .unwrap();
}

fn custom_entries(session: &AgentSession, custom_type: &str) -> Vec<JsonValue> {
    session
        .entries()
        .iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .filter(|e| e["customType"] == custom_type)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_tool_delegates_to_a_child_agent() {
    let (env, provider) = env(vec![
        FauxResponse::ToolCalls(vec![(
            "c1".into(),
            "delegate".into(),
            serde_json::from_value(json!({"task": "summarise"})).unwrap(),
        )]),
        FauxResponse::Text("child says hi".into()),
        FauxResponse::Text("parent done".into()),
    ])
    .await;
    let session = env.runtime.session();
    session
        .prompt("delegate it".into(), PromptOptions::default())
        .await
        .unwrap();
    idle(&session).await;
    let result = session
        .messages()
        .into_iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) => Some(r),
            _ => None,
        })
        .unwrap();
    assert!(!result.is_error, "{result:?}");
    assert_eq!(
        result.content,
        vec![oino_ai::UserContent::text("child says hi")]
    );
    let details = result.details.clone().unwrap();
    let events: Vec<&str> = details["events"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(JsonValue::as_str)
        .collect();
    for expected in ["agent_start", "message_end", "agent_end", "agent_settled"] {
        assert!(events.contains(&expected), "{expected}: {events:?}");
    }
    // The child's request carried only its own prompt.
    let child_request = provider.requests()[1].to_string();
    assert!(child_request.contains("summarise") && !child_request.contains("delegate it"));
    assert!(
        session
            .entries()
            .iter()
            .filter_map(|e| serde_json::to_value(e).ok())
            .any(|e| e["type"] == "usage" && e["kind"] == "child:delegate"),
    );
    assert_eq!(
        session.last_assistant_text().as_deref(),
        Some("parent done")
    );
    assert!(session.child_agents().is_empty(), "closed by the tool");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn spawn_command_persists_lists_and_closes() {
    let (env, _provider) = env(vec![FauxResponse::Text("persisted answer".into())]).await;
    let spec = json!({"label": "file child", "persistence": {"file": {}}, "tools": {"builtin": ["read"]}, "ui": "none"});
    let report = command(&env, "spawn", &spec.to_string()).await;
    assert_eq!(report["disposition"], "started");
    assert_eq!(report["text"], "persisted answer");
    let file = report["sessionFile"].as_str().unwrap().to_owned();
    assert!(file.contains("/children/"), "{file}");
    assert!(std::path::Path::new(&file).is_file());
    assert_eq!(report["listed"][0]["label"], "file child");
    assert_eq!(report["listed"][0]["state"], "idle");
    assert_eq!(report["usage"]["assistantMessages"], 1);
    let list = command(&env, "close", "").await;
    assert_eq!(list, json!([]));
    assert!(env.runtime.session().child_agents().is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn arbiter_nudges_and_wait_sources_reach_the_component() {
    let (env, provider) = env(vec![
        FauxResponse::Text("first".into()),
        FauxResponse::Text("after nudge".into()),
        FauxResponse::Text("waiting run".into()),
    ])
    .await;
    let session = env.runtime.session();
    command(&env, "nudge", "on").await;
    session
        .prompt("go".into(), PromptOptions::default())
        .await
        .unwrap();
    idle(&session).await;
    assert_eq!(provider.requests().len(), 2);
    assert!(
        session.messages().iter().any(
            |m| matches!(m, AgentMessage::Custom(c) if c.custom_type == "probe:nudge" && c.content.text("") == "nudged: keep going")
        )
    );
    assert_eq!(
        custom_entries(&session, "probe.delivered")[0]["data"]["id"],
        "probe-nudge"
    );
    assert_eq!(
        session.last_assistant_text().as_deref(),
        Some("after nudge")
    );

    command(&env, "wait", "background job").await;
    command(&env, "nudge", "on").await;
    session
        .prompt("again".into(), PromptOptions::default())
        .await
        .unwrap();
    idle(&session).await;
    assert_eq!(provider.requests().len(), 3, "waiting suppresses the nudge");
    assert_eq!(session.run_state(), RunState::Waiting);
    assert_eq!(
        command(&env, "run-state", "").await,
        json!({"state": "waiting", "waitingOn": ["background job"]})
    );
    command(&env, "wait", "off").await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(8);
    while session.run_state() == RunState::Waiting && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(session.run_state(), RunState::Idle);
    assert_eq!(
        command(&env, "run-state", "").await,
        json!({"state": "idle"})
    );

    let shutdown = env.runtime.shutdown_requested();
    command(&env, "shutdown", "").await;
    assert!(*shutdown.borrow());
}
