//! [sidebar-data] Sidebar listing cost on a real session store (owner: ~10 s before the
//! Task list shows rows with ~1944 archived sessions). Ignored by default; point
//! `SIDEBAR_PERF_AGENT_DIR` at a COPY of an agent dir (`sessions/` + `trash/`) and run
//! `cargo test -p unipi-host --test sidebar_perf -- --ignored --nocapture`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::print_stdout,
    reason = "a measurement harness"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;
use unipi_host::sessions::hub::{HostHubOptions, SessionHub, SessionsQuery};

fn hub(agent_dir: PathBuf, host: PathBuf) -> Arc<SessionHub> {
    let provider = FauxProvider::new(vec![]);
    SessionHub::create(HostHubOptions {
        cwd: agent_dir.clone(),
        agent_dir,
        docs_root: None,
        session_dir: None,
        mode: ExtensionMode::Desktop,
        has_ui: false,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        loader: Arc::new(StaticLoader(vec![Arc::new(FauxComponent(provider))
            as Arc<dyn oino_core::extensions::api::ExtensionInstance>])),
        models: Some(ModelRegistry::in_memory(Arc::new(
            oino_models::MapEnv::default(),
        ))),
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: Some(host),
    })
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "measurement on a copy of a real store (SIDEBAR_PERF_AGENT_DIR)"]
async fn sidebar_listing_cost() {
    let Ok(dir) = std::env::var("SIDEBAR_PERF_AGENT_DIR") else {
        println!("SIDEBAR_PERF_AGENT_DIR not set");
        return;
    };
    let host = tempfile::tempdir().unwrap();
    let hub = hub(PathBuf::from(dir), host.path().to_path_buf());
    for round in 0..2 {
        let t = Instant::now();
        let list = hub.list().await.unwrap();
        let list_ms = t.elapsed().as_millis();
        let bytes = serde_json::to_vec(&list).unwrap().len();
        let t = Instant::now();
        let archived = hub.archived().await.unwrap();
        let archived_ms = t.elapsed().as_millis();
        let archived_bytes = serde_json::to_vec(&archived).unwrap().len();
        let t = Instant::now();
        let page = hub
            .query(&SessionsQuery {
                limit: Some(50),
                ..SessionsQuery::default()
            })
            .await
            .unwrap();
        let query_ms = t.elapsed().as_millis();
        println!(
            "round {round}: list {} cards {list_ms} ms ({bytes} B) | archived {} cards {archived_ms} ms ({archived_bytes} B) | query total {} {query_ms} ms",
            list.len(),
            archived.len(),
            page.total
        );
    }
}
