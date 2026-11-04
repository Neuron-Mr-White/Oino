//! Subagent hibernation (E-HIBERNATE, core spec §2.2, WP-C8): an idle file-persisted
//! child hibernates within `subagents.hibernateAfterSec` (its `AgentSession` is dropped,
//! `session_shutdown{hibernate}` reaches the child's extensions), a handle operation
//! wakes it and continues the same session file, `events()` subscribers prevent
//! hibernation, and parent dispose closes hibernated shells cleanly. The strongest drop
//! evidence (a weak reference to the child's session `Inner` going dead) is the unit
//! test in `oino-core/src/agents/child.rs`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use futures::StreamExt;
use indexmap::IndexMap;
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::{CallContext, CallKind, ExtensionInstance, ExtensionMode};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::session::AgentSession;
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_settings::SettingsManager;
use serde_json::{Value, json};

/// A probe extension that receives the child's `session_shutdown` events.
fn shutdown_probe() -> Arc<FakeExtension> {
    Arc::new(FakeExtension::new(
        "probe",
        oino_core::extensions::api::Registration {
            events: vec!["session_shutdown".into()],
            ..Default::default()
        },
    ))
}

async fn services_builder(
    dir: &Path,
    provider: Arc<FauxProvider>,
    probe: Arc<FakeExtension>,
    hibernate_after_sec: u64,
) -> Arc<Services> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let instances: Vec<Arc<dyn ExtensionInstance>> =
        vec![Arc::new(FauxComponent(Arc::clone(&provider))), probe];
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: dir.join("agent"),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Rpc,
        has_ui: false,
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(instances)),
        approve: Some(false),
        models: Some(models),
        settings: Some(SettingsManager::in_memory(
            json!({
                "extensionSettings": {"subagents": {"hibernateAfterSec": hibernate_after_sec}}
            })
            .as_object()
            .unwrap()
            .clone(),
        )),
    })
    .await
    .unwrap()
}

