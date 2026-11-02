//! fix-frames P0: live session events must never leave the host as an oversize frame.
//!
//! The owner's chat ran `browser_navigate` and the app showed "oversize frame from host":
//! a live `tool_execution_end` / `message_end` carrying a browser screenshot (or a huge
//! tool output) was one frame over `MAX_FRAME`, the app's reader rejected it and the chat
//! stopped updating. Snapshots were bounded (`bound_entries`); live events were not.
//!
//! Here a 5 MB tool result, a 3 MB base64 screenshot and a burst of big deltas go through
//! a real session stream: every frame stays ≤ `MAX_FRAME`, the text arrives clipped and
//! flagged `truncated`, the screenshot as a blob `mediaRef`, and a resume whose replay
//! would not fit one frame reopens with a full snapshot instead.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use unipi_host::sessions::bound::{EVENT_BUDGET, INLINE_IMAGE_LIMIT};
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::SessionService;
use unipi_proto::{frame::MAX_FRAME, Control, Request, SessionEventDto, SessionSnapshot};

use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;

const READ_TIMEOUT: Duration = Duration::from_secs(30);

struct Fixture {
    _dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
    service: Arc<SessionService>,
    session_id: String,
}

async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(Vec::new());
    let loader = StaticLoader(vec![Arc::new(FauxComponent(Arc::clone(&provider)))
        as Arc<dyn oino_core::extensions::api::ExtensionInstance>]);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
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
    let session = hub.create_session(None, None).await.unwrap();
    let session_id = session.session_id();
    let service = SessionService::new(Arc::clone(&hub));
    Fixture {
        _dir: dir,
        hub,
        service,
        session_id,
    }
}

struct Client {
    read: tokio::io::ReadHalf<DuplexStream>,
    write: tokio::io::WriteHalf<DuplexStream>,
}

impl Client {
    fn open(service: &Arc<SessionService>, req: Request) -> Self {
        let (client, server) = tokio::io::duplex(1 << 20);
        let service = Arc::clone(service);
        tokio::spawn(async move {
            let (mut read, mut write) = tokio::io::split(server);
            unipi_host::sessions::stream::serve_request(&service, req, &mut read, &mut write).await
        });
        let (read, write) = tokio::io::split(client);
        Self { read, write }
    }

    /// The next control frame; panics on an oversize one (the app's `read_frame` rule).
    async fn json(&mut self) -> Value {
        let (kind, bytes) = tokio::time::timeout(READ_TIMEOUT, frame(&mut self.read))
            .await
            .expect("timed out waiting for the host")
            .expect("stream ended");
        assert_eq!(kind, 0);
        serde_json::from_slice(&bytes).unwrap()
    }

    async fn snapshot(&mut self) -> SessionSnapshot {
        let value = self.json().await;
        let control: Control = serde_json::from_value(value).unwrap();
        match control {
            Control::Ok { data } => serde_json::from_value(data.unwrap()).unwrap(),
            other => panic!("expected the snapshot, got {other:?}"),
        }
    }

    /// Events (one `Vec` per frame) until one matches `done`.
    async fn events_until(&mut self, done: impl Fn(&Value) -> bool) -> Vec<Vec<SessionEventDto>> {
        let mut frames = Vec::new();
        loop {
            let value = self.json().await;
            if value.get("type").and_then(Value::as_str) != Some("events") {
                continue;
            }
            let items: Vec<SessionEventDto> =
                serde_json::from_value(value["items"].clone()).unwrap();
            let stop = items.iter().any(|item| done(&item.event));
            frames.push(items);
            if stop {
                return frames;
            }
        }
    }

    async fn close(mut self) {
        let _ = self.write.shutdown().await;
    }
}

async fn frame<R: AsyncRead + Unpin>(read: &mut R) -> Option<(u8, Vec<u8>)> {
    let kind = read.read_u8().await.ok()?;
    let len = read.read_u32().await.ok()? as usize;
    assert!(len <= MAX_FRAME, "oversize frame from host: {len} bytes");
    let mut payload = vec![0u8; len];
    read.read_exact(&mut payload).await.ok()?;
    Some((kind, payload))
}

fn screenshot_b64(bytes: usize) -> String {
    let raw: Vec<u8> = (0..bytes).map(|i| (i * 131 % 251) as u8).collect();
    base64::engine::general_purpose::STANDARD.encode(raw)
}

