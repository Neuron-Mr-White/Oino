//! WP-H4 acceptance (spec-host §7.3–7.5): session streams over the hub —
//! 5000 streamed deltas coalesce into a handful of `events` frames, a reconnect with
//! `since_seq` resumes from the ring, a disposed runtime resets, two subscribers of one
//! session see the same numbered events, and `events_open` pushes retained notices, cards
//! and host-scope ui registry events.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::task::JoinHandle;
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::SessionService;
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    Control, HubEvent, Request, SessionCard, SessionEventDto, SessionSnapshot, PROTO_VERSION,
};

use oino_client::protocol::{AgentCommand, RpcCommand, RpcSessionState};
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_models::ModelRegistry;

const READ_TIMEOUT: Duration = Duration::from_secs(20);

struct Fixture {
    _dir: tempfile::TempDir,
    hub: Arc<SessionHub>,
    service: Arc<SessionService>,
    session_id: String,
}

/// A hub whose sessions are answered by a faux provider, plus the session service over it.
async fn fixture() -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first reply".into()),
        FauxResponse::Text("second reply".into()),
    ]);
    let loader = StaticLoader(vec![Arc::new(FauxComponent(Arc::clone(&provider)))
        as Arc<dyn oino_core::extensions::api::ExtensionInstance>]);
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
    wait_idle(&hub, &id).await;
    let service = SessionService::new(Arc::clone(&hub));
    Fixture {
        _dir: dir,
        hub,
        service,
        session_id: id,
    }
}