async fn parent_runtime(services: &Arc<Services>) -> AgentSessionRuntime {
    AgentSessionRuntime::create(
        Arc::clone(services),
        SessionOptions {
            source: SessionSource::New,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap()
}

fn cx(session: &AgentSession, kind: CallKind) -> CallContext {
    session.runner().call_context().with_kind(kind)
}

/// Spawns a file-persisted child that runs the probe extension only.
async fn spawn_child(
    services: &Arc<Services>,
    parent: &AgentSession,
    label: &str,
) -> Arc<oino_core::agents::ChildAgent> {
    services
        .host
        .spawn_agent(
            "probe",
            &cx(parent, CallKind::Tool),
            json!({
                "label": label,
                "persistence": {"file": {}},
                "extensions": {"only": ["probe"]},
                "tools": {"builtin": []},
            }),
        )
        .await
        .unwrap()
}

async fn until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while !f() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Reads every line of a session file as parsed JSON.
fn read_file(path: &Path) -> Vec<Value> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// Waits for an `AgentsChanged` event whose agent `id` has the given `state`.
async fn wait_agents_state(events: &mut oino_core::hub::EventStream, id: &str, state: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        for event in events.drain_now() {
            if let ClientEvent::Oino(OinoEvent::AgentsChanged { agents }) = event
                && let Some(agent) = agents.iter().find(|a| a.id == id)
                && serde_json::to_value(agent.state).unwrap() == json!(state)
            {
                return;
            }
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "no agents_changed with state {state} for {id}"
        );
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

/// An idle file-persisted child hibernates within the configured window: the shutdown
/// event reaches its extension, the shell serves cached stats, `agents_changed` reports
/// `hibernated`, and the session file keeps the transcript for the wake.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn idle_file_child_hibernates_within_the_window() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("child reply".into())]);
    let probe = shutdown_probe();
    let services = services_builder(dir.path(), Arc::clone(&provider), Arc::clone(&probe), 1).await;
    let rt = parent_runtime(&services).await;
    let parent = rt.session();
    let child = spawn_child(&services, &parent, "sleeper").await;
    assert_eq!(
        child.prompt("task one".into(), None).await.unwrap(),
        "started"
    );
    child.wait_idle().await;
    let id = child.id().to_owned();
    let file = child.session_file().unwrap();
    let mut parent_events = parent.subscribe();

    until("hibernation", || child.is_hibernated()).await;

    // The child's extension observed the hibernate shutdown.
    until("session_shutdown{hibernate} delivered", || {
        probe.received().iter().any(|(kind, payload)| {
            kind == "session_shutdown" && payload["reason"] == json!("hibernate")
        })
    })
    .await;
    // Shell state, cached reads and the agents view.
    assert_eq!(child.state(), "hibernated");
    assert!(child.session().is_none(), "the live session is gone");
    assert_eq!(child.session_file().as_deref(), Some(file.as_path()));
    let usage = child.usage();
    assert!(usage.is_object(), "cached stats: {usage}");
    assert_eq!(child.last_assistant_text().as_deref(), Some("child reply"));
    wait_agents_state(&mut parent_events, &id, "hibernated").await;
    let listed = services
        .host
        .call("probe", &cx(&parent, CallKind::Event), HostOp::ListAgents)
        .await
        .unwrap();
    assert_eq!(listed[0]["state"], json!("hibernated"));
    assert_eq!(listed[0]["sessionFile"], json!(file.display().to_string()));
    // The transcript is intact on disk for the cold resume.
    let lines = read_file(&file);
    assert!(
        lines
            .iter()
            .any(|l| l["message"]["content"].to_string().contains("task one")),
        "child file keeps the transcript: {lines:?}"
    );
    child.close(Duration::ZERO).await;
}

/// A child whose wait sources still report work (a running wake-up `bg_run`) does not
/// hibernate: hibernation disposes the session, which killed the task ("Killed during
/// Oino session shutdown/reload") while the child's run was parked on its completion
/// notification; the worker then hung `running` forever (owner graph wl-fix-2,
/// 2026-11-04). Once the work ends the child hibernates as usual.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_child_waiting_on_a_background_task_does_not_hibernate() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("waiting for the test run".into())]);
    let busy = Arc::new(std::sync::Mutex::new(Some("bg: mise run test".to_owned())));
    let probe = {
        let busy = Arc::clone(&busy);
        Arc::new(
            FakeExtension::new(
                "probe",
                oino_core::extensions::api::Registration {
                    events: vec!["session_shutdown".into()],
                    ..Default::default()
                },
            )
            .on_wait(move |_| busy.lock().unwrap().clone()),
        )
    };
    let services = services_builder(dir.path(), Arc::clone(&provider), Arc::clone(&probe), 1).await;
    let rt = parent_runtime(&services).await;
    let parent = rt.session();
    let child = spawn_child(&services, &parent, "tester").await;
    assert_eq!(
        child.prompt("run the tests".into(), None).await.unwrap(),
        "started"
    );
    child.wait_idle().await;
    // Several windows (1 s each) pass: the child stays live and its extension never saw
    // the hibernate shutdown (which is what kills the background task).
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(!child.is_hibernated(), "hibernated while a bg task ran");
    assert!(child.session().is_some());
    assert!(
        !probe
            .received()
            .iter()
            .any(|(kind, _)| kind == "session_shutdown"),
        "session_shutdown reached the child while its task ran"
    );
    // The task ends: the next due window hibernates.
    *busy.lock().unwrap() = None;
    until("hibernation after the task ended", || child.is_hibernated()).await;
    child.close(Duration::ZERO).await;
}

/// `prompt` wakes a hibernated child and continues the same child session id: the
/// provider request carries the full history and the visible state returns to idle.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn prompt_wakes_and_continues_the_same_session() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first reply".into()),
        FauxResponse::Text("second reply".into()),
    ]);
    let probe = shutdown_probe();
    let services = services_builder(dir.path(), Arc::clone(&provider), Arc::clone(&probe), 1).await;
    let rt = parent_runtime(&services).await;
    let parent = rt.session();
    let child = spawn_child(&services, &parent, "waker").await;
    assert_eq!(
        child.prompt("task one".into(), None).await.unwrap(),
        "started"
    );
    child.wait_idle().await;
    let id = child.id().to_owned();
    let file = child.session_file().unwrap();
    let mut parent_events = parent.subscribe();
    until("hibernation", || child.is_hibernated()).await;
    assert!(child.messages().is_empty(), "no live transcript asleep");

    // Wake by prompt: same agent id, same session id, transcript continues.
    assert_eq!(
        child.prompt("task two".into(), None).await.unwrap(),
        "started"
    );
    assert_eq!(child.id(), id);
    assert_eq!(child.session().expect("awake").session_id(), id);
    child.wait_idle().await;
    wait_agents_state(&mut parent_events, &id, "idle").await;
    assert_eq!(child.state(), "idle");
    let texts: Vec<String> = child
        .session()
        .expect("awake")
        .messages()
        .iter()
        .filter_map(|m| match m {
            oino_ai::AgentMessage::User(u) => Some(u.content.text("")),
            _ => None,
        })
        .collect();
    assert_eq!(texts, vec!["task one", "task two"]);
    assert_eq!(child.last_assistant_text().as_deref(), Some("second reply"));
    // The woken run fed the provider the whole (resumed) conversation.
    let request = provider.requests().last().cloned().unwrap();
    let wire = request["context"]["messages"].to_string();
    assert!(
        wire.contains("task one") && wire.contains("task two"),
        "provider saw the resumed transcript: {wire}"
    );
    // The second run was appended to the same file.
    let lines = read_file(&file);
    assert!(
        lines
            .iter()
            .any(|l| l["message"]["content"].to_string().contains("task two"))
    );
    child.close(Duration::ZERO).await;
}

