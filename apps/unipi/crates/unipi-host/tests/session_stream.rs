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

use std::fmt::Write as _;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, DuplexStream};
use tokio::task::JoinHandle;
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_host::sessions::stream::{Feed, SessionService};
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
    provider: Arc<FauxProvider>,
}

/// A hub whose sessions are answered by a faux provider, plus the session service over it.
async fn fixture() -> Fixture {
    fixture_with_extensions(Vec::new()).await
}

async fn fixture_with_extensions(
    extensions: Vec<Arc<dyn oino_core::extensions::api::ExtensionInstance>>,
) -> Fixture {
    fixture_options(extensions, false).await
}

async fn fixture_options(
    extensions: Vec<Arc<dyn oino_core::extensions::api::ExtensionInstance>>,
    has_ui: bool,
) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let agent_dir = dir.path().join("agent");
    let sessions = dir.path().join("sessions");
    std::fs::create_dir_all(&agent_dir).unwrap();
    std::fs::create_dir_all(&sessions).unwrap();
    let provider = FauxProvider::new(vec![
        FauxResponse::Text("first reply".into()),
        FauxResponse::Text("second reply".into()),
    ]);
    let mut instances = vec![Arc::new(FauxComponent(Arc::clone(&provider)))
        as Arc<dyn oino_core::extensions::api::ExtensionInstance>];
    instances.extend(extensions);
    let loader = StaticLoader(instances);
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "test-key");
    let hub = SessionHub::create(HostHubOptions {
        cwd: dir.path().to_path_buf(),
        agent_dir,
        docs_root: None,
        session_dir: Some(sessions),
        mode: ExtensionMode::Desktop,
        has_ui,
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
    wait_idle(&hub, &id).await;
    let service = SessionService::new(Arc::clone(&hub));
    Fixture {
        _dir: dir,
        hub,
        service,
        session_id: id,
        provider,
    }
}

/// A 1.0.6 app's plain send must survive an already-running wake continuation:
/// it queues as a follow-up (its own later turn) and an explicit steer still steers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_app_prompt_follows_active_run_without_steering() {
    let f = fixture().await;
    f.provider.route(|_| Some(FauxResponse::Never));
    let session = f.hub.open(&f.session_id).await.unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::Prompt {
            message: "wake continuation".into(),
            images: Vec::new(),
            streaming_behavior: None,
        }))
        .await
        .unwrap();
    let mut client = Client::open(
        &f.service,
        Request::SessionOpen {
            session: f.session_id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let snapshot = client.snapshot().await;
    assert_eq!(snapshot.state.unwrap()["isStreaming"], true);
    client
        .send(&Control::SessionCmd {
            id: Some(json!("legacy-prompt")),
            cmd: json!({"type": "prompt", "message": "phone send"}),
        })
        .await;
    let (_, reply) = until_reply(&mut client, "legacy-prompt").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(reply["data"]["disposition"], "queued");
    assert!(reply["data"]["messageId"].is_string());
    client
        .send(&Control::SessionCmd {
            id: Some(json!("explicit-steer")),
            cmd: json!({"type": "steer", "message": "steer me"}),
        })
        .await;
    let (_, reply) = until_reply(&mut client, "explicit-steer").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    let state = session
        .send(RpcCommand::Agent(AgentCommand::GetState))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(state["followUp"], json!(["phone send"]));
    assert_eq!(state["steering"], json!(["steer me"]));
    session
        .send(RpcCommand::Agent(AgentCommand::ClearQueue))
        .await
        .unwrap();
    session
        .send(RpcCommand::Agent(AgentCommand::Abort))
        .await
        .unwrap();
    client.close().await;
}

/// T15: a chat whose last turn was cut off wakes, repair appends the interrupted marker,
/// and a 1.0.6 plain prompt that arrived before readiness still starts its own run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_app_prompt_after_interrupted_wake_starts_a_run() {
    let f = fixture().await;
    let id = f.session_id.clone();
    let card = f.hub.card(&id).await.unwrap().expect("session card");
    f.hub.close(&id).await.unwrap();
    // Cut off the leaf turn: a user message followed by an open run, no answer.
    let text = std::fs::read_to_string(&card.file).unwrap();
    let leaf = text
        .lines()
        .last()
        .and_then(|line| serde_json::from_str::<Value>(line).ok())
        .and_then(|entry| entry["id"].as_str().map(str::to_owned))
        .expect("leaf entry");
    let mut text = text;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    writeln!(
        text,
        "{}",
        json!({"id": "u-unfinished", "parentId": leaf,
            "timestamp": "2026-11-16T00:00:00.000Z", "type": "message",
            "message": {"role": "user", "content": [{"type": "text", "text": "unfinished work"}]}})
    )
    .unwrap();
    writeln!(
        text,
        "{}",
        json!({"id": "r-unfinished", "parentId": "u-unfinished",
            "timestamp": "2026-11-16T00:00:00.001Z", "type": "custom",
            "customType": "oino.run-begun", "data": {}})
    )
    .unwrap();
    std::fs::write(&card.file, text).unwrap();

    let mut client = Client::open(
        &f.service,
        Request::SessionOpen {
            session: id.try_into().unwrap(),
            since_seq: None,
        },
    );
    let snapshot = client.snapshot().await;
    assert!(
        snapshot.waking,
        "the file-opened chat wakes behind its history"
    );
    // Sent before the wake completes: the pre-ready path that used to become a steer.
    client
        .send(&Control::SessionCmd {
            id: Some(json!("after-wake")),
            cmd: json!({"type": "prompt", "message": "phone send"}),
        })
        .await;
    let (mut events, reply) = until_reply(&mut client, "after-wake").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(
        reply["data"]["disposition"], "started",
        "repair must leave the chat idle so the user's prompt starts a run: {reply}"
    );
    // The prompt's own turn really runs and settles (not merely queued).
    if !events.iter().any(settled) {
        let (_, more, _) = read_events_until(&mut client, |_, event| settled(event)).await;
        events.extend(more.into_iter().map(|item| item.event.into_value()));
    }
    assert!(
        events.iter().any(|event| {
            event["type"] == "message_start" && event["message"]["role"] == "assistant"
        }),
        "the prompt produced an assistant turn"
    );
    client.close().await;
}

