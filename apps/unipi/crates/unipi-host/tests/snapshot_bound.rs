//! A session larger than the 1 MiB frame must still open (v2 QA defect: sessions
//! with multi-megabyte tool outputs / inline images failed `session_open` with
//! "oversize frame from host"). The snapshot bounds big text (flagged
//! `truncated: true` for the app's `get_entries` full-output refetch) and turns
//! big inline images into blob `mediaRef`s the stream's `media` command serves.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use base64::Engine as _;
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;
use serde_json::{json, Value};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use unipi_host::sessions::bound::{ENTRIES_BUDGET, ENTRY_TEXT_BUDGET, INLINE_IMAGE_LIMIT};
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::SessionService;
use unipi_proto::frame::{encode, FrameKind, MAX_FRAME};
use unipi_proto::{Control, Request};

/// Keep the whole reply comfortably below `MAX_FRAME` (the app's read_frame).
const FRAME_HEADROOM: usize = MAX_FRAME / 4;

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
        Self { read, write, task }
    }

    /// The next control frame (kind, bytes).
    async fn frame(&mut self) -> (u8, Vec<u8>) {
        let mut header = [0u8; 5];
        self.read.read_exact(&mut header).await.unwrap();
        let kind = header[0];
        let len = u32::from_be_bytes([header[1], header[2], header[3], header[4]]) as usize;
        assert!(len <= MAX_FRAME, "oversize frame from host: {len} bytes");
        let mut payload = vec![0u8; len];
        self.read.read_exact(&mut payload).await.unwrap();
        (kind, payload)
    }

    /// The snapshot `ok` reply.
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

    async fn close(mut self) {
        let _ = self.write.shutdown().await;
        let _ = tokio::time::timeout(Duration::from_secs(5), self.task).await;
    }
}

/// Writes a synthetic session: `plain` small entries, one with a huge tool-result
/// string and one with a big inline base64 image, totalling > 5 MiB on disk.
fn synthetic_huge_session(sessions: &std::path::Path, cwd: &std::path::Path) -> String {
    let id = "01aaaaaa-bbbb-7ccc-8ddd-000000005000".to_owned();
    let dir = oino_session::manager::cwd_session_dir(sessions, cwd);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(format!("2026-10-31T00-00-00-000Z_{id}.jsonl"));
    let header = json!({
        "type": "session", "version": 3, "id": id,
        "timestamp": "2026-10-31T00:00:00.000Z", "cwd": cwd.display().to_string(),
    });
    let image = base64::engine::general_purpose::STANDARD.encode(vec![7u8; 300 * 1024]);
    let lines = std::iter::once(header.to_string())
        .chain((0..40).map(|index| {
            json!({
                "id": format!("p{index:03}"), "parentId": if index == 0 { Value::Null }
                    else { json!(format!("p{:03}", index - 1)) },
                "timestamp": "2026-10-31T00:00:01.000Z", "type": "message",
                "message": {"role": "user", "content": format!("note {index}"), "timestamp": index},
            })
            .to_string()
        }))
        .chain(std::iter::once(
            json!({
                "id": "huge", "parentId": "p039",
                "timestamp": "2026-10-31T00:00:02.000Z", "type": "message",
                "message": {"role": "toolResult", "toolCallId": "c1", "toolName": "read",
                    "isError": false,
                    "content": [{"type": "text", "text": "X".repeat(5 * 1024 * 1024)}],
                    "timestamp": 99},
            })
            .to_string(),
        ))
        .chain(std::iter::once(
            json!({
                "id": "img", "parentId": "huge",
                "timestamp": "2026-10-31T00:00:03.000Z", "type": "message",
                "message": {"role": "user", "timestamp": 100, "content": [
                    {"type": "image", "data": image, "mimeType": "image/png"},
                ]},
            })
            .to_string(),
        ));
    std::fs::write(&file, lines.collect::<Vec<_>>().join("\n") + "\n").unwrap();
    assert!(
        std::fs::metadata(&file).unwrap().len() > 5 * 1024 * 1024,
        "the synthetic session is really huge"
    );
    id
}

