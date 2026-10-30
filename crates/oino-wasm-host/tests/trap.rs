//! X-TRAP end-to-end (core spec §2.3, WP-C5): a real trapping component (the
//! `panic-probe` fixture) disables itself for the session that trapped only; a second
//! session in the same process keeps dispatching through the same shared instance.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]
#![cfg(feature = "core")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::extensions::api::ExtensionInstance;
use oino_core::extensions::host::HostApi;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::testing::{build_fixtures, fixture_wasm};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

const PROBE: &str = "oino.fixture.panic-probe";

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

/// Two sessions over one `Services` (one shared wasm instance of the panic-probe fixture).
async fn two_sessions() -> (tempfile::TempDir, AgentSessionRuntime, AgentSessionRuntime) {
    build_fixtures().unwrap();
    // Stage the built fixture as a package (extension.json + component.wasm).
    let dir = tempfile::Builder::new()
        .prefix("oino-trap-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let agent = dir.path().join("agent");
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(agent.join("cache")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let stage = dir.path().join("stage/panic-probe");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::copy(
        repo_fixture_dir().join("extension.json"),
        stage.join("extension.json"),
    )
    .unwrap();
    std::fs::copy(fixture_wasm("panic-probe"), stage.join("component.wasm")).unwrap();
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("oino-wasmtime-cache");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&cache, agent.join("cache/wasmtime")).unwrap();

    let provider = FauxProvider::new(vec![FauxResponse::Text("unused".into())]);
    let wasm =
        Arc::new(WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(&agent, &cwd))).unwrap());
    let packages = vec![SourcedPackage {
        package: oino_wasm_host::packages::load_dir(&stage, true).unwrap(),
        project: false,
    }];
    let loader = Loader {
        faux: Arc::clone(&provider),
        wasm: CoreLoader::new(wasm, packages),
    };
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let services = Services::create(ServicesOptions {
        cwd: cwd.clone(),
        agent_dir: agent.clone(),
        docs_root: dir.path().join("docs"),
        mode: oino_core::extensions::api::ExtensionMode::Print,
        has_ui: false,
        flags: ResourceFlags {
            no_context_files: true,
            offline: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(loader),
        approve: Some(true),
        models: Some(models),
        settings: Some(oino_settings::SettingsManager::in_memory(
            serde_json::from_value(json!({})).unwrap(),
        )),
    })
    .await
    .unwrap();
    assert!(
        services
            .diagnostics
            .iter()
            .all(|d| !d.contains("init failed")),
        "{:?}",
        services.diagnostics
    );
    let context = Arc::new(SessionToolContext::default());
    let snapshot = services.settings.lock().unwrap().snapshot();
    let tools = builtin_tool_entries(&cwd, &agent, &snapshot, Arc::clone(&context));
    let runtime = |services: Arc<Services>| async {
        AgentSessionRuntime::create(
            services,
            SessionOptions {
                source: SessionSource::InMemory,
                builtin_tools: tools.clone(),
                ..SessionOptions::default()
            },
        )
        .await
        .unwrap()
    };
    let a = runtime(Arc::clone(&services)).await;
    let b = runtime(Arc::clone(&services)).await;
    context.bind(&a.session());
    (dir, a, b)
}

fn repo_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/fixtures/panic-probe")
}

/// Waits (bounded) until `f` holds.
async fn until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while !f() {
        assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Collecting event watcher: nothing published after subscription is ever lost.
struct Events {
    stream: oino_core::hub::EventStream,
    seen: Vec<Value>,
}

impl Events {
    fn new(session: &oino_core::session::AgentSession) -> Self {
        Self {
            stream: session.subscribe(),
            seen: Vec::new(),
        }
    }

    fn pump(&mut self) {
        self.seen.extend(
            self.stream
                .drain_now()
                .into_iter()
                .map(|event| serde_json::to_value(&event).unwrap()),
        );
    }

    fn kind(&self, kind: &str) -> Vec<&Value> {
        self.seen.iter().filter(|v| v["type"] == kind).collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn trap_disables_extension_for_session_a_only_and_emits_both_events() {
    let (_dir, a, b) = two_sessions().await;
    let session_a = a.session();
    let session_b = b.session();
    let mut events = Events::new(&session_a);
    assert!(
        session_a.runner().has_handlers("tool_call"),
        "the fixture subscribes to tool_call"
    );

    // Session A: the gate call traps; the extension disables itself for A only.
    session_a
        .runner()
        .emit(json!({"type": "tool_call", "toolName": "trap"}))
        .await;
    until("extension_disabled on A", || {
        events.pump();
        !events.kind("extension_disabled").is_empty()
    })
    .await;
    let disabled = events.kind("extension_disabled")[0].clone();
    assert_eq!(disabled["extensionId"], json!(PROBE));
    assert_eq!(disabled["sessionId"], json!(session_a.session_id()));
    assert_eq!(disabled["reason"], json!("trap"));
    assert!(
        events
            .kind("extension_error")
            .iter()
            .any(|e| e["event"] == "tool_call"
                && e["extensionPath"].to_string().contains("panic-probe")),
        "extension_error accompanies the disable: {:?}",
        events.seen
    );
    assert!(session_a.runner().is_disabled(PROBE));
    assert!(!session_a.runner().has_handlers("tool_call"));

    // Further dispatch on A skips the extension entirely (no second trap).
    session_a
        .runner()
        .emit(json!({"type": "tool_call", "toolName": "trap"}))
        .await;

    // Session B (same process, same instance) still dispatches: the restarted instance
    // records the seen entry in B's session.
    session_b
        .runner()
        .emit(json!({"type": "tool_call", "toolName": "harmless"}))
        .await;
    assert!(!session_b.runner().is_disabled(PROBE));
    assert!(session_b.runner().has_handlers("tool_call"));
    until("panic-probe.seen entry in B", || {
        session_b.entries().iter().any(|entry| {
            serde_json::to_value(entry).unwrap()["customType"] == json!("panic-probe.seen")
        })
    })
    .await;

    // A never recorded anything: its entries have no panic-probe.seen.
    assert!(
        !session_a.entries().iter().any(|entry| {
            serde_json::to_value(entry).unwrap()["customType"] == json!("panic-probe.seen")
        }),
        "session A stopped dispatching to the fixture"
    );
}
