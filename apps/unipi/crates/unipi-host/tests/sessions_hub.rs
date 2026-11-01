//! WP-H3 acceptance (spec-host §4.2, §4.4): the host session hub over the wave-1 core hub —
//! create, prompt, concurrent sessions, a crash in one session marking *that* card while the
//! others keep running, and archiving a session's file into `<agent dir>/trash/`.
//!
//! The faux provider answers prompts; a fake extension supplies a `boom` tool whose body
//! panics inside the agent loop (core's X-PANIC path, which the host supervisor observes).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_agent::AgentToolResult;
use oino_client::protocol::{AgentCommand, RpcCommand, RpcResponseData, RpcSessionState};
use oino_core::extensions::api::{
    ExtensionInstance, ExtensionMode, Registration, ToolExposure, ToolSpec,
};
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use serde_json::json;
use unipi_host::sessions::card::HubState;
use unipi_host::sessions::hub::{HostHubOptions, HubEvent, HubSession, NoticeLevel, SessionHub};
use unipi_host::sessions::SessionCard;

fn tool(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        label: name.into(),
        description: format!("{name} tool"),
        parameters: json!({"type": "object"}),
        prompt_snippet: None,
        prompt_guidelines: Vec::new(),
        exposure: ToolExposure::Direct,
        default_active: None,
        execution_mode: None,
        namespace: None,
        annotations: None,
        reaches_tools: None,
        output_schema: None,
        prepare_arguments: false,
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
}

/// A host hub whose sessions are answered by a faux provider and can call `boom`, which
/// panics inside the tool body.
fn hub_with(responses: Vec<FauxResponse>) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(responses);
    let boom = FakeExtension::new(
        "ext",
        Registration {
            tools: vec![tool("boom")],
            ..Registration::default()
        },
    )
    .on_tool(|name, _| {
        if name != "boom" {
            return Ok(AgentToolResult::text("ok"));
        }
        panic!("tool body exploded");
    });
    let loader = StaticLoader(vec![
        Arc::new(FauxComponent(Arc::clone(&provider))) as Arc<dyn ExtensionInstance>,
        Arc::new(boom) as Arc<dyn ExtensionInstance>,
    ]);
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
    Fixture { _dir: dir, hub }
}

