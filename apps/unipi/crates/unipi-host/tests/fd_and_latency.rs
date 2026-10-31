//! v2 QA defect tests (host.md fixes): descriptor exhaustion and slow session opens.
//!
//! 1. **fd exhaustion** — every cached services set holds one wasm instance per
//!    extension, each with preopen directory and log file descriptors (~290 fds for a
//!    31-extension install), so the hub's `maxServices` must fit the process's
//!    `RLIMIT_NOFILE` ([`unipi_host::fd_budget`]). The open/close cycle itself must
//!    also stay descriptor-neutral: streams, feeds, taps, supervisors and watchers all
//!    end with their session.
//! 2. **slow opens** — `session_open` of a large existing session answers its snapshot
//!    without rebuilding services on the click path, and provider `list-models` probes
//!    (dead local servers) never block it.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt as _;
use oino_ai::message::{AgentMessage, UserMessage};
use oino_ai::{JsonValue, Model, ProviderEvent};
use oino_client::protocol::{AgentCommand, RpcCommand};
use oino_core::extensions::api::{
    CallContext, ExtensionCallError, ExtensionInstance, Registration, ToolExposure, ToolSpec,
};
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;
use oino_session::{
    EntryId, EntryKind, SessionEntry, SessionHeader, SessionManager, CURRENT_SESSION_VERSION,
};
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use unipi_host::sessions::card::HubState;
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::SessionService;
use unipi_proto::frame::{encode, FrameKind, MAX_FRAME};
use unipi_proto::{Control, Request};

/// Generous bound so a loaded CI machine does not flake; the measured numbers live in
/// the QA report (warm reopen ~20 ms, cold wake of a 1000-entry session ~160-420 ms).
const OPEN_BUDGET: Duration = Duration::from_millis(1500);
/// How many open/close cycles the fd test drives through the real stream handler.
const OPEN_CYCLES: usize = 25;

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
    service: Arc<SessionService>,
    sessions: PathBuf,
}

/// A host hub in Desktop mode whose sessions are answered by the faux provider, plus
/// the session service over it (the same wiring `serve` installs).
async fn fixture(
    services_cap: Option<usize>,
    instances: Vec<Arc<dyn ExtensionInstance>>,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let loader = StaticLoader(instances);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir: agent_dir.clone(),
        docs_root: None,
        session_dir: Some(sessions.clone()),
        mode: oino_core::extensions::api::ExtensionMode::Desktop,
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
        services_cap,
        host_data_dir: None,
    })
    .unwrap();
    let service = SessionService::new(Arc::clone(&hub));
    Fixture {
        _dir: dir,
        hub,
        service,
        sessions,
    }
}

async fn faux_fixture(services_cap: Option<usize>) -> Fixture {
    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    fixture(
        services_cap,
        vec![Arc::new(FauxComponent(Arc::clone(&provider)))],
    )
    .await
}

/// One client end of a session stream served by the real handler (as `Handler::handle`
/// does after the hello frame).
struct Client {
    read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    task: tokio::task::JoinHandle<Result<(), unipi_proto::AppErrorDto>>,
}

