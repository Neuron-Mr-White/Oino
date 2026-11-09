//! E-LAZY lazy session hub (core spec §2.2, WP-C7): listing is lazy (records only, no
//! services/engines/leases), `open` wakes with lease + repair, idle eviction disposes and
//! releases the lease (guests observe `session_shutdown`), re-opening continues the
//! conversation, and a crashed session (X-PANIC) is listed `crashed` until re-opened.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use futures::future::BoxFuture;
use oino_agent::AgentToolResult;
use oino_ai::AgentMessage;
use oino_client::backend::SessionBackend;
use oino_client::hub::{HubLimits, HubOptions, HubSessionState, SessionHub};
use oino_client::protocol::{AgentCommand, RpcCommand, RpcResponseData, RpcSessionState};
use oino_core::extensions::api::{
    ExtensionInstance, ExtensionMode, Registration, ToolExposure, ToolSpec,
};
use oino_core::extensions::host::HostApi;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ExtensionLoader, ResourceFlags, StaticLoader};
use oino_core::testing::{FakeExtension, FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_session::{CreateOptions, SessionManager, lock_file_path, lock_metadata_path};
use serde_json::{Value, json};

/// A loader counting how often services were built (the laziness observable).
struct CountingLoader {
    inner: StaticLoader,
    loads: Arc<AtomicUsize>,
    /// Extension loading time (a cold wasm load in the real app takes seconds).
    delay: Duration,
}

impl ExtensionLoader for CountingLoader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        let delay = self.delay;
        Box::pin(async move {
            tokio::time::sleep(delay).await;
            self.inner.load(host, project_trusted).await
        })
    }
}

/// A `boom` tool spec (the crashing tool of the C5 tests).
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

fn hub_with(
    settings: Option<&str>,
    responses: Vec<FauxResponse>,
    extra: Vec<Arc<dyn ExtensionInstance>>,
    tools: ToolSelection,
) -> (
    tempfile::TempDir,
    Arc<SessionHub>,
    Arc<AtomicUsize>,
    Arc<FauxProvider>,
) {
    hub_with_load_delay(settings, responses, extra, tools, Duration::ZERO)
}

fn hub_with_load_delay(
    settings: Option<&str>,
    responses: Vec<FauxResponse>,
    extra: Vec<Arc<dyn ExtensionInstance>>,
    tools: ToolSelection,
    delay: Duration,
) -> (
    tempfile::TempDir,
    Arc<SessionHub>,
    Arc<AtomicUsize>,
    Arc<FauxProvider>,
) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
    std::fs::create_dir_all(dir.path().join("agent")).unwrap();
    if let Some(settings) = settings {
        std::fs::write(dir.path().join("agent/settings.json"), settings).unwrap();
    }
    let provider = FauxProvider::new(responses);
    let loads = Arc::new(AtomicUsize::new(0));
    let mut instances: Vec<Arc<dyn ExtensionInstance>> =
        vec![Arc::new(FauxComponent(Arc::clone(&provider)))];
    instances.extend(extra);
    let loader = CountingLoader {
        inner: StaticLoader(instances),
        loads: Arc::clone(&loads),
        delay,
    };
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let hub = SessionHub::create(HubOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir: dir.path().join("agent"),
        docs_root: None,
        session_dir: Some(dir.path().join("sessions")),
        mode: ExtensionMode::Rpc,
        has_ui: false,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        loader: Arc::new(loader),
        models: Some(models),
        tools,
        exclude_tools: Vec::new(),
    })
    .unwrap();
    (dir, hub, loads, provider)
}

/// A session file with one user/assistant turn and the faux model recorded.
fn conversation_file(dir: &Path) -> (PathBuf, String) {
    let sessions = dir.join("sessions");
    let mut manager = SessionManager::create(dir, &sessions, CreateOptions::default()).unwrap();
    manager.append_model_change("faux", "faux-1").unwrap();
    manager.append_thinking_level_change("off").unwrap();
    manager
        .append_message(AgentMessage::User(oino_ai::UserMessage {
            author: None,
            content: oino_ai::UserContentField::Text("seed question".into()),
            timestamp: 1,
        }))
        .unwrap();
    let mut assistant = oino_ai::AssistantMessage::empty("faux", "faux", "faux-1", 2);
    assistant.content = vec![oino_ai::AssistantContent::text("seed answer")];
    manager
        .append_message(AgentMessage::Assistant(assistant))
        .unwrap();
    (
        PathBuf::from(manager.session_file().unwrap()),
        manager.session_id().to_owned(),
    )
}

