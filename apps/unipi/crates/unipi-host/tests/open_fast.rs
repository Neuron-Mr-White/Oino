//! perf-open: a 100 MB session opens at its tail without waking first — the snapshot is
//! the branch's last page read through the offset index (`waking: true`), the runtime
//! wakes behind it (`{kind:"woken"}`), and older history pages in with
//! `get_entries{before: cursor}`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::io::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::open::PAGE_LIMIT;
use unipi_host::sessions::stream::SessionService;
use unipi_proto::frame::{encode, FrameKind, MAX_FRAME};
use unipi_proto::{Control, Request};

/// Debug-build budget for the first snapshot of a cold 100 MB session (a release host
/// answers in tens of ms; the old path parsed the whole file and took seconds).
const COLD_BUDGET: Duration = Duration::from_secs(3);
/// Debug-build budget for a page fetch or a reopen once indexed.
const WARM_BUDGET: Duration = Duration::from_millis(500);

struct Fixture {
    _dir: tempfile::TempDir,
    _hub: Arc<SessionHub>,
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
        agent_dir,
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
        _dir: dir,
        _hub: hub,
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

    /// The first session event of `kind` (already seen or still to come).
    async fn event_kind(&mut self, kind: &str) -> Value {
        loop {
            if let Some(at) = self.seen.iter().position(|event| event["kind"] == kind) {
                return self.seen.remove(at);
            }
            let (_, value) = self.frame_or_event().await;
            self.keep_events(&value);
        }
    }

