//! owner-fix-6 P1-2: opening a chat whose cron run is still in flight killed the run — the
//! background wake took the session over from the `oino -p` subprocess (D-TAKEOVER), the
//! run ended `failed` ("Request was aborted") and the chat read "Done in 0s". An open of a
//! running cron run's chat now joins it: the snapshot comes from the file (`waking`, the
//! provisional state streaming), the host follows the file while the run holds the lease
//! (`session_tree` with the new leaf → the app reloads its tail page), and the runtime
//! wakes only once the run has ended and released the session.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use unipi_cron::model::{Run, RunStatus};
use unipi_cron::{store, CronPaths};
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::SessionService;
use unipi_proto::frame::{encode, FrameKind};
use unipi_proto::{Control, Request};

struct Fixture {
    _dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
    agent_dir: PathBuf,
    service: Arc<SessionService>,
    sessions: std::path::PathBuf,
    project: std::path::PathBuf,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    let project = dir.path().join("project");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::create_dir_all(&project).unwrap();
    let provider = FauxProvider::new(Vec::new());
    let loader = StaticLoader(vec![Arc::new(FauxComponent(Arc::clone(&provider)))]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir: agent_dir.clone(),
        docs_root: None,
        session_dir: Some(sessions.clone()),
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
        agent_dir: dir.path().join("agent"),
        _dir: dir,
        hub,
        service,
        sessions,
        project,
    }
}

/// One client end of a session stream served by the real handler.
struct Client {
    /// Session events seen while waiting for replies.
    seen: Vec<Value>,
    read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    write: tokio::io::WriteHalf<tokio::io::DuplexStream>,
    task: tokio::task::JoinHandle<Result<(), unipi_proto::AppErrorDto>>,
}

impl Client {
    fn open(service: &Arc<SessionService>, req: Request) -> Self {
        let (client, server) = tokio::io::duplex(4 << 20);
        let service = Arc::clone(service);
        let task = tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(server);
            unipi_host::sessions::stream::serve_request(&service, req, &mut read, &mut write).await
        });
        let (read, write) = tokio::io::split(client);
        Self {
            seen: Vec::new(),
            read,
            write,
            task,
        }
    }

    /// The next control frame (kind, bytes).
    async fn frame(&mut self) -> (u8, Vec<u8>) {
        let mut header = [0u8; 5];
        self.read.read_exact(&mut header).await.unwrap();
        let kind = header[0];
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        let mut payload = vec![0u8; len];
        self.read.read_exact(&mut payload).await.unwrap();
        (kind, payload)
    }

    /// The snapshot `ok` reply.
    #[allow(dead_code)]
    async fn snapshot(&mut self) -> Value {
        let (kind, bytes) = tokio::time::timeout(Duration::from_secs(20), self.frame())
            .await
            .expect("timed out waiting for the host");
        assert_eq!(kind, 0, "expected a control frame");
        serde_json::from_slice(&bytes).unwrap()
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
            let (_, value) = self.frame_or_event().await;
            self.keep_events(&value);
            if matches!(
                value.get("type").and_then(Value::as_str),
                Some("ok") | Some("err")
            ) && value.get("id").and_then(Value::as_u64) == Some(id)
            {
                return value;
            }
        }
    }

    async fn frame_or_event(&mut self) -> (u8, Value) {
        let (kind, bytes) = tokio::time::timeout(Duration::from_secs(20), self.frame())
            .await
            .expect("timed out waiting for the reply");
        let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
        (kind, value)
    }

    fn keep_events(&mut self, value: &Value) {
        if let Some(items) = value.get("items").and_then(Value::as_array) {
            self.seen
                .extend(items.iter().filter_map(|item| item.get("event").cloned()));
        }
    }

    async fn close(mut self) {
        let _ = self.write.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

const ID: &str = "01bbbbbb-cccc-7ddd-8eee-000000000001";

fn line(value: &Value) -> String {
    format!("{value}\n")
}

fn message(id: &str, parent: Option<&str>, role: &str, text: &str) -> Value {
    let body = if role == "user" {
        json!({"role": "user", "content": text, "timestamp": 1})
    } else {
        json!({"role": "assistant", "provider": "faux", "model": "faux-1", "api": "faux",
            "content": [{"type": "text", "text": text}],
            "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
            "stopReason": "stop", "timestamp": 2})
    };
    json!({"type": "message", "id": id, "parentId": parent,
        "timestamp": "2026-11-05T00:00:01.000Z", "message": body})
}

/// The cron subprocess's session file so far: the prompt and a first reply.
fn run_session(sessions: &Path, cwd: &Path) -> PathBuf {
    let dir = oino_session::manager::cwd_session_dir(sessions, cwd);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("2026-11-05T00-00-00-000Z_{ID}.jsonl"));
    let mut out = std::fs::File::create(&file).unwrap();
    let header = json!({"type": "session", "version": 3, "id": ID,
        "timestamp": "2026-11-05T00:00:00.000Z", "cwd": cwd.display().to_string()});
    out.write_all(line(&header).as_bytes()).unwrap();
    out.write_all(line(&message("u1", None, "user", "slow: digest")).as_bytes())
        .unwrap();
    out.write_all(line(&message("a1", Some("u1"), "assistant", "working on it")).as_bytes())
        .unwrap();
    file
}