/// A session file whose turn was interrupted after tool `c1` but before `c2`
/// (C2's `interrupted_file` shape): re-opening must repair the dangling call.
fn interrupted_file(dir: &Path) -> (PathBuf, String) {
    let sessions = dir.join("sessions");
    let mut manager = SessionManager::create(dir, &sessions, CreateOptions::default()).unwrap();
    manager
        .append_message(AgentMessage::User(oino_ai::UserMessage {
            author: None,
            content: oino_ai::UserContentField::Text("run probe".into()),
            timestamp: 1,
        }))
        .unwrap();
    let mut assistant = oino_ai::AssistantMessage::empty("faux", "faux", "faux-1", 2);
    assistant.content = vec![
        oino_ai::AssistantContent::text("running"),
        oino_ai::AssistantContent::ToolCall(oino_ai::ToolCall {
            id: "c1".into(),
            name: "probe".into(),
            ..oino_ai::ToolCall::default()
        }),
        oino_ai::AssistantContent::ToolCall(oino_ai::ToolCall {
            id: "c2".into(),
            name: "probe".into(),
            ..oino_ai::ToolCall::default()
        }),
    ];
    assistant.stop_reason = oino_ai::StopReason::ToolUse;
    manager
        .append_message(AgentMessage::Assistant(assistant))
        .unwrap();
    manager
        .append_message(AgentMessage::ToolResult(oino_ai::ToolResultMessage {
            tool_call_id: "c1".into(),
            tool_name: "probe".into(),
            content: vec![oino_ai::UserContent::text("ok")],
            details: None,
            usage: None,
            nested_calls: None,
            is_error: false,
            timestamp: 3,
            duration_ms: None,
        }))
        .unwrap();
    (
        PathBuf::from(manager.session_file().unwrap()),
        manager.session_id().to_owned(),
    )
}

async fn state_of(backend: &Arc<dyn SessionBackend>) -> RpcSessionState {
    let data: RpcResponseData = backend
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await
        .unwrap();
    serde_json::from_value(data.unwrap()).unwrap()
}

