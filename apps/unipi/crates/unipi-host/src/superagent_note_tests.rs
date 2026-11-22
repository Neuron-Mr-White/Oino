//! "Send as note" in the chats that run inside the host (the Superagent chat and a
//! dispatcher lead's chat): the app's `append_note` command goes through the same
//! `HubSession` → `InProcessBackend` path as any session, saves an `oino.note` custom
//! entry — idle or mid-run — and the model never sees it. The Superagent's own readers
//! of the owner's words (`owner_messages`, the image handoff, the ticket origin stamp)
//! stay on real user turns.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use crate::sessions::hub::{HostHubOptions, HubSession, SessionHub};
use oino_client::protocol::{AgentCommand, RpcCommand};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource, ToolSelection};
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use serde_json::json;

struct Fixture {
    _dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
    provider: Arc<FauxProvider>,
}

fn fixture(responses: Vec<FauxResponse>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let cwd: PathBuf = dir.path().to_path_buf();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    let host_data = dir.path().join("host");
    for path in [&agent_dir, &sessions, &host_data] {
        std::fs::create_dir_all(path).unwrap();
    }
    let provider = FauxProvider::new(responses);
    let hub = SessionHub::create(HostHubOptions {
        cwd,
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
        loader: Arc::new(StaticLoader(vec![
            Arc::new(FauxComponent(Arc::clone(&provider))) as Arc<dyn ExtensionInstance>,
        ])),
        models: {
            let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
            models.set_runtime_api_key("faux", "test-key");
            Some(models)
        },
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
        host_data_dir: Some(host_data),
    })
    .unwrap();
    Fixture {
        _dir: dir,
        hub,
        provider,
    }
}

impl Fixture {
    async fn superagent(&self) -> HubSession {
        let runtime = super::create_with_loader(
            Arc::clone(&self.hub),
            None,
            Arc::new(StaticLoader(vec![Arc::new(FauxComponent(Arc::clone(
                &self.provider,
            )))])),
        )
        .await
        .unwrap();
        let chat = self
            .hub
            .open(&runtime.session().session_id())
            .await
            .unwrap();
        chat.send(RpcCommand::Agent(AgentCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
            persist: None,
        }))
        .await
        .unwrap();
        chat
    }
}

fn note(text: &str) -> RpcCommand {
    RpcCommand::Agent(AgentCommand::AppendNote {
        text: text.into(),
        images: Vec::new(),
    })
}

fn prompt(text: &str) -> RpcCommand {
    RpcCommand::Agent(AgentCommand::Prompt {
        message: text.into(),
        images: Vec::new(),
        streaming_behavior: None,
    })
}

/// `(type, customType)` of every entry on the chat's branch, oldest first.
fn kinds(session_id: &str) -> Vec<(String, Option<String>)> {
    let runtime = crate::superagent_session::runtime(session_id).expect("registered");
    runtime.session().with_manager(|m| {
        m.branch(None)
            .iter()
            .map(|entry| {
                let value = serde_json::to_value(entry).unwrap();
                (
                    value["type"].as_str().unwrap().to_owned(),
                    value["customType"].as_str().map(str::to_owned),
                )
            })
            .collect()
    })
}

async fn save_note(chat: &HubSession, text: &str) -> String {
    let reply = tokio::time::timeout(Duration::from_secs(5), chat.send(note(text)))
        .await
        .expect("the note must not wait for the run")
        .expect("note saved")
        .expect("a reply");
    reply["entryId"].as_str().expect("entry id").to_owned()
}