    async fn close(mut self) {
        let _ = self.write.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

/// Writes a ~100 MB linear session: `turns` × (user, assistant, 48 KB tool result).
fn big_session(sessions: &std::path::Path, cwd: &std::path::Path, turns: usize) -> String {
    let id = "01aaaaaa-bbbb-7ccc-8ddd-000000100000".to_owned();
    let dir = oino_session::manager::cwd_session_dir(sessions, cwd);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("2026-11-02T00-00-00-000Z_{id}.jsonl"));
    let mut out = std::io::BufWriter::new(std::fs::File::create(&file).unwrap());
    let header = json!({
        "type": "session", "version": 3, "id": id,
        "timestamp": "2026-11-02T00:00:00.000Z", "cwd": cwd.display().to_string(),
    });
    writeln!(out, "{header}").unwrap();
    let blob = "x".repeat(48 * 1024);
    let mut parent = Value::Null;
    for n in 0..turns {
        let user = format!("u{n:05}");
        let assistant = format!("a{n:05}");
        let tool = format!("t{n:05}");
        writeln!(
            out,
            "{}",
            json!({"type": "message", "id": user, "parentId": parent,
            "timestamp": "2026-11-02T00:00:01.000Z",
            "message": {"role": "user", "content": format!("question {n}"), "timestamp": n}})
        )
        .unwrap();
        writeln!(out, "{}", json!({"type": "message", "id": assistant, "parentId": user,
            "timestamp": "2026-11-02T00:00:02.000Z",
            "message": {"role": "assistant", "provider": "faux", "model": "faux-1", "api": "faux",
                "content": [{"type": "toolCall", "id": format!("c{n}"), "name": "read", "arguments": {"path": "f"}}],
                "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "totalTokens": 0,
                    "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}},
                "stopReason": "toolUse", "timestamp": n}})).unwrap();
        writeln!(
            out,
            "{}",
            json!({"type": "message", "id": tool, "parentId": assistant,
            "timestamp": "2026-11-02T00:00:03.000Z",
            "message": {"role": "toolResult", "toolCallId": format!("c{n}"), "toolName": "read",
                "isError": false, "content": [{"type": "text", "text": blob}], "timestamp": n}})
        )
        .unwrap();
        parent = json!(tool);
    }
    out.flush().unwrap();
    drop(out);
    assert!(
        std::fs::metadata(&file).unwrap().len() > 100_000_000,
        "really 100 MB"
    );
    id
}

fn ids(entries: &Value) -> Vec<String> {
    entries
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_100_mb_session_opens_at_its_tail_and_pages_back() {
    let fixture = fixture().await;
    let turns = 2_100;
    let id = big_session(&fixture.sessions, &fixture.project, turns);

    // Cold: no runtime, no index yet.
    let started = Instant::now();
    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let (kind, bytes) = tokio::time::timeout(Duration::from_secs(60), client.frame())
        .await
        .expect("the snapshot");
    let cold = started.elapsed();
    eprintln!(
        "perf-open: cold 100 MB open answered in {cold:?} ({} bytes)",
        bytes.len()
    );
    assert_eq!(kind, 0);
    assert!(
        bytes.len() < MAX_FRAME,
        "the snapshot fits one frame: {}",
        bytes.len()
    );
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["type"], "ok", "{reply}");
    let data = &reply["data"];
    assert_eq!(
        data["waking"], true,
        "history first, the runtime wakes behind it"
    );
    assert_eq!(data["page"]["total"], json!(turns * 3));
    assert_eq!(data["page"]["has_more"], true);
    let tail = ids(&data["entries"]);
    assert!(
        !tail.is_empty() && tail.len() <= PAGE_LIMIT,
        "{}",
        tail.len()
    );
    assert_eq!(tail.last().unwrap(), &format!("t{:05}", turns - 1));
    assert!(
        tail[0].starts_with('u'),
        "the page starts at a turn: {}",
        tail[0]
    );
    assert_eq!(data["page"]["cursor"], json!(tail[0]));
    assert_eq!(data["leaf_id"], json!(format!("t{:05}", turns - 1)));
    assert_eq!(
        data["state"]["model"]["id"], "faux-1",
        "provisional model from the file"
    );
    assert!(cold < COLD_BUDGET, "cold open took {cold:?}");

    // "Load earlier": the page before the cursor, contiguous with the tail.
    let started = Instant::now();
    let older = client
        .cmd(
            1,
            json!({"type": "get_entries", "before": tail[0], "limit": PAGE_LIMIT}),
        )
        .await;
    let page_took = started.elapsed();
    eprintln!("perf-open: load-earlier page in {page_took:?}");
    assert_eq!(older["type"], "ok", "{older}");
    let older_ids = ids(&older["data"]["entries"]);
    assert!(!older_ids.is_empty());
    let first_of_tail: usize = tail[0][1..].parse().unwrap();
    assert_eq!(
        older_ids.last().unwrap(),
        &format!("t{:05}", first_of_tail - 1)
    );
    assert_eq!(older["data"]["hasMore"], true);
    assert_eq!(older["data"]["total"], json!(turns * 3));
    assert!(page_took < WARM_BUDGET, "page took {page_took:?}");

    // The full-output refetch by id answers just that entry, bounded for the frame.
    let full = client
        .cmd(2, json!({"type": "get_entries", "ids": ["t00010"]}))
        .await;
    assert_eq!(full["type"], "ok", "{full}");
    assert_eq!(ids(&full["data"]["entries"]), ["t00010"]);
    let text = full["data"]["entries"][0]["message"]["content"][0]["text"]
        .as_str()
        .unwrap();
    assert_eq!(text.len(), 48 * 1024, "small enough to come whole");

    // The runtime woke behind the history: `woken` carries the real state and commands.
    let woken = client.event_kind("woken").await;
    assert!(woken.get("state").is_some_and(Value::is_object), "{woken}");
    // Waking may append (the resume repair closes the interrupted tool turn): `leafId`
    // tells the app, which reconciles with `get_entries{since}` — just the new entries.
    let leaf = woken["leafId"].as_str().expect("a leaf").to_owned();
    let since = client
        .cmd(
            4,
            json!({"type": "get_entries", "since": format!("t{:05}", turns - 1)}),
        )
        .await;
    assert_eq!(since["type"], "ok", "{since}");
    let appended = ids(&since["data"]["entries"]);
    if leaf == format!("t{:05}", turns - 1) {
        assert!(appended.is_empty(), "{appended:?}");
    } else {
        assert_eq!(appended.last(), Some(&leaf), "{appended:?}");
        eprintln!(
            "perf-open: the wake appended {} entries: {}",
            appended.len(),
            since["data"]["entries"]
        );
    }
    // A runtime command works once woken (it would have waited for the wake otherwise).
    let state = client.cmd(3, json!({"type": "get_state"})).await;
    assert_eq!(state["type"], "ok", "{state}");
    client.close().await;

    // Reopen while live: the runtime path, paged the same way.
    let started = Instant::now();
    let mut again = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let (_, bytes) = again.frame().await;
    let warm = started.elapsed();
    eprintln!("perf-open: warm reopen in {warm:?} ({} bytes)", bytes.len());
    let reply: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(reply["type"], "ok");
    assert!(reply["data"].get("waking").is_none(), "live: no wake");
    // The branch plus whatever the wake appended.
    assert!(reply["data"]["page"]["total"].as_u64().unwrap() >= (turns * 3) as u64);
    assert!(bytes.len() < MAX_FRAME);
    assert!(warm < WARM_BUDGET, "warm reopen took {warm:?}");
    again.close().await;
}