async fn wait_idle(backend: &Arc<dyn SessionBackend>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let state = state_of(backend).await;
        if !state.is_streaming && !state.is_compacting {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "session never went idle"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn wait_state(hub: &SessionHub, id: &str, want: HubSessionState) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let listed = hub.list().await.unwrap();
        let found = listed.iter().find(|s| s.info.id == id);
        if let Some(found) = found
            && found.state == want
        {
            return;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "session {id} never reached {want:?}: {listed:?}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

async fn prompt(backend: &Arc<dyn SessionBackend>, message: &str) {
    backend
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: message.into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    wait_idle(backend).await;
}

/// Listing shows records without engines/leases (lazy); opening acquires the lease,
/// repairs the interrupted turn and lists `live`; closing returns it to a record.
#[tokio::test]
async fn hub_limits_override_and_default_keeps_the_max_services_setting() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
    std::fs::create_dir_all(dir.path().join("agent")).unwrap();
    std::fs::write(
        dir.path().join("agent/settings.json"),
        r#"{"extensionSettings":{"hub":{"maxServices":8}}}"#,
    )
    .unwrap();
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let options = |models: Option<ModelRegistry>| HubOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir: dir.path().join("agent"),
        docs_root: None,
        session_dir: Some(dir.path().join("sessions")),
        mode: ExtensionMode::Rpc,
        has_ui: false,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        loader: Arc::new(StaticLoader(Vec::new())),
        models,
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
    };
    let capped = SessionHub::create_with(
        options(Some(models.clone())),
        HubLimits::default().with_max_services(2),
    )
    .unwrap();
    assert_eq!(
        capped.max_services(),
        2,
        "the host limit wins over the setting"
    );
    let plain = SessionHub::create(options(None)).unwrap();
    assert_eq!(plain.max_services(), 8, "default keeps the setting");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn list_never_waits_for_a_wake() {
    // sidebar-data: at every app start the first `sessions_list` raced the restored
    // chat's open, whose wake holds the hub lock through services creation (cold
    // extension loading: seconds) - the Task list showed its rows ~12 s late.
    let (dir, hub, _loads, _provider) = hub_with_load_delay(
        None,
        Vec::new(),
        Vec::new(),
        ToolSelection::Default,
        Duration::from_secs(3),
    );
    let (_one, one_id) = conversation_file(dir.path());
    let (_two, _two_id) = conversation_file(dir.path());
    let waking = {
        let hub = Arc::clone(&hub);
        let id = one_id.clone();
        tokio::spawn(async move { hub.open(&id).await.map(|_| ()) })
    };
    // The wake has taken the lock and is loading extensions.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(hub.is_busy(), "the wake holds the hub lock");
    let started = std::time::Instant::now();
    let listed = hub.list().await.unwrap();
    assert!(
        started.elapsed() < Duration::from_millis(1000),
        "list waited {:?} for the wake",
        started.elapsed()
    );
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().all(|s| s.state == HubSessionState::Record));
    waking.await.unwrap().unwrap();
    // Once the wake is done the listing has the lock again: the woken session is live.
    let listed = hub.list().await.unwrap();
    let live = listed.iter().find(|s| s.info.id == one_id).unwrap();
    assert_eq!(live.state, HubSessionState::Live);
}

#[tokio::test]
async fn list_is_lazy_open_wakes_with_lease_and_repair() {
    let (dir, hub, loads, _provider) =
        hub_with(None, Vec::new(), Vec::new(), ToolSelection::Default);
    let (clean, _clean_id) = conversation_file(dir.path());
    let (torn, torn_id) = interrupted_file(dir.path());

    let listed = hub.list().await.unwrap();
    assert_eq!(listed.len(), 2, "{listed:?}");
    assert!(listed.iter().all(|s| s.state == HubSessionState::Record));
    assert_eq!(
        loads.load(Ordering::SeqCst),
        0,
        "a lazy list builds no services"
    );
    for path in [&clean, &torn] {
        assert!(!lock_file_path(path).exists(), "no kernel lock file");
        assert!(!lock_metadata_path(path).exists(), "no lease metadata");
    }

    // Two sessions created in the same millisecond share their uuid7 timestamp prefix:
    // the prefix lookup reports the ambiguity instead of guessing.
    let ambiguous = hub
        .open(&torn_id[..6])
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(ambiguous.contains("Ambiguous session id"), "{ambiguous}");

    // Wake by exact id: acquire lease → open → repair → live.
    let backend = hub.open(&torn_id).await.unwrap();
    assert_eq!(backend.session_id(), torn_id);
    assert!(
        loads.load(Ordering::SeqCst) >= 1,
        "waking builds the services once"
    );
    assert!(lock_metadata_path(&torn).exists(), "the lease is held");
    let text = std::fs::read_to_string(&torn).unwrap();
    assert!(
        text.contains("interrupted before completion"),
        "D-REPAIR closed the dangling tool call: {text}"
    );
    wait_state(&hub, &torn_id, HubSessionState::Live).await;

    // Close: back to a record with the lease released.
    hub.close(&torn_id).await.unwrap();
    assert!(
        !lock_metadata_path(&torn).exists(),
        "close releases the lease"
    );
    wait_state(&hub, &torn_id, HubSessionState::Record).await;

    // Unknown and too-short targets fail precisely.
    let missing = hub.open("no-such-session").await;
    let missing = missing
        .err()
        .map(|error| error.to_string())
        .unwrap_or_default();
    assert!(missing.contains("No session found matching"), "{missing}");
    let short = hub
        .open(&torn_id[..2])
        .await
        .err()
        .map(|e| e.to_string())
        .unwrap_or_default();
    assert!(short.contains("No session found matching"), "{short}");
}

/// `create_session` (WP-H3): a new runtime is live immediately (no file until the first
/// message, core durability rule), a later `open` reuses that runtime instead of re-waking the
/// record, and the named session is listed once its first message wrote the file.
#[tokio::test]
async fn create_session_is_live_reused_and_listed_after_the_first_message() {
    let (dir, hub, loads, _provider) = hub_with(
        None,
        vec![FauxResponse::Text("fresh reply".into())],
        Vec::new(),
        ToolSelection::Default,
    );
    let backend = hub.create_session(None, Some("Fresh")).await.unwrap();
    let id = backend.session_id();
    backend
        .send(
            AgentCommand::SetModel {
                provider: "faux".into(),
                model_id: "faux-1".into(),
                persist: None,
            }
            .into(),
        )
        .await
        .unwrap();
    assert_eq!(
        loads.load(Ordering::SeqCst),
        1,
        "create builds the services once"
    );
    assert!(
        hub.list().await.unwrap().is_empty(),
        "a new session has no file before its first message"
    );

    // A second handle by id reuses the live runtime (one services set, no re-wake).
    let same = hub.open(&id).await.unwrap();
    assert_eq!(same.session_id(), id);
    assert_eq!(loads.load(Ordering::SeqCst), 1);

    prompt(&same, "hello").await;
    let listed = hub.list().await.unwrap();
    let created = listed
        .iter()
        .find(|session| session.info.id == id)
        .unwrap_or_else(|| panic!("the created session is listed: {listed:?}"));
    assert_eq!(created.info.name.as_deref(), Some("Fresh"));
    assert!(created.info.message_count >= 2, "{created:?}");
    assert_eq!(created.state, HubSessionState::Live);
    assert!(
        created.info.path.is_file(),
        "the file exists after a message"
    );
    assert!(created.info.path.starts_with(dir.path().join("sessions")));

    // Closing returns it to a record; the file stays.
    hub.close(&id).await.unwrap();
    wait_state(&hub, &id, HubSessionState::Record).await;
    assert!(!lock_metadata_path(&created.info.path).exists());
    assert!(created.info.path.is_file());
}

/// Idle eviction (short `sessionIdleSec`): disposes the session, releases the lease,
/// clears guest state (`session_shutdown{reason:"idle"}` observed by the fixture), and a
/// re-open continues the conversation with no lost entries.
#[tokio::test]
async fn idle_eviction_disposes_releases_and_reopen_continues() {
    let observer = Arc::new(FakeExtension::new(
        "observer",
        Registration {
            events: vec!["session_shutdown".into()],
            ..Registration::default()
        },
    ));
    let (dir, hub, _loads, _provider) = hub_with(
        Some(r#"{"extensionSettings":{"hub":{"sessionIdleSec":1}}}"#),
        vec![
            FauxResponse::Text("first reply".into()),
            FauxResponse::Text("second reply".into()),
        ],
        vec![Arc::clone(&observer) as Arc<dyn ExtensionInstance>],
        ToolSelection::Default,
    );
    let (file, id) = conversation_file(dir.path());

    let backend = hub.open(&id).await.unwrap();
    prompt(&backend, "hello").await;
    let after_first = state_of(&backend).await;
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("first reply"), "{text}");
    drop(backend);

    // No run, no compaction, no children, no subscribers: evicted back to a record.
    wait_state(&hub, &id, HubSessionState::Record).await;
    assert!(
        !lock_metadata_path(&file).exists(),
        "eviction releases the lease"
    );

    // Guest state was cleared: the fixture observed the shutdown.
    let received = observer.received();
    let shutdown = received
        .iter()
        .find(|(kind, _)| kind == "session_shutdown")
        .unwrap_or_else(|| panic!("fixture did not observe session_shutdown: {received:?}"));
    let reason = shutdown.1.get("reason").and_then(Value::as_str);
    assert_eq!(reason, Some("idle"), "{received:?}");

    // Re-open: same session, no lost entries, and the conversation continues.
    let backend = hub.open(&id).await.unwrap();
    let resumed = state_of(&backend).await;
    assert_eq!(
        resumed.message_count, after_first.message_count,
        "no entries lost across eviction"
    );
    prompt(&backend, "again").await;
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("first reply"), "{text}");
    assert!(text.contains("second reply"), "{text}");
    assert!(
        state_of(&backend).await.message_count >= resumed.message_count + 2,
        "the conversation continues after the eviction"
    );
    wait_state(&hub, &id, HubSessionState::Live).await;
}

