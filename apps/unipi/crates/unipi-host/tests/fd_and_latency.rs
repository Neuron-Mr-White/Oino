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

    /// The first frame, or `None` when the handler refused the request and ended the
    /// stream without one (its `Err` is then the task's result).
    async fn first_reply_or_end(&mut self) -> Option<Value> {
        let mut header = [0u8; 5];
        let read =
            tokio::time::timeout(Duration::from_secs(20), self.read.read_exact(&mut header)).await;
        let Ok(Ok(_)) = read else {
            return None;
        };
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        let mut payload = vec![0u8; len];
        self.read.read_exact(&mut payload).await.ok()?;
        serde_json::from_slice(&payload).ok()
    }

    /// The refusal the handler returned (the task's `Err`), when the stream ended
    /// without a frame.
    async fn refusal(mut self) -> Option<unipi_proto::AppErrorDto> {
        let _ = self.write.shutdown().await;
        match tokio::time::timeout(Duration::from_secs(5), self.task).await {
            Ok(Ok(Err(error))) => Some(error),
            _ => None,
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

/// The fd-bound asserts below check that the table comes back down — not that async
/// teardown is instant. Watchers, sockets and wasm caches close lazily (dispose is
/// async, and a loaded machine stretches it), so a single snapshot races the close:
/// poll until two samples 100 ms apart agree (3 s ceiling) before asserting.
#[cfg(target_os = "linux")]
async fn fd_count_settled() -> usize {
    let mut after = fd_count();
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let next = fd_count();
        if next == after {
            break;
        }
        after = next;
    }
    after
}

/// Serializes the tests that assert on the process-wide fd count (they run in one
/// process; concurrent fixtures would move the number under each other). Async-aware:
/// the guard is held across the whole test.
static FD_ASSERT_LOCK: std::sync::LazyLock<tokio::sync::Mutex<()>> =
    std::sync::LazyLock::new(|| tokio::sync::Mutex::new(()));

/// Creates a session with one faux exchange, waits for it to go idle and returns its id.
async fn session_with_a_message(hub: &Arc<SessionHub>) -> String {
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
    let _fd_lock = FD_ASSERT_LOCK.lock().await;
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

    let after = fd_count_settled().await;
    assert!(
        after <= baseline + 8,
        "fd table grew across {OPEN_CYCLES} open/close cycles: {baseline} -> {after}"
    );
}

/// The hub's `maxServices` override reaches the core hub, and the cache trims by
/// evicting idle sets — never by refusing an open of an idle session: sequential
/// projects all open, the least-recently-used idle set (its idle sessions going
/// back to records) rotating out.
#[tokio::test]
async fn services_cap_evicts_idle_sets_and_never_blocks_an_idle_open() {
    #[cfg(target_os = "linux")]
    let _fd_lock = FD_ASSERT_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let fixture = faux_fixture(Some(2)).await;

    // Sequential opens of idle sessions (no stream attached: evictable at once).
    #[cfg(target_os = "linux")]
    let baseline = fd_count();
    for index in 0..5 {
        let cwd = dir.path().join(format!("p{index}"));
        std::fs::create_dir_all(&cwd).unwrap();
        let session = fixture
            .hub
            .create_session(Some(&cwd), None)
            .await
            .unwrap_or_else(|error| panic!("project {index} must open: {error}"));
        drop(session);
    }
    // The cache rotated instead of growing: the descriptor table stayed flat
    // (the new sessions have written no files yet, so the list shows none of them).
    #[cfg(target_os = "linux")]
    {
        let after = fd_count_settled().await;
        assert!(
            after <= baseline + 8,
            "fd table grew while rotating idle sets: {baseline} -> {after}"
        );
    }
}

/// With every cached set busy — here two open session streams, which pin their
/// runtimes exactly like running turns — a third project still opens (the cap is
/// exceeded by one, with a warning), and only the open past that bound is refused.
#[tokio::test]
async fn a_busy_cache_still_opens_one_project_beyond_the_cap() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = faux_fixture(Some(2)).await;

    // Two open streams pin two sets (the feed tap keeps a runtime subscriber).
    let mut first = Client::open(
        &fixture.service,
        Request::SessionNew {
            cwd: Some(
                dir.path()
                    .join("a")
                    .display()
                    .to_string()
                    .try_into()
                    .unwrap(),
            ),
            name: None,
        },
    );
    let r1 = first.first_reply().await;
    assert_eq!(r1["type"], "ok", "first: {r1}");
    let mut second = Client::open(
        &fixture.service,
        Request::SessionNew {
            cwd: Some(
                dir.path()
                    .join("b")
                    .display()
                    .to_string()
                    .try_into()
                    .unwrap(),
            ),
            name: None,
        },
    );
    let r2 = second.first_reply().await;
    assert_eq!(r2["type"], "ok", "second: {r2}");

    // The third project opens: both cached sets are busy, so the hub exceeds the cap.
    let mut third = Client::open(
        &fixture.service,
        Request::SessionNew {
            cwd: Some(
                dir.path()
                    .join("c")
                    .display()
                    .to_string()
                    .try_into()
                    .unwrap(),
            ),
            name: None,
        },
    );
    let r3 = third.first_reply().await;
    assert_eq!(r3["type"], "ok", "third: {r3}");

    // A fourth open while every set is busy and the cache is already one over is the
    // one refusal left — the alternative would be unbounded growth.
    let mut fourth = Client::open(
        &fixture.service,
        Request::SessionNew {
            cwd: Some(
                dir.path()
                    .join("d")
                    .display()
                    .to_string()
                    .try_into()
                    .unwrap(),
            ),
            name: None,
        },
    );
    let mut reply = fourth.first_reply_or_end().await;
    if reply.is_none() {
        let refusal = fourth.refusal().await;
        reply = refusal.map(|error| json!({"type": "err", "error": {"message": error.message}}));
    }
    let reply = reply.expect("a refusal, not silence");
    assert_eq!(reply["type"], "err", "one-over is the limit: {reply}");
    assert!(
        reply["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("over budget"),
        "{reply}"
    );

    // Closing one stream frees its set's sessions: the next open evicts it and works.
    third.close().await;
    // The feed tap lets go of an unwatched idle session after ~3 s of idle polls.
    tokio::time::sleep(Duration::from_millis(4500)).await;
    let mut fifth = Client::open(
        &fixture.service,
        Request::SessionNew {
            cwd: Some(
                dir.path()
                    .join("e")
                    .display()
                    .to_string()
                    .try_into()
                    .unwrap(),
            ),
            name: None,
        },
    );
    assert_eq!(fifth.first_reply().await["type"], "ok");
    first.close().await;
    second.close().await;
    fifth.close().await;
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
    // perf-open: the snapshot is the branch's tail page; `page.total` counts the branch.
    assert!(
        (1..=1000).contains(&entries),
        "the snapshot carries the tail page: {entries}"
    );
    assert_eq!(
        reply.pointer("/data/page/total").and_then(Value::as_u64),
        Some(1000),
        "{}",
        reply["data"]["page"]
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
async fn cycling_projects_beyond_the_cap_stay_bounded() {
    #[cfg(target_os = "linux")]
    let _fd_lock = FD_ASSERT_LOCK.lock().await;
    let dir = tempfile::tempdir().unwrap();
    // One services set fits: cycling projects must rotate idle sets, never accumulate.
    let fixture = faux_fixture(Some(1)).await;

    let baseline = fd_count();
    let mut live: Vec<String> = Vec::new();
    for index in 0..8 {
        let cwd = dir.path().join(format!("q{index}"));
        std::fs::create_dir_all(&cwd).unwrap();
        // Open, then close the previous session: its set becomes idle-evictable, so
        // the next open retires it instead of growing the cache.
        if let Some(id) = live.pop() {
            fixture.hub.close(&id).await.unwrap();
        }
        let session = fixture
            .hub
            .create_session(Some(&cwd), None)
            .await
            .unwrap_or_else(|error| panic!("project {index} must open: {error}"));
        live.push(session.session_id());
    }
    let after = fd_count_settled().await;
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