impl Client {
    fn open(service: &Arc<SessionService>, req: Request) -> Self {
        let (client, server) = tokio::io::duplex(1 << 20);
        let service = Arc::clone(service);
        let task = tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(server);
            unipi_host::sessions::stream::serve_request(&service, req, &mut read, &mut write).await
        });
        let (read, write) = tokio::io::split(client);
        Self { read, write, task }
    }

    async fn frame(&mut self) -> (u8, Value) {
        let mut header = [0u8; 5];
        self.read.read_exact(&mut header).await.unwrap();
        let kind = header[0];
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        assert!(len <= MAX_FRAME, "oversize frame");
        let mut payload = vec![0u8; len];
        self.read.read_exact(&mut payload).await.unwrap();
        let value = serde_json::from_slice(&payload).unwrap();
        (kind, value)
    }

    /// The snapshot `ok` (or the `err` the hub answered).
    async fn first_reply(&mut self) -> Value {
        tokio::time::timeout(Duration::from_secs(20), self.frame())
            .await
            .expect("timed out waiting for the host")
            .1
    }

    async fn send(&mut self, control: &Control) {
        let bytes = serde_json::to_vec(control).unwrap();
        self.write
            .write_all(&encode(FrameKind::Control, &bytes))
            .await
            .unwrap();
    }

    /// Sends one `session_cmd` and waits for its correlated reply (skipping events).
    async fn cmd(&mut self, id: u64, cmd: Value) -> Value {
        self.send(&Control::SessionCmd {
            id: Some(json!(id)),
            cmd,
        })
        .await;
        loop {
            let (_, value) = tokio::time::timeout(Duration::from_secs(20), self.frame())
                .await
                .expect("timed out waiting for the reply");
            let kind = value.get("type").and_then(Value::as_str);
            if matches!(kind, Some("ok") | Some("err"))
                && value.get("id").and_then(Value::as_u64) == Some(id)
            {
                return value;
            }
        }
    }

    async fn close(mut self) {
        let _ = self.write.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

#[cfg(target_os = "linux")]
fn fd_count() -> usize {
    std::fs::read_dir("/proc/self/fd")
        .map(|entries| entries.count())
        .unwrap_or(0)
}

/// Creates a session with one faux exchange, waits for it to go idle and returns its id.
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
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(card)) = hub.card(&id).await {
            if !matches!(card.state, HubState::Working) {
                break;
            }
        }
        assert!(Instant::now() < deadline, "session never went idle");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    id
}

/// Writes `count` linear user-message entries under a fresh session file in `cwd`'s
/// directory and returns the session id.
fn synthetic_session(sessions_root: &Path, cwd: &Path, count: usize) -> String {
    let id = format!("01aaaaaa-bbbb-7ccc-8ddd-{:012x}", count);
    let dir = oino_session::manager::cwd_session_dir(sessions_root, cwd);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("2026-10-31T00-00-00-000Z_{id}.jsonl"));
    let header = SessionHeader {
        version: CURRENT_SESSION_VERSION,
        id: id.clone(),
        timestamp: oino_session::IsoTimestamp::from_ms(1_799_200_000_000),
        cwd: cwd.display().to_string(),
        parent_session: None,
    };
    let mut parent: Option<EntryId> = None;
    let mut lines = vec![serde_json::to_string(&header).unwrap()];
    for index in 0..count {
        let entry = SessionEntry {
            id: EntryId::new(format!("e{index:04}")),
            parent_id: parent.clone(),
            timestamp: oino_session::IsoTimestamp::from_ms(
                1_799_200_000_000 + (index as i64) * 1000,
            ),
            kind: EntryKind::Message {
                message: AgentMessage::User(UserMessage {
                    content: format!("synthetic message {index}").into(),
                    timestamp: index as i64,
                }),
            },
        };
        parent = Some(entry.id.clone());
        lines.push(serde_json::to_string(&entry).unwrap());
    }
    std::fs::write(&file, lines.join("\n") + "\n").unwrap();
    id
}

