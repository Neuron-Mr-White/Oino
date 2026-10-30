//! WP-A4 acceptance (app spec §3.14): session meta — pin / unread / title lock —
//! persisted in `session-meta.json` under the host data dir, carried on every
//! card, and Z-86's unread rule: a run that ends (or a session that starts
//! needing the user) while **no connection focuses it** marks the session
//! unread; a focused session never is, and focusing clears the dot.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_client::protocol::{PiCommand, RpcCommand};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use unipi_host::sessions::hub::{HostHubOptions, HubSession, SessionHub};
use unipi_host::sessions::SessionCard;
use unipi_proto::session::SessionMetaPatch;

struct Fixture {
    _dir: tempfile::TempDir,
    /// The host data dir (`$OINO_HOST_DIR` stand-in): `session-meta.json` lives here.
    host_data: PathBuf,
    cwd: PathBuf,
    agent_dir: PathBuf,
    sessions: PathBuf,
}

impl Fixture {
    /// A hub over the fixture; `host_data_dir` points at the shared temp dir, so a
    /// second hub (a "restart") reads the same `session-meta.json`. The faux
    /// provider has 8 scripted answers (one per prompt, like `sessions_hub.rs`).
    fn hub(&self) -> Arc<SessionHub> {
        let provider =
            FauxProvider::new((0..8).map(|_| FauxResponse::Text("done".into())).collect());
        SessionHub::create(HostHubOptions {
            cwd: self.cwd.clone(),
            agent_dir: self.agent_dir.clone(),
            docs_root: None,
            session_dir: Some(self.sessions.clone()),
            mode: ExtensionMode::Desktop,
            has_ui: false,
            approve: Some(false),
            flags: ResourceFlags {
                no_context_files: true,
                ..ResourceFlags::default()
            },
            loader: Arc::new(StaticLoader(vec![
                Arc::new(FauxComponent(provider)) as Arc<dyn ExtensionInstance>
            ])),
            models: {
                let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
                models.set_runtime_api_key("faux", "test-key");
                Some(models)
            },
            tools: ToolSelection::Default,
            exclude_tools: Vec::new(),
            host_data_dir: Some(self.host_data.clone()),
        })
        .unwrap()
    }
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    let host_data = dir.path().join("host");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::create_dir_all(&host_data).unwrap();
    Fixture {
        _dir: dir,
        host_data,
        cwd,
        agent_dir,
        sessions,
    }
}

async fn create(hub: &Arc<SessionHub>) -> HubSession {
    let session = hub.create_session(None, None).await.unwrap();
    session
        .send(RpcCommand::Pi(PiCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
        }))
        .await
        .unwrap();
    session
}