/// The 5 MiB session opens, the snapshot frame is comfortably under `MAX_FRAME`,
/// the tool result is truncated (the app's full-output flag) and the image became
/// a blob `mediaRef` that the stream's `media` command serves back byte-exact.
#[tokio::test]
async fn a_huge_session_opens_with_a_bounded_snapshot() {
    let fixture = fixture().await;
    let id = synthetic_huge_session(&fixture.sessions, &fixture.project);

    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let reply = client.snapshot().await;
    assert_eq!(reply["type"], "ok", "the huge session opens: {reply}");

    // The whole snapshot reply frame fits with headroom.
    let entries = reply
        .pointer("/data/entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let payload: usize = entries
        .iter()
        .map(|entry| serde_json::to_vec(entry).map_or(0, |bytes| bytes.len()))
        .sum();
    assert!(
        payload <= ENTRIES_BUDGET + FRAME_HEADROOM,
        "bounded entries are {payload} bytes"
    );
    // perf-open: read from the file (no runtime yet) — every entry of the branch fits the page.
    assert!(
        entries.len() >= 42,
        "every entry is still listed: {}",
        entries.len()
    );
    assert_eq!(
        reply.pointer("/data/page/has_more"),
        Some(&Value::Bool(false))
    );

    // The huge tool result: cut and flagged for the app's `get_entries` refetch.
    let huge = entries
        .iter()
        .find(|entry| entry["id"] == "huge")
        .expect("the tool result entry");
    let text = huge["message"]["content"][0]["text"].as_str().unwrap();
    assert!(text.len() <= ENTRY_TEXT_BUDGET + 128, "text bounded");
    assert!(text.contains("truncated"), "the cut is explained");
    assert_eq!(huge["message"]["truncated"], Value::Bool(true));

    // The image: no inline data, a sha256 mediaRef, served byte-exact by `media`.
    let image = entries
        .iter()
        .find(|entry| entry["id"] == "img")
        .expect("the image entry");
    let block = &image["message"]["content"][0];
    assert!(block.get("data").is_none(), "the inline base64 is gone");
    let reference = block["mediaRef"].as_str().unwrap();
    assert_eq!(reference.len(), 64);

    let media = client
        .cmd(1, json!({"type": "media", "mediaRef": reference}))
        .await;
    assert_eq!(media["type"], "ok", "{media}");
    let data_url = media["data"]["dataUrl"].as_str().expect("a data url");
    let encoded = data_url.rsplit(',').next().unwrap();
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap();
    assert_eq!(decoded, vec![7u8; 300 * 1024], "the blob serves the bytes");
    assert_eq!(media["data"]["mime"], "application/octet-stream");

    // Core's whole-history `get_entries` (no `ids`/`before`) is 5 MiB here: one frame
    // cannot carry it, and the app's reader would reject it. fix-frames: the host never
    // writes an oversize frame — the reply becomes a retryable error with the same id
    // (the app's expand path uses the bounded `get_entries{ids}` instead).
    let full = client.cmd(2, json!({"type": "get_entries"})).await;
    assert_eq!(full["type"], "err", "{full}");
    assert_eq!(full["id"], 2);
    assert_eq!(full["error"]["retryable"], Value::Bool(true));
    client.close().await;

    // Reopening works too (the blob is deduped by content).
    let mut second = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.try_into().unwrap(),
            since_seq: None,
        },
    );
    let reply = second.snapshot().await;
    assert_eq!(reply["type"], "ok", "reopen: {reply}");
    second.close().await;
}

/// Small images stay inline — the reference path only kicks in past the limit.
#[tokio::test]
async fn small_images_stay_inline() {
    let fixture = fixture().await;
    let small = base64::engine::general_purpose::STANDARD.encode(vec![1u8; 1024]);
    assert!(small.len() < INLINE_IMAGE_LIMIT);
    let dir = oino_session::manager::cwd_session_dir(&fixture.sessions, &fixture.project);
    std::fs::create_dir_all(&dir).unwrap();
    let id = "01aaaaaa-bbbb-7ccc-8ddd-000000006000".to_owned();
    std::fs::write(
        dir.join(format!("2026-10-31T00-00-00-000Z_{id}.jsonl")),
        format!(
            "{}\n{}\n",
            json!({"type": "session", "version": 3, "id": id,
                "timestamp": "2026-10-31T00:00:00.000Z",
                "cwd": fixture.project.display().to_string()}),
            json!({"id": "i1", "parentId": null,
            "timestamp": "2026-10-31T00:00:01.000Z", "type": "message",
            "message": {"role": "user", "timestamp": 1, "content": [
                {"type": "image", "data": small, "mimeType": "image/png"},
            ]}}),
        ),
    )
    .unwrap();

    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let reply = client.snapshot().await;
    assert_eq!(reply["type"], "ok", "{reply}");
    let block = &reply["data"]["entries"][0]["message"]["content"][0];
    assert!(block["data"].is_string(), "small images ride inline");
    assert!(block.get("mediaRef").is_none());
    client.close().await;
}