/// An idle chat starts a plain prompt's run immediately (the pre-T15 behaviour).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn plain_app_prompt_on_idle_session_starts_at_once() {
    let f = fixture().await;
    let mut client = Client::open(
        &f.service,
        Request::SessionOpen {
            session: f.session_id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let snapshot = client.snapshot().await;
    assert_eq!(snapshot.state.unwrap()["isStreaming"], false);
    client
        .send(&Control::SessionCmd {
            id: Some(json!("idle-prompt")),
            cmd: json!({"type": "prompt", "message": "start now"}),
        })
        .await;
    let (mut events, reply) = until_reply(&mut client, "idle-prompt").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(reply["data"]["disposition"], "started", "{reply}");
    if !events.iter().any(settled) {
        let (_, more, _) = read_events_until(&mut client, |_, event| settled(event)).await;
        events.extend(more.into_iter().map(|item| item.event.into_value()));
    }
    assert!(
        events.iter().any(|event| event["type"] == "agent_start"
            || (event["type"] == "message_start" && event["message"]["role"] == "assistant")),
        "the run started without waiting for anything else"
    );
    client.close().await;
}

/// T8: closing a chat and reopening it at once must not leave the new stream subscribed to
/// the dead runtime's feed (whose tap ends every subscriber it sees) — the reopen gets a
/// fresh feed for the new runtime and keeps receiving events.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn reopen_after_close_keeps_receiving_events() {
    let f = fixture().await;
    let id = f.session_id.clone();
    let mut first = Client::open(
        &f.service,
        Request::SessionOpen {
            session: id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    let _ = first.snapshot().await;
    f.hub.close(&id).await.unwrap();
    // What `session_close` does: the old runtime generation's feed can never resume.
    f.service.feed(&id).mark_disposed();

    let mut second = Client::open(
        &f.service,
        Request::SessionOpen {
            session: id.try_into().unwrap(),
            since_seq: None,
        },
    );
    let snapshot = second.snapshot().await;
    assert!(snapshot.reset);
    second
        .send(&Control::SessionCmd {
            id: Some(json!("after-reopen")),
            cmd: json!({"type": "prompt", "message": "after reopen"}),
        })
        .await;
    let (mut events, reply) = until_reply(&mut second, "after-reopen").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    if !events.iter().any(settled) {
        let (_, more, _) = read_events_until(&mut second, |_, event| settled(event)).await;
        events.extend(more.into_iter().map(|item| item.event.into_value()));
    }
    assert!(
        events.iter().any(|event| event["type"] == "agent_start"),
        "the reopened stream follows the new runtime"
    );
    first.close().await;
    second.close().await;
}

/// T8: a disposed feed belongs to a dead runtime generation. A reopen must get a fresh
/// feed so the dead tap's `end_streams` cannot clear the new stream's subscription.
#[tokio::test]
async fn a_disposed_feed_is_never_reused_by_a_reopen() {
    let f = fixture().await;
    let old = f.service.feed(&f.session_id);
    old.mark_disposed();
    let new = f.service.feed(&f.session_id);
    assert!(!Arc::ptr_eq(&old, &new));
    assert!(!new.is_disposed());
    assert_eq!(new.session_id(), f.session_id);
}

#[tokio::test]
async fn feed_retaps_after_runtime_rekeys_and_old_id_wakes() {
    let f = fixture().await;
    let old = f.hub.open(&f.session_id).await.unwrap();
    let feed = Feed::new(&f.session_id);
    let mut subscriber = feed.subscribe();
    feed.ensure_tap(Arc::clone(old.backend()), std::sync::Weak::new());
    old.send(RpcCommand::Agent(AgentCommand::NewSession {
        parent_session: None,
    }))
    .await
    .unwrap();
    tokio::time::timeout(READ_TIMEOUT, async {
        while feed.session_id() == f.session_id {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("runtime did not rekey");
    let new = f.hub.open(&f.session_id).await.unwrap();
    assert_ne!(
        old.backend().runtime_identity(),
        new.backend().runtime_identity()
    );
    let before = feed.seq();
    feed.ensure_tap(Arc::clone(new.backend()), std::sync::Weak::new());
    new.send(RpcCommand::Agent(AgentCommand::SetSessionName {
        name: "retapped".into(),
    }))
    .await
    .unwrap();
    tokio::time::timeout(READ_TIMEOUT, async {
        loop {
            let event = subscriber.recv().await.expect("subscriber ended");
            if event.seq > before && event.event["type"] == "session_info_changed" {
                break;
            }
        }
    })
    .await
    .expect("new runtime event was not numbered");
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

    /// Hub cards can arrive independently of a topic subscription reply.
    async fn topic(&mut self) -> Value {
        tokio::time::timeout(READ_TIMEOUT, async {
            loop {
                let value = self.json().await;
                if value.get("topic").is_some() {
                    return value;
                }
                assert_eq!(value["kind"], "session_changed", "{value}");
            }
        })
        .await
        .expect("timed out waiting for a topic frame")
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
            write!(text, "w{i} ").unwrap();
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
    let windows = usize::try_from(elapsed.as_millis().div_ceil(16)).unwrap() + 2;
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
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
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
        .map(|event| (event.seq, event.event.clone().into_value()))
        .collect();
    let live_c: Vec<(u64, Value)> = events_c
        .iter()
        .filter(|event| event.seq > full.seq)
        .map(|event| (event.seq, event.event.clone().into_value()))
        .collect();
    assert_eq!(
        live_b, live_c,
        "two subscribers of one session see the same numbered events"
    );
    let (_, events_stream_b, _) = read_events_until(&mut stream_b, |_, event| settled(event)).await;
    let live_stream_b: Vec<(u64, Value)> = events_stream_b
        .iter()
        .filter(|event| event.seq > full.seq)
        .map(|event| (event.seq, event.event.clone().into_value()))
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
    let hello = hub_stream.json().await;
    assert_eq!(hello["kind"], "hello");
    assert!(hello["capabilities"]
        .as_array()
        .unwrap()
        .contains(&json!("topics/1")));
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
    assert!(
        std::path::Path::new(&card.file)
            .extension()
            .is_some_and(|ext| ext == "jsonl"),
        "{}",
        card.file
    );

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

/// The `id`s of a full snapshot's assistant / user messages, in branch order.
fn message_ids(snapshot: &SessionSnapshot, role: &str) -> Vec<String> {
    snapshot
        .entries
        .as_ref()
        .expect("a full snapshot has entries")
        .iter()
        .filter(|entry| entry["type"] == "message" && entry["message"]["role"] == role)
        .map(|entry| entry["id"].as_str().unwrap().to_owned())
        .collect()
}

/// Reads until the correlated reply `id` arrives; returns the events before it and the reply.
async fn until_reply(client: &mut Client, id: &str) -> (Vec<Value>, Value) {
    let mut events = Vec::new();
    loop {
        let value = client.json().await;
        match value.get("type").and_then(Value::as_str) {
            Some("events") => {
                for item in value["items"].as_array().unwrap() {
                    events.push(item["event"].clone());
                }
            }
            Some("ok" | "err") if value["id"] == id => return (events, value),
            _ => {}
        }
    }
}

/// [chat-tree] "Return here" / the tree view: `_oino/navigate_tree` moves the leaf, every
/// client of the session hears `session_tree{leafId}` (the requester before its reply), and
/// the branch page the app then reloads ends at the new leaf. Navigating to a user message
/// lands on its parent and hands its text back (pi `navigateTree`).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
async fn navigate_tree_moves_the_branch_for_every_client() {
    let fixture = fixture().await;
    let id = fixture.session_id.clone();
    let target = || -> unipi_proto::SessionTarget { id.clone().try_into().unwrap() };
    let mut a = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: None,
        },
    );
    let _ = a.snapshot().await;
    a.send(&Control::SessionCmd {
        id: Some(json!("p2")),
        cmd: json!({"type": "prompt", "message": "second question"}),
    })
    .await;
    let _ = read_events_until(&mut a, |_, event| settled(event)).await;
    wait_idle(&fixture.hub, &id).await;

    let mut b = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: None,
        },
    );
    let full = b.snapshot().await;
    let answers = message_ids(&full, "assistant");
    let users = message_ids(&full, "user");
    assert_eq!(answers.len(), 2, "two answered turns: {:?}", full.entries);
    // Durable run boundaries follow the assistant; the leaf is the last entry,
    // not necessarily the last visible message.
    assert_eq!(
        full.leaf_id.as_ref(),
        full.entries.as_ref().unwrap().last().map(|e| &e["id"])
    );

    // 1. Return to the first answer.
    a.send(&Control::SessionCmd {
        id: Some(json!("nav1")),
        cmd: json!({"type": "_oino/navigate_tree", "targetId": answers[0]}),
    })
    .await;
    let (events, reply) = until_reply(&mut a, "nav1").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(reply["data"]["cancelled"], false);
    let moved: Vec<&Value> = events
        .iter()
        .filter(|event| event["type"] == "session_tree")
        .collect();
    assert_eq!(
        moved.len(),
        1,
        "the requester hears it before the reply: {events:?}"
    );
    let first_leaf = moved[0]["leafId"].clone();
    assert_ne!(first_leaf, full.leaf_id.unwrap());
    let (_, heard, _) = read_events_until(&mut b, |_, event| event["type"] == "session_tree").await;
    assert_eq!(
        heard.last().unwrap().event["leafId"],
        first_leaf,
        "the other client hears it too"
    );

    // 2. The branch page the app reloads ends at the new leaf.
    a.send(&Control::SessionCmd {
        id: Some(json!("page")),
        cmd: json!({"type": "get_entries", "limit": 50}),
    })
    .await;
    let (_, page) = until_reply(&mut a, "page").await;
    assert_eq!(page["data"]["leafId"], first_leaf);
    let ids: Vec<&str> = page["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| entry["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids.last().copied(), first_leaf.as_str());
    let leaf_entry = page["data"]["entries"].as_array().unwrap().last().unwrap();
    assert_eq!(leaf_entry["type"], "model_change");
    assert_eq!(leaf_entry["parentId"], answers[0]);
    assert!(
        !ids.contains(&users[1].as_str()),
        "the later turn left the branch"
    );

    // 3. A user message: the leaf is its parent, its text comes back for the composer.
    a.send(&Control::SessionCmd {
        id: Some(json!("nav2")),
        cmd: json!({"type": "_oino/navigate_tree", "targetId": users[1]}),
    })
    .await;
    let (events, reply) = until_reply(&mut a, "nav2").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(reply["data"]["editorText"], "second question");
    let moved = events
        .iter()
        .find(|event| event["type"] == "session_tree")
        .expect("session_tree");
    let second_leaf = moved["leafId"].clone();
    a.send(&Control::SessionCmd {
        id: Some(json!("page2")),
        cmd: json!({"type": "get_entries", "limit": 50}),
    })
    .await;
    let (_, page) = until_reply(&mut a, "page2").await;
    assert_eq!(page["data"]["leafId"], second_leaf);
    let entries = page["data"]["entries"].as_array().unwrap();
    let leaf_entry = entries.last().unwrap();
    let parent = &full
        .entries
        .as_ref()
        .unwrap()
        .iter()
        .find(|e| e["id"] == users[1])
        .unwrap()["parentId"];
    assert_eq!(leaf_entry["type"], "model_change");
    assert_eq!(
        &leaf_entry["parentId"], parent,
        "navigation lands on the user's parent before persisting the model"
    );
    assert!(
        entries.iter().all(|e| e["id"] != users[1]),
        "the user being edited left the branch"
    );

    // 4. An unknown entry fails and moves nothing.
    a.send(&Control::SessionCmd {
        id: Some(json!("nav3")),
        cmd: json!({"type": "_oino/navigate_tree", "targetId": "no-such-entry"}),
    })
    .await;
    let (events, reply) = until_reply(&mut a, "nav3").await;
    assert_eq!(
        reply["type"], "err",
        "an unknown entry is an error: {reply}"
    );
    assert!(events.iter().all(|event| event["type"] != "session_tree"));
    a.close().await;
    b.close().await;
}

/// OIN-75: switching away from a session and re-opening `session_open` without `since_seq`
/// returns queued `steering` / `followUp` texts in `SessionSnapshot.state` (`RpcSessionState`).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn session_open_snapshot_includes_queued_steering_on_reattach() {
    let fixture = fixture().await;
    let id = fixture.session_id.clone();
    let target = || -> unipi_proto::SessionTarget { id.clone().try_into().unwrap() };

    let mut a = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: None,
        },
    );
    let _ = a.snapshot().await;
    a.send(&Control::SessionCmd {
        id: Some(json!("steer1")),
        cmd: json!({"type": "steer", "message": "keep API unchanged"}),
    })
    .await;
    let (_, reply) = until_reply(&mut a, "steer1").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    a.send(&Control::SessionCmd {
        id: Some(json!("fu1")),
        cmd: json!({"type": "follow_up", "message": "run cargo test after"}),
    })
    .await;
    let (_, reply) = until_reply(&mut a, "fu1").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    a.close().await;

    let mut b = Client::open(
        &fixture.service,
        Request::SessionOpen {
            session: target(),
            since_seq: None,
        },
    );
    let full = b.snapshot().await;
    assert!(full.reset);
    let state: RpcSessionState =
        serde_json::from_value(full.state.expect("snapshot state")).expect("RpcSessionState");
    assert_eq!(state.pending_message_count, 2);
    assert_eq!(state.steering, vec!["keep API unchanged".to_owned()]);
    assert_eq!(state.follow_up, vec!["run cargo test after".to_owned()]);
    b.close().await;
}