/// A running background task keeps the session live (spec risk 6): the idle monitor must
/// not evict a session with an open `!` command.
#[tokio::test]
async fn running_bash_prevents_idle_eviction() {
    let (dir, hub, _loads, _provider) = hub_with(
        Some(
            r#"{"autonomy":{"defaultLevel":"high"},"extensionSettings":{"hub":{"sessionIdleSec":1}}}"#,
        ),
        Vec::new(),
        Vec::new(),
        ToolSelection::Default,
    );
    let (_file, id) = conversation_file(dir.path());
    let backend = hub.open(&id).await.unwrap();
    let sender = Arc::clone(&backend);
    let bash = tokio::spawn(async move {
        sender
            .send(RpcCommand::Agent(AgentCommand::Bash {
                command: "sleep 3".into(),
                exclude_from_context: Some(true),
            }))
            .await
    });

    // Well past the 1 s idle window while the command runs: still live.
    tokio::time::sleep(Duration::from_millis(2000)).await;
    let listed = hub.list().await.unwrap();
    assert_eq!(
        listed.iter().find(|s| s.info.id == id).unwrap().state,
        HubSessionState::Live,
        "an open `!` command keeps the session live"
    );

    bash.await.unwrap().unwrap();
    wait_state(&hub, &id, HubSessionState::Record).await;
}

