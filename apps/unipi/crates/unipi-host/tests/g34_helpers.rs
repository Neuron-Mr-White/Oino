//! Real host runtime + bundled kanboard WASM at fresh-install autonomy (G3-4).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use futures_util::{future::BoxFuture, StreamExt};
use indexmap::IndexMap;
use oino_core::{
    events::{ClientEvent, OinoEvent},
    extensions::{
        api::{ExtensionInstance, ExtensionMode},
        host::HostApi,
    },
    runtime::{AgentSessionRuntime, SessionOptions, SessionSource},
    services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions},
    session::{
        permission::{ApprovalDecision, ApprovalResponse, Level},
        PromptOptions,
    },
    testing::{FauxComponent, FauxProvider, FauxResponse},
};
use oino_wasm_host::{
    core_bridge::{CoreLoader, SourcedPackage},
    HostConfig, WasmRuntime, WasmRuntimeOptions,
};
use serde_json::json;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

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
                vec![Arc::new(FauxComponent(self.faux.clone()))];
            out.extend(self.wasm.load(host, trusted).await);
            out
        })
    }
}
struct EnvGuard(Vec<(&'static str, Option<std::ffi::OsString>)>);
impl EnvGuard {
    fn set(vars: &[(&'static str, PathBuf)]) -> Self {
        Self(
            vars.iter()
                .map(|(key, value)| {
                    let old = std::env::var_os(key);
                    std::env::set_var(key, value);
                    (*key, old)
                })
                .collect(),
        )
    }
}
impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (key, old) in &self.0 {
            match old {
                Some(value) => std::env::set_var(key, value),
                None => std::env::remove_var(key),
            }
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
async fn board_turn_at_default_autonomy_has_zero_cards_and_agent_bash_still_asks() {
    let dir = tempfile::tempdir().unwrap();
    let agent = dir.path().join("agent");
    let cwd = dir.path().join("project");
    let home = dir.path().join("home");
    for p in [&agent, &cwd, &home] {
        std::fs::create_dir_all(p).unwrap();
    }
    let _env = EnvGuard::set(&[
        ("OINO_AGENT_DIR", agent.clone()),
        ("HOME", home),
        ("UNIPI_KANBOARD_HOME", agent.join("state/unipi.kanboard")),
    ]);
    let dist = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../../extensions/dist/unipi.kanboard")
        .canonicalize()
        .unwrap();
    let package = oino_wasm_host::packages::load_dir(&dist, true).unwrap();
    let wasm =
        Arc::new(WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(&agent, &cwd))).unwrap());
    let faux = FauxProvider::new(vec![FauxResponse::Text("board turn complete".into())]);
    let loader = Loader {
        faux: faux.clone(),
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
        cwd: cwd.clone(),
        agent_dir: agent.clone(),
        docs_root: dir.path().join("docs"),
        mode: ExtensionMode::Desktop,
        has_ui: true,
        flags: ResourceFlags {
            no_context_files: true,
            offline: true,
            ..Default::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(loader),
        approve: Some(false),
        models: Some(models),
        settings: Some(oino_settings::SettingsManager::in_memory(
            serde_json::Map::default(),
        )),
    })
    .await
    .unwrap();
    assert!(
        services.extensions.get("unipi.kanboard").is_some(),
        "kanboard must really load"
    );
    assert!(
        services
            .diagnostics
            .iter()
            .all(|d| !d.contains("init failed")),
        "{:?}",
        services.diagnostics
    );
    let registered = std::process::Command::new(dist.join("bin/unipi-kanboard"))
        .current_dir(&cwd)
        .args(["project", "add", "--root"])
        .arg(&cwd)
        .args(["--name", "G34 test", "--prefix", "GQA", "--json"])
        .output()
        .unwrap();
    assert!(
        registered.status.success(),
        "{}",
        String::from_utf8_lossy(&registered.stderr)
    );
    let runtime = AgentSessionRuntime::create(
        services.clone(),
        SessionOptions {
            source: SessionSource::New,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let session = runtime.session();
    assert_eq!(session.autonomy(), Level::Medium);
    session.set_approval_ui(true);
    let mut events = session.subscribe();
    let start = Instant::now();
    tokio::time::timeout(
        Duration::from_secs(30),
        session.prompt(
            "Read the board context and reply".into(),
            PromptOptions::default(),
        ),
    )
    .await
    .unwrap()
    .unwrap();
    tokio::time::timeout(Duration::from_secs(30), session.wait_for_idle())
        .await
        .unwrap();
    // Fence detached observers, including kanboard's agent_settled refresh.
    // A result-bearing hook fences earlier observer deliveries on this extension.
    tokio::time::timeout(
        Duration::from_secs(10),
        session
            .runner()
            .emit(json!({"type":"context", "messages":[]})),
    )
    .await
    .unwrap();
    let elapsed = start.elapsed();
    let seen = events.drain_now();
    assert!(
        !seen
            .iter()
            .any(|e| matches!(e, ClientEvent::Oino(OinoEvent::ApprovalRequest { .. }))),
        "{seen:?}"
    );
    assert!(!session.has_pending_approval());
    assert_eq!(faux.requests().len(), 1);
    println!("G34 real host board turn: zero approval cards, {elapsed:?}, Medium autonomy");
    let s = session.clone();
    let agent = tokio::spawn(async move {
        s.check_permission(
            "bash",
            json!({"command":"git push", "summary":"agent call", "riskLevel":"high"})
                .as_object()
                .unwrap(),
        )
        .await
    });
    let request = tokio::time::timeout(Duration::from_secs(5), async {
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
    session
        .approval_response(ApprovalResponse {
            request_id: request.request_id,
            decision: ApprovalDecision::Deny,
        })
        .unwrap();
    assert!(agent.await.unwrap().is_err());
}