/// perf-fix #4: a live `events()` subscriber does not keep the child awake. The child
/// hibernates with the subscription open, and the same stream carries the woken run's
/// events (it rides the relay instead of the disposed session's hub).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn events_subscribers_ride_hibernation() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("reply".into()),
        FauxResponse::Text("second".into()),
    ]);
    let probe = shutdown_probe();
    let services = services_builder(dir.path(), Arc::clone(&provider), Arc::clone(&probe), 1).await;
    let rt = parent_runtime(&services).await;
    let parent = rt.session();
    let child = spawn_child(&services, &parent, "watched").await;

    let mut events = child.events();
    assert_eq!(
        child.prompt("task one".into(), None).await.unwrap(),
        "started"
    );
    assert_eq!(
        events
            .next()
            .await
            .and_then(|e| e["type"].as_str().map(str::to_owned)),
        Some("agent_start".to_owned()),
        "the stream delivers live events"
    );
    child.wait_idle().await;
    until("hibernation with a live subscriber", || {
        child.is_hibernated()
    })
    .await;

    // Drain what the first run left queued, then wake the child with a second prompt.
    while let Ok(Some(_)) = tokio::time::timeout(Duration::from_millis(50), events.next()).await {}
    assert_eq!(
        child.prompt("task two".into(), None).await.unwrap(),
        "started"
    );
    let mut kinds = Vec::new();
    while let Ok(Some(event)) = tokio::time::timeout(Duration::from_secs(5), events.next()).await {
        let kind = event["type"].as_str().unwrap_or_default().to_owned();
        let done = kind == "agent_settled";
        kinds.push(kind);
        if done {
            break;
        }
    }
    assert!(kinds.iter().any(|k| k == "agent_start"), "{kinds:?}");
    assert!(kinds.iter().any(|k| k == "agent_settled"), "{kinds:?}");
    child.close(Duration::ZERO).await;
    assert!(
        tokio::time::timeout(Duration::from_secs(2), events.next())
            .await
            .unwrap()
            .is_none(),
        "close ends the stream"
    );
}

/// Parent dispose closes hibernated shells cleanly (E-LAZY keeps outranking
/// hibernation): no rebuild, no panic, the tree empties, the shell refuses prompts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn parent_dispose_closes_hibernated_shells() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("reply".into())]);
    let probe = shutdown_probe();
    let services = services_builder(dir.path(), Arc::clone(&provider), Arc::clone(&probe), 1).await;
    let rt = parent_runtime(&services).await;
    let parent = rt.session();
    let child = spawn_child(&services, &parent, "doomed").await;
    assert_eq!(
        child.prompt("task one".into(), None).await.unwrap(),
        "started"
    );
    child.wait_idle().await;
    until("hibernation", || child.is_hibernated()).await;

    parent.dispose().await;
    assert!(child.is_closed());
    assert_eq!(child.state(), "closed");
    assert_eq!(parent.agents_list(), json!([]));
    assert!(child.prompt("again".into(), None).await.is_err());
}