/// The fd table stays bounded while the real stream handler serves many session
/// open/close cycles, git watchers toggle and fs stats run — nothing the host does per
/// open may keep a descriptor (os error 24: `app.sock accept failed`).
#[cfg(target_os = "linux")]
#[tokio::test]
async fn many_opens_keep_the_fd_table_bounded() {
    let fixture = faux_fixture(None).await;
    let id = session_with_a_message(&fixture.hub).await;

    // The watcher needs a git repo to resolve (the `git_watch` op runs `rev-parse`).
    let repo = tempfile::tempdir().unwrap();
    let git = std::process::Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["init", "--quiet"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if !git {
        eprintln!("skipping the git-watch part: no git in this environment");
    }

    // Baseline after one full open (services built, one live session).
    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let reply = client.first_reply().await;
    assert_eq!(reply["type"], "ok", "first open: {reply}");
    client.close().await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let baseline = fd_count();

    for cycle in 0..OPEN_CYCLES {
        let mut client = Client::open(
            &fixture.service,
            Request::SessionOpen {
                session: id.clone().try_into().unwrap(),
                since_seq: None,
            },
        );
        let reply = client.first_reply().await;
        assert_eq!(reply["type"], "ok", "open {cycle}: {reply}");
        // The accordion-expand fetch: `get_entries` over the open stream.
        let entries = client
            .cmd(cycle as u64, json!({"type": "get_entries"}))
            .await;
        assert_eq!(entries["type"], "ok", "get_entries {cycle}: {entries}");
        client.close().await;

        // `git_watch` on/off (the op's own refcounted registry) and fs stats.
        if git {
            let root = unipi_host::git_watch::GitWatchers::shared()
                .set(repo.path().to_path_buf(), true)
                .await;
            assert!(root.is_some(), "the temp repo resolves");
            let off = unipi_host::git_watch::GitWatchers::shared()
                .set(repo.path().to_path_buf(), false)
                .await;
            assert!(off.is_some());
        }
        let _ = unipi_host::fs::stat(Path::new(env!("CARGO_MANIFEST_DIR"))).await;
        let _ = unipi_host::fs::list(Path::new(env!("CARGO_MANIFEST_DIR")), false).await;
    }
    // Let the feed taps and supervisors notice the closed streams.
    tokio::time::sleep(Duration::from_millis(500)).await;

    let after = fd_count();
    assert!(
        after <= baseline + 8,
        "fd table grew across {OPEN_CYCLES} open/close cycles: {baseline} -> {after}"
    );
}

/// The hub's `maxServices` override reaches the core hub: a second concurrent project
/// beyond the cap fails with the core hub's actionable text instead of exhausting the
/// descriptor table (the crash path), and the first project keeps working.
#[tokio::test]
async fn services_cap_bounds_concurrent_project_sets() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = faux_fixture(Some(1)).await;

    let first = fixture
        .hub
        .create_session(Some(&dir.path().join("a")), None)
        .await
        .unwrap();
    let second = fixture
        .hub
        .create_session(Some(&dir.path().join("b")), None)
        .await;
    assert!(
        second.is_err(),
        "the second project must not build services"
    );
    let error = second.err().unwrap().to_string();
    assert!(
        error.contains("cache is full"),
        "the failure names the budget: {error}"
    );

    // Closing the first session's runtime frees its services for the next project.
    fixture.hub.close(&first.session_id()).await.unwrap();
    let reopened = fixture
        .hub
        .create_session(Some(&dir.path().join("b")), None)
        .await;
    assert!(reopened.is_ok(), "after closing: {reopened:?}");
}

/// A 1000-entry session opens with its snapshot well inside the QA budget — no runtime
/// rebuild on the entries fetch, no services rebuild while the set stays cached.
#[tokio::test]
async fn session_open_of_a_1000_entry_session_answers_fast() {
    let fixture = faux_fixture(None).await;
    let project = fixture.sessions.parent().unwrap().join("project");
    std::fs::create_dir_all(&project).unwrap();
    let id = synthetic_session(&fixture.sessions, &project, 1000);

    // Warm the project's services off the click path (the host's background pre-warm).
    fixture
        .hub
        .core_services_for(&project)
        .await
        .expect("warm services");

    let start = Instant::now();
    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let reply = client.first_reply().await;
    let open = start.elapsed();
    assert_eq!(reply["type"], "ok", "open: {reply}");
    let entries = reply
        .pointer("/data/entries")
        .and_then(Value::as_array)
        .map(Vec::len)
        .unwrap_or_default();
    assert!(
        entries >= 1000,
        "the snapshot carries the branch: {entries}"
    );
    assert!(
        open < OPEN_BUDGET,
        "session_open took {open:?} (budget {OPEN_BUDGET:?})"
    );

    // The expand fetch rides the live stream: no runtime rebuild, single command.
    let start = Instant::now();
    let reply = client.cmd(1, json!({"type": "get_entries"})).await;
    let fetch = start.elapsed();
    assert_eq!(reply["type"], "ok");
    assert!(
        fetch < Duration::from_millis(300),
        "get_entries took {fetch:?}"
    );
    client.close().await;

    // Reopening while the runtime is live is the app's every-click path: near-instant.
    let start = Instant::now();
    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.try_into().unwrap(),
            since_seq: None,
        },
    );
    let reply = client.first_reply().await;
    assert_eq!(reply["type"], "ok");
    assert!(
        start.elapsed() < Duration::from_millis(300),
        "reopen while live: {:?}",
        start.elapsed()
    );
    client.close().await;
}

