#![allow(clippy::print_stderr, reason = "diagnostic test timing output")]
//! Core integration: the echo component loaded through `core_bridge::CoreLoader` into an
//! `oino-core` session sees client events, serves tools and blocks tool calls.

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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn guest_event_dialog_reenters_same_extension_prompt_hooks() {
    use oino_ai::JsonValue;
    use oino_core::events::{ExtensionErrorEvent, OinoEvent};
    use oino_core::extensions::{
        api::*,
        host::{HostError, HostOp},
        runner::*,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct Host {
        runner: Mutex<Option<ExtensionRunner>>,
        effects: Mutex<Vec<String>>,
    }
    impl HostApi for Host {
        fn call<'a>(
            &'a self,
            _: &'a str,
            _: &'a CallContext,
            op: HostOp,
        ) -> BoxFuture<'a, Result<JsonValue, HostError>> {
            Box::pin(async move {
                if matches!(op, HostOp::UiDialog { .. }) {
                    let runner = self.runner.lock().unwrap().clone().unwrap();
                    runner.emit(json!({"type":"ui_prompt_start"})).await;
                    runner.emit(json!({"type":"ui_prompt_end"})).await;
                    Ok(json!({"confirmed":true}))
                } else {
                    self.effects.lock().unwrap().push(format!("{op:?}"));
                    Ok(json!("entry-id"))
                }
            })
        }
    }
    struct Sink;
    impl ErrorSink for Sink {
        fn report(&self, e: ExtensionErrorEvent) {
            panic!("hook failed: {e:?}");
        }
        fn disabled(&self, _: OinoEvent) {
            panic!("extension disabled");
        }
    }
    build_fixtures().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let runtime = Arc::new(
        WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
            dir.path().join("agent"),
            dir.path(),
        )))
        .unwrap(),
    );
    let loader = CoreLoader::new(
        runtime,
        vec![SourcedPackage {
            package: LoadedPackage {
                manifest: fixture_manifest("panic-probe", None).unwrap(),
                component: ComponentSource::Path(fixture_wasm("panic-probe")),
                root: None,
                bundled: false,
            },
            project: false,
        }],
    );
    let host = Arc::new(Host::default());
    let instance = loader.load(host.clone(), true).await.remove(0);
    let registration = instance.init(CallContext::default()).await.unwrap();
    let runner = ExtensionRunner::new(
        Arc::new(ExtensionSet::new(vec![Arc::new(LoadedExtension::new(
            instance,
            registration,
        ))])),
        CallContext::default(),
        Arc::new(Sink),
        IndexMap::new(),
    );
    *host.runner.lock().unwrap() = Some(runner.clone());
    let started = std::time::Instant::now();
    tokio::time::timeout(
        Duration::from_millis(750),
        runner.emit_tool_call(json!({"type":"tool_call", "toolName":"dialog"})),
    )
    .await
    .unwrap();
    eprintln!(
        "guest_dialog_prompt_hooks_ms={}",
        started.elapsed().as_millis()
    );
    let effects = host.effects.lock().unwrap();
    let writes: Vec<_> = effects
        .iter()
        .filter(|e| e.contains("AppendEntry"))
        .collect();
    assert_eq!(writes.len(), 3, "{effects:?}");
    assert!(writes[0].contains("prompt-start"));
    assert!(writes[1].contains("prompt-end"));
    assert!(writes[2].contains("panic-probe.seen"));
    drop(effects);
    *host.runner.lock().unwrap() = None;
}