/// A crashed session (X-PANIC) is listed `crashed` until re-opened; the live session
/// stays usable and the re-opened record clears the marker.
#[tokio::test]
async fn crashed_session_is_listed_crashed_until_reopened() {
    let boom = Arc::new(
        FakeExtension::new(
            "boom-ext",
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
        }),
    );
    let (dir, hub, _loads, _provider) = hub_with(
        None,
        vec![
            FauxResponse::ToolCalls(vec![(
                "c1".into(),
                "boom".into(),
                serde_json::from_value(json!({})).unwrap(),
            )]),
            FauxResponse::Text("recovered".into()),
        ],
        vec![Arc::clone(&boom) as Arc<dyn ExtensionInstance>],
        ToolSelection::List(vec!["boom".into()]),
    );
    let (_file, id) = conversation_file(dir.path());

    let backend = hub.open(&id).await.unwrap();
    prompt(&backend, "go").await;
    wait_state(&hub, &id, HubSessionState::Crashed).await;

    // X-PANIC containment: the live session still accepts a prompt.
    prompt(&backend, "once more").await;
    let still = hub.list().await.unwrap();
    assert_eq!(
        still.iter().find(|s| s.info.id == id).unwrap().state,
        HubSessionState::Crashed,
        "the marker stands until re-opened"
    );

    // Close, then re-open the record: the marker clears.
    hub.close(&id).await.unwrap();
    let backend = hub.open(&id).await.unwrap();
    assert_eq!(backend.session_id(), id);
    wait_state(&hub, &id, HubSessionState::Live).await;
}

