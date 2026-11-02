//! "No workspace" chats (UniPi Oino): `session_new` without a cwd runs the chat in its own
//! private scratch folder (never `$HOME`), and a scratch chat moves to a real workspace
//! one way only — append-only, with the system prompt and earlier messages byte-identical
//! before and after (prompt cache), surviving a close + reopen.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_client::protocol::{AgentCommand, RpcCommand, RpcResponseData, RpcSessionState};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_session::{EntryKind, SessionManager, CWD_CHANGE_CUSTOM_TYPE};
use serde_json::Value;
use unipi_host::sessions::hub::{HostHubOptions, HubSession, SessionHub};

struct Fixture {
    dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
    provider: Arc<FauxProvider>,
}

fn fixture(responses: Vec<FauxResponse>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(responses);
    let loader = StaticLoader(vec![
        Arc::new(FauxComponent(Arc::clone(&provider))) as Arc<dyn ExtensionInstance>
    ]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        // The host's primary cwd plays `$HOME`: no scratch chat may run there.
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
    Fixture { dir, hub, provider }
}

async fn select_faux(session: &HubSession) {
    session
        .send(RpcCommand::Agent(AgentCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
            persist: None,
        }))
        .await
        .unwrap();
}

async fn state_of(session: &HubSession) -> RpcSessionState {
    let data: RpcResponseData = session
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await
        .unwrap();
    serde_json::from_value(data.unwrap()).unwrap()
}

