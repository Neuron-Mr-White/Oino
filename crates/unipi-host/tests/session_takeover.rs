//! WP-H5 acceptance (spec-host §8): session takeover.
//!
//! (a) A TUI/RPC-style holder **process** (spawned like `oino --mode rpc` on the same
//! session file) loses the session on the host's `session_open`: core's D-TAKEOVER closes
//! it there, the host reloads the session here and its stream announces
//! `{kind:"taken_over", from:"cli"}` as the first event after the snapshot (§8.1–8.2).
//! (b) A second host opening this host's session takes the lease over: the first host's
//! stream gets `{kind:"taken_over"}` with the new host's label, its card turns
//! `taken_over` and its feed stops resuming; opening it again from the first host takes it
//! back (§8.4).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

use futures_util::StreamExt;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::task::JoinHandle;

use unipi_host::sessions::card::HubState;
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::SessionService;
use unipi_proto::frame::MAX_FRAME;
use unipi_proto::{Request, SessionEventDto, SessionSnapshot};

use oino_client::protocol::{PiCommand, RpcCommand};
use oino_core::events::{ClientEvent, OinoEvent};
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;

const READ_TIMEOUT: Duration = Duration::from_secs(30);
/// Core's D-TAKEOVER deadline (spec-core §D-TAKEOVER step 2): an open replies only after
/// the session is exclusively ours, or the holder-refusal error.
const TAKEOVER_DEADLINE: Duration = Duration::from_secs(10);
/// The holder child's env: shared dir, session file, and the markers it leaves behind.
const HOLDER_DIR: &str = "OINO_H5_HOLDER_DIR";
const HOLDER_FILE: &str = "OINO_H5_HOLDER_FILE";
const HOLDER_READY: &str = "holder.ready";
const HOLDER_TAKEN: &str = "holder.taken-over";

/// One host over the shared dirs: the session hub and its streams (§4.2, §7.3).
fn open_host(dir: &Path) -> (Arc<SessionHub>, Arc<SessionService>) {
    let agent = dir.join("agent");
    let sessions = dir.join("sessions");
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(vec![FauxResponse::Text("host reply".into())]);
    let loader = StaticLoader(vec![
        Arc::new(FauxComponent(Arc::clone(&provider))) as Arc<dyn ExtensionInstance>
    ]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.to_path_buf(),
        agent_dir: agent,
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
        tools: oino_core::runtime::ToolSelection::Default,
        exclude_tools: Vec::new(),
    })
    .unwrap();
    let service = SessionService::new(Arc::clone(&hub));
    (hub, service)
}