/// A services set with zero live sessions for `servicesIdleSec` is disposed (extension
/// `shutdown`) and rebuilt on the next wake.
#[tokio::test]
async fn idle_services_set_is_disposed_and_rebuilt_on_demand() {
    let (dir, hub, loads, _provider) = hub_with(
        Some(r#"{"extensionSettings":{"hub":{"sessionIdleSec":1,"servicesIdleSec":1}}}"#),
        vec![
            FauxResponse::Text("one".into()),
            FauxResponse::Text("two".into()),
        ],
        Vec::new(),
        ToolSelection::Default,
    );
    let (_file, id) = conversation_file(dir.path());

    let backend = hub.open(&id).await.unwrap();
    prompt(&backend, "hello").await;
    drop(backend);
    assert_eq!(loads.load(Ordering::SeqCst), 1);
    wait_state(&hub, &id, HubSessionState::Record).await;

    // Past servicesIdleSec with no live session: the set is disposed; the next wake
    // rebuilds it (the counter is the observable).
    tokio::time::sleep(Duration::from_secs(3)).await;
    let backend = hub.open(&id).await.unwrap();
    assert_eq!(
        loads.load(Ordering::SeqCst),
        2,
        "the idle services set was disposed and rebuilt"
    );
    prompt(&backend, "again").await;
}

/// owner-fix-6 P1-3: the `maxServices` LRU retired the session a `session_open` had just
/// woken (no stream attached yet, so it read as idle) to make room for the next services
/// request 11 ms later (the host's hub-level `_oino/*` commands load the primary cwd's
/// set): the chat answered "could not start: backend closed" until the app reloaded. A
/// session woken (or used) moments ago is never the victim while the cap may grow by one.
#[tokio::test]
async fn services_cap_never_retires_a_session_it_just_woke() {
    let (dir, hub, _loads, _provider) = hub_with(
        Some(r#"{"extensionSettings":{"hub":{"maxServices":1}}}"#),
        Vec::new(),
        Vec::new(),
        ToolSelection::Default,
    );
    // A chat in another project than the hub's primary cwd: its own services set.
    let project = dir.path().join("project-a");
    std::fs::create_dir_all(&project).unwrap();
    let sessions = dir.path().join("sessions");
    let mut manager =
        SessionManager::create(&project, &sessions, CreateOptions::default()).unwrap();
    manager.append_model_change("faux", "faux-1").unwrap();
    manager
        .append_message(AgentMessage::User(oino_ai::UserMessage {
            author: None,
            content: oino_ai::UserContentField::Text("seed".into()),
            timestamp: 1,
        }))
        .unwrap();
    let mut assistant = oino_ai::AssistantMessage::empty("faux", "faux", "faux-1", 2);
    assistant.content = vec![oino_ai::AssistantContent::text("seed answer")];
    manager
        .append_message(AgentMessage::Assistant(assistant))
        .unwrap();
    let id = manager.session_id().to_owned();
    drop(manager);

    // The open wakes it (one services set: the cap) — the stream has not attached yet.
    let backend = hub.open(&id).await.unwrap();
    // A services request for the primary cwd right after (a hub-level command).
    hub.services().await.unwrap();
    // The just-woken session is still live and answers.
    let state = backend
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await;
    assert!(state.is_ok(), "the woken session was retired: {state:?}");
    assert_eq!(
        hub.find(&id).await.unwrap().state,
        HubSessionState::Live,
        "still live"
    );
}

/// A tool that only the embedding host knows about (the desktop's lead board tool).
struct HostTool {
    meta: oino_agent::ToolDefinitionMeta,
    calls: Arc<AtomicUsize>,
}

impl oino_agent::AgentTool for HostTool {
    fn meta(&self) -> &oino_agent::ToolDefinitionMeta {
        &self.meta
    }
    fn execute(
        &self,
        _: oino_agent::ToolInvocation,
        _: oino_agent::UpdateSink,
        _: tokio_util::sync::CancellationToken,
    ) -> BoxFuture<'static, Result<AgentToolResult, oino_agent::ToolError>> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async { Ok(AgentToolResult::text("host tool ran")) })
    }
}

/// `HubLimits::with_session_tools`: a session woken from its file gets the host's extra
/// tools for that file (a reopened dispatcher lead keeps its board tool), and the hook is
/// asked with the file and id it wakes.
#[tokio::test]
async fn session_tools_are_attached_when_a_record_wakes() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("sessions")).unwrap();
    std::fs::create_dir_all(dir.path().join("agent")).unwrap();
    let (file, id) = conversation_file(dir.path());
    let provider = FauxProvider::new(vec![
        FauxResponse::ToolCalls(vec![(
            "c1".into(),
            "host_tool".into(),
            serde_json::from_value(json!({})).unwrap(),
        )]),
        FauxResponse::Text("done".into()),
    ]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let calls = Arc::new(AtomicUsize::new(0));
    let asked = Arc::new(std::sync::Mutex::new(Vec::new()));
    let hook = {
        let calls = Arc::clone(&calls);
        let asked = Arc::clone(&asked);
        oino_client::hub::SessionTools(Arc::new(move |path: &Path, session: &str| {
            asked
                .lock()
                .unwrap()
                .push((path.to_path_buf(), session.to_owned()));
            vec![oino_core::session::ToolEntry::builtin(
                Arc::new(HostTool {
                    meta: oino_agent::ToolDefinitionMeta::new(
                        "host_tool",
                        "Host tool",
                        "host tool",
                        json!({"type": "object"}),
                    ),
                    calls: Arc::clone(&calls),
                }),
                None,
                Vec::new(),
            )]
        }))
    };
    let hub = SessionHub::create_with(
        HubOptions {
            cwd: dir.path().to_path_buf(),
            agent_dir: dir.path().join("agent"),
            docs_root: None,
            session_dir: Some(dir.path().join("sessions")),
            mode: ExtensionMode::Rpc,
            has_ui: false,
            approve: Some(false),
            flags: ResourceFlags {
                no_context_files: true,
                ..ResourceFlags::default()
            },
            loader: Arc::new(StaticLoader(vec![
                Arc::new(FauxComponent(Arc::clone(&provider))) as Arc<dyn ExtensionInstance>,
            ])),
            models: Some(models),
            tools: ToolSelection::Default,
            exclude_tools: Vec::new(),
        },
        HubLimits::default().with_session_tools(hook),
    )
    .unwrap();

    let backend = hub.open(&id).await.unwrap();
    assert_eq!(*asked.lock().unwrap(), vec![(file, id.clone())]);
    prompt(&backend, "use the host tool").await;
    assert_eq!(calls.load(Ordering::SeqCst), 1, "the woken session ran the host tool");
}
