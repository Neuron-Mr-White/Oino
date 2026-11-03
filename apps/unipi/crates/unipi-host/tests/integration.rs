//! WP-H10 acceptance (spec-host §12): the headless host + client end to end and the
//! §7.5 performance gates.
//!
//! Spawns the real `unipi-host` binary:
//!
//! ```text
//! unipi-host serve --data-dir <tmp> --relay https://127.0.0.1:9 --relay-only --no-extensions
//! ```
//!
//! with a sandboxed `OINO_AGENT_DIR`, then drives it over `app.sock` (the named pipe on
//! Windows) with the wire frames: local-admin ops, `sessions_list`, `session_new`, a
//! deterministic faux provider streaming a 100k-token reply, a drop + `since_seq` resume, a
//! terminal, a cron run and `push_list`.
//!
//! The faux provider is an `openai-completions` provider defined in the sandbox's
//! `models.json`, talking to a loopback SSE server in this test (the `provider-faux` fixture
//! reads a script from request metadata, which the host protocol cannot set). It streams
//! [`REPLY_CHARS`] characters in [`CHUNKS`] SSE chunks, so the 16 ms coalescing bound is
//! measured on real deltas.
//!
//! Ignored by default: it builds/loads a wasm provider from `extensions/dist`, spawns a
//! process and measures. Run it with
//!
//! ```text
//! cargo xtask wasm   # once: the provider package is read from extensions/dist
//! cargo test -p unipi-host --test integration -- --ignored --nocapture
//! ```
//!
//! Numbers are recorded in `docs/dev/v2/qa/host-perf.md`.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::{json, Value};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

use unipi_host::data_dir::HostPaths;
use unipi_host::ipc;
use unipi_proto::frame::{encode, FrameKind};
use unipi_proto::PROTO_VERSION;

/// The deterministic reply: 100k tokens at the usual ~4 characters per token.
const REPLY_CHARS: usize = 400_000;
/// SSE content chunks the loopback provider sends (4k characters each, ~2 ms apart).
const CHUNKS: usize = 100;
/// Per-chunk delay: 100 × 2 ms keeps the stream spread over several 16 ms flush windows.
const CHUNK_DELAY: Duration = Duration::from_millis(2);
/// A marker the terminal and the faux cron run print (Unix-only scenarios).
#[cfg(unix)]
const MARKER: &str = "H10-OK";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../../..")
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("../../../.."))
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().filter_map(Result::ok) {
        let path = entry.path();
        let target = to.join(entry.file_name());
        if path.is_dir() {
            copy_dir(&path, &target);
        } else {
            std::fs::copy(&path, &target).unwrap();
        }
    }
}

/// Stages the distribution's `openai-completions` provider into the sandbox's extension
/// dir and allows the loopback SSE server's host (the manifest lists `localhost`, not
/// `127.0.0.1`).
fn stage_provider(agent_dir: &Path) {
    let package = repo_root().join("extensions/dist/oino.openai-compat");
    assert!(
        package.join("component.wasm").is_file(),
        "missing {}; run `cargo xtask wasm` first (the H10 test loads a real provider \
         component)",
        package.display()
    );
    let target = agent_dir.join("extensions/oino.openai-compat");
    copy_dir(&package, &target);
    let manifest = target.join("extension.json");
    let mut value: Value = serde_json::from_slice(&std::fs::read(&manifest).unwrap()).unwrap();
    if let Some(hosts) = value
        .pointer_mut("/permissions/network/hosts")
        .and_then(Value::as_array_mut)
    {
        hosts.push(Value::String("127.0.0.1".to_owned()));
    }
    std::fs::write(&manifest, serde_json::to_vec(&value).unwrap()).unwrap();
}

/// A spawned `unipi-host serve`, killed on drop.
struct Host {
    _dir: tempfile::TempDir,
    paths: HostPaths,
    child: Child,
    log: PathBuf,
    agent_dir: PathBuf,
    work: PathBuf,
}

