//! Core integration: the echo component loaded through `core_bridge::CoreLoader` into an
//! `oino-core` session sees pi events, serves tools and blocks tool calls.

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
use oino_ai::{AgentMessage, JsonObject};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::extensions::host::HostApi;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::session::PromptOptions;
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_extension_api::{ComponentSource, LoadedPackage};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::testing::{build_fixtures, fixture_manifest, fixture_wasm};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::json;

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn echo_component_in_a_core_session() {
    build_fixtures().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![(
            "c1".into(),
            "echo".into(),
            serde_json::from_value(json!({"text": "from wasm"})).unwrap(),
        )]),
        FauxResponse::Text("done".into()),
    ]);
    let runtime = Arc::new(
        WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
            dir.path().join("agent"),
            dir.path(),
        )))
        .unwrap(),
    );
    let package = LoadedPackage {
        manifest: fixture_manifest("echo", None).unwrap(),
        component: ComponentSource::Path(fixture_wasm("echo")),
        root: None,
        bundled: false,
    };
    let loader = Loader {
        faux: Arc::clone(&provider),
        wasm: CoreLoader::new(
            runtime,
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
    let options = SessionOptions {
        source: SessionSource::InMemory,
        ..SessionOptions::default()
    };
    let runtime = AgentSessionRuntime::create(Arc::clone(&services), options)
        .await
        .unwrap();
    let session = runtime.session();
    assert!(
        session.active_tool_names().contains(&"echo".to_owned()),
        "{:?}",
        session.active_tool_names()
    );
    session
        .prompt("hi".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(20), session.wait_for_idle())
        .await
        .unwrap();

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
        vec![oino_ai::UserContent::text("from wasm")]
    );

    let kinds: Vec<String> = session
        .entries()
        .iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .filter(|e| e["customType"] == "echo.event")
        .map(|e| e["data"]["kind"].as_str().unwrap_or_default().to_owned())
        .collect();
    for expected in [
        "before_agent_start",
        "agent_start",
        "turn_start",
        "message_start",
        "message_end",
        "tool_call",
        "tool_execution_start",
        "tool_execution_end",
        "tool_result",
        "turn_end",
        "agent_end",
    ] {
        assert!(
            kinds.iter().any(|k| k == expected),
            "missing {expected} in {kinds:?}"
        );
    }
}