async fn ask(session: &HubSession, text: &str) {
    let before = state_of(session).await.message_count;
    session
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: text.into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = state_of(session).await;
        if !state.is_streaming && !state.is_compacting && state.message_count >= before + 2 {
            return;
        }
        assert!(Instant::now() < deadline, "session never answered");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn messages(request: &Value) -> Vec<String> {
    request["context"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| serde_json::to_string(m).unwrap())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn no_workspace_chats_run_in_their_own_scratch_folder_never_home() {
    let fx = fixture(Vec::new());
    let root = fx.hub.scratch_root().to_path_buf();
    assert!(root.is_dir(), "the hub creates the scratch root");
    let a = fx.hub.create_scratch_session(None).await.unwrap();
    let b = fx.hub.create_scratch_session(Some("named")).await.unwrap();
    let cwd_a = state_of(&a).await.cwd.unwrap();
    let cwd_b = state_of(&b).await.cwd.unwrap();
    assert_eq!(
        Path::new(&cwd_a),
        root.join(a.session_id()),
        "named after the session id"
    );
    assert_eq!(Path::new(&cwd_b), root.join(b.session_id()));
    assert_ne!(cwd_a, cwd_b, "one folder per chat");
    assert!(Path::new(&cwd_a).is_dir(), "created on session start");
    assert_ne!(
        Path::new(&cwd_a),
        fx.hub.cwd(),
        "never the home / primary cwd"
    );
    assert_eq!(a.card().cwd, cwd_a);
    // An explicit workspace still runs exactly there.
    let work = fx.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let c = fx.hub.create_session(Some(&work), None).await.unwrap();
    assert_eq!(state_of(&c).await.cwd.unwrap(), work.display().to_string());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_scratch_chat_moves_to_a_workspace_one_way_append_only() {
    let fx = fixture(vec![
        FauxResponse::Text("first".into()),
        FauxResponse::Text("second".into()),
        FauxResponse::Text("third".into()),
    ]);
    let work = fx.dir.path().join("work");
    let other = fx.dir.path().join("other");
    std::fs::create_dir_all(&work).unwrap();
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(work.join("keep.txt"), "workspace").unwrap();

    let chat = fx.hub.create_scratch_session(None).await.unwrap();
    let id = chat.session_id();
    select_faux(&chat).await;
    let scratch = std::path::PathBuf::from(state_of(&chat).await.cwd.unwrap());
    std::fs::write(scratch.join("draft.md"), "from scratch").unwrap();
    std::fs::write(scratch.join("keep.txt"), "scratch").unwrap();
    ask(&chat, "hello").await;
    let file = std::path::PathBuf::from(state_of(&chat).await.session_file.unwrap());
    let before = std::fs::read(&file).unwrap();

    let moved = fx.hub.move_to_workspace(&id, &work, true).await.unwrap();
    let work_canon = dunce::canonicalize(&work).unwrap();
    assert_eq!(
        moved.card.cwd,
        work_canon.display().to_string(),
        "listed under the workspace"
    );
    assert_eq!(
        (moved.copied, moved.skipped),
        (1, 1),
        "copy never overwrites"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("keep.txt")).unwrap(),
        "workspace"
    );
    assert_eq!(
        std::fs::read_to_string(work.join("draft.md")).unwrap(),
        "from scratch"
    );
    assert_eq!(
        state_of(&chat).await.cwd.unwrap(),
        work_canon.display().to_string(),
        "tools run in the workspace from the next turn"
    );

    // Append-only persistence: earlier bytes untouched, header cwd kept, one cwd_change.
    let after = std::fs::read(&file).unwrap();
    assert!(after.starts_with(&before), "earlier entries were rewritten");
    let manager =
        SessionManager::open(&file, file.parent().unwrap().parent().unwrap(), None).unwrap();
    assert_eq!(
        manager.cwd(),
        scratch.as_path(),
        "the header cwd stays the scratch folder"
    );
    assert!(matches!(
        &manager.entries().last().unwrap().kind,
        EntryKind::CustomMessage { custom_type, .. } if custom_type == CWD_CHANGE_CUSTOM_TYPE
    ));
    drop(manager);

    // One way: never back to scratch, never workspace → another workspace.
    let back = fx.hub.move_to_workspace(&id, &scratch, false).await;
    assert!(back.is_err(), "workspace → scratch must be refused");
    let again = fx.hub.move_to_workspace(&id, &other, false).await;
    assert!(again.is_err(), "workspace → workspace must be refused");
    let plain = fx.hub.create_session(Some(&other), None).await.unwrap();
    assert!(
        fx.hub
            .move_to_workspace(&plain.session_id(), &work, false)
            .await
            .is_err(),
        "a chat that already has a workspace has no move"
    );

    // Prompt cache: the next request's prefix (system prompt + earlier messages) is
    // byte-identical; the reminder follows as a user message.
    ask(&chat, "go on").await;
    let requests = fx.provider.requests();
    let first = messages(&requests[0]);
    let second = messages(&requests[1]);
    assert_eq!(second[..first.len()], first[..], "prompt prefix changed");
    let reminder = &requests[1]["context"]["messages"][first.len() + 1];
    assert_eq!(reminder["role"], "user");
    assert!(
        reminder.to_string().contains("The workspace changed"),
        "{reminder}"
    );

    // Close + reopen: still in the workspace, still listed there, same system prompt.
    drop(chat);
    fx.hub.close(&id).await.unwrap();
    let card = fx.hub.card(&id).await.unwrap().unwrap();
    assert_eq!(card.cwd, work_canon.display().to_string());
    let reopened = fx.hub.open(&id).await.unwrap();
    assert_eq!(
        state_of(&reopened).await.cwd.unwrap(),
        work_canon.display().to_string()
    );
    select_faux(&reopened).await;
    ask(&reopened, "after reopen").await;
    let requests = fx.provider.requests();
    let third = messages(requests.last().unwrap());
    assert_eq!(
        third[0], first[0],
        "the reopened chat's system prompt is unchanged"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_scratch_chats_fork_gets_its_own_copy_of_the_folder() {
    let fx = fixture(vec![FauxResponse::Text("first".into())]);
    let root = fx.hub.scratch_root().to_path_buf();
    let chat = fx.hub.create_scratch_session(None).await.unwrap();
    select_faux(&chat).await;
    let scratch = root.join(chat.session_id());
    std::fs::create_dir_all(scratch.join("sub")).unwrap();
    std::fs::write(scratch.join("a.txt"), "original").unwrap();
    std::fs::write(scratch.join("sub").join("b.txt"), "nested").unwrap();
    ask(&chat, "hello").await;

    let fork = fx.hub.fork(&chat.session_id(), None, None).await.unwrap();
    assert_ne!(fork.session_id(), chat.session_id());
    let fork_dir = root.join(fork.session_id());
    assert_eq!(
        state_of(&fork).await.cwd.unwrap(),
        fork_dir.display().to_string(),
        "the fork runs in its own scratch folder, named after its id"
    );
    assert_eq!(fork.card().cwd, fork_dir.display().to_string());
    assert_eq!(std::fs::read_to_string(fork_dir.join("a.txt")).unwrap(), "original");
    assert_eq!(
        std::fs::read_to_string(fork_dir.join("sub").join("b.txt")).unwrap(),
        "nested"
    );
    // A copy, not a share: writes on one side never show on the other.
    std::fs::write(fork_dir.join("a.txt"), "forked").unwrap();
    std::fs::write(fork_dir.join("new.txt"), "only in the fork").unwrap();
    assert_eq!(std::fs::read_to_string(scratch.join("a.txt")).unwrap(), "original");
    assert!(!scratch.join("new.txt").exists());
    assert_eq!(state_of(&chat).await.cwd.unwrap(), scratch.display().to_string());

    // A workspace chat's fork stays in the workspace (no copy anywhere).
    let work = fx.dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let before = std::fs::read_dir(&root).unwrap().count();
    let plain = fx.hub.create_session(Some(&work), None).await.unwrap();
    select_faux(&plain).await;
    fx.provider.push(FauxResponse::Text("ok".into()));
    ask(&plain, "hi").await;
    let plain_fork = fx.hub.fork(&plain.session_id(), None, None).await.unwrap();
    assert_eq!(state_of(&plain_fork).await.cwd.unwrap(), work.display().to_string());
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), before);
}