impl Host {
    fn start(sse_port: u16) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let agent_dir = root.join("agent");
        let work = root.join("work");
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        std::fs::create_dir_all(root.join("home")).unwrap();
        // An empty dist dir: the staging tree's real packages must not load (only the
        // sandboxed provider below).
        let dist = root.join("dist");
        std::fs::create_dir_all(&dist).unwrap();
        stage_provider(&agent_dir);
        std::fs::write(
            agent_dir.join("models.json"),
            serde_json::to_vec(&json!({
                "providers": {
                    "faux": {
                        "baseUrl": format!("http://127.0.0.1:{sse_port}/v1"),
                        "api": "openai-completions",
                        "apiKey": "h10-test-key",
                        "authHeader": true,
                        "models": [{
                            "id": "faux-1", "name": "Faux 1",
                            "reasoning": false, "input": ["text"],
                            "contextWindow": 1_000_000, "maxTokens": 200_000,
                            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}
                        }]
                    }
                }
            }))
            .unwrap(),
        )
        .unwrap();
        let paths = HostPaths::from_dir(root.join("host"));
        std::fs::create_dir_all(paths.root.clone()).unwrap();
        // Push on for this PC's own client, so `push_list` proves the enabled round trip.
        std::fs::write(
            paths.config(),
            serde_json::to_vec(&json!({"push": {"enabled": true}})).unwrap(),
        )
        .unwrap();
        let log_path = root.join("host.log");
        let log = std::fs::File::create(&log_path).unwrap();
        let mut command = Command::new(env!("CARGO_BIN_EXE_unipi-host"));
        command
            .arg("--data-dir")
            .arg(&paths.root)
            .arg("serve")
            .arg("--relay")
            .arg("https://127.0.0.1:9")
            .arg("--relay-only")
            .arg("--no-extensions")
            .arg("--mode")
            .arg("manual")
            .env("OINO_AGENT_DIR", &agent_dir)
            .env("OINO_DIST_EXTENSIONS", &dist)
            .env(
                "RUST_LOG",
                "unipi_host=info,oino_client=info,oino_wasm_host=warn",
            )
            .env_remove("OINO_HOST_DIR")
            .env_remove("UNIPI_HOST_DIR")
            .env_remove("UNIPI_UPDATE_FEED")
            .env_remove("UNIPI_NOTIFY_CONFIG_JSON")
            .env_remove("UNIPI_NOTIFY_ENDPOINTS_JSON")
            .env_remove("OINO_FAUX_RESPONSES")
            .stdin(Stdio::null())
            .stdout(Stdio::from(log.try_clone().unwrap()))
            .stderr(Stdio::from(log));
        if cfg!(unix) {
            command.env("HOME", root.join("home"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let bin = root.join("faux-oino");
            std::fs::write(
                &bin,
                format!(
                    "#!/bin/sh\necho '{{\"type\":\"turn_end\",\"message\":{{\"role\":\"assistant\",\
                     \"content\":[{{\"type\":\"text\",\"text\":\"{MARKER} cron\"}}],\
                     \"usage\":{{\"cost\":{{\"total\":0.0}}}}}}}}'\nexit 0\n"
                ),
            )
            .unwrap();
            std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
            let check = Command::new(&bin).output().expect("run the faux oino");
            let stdout = String::from_utf8_lossy(&check.stdout);
            assert!(
                check.status.success()
                    && serde_json::from_str::<Value>(stdout.trim()).is_ok()
                    && stdout.contains(MARKER),
                "the faux oino is not a working script: status {:?}, stdout {stdout:?}, \
                 stderr {:?}",
                check.status,
                String::from_utf8_lossy(&check.stderr)
            );
            command.env("OINO_CRON_BIN", &bin);
        }
        let child = command.spawn().expect("spawn unipi-host serve");
        Self {
            _dir: dir,
            paths,
            child,
            log: log_path,
            agent_dir,
            work,
        }
    }

    fn log_tail(&self) -> String {
        let text = std::fs::read_to_string(&self.log).unwrap_or_default();
        let lines: Vec<&str> = text.lines().collect();
        lines[lines.len().saturating_sub(30)..].join("\n")
    }

    /// Waits until the local listener answers a `host_info`.
    async fn wait_ready(&mut self) -> Value {
        let deadline = Instant::now() + Duration::from_secs(120);
        loop {
            if let Some(status) = self.child.try_wait().expect("try_wait") {
                panic!(
                    "the host exited during startup ({status}):\n{}",
                    self.log_tail()
                );
            }
            if let Ok(mut stream) = ipc::connect(&self.paths.local_addr()).await {
                if stream
                    .write_all(&hello(json!({"op": "host_info"})))
                    .await
                    .is_ok()
                {
                    if let Ok(Some((0, frame))) =
                        tokio::time::timeout(Duration::from_secs(5), next_frame(&mut stream)).await
                    {
                        if let Some(reply) = parse_control(&frame) {
                            if let Some(data) = reply.get("data") {
                                return data.clone();
                            }
                        }
                    }
                }
            }
            assert!(
                Instant::now() < deadline,
                "the host did not become ready:\n{}",
                self.log_tail()
            );
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    /// One op, one connection (the local listener's one-op-per-connection rule).
    async fn op(&self, req: Value) -> Value {
        let mut stream = ipc::connect(&self.paths.local_addr())
            .await
            .expect("connect app.sock");
        stream.write_all(&hello(req)).await.unwrap();
        let (kind, frame) = tokio::time::timeout(Duration::from_secs(60), next_frame(&mut stream))
            .await
            .expect("timed out")
            .expect("a frame");
        assert_eq!(kind, 0, "a one-shot op answers one control frame");
        parse_control(&frame).unwrap_or_else(|| panic!("not a control frame"))
    }

    /// One op that must answer `ok`, returning its `data`.
    async fn op_ok(&self, req: Value) -> Value {
        let reply = self.op(req).await;
        assert_eq!(reply["type"], "ok", "op failed: {reply}");
        reply.get("data").cloned().unwrap_or(Value::Null)
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn hello(req: Value) -> Vec<u8> {
    encode(
        FrameKind::Control,
        &serde_json::to_vec(&json!({"type": "hello", "proto": PROTO_VERSION, "req": req})).unwrap(),
    )
}

fn parse_control(frame: &[u8]) -> Option<Value> {
    serde_json::from_slice(frame).ok()
}

/// Reads one frame: its kind (0 control, 1 output, 2 input) and payload; `None` at EOF.
async fn next_frame<R: tokio::io::AsyncRead + Unpin>(stream: &mut R) -> Option<(u8, Vec<u8>)> {
    let mut head = [0u8; 5];
    stream.read_exact(&mut head).await.ok()?;
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    let mut payload = vec![0u8; len];
    stream.read_exact(&mut payload).await.ok()?;
    Some((head[0], payload))
}

/// Serves the `openai-completions` SSE stream the sandboxed provider talks to: a role
/// chunk, [`CHUNKS`] content chunks [`CHUNK_DELAY`] apart, a finish chunk, usage and
/// `[DONE]`. Every accepted request is counted.
fn serve_provider(
    listener: TcpListener,
    requests: Arc<AtomicUsize>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        loop {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };
            let requests = Arc::clone(&requests);
            tokio::spawn(async move {
                let mut buffer = Vec::new();
                let mut chunk = [0u8; 8192];
                loop {
                    let Ok(read) = socket.read(&mut chunk).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    buffer.extend_from_slice(&chunk[..read]);
                    let Some(header_end) = buffer.windows(4).position(|w| w == b"\r\n\r\n") else {
                        continue;
                    };
                    let head = String::from_utf8_lossy(&buffer[..header_end]).to_ascii_lowercase();
                    let length = head
                        .lines()
                        .find_map(|line| line.strip_prefix("content-length:"))
                        .and_then(|value| value.trim().parse::<usize>().ok())
                        .unwrap_or(0);
                    if buffer.len() >= header_end + 4 + length {
                        break;
                    }
                }
                requests.fetch_add(1, Ordering::SeqCst);
                let head = b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\
                             cache-control: no-cache\r\nconnection: close\r\n\r\n";
                if socket.write_all(head).await.is_err() {
                    return;
                }
                let event = |delta: Value, finish: Option<&str>| {
                    let chunk = json!({
                        "id": "h10-1", "object": "chat.completion.chunk", "created": 1,
                        "model": "faux-1",
                        "choices": [{"index": 0, "delta": delta, "finish_reason": finish}]
                    });
                    format!("data: {chunk}\n\n")
                };
                if socket
                    .write_all(event(json!({"role": "assistant", "content": ""}), None).as_bytes())
                    .await
                    .is_err()
                {
                    return;
                }
                let piece = "x".repeat(REPLY_CHARS / CHUNKS);
                for _ in 0..CHUNKS {
                    let frame = event(json!({"content": piece}), None);
                    if socket.write_all(frame.as_bytes()).await.is_err() {
                        return;
                    }
                    tokio::time::sleep(CHUNK_DELAY).await;
                }
                let usage = json!({
                    "id": "h10-1", "object": "chat.completion.chunk", "created": 1,
                    "model": "faux-1", "choices": [],
                    "usage": {"prompt_tokens": 10, "completion_tokens": 1, "total_tokens": 11}
                });
                let mut tail = event(json!({}), Some("stop"));
                tail.push_str(&format!("data: {usage}\n\ndata: [DONE]\n\n"));
                let _ = socket.write_all(tail.as_bytes()).await;
            });
        }
    })
}

/// One session bi-stream: the snapshot/command-reply frames and the `events` batches.
struct SessionStream {
    stream: ipc::Stream,
}

impl SessionStream {
    /// Opens a session stream (`session_new`/`session_open`/`session_fork`) and returns the
    /// snapshot data from its first `ok` reply.
    async fn open(host: &Host, req: Value) -> (Self, Value) {
        let mut stream = ipc::connect(&host.paths.local_addr())
            .await
            .expect("connect app.sock");
        stream.write_all(&hello(req)).await.unwrap();
        let (kind, payload) =
            tokio::time::timeout(Duration::from_secs(120), next_frame(&mut stream))
                .await
                .expect("snapshot timed out")
                .expect("a snapshot frame");
        assert_eq!(kind, 0);
        let reply = parse_control(&payload).expect("snapshot control");
        assert_eq!(reply["type"], "ok", "session open failed: {reply}");
        let snapshot = reply["data"].clone();
        (Self { stream }, snapshot)
    }

    async fn send(&mut self, value: Value) {
        self.stream
            .write_all(&encode(
                FrameKind::Control,
                &serde_json::to_vec(&value).unwrap(),
            ))
            .await
            .unwrap();
    }

    /// The next control frame (kind 0), or `None` at EOF / timeout.
    async fn control(&mut self) -> Option<Value> {
        loop {
            let (kind, payload) =
                tokio::time::timeout(Duration::from_secs(120), next_frame(&mut self.stream))
                    .await
                    .ok()??;
            if kind == 0 {
                return parse_control(&payload);
            }
        }
    }

    /// Waits for the correlated `ok`/`err` reply to `id`, skipping events and other
    /// replies (§7.3: events a command caused precede its reply on the same stream).
    async fn wait_reply(&mut self, id: &str) -> Value {
        loop {
            let frame = self
                .control()
                .await
                .expect("the session stream closed while waiting for a reply");
            if matches!(frame["type"].as_str(), Some("ok" | "err")) && frame["id"] == id {
                return frame;
            }
        }
    }

    async fn close(self) {
        let mut stream = self.stream;
        let _ = stream.shutdown().await;
    }
}

/// What one streamed reply measured.
struct StreamReport {
    frames: usize,
    events: usize,
    delta_events: usize,
    delta_frames: usize,
    stream: Duration,
    total: Duration,
    first_seq: u64,
    last_seq: u64,
    assistant_chars: usize,
    /// The live `message_end` was clipped to the event budget (fix-frames, `bound_event`).
    assistant_truncated: bool,
    replies: Vec<Value>,
}

/// Text length of an assistant message's content blocks.
fn assistant_text_len(message: &Value) -> usize {
    message["content"]
        .as_array()
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|block| block["text"].as_str())
                .map(str::len)
                .sum()
        })
        .unwrap_or(0)
}