fn is(kind: &'static str) -> impl Fn(&Value) -> bool {
    move |event| event.get("type").and_then(Value::as_str) == Some(kind)
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_five_megabyte_tool_result_and_a_three_megabyte_screenshot_stay_under_the_frame() {
    let fixture = fixture().await;
    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: fixture.session_id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let snapshot = client.snapshot().await;
    let feed = fixture.service.feed(&fixture.session_id);

    // 1. A bash call printing 5 MB: the live end event and its toolResult message.
    let huge = "x".repeat(5 * 1024 * 1024);
    feed.publish(json!({
        "type": "tool_execution_end", "toolCallId": "t1", "toolName": "bash", "isError": false,
        "result": {"content": [{"type": "text", "text": huge}], "details": {}}
    }))
    .await;
    let frames = client.events_until(is("tool_execution_end")).await;
    let end = frames
        .concat()
        .into_iter()
        .find(|e| e.kind() == Some("tool_execution_end"))
        .unwrap();
    assert_eq!(end.event["truncated"], Value::Bool(true));
    assert_eq!(end.event["result"]["truncated"], Value::Bool(true));
    assert_eq!(end.event["toolCallId"], "t1");
    let text = end.event["result"]["content"][0]["text"].as_str().unwrap();
    assert!(text.len() < EVENT_BUDGET && text.starts_with("xxxx") && text.contains("truncated"));

    // 2. browser_navigate's screenshot, 3 MB of base64, in the toolResult `message_end`
    //    and the `tool_execution_end` before it.
    let shot = screenshot_b64(3 * 1024 * 1024 / 4 * 3);
    assert!(shot.len() >= 3 * 1024 * 1024 - 8);
    let content = json!([
        {"type": "text", "text": "Navigated to https://example.com"},
        {"type": "image", "data": shot, "mimeType": "image/png"}
    ]);
    feed.publish(json!({
        "type": "tool_execution_end", "toolCallId": "b1", "toolName": "browser_navigate",
        "isError": false, "result": {"content": content.clone(), "details": {}}
    }))
    .await;
    feed.publish(json!({
        "type": "message_end",
        "message": {"role": "toolResult", "toolCallId": "b1", "toolName": "browser_navigate",
                    "content": content, "isError": false, "timestamp": 1}
    }))
    .await;
    let frames = client.events_until(is("message_end")).await;
    let message = frames
        .concat()
        .into_iter()
        .find(|e| e.kind() == Some("message_end"))
        .unwrap();
    let image = &message.event["message"]["content"][1];
    assert!(image.get("data").is_none(), "the screenshot left the frame");
    let reference = image["mediaRef"].as_str().unwrap();
    assert!(
        fixture.hub.blobs_dir().join(reference).is_file(),
        "it is in the blob store"
    );
    assert_eq!(image["mimeType"], "image/png");
    assert_eq!(
        message.event["message"]["content"][0]["text"],
        "Navigated to https://example.com"
    );
    assert_eq!(message.event["message"]["truncated"], Value::Bool(true));

    // 3. A burst of big, non-coalescing deltas (alternating blocks): one flush's batch is
    //    split over several frames, none oversize, nothing lost or reordered.
    let block = "d".repeat(INLINE_IMAGE_LIMIT * 12); // ~192 KiB, under the event budget
    let block_len = block.len();
    let publisher = {
        let feed = Arc::clone(&feed);
        tokio::spawn(async move {
            for i in 0..24u64 {
                feed.publish(json!({
                    "type": "message_update",
                    "message": {"role": "assistant", "timestamp": 2, "provider": "faux", "model": "faux-1", "api": "faux"},
                    "assistantMessageEvent": {"type": "text_delta", "contentIndex": i % 2, "delta": block.clone()}
                }))
                .await;
            }
            feed.publish(json!({"type": "agent_settled"})).await;
        })
    };
    let frames = client.events_until(is("agent_settled")).await;
    publisher.await.unwrap();
    let deltas: usize = frames
        .concat()
        .iter()
        .filter(|e| e.kind() == Some("message_update"))
        .map(|e| {
            e.event["assistantMessageEvent"]["delta"]
                .as_str()
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(deltas, block_len * 24, "every delta arrives exactly once");
    let seqs: Vec<u64> = frames.concat().iter().map(|e| e.seq).collect();
    assert!(seqs.windows(2).all(|p| p[0] < p[1]), "in order: {seqs:?}");

    // 4. A resume from before all of that: the replay is far over one frame, so the host
    //    answers with a (bounded) full snapshot instead of an oversize resume patch.
    client.close().await;
    let mut again = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: fixture.session_id.clone().try_into().unwrap(),
            since_seq: Some(snapshot.seq),
        },
    );
    let reopened = again.snapshot().await;
    assert!(
        reopened.reset,
        "a replay too big for one frame falls back to a snapshot"
    );
    again.close().await;

    // 5. A small gap still resumes.
    let seq = feed.seq();
    feed.publish(json!({"type": "agent_start"})).await;
    let mut resumed = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: fixture.session_id.clone().try_into().unwrap(),
            since_seq: Some(seq),
        },
    );
    let patch = resumed.snapshot().await;
    assert!(!patch.reset);
    assert_eq!(patch.replay.unwrap().len(), 1);
    resumed.close().await;
}
