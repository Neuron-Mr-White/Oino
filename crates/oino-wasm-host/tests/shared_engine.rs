//! WP-C4 acceptance (core.md §5): two `Services` (two cwds, one process) share one wasmtime
//! engine, one epoch-ticker thread and one local-services hub; the same package compiles
//! (and pre-instantiates) once per process; the second service's cold start does not repay
//! the first's compile.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]
#![cfg(feature = "core")]

use std::sync::Arc;
use std::time::{Duration, Instant};

/// The registry counters are process-global: the two tests must not run concurrently
/// (an async-aware lock, held across awaits).
static SEQUENTIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oino_ai::JsonObject;
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::extensions::host::HostApi;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::session::PromptOptions;
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_extension_api::{ComponentSource, LoadedPackage};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::testing::{build_fixtures, fixture_manifest, fixture_wasm};
use oino_wasm_host::{EngineRegistry, EngineShared, HostConfig, WasmRuntime, WasmRuntimeOptions};

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

/// One `Services` (cwd `work`, agent dir `agent`) with the echo fixture loaded, like the
/// production `CliLoader` path: a `WasmRuntime` per cwd, everything heavy shared.
async fn services(
    dir: &std::path::Path,
    work: &str,
    faux: Arc<FauxProvider>,
) -> (Arc<Services>, Arc<WasmRuntime>) {
    let agent_dir = dir.join("agent");
    let cwd = dir.join(work);
    std::fs::create_dir_all(&cwd).unwrap();
    let runtime = Arc::new(
        WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(
            agent_dir.clone(),
            cwd.clone(),
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
        faux,
        wasm: CoreLoader::new(
            Arc::clone(&runtime),
            vec![SourcedPackage {
                package,
                project: false,
            }],
        ),
    };
    let models = oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let services = Services::create(ServicesOptions {
        cwd,
        agent_dir,
        docs_root: dir.join("docs"),
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
    (services, runtime)
}

/// The session of one service sees the echo tool and runs it through the shared engine.
async fn run_one_turn(services: &Arc<Services>) {
    let runtime = AgentSessionRuntime::create(
        Arc::clone(services),
        SessionOptions {
            source: SessionSource::InMemory,
            ..SessionOptions::default()
        },
    )
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
    assert_eq!(session.last_assistant_text().unwrap(), "done");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_services_share_one_engine_ticker_and_compile() {
    let _guard = SEQUENTIAL.lock().await;
    build_fixtures().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let turn = vec![
        FauxResponse::Text("done".into()),
        FauxResponse::Text("done".into()),
    ];

    let engines_before = EngineRegistry::engines_created();
    let tickers_before = EngineRegistry::ticker_threads_started();

    // Service 1 (cwd work1): pays engine + ticker + one compile of the echo component.
    let t0 = Instant::now();
    let faux1 = FauxProvider::new(turn.clone());
    let (services1, runtime1) = services(dir.path(), "work1", faux1).await;
    let first = t0.elapsed();

    // This test binary is the only engine user at this point (fixture builds done), so the
    // counters are exact: exactly one engine and one ticker for the whole process so far.
    assert_eq!(
        EngineRegistry::engines_created() - engines_before,
        1,
        "service 1 creates the one engine"
    );
    assert_eq!(
        EngineRegistry::ticker_threads_started() - tickers_before,
        1,
        "one epoch-ticker thread per process"
    );

    // Service 2 (cwd work2, same agent dir): resolves the same engine, hub and compile
    // caches — no second engine, no second ticker, no second compile.
    let t1 = Instant::now();
    let faux2 = FauxProvider::new(turn);
    let (services2, runtime2) = services(dir.path(), "work2", faux2).await;
    let second = t1.elapsed();

    assert_eq!(
        EngineRegistry::engines_created() - engines_before,
        1,
        "the second service must not create another engine"
    );
    assert_eq!(
        EngineRegistry::ticker_threads_started() - tickers_before,
        1,
        "the second service must not start another ticker thread"
    );
    assert!(
        Arc::ptr_eq(runtime1.engine(), runtime2.engine()),
        "both services run on one shared engine"
    );
    assert!(
        Arc::ptr_eq(
            runtime1.local_services().unwrap(),
            runtime2.local_services().unwrap()
        ),
        "both services share one local-services hub per agent dir"
    );
    assert_eq!(
        runtime1.engine().compiles(),
        1,
        "the echo package compiled once for both services"
    );
    assert_eq!(runtime1.engine().pre_instantiations(), 1);

    // Both services work on the shared engine.
    run_one_turn(&services1).await;
    run_one_turn(&services2).await;
    assert_eq!(runtime1.engine().compiles(), 1, "turns compile nothing");

    // Cold start: the second service must not repay the first's compile (it skips engine
    // setup and compilation; generous bound, the exact numbers are a recorded measurement).
    assert!(
        second < first,
        "second service cold start ({second:?}) must be cheaper than the first ({first:?})"
    );
}

/// An explicitly passed engine bypasses the registry (test isolation, E-ENGINE).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn explicit_engine_option_isolates_from_the_registry() {
    let _guard = SEQUENTIAL.lock().await;
    build_fixtures().unwrap();
    let dir = tempfile::tempdir().unwrap();
    let engines_before = EngineRegistry::engines_created();
    let tickers_before = EngineRegistry::ticker_threads_started();

    // A standalone engine (no registry entry) handed to the runtime directly.
    let engine = Arc::new(EngineShared::new(None).unwrap());
    let options = WasmRuntimeOptions {
        engine: Some(Arc::clone(&engine)),
        ..WasmRuntimeOptions::new(HostConfig::new(
            dir.path().join("agent"),
            dir.path().join("w"),
        ))
    };
    let runtime = WasmRuntime::new(options).unwrap();

    // Exactly the standalone engine was created — nothing resolved from the registry.
    assert_eq!(EngineRegistry::engines_created() - engines_before, 1);
    assert_eq!(EngineRegistry::ticker_threads_started() - tickers_before, 1);
    assert!(Arc::ptr_eq(runtime.engine(), &engine));
    assert!(
        runtime.local_services().is_some(),
        "hub lookup is unaffected"
    );
    // Exercise the isolated engine for at least one epoch tick.
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert_eq!(runtime.engine().compiles(), 0);
}