fn run_record(file: &Path, status: RunStatus) -> Run {
    Run {
        id: ID.to_owned(),
        job_id: "digest".to_owned(),
        status,
        fired_at: 1,
        started_at: Some(1),
        ended_at: None,
        exit_code: None,
        summary: None,
        reason: None,
        session_path: Some(file.display().to_string()),
        cost_usd: None,
        missed: false,
        pid: None,
        notify_errors: Vec::new(),
        wake: None,
    }
}

impl Client {
    /// The first session event matching `pred` (already seen or still to come), within
    /// `within`; `None` on timeout.
    async fn event_where(
        &mut self,
        within: Duration,
        pred: impl Fn(&Value) -> bool,
    ) -> Option<Value> {
        let deadline = Instant::now() + within;
        loop {
            if let Some(at) = self.seen.iter().position(|event| pred(event)) {
                return Some(self.seen.remove(at));
            }
            let left = deadline.checked_duration_since(Instant::now())?;
            let Ok((_, bytes)) = tokio::time::timeout(left, self.frame()).await else {
                return None;
            };
            let value: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
            self.keep_events(&value);
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn opening_a_running_cron_chat_joins_the_run_instead_of_taking_it_over() {
    let fixture = fixture().await;
    let file = run_session(&fixture.sessions, &fixture.project);
    let cron = CronPaths::new(fixture.agent_dir.clone());
    store::save_run(&cron, &run_record(&file, RunStatus::Running)).unwrap();
    // The `oino -p` subprocess holds the session's single-writer lease.
    let holder = oino_session::LeaseInfo::for_process("oino json", &fixture.agent_dir);
    let lease = oino_session::SessionLease::acquire(&file, &holder).unwrap();

    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: ID.to_owned().try_into().unwrap(),
            since_seq: None,
        },
    );
    let reply = client.snapshot().await;
    assert_eq!(reply["type"], "ok", "{reply}");
    let data = &reply["data"];
    assert_eq!(data["waking"], true, "served from the file");
    assert_eq!(
        data["state"]["isStreaming"], true,
        "the run is in flight: {data}"
    );

    // The run keeps its session: no takeover request, no wake while it runs.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    assert!(
        !oino_session::takeover_path(&file).exists(),
        "the open asked the running cron subprocess to give the session up"
    );
    assert!(
        client
            .event_where(Duration::from_millis(100), |e| e["kind"] == "woken"
                || e["kind"] == "wake_failed")
            .await
            .is_none(),
        "no wake while the run holds the session"
    );

    // The run appends a reply: the open follows it (the app reloads its tail page).
    std::fs::OpenOptions::new()
        .append(true)
        .open(&file)
        .unwrap()
        .write_all(line(&message("a2", Some("a1"), "assistant", "half way")).as_bytes())
        .unwrap();
    let moved = client
        .event_where(Duration::from_secs(5), |e| e["type"] == "session_tree")
        .await
        .expect("no session_tree after the run appended");
    assert_eq!(moved["leafId"], "a2", "{moved}");
    let page = client
        .cmd(7, json!({"type": "get_entries", "limit": 10}))
        .await;
    assert_eq!(page["type"], "ok", "{page}");
    assert_eq!(page["data"]["leafId"], "a2", "{page}");

    // The run ends and releases the session: the runtime wakes behind the history.
    let mut done = run_record(&file, RunStatus::Ok);
    done.ended_at = Some(2);
    store::save_run(&cron, &done).unwrap();
    fixture.hub.announce_cron_run(ID).await;
    lease.release();
    let woken = client
        .event_where(Duration::from_secs(20), |e| {
            e["kind"] == "woken" || e["kind"] == "wake_failed"
        })
        .await
        .expect("no wake after the run ended");
    assert_eq!(woken["kind"], "woken", "{woken}");
    client.close().await;
}