/// Reads the run's events until `agent_settled`, asserting strictly increasing seqs.
async fn read_run(stream: &mut SessionStream, session_id: &str, base: u64) -> StreamReport {
    let started = Instant::now();
    let mut first_delta = None;
    let mut last_delta = Instant::now();
    let mut report = StreamReport {
        frames: 0,
        events: 0,
        delta_events: 0,
        delta_frames: 0,
        stream: Duration::ZERO,
        total: Duration::ZERO,
        first_seq: u64::MAX,
        last_seq: base,
        assistant_chars: 0,
        assistant_truncated: false,
        replies: Vec::new(),
    };
    loop {
        let Some(frame) = stream.control().await else {
            panic!("the session stream closed before agent_settled");
        };
        match frame["type"].as_str() {
            Some("events") => {
                report.frames += 1;
                let items = frame["items"].as_array().expect("events items");
                let mut delta = false;
                for item in items {
                    let seq = item["seq"].as_u64().expect("seq");
                    assert_eq!(item["session"], session_id, "event session id");
                    assert!(
                        seq > report.last_seq,
                        "seq must increase strictly: {seq} after {}",
                        report.last_seq
                    );
                    if report.first_seq == u64::MAX {
                        report.first_seq = seq;
                    }
                    report.last_seq = seq;
                    report.events += 1;
                    let event = &item["event"];
                    match event["type"].as_str() {
                        Some("message_update") => {
                            report.delta_events += 1;
                            delta = true;
                            if first_delta.is_none() {
                                first_delta = Some(Instant::now());
                            }
                            last_delta = Instant::now();
                        }
                        Some("message_end") if event["message"]["role"] == "assistant" => {
                            // The final message carries the reply (`content` is an array
                            // of blocks), clipped and flagged `truncated` when the event
                            // is over the live event budget.
                            report.assistant_chars = assistant_text_len(&event["message"]);
                            report.assistant_truncated =
                                event["truncated"] == true || event["message"]["truncated"] == true;
                        }
                        Some("agent_settled") => {
                            report.total = started.elapsed();
                            report.stream = match first_delta {
                                Some(first) => last_delta.duration_since(first),
                                None => Duration::ZERO,
                            };
                            return report;
                        }
                        _ => {}
                    }
                }
                if delta {
                    report.delta_frames += 1;
                }
            }
            Some("ok") | Some("err") => report.replies.push(frame),
            _ => {}
        }
    }
}