async fn wait_running(chat: &HubSession, provider: &FauxProvider) {
    for _ in 0..200 {
        let state = chat
            .send(RpcCommand::Agent(AgentCommand::GetState))
            .await
            .unwrap()
            .unwrap();
        if state["isStreaming"] == json!(true) && !provider.requests().is_empty() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the run never started");
}

fn note_entries(session_id: &str) -> usize {
    kinds(session_id)
        .iter()
        .filter(|(kind, custom)| kind == "custom" && custom.as_deref() == Some("oino.note"))
        .count()
}

/// (b) A Superagent chat, idle: the note is a custom entry, not a user turn, and the
/// owner-message readers keep naming the last real owner turn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn superagent_chat_saves_a_note_idle_and_owner_readers_ignore_it() {
    let f = fixture(vec![FauxResponse::Text("done".into())]);
    let chat = f.superagent().await;
    let sid = chat.session_id();
    chat.send(prompt("please fix the login timeout"))
        .await
        .unwrap();
    for _ in 0..200 {
        if kinds(&sid).iter().any(|(kind, _)| kind == "message")
            && !f.provider.requests().is_empty()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let owner_turn = crate::superagent_owner_messages::latest_owner_entry_from(&sid)
        .expect("the owner's prompt is the latest owner turn");

    let note_id = save_note(&chat, "remind me: ask about the SSO ticket").await;

    assert_eq!(note_entries(&sid), 1);
    assert_ne!(note_id, owner_turn);
    assert_eq!(
        crate::superagent_owner_messages::latest_owner_entry_from(&sid),
        Some(owner_turn),
        "a note is not the owner's latest message (ticket origin stamp)"
    );
    assert!(
        !f.provider.requests().is_empty(),
        "the faux provider must receive the run"
    );
    let seen = serde_json::to_string(&f.provider.requests()).unwrap();
    assert!(
        !seen.contains("SSO ticket"),
        "the model must never see a note"
    );
}

/// (b) A Superagent chat, mid-run: the note lands at once, the run keeps going and
/// never receives it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn superagent_chat_saves_a_note_while_a_run_is_going() {
    let f = fixture(vec![FauxResponse::Never]);
    let chat = f.superagent().await;
    let sid = chat.session_id();
    chat.send(prompt("start the nightly checks")).await.unwrap();
    wait_running(&chat, &f.provider).await;
    // isStreaming becomes true before the prompt is appended. Establish the
    // persisted owner turn before checking that a note cannot replace it.
    let owner_turn = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(id) = crate::superagent_owner_messages::latest_owner_entry_from(&sid) {
                break Some(id);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("the owner's prompt is persisted while the run is active");

    save_note(&chat, "mid-run thought for me").await;

    assert_eq!(note_entries(&sid), 1);
    let state = chat
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state["isStreaming"],
        json!(true),
        "a note leaves the run alone"
    );
    assert_eq!(
        crate::superagent_owner_messages::latest_owner_entry_from(&sid),
        owner_turn
    );
    assert!(
        !f.provider.requests().is_empty(),
        "the faux provider must receive the run"
    );
    let seen = serde_json::to_string(&f.provider.requests()).unwrap();
    assert!(
        !seen.contains("mid-run thought"),
        "the model must never see a note"
    );
    let _ = chat.send(RpcCommand::Agent(AgentCommand::Abort)).await;
}

/// (c) A dispatcher lead's chat is a runtime registered with the same registry
/// (`HostWorkers` → `superagent_session::register`) and opened through the same hub path.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn lead_chat_saves_a_note_idle_and_mid_run() {
    let f = fixture(vec![FauxResponse::Never]);
    let services = f.hub.core_services().await.unwrap();
    let runtime = AgentSessionRuntime::create_for_hub(
        services,
        SessionOptions {
            source: SessionSource::New,
            model: Some(FauxProvider::model()),
            tools: ToolSelection::NoBuiltin,
            session_dir: Some(f.hub.sessions_root().to_path_buf()),
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();
    let runtime = Arc::new(runtime);
    let sid = runtime.session().session_id();
    crate::superagent_session::register(runtime);
    let lead = f.hub.open(&sid).await.unwrap();

    save_note(&lead, "idle note in a lead chat").await;
    assert_eq!(note_entries(&sid), 1);

    lead.send(prompt("work the board")).await.unwrap();
    wait_running(&lead, &f.provider).await;
    save_note(&lead, "busy note in a lead chat").await;
    assert_eq!(note_entries(&sid), 2);
    assert!(
        !f.provider.requests().is_empty(),
        "the faux provider must receive the run"
    );
    let seen = serde_json::to_string(&f.provider.requests()).unwrap();
    assert!(!seen.contains("note in a lead chat"));
    let _ = lead.send(RpcCommand::Agent(AgentCommand::Abort)).await;
    crate::superagent_session::unregister(&sid);
}
