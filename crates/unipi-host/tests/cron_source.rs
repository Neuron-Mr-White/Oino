//! WP-A23 acceptance (app spec §3.14 "Cron source", Z-112): a session a cron run owns
//! carries `source: "cron"` on its card — the sidebar's clock badge and the "Sent by
//! cron" bubble label data. A run's Oino session is identified the way the scheduler
//! records it (`exec.rs`): the run id *is* the session id, and the run record names the
//! session file. The run records here are exactly what the scheduler writes after a
//! fire (`store::save_run`), so this covers "Run now → the run's session appears with a
//! `source:"cron"` card" at the hub level; the op-level fire is WP-H8's
//! `tests/cron_ops.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_client::protocol::{AgentCommand, RpcCommand};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use unipi_cron::model::{Job, Run, RunStatus};
use unipi_cron::{store, CronPaths};
use unipi_host::sessions::hub::{HostHubOptions, HubSession, SessionHub};

struct Fixture {
    _dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
    cron: CronPaths,
}

impl Fixture {
    /// A fresh hub over the same dirs — the cron index starts empty (the 5 s read
    /// cache otherwise hides a run that just landed).
    fn fresh_hub(&self) -> Arc<SessionHub> {
        hub_over(self._dir.path())
    }
}

/// A hub whose sessions are answered by a faux provider, over an agent dir the cron
/// store also lives in (the same layout `serve` pins: `<data>/agent`, `<data>/sessions`).
fn hub_over(dir: &std::path::Path) -> Arc<SessionHub> {
    let agent_dir = dir.join("agent");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("digest done".into())]);
    let loader = StaticLoader(vec![Arc::new(FauxComponent(provider))]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    SessionHub::create(HostHubOptions {
        cwd: dir.to_path_buf(),
        agent_dir: agent_dir.clone(),
        docs_root: None,
        session_dir: Some(sessions),
        mode: ExtensionMode::Desktop,
        has_ui: false,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        loader: Arc::new(loader),
        models: Some(models),
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        host_data_dir: None,
    })
    .unwrap()
}

fn hub() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
    let hub = hub_over(dir.path());
    Fixture {
        _dir: dir,
        hub,
        cron: CronPaths::new(agent_dir),
    }
}

/// Creates a session, selects the faux model and prompts once so the file is
/// written; answers `(session, file)` once the card is listed (`hub.list()` reads
/// records, and a record exists once the first message settled — like
/// `wait_card` in `tests/sessions_hub.rs`).
async fn session_with_file(hub: &Arc<SessionHub>) -> (HubSession, String) {
    let session = hub.create_session(None, None).await.unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
        }))
        .await
        .unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: "summarize the inbox".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    let id = session.session_id();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(card) = hub
            .list()
            .await
            .unwrap()
            .into_iter()
            .find(|card| card.session_id == id)
        {
            return (session, card.file.display().to_string());
        }
        assert!(
            Instant::now() < deadline,
            "the session never became a record"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn job(id: &str) -> Job {
    Job::new(id, id, "prompt", "0 0 9 * * *", "UTC", "/tmp")
}

fn run(id: &str, job_id: &str, session_path: Option<&str>) -> Run {
    Run {
        id: id.to_owned(),
        job_id: job_id.to_owned(),
        status: RunStatus::Ok,
        fired_at: 1,
        started_at: Some(1),
        ended_at: Some(2),
        exit_code: Some(0),
        summary: None,
        reason: None,
        session_path: session_path.map(str::to_owned),
        cost_usd: None,
        missed: false,
        pid: None,
        notify_errors: Vec::new(),
        wake: None,
    }
}

#[tokio::test]
async fn a_run_owned_session_carries_source_cron() {
    let fixture = hub();
    let (session, file) = session_with_file(&fixture.hub).await;
    let id = session.session_id();

    // Before any run: the card has no source.
    let plain = fixture
        .hub
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|card| card.session_id == id)
        .unwrap();
    assert_eq!(plain.source, None);

    // The scheduler's record after a fire: the run id is the session id and the run
    // names the session file.
    store::save_job(&fixture.cron, &job("digest")).unwrap();
    store::save_run(&fixture.cron, &run(&id, "digest", Some(&file))).unwrap();

    // A fresh hub (the read cache is 5 s; `serve` reads at startup like this).
    let card = fixture
        .fresh_hub()
        .list()
        .await
        .unwrap()
        .into_iter()
        .find(|card| card.session_id == id)
        .unwrap();
    assert_eq!(card.source.as_deref(), Some("cron"), "by run id");

    // A second session matched only through its file (a fork of the run's session, say).
    let (other, other_file) = {
        let s = fixture.hub.create_session(None, None).await.unwrap();
        s.send(RpcCommand::Agent(AgentCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
        }))
        .await
        .unwrap();
        s.send(RpcCommand::Agent(AgentCommand::Prompt {
            message: "second".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
        let id = s.session_id();
        let deadline = Instant::now() + Duration::from_secs(20);
        loop {
            if let Some(card) = fixture
                .hub
                .list()
                .await
                .unwrap()
                .into_iter()
                .find(|card| card.session_id == id)
            {
                break (s, card.file.display().to_string());
            }
            assert!(Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    store::save_run(
        &fixture.cron,
        &run("digest-0182", "digest", Some(&other_file)),
    )
    .unwrap();
    let cards = fixture.fresh_hub().list().await.unwrap();
    let by_file = cards
        .iter()
        .find(|card| card.session_id == other.session_id())
        .unwrap();
    assert_eq!(by_file.source.as_deref(), Some("cron"), "by session file");
}

#[tokio::test]
async fn sessions_list_over_the_wire_keeps_the_source() {
    let fixture = hub();
    let (session, file) = session_with_file(&fixture.hub).await;
    store::save_job(&fixture.cron, &job("digest")).unwrap();
    store::save_run(
        &fixture.cron,
        &run(&session.session_id(), "digest", Some(&file)),
    )
    .unwrap();
    let value = serde_json::to_value(fixture.fresh_hub().list().await.unwrap()).unwrap();
    let card = value
        .as_array()
        .unwrap()
        .iter()
        .find(|card| card["sessionId"] == serde_json::json!(session.session_id()))
        .unwrap();
    assert_eq!(card["source"], serde_json::json!("cron"));
}
