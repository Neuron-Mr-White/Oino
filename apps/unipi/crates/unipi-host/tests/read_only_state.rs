//! Read-only RPCs must not flip a session's card to `Working` (v2 QA defect):
//! `HubSession::send`/`send_with_id` used to start the run supervisor for *every*
//! command, so the snapshot's `get_state`/`get_entries`/`ui`/`topics` and the
//! accordion-expand fetches turned every open into a "Working…" blip and then
//! marked unfocused sessions `unread` when the supervisor retired. Only commands
//! that start agent work may supervise; anything else supervises only when its
//! reply says a run started (`{disposition: "started" | "queued"}`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use oino_client::protocol::{AgentCommand, RpcCommand};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use serde_json::json;
use unipi_host::sessions::card::HubState;
use unipi_host::sessions::hub::{HostHubOptions, HubEvent, SessionHub};
use unipi_host::sessions::stream::SessionService;

struct Fixture {
    _dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
    /// Kept alive: the streams' feeds live on it.
    _service: Arc<SessionService>,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    let loader = StaticLoader(vec![Arc::new(FauxComponent(Arc::clone(&provider)))]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir,
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
        services_cap: None,
        host_data_dir: None,
    })
    .unwrap();
    let service = SessionService::new(Arc::clone(&hub));
    Fixture {
        _dir: dir,
        hub,
        _service: service,
    }
}

/// Creates a session with one faux exchange and returns its id once idle.
async fn session_with_a_message(hub: &Arc<SessionHub>) -> String {
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
            message: "hello".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    let id = session.session_id();
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(card)) = hub.card(&id).await {
            if !matches!(card.state, HubState::Working) {
                break;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "session never went idle"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    id
}

/// The read-only RPCs the app sends on every open and accordion expand.
const READ_ONLY: &[RpcCommand] = &[
    RpcCommand::Agent(AgentCommand::GetState),
    RpcCommand::Agent(AgentCommand::GetEntries { since: None }),
    RpcCommand::Agent(AgentCommand::GetCommands),
    RpcCommand::Agent(AgentCommand::GetAvailableModels),
    RpcCommand::Agent(AgentCommand::GetSessionStats),
    RpcCommand::Oino(oino_client::protocol::OinoCommand::UiSnapshot),
    RpcCommand::Oino(oino_client::protocol::OinoCommand::GetShortcuts),
    RpcCommand::Oino(oino_client::protocol::OinoCommand::Topics {
        subscribe: Vec::new(),
        unsubscribe: Vec::new(),
    }),
];

/// Opening a session and running the read-only RPCs of an accordion expand never
/// emits a `Working` card and never marks the session unread.
#[tokio::test]
async fn read_only_commands_never_mark_a_session_working_or_unread() {
    let fixture = fixture().await;
    let id = session_with_a_message(&fixture.hub).await;

    // The fixture's own prompt may have marked the fresh session unread (Z-86 by
    // design); clear it so the assertion measures only what the read-only phase does.
    // Nothing focuses the session (no events_open Focus), so a supervisor retiring
    // would mark it unread again — that is exactly what must not start.
    fixture
        .hub
        .meta_set(
            &id,
            unipi_proto::session::SessionMetaPatch {
                unread: Some(false),
                ..unipi_proto::session::SessionMetaPatch::default()
            },
        )
        .await
        .unwrap();
    let mut events = fixture.hub.subscribe();
    let opened = fixture.hub.open(&id).await.unwrap();
    for command in READ_ONLY {
        opened.send(command.clone()).await.unwrap();
    }
    // `_oino/render` (the transcript row render the timeline does on expand).
    opened
        .send(RpcCommand::Oino(
            oino_client::protocol::OinoCommand::Render {
                target: "tool".into(),
                key: "text".into(),
                entry_id: None,
                turn_entry_id: None,
                turn_index: None,
                running: false,
                density: None,
                expanded: true,
                width: None,
                data: json!({"text": "hi"}),
            },
        ))
        .await
        .unwrap(); // renders may answer null; the card must not move either way

    let mut working_events = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(2);
    while std::time::Instant::now() < deadline {
        match events.try_recv() {
            Ok(HubEvent::SessionChanged { card }) if card.state == HubState::Working => {
                working_events += 1;
            }
            Ok(_) => {}
            Err(_) => break,
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        working_events, 0,
        "read-only commands must not mark the session Working"
    );

    let card = fixture.hub.card(&id).await.unwrap().unwrap();
    assert_eq!(card.state, HubState::Idle, "card stays idle");
    assert!(!card.unread, "no false unread from read-only commands");
    drop(opened);
}

/// A prompt still supervises: the card flips to `Working` while the run is in
/// flight (and the reply's `started` disposition rides the same path).
#[tokio::test]
async fn a_prompt_still_marks_the_session_working() {
    let fixture = fixture().await;
    let id = session_with_a_message(&fixture.hub).await;

    let mut events = fixture.hub.subscribe();
    let opened = fixture.hub.open(&id).await.unwrap();
    let reply = opened
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: "again".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    assert_eq!(
        reply.as_ref().unwrap()["disposition"],
        "started",
        "the faux provider starts a run: {reply:?}"
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        match events.try_recv() {
            Ok(HubEvent::SessionChanged { card }) if card.state == HubState::Working => break,
            Ok(_) => {}
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => {
                assert!(
                    std::time::Instant::now() < deadline,
                    "the prompt's run never showed as Working"
                );
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            Err(_) => panic!("hub events closed"),
        }
    }
    drop(opened);
}