async fn prompt(session: &HubSession, text: &str) {
    session
        .send(RpcCommand::Pi(PiCommand::Prompt {
            message: text.into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
}

async fn card(hub: &Arc<SessionHub>, session_id: &str) -> SessionCard {
    hub.card(session_id).await.unwrap().unwrap()
}

/// Waits (bounded) until `f(card)` holds; the run supervisor marks unread a few
/// idle polls after the run ends.
async fn wait_for(hub: &Arc<SessionHub>, session_id: &str, f: fn(&SessionCard) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if f(&card(hub, session_id).await) {
            return;
        }
        if Instant::now() > deadline {
            panic!(
                "condition never held for {session_id}: {:?}",
                card(hub, session_id).await
            );
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

/// Drives one full run to its end (the faux answer settles), returning its card.
async fn run_once(session: &HubSession, text: &str) {
    prompt(session, text).await;
    // The run ends when the assistant text settles and the supervisor's idle
    // polls release it (≤ SUPERVISE_IDLE_POLLS × 1 s).
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let data = session
            .send(RpcCommand::Pi(PiCommand::GetState))
            .await
            .unwrap();
        let value: serde_json::Value =
            serde_json::from_value(data.expect("get_state data")).unwrap();
        if value["isStreaming"] == serde_json::json!(false) {
            return;
        }
        assert!(Instant::now() < deadline, "the faux run never settled");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

#[tokio::test]
async fn meta_persists_and_rides_every_card() {
    let fx = fixture();
    let hub = fx.hub();
    let session = create(&hub).await;
    let id = session.session_id();

    // A prompt writes the file, so the record exists.
    run_once(&session, "hello").await;
    // A second hub is a second process over the same dirs (host.lock stops that
    // in production): what it sets reaches clients as an event, and any hub
    // started after via the file — never the first hub's memory.
    let mut events = {
        let hub2 = fx.hub();
        let events = hub2.subscribe();
        let patched = hub2
            .meta_set(
                &id,
                SessionMetaPatch {
                    pinned: Some(true),
                    title_locked: Some(true),
                    ..SessionMetaPatch::default()
                },
            )
            .await
            .unwrap();
        assert!(patched.pinned);
        assert!(patched.title_locked);
        events
    };
    use unipi_host::sessions::hub::HubEvent;
    match events.try_recv() {
        Ok(HubEvent::SessionChanged { card }) => assert!(card.pinned && card.title_locked),
        other => panic!("expected a SessionChanged with the flags, got {other:?}"),
    }

    // A hub started after reads the same `session-meta.json` (a restart).
    let restarted = fx.hub();
    let fetched = card(&restarted, &id).await;
    assert!(fetched.pinned && fetched.title_locked);
    assert!(restarted
        .list()
        .await
        .unwrap()
        .iter()
        .any(|c| c.session_id == id && c.pinned));

    // The file is where §3.14 says, and clearing every flag drops the record.
    assert!(fx.host_data.join("session-meta.json").exists());
    let _ = &hub;
    restarted
        .meta_set(
            &id,
            SessionMetaPatch {
                pinned: Some(false),
                title_locked: Some(false),
                ..SessionMetaPatch::default()
            },
        )
        .await
        .unwrap();
    let flags: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(fx.host_data.join("session-meta.json")).unwrap(),
    )
    .unwrap();
    assert!(flags["sessions"].as_object().unwrap().is_empty(), "{flags}");
    assert!(!card(&fx.hub(), &id).await.pinned);
}

#[tokio::test]
async fn a_run_that_ends_unfocused_marks_unread_but_a_focused_one_does_not() {
    let fx = fixture();
    let hub = fx.hub();
    let session = create(&hub).await;
    let id = session.session_id();

    // No connection focuses it: the first run's end marks it unread (Z-86).
    run_once(&session, "one").await;
    wait_for(&hub, &id, |card| card.unread).await;

    // A connection focuses it: the dot clears (Z-82 "cleared on open")…
    let conn = hub.focus_connection();
    hub.set_focus(conn, Some(&id)).await;
    wait_for(&hub, &id, |card| !card.unread).await;

    // …and the next run's end does not mark it again while it stays focused.
    run_once(&session, "two").await;
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert!(
        !card(&hub, &id).await.unread,
        "a focused session is never unread"
    );

    // The focus leaves with its connection; the next run's end marks it again.
    hub.release_focus(conn).await;
    run_once(&session, "three").await;
    wait_for(&hub, &id, |card| card.unread).await;
}

#[tokio::test]
async fn needing_the_user_unfocused_marks_unread() {
    let fx = fixture();
    let hub = fx.hub();
    let session = create(&hub).await;
    let id = session.session_id();
    // A dialog only ever arrives for a session that exists: write the file.
    run_once(&session, "hello").await;

    // Unfocused: a dialog marks the session unread (§3.14), and the card says needs-you.
    hub.set_needs_you(&id, true).await;
    let fetched = card(&hub, &id).await;
    assert!(fetched.needs_you, "{fetched:?}");

    // Focused: the dot clears, and a dialog arriving while focused never re-marks it.
    let conn = hub.focus_connection();
    hub.set_focus(conn, Some(&id)).await;
    wait_for(&hub, &id, |card| !card.unread).await;
    hub.set_needs_you(&id, false).await;
    hub.set_needs_you(&id, true).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let fetched = card(&hub, &id).await;
    assert!(fetched.needs_you && !fetched.unread, "{fetched:?}");

    // "Mark as unread" is manual and works even while focused (Z-82).
    hub.meta_set(
        &id,
        SessionMetaPatch {
            unread: Some(true),
            ..SessionMetaPatch::default()
        },
    )
    .await
    .unwrap();
    assert!(card(&hub, &id).await.unread);
}

#[tokio::test]
async fn meta_set_rejects_unknown_sessions() {
    let fx = fixture();
    let hub = fx.hub();
    let error = hub
        .meta_set(
            "no-such-session",
            SessionMetaPatch {
                pinned: Some(true),
                ..SessionMetaPatch::default()
            },
        )
        .await
        .unwrap_err();
    assert!(error.to_string().contains("No session"), "{error}");
}
