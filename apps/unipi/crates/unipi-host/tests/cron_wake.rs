//! WP-H8 acceptance (spec-host §9.8): a cron run's wake link opens its chat in the session
//! hub — takeover semantics, no socket-poking a live pi — and prompts the wake message.
//!
//! The faux provider answers the seed prompt and then the wake prompt; a fake
//! `$OINO_CRON_BIN` is the cron run itself (it only has to exit 0).
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::await_holding_lock,
    reason = "tests assert by unwrapping; the binary override is process-global"
)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use oino_client::protocol::{AgentCommand, RpcCommand, RpcResponseData};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use unipi_cron::model::SessionLink;
use unipi_cron::{scheduler::Scheduler, store, CronPaths};
use unipi_host::sessions::hub::{HostHubOptions, HubSession, SessionHub};

static ENV_LOCK: Mutex<()> = Mutex::new(());

struct Fixture {
    _dir: tempfile::TempDir,
    agent_dir: std::path::PathBuf,
    hub: Arc<SessionHub>,
}

fn hub_fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("seed answer".into()),
        FauxResponse::Text("wake answer".into()),
    ]);
    let loader = StaticLoader(vec![Arc::new(FauxComponent(Arc::clone(&provider)))
        as Arc<dyn oino_core::extensions::api::ExtensionInstance>]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.path().to_path_buf(),
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
    .unwrap();
    Fixture {
        _dir: dir,
        agent_dir,
        hub,
    }
}

async fn last_text(session: &HubSession) -> String {
    let data: RpcResponseData = session
        .send(RpcCommand::Agent(AgentCommand::GetLastAssistantText))
        .await
        .unwrap();
    data.and_then(|value| value["text"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

async fn wait_text(session: &HubSession, want: &str) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = last_text(session).await;
        if text.contains(want) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "session never answered {want:?} (last: {text:?})"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn fake_oino(dir: &Path, script: &str) -> Option<String> {
    let bin = dir.join("oino");
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let previous = std::env::var("OINO_CRON_BIN").ok();
    std::env::set_var("OINO_CRON_BIN", &bin);
    previous
}

fn restore_oino(previous: Option<String>) {
    match previous {
        Some(value) => std::env::set_var("OINO_CRON_BIN", value),
        None => std::env::remove_var("OINO_CRON_BIN"),
    }
}

/// A job linking the session finishes → the hub opens that session and prompts the wake
/// text; the faux provider answers, and the outcome is recorded on the run.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_finished_run_wakes_its_linked_session_through_the_hub() {
    let _guard = ENV_LOCK.lock().unwrap();
    let fixture = hub_fixture();
    let hub = &fixture.hub;

    // The chat to wake: a live hub session that has answered once.
    let session = hub.create_session(None, Some("wake target")).await.unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
        }))
        .await
        .unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: "seed".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    wait_text(&session, "seed answer").await;
    let session_id = session.session_id();

    // The scheduler with the wake sink installed, and a job linked to that session.
    let cron_paths = CronPaths::new(&fixture.agent_dir);
    let scheduler = unipi_host::cron_wake::install(
        Scheduler::with_paths(cron_paths.clone()),
        Arc::clone(hub),
        cron_paths.clone(),
        None,
    );
    let cwd = fixture._dir.path().to_string_lossy().into_owned();
    let job = unipi_cron::model::Job {
        id: "waker".into(),
        name: "Daily digest".into(),
        prompt: "do the thing".into(),
        schedule: "at:2000-01-01T00:00:00Z".into(),
        tz: "UTC".into(),
        cwd,
        model: None,
        thinking: None,
        enabled: true,
        time_limit_s: 10,
        cost_cap_usd: None,
        overlap: unipi_cron::model::OverlapPolicy::default(),
        missed: unipi_cron::model::MissedPolicy::default(),
        notify: None,
        session_link: Some(SessionLink {
            session_file: None,
            session_id: Some(session_id.clone()),
        }),
        created: 0,
        updated: 0,
    };
    store::save_jobs(&cron_paths, std::slice::from_ref(&job)).unwrap();
    let bin_dir = tempfile::tempdir().unwrap();
    let previous = fake_oino(bin_dir.path(), "#!/bin/sh\necho done\nexit 0\n");

    let run_id = scheduler.run_now(&job).await.unwrap();
    // The run itself finishes, and its wake reaches the hub session.
    let deadline = Instant::now() + Duration::from_secs(20);
    let mut outcome = None;
    while Instant::now() < deadline {
        if let Ok(Some(run)) = store::load_run(&cron_paths, "waker", &run_id) {
            if let Some(wake) = run.wake.clone() {
                outcome = Some(wake);
                break;
            }
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    restore_oino(previous);
    assert_eq!(
        outcome.as_deref(),
        Some("sent"),
        "the wake should be recorded as sent on the run"
    );
    wait_text(&session, "wake answer").await;
    // The prompt the hub carried names the job and the run.
    let card = hub.card(&session_id).await.unwrap().expect("the card");
    assert!(card.message_count >= 4, "messages: {}", card.message_count);
}
