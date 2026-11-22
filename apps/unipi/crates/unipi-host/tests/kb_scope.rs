//! KB-SCOPE: production host services, real WASM and CLI, recording mock provider.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
use futures_util::future::BoxFuture;
use indexmap::IndexMap;
use oino_core::{
    extensions::{
        api::{ExtensionInstance, ExtensionMode},
        host::HostApi,
    },
    runtime::{AgentSessionRuntime, SessionOptions, SessionSource},
    services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions},
    session::PromptOptions,
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

async fn turn(session: &oino_core::session::AgentSession) {
    session
        .prompt("hello".into(), PromptOptions::default())
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(30), session.wait_for_idle())
        .await
        .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
async fn real_host_provider_tools_follow_live_project_scope() {
    let base = PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/oino-work/kb-scope");
    std::fs::create_dir_all(&base).unwrap();
    let dir = tempfile::tempdir_in(&base).unwrap();
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
    let registry =
        oino_wasm_host::packages::load_dir(&dist.with_file_name("unipi.skill-registry"), true)
            .unwrap();
    assert!(dist.join("bin/unipi-kanboard").exists());
    let wasm =
        Arc::new(WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(&agent, &cwd))).unwrap());
    let faux = FauxProvider::new(vec![FauxResponse::Text("ok".into()); 6]);
    let loader = Loader {
        faux: faux.clone(),
        wasm: CoreLoader::new(
            wasm,
            vec![
                SourcedPackage {
                    package,
                    project: false,
                },
                SourcedPackage {
                    package: registry,
                    project: false,
                },
            ],
        ),
    };
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let services = Services::create(ServicesOptions {
        cwd: cwd.clone(),
        agent_dir: agent.clone(),
        docs_root: dir.path().join("docs"),
        mode: ExtensionMode::Desktop,
        has_ui: false,
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
            serde_json::from_value(
                json!({"extensionSettings":{"skills":{"exposure":{"mode":"all"}}}}),
            )
            .unwrap(),
        )),
    })
    .await
    .unwrap();
    assert!(services.extensions.get("unipi.kanboard").is_some());
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
    let cli = |args: &[&str]| {
        let out = std::process::Command::new(dist.join("bin/unipi-kanboard"))
            .args(args)
            .arg("--json")
            .current_dir(&cwd)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice::<serde_json::Value>(&out.stdout).unwrap()
    };
    let start = Instant::now();
    turn(&session).await;
    let project = cli(&["project", "add"]);
    let slug = project["slug"].as_str().unwrap();
    turn(&session).await;
    cli(&["project", "archive", slug]);
    turn(&session).await;
    cli(&["project", "unarchive", slug]);
    turn(&session).await;
    let requests = faux.requests();
    std::fs::write(
        base.join("provider-requests.json"),
        serde_json::to_vec_pretty(&requests).unwrap(),
    )
    .unwrap();
    for (i, expected) in [false, true, false, true].into_iter().enumerate() {
        let system = &requests[i]["context"]["messages"][0];
        let tools = system["toolsAdded"].as_array().unwrap();
        assert_eq!(
            tools.iter().any(|t| t["name"] == "kanboard"),
            expected,
            "turn {i}: {tools:?}"
        );
    }
    assert!(!requests[0]["context"]["messages"]
        .to_string()
        .contains("kanboard"));
    println!(
        "KB-SCOPE real host: four turns {:?}; kanboard schemas absent/present/absent/present",
        start.elapsed()
    );
    // A child doing work outside the parent cwd still resolves the parent's board.
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(&elsewhere).unwrap();
    let child = services.host.spawn_agent("test", &session.runner().call_context().with_kind(oino_core::extensions::api::CallKind::Tool),
        json!({"label": "scoped child", "cwd": elsewhere, "extensions": "inherit", "tools": {"extensionTools": ["kanboard"]}})).await.unwrap();
    let child_session = child.session().unwrap();
    turn(&child_session).await;
    assert!(child_session
        .active_tool_names()
        .contains(&"kanboard".to_owned()));
    cli(&["project", "archive", slug]);
    turn(&child_session).await;
    assert!(!child_session
        .active_tool_names()
        .contains(&"kanboard".to_owned()));
    println!("KB-SCOPE child cwd override follows root project's archive state");
    let mut measurements = Vec::new();
    for (state, active) in [("archived", false), ("active", true)] {
        if active {
            cli(&["project", "unarchive", slug]);
        }
        let mut ms = Vec::new();
        for _ in 0..50 {
            let start = Instant::now();
            turn(&session).await;
            ms.push(start.elapsed().as_secs_f64() * 1000.0);
        }
        ms.sort_by(f64::total_cmp);
        let record = json!({"state":state,"runs":50,"p50Ms":ms[25],"p95Ms":ms[47],"maxMs":ms[49]});
        println!("KB-SCOPE warm mock turn: {record}");
        measurements.push(record);
    }
    std::fs::write(
        base.join("turn-measurements.json"),
        serde_json::to_vec_pretty(&measurements).unwrap(),
    )
    .unwrap();
}