/// "Send as note": the app's `SessionCmd{type: append_note}` reaches the session as
/// `AgentCommand::AppendNote` (no prompt, no run): the reply carries the entry id and the
/// stream publishes the `oino.note` custom entry to the subscribers.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn append_note_is_saved_and_published_without_starting_a_run() {
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
            id: Some(json!("note-1")),
            cmd: json!({"type": "append_note", "text": "only for me"}),
        })
        .await;
    let deadline = Instant::now() + READ_TIMEOUT;
    let mut reply = None;
    let mut published = None;
    while reply.is_none() || published.is_none() {
        let value = tokio::time::timeout_at(deadline.into(), client.json())
            .await
            .expect("no reply / entry for the note");
        match value.get("type").and_then(Value::as_str) {
            Some("events") => {
                for item in value["items"].as_array().unwrap() {
                    let event = &item["event"];
                    assert_ne!(event["type"], "agent_start", "a note never starts a run");
                    if event["type"] == "entry_appended" {
                        published = Some(event["entry"].clone());
                    }
                }
            }
            Some("ok" | "err") if value["id"] == "note-1" => reply = Some(value),
            _ => {}
        }
    }
    let reply = reply.unwrap();
    assert_eq!(reply["type"], "ok", "{reply}");
    let entry = published.unwrap();
    assert_eq!(reply["data"]["entryId"], entry["id"]);
    assert_eq!(entry["type"], "custom");
    assert_eq!(entry["customType"], "oino.note");
    assert_eq!(entry["data"]["text"], "only for me");
    client.close().await;
}