/// Hibernation is opt-out per settings and never applies to memory-persisted children.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn zero_setting_and_memory_children_never_hibernate() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(Vec::new());
    // hibernateAfterSec = 1, but the single child is memory-persisted.
    let probe = shutdown_probe();
    let services = services_builder(dir.path(), Arc::clone(&provider), Arc::clone(&probe), 1).await;
    let rt = parent_runtime(&services).await;
    let parent = rt.session();
    let memory = services
        .host
        .spawn_agent(
            "probe",
            &cx(&parent, CallKind::Tool),
            json!({
                "label": "ram", "tools": {"builtin": []}
            }),
        )
        .await
        .unwrap();
    assert!(memory.session_file().is_none());

    // A file-persisted child with hibernateAfterSec = 0 stays awake too.
    let services_zero = {
        let provider2 = FauxProvider::new(Vec::new());
        let probe2 = shutdown_probe();
        services_builder(dir.path(), provider2, Arc::clone(&probe2), 0).await
    };
    let rt2 = parent_runtime(&services_zero).await;
    let parent2 = rt2.session();
    let awake = spawn_child(&services_zero, &parent2, "alert").await;

    tokio::time::sleep(Duration::from_millis(2600)).await;
    assert!(!memory.is_hibernated(), "memory children never hibernate");
    assert!(!awake.is_hibernated(), "hibernateAfterSec = 0 disables it");
    assert_eq!(memory.state(), "idle");
    assert_eq!(awake.state(), "idle");
    memory.close(Duration::ZERO).await;
    awake.close(Duration::ZERO).await;
}

/// Wake repairs a damaged child file (D-REPAIR): a dangling tool call appended to the
/// file of a hibernated child is closed with the unknown-outcome toolResult before the
/// woken run, so the provider request is balanced.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn wake_repairs_a_damaged_child_file() {
    let dir = tempfile::tempdir().unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first reply".into()),
        FauxResponse::Text("second reply".into()),
    ]);
    let probe = shutdown_probe();
    let services = services_builder(dir.path(), Arc::clone(&provider), Arc::clone(&probe), 1).await;
    let rt = parent_runtime(&services).await;
    let parent = rt.session();
    let child = spawn_child(&services, &parent, "damaged").await;
    assert_eq!(
        child.prompt("task one".into(), None).await.unwrap(),
        "started"
    );
    child.wait_idle().await;
    let file = child.session_file().unwrap();
    until("hibernation", || child.is_hibernated()).await;

    // Damage the file "while the child sleeps": an assistant message with a dangling
    // tool call (as after a crash between dispatch and result).
    let lines = read_file(&file);
    let leaf = lines.last().unwrap()["id"].as_str().unwrap().to_owned();
    let torn = json!({
        "type": "message",
        "id": "dangle0001",
        "parentId": leaf,
        "timestamp": "2026-11-30T10:00:00.000Z",
        "message": {
            "role": "assistant",
            "content": [
                {"type": "text", "text": "interrupted mid-flight"},
                {"type": "toolCall", "id": "dangle1", "name": "read", "arguments": {"path": "/x"}}
            ],
            "api": "faux",
            "provider": "faux",
            "model": "faux-1",
            "thinkingLevel": "medium",
            "usage": {
                "input": 1,
                "output": 1,
                "cacheRead": 0,
                "cacheWrite": 0,
                "totalTokens": 2,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}
            },
            "stopReason": "toolUse",
            "timestamp": 1_762_000_802_000_i64
        }
    });
    let mut text = std::fs::read_to_string(&file).unwrap();
    text.push_str(&serde_json::to_string(&torn).unwrap());
    text.push('\n');
    std::fs::write(&file, text).unwrap();

    // Wake: repair closes `dangle1` and the run sees a balanced transcript.
    assert_eq!(
        child.prompt("task two".into(), None).await.unwrap(),
        "started"
    );
    child.wait_idle().await;
    let request = provider.requests().last().cloned().unwrap();
    let wire = request["context"]["messages"].to_string();
    assert!(
        wire.contains("dangle1"),
        "the dangling call reached the woken context: {wire}"
    );
    assert!(
        wire.contains("outcome is unknown"),
        "the woken run fed the repair toolResult: {wire}"
    );
    let repaired = read_file(&file);
    assert!(
        repaired
            .iter()
            .any(|l| l["customType"] == json!("oino.resume-repair")),
        "the repair record is on the child log"
    );
    child.close(Duration::ZERO).await;
}