/// Creates a session and selects the faux model (a fresh session has no selection yet).
async fn create(hub: &Arc<SessionHub>) -> HubSession {
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

async fn prompt(session: &HubSession, text: &str) {
    session
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: text.into(),
            images: Vec::new(),
            streaming_behavior: None,
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

async fn last_text(session: &HubSession) -> String {
    let data: RpcResponseData = session
        .send(RpcCommand::Agent(AgentCommand::GetLastAssistantText))
        .await
        .unwrap();
    data.and_then(|value| value["text"].as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Waits (bounded) for the session's last assistant text (the run may still be starting when
/// `prompt` returns).
async fn wait_text(session: &HubSession) -> String {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let text = last_text(session).await;
        if !text.is_empty() {
            return text;
        }
        assert!(Instant::now() < deadline, "session never answered");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits (bounded) for the run to settle (`get_state` says neither streaming nor compacting).
async fn wait_idle(session: &HubSession) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let state = state_of(session).await;
        if !state.is_streaming && !state.is_compacting {
            return;
        }
        assert!(Instant::now() < deadline, "session never went idle");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Waits (bounded) for a card whose state matches `want`.
async fn wait_card(
    hub: &SessionHub,
    session_id: &str,
    want: impl Fn(&HubState) -> bool,
) -> SessionCard {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(card) = hub.card(session_id).await.unwrap() {
            if want(&card.state) {
                return card;
            }
        }
        assert!(
            Instant::now() < deadline,
            "session {session_id} never reached the expected state"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn create_prompt_concurrent_sessions_crash_marks_one_and_archive_moves_to_trash() {
    let fixture = hub_with(vec![
        // A's first run: the `boom` tool body panics (contained by core, X-PANIC).
        FauxResponse::ToolCalls(vec![(
            "c1".into(),
            "boom".into(),
            serde_json::from_value(json!({})).unwrap(),
        )]),
        // B and C answer concurrently (either order).
        FauxResponse::Text("concurrent reply one".into()),
        FauxResponse::Text("concurrent reply two".into()),
        // A after reopening.
        FauxResponse::Text("recovered reply".into()),
    ]);
    let hub = &fixture.hub;

    // Three sessions live in one host at the same time.
    let a = create(hub).await;
    let b = create(hub).await;
    let c = create(hub).await;
    assert_ne!(a.session_id(), b.session_id());
    assert_eq!(
        hub.list().await.unwrap().len(),
        0,
        "no file before a message"
    );

    // A crashes: the card goes `crashed` with the panic text, and the host process stays up.
    prompt(&a, "run the boom tool").await;
    let crashed = wait_card(hub, &a.session_id(), |state| {
        matches!(state, HubState::Crashed { .. })
    })
    .await;
    let HubState::Crashed { error } = crashed.state else {
        panic!("expected a crashed card");
    };
    assert!(error.contains("tool body exploded"), "{error}");

    // The other sessions keep running: two prompts in parallel, both answered.
    let (text_b, text_c) = tokio::join!(
        async {
            prompt(&b, "hello b").await;
            wait_text(&b).await
        },
        async {
            prompt(&c, "hello c").await;
            wait_text(&c).await
        }
    );
    for text in [&text_b, &text_c] {
        assert!(
            text == "concurrent reply one" || text == "concurrent reply two",
            "{text_b:?} {text_c:?}"
        );
    }
    // Their supervisors settle them back to idle (§4.2 card state).
    wait_card(hub, &b.session_id(), |state| *state == HubState::Idle).await;
    wait_card(hub, &c.session_id(), |state| *state == HubState::Idle).await;
    // A is still listed as crashed until it is re-opened.
    assert!(matches!(
        hub.card(&a.session_id()).await.unwrap().unwrap().state,
        HubState::Crashed { .. }
    ));

    // Re-open: the crashed runtime is disposed, the record repairs and continues.
    let reopened = hub.open(&a.session_id()).await.unwrap();
    assert_eq!(reopened.session_id(), a.session_id());
    prompt(&reopened, "continue").await;
    assert_eq!(wait_text(&reopened).await, "recovered reply");
    wait_idle(&reopened).await;
    wait_card(hub, &a.session_id(), |state| *state == HubState::Idle).await;

    // Archive: the runtime is disposed and the file moves to <agent>/trash/.
    let archived = hub.archive(&a.session_id()).await.unwrap();
    assert!(
        !archived.file.exists(),
        "the session file left the sessions root"
    );
    assert!(
        hub.card(&a.session_id()).await.unwrap().is_none(),
        "an archived session is no longer listed"
    );
    let trash = hub.trash_dir();
    assert_eq!(hub.agent_dir(), trash.parent().unwrap());
    let files: Vec<_> = std::fs::read_dir(&trash)
        .unwrap()
        .map(|entry| entry.unwrap().file_name())
        .collect();
    assert_eq!(files.len(), 1, "{files:?}");
    assert_eq!(
        files[0].to_string_lossy(),
        archived.file.file_name().unwrap().to_string_lossy()
    );
    // B and C are untouched by the crash and the archive.
    assert_eq!(hub.list().await.unwrap().len(), 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn cards_are_grouped_by_project_and_close_keeps_the_file() {
    let fixture = hub_with(vec![
        FauxResponse::Text("first reply".into()),
        FauxResponse::Text("second reply".into()),
    ]);
    let hub = &fixture.hub;
    let sessions_root = fixture._dir.path().join("sessions");
    let project = fixture._dir.path().join("work");
    std::fs::create_dir_all(&project).unwrap();

    let one = hub
        .create_session(Some(&project), Some("Triage"))
        .await
        .unwrap();
    // The runtime re-keys on fork; keep the original id for the assertions below.
    let one_id = one.session_id();
    prompt(&one, "hi").await;
    wait_text(&one).await;
    wait_idle(&one).await;
    let other = create(hub).await;
    prompt(&other, "hi").await;
    wait_text(&other).await;
    wait_idle(&other).await;

    let groups = hub.projects().await.unwrap();
    assert!(groups.len() == 2, "groups: {groups:#?}");
    let project_group = groups
        .iter()
        .find(|group| group.cwd == project.display().to_string())
        .expect("the project group");
    assert_eq!(project_group.name, "work");
    assert_eq!(project_group.cards.len(), 1);
    assert_eq!(project_group.cards[0].name.as_deref(), Some("Triage"));
    assert!(
        project_group.cards[0].message_count >= 2,
        "{project_group:#?}"
    );

    // The session file exists once the first message was written, under the pinned root.
    assert!(project_group.cards[0].file.starts_with(&sessions_root));
    assert!(Path::new(&project_group.cards[0].file).is_file());

    // Fork at the leaf (pi `clone`): a new session with the same conversation, listed apart,
    // and the fork becomes the stream's session (the original returns to a record).
    let forked = hub
        .fork(&one_id, None, Some("Fork of Triage"))
        .await
        .unwrap();
    assert_ne!(forked.session_id(), one_id);
    let forked_card = hub.card(&forked.session_id()).await.unwrap().unwrap();
    assert_eq!(forked_card.name.as_deref(), Some("Fork of Triage"));
    assert_ne!(forked_card.file, project_group.cards[0].file);
    assert!(forked_card.file.is_file());
    assert_eq!(hub.list().await.unwrap().len(), 3);

    // `close` disposes the runtime and keeps the record.
    hub.close(&one_id).await.unwrap();
    let card = hub.card(&one_id).await.unwrap().unwrap();
    assert_eq!(card.state, HubState::Idle);
    assert!(card.file.is_file());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn host_notices_are_retained_for_late_clients() {
    let fixture = hub_with(Vec::new());
    let hub = &fixture.hub;
    let mut events = hub.subscribe();
    hub.notice(NoticeLevel::Warning, "Safe mode: extensions are off");
    hub.notice(NoticeLevel::Info, "update ready");
    // The app's `events_open` connects after startup: the banner is replayed, not lost.
    let retained = hub.notices();
    assert_eq!(retained.len(), 2, "{retained:?}");
    assert_eq!(
        retained[0],
        HubEvent::HostNotice {
            level: NoticeLevel::Warning,
            text: "Safe mode: extensions are off".into(),
        }
    );
    // Live subscribers get them too.
    let live = events.recv().await.unwrap();
    assert!(matches!(live, HubEvent::HostNotice { .. }), "{live:?}");
}

/// v2 QA D-09: after `session_fork`, the ORIGINAL session keeps its own runtime — a prompt
/// or a model change sent through the original's open stream lands in the original's file,
/// and the fork (a new id over a new file) sees only its own conversation. The old in-place
/// fork re-keyed the shared runtime to the fork, silently wiring the original chat to it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn fork_keeps_the_original_wired_to_its_own_runtime() {
    let fixture = hub_with(vec![
        FauxResponse::Text("first reply".into()),
        FauxResponse::Text("original check".into()),
    ]);
    let hub = &fixture.hub;
    let a = create(hub).await;
    let a_id = a.session_id();
    prompt(&a, "first").await;
    wait_text(&a).await;
    wait_idle(&a).await;
    let before = state_of(&a).await.message_count;

    // Fork at the leaf: a new id over a new file, with the conversation copied.
    let b = hub.fork(&a_id, None, None).await.unwrap();
    let b_id = b.session_id();
    assert_ne!(b_id, a_id);
    // D-09's core regression: the original's handle still serves the original session
    // (the in-place fork made this backend report the fork's id).
    assert_eq!(a.session_id(), a_id);

    // A prompt through the ORIGINAL's stream answers there …
    prompt(&a, "ORIGINALCHECK").await;
    wait_text(&a).await;
    wait_idle(&a).await;
    // … lands in the original's file …
    let a_state = state_of(&a).await;
    assert_eq!(a.session_id(), a_id);
    assert!(a_state.message_count > before, "{a_state:?}");
    let a_file = std::fs::read_to_string(a_state.session_file.clone().unwrap()).unwrap();
    assert!(
        a_file.contains("ORIGINALCHECK"),
        "the prompt must reach A's file"
    );
    // … and never in the fork's.
    let b_state = state_of(&b).await;
    assert_eq!(b.session_id(), b_id);
    assert_eq!(
        b_state.message_count, before,
        "the fork must not receive the original's later turns"
    );
    let b_file = std::fs::read_to_string(b_state.session_file.clone().unwrap()).unwrap();
    assert!(!b_file.contains("ORIGINALCHECK"));
    assert_ne!(a_state.session_file, b_state.session_file);

    // A model change in the original (the Ctrl+Alt+P repro) stays out of the fork's file.
    a.send(RpcCommand::Agent(AgentCommand::SetModel {
        provider: "faux".into(),
        model_id: "faux-1".into(),
        persist: None,
    }))
    .await
    .unwrap();
    let b_file_after = std::fs::read_to_string(b_state.session_file.unwrap()).unwrap();
    assert_eq!(b_file_after, b_file, "the fork's file must not change");
}

/// E-03: a session created here (`session_new`) has no record until its first
/// conversation entry writes the file. Opening it by id before that first send must
/// wake the same live runtime instead of failing with "No session found matching"
/// (the app's "Couldn't open this session" on the first `/graph` message).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn open_before_first_write_wakes_the_created_session() {
    let fixture = hub_with(vec![FauxResponse::Text("graph reply".into())]);
    let hub = &fixture.hub;

    // A fresh chat: the runtime exists, no session file yet.
    let fresh = hub.create_session(None, None).await.unwrap();
    let id = fresh.session_id();
    assert!(hub.card(&id).await.unwrap().is_none(), "no record yet");

    // The app's first send opens the session by id (a new stream): the same live
    // runtime comes back, and the turn lands and writes the file.
    let reopened = hub.open(&id).await.expect("a live file-less session opens");
    assert_eq!(reopened.session_id(), id);
    prompt(&reopened, "hi").await;
    wait_text(&reopened).await;
    wait_idle(&reopened).await;
    let card = hub
        .card(&id)
        .await
        .expect("card lookup")
        .expect("the record exists after the first entry");
    assert!(Path::new(&card.file).is_file(), "{}", card.file.display());
    assert_eq!(card.cwd, fixture._dir.path().display().to_string());
}