async fn wait_idle(hub: &Arc<SessionHub>, session_id: &str) {
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        if let Ok(Some(card)) = hub.card(session_id).await {
            if !card.state.is_working() {
                break;
            }
        }
        assert!(Instant::now() < deadline, "session never went idle");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// One client end of a session stream (or `events_open`) served by the host.
struct Client {
    read: tokio::io::ReadHalf<DuplexStream>,
    write: tokio::io::WriteHalf<DuplexStream>,
    task: JoinHandle<Result<(), unipi_proto::AppErrorDto>>,
}

impl Client {
    /// Serves `req` on a fresh duplex pair, as `Handler::handle` does after the hello.
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

    async fn frame(&mut self) -> Option<(u8, Vec<u8>)> {
        frame(&mut self.read).await
    }

    /// The next control frame as raw JSON (correlated replies carry an `id` that the generic
    /// `Control` type does not model; a session stream's own envelope does).
    async fn json(&mut self) -> Value {
        let (kind, bytes) = tokio::time::timeout(READ_TIMEOUT, self.frame())
            .await
            .expect("timed out waiting for the host")
            .expect("stream ended");
        assert_eq!(kind, 0, "expected a control frame");
        serde_json::from_slice(&bytes).expect("control json")
    }

    async fn control(&mut self) -> Control {
        let (kind, bytes) = tokio::time::timeout(READ_TIMEOUT, self.frame())
            .await
            .expect("timed out waiting for the host")
            .expect("stream ended");
        assert_eq!(kind, 0, "expected a control frame");
        serde_json::from_slice(&bytes).expect("control json")
    }

    /// The `ok` data of the stream's first frame (the snapshot).
    async fn snapshot(&mut self) -> SessionSnapshot {
        match self.control().await {
            Control::Ok { data } => {
                serde_json::from_value(data.expect("snapshot data")).expect("snapshot json")
            }
            other => panic!("expected the snapshot, got {other:?}"),
        }
    }

    async fn send(&mut self, control: &Control) {
        let bytes = serde_json::to_vec(control).unwrap();
        self.write
            .write_all(&encode(FrameKind::Control, &bytes))
            .await
            .unwrap();
    }

    async fn close(mut self) {
        let _ = self.write.shutdown().await;
        let _ = tokio::time::timeout(READ_TIMEOUT, self.task).await;
    }
}

async fn frame<R: AsyncRead + Unpin>(read: &mut R) -> Option<(u8, Vec<u8>)> {
    let kind = read.read_u8().await.ok()?;
    let len = read.read_u32().await.ok()? as usize;
    assert!(len <= MAX_FRAME, "oversize frame");
    let mut payload = vec![0u8; len];
    read.read_exact(&mut payload).await.ok()?;
    Some((kind, payload))
}

/// Reads session events until `done` says stop; returns the frames (`items` per `events`
/// frame) and everything received.
async fn read_events_until(
    client: &mut Client,
    mut done: impl FnMut(&[SessionEventDto], &Value) -> bool,
) -> (Vec<Vec<SessionEventDto>>, Vec<SessionEventDto>, Vec<Value>) {
    let mut frames: Vec<Vec<SessionEventDto>> = Vec::new();
    let mut all: Vec<SessionEventDto> = Vec::new();
    let mut replies: Vec<Value> = Vec::new();
    loop {
        let value = client.json().await;
        match value.get("type").and_then(Value::as_str) {
            Some("events") => {
                let items: Vec<SessionEventDto> =
                    serde_json::from_value(value["items"].clone()).unwrap();
                all.extend(items.iter().cloned());
                let stop = items.iter().any(|item| done(&items, &item.event));
                frames.push(items);
                if stop {
                    return (frames, all, replies);
                }
            }
            Some("ok" | "err") => {
                // The session stream's correlated reply envelope (§7.3).
                let _: unipi_proto::SessionReply = serde_json::from_value(value.clone()).unwrap();
                replies.push(value);
            }
            other => panic!("unexpected control on a session stream: {other:?}"),
        }
    }
}

/// A run's end on a session stream (§7.4: `agent_settled` closes the activity).
fn settled(event: &Value) -> bool {
    event.get("type").and_then(Value::as_str) == Some("agent_settled")
}

fn text_delta(timestamp: u64, partial: &str, delta: &str) -> Value {
    let message = json!({
        "content": [{"type": "text", "text": partial}],
        "api": "faux", "provider": "faux", "model": "faux-1",
        "usage": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "stopReason": "pending", "timestamp": timestamp
    });
    json!({
        "type": "message_update",
        "message": message,
        "assistantMessageEvent": {
            "type": "text_delta", "contentIndex": 0, "delta": delta,
            "partial": message
        }
    })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn five_thousand_deltas_coalesce_into_few_frames() {
    const DELTAS: usize = 5000;
    const CHUNK: usize = 50;
    let fixture = fixture().await;
    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: fixture.session_id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let snapshot = client.snapshot().await;
    assert!(snapshot.reset, "a fresh stream starts with a snapshot");
    assert_eq!(snapshot.session_id, fixture.session_id);
    assert!(snapshot.entries.as_ref().is_some_and(|e| !e.is_empty()));
    assert!(snapshot.commands.is_some());
    assert!(snapshot.state.is_some());

    // A faux model streaming 5000 deltas of one assistant message (100 chunks of 50, one
    // millisecond apart, so the publisher is never ahead of the writer by more than the
    // tap's queue: nothing is dropped and completeness is assertable).
    let feed = fixture.service.feed(&fixture.session_id);
    let (started, text) = (Instant::now(), {
        let mut text = String::new();
        for i in 0..DELTAS {
            text.push_str(&format!("w{i} "));
        }
        text
    });
    let publisher = {
        let feed = Arc::clone(&feed);
        tokio::spawn(async move {
            let mut partial = String::new();
            for chunk in 0..(DELTAS / CHUNK) {
                for index in 0..CHUNK {
                    let piece = format!("w{} ", chunk * CHUNK + index);
                    partial.push_str(&piece);
                    feed.publish(text_delta(1, &partial, &piece)).await;
                }
                // Let the writer drain the batch it was handed (§7.4 backpressure is the
                // queue; this test only keeps the publisher from outrunning it).
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
            feed.publish(json!({"type": "message_end", "message": {"role": "assistant"}}))
                .await;
        })
    };
    let (frames, events, _) = read_events_until(&mut client, |_, event| {
        event.get("type").and_then(Value::as_str) == Some("message_end")
    })
    .await;
    publisher.await.unwrap();
    let elapsed = started.elapsed();

    // §7.4: one `events` frame per 16 ms flush tick, not one per delta.
    let windows = (elapsed.as_millis() as f64 / 16.0).ceil() as usize + 2;
    println!(
        "{DELTAS} deltas -> {} `events` frames carrying {} events in {elapsed:?} (limit {windows})",
        frames.len(),
        events.len()
    );
    assert!(
        frames.len() <= windows,
        "{} frames for {} deltas in {elapsed:?} (limit {windows})",
        frames.len(),
        DELTAS
    );
    assert!(
        events.len() < DELTAS / 10,
        "coalescing must merge the deltas, got {} events",
        events.len()
    );
    let merged: usize = events
        .iter()
        .filter(|event| event.kind() == Some("message_update"))
        .map(|event| {
            event.event["assistantMessageEvent"]["delta"]
                .as_str()
                .unwrap()
                .len()
        })
        .sum();
    assert_eq!(
        merged,
        text.len(),
        "every delta's text arrives exactly once"
    );

    // Ordering: the delta seqs are strictly increasing, and the final `message_end` is the
    // last event (it flushed immediately, §7.4).
    let seqs: Vec<u64> = events.iter().map(|event| event.seq).collect();
    assert!(
        seqs.windows(2).all(|pair| pair[0] < pair[1]),
        "seq must increase: {seqs:?}"
    );
    assert_eq!(events.last().unwrap().kind(), Some("message_end"));

    client.close().await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn resume_replays_from_the_ring_reset_after_a_dispose_and_two_subscribers_agree() {
    let fixture = fixture().await;
    let id = fixture.session_id.clone();
    let target = || -> unipi_proto::SessionTarget { id.clone().try_into().unwrap() };

    // A: open, run a command, collect what the stream numbered.
    let mut a = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: None,
        },
    );
    let snapshot = a.snapshot().await;
    assert!(snapshot.reset);
    let base = snapshot.seq;
    a.send(&Control::SessionCmd {
        id: Some(json!("c1")),
        cmd: json!({"type": "prompt", "message": "resume me"}),
    })
    .await;
    let (_, events_a, replies) = read_events_until(&mut a, |_, event| settled(event)).await;
    assert!(
        events_a
            .iter()
            .any(|event| event.kind() == Some("agent_start")),
        "the run's events arrive: {events_a:?}"
    );
    assert_eq!(replies.len(), 1, "one correlated reply");
    assert_eq!(replies[0]["type"], "ok");
    assert_eq!(replies[0]["id"], "c1");
    let last = events_a.last().unwrap().seq;
    assert!(last > base);

    // Two subscribers at once: B resumes from A's last seq, C takes a full snapshot.
    let mut b = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: Some(last),
        },
    );
    let resumed = b.snapshot().await;
    assert!(
        !resumed.reset,
        "the ring still holds the run: resume, not reset"
    );
    assert_eq!(resumed.session_id, id);
    assert!(resumed.entries.is_none(), "a resume patch has no entries");
    let replay = resumed.replay.expect("resume carries replay");
    assert!(
        replay.is_empty() || replay.last().unwrap().seq == resumed.seq,
        "replay ends at seq {}: {replay:?}",
        resumed.seq
    );
    assert!(resumed.seq >= last);

    let mut c = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: None,
        },
    );
    let full = c.snapshot().await;
    assert!(full.reset, "C asked for the whole session");
    assert!(full.seq >= resumed.seq);

    // A new run: both live subscribers see the same numbered events.
    let mut stream_b = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: Some(resumed.seq),
        },
    );
    let resumed_b = stream_b.snapshot().await;
    assert!(!resumed_b.reset);
    b.send(&Control::SessionCmd {
        id: Some(json!(2)),
        cmd: json!({"type": "prompt", "message": "second run"}),
    })
    .await;
    let (_, events_b, replies_b) = read_events_until(&mut b, |_, event| settled(event)).await;
    let (_, events_c, _) = read_events_until(&mut c, |_, event| settled(event)).await;
    assert_eq!(replies_b.len(), 1);
    assert_eq!(replies_b[0]["id"], 2);
    let live_b: Vec<(u64, Value)> = events_b
        .iter()
        .filter(|event| event.seq > full.seq)
        .map(|event| (event.seq, event.event.clone()))
        .collect();
    let live_c: Vec<(u64, Value)> = events_c
        .iter()
        .filter(|event| event.seq > full.seq)
        .map(|event| (event.seq, event.event.clone()))
        .collect();
    assert_eq!(
        live_b, live_c,
        "two subscribers of one session see the same numbered events"
    );
    let (_, events_stream_b, _) = read_events_until(&mut stream_b, |_, event| settled(event)).await;
    let live_stream_b: Vec<(u64, Value)> = events_stream_b
        .iter()
        .filter(|event| event.seq > full.seq)
        .map(|event| (event.seq, event.event.clone()))
        .collect();
    assert_eq!(
        live_stream_b, live_b,
        "a third subscriber of the same session sees the run too"
    );

    // Reset path: `session_close` disposes the runtime, so the replay can no longer be
    // trusted and the next open with the same `since_seq` answers with a snapshot.
    let mut closer = Client::open(
        &fixture.service,
        Request::SessionClose { session: target() },
    );
    match closer.control().await {
        Control::Ok { data } => assert!(data.is_none()),
        other => panic!("expected ok from session_close, got {other:?}"),
    }
    closer.close().await;
    let mut d = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: Some(resumed.seq),
        },
    );
    let reset = d.snapshot().await;
    assert!(reset.reset, "a disposed runtime resets rather than replays");
    assert!(reset.entries.is_some());

    for client in [a, b, c, stream_b, d] {
        client.close().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_list_cards_and_events_open_push_notices_cards_and_ui() {
    let fixture = fixture().await;
    // Retained before any client: the safe-mode banner must not be lost.
    fixture.hub.notice(
        unipi_host::sessions::NoticeLevel::Warning,
        "safe mode: extensions off",
    );
    let mut hub_stream = Client::open(&fixture.service, Request::EventsOpen);
    match hub_stream.control().await {
        Control::Ok { data } => assert!(data.is_none()),
        other => panic!("expected ok from events_open, got {other:?}"),
    }
    let (kind, bytes) = hub_stream.frame().await.expect("retained notice");
    assert_eq!(kind, 0);
    let notice: HubEvent = serde_json::from_slice(&bytes).unwrap();
    match notice {
        HubEvent::HostNotice { level, text } => {
            assert_eq!(level, unipi_proto::NoticeLevel::Warning);
            assert!(text.contains("safe mode"), "{text}");
        }
        other => panic!("expected the retained notice, got {other:?}"),
    }

    // `sessions_list`: headers only, the card of the session we created.
    let mut list = Client::open(&fixture.service, Request::SessionsList { archived: false });
    let cards: Vec<SessionCard> = match list.control().await {
        Control::Ok { data } => serde_json::from_value(data.expect("sessions_list data")).unwrap(),
        other => panic!("expected ok from sessions_list, got {other:?}"),
    };
    list.close().await;
    let card = cards
        .iter()
        .find(|card| card.session_id == fixture.session_id)
        .expect("the session is listed");
    assert!(card.message_count >= 2, "{}", card.message_count);
    assert_eq!(card.run_state, unipi_proto::RunState::Idle);
    assert!(card.file.ends_with(".jsonl"), "{}", card.file);

    // Live hub events: a card change, a host notice and the host-scope ui registry.
    fixture.hub.notice(
        unipi_host::sessions::NoticeLevel::Info,
        "UniPi 2.0.1 is available",
    );
    fixture.hub.emit_ui(
        Some(json!({"scope": "host", "rev": 1, "entries": [], "dialogs": [], "meta": {}})),
        None,
    );
    fixture.hub.refresh(&fixture.session_id).await;
    let mut seen = Vec::new();
    while seen.len() < 3 {
        let (kind, bytes) = tokio::time::timeout(READ_TIMEOUT, hub_stream.frame())
            .await
            .expect("timed out waiting for hub events")
            .expect("hub stream ended");
        assert_eq!(kind, 0);
        seen.push(serde_json::from_slice::<HubEvent>(&bytes).unwrap());
    }
    assert!(
        seen.iter().any(|event| matches!(
            event,
            HubEvent::HostNotice {
                level: unipi_proto::NoticeLevel::Info,
                ..
            }
        )),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|event| matches!(
            event,
            HubEvent::Ui { snapshot: Some(snapshot), .. } if snapshot["rev"] == 1
        )),
        "{seen:?}"
    );
    assert!(
        seen.iter().any(|event| matches!(
            event,
            HubEvent::SessionChanged { card } if card.session_id == fixture.session_id
        )),
        "{seen:?}"
    );
    hub_stream.close().await;
}