/// Reads kind-1 terminal output until `MARKER` shows up.
#[cfg(unix)]
async fn read_terminal(stream: &mut ipc::Stream) -> String {
    let mut output = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        assert!(Instant::now() < deadline, "terminal output timed out");
        let Some((kind, payload)) = next_frame(stream).await else {
            panic!(
                "the terminal stream closed early: {:?}",
                String::from_utf8_lossy(&output)
            );
        };
        match kind {
            1 => {
                output.extend_from_slice(&payload);
                if String::from_utf8_lossy(&output).contains(MARKER) {
                    return String::from_utf8_lossy(&output).into_owned();
                }
            }
            0 => {}
            _ => {}
        }
    }
}

#[cfg(target_os = "linux")]
fn rss_bytes(pid: u32) -> Option<u64> {
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let line = status.lines().find(|line| line.starts_with("VmRSS:"))?;
    let kb = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(kb * 1024)
}

#[cfg(not(target_os = "linux"))]
fn rss_bytes(_pid: u32) -> Option<u64> {
    None
}

fn write_extra_sessions(session_file: &Path, count: usize, cwd: &Path) {
    let dir = session_file.parent().expect("session dir");
    for index in 0..count {
        let id = format!("01a18c5b-0000-7000-8000-{index:012x}");
        let header = json!({
            "type": "session", "version": 3, "id": id,
            "timestamp": "2026-10-30T10:30:00.000Z",
            "cwd": cwd.to_string_lossy(),
        });
        std::fs::write(
            dir.join(format!("2026-10-30T10-30-{index:02}-000Z_{id}.jsonl")),
            format!("{header}\n"),
        )
        .unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "WP-H10: spawns unipi-host serve; needs extensions/dist (cargo xtask wasm)"]
#[allow(
    clippy::too_many_lines,
    reason = "one end-to-end scenario; the sections are the acceptance steps"
)]
async fn headless_host_client_e2e_and_performance_gates() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let sse_port = listener.local_addr().unwrap().port();
    let requests = Arc::new(AtomicUsize::new(0));
    let provider = serve_provider(listener, Arc::clone(&requests));

    let spawn = Instant::now();
    let mut host = Host::start(sse_port);
    let info = host.wait_ready().await;
    let startup = spawn.elapsed();
    let work = host.work.to_string_lossy().into_owned();
    println!("H10 startup: {startup:?}");

    // ---------------------------------------------------------------- local admin
    // This PC's app is `Peer::Local`: the local-admin ops need no pairing.
    assert_eq!(info["pid"], json!(host.child.id()));
    assert_eq!(info["mode"], "manual");
    assert_eq!(info["proto"], PROTO_VERSION);
    let devices = host.op_ok(json!({"op": "devices_list"})).await;
    assert!(devices["devices"].as_array().unwrap().is_empty());
    let pair = host
        .op_ok(json!({"op": "pair_begin", "ttl_secs": 60}))
        .await;
    assert!(
        pair["link"].as_str().unwrap().starts_with("unipi://pair"),
        "pair link: {pair}"
    );
    let cancelled = host
        .op_ok(json!({"op": "pair_status", "code": pair["code"], "cancel": true}))
        .await;
    assert_eq!(cancelled["state"], "expired", "{cancelled}");
    assert_eq!(cancelled["reason"], "cancelled", "{cancelled}");

    // ---------------------------------------------------------------- empty list
    let list = host.op_ok(json!({"op": "sessions_list"})).await;
    assert!(list.as_array().unwrap().is_empty(), "fresh host: {list}");

    // ---------------------------------------------------------------- 100k-token stream
    let (mut session, snapshot) =
        SessionStream::open(&host, json!({"op": "session_new", "cwd": work})).await;
    assert_eq!(snapshot["reset"], true);
    assert!(snapshot["state"].is_object(), "snapshot state: {snapshot}");
    assert!(
        snapshot["entries"].is_array(),
        "snapshot entries: {snapshot}"
    );
    let session_id = snapshot["session_id"].as_str().unwrap().to_owned();
    let base = snapshot["seq"].as_u64().unwrap();
    session
        .send(json!({
            "type": "session_cmd", "id": "model",
            "cmd": {"type": "set_model", "provider": "faux", "modelId": "faux-1"}
        }))
        .await;
    let reply = session.wait_reply("model").await;
    assert_eq!(reply["type"], "ok", "{reply}");
    assert_eq!(reply["data"]["id"], "faux-1");

    session
        .send(json!({
            "type": "session_cmd", "id": "prompt",
            "cmd": {"type": "prompt", "message": "stream the long reply"}
        }))
        .await;
    let run = read_run(&mut session, &session_id, base).await;
    assert!(
        run.replies
            .iter()
            .any(|reply| reply["id"] == "prompt" && reply["type"] == "ok"),
        "no correlated prompt reply: {:?}",
        run.replies
    );
    if run.assistant_truncated {
        // A live event is bounded (fix-frames P0: no oversize frame); the app refetches
        // the whole entry by id on expand, so that path must return all of it.
        assert!(
            run.assistant_chars > 0 && run.assistant_chars < REPLY_CHARS,
            "a truncated message_end carries a clipped head: {}",
            run.assistant_chars
        );
        session
            .send(json!({
                "type": "session_cmd", "id": "page",
                "cmd": {"type": "get_entries", "limit": 4}
            }))
            .await;
        let page = session.wait_reply("page").await;
        assert_eq!(page["type"], "ok", "{page}");
        let entry_id = page["data"]["entries"]
            .as_array()
            .and_then(|entries| {
                entries
                    .iter()
                    .rev()
                    .find(|entry| entry["message"]["role"] == "assistant")
            })
            .and_then(|entry| entry["id"].as_str())
            .unwrap_or_else(|| panic!("no assistant entry in the page: {page}"))
            .to_owned();
        session
            .send(json!({
                "type": "session_cmd", "id": "full",
                "cmd": {"type": "get_entries", "ids": [entry_id]}
            }))
            .await;
        let full = session.wait_reply("full").await;
        assert_eq!(full["type"], "ok", "{full}");
        assert_eq!(
            assistant_text_len(&full["data"]["entries"][0]["message"]),
            REPLY_CHARS,
            "the whole 100k-token reply is served by id"
        );
    } else {
        assert_eq!(
            run.assistant_chars, REPLY_CHARS,
            "the whole 100k-token reply arrived"
        );
    }
    assert!(
        run.delta_events < CHUNKS,
        "{} deltas must coalesce, got {} message_update events",
        CHUNKS,
        run.delta_events
    );
    let stream_ms = u64::try_from(run.stream.as_millis())
        .unwrap_or(u64::MAX)
        .max(1);
    let window_limit = stream_ms.div_ceil(16) + 4;
    assert!(
        u64::try_from(run.delta_frames).unwrap_or(u64::MAX) <= window_limit,
        "{} delta frames for a {stream_ms} ms stream (limit {window_limit}; §7.4 is one \
         frame per 16 ms flush tick)",
        run.delta_frames
    );
    assert!(
        run.first_seq > base,
        "the first event follows the snapshot ({} > {base})",
        run.first_seq
    );
    assert!(
        requests.load(Ordering::SeqCst) >= 1,
        "the provider was not called"
    );
    println!(
        "H10 stream: {} chars in {CHUNKS} SSE chunks -> {} frames / {} events \
         ({} delta events in {} frames) in {:?} total {:?}; seq {}..={}; delta-frame limit {}",
        run.assistant_chars,
        run.frames,
        run.events,
        run.delta_events,
        run.delta_frames,
        run.stream,
        run.total,
        run.first_seq,
        run.last_seq,
        window_limit
    );
    let last_seq = run.last_seq;
    session.close().await;

    // ---------------------------------------------------------------- card + resume
    // `agent_settled` ends the stream; the card follows it asynchronously (the run
    // supervisor polls the backend once a second).
    let card_deadline = Instant::now() + Duration::from_secs(15);
    let card = loop {
        let cards = host.op_ok(json!({"op": "sessions_list"})).await;
        let found = cards
            .as_array()
            .unwrap()
            .iter()
            .find(|card| card["session_id"] == session_id.as_str())
            .cloned();
        if let Some(card) = found {
            if card["run_state"] == "idle" && card["message_count"].as_u64().unwrap_or(0) >= 2 {
                break card;
            }
        }
        assert!(
            Instant::now() < card_deadline,
            "the card never went idle: {cards}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(card["run_state"], "idle", "{card}");
    let session_file = PathBuf::from(card["file"].as_str().expect("session file"));

    let (mut resumed, patch) = SessionStream::open(
        &host,
        json!({"op": "session_open", "session": session_id, "since_seq": last_seq}),
    )
    .await;
    assert_eq!(patch["reset"], false, "since_seq resumes: {patch}");
    assert_eq!(patch["session_id"], session_id.as_str());
    let replayed = patch["replay"].as_array().cloned().unwrap_or_default();
    assert!(
        replayed
            .iter()
            .all(|event| event["seq"].as_u64().unwrap() > last_seq),
        "replay is after the client's seq: {replayed:?}"
    );
    assert!(patch["seq"].as_u64().unwrap() >= last_seq);
    resumed
        .send(json!({
            "type": "session_cmd", "id": "state",
            "cmd": {"type": "get_state"}
        }))
        .await;
    let state_reply = resumed.wait_reply("state").await;
    assert_eq!(state_reply["type"], "ok", "{state_reply}");
    assert_eq!(state_reply["data"]["sessionId"], session_id.as_str());
    resumed.close().await;
    println!(
        "H10 resume: reset={} replay={} seq={}",
        patch["reset"],
        replayed.len(),
        patch["seq"]
    );

    // ---------------------------------------------------------------- idle RSS
    // §7.5/spec-core §3.2: ten idle live sessions must not multiply the process (one
    // shared engine; a session is small until it runs). The baseline is taken after the
    // first run, so the one-time engine/provider warm-up is not counted as per-session.
    let pid = host.child.id();
    let rss_base = rss_bytes(pid);
    for _ in 0..10 {
        host.op_ok(json!({"op": "session_new", "cwd": work})).await;
    }
    let rss_ten = rss_bytes(pid);

    // ---------------------------------------------------------------- terminal
    #[cfg(unix)]
    {
        let mut terminal = ipc::connect(&host.paths.local_addr())
            .await
            .expect("connect the terminal stream");
        terminal
            .write_all(&hello(json!({
                "op": "term_open", "cwd": work, "cols": 80, "rows": 24
            })))
            .await
            .unwrap();
        let (kind, payload) =
            tokio::time::timeout(Duration::from_secs(30), next_frame(&mut terminal))
                .await
                .expect("term_open timed out")
                .expect("term_open reply");
        assert_eq!(kind, 0);
        let reply = parse_control(&payload).expect("term_open control");
        assert_eq!(reply["type"], "ok", "{reply}");
        let term_id = reply["data"]["term_id"]
            .as_str()
            .expect("term id")
            .to_owned();
        terminal
            .write_all(&encode(
                FrameKind::Input,
                format!("echo {MARKER}\n").as_bytes(),
            ))
            .await
            .unwrap();
        let output = read_terminal(&mut terminal).await;
        assert!(output.contains(MARKER), "terminal output: {output:?}");
        terminal.shutdown().await.ok();
        host.op_ok(json!({"op": "term_kill", "term_id": term_id}))
            .await;
        println!("H10 terminal: open -> {MARKER} over a host PTY");
    }

    // ---------------------------------------------------------------- cron
    #[cfg(unix)]
    let cron_elapsed = {
        let started = Instant::now();
        let job = host
            .op_ok(json!({
                "op": "cron_add", "name": "h10-integration", "prompt": "say the marker",
                "schedule": "at:2000-01-01T00:00:00Z", "tz": "UTC", "cwd": work
            }))
            .await;
        let id = job["id"].as_str().expect("job id").to_owned();
        let run = host.op_ok(json!({"op": "cron_run", "id": id})).await;
        assert_eq!(run["id"], id.as_str(), "{run}");
        let mut finished = false;
        let mut last_seen = Value::Null;
        for _ in 0..200 {
            let runs = host.op_ok(json!({"op": "cron_runs", "id": id})).await;
            if let Some(first) = runs["runs"].as_array().and_then(|runs| runs.first()) {
                last_seen = first.clone();
                if first["status"] == "ok" {
                    assert!(
                        first["summary"]
                            .as_str()
                            .unwrap_or_default()
                            .contains(MARKER),
                        "cron summary: {first}"
                    );
                    finished = true;
                    break;
                }
                assert_ne!(first["status"], "error", "cron run failed: {first}");
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        if !finished {
            let mut logs = String::new();
            let root = host.agent_dir.join("cron");
            let mut stack = vec![root];
            while let Some(dir) = stack.pop() {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                for entry in entries.filter_map(Result::ok) {
                    let path = entry.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.extension().is_some_and(|ext| ext == "log") {
                        logs.push_str(&format!(
                            "--- {}\n{}\n",
                            path.display(),
                            std::fs::read_to_string(&path).unwrap_or_default()
                        ));
                    }
                }
            }
            panic!(
                "the faux cron job never finished; last run {last_seen}; run logs:\n{logs}\nhost log:\n{}",
                host.log_tail()
            );
        }
        let jobs = host.op_ok(json!({"op": "cron_list"})).await;
        assert!(
            jobs["jobs"]
                .as_array()
                .unwrap()
                .iter()
                .any(|job| job["id"] == id.as_str()),
            "cron_list: {jobs}"
        );
        started.elapsed()
    };
    #[cfg(unix)]
    println!("H10 cron: add + run_now -> ok in {cron_elapsed:?}");

    // ---------------------------------------------------------------- push
    let push = host.op_ok(json!({"op": "push_list"})).await;
    assert_eq!(push["enabled"], true, "push config: {push}");
    assert_eq!(push["host_name"], info["host_name"]);
    assert!(push["pushes"].as_array().unwrap().is_empty(), "{push}");
    println!(
        "H10 push_list: enabled={} host={}",
        push["enabled"], push["host_name"]
    );

    // ---------------------------------------------------------------- 50-session list
    // Header-only records beside the session the stream created: the next `sessions_list`
    // has to read 50 files (the §7.5 "headers only, never loading runtimes" path).
    write_extra_sessions(&session_file, 49, &host.work);
    let list_started = Instant::now();
    let fifty = host.op_ok(json!({"op": "sessions_list"})).await;
    let list_elapsed = list_started.elapsed();
    let count = fifty.as_array().map_or(0, Vec::len);
    assert!(count >= 50, "expected 50 cards, got {count}");
    assert!(
        list_elapsed < Duration::from_millis(150),
        "50-session cold list took {list_elapsed:?} (limit 150 ms)"
    );
    println!("H10 sessions_list: {count} cards in {list_elapsed:?} (limit 150 ms)");

    // ---------------------------------------------------------------- RSS record + gate
    if let (Some(before), Some(after)) = (rss_base, rss_ten) {
        let delta = after.saturating_sub(before);
        // spec-core §3.2 names no byte figure; the gate here is "ten idle live sessions do
        // not duplicate the process": ≤ 32 MB total (the measured value is in host-perf.md).
        assert!(
            delta < 32 * 1024 * 1024,
            "ten idle sessions added {} MB RSS",
            delta / (1024 * 1024)
        );
        println!(
            "H10 RSS: baseline {} MB, 10 idle sessions {} MB (delta {} MB, {:.2} MB/session)",
            before / (1024 * 1024),
            after / (1024 * 1024),
            delta / (1024 * 1024),
            delta as f64 / 10.0 / (1024.0 * 1024.0)
        );
    } else {
        println!("H10 RSS: unsupported on this OS");
    }

    provider.abort();
}