/// Topic controls use the same events stream, and an inotify change produces a delta.
#[tokio::test]
async fn topics_subscribe_delta_resume_gap_and_unsubscribe() {
    let fixture = fixture().await;
    let dir = tempfile::tempdir().unwrap();
    let topic = format!("fs:{}", dir.path().display());
    let mut client = Client::open(&fixture.service, Request::EventsOpen);
    assert!(matches!(client.control().await, Control::Ok { .. }));
    assert_eq!(client.json().await["kind"], "hello");
    client
        .send(&Control::Subscribe {
            topics: vec![unipi_proto::TopicSub {
                topic: topic.clone(),
                since: None,
            }],
        })
        .await;
    let snapshot = client.topic().await;
    assert_eq!(snapshot["topic"], topic, "{snapshot}");
    assert_eq!(snapshot["seq"], 0);
    assert_eq!(snapshot["snapshot"]["invalidate"], true);
    std::fs::write(dir.path().join("changed"), "one").unwrap();
    let delta = client.topic().await;
    assert_eq!(delta["topic"], topic);
    assert_eq!(delta["seq"], 1);
    assert_eq!(delta["delta"]["invalidate"], true);
    // A second connection can resume the first connection's cursor while its source lives.
    let mut resumed = Client::open(&fixture.service, Request::EventsOpen);
    resumed.control().await;
    assert_eq!(resumed.json().await["kind"], "hello");
    resumed
        .send(&Control::Subscribe {
            topics: vec![unipi_proto::TopicSub {
                topic: topic.clone(),
                since: Some(unipi_proto::TopicCursor {
                    epoch: snapshot["epoch"].as_u64().unwrap(),
                    seq: 0,
                }),
            }],
        })
        .await;
    assert_eq!(resumed.topic().await, delta);
    // A foreign epoch must snapshot, not replay.
    resumed
        .send(&Control::Subscribe {
            topics: vec![unipi_proto::TopicSub {
                topic: topic.clone(),
                since: Some(unipi_proto::TopicCursor { epoch: 0, seq: 0 }),
            }],
        })
        .await;
    assert!(resumed.topic().await.get("snapshot").is_some());
    resumed.close().await;
    client
        .send(&Control::Unsubscribe {
            topics: vec![topic.clone()],
        })
        .await;
    // Read a fresh subscription reply as a barrier proving unsubscribe was processed.
    client
        .send(&Control::Subscribe {
            topics: vec![unipi_proto::TopicSub {
                topic: "sessions".into(),
                since: None,
            }],
        })
        .await;
    assert_eq!(client.topic().await["topic"], "sessions");
    assert_eq!(
        unipi_host::topic_hub::TopicHub::shared().subscribers(&topic),
        0
    );
    assert!(!unipi_host::topic_hub::TopicHub::shared().source_running(&topic));
    client.close().await;
}

