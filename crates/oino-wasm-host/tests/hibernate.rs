//! E-HIBERNATE end-to-end with a real guest (core spec §2.2, WP-C8): the
//! `hibernate-probe` fixture runs inside a file-persisted child, observes
//! `session_shutdown{reason:"hibernate"}` when the child hibernates and records it as a
//! session entry in the child's own log; `prompt` wakes the child and continues the same
//! session id.

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
use oino_core::extensions::api::{CallKind, ExtensionInstance, ExtensionMode};
use oino_core::extensions::host::HostApi;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::testing::{build_fixtures, fixture_wasm};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

const PROBE: &str = "oino.fixture.hibernate-probe";

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

/// One file-persisted root session whose children run the hibernate-probe fixture.
async fn setup() -> (tempfile::TempDir, Arc<Services>, AgentSessionRuntime) {
    build_fixtures().unwrap();
    let dir = tempfile::Builder::new()
        .prefix("oino-hibernate-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let agent = dir.path().join("agent");
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(agent.join("cache")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let stage = dir.path().join("stage/hibernate-probe");
    std::fs::create_dir_all(&stage).unwrap();
    std::fs::copy(
        repo_fixture_dir().join("extension.json"),
        stage.join("extension.json"),
    )
    .unwrap();
    std::fs::copy(
        fixture_wasm("hibernate-probe"),
        stage.join("component.wasm"),
    )
    .unwrap();
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("oino-wasmtime-cache");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&cache, agent.join("cache/wasmtime")).unwrap();

    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first reply".into()),
        FauxResponse::Text("second reply".into()),
    ]);
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
        mode: ExtensionMode::Print,
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
            json!({
                "extensionSettings": {"subagents": {"hibernateAfterSec": 1}}
            })
            .as_object()
            .unwrap()
            .clone(),
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
    let rt = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source: SessionSource::New,
            builtin_tools: tools,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    context.bind(&rt.session());
    (dir, services, rt)
}

fn repo_fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/fixtures/hibernate-probe")
}

/// Parsed JSONL lines of a session file.
fn read_file(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// A real guest observes hibernation and survives it: `session_shutdown{hibernate}` is
/// recorded in the child's log by the fixture, `prompt` wakes the same session id, and
/// the final close records the `quit` reason.
#[tokio::test(flavor = "multi_thread")]
async fn fixture_observes_hibernate_shutdown_and_wake_continues_the_session() {
    let (_dir, services, rt) = setup().await;
    let parent = rt.session();
    let cx = parent.runner().call_context().with_kind(CallKind::Tool);
    let child = services
        .host
        .spawn_agent(
            PROBE,
            &cx,
            json!({
                "label": "guest-sleeper",
                "persistence": {"file": {}},
                "extensions": {"only": [PROBE]},
                "tools": {"builtin": []},
            }),
        )
        .await
        .unwrap();
    assert!(
        child
            .session()
            .expect("awake")
            .runner()
            .has_handlers("session_shutdown"),
        "the fixture subscribes session_shutdown in the child"
    );
    assert_eq!(
        child.prompt("task one".into(), None).await.unwrap(),
        "started"
    );
    child.wait_idle().await;
    let id = child.id().to_owned();
    let file = child.session_file().unwrap();

    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while !child.is_hibernated() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "the child never hibernated"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_eq!(child.state(), "hibernated");

    // The real guest recorded the hibernate shutdown in the child's own session log.
    let lines = read_file(&file);
    let shutdowns: Vec<&Value> = lines
        .iter()
        .filter(|l| l["customType"] == json!("hibernate-probe.shutdown"))
        .collect();
    assert_eq!(
        shutdowns.len(),
        1,
        "exactly one recorded shutdown: {lines:?}"
    );
    assert_eq!(shutdowns[0]["data"]["reason"], json!("hibernate"));

    // Wake by prompt: same agent/session id, history continues, fixture still subscribed.
    assert_eq!(
        child.prompt("task two".into(), None).await.unwrap(),
        "started"
    );
    assert_eq!(child.id(), id);
    assert_eq!(child.session().expect("awake").session_id(), id);
    child.wait_idle().await;
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(
        text.contains("task one") && text.contains("task two"),
        "the resumed log keeps both runs: {text}"
    );
    assert!(text.contains("first reply") && text.contains("second reply"));
    child.close(Duration::ZERO).await;
    // The final close emitted `session_shutdown{quit}` through the same fixture.
    let lines = read_file(&file);
    let reasons: Vec<Value> = lines
        .iter()
        .filter(|l| l["customType"] == json!("hibernate-probe.shutdown"))
        .map(|l| l["data"]["reason"].clone())
        .collect();
    assert_eq!(reasons, vec![json!("hibernate"), json!("quit")]);
}