/// A session with one settled faux turn — a file on disk the host or the terminal can open.
async fn settled_session(hub: &Arc<SessionHub>) -> String {
    let session = hub.create_session(None, None).await.unwrap();
    session
        .send(RpcCommand::Pi(PiCommand::SetModel {
            provider: "faux".into(),
            model_id: "faux-1".into(),
        }))
        .await
        .unwrap();
    session
        .send(RpcCommand::Pi(PiCommand::Prompt {
            message: "hello".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    let id = session.session_id();
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        if let Ok(Some(card)) = hub.card(&id).await {
            if !card.state.is_working() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "session never went idle");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    id
}

/// One client end of a session stream served by a host (as `Handler::handle` does).
struct Client {
    read: tokio::io::ReadHalf<DuplexStream>,
    write: tokio::io::WriteHalf<DuplexStream>,
    task: JoinHandle<Result<(), unipi_proto::AppErrorDto>>,
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

    async fn json(&mut self) -> Value {
        let (kind, bytes) = tokio::time::timeout(READ_TIMEOUT, read_frame(&mut self.read))
            .await
            .expect("timed out waiting for the host")
            .expect("stream ended");
        assert_eq!(kind, 0, "expected a control frame");
        serde_json::from_slice(&bytes).expect("control json")
    }

    /// The stream's first frame, the snapshot (§7.3).
    async fn snapshot(&mut self) -> SessionSnapshot {
        let value = self.json().await;
        match value.get("type").and_then(Value::as_str) {
            Some("ok") => serde_json::from_value(value["data"].clone()).expect("snapshot json"),
            other => panic!("expected the snapshot, got {other:?}: {value}"),
        }
    }

    /// Reads `events` batches until one carries a host-only event of `kind` (§7.4); returns
    /// everything received, in order.
    async fn events_until(&mut self, kind: &str) -> Vec<SessionEventDto> {
        let mut all = Vec::new();
        loop {
            let batch = self.events().await;
            let found = batch.iter().any(|event| {
                event.event.get("kind").and_then(Value::as_str) == Some(kind)
            });
            all.extend(batch);
            if found {
                return all;
            }
        }
    }

    /// The next `events` batch (correlated replies and the snapshot are skipped).
    async fn events(&mut self) -> Vec<SessionEventDto> {
        loop {
            let value = self.json().await;
            match value.get("type").and_then(Value::as_str) {
                Some("events") => {
                    return serde_json::from_value(value["items"].clone()).expect("events json");
                }
                Some("ok" | "err") => continue,
                other => panic!("unexpected control on a session stream: {other:?}"),
            }
        }
    }

    async fn close(mut self) {
        let _ = self.write.shutdown().await;
        let _ = tokio::time::timeout(READ_TIMEOUT, self.task).await;
    }
}

async fn read_frame<R: AsyncRead + Unpin>(read: &mut R) -> Option<(u8, Vec<u8>)> {
    let kind = read.read_u8().await.ok()?;
    let len = read.read_u32().await.ok()? as usize;
    assert!(len <= MAX_FRAME, "oversize frame");
    let mut payload = vec![0u8; len];
    read.read_exact(&mut payload).await.ok()?;
    Some((kind, payload))
}

/// The first host-only event of `kind` (§7.4: host events are tagged `kind`, core's `type`).
fn host_event(events: &[SessionEventDto], kind: &str) -> Option<Value> {
    events
        .iter()
        .find(|event| event.event.get("kind").and_then(Value::as_str) == Some(kind))
        .map(|event| event.event.clone())
}

fn from_of(event: &Value) -> String {
    event
        .get("from")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn target(id: &str) -> unipi_proto::SessionTarget {
    id.to_owned().try_into().unwrap()
}

/// (b) Two hosts, one session: the second open takes the lease over and the first host's
/// open stream sees `taken_over` (and stops resuming); opening it again takes it back.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn second_host_marks_the_first_stream_taken_over() {
    let dir = tempfile::tempdir().unwrap();
    let (hub_a, service_a) = open_host(dir.path());
    let id = settled_session(&hub_a).await;

    // Host A's user has the session open in a stream.
    let mut stream_a = Client::open(
        &service_a,
        Request::SessionOpen {
            session: target(&id),
            since_seq: None,
        },
    );
    let snapshot_a = stream_a.snapshot().await;
    assert!(snapshot_a.reset, "a fresh stream starts with a snapshot");
    assert_eq!(snapshot_a.session_id, id);

    // Host B opens the same session: core's D-TAKEOVER closes A's runtime and reloads the
    // session in B.
    let (hub_b, service_b) = open_host(dir.path());
    let started = Instant::now();
    let mut stream_b = Client::open(
        &service_b,
        Request::SessionOpen {
            session: target(&id),
            since_seq: None,
        },
    );
    let snapshot_b = stream_b.snapshot().await;
    assert!(
        snapshot_b.reset,
        "a takeover is a new runtime, never a resume"
    );
    assert_eq!(snapshot_b.session_id, id);

    // §8.2: the winner's first session event after the snapshot names the loser.
    let events_b = stream_b.events().await;
    assert_eq!(
        events_b[0].event.get("kind").and_then(Value::as_str),
        Some("taken_over"),
        "the takeover notice comes first: {events_b:?}"
    );
    assert_eq!(events_b[0].seq, snapshot_b.seq + 1);
    let from_b = from_of(&events_b[0].event);
    assert!(
        from_b.starts_with("host:"),
        "the previous holder was a host: {from_b}"
    );
    assert!(
        started.elapsed() < TAKEOVER_DEADLINE,
        "the open replies once the session is ours: {:?}",
        started.elapsed()
    );

    // §8.4: the loser's stream is told, and its card names who took it.
    let events_a = stream_a.events_until("taken_over").await;
    let taken_a = host_event(&events_a, "taken_over").expect("A's stream sees the takeover");
    assert_eq!(from_of(&taken_a), from_b, "both sides name the same host");
    let card_a = hub_a.card(&id).await.unwrap().expect("the record stays");
    match &card_a.state {
        HubState::TakenOver { from } => assert_eq!(from, &from_b),
        other => panic!("A's card must be taken_over, got {other:?}"),
    }
    assert!(
        !service_a.feed(&id).can_resume(1),
        "the taken-over feed must not resume: the other host wrote in between"
    );

    // Taking it back reopens the record (a reset, `since_seq` ignored) and clears the mark.
    let mut stream_a2 = Client::open(
        &service_a,
        Request::SessionOpen {
            session: target(&id),
            since_seq: Some(1),
        },
    );
    let snapshot_a2 = stream_a2.snapshot().await;
    assert!(snapshot_a2.reset, "taking back a taken-over session resets");
    let events_a2 = stream_a2.events().await;
    let taken_a2 = host_event(&events_a2, "taken_over").expect("taking back is a takeover too");
    assert!(
        from_of(&taken_a2).starts_with("host:"),
        "it came back from the other host: {taken_a2}"
    );
    let card_a2 = hub_a.card(&id).await.unwrap().expect("the record stays");
    assert!(
        !matches!(card_a2.state, HubState::TakenOver { .. }),
        "taking it back clears the mark: {:?}",
        card_a2.state
    );

    stream_a2.close().await;
    stream_a.close().await;
    stream_b.close().await;
    hub_a.close(&id).await.unwrap();
    hub_b.close(&id).await.unwrap();
}

/// The holder half of [`tui_style_holder_loses_the_session`]: opens the session from another
/// process exactly as `oino --mode rpc --session <file>` would (its own runtime and lease),
/// then exits once the host took the session over.
#[test]
fn tui_style_holder() {
    let Some(dir) = std::env::var_os(HOLDER_DIR) else {
        return; // normal test run: nothing to do
    };
    let dir = PathBuf::from(dir);
    let file = PathBuf::from(std::env::var_os(HOLDER_FILE).unwrap());
    let ready = dir.join(HOLDER_READY);
    let taken = dir.join(HOLDER_TAKEN);
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    runtime.block_on(async move {
        let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
        models.set_runtime_api_key("faux", "test-key");
        let services = Services::create(ServicesOptions {
            cwd: dir.join("project"),
            agent_dir: dir.join("agent"),
            docs_root: dir.join("docs"),
            mode: ExtensionMode::Rpc,
            has_ui: false,
            flags: ResourceFlags {
                no_context_files: true,
                ..ResourceFlags::default()
            },
            extension_flag_values: Default::default(),
            loader: Arc::new(StaticLoader(Vec::new())),
            approve: Some(false),
            models: Some(models),
            settings: None,
        })
        .await
        .unwrap();
        let session = AgentSessionRuntime::create(
            Arc::clone(&services),
            SessionOptions {
                source: SessionSource::Open(file),
                session_dir: Some(dir.join("sessions")),
                ..SessionOptions::default()
            },
        )
        .await
        .unwrap();
        let mut events = session.subscribe();
        std::fs::write(&ready, "ready").unwrap();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            match tokio::time::timeout_at(deadline, events.next()).await {
                Ok(Some(ClientEvent::Oino(OinoEvent::SessionTakenOver { by }))) => {
                    std::fs::write(&taken, format!("{}|{}", by.client, by.pid)).unwrap();
                    break;
                }
                Ok(Some(_)) => {}
                Ok(None) | Err(_) => break,
            }
        }
        session.dispose().await;
    });
}

/// (a) A TUI-style holder process loses the session when the host opens it.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tui_style_holder_loses_the_session() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("project")).unwrap();
    let (hub, service) = open_host(dir.path());
    let id = settled_session(&hub).await;
    let file = hub.card(&id).await.unwrap().expect("the file exists").file;
    // The user leaves the host's session (a record on disk) and opens it in the terminal.
    hub.close(&id).await.unwrap();

    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "tui_style_holder"])
        .env(HOLDER_DIR, dir.path())
        .env(HOLDER_FILE, &file)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let ready = dir.path().join(HOLDER_READY);
    let deadline = Instant::now() + READ_TIMEOUT;
    while !ready.exists() {
        assert!(
            Instant::now() < deadline,
            "the terminal holder never opened the session"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let metadata = std::fs::read_to_string(oino_session::lock_metadata_path(&file)).unwrap();
    assert!(
        metadata.contains("\"oino rpc\""),
        "the terminal holds the lease: {metadata}"
    );

    // The host opens the session: it asks the holder to close and reloads here (§8.1).
    let started = Instant::now();
    let mut stream = Client::open(
        &service,
        Request::SessionOpen {
            session: target(&id),
            since_seq: None,
        },
    );
    let snapshot = stream.snapshot().await;
    assert!(snapshot.reset, "the reload is a fresh snapshot");
    assert_eq!(snapshot.session_id, id);
    assert!(
        started.elapsed() < TAKEOVER_DEADLINE,
        "session_open replies only once the session is ours: {:?}",
        started.elapsed()
    );
    // §8.2: the winner announces where the session came from (`oino rpc` is a CLI runner).
    let events = stream.events().await;
    assert_eq!(
        events[0].event.get("kind").and_then(Value::as_str),
        Some("taken_over"),
        "the takeover notice comes first: {events:?}"
    );
    assert_eq!(events[0].seq, snapshot.seq + 1);
    assert_eq!(from_of(&events[0].event), "cli");

    // The holder lost it: it saw `session_taken_over` and exited (spec-core D-TAKEOVER).
    let status = wait_child(&mut child, Duration::from_secs(15));
    assert!(status.success(), "the holder exits cleanly: {status:?}");
    let taken = std::fs::read_to_string(dir.path().join(HOLDER_TAKEN)).unwrap();
    assert_eq!(
        taken,
        format!("oino desktop|{}", std::process::id()),
        "the holder was told who took the session over"
    );
    // …and the host holds the lease now.
    let metadata = std::fs::read_to_string(oino_session::lock_metadata_path(&file)).unwrap();
    assert!(
        metadata.contains(&format!("\"pid\":{}", std::process::id())),
        "the host is the single writer: {metadata}"
    );
    // The session is live here again (the reload continued, not a dead record).
    let card = hub.card(&id).await.unwrap().expect("the record stays");
    assert!(
        !matches!(card.state, HubState::TakenOver { .. }),
        "the winning host is not marked taken over: {:?}",
        card.state
    );

    stream.close().await;
    hub.close(&id).await.unwrap();
}

/// Waits for `child`, killing it after `limit` (a hung helper must fail the test, not hang it).
fn wait_child(child: &mut Child, limit: Duration) -> std::process::ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("the holder did not exit within {limit:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}