/// Bundled commands forward their args through the host API, not through a client prompt.
/// Admission of the outer slash command must not consume that independent user input.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[allow(
    clippy::too_many_lines,
    reason = "Keep the complete operation scenario and its lifecycle in one place."
)]
async fn extension_command_follow_up_runs_from_idle_over_host_stream() {
    use oino_core::extensions::api::{CommandSpec, Registration};
    use oino_core::extensions::host::{Delivery, HostApi, HostOp};
    use oino_core::testing::FakeExtension;
    let binding = Arc::new(std::sync::OnceLock::<(
        Arc<oino_core::services::HostRouter>,
        oino_core::extensions::api::CallContext,
    )>::new());
    let command_binding = Arc::clone(&binding);
    let extension = FakeExtension::new(
        "forward-command",
        Registration {
            commands: vec![CommandSpec {
                name: "forward".into(),
                description: None,
                hidden: false,
                completions: false,
            }],
            ..Registration::default()
        },
    )
    .on_command(move |_, args| {
        let (host, cx) = command_binding.get().unwrap();
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current().block_on(async {
                assert_eq!(
                    host.call("forward-command", cx, HostOp::IsIdle)
                        .await
                        .unwrap(),
                    json!(true),
                    "a command alone is idle"
                );
                host.call(
                    "forward-command",
                    cx,
                    HostOp::SendUserMessage {
                        content: oino_ai::UserContentField::Text(args.into()),
                        deliver_as: Some(Delivery::FollowUp),
                    },
                )
                .await
                .unwrap();
            });
        });
        Ok(None)
    });
    let f = fixture_with_extensions(vec![Arc::new(extension)]).await;
    let services = f.hub.core_services().await.unwrap();
    binding
        .set((
            Arc::clone(&services.host),
            oino_core::extensions::api::CallContext {
                session_id: f.session_id.clone(),
                call_kind: oino_core::extensions::api::CallKind::Command,
                ..Default::default()
            },
        ))
        .unwrap();
    let mut client = Client::open(
        &f.service,
        Request::SessionOpen {
            session: f.session_id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    client.snapshot().await;
    client.send(&Control::SessionCmd {
        id: Some(json!("command")),
        cmd: json!({"type":"prompt", "message":"/forward do X\n- long task", "streamingBehavior":"followUp"}),
    }).await;
    let (_, reply) = until_reply(&mut client, "command").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(reply["data"]["disposition"], "handled");
    let deadline = Instant::now() + READ_TIMEOUT;
    while f.provider.requests().len() < 2 {
        assert!(
            Instant::now() < deadline,
            "extension input never started a run"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    wait_idle(&f.hub, &f.session_id).await;
    assert_eq!(
        f.provider.requests().len(),
        2,
        "one initial run plus one extension run"
    );
    let requests = f.provider.requests();
    let users: Vec<_> = requests[1]["context"]["messages"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|m| m["role"] == "user" && m.to_string().contains("do X"))
        .collect();
    assert_eq!(users.len(), 1);
    client
        .send(&Control::SessionCmd {
            id: Some(json!("entries")),
            cmd: json!({"type":"get_entries", "limit":50}),
        })
        .await;
    let (_, page) = until_reply(&mut client, "entries").await;
    let persisted: Vec<_> = page["data"]["entries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|e| e["message"]["role"] == "user" && e.to_string().contains("do X"))
        .collect();
    assert_eq!(persisted.len(), 1, "extension args persisted once");
    assert_ne!(persisted[0]["id"], reply["data"]["messageId"]);
    client.close().await;
}

/// Hidden client misses the request and its ring slot. A full open must restore the
/// real bridge request, and answering the restored id must resolve the host wait.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn hidden_dialog_survives_ring_overrun_and_answer_resolves_wait() {
    use oino_core::extensions::api::CallContext;
    use oino_core::extensions::host::{HostApi, HostOp};
    let f = fixture_options(Vec::new(), true).await;
    let open = |since_seq| {
        Client::open(
            &f.service,
            Request::SessionOpen {
                session: f.session_id.clone().try_into().unwrap(),
                since_seq,
            },
        )
    };
    let mut hidden = open(None);
    let before = hidden.snapshot().await.seq;
    hidden.close().await; // hide before the request exists
    let services = f.hub.core_services().await.unwrap();
    let host = Arc::clone(&services.host);
    let cx = CallContext {
        session_id: f.session_id.clone(),
        ..Default::default()
    };
    let wait = tokio::spawn(async move {
        host.call(
            "snapshot-probe",
            &cx,
            HostOp::UiDialog {
                method: "confirm".into(),
                params: serde_json::from_value(json!({"title":"Answer after reconnect"})).unwrap(),
                cancel: None,
            },
        )
        .await
        .unwrap()
    });
    let session = f.hub.open(&f.session_id).await.unwrap();
    let deadline = Instant::now() + READ_TIMEOUT;
    let request = loop {
        let ui = session
            .send(RpcCommand::Oino(
                oino_client::protocol::OinoCommand::UiSnapshot,
            ))
            .await
            .unwrap()
            .unwrap();
        let requests = ui["pending_requests"].as_array().unwrap();
        if let Some(request) = requests.first() {
            break request.clone();
        }
        assert!(Instant::now() < deadline, "dialog did not become pending");
        tokio::task::yield_now().await;
    };
    assert!(!wait.is_finished());
    let feed = f.service.feed(&f.session_id);
    for _ in 0..=unipi_host::sessions::stream::RING_CAPACITY {
        feed.publish(json!({"type":"turn_start"})).await;
    }
    assert!(!feed.can_resume(before));
    let opened_at = Instant::now();
    let mut shown = open(Some(before));
    let snapshot = shown.snapshot().await;
    eprintln!(
        "pending-request probe: full open {:?}, snapshot {} bytes, requests {}",
        opened_at.elapsed(),
        serde_json::to_vec(&snapshot).unwrap().len(),
        snapshot.pending_requests.as_ref().map_or(0, Vec::len)
    );
    assert!(snapshot.reset);
    assert_eq!(
        snapshot.pending_requests.as_ref().unwrap(),
        &vec![request.clone()]
    );
    shown.close().await;
    // since_seq path carries the same authoritative state, even with an empty replay.
    let mut resumed = open(Some(snapshot.seq));
    let snapshot = resumed.snapshot().await;
    assert!(!snapshot.reset);
    assert_eq!(snapshot.pending_requests.unwrap(), vec![request.clone()]);
    resumed
        .send(&Control::SessionCmd {
            id: Some(json!("answer-restored")),
            cmd: json!({"type":"extension_ui_response", "id":request["id"], "confirmed":true}),
        })
        .await;
    let (_, reply) = until_reply(&mut resumed, "answer-restored").await;
    assert_eq!(reply["type"], "ok");
    assert_eq!(
        tokio::time::timeout(READ_TIMEOUT, wait)
            .await
            .unwrap()
            .unwrap(),
        json!({"confirmed":true})
    );
    resumed.close().await;
    let mut answered = open(None);
    assert_eq!(answered.snapshot().await.pending_requests, Some(Vec::new()));
    answered.close().await;
}

/// Refusal is a successful protocol reply, but never a delivered user message.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refused_graph_command_returns_reason_without_consuming_images() {
    use oino_core::extensions::api::{CommandSpec, Registration};
    use oino_core::testing::FakeExtension;
    let extension = FakeExtension::new(
        "refuse-graph",
        Registration {
            commands: vec![CommandSpec {
                name: "swarm".into(),
                description: None,
                hidden: false,
                completions: false,
            }],
            ..Default::default()
        },
    )
    .on_command(|_, _| {
        Ok(Some(
            json!({"disposition":"refused", "reason":"live graph in another chat"}),
        ))
    });
    let f = fixture_with_extensions(vec![Arc::new(extension)]).await;
    let before = f.provider.requests().len();
    let mut client = Client::open(
        &f.service,
        Request::SessionOpen {
            session: f.session_id.clone().try_into().unwrap(),
            since_seq: None,
        },
    );
    client.snapshot().await;
    client
        .send(&Control::SessionCmd {
            id: Some(json!("refused")),
            cmd: json!({
                "type":"prompt", "message":"/swarm preserve this text", "images":[
                    {"type":"image", "data":"aW1hZ2Ux", "mimeType":"image/png"},
                    {"type":"image", "data":"aW1hZ2Uy", "mimeType":"image/png"}
                ]
            }),
        })
        .await;
    let (_, reply) = until_reply(&mut client, "refused").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(reply["data"]["disposition"], "refused");
    assert_eq!(reply["data"]["reason"], "live graph in another chat");
    assert_eq!(f.provider.requests().len(), before);
    client.close().await;
}