/// Dead local providers must not tax `session_open`: with a dynamic provider whose
/// `list-models` hangs, services for a Desktop-mode host still come up immediately
/// (the refresh runs in the background and lands when it completes).
#[tokio::test]
async fn hanging_list_models_does_not_block_services() {
    use std::sync::atomic::{AtomicBool, Ordering};

    /// When the slow provider's `list-models` finished (the background refresh ran).
    static LISTED: AtomicBool = AtomicBool::new(false);

    /// A provider registration (`models: "dynamic"`, no key needed) whose listing is slow.
    struct SlowProvider;
    impl oino_core::extensions::api::ProviderInstance for SlowProvider {
        fn stream(
            &self,
            _request: JsonValue,
            _cancel: oino_extension_api::CancellationToken,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<
                futures_util::stream::BoxStream<'static, Result<ProviderEvent, ExtensionCallError>>,
                ExtensionCallError,
            >,
        > {
            Box::pin(async { Err(ExtensionCallError::new("unused")) })
        }

        fn list_models(
            &self,
            _provider_id: &str,
            _credential: Option<JsonValue>,
        ) -> futures_util::future::BoxFuture<'_, Result<Vec<Model>, ExtensionCallError>> {
            Box::pin(async {
                tokio::time::sleep(Duration::from_millis(1500)).await;
                LISTED.store(true, Ordering::Release);
                Ok(Vec::new())
            })
        }
    }
    struct SlowComponent;
    impl ExtensionInstance for SlowComponent {
        fn id(&self) -> &str {
            "slow-provider"
        }
        fn init(
            &self,
            _cx: CallContext,
        ) -> futures_util::future::BoxFuture<'_, Result<Registration, ExtensionCallError>> {
            Box::pin(async {
                Ok(Registration {
                    apis: vec!["slow-api".to_owned()],
                    providers: vec![json!({
                        "id": "slow",
                        "name": "Slow",
                        "api": "slow-api",
                        "models": "dynamic",
                    })],
                    tools: vec![tool("never")],
                    ..Registration::default()
                })
            })
        }
        fn on_event(
            &self,
            _kind: &str,
            _payload: JsonValue,
            _cx: CallContext,
        ) -> futures_util::future::BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>>
        {
            Box::pin(async { Ok(None) })
        }
        fn run_tool(
            &self,
            _tool: &str,
            _invocation: oino_agent::ToolInvocation,
            _updates: oino_agent::UpdateSink,
            _cancel: oino_extension_api::CancellationToken,
            _cx: CallContext,
        ) -> futures_util::future::BoxFuture<
            '_,
            Result<oino_agent::AgentToolResult, ExtensionCallError>,
        > {
            Box::pin(async { Err(ExtensionCallError::new("unused")) })
        }
        fn run_command(
            &self,
            _name: &str,
            _args: &str,
            _cancel: oino_extension_api::CancellationToken,
            _cx: CallContext,
        ) -> futures_util::future::BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>>
        {
            Box::pin(async { Ok(None) })
        }
        fn provider(&self) -> Option<&dyn oino_core::extensions::api::ProviderInstance> {
            Some(&SlowProvider)
        }
    }

    let provider = FauxProvider::new(vec![FauxResponse::Text("ok".into())]);
    let fixture = fixture(
        None,
        vec![
            Arc::new(FauxComponent(Arc::clone(&provider))),
            Arc::new(SlowComponent),
        ],
    )
    .await;

    let start = Instant::now();
    let services = fixture
        .hub
        .core_services()
        .await
        .expect("services come up without the provider probe");
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(800),
        "services waited for a dead provider: {elapsed:?}"
    );
    assert!(
        !LISTED.load(Ordering::Acquire),
        "the probe cannot have finished yet"
    );
    drop(services);

    // The refresh still runs — the model list fills in when the provider answers.
    let deadline = Instant::now() + Duration::from_secs(10);
    while !LISTED.load(Ordering::Acquire) {
        assert!(
            Instant::now() < deadline,
            "the background refresh never ran"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Sessions across more project cwds than the fd budget allows keep the descriptor
/// table bounded: the cap rejects the extra concurrent set instead of accumulating
/// per-services descriptors until `accept` fails.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn concurrent_projects_beyond_the_cap_stay_bounded() {
    let dir = tempfile::tempdir().unwrap();
    // One services set fits: open sessions across three project cwds in a loop.
    let fixture = faux_fixture(Some(1)).await;
    let mut live = Vec::new();
    for index in 0..3 {
        let cwd = dir.path().join(format!("p{index}"));
        std::fs::create_dir_all(&cwd).unwrap();
        match fixture.hub.create_session(Some(&cwd), None).await {
            Ok(session) => live.push(session.session_id()),
            Err(error) => assert!(
                error.to_string().contains("cache is full"),
                "unexpected failure: {error}"
            ),
        }
    }
    assert_eq!(live.len(), 1, "only the capped number of sets came up");

    let baseline = fd_count();
    for index in 0..6 {
        let cwd = dir.path().join(format!("q{index}"));
        std::fs::create_dir_all(&cwd).unwrap();
        // Cycle: open beyond the cap (rejected), close the live one, open the next.
        let _ = fixture.hub.create_session(Some(&cwd), None).await;
        if let Some(id) = live.pop() {
            fixture.hub.close(&id).await.unwrap();
        }
        if let Ok(session) = fixture.hub.create_session(Some(&cwd), None).await {
            live.push(session.session_id());
        }
    }
    let after = fd_count();
    assert!(
        after <= baseline + 8,
        "fd table grew while cycling projects beyond the cap: {baseline} -> {after}"
    );
    for id in live {
        fixture.hub.close(&id).await.unwrap();
    }
    // Sessions list stays truthful through it all.
    let root = dir.path().display().to_string();
    let cards = fixture.hub.list().await.unwrap();
    assert!(cards.iter().all(|card| card.cwd.starts_with(&root)));
}

/// The hub lists what the synthetic sessions wrote (sanity for the fixture itself).
#[tokio::test]
async fn synthetic_sessions_are_listed_and_openable() {
    let fixture = faux_fixture(None).await;
    let project = fixture.sessions.parent().unwrap().join("p");
    std::fs::create_dir_all(&project).unwrap();
    let id = synthetic_session(&fixture.sessions, &project, 4);
    let records = SessionManager::list_all(&fixture.sessions, None, None).unwrap();
    assert!(records.iter().any(|record| record.id == id));
    let opened = fixture.hub.open(&id).await;
    assert!(opened.is_ok(), "open: {opened:?}");
    let mut events = opened.unwrap().events();
    // The tap lets go once idle; just prove the stream is alive.
    let _ = tokio::time::timeout(Duration::from_secs(5), events.next()).await;
}