#[test]
fn proto_is_v2() {
    assert!(PROTO_VERSION.starts_with("2."));
}

#[allow(dead_code, reason = "kept for debugging the stream tests")]
fn state_json() -> RpcSessionState {
    serde_json::from_value(json!({
        "sessionId": "s1", "thinkingLevel": "off", "isStreaming": false, "isCompacting": false,
        "steeringMode": "one-at-a-time", "followUpMode": "one-at-a-time",
        "autoCompactionEnabled": true, "messageCount": 2, "pendingMessageCount": 0
    }))
    .unwrap()
}

/// v2 QA D-03: when the agent names a session (`session_info_changed`, e.g. the auto-title
/// after the first turn), the host re-emits the card on `events_open` — the sidebar row and
/// the header update live instead of after a restart.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_info_changed_re_emits_the_card_with_the_new_name() {
    let fixture = fixture().await;
    let mut events = fixture.hub.subscribe();
    // The tap is what watches the runtime (the feed only numbers once something taps).
    let feed = fixture.service.feed(&fixture.session_id);
    let session = fixture.hub.open(&fixture.session_id).await.unwrap();
    feed.ensure_tap(
        Arc::clone(session.backend()),
        Arc::downgrade(&fixture.service),
    );
    session
        .send(RpcCommand::Agent(AgentCommand::SetSessionName {
            name: "Ping Pong Reply".into(),
        }))
        .await
        .unwrap();
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        let event = tokio::time::timeout_at(deadline.into(), events.recv())
            .await
            .expect("the card never arrived")
            .expect("the hub event channel closed");
        let unipi_host::sessions::HubEvent::SessionChanged { card } = event else {
            continue;
        };
        if card.session_id == fixture.session_id && card.name.as_deref() == Some("Ping Pong Reply")
        {
            break;
        }
        assert!(Instant::now() < deadline, "no card carried the new name");
    }
}

/// E-04: a dialog answer rides the session stream as `SessionCmd{type:
/// extension_ui_response}` — not an RPC command. The host must hand it to the session's
/// UI bridge (the tool waiting on the approval card unblocks) instead of rejecting it
/// as a schema error: "Allow once" then never ran and the turn hung on "Working…".
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn extension_ui_response_reaches_the_session_backend() {
    let fixture = fixture().await;
    let mut client = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: fixture.session_id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let _ = client.snapshot().await;
    client
        .send(&Control::SessionCmd {
            id: Some(json!("dlg-1")),
            cmd: json!({"type": "extension_ui_response", "id": "approval-1", "confirmed": true}),
        })
        .await;
    // The correlated reply: ok (the answer was delivered), not a schema rejection.
    let deadline = Instant::now() + READ_TIMEOUT;
    loop {
        let value = tokio::time::timeout_at(deadline.into(), client.json())
            .await
            .expect("no reply for the dialog answer");
        if value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t == "ok" || t == "err")
        {
            assert_eq!(
                value["type"], "ok",
                "the answer is delivered, not rejected: {value}"
            );
            assert_eq!(value["id"], json!("dlg-1"));
            break;
        }
    }
    client.close().await;
}
