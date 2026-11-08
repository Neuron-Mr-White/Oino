//! WP-A5 acceptance (app spec §3.14): the archived list — `sessions_list{archived:true}`,
//! `session_restore`, `session_delete` (trash only) and `trash_empty`.
//!
//! Archive moves a session's file to `<agent dir>/trash/` (§14.2); the archived list is
//! built from those files (headers only, no runtime), restore puts the file back under
//! the sessions root in its project's folder, and delete/empty only ever touch the trash.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::sync::Arc;

use oino_client::protocol::{AgentCommand, RpcCommand};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use unipi_host::sessions::hub::{HostHubOptions, HubSession, SessionHub};

struct Fixture {
    _dir: tempfile::TempDir,
    cwd: PathBuf,
    agent_dir: PathBuf,
    sessions: PathBuf,
    host_data: PathBuf,
}

impl Fixture {
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
            services_cap: None,
            host_data_dir: Some(self.host_data.clone()),
        })
        .unwrap()
    }

    fn trash(&self) -> PathBuf {
        self.agent_dir.join("trash")
    }
}

fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_path_buf();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    let host_data = dir.path().join("host");
    for path in [&agent_dir, &sessions, &host_data] {
        std::fs::create_dir_all(path).unwrap();
    }
    Fixture {
        _dir: dir,
        cwd,
        agent_dir,
        sessions,
        host_data,
    }
}

/// A session with one settled run, so its file exists on disk.
async fn session_with_a_run(hub: &Arc<SessionHub>) -> HubSession {
    let session = start(hub).await;
    prompt(&session).await;
    session
}

async fn start(hub: &Arc<SessionHub>) -> HubSession {
    let session = hub.create_session(None, None).await.unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
            persist: None,
        }))
        .await
        .unwrap();
    session
}

async fn prompt(session: &HubSession) {
    session
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: "hello".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
}

/// The session's card once its file has the first entry (the prompt writes it).
async fn card_after_prompt(hub: &Arc<SessionHub>, id: &str) -> unipi_host::sessions::SessionCard {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    loop {
        if let Some(card) = hub.card(id).await.unwrap() {
            if card.message_count > 0 && card.file.exists() {
                return card;
            }
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the session file never appeared"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn archive_restore_round_trip() {
    let fx = fixture();
    let hub = fx.hub();
    let session = session_with_a_run(&hub).await;
    let id = session.session_id();
    let file = card_after_prompt(&hub, &id).await.file;

    // Archive: the file leaves the sessions root for trash/ and the list empties.
    let archived = hub.archive(&id).await.unwrap();
    assert_eq!(archived.session_id, id);
    assert!(!file.exists());
    assert!(
        hub.list().await.unwrap().is_empty(),
        "archived sessions leave the live list"
    );

    // The archived list reads the trashed file (headers only) and keeps the facts.
    let trashed = hub.archived().await.unwrap();
    assert_eq!(trashed.len(), 1);
    assert_eq!(trashed[0].session_id, id);
    assert_eq!(trashed[0].cwd, fx.cwd.display().to_string());
    assert!(trashed[0].file.starts_with(fx.trash()));
    assert!(
        trashed[0].message_count >= 2,
        "{}",
        trashed[0].message_count
    );

    // Restore: the file goes back under the sessions root in its project folder
    // (core's own `--<cwd>--` layout) and the live list has it again.
    let restored = hub.restore(&id).await.unwrap();
    assert_eq!(restored.session_id, id);
    assert!(
        restored.file.starts_with(&fx.sessions),
        "{}",
        restored.file.display()
    );
    assert!(restored.file.exists());
    assert!(hub.archived().await.unwrap().is_empty());
    let listed = hub.list().await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].session_id, id);
    assert_eq!(listed[0].message_count, restored.message_count);
    // A restored file is still a live session: it opens again.
    let reopened = hub.open(&id).await.unwrap();
    assert_eq!(reopened.session_id(), id);
}

#[tokio::test]
async fn delete_and_empty_only_touch_the_trash() {
    let fx = fixture();
    let hub = fx.hub();
    let first = session_with_a_run(&hub).await;
    let second = session_with_a_run(&hub).await;
    let (a, b) = (first.session_id(), second.session_id());
    let a_file = card_after_prompt(&hub, &a).await.file;

    // A live session is not deletable: archive first (the app's Delete does that).
    let refused = hub.delete_archived(&a).await;
    assert!(refused.is_err(), "session_delete is trash-only");
    assert!(a_file.exists());

    hub.archive(&a).await.unwrap();
    hub.archive(&b).await.unwrap();
    assert_eq!(hub.archived().await.unwrap().len(), 2);

    hub.delete_archived(&a).await.unwrap();
    let left = hub.archived().await.unwrap();
    assert_eq!(left.len(), 1);
    assert_eq!(left[0].session_id, b);

    // Delete all archived (ZCode's "Delete all archived").
    assert_eq!(hub.empty_trash().await.unwrap(), 1);
    assert!(hub.archived().await.unwrap().is_empty());
    assert!(
        std::fs::read_dir(fx.trash())
            .map(|entries| entries.flatten().count())
            .unwrap_or(0)
            == 0,
        "trash is empty"
    );
    // Deleting the same id twice reports a clear failure, not a panic.
    assert!(hub.delete_archived(&a).await.is_err());
}

#[tokio::test]
async fn fileless_live_session_supports_drafts_meta_and_archive_and_duplicate_archive_is_idempotent() {
    let fx = fixture();
    let hub = fx.hub();

    // 1. File-less live session (`session_new` before any prompt writes `.jsonl`, E-03):
    let empty = start(&hub).await;
    let empty_id = empty.session_id();
    drop(empty);

    assert_eq!(hub.draft_get(&empty_id).await.unwrap(), None);
    let card = hub
        .draft_set(&empty_id, Some(serde_json::json!({"text": "draft"})))
        .await
        .unwrap();
    assert_eq!(card.session_id, empty_id);
    assert!(card.draft_at > 0);
    let cleared = hub.draft_set(&empty_id, None).await.unwrap();
    assert_eq!(cleared.draft_at, 0);

    let archived_empty = hub.archive(&empty_id).await.unwrap();
    assert_eq!(archived_empty.session_id, empty_id);
    assert!(
        hub.open(&empty_id).await.is_err(),
        "archiving a file-less live session closes its runtime"
    );

    // 2. Duplicate archive on an already-trashed session succeeds idempotently instead of E_NOT_FOUND.
    let ran = session_with_a_run(&hub).await;
    let ran_id = ran.session_id();
    let _ = card_after_prompt(&hub, &ran_id).await;
    let first = hub.archive(&ran_id).await.unwrap();
    let second = hub.archive(&ran_id).await.unwrap();
    assert_eq!(first.session_id, ran_id);
    assert_eq!(second.session_id, ran_id);
}
