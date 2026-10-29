//! Board relay round trips through the host handler (DESIGN.md §3.9):
//! a fake daemon (always) and the real `unipi-kanboard` binary (when it is
//! installed) behind `board_http` / `board_sse` streams.

use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use unipi_host::{
    board::{discovery::BinaryLookup, Board, BoardConfig},
    data_dir::HostPaths,
    devices::DeviceStore,
    protocol::{read_frame, Handler},
};
use unipi_proto::{
    frame::{encode, FrameKind, MAX_FRAME},
    AppErrorCode, BoardHttpData, Control, PROTO_VERSION,
};

struct Host {
    handler: Handler,
    phone: iroh::EndpointId,
    _dir: tempfile::TempDir,
}

fn host(board: Board, paired: bool) -> Host {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let phone = iroh::SecretKey::generate().public();
    if paired {
        DeviceStore::new(paths.devices())
            .add(&phone, "Test phone".into())
            .unwrap();
    }
    Host {
        handler: Handler {
            cron: unipi_host::cron::Scheduler::new(paths.clone()),
            paths,
            host_id: iroh::SecretKey::generate().public(),
            host_name: "Test".into(),
            lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            protected: std::sync::Arc::new(vec!["default".into()]),
            board,
        },
        phone,
        _dir: dir,
    }
}

fn config(home: &Path, lookup: BinaryLookup) -> BoardConfig {
    BoardConfig {
        home: Some(home.to_path_buf()),
        lookup,
        start_timeout: Duration::from_secs(10),
        serve_args: Vec::new(),
        serve_env: Vec::new(),
    }
}

fn hello(req: serde_json::Value) -> Vec<u8> {
    encode(
        FrameKind::Control,
        &serde_json::to_vec(&serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req}))
            .unwrap(),
    )
}

/// The whole reply of one `board_http` stream.
#[derive(Debug)]
struct HttpReply {
    first: Control,
    body: Vec<u8>,
    last: Option<Control>,
}

impl HttpReply {
    fn data(&self) -> BoardHttpData {
        match &self.first {
            Control::Ok { data } => serde_json::from_value(data.clone().unwrap()).unwrap(),
            other => panic!("expected ok, got {other:?}"),
        }
    }
    fn error_code(&self) -> AppErrorCode {
        match &self.first {
            Control::Err { error } => error.code,
            other => panic!("expected err, got {other:?}"),
        }
    }
    fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap()
    }
    fn complete(&self) -> bool {
        matches!(self.last, Some(Control::Exit { code: Some(0) }))
    }
}

async fn board_http(
    host: &Host,
    method: &str,
    path: &str,
    content_type: Option<&str>,
    body: &[u8],
) -> HttpReply {
    let mut req = serde_json::json!({"op":"board_http","method":method,"path":path});
    if let Some(content_type) = content_type {
        req["content_type"] = content_type.into();
    }
    if !body.is_empty() {
        req["body_len"] = body.len().into();
    }
    let mut bytes = hello(req);
    for piece in body.chunks(MAX_FRAME) {
        bytes.extend(encode(FrameKind::Input, piece));
    }
    raw_http(host, bytes).await
}

async fn raw_http(host: &Host, bytes: Vec<u8>) -> HttpReply {
    let (client, server) = tokio::io::duplex(1 << 20);
    let (read, write) = tokio::io::split(server);
    let handler = host.handler.clone();
    let phone = host.phone;
    let task = tokio::spawn(async move { handler.handle(read, write, phone).await });
    let (mut read, mut write) = tokio::io::split(client);
    let writer = tokio::spawn(async move {
        let _ = write.write_all(&bytes).await;
        write
    });
    let (kind, first) = read_frame(&mut read).await.unwrap();
    assert_eq!(kind, 0);
    let first: Control = serde_json::from_slice(&first).unwrap();
    let mut body = Vec::new();
    let mut last = None;
    if matches!(first, Control::Ok { .. }) {
        while let Ok((kind, payload)) = read_frame(&mut read).await {
            match kind {
                1 => body.extend_from_slice(&payload),
                0 => {
                    last = Some(serde_json::from_slice(&payload).unwrap());
                    break;
                }
                other => panic!("unexpected frame kind {other}"),
            }
        }
    }
    drop(writer.await.unwrap());
    task.await.unwrap().unwrap();
    HttpReply { first, body, last }
}

/// An open `board_sse` stream.
struct Sse {
    read: tokio::io::ReadHalf<tokio::io::DuplexStream>,
    write: Option<tokio::io::WriteHalf<tokio::io::DuplexStream>>,
    task: tokio::task::JoinHandle<anyhow::Result<()>>,
}

impl Sse {
    async fn open(host: &Host, project: &str) -> (Control, Sse) {
        let (client, server) = tokio::io::duplex(1 << 16);
        let (read, write) = tokio::io::split(server);
        let handler = host.handler.clone();
        let phone = host.phone;
        let task = tokio::spawn(async move { handler.handle(read, write, phone).await });
        let (mut read, mut write) = tokio::io::split(client);
        write
            .write_all(&hello(
                serde_json::json!({"op":"board_sse","project":project}),
            ))
            .await
            .unwrap();
        let (_, first) = read_frame(&mut read).await.unwrap();
        let first = serde_json::from_slice(&first).unwrap();
        (
            first,
            Sse {
                read,
                write: Some(write),
                task,
            },
        )
    }

    async fn next(&mut self) -> Option<Control> {
        let (kind, payload) =
            tokio::time::timeout(Duration::from_secs(10), read_frame(&mut self.read))
                .await
                .expect("sse frame timed out")
                .ok()?;
        assert_eq!(kind, 0);
        Some(serde_json::from_slice(&payload).unwrap())
    }

    async fn close(mut self) {
        if let Some(mut write) = self.write.take() {
            let _ = write.shutdown().await;
        }
        tokio::time::timeout(Duration::from_secs(5), self.task)
            .await
            .expect("host did not end the sse stream after the client closed")
            .unwrap()
            .unwrap();
    }
}

fn event(control: Option<Control>) -> (Option<String>, String) {
    match control {
        Some(Control::BoardEvent { event, data }) => (event, data),
        other => panic!("expected board_event, got {other:?}"),
    }
}

// ─── fake daemon ───────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body_len: usize,
}

async fn read_request(stream: &mut tokio::net::TcpStream) -> Option<(Seen, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head_end = loop {
        if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break pos;
        }
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).to_string();
    let mut lines = head.split("\r\n");
    let mut start = lines.next()?.split(' ');
    let method = start.next()?.to_string();
    let path = start.next()?.to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|line| line.split_once(':'))
        .map(|(k, v)| (k.trim().to_ascii_lowercase(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k == "content-length")
        .map(|(_, v)| v.parse().unwrap())
        .unwrap_or(0);
    let mut body = buf[head_end + 4..].to_vec();
    while body.len() < len {
        let n = stream.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Some((
        Seen {
            method,
            path,
            headers,
            body_len: body.len(),
        },
        body,
    ))
}

fn checksum(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325u64, |hash, byte| {
        (hash ^ *byte as u64).wrapping_mul(0x100000001b3)
    })
}

fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|i| (i * 31 % 251) as u8).collect()
}

async fn respond(stream: &mut tokio::net::TcpStream, status: &str, ctype: &str, body: &[u8]) {
    let head = format!(
        "HTTP/1.1 {status}\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        body.len()
    );
    let _ = stream.write_all(head.as_bytes()).await;
    let _ = stream.write_all(body).await;
    let _ = stream.shutdown().await;
}

/// Starts a fake kanboard daemon and writes its `daemon.json` into `home`.
async fn fake_daemon(
    home: &Path,
    token: Option<&str>,
) -> tokio::sync::mpsc::UnboundedReceiver<Seen> {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut info = serde_json::json!({
        "pid": std::process::id(), "port": port, "version": "9.9.9-fake",
        "startedAt": "2026-10-07T00:00:00Z", "host": "127.0.0.1"
    });
    if let Some(token) = token {
        info["token"] = token.into();
    }
    std::fs::create_dir_all(home).unwrap();
    std::fs::write(home.join("daemon.json"), info.to_string()).unwrap();
    let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
    tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let tx = tx.clone();
            tokio::spawn(async move {
                let Some((seen, body)) = read_request(&mut stream).await else {
                    return;
                };
                let _ = tx.send(seen.clone());
                match (seen.method.as_str(), seen.path.as_str()) {
                    ("GET", "/api/health") => {
                        respond(
                            &mut stream,
                            "200 OK",
                            "application/json",
                            br#"{"ok":true,"version":"9.9.9-fake","pid":1}"#,
                        )
                        .await
                    }
                    ("GET", "/api/projects") => {
                        respond(
                            &mut stream,
                            "200 OK",
                            "application/json",
                            br#"[{"slug":"p-1"}]"#,
                        )
                        .await
                    }
                    ("POST", "/api/tasks/p-1/P-1/move") => {
                        let echoed: serde_json::Value = serde_json::from_slice(&body).unwrap();
                        let reply = serde_json::json!({"error":"needs a comment","needsComment":true,"needsAgent":false,"echo":echoed});
                        respond(
                            &mut stream,
                            "409 Conflict",
                            "application/json",
                            reply.to_string().as_bytes(),
                        )
                        .await
                    }
                    ("POST", path) if path.starts_with("/api/tasks/p-1/P-1/attachments?name=") => {
                        let reply = serde_json::json!({"len": body.len(), "sum": checksum(&body).to_string()});
                        respond(
                            &mut stream,
                            "200 OK",
                            "application/json",
                            reply.to_string().as_bytes(),
                        )
                        .await
                    }
                    ("GET", "/api/files/p-1/P-1/big.bin") => {
                        respond(
                            &mut stream,
                            "200 OK",
                            "application/octet-stream",
                            &pattern(3 * 1024 * 1024 + 17),
                        )
                        .await
                    }
                    ("GET", "/events?project=p-1") => {
                        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncache-control: no-cache\r\nconnection: close\r\n\r\n";
                        let _ = stream.write_all(head.as_bytes()).await;
                        for piece in [
                            &b"event: revision\r\ndata: 1\r\n\r\n"[..],
                            b": keep-alive\n\n",
                            b"data: line one\ndata: line",
                            b" two\n\nevent: revision\ndata: 2\r",
                            b"\n\r\n",
                        ] {
                            let _ = stream.write_all(piece).await;
                            let _ = stream.flush().await;
                            tokio::time::sleep(Duration::from_millis(20)).await;
                        }
                        let _ = stream.shutdown().await;
                    }
                    ("GET", "/events?project=forever") => {
                        let head = "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\nconnection: close\r\n\r\nevent: revision\ndata: 5\n\n";
                        let _ = stream.write_all(head.as_bytes()).await;
                        // Keep the stream open until the relay hangs up.
                        let mut sink = [0u8; 64];
                        while let Ok(n) = stream.read(&mut sink).await {
                            if n == 0 {
                                break;
                            }
                        }
                    }
                    ("GET", path) if path.starts_with("/events") => {
                        respond(
                            &mut stream,
                            "404 Not Found",
                            "text/plain",
                            b"no such project",
                        )
                        .await
                    }
                    _ => respond(&mut stream, "404 Not Found", "text/plain", b"nope").await,
                }
            });
        }
    });
    rx
}

fn header<'a>(seen: &'a Seen, name: &str) -> Option<&'a str> {
    seen.headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

async fn drain(rx: &mut tokio::sync::mpsc::UnboundedReceiver<Seen>) -> Vec<Seen> {
    tokio::time::sleep(Duration::from_millis(50)).await;
    let mut out = Vec::new();
    while let Ok(seen) = rx.try_recv() {
        out.push(seen);
    }
    out
}

#[tokio::test]
async fn fake_daemon_round_trips() {
    let kb = tempfile::tempdir().unwrap();
    let mut seen = fake_daemon(kb.path(), None).await;
    let host = host(Board::new(config(kb.path(), BinaryLookup::default())), true);

    // GET JSON.
    let reply = board_http(&host, "GET", "/api/projects", None, b"").await;
    assert_eq!(reply.data().status.value(), 200);
    assert_eq!(reply.data().content_type, "application/json");
    assert_eq!(reply.json(), serde_json::json!([{"slug":"p-1"}]));
    assert!(reply.complete());

    // POST JSON; the daemon's 4xx JSON is passed through verbatim.
    let reply = board_http(
        &host,
        "POST",
        "/api/tasks/p-1/P-1/move",
        Some("application/json"),
        br#"{"to":"blocked"}"#,
    )
    .await;
    assert_eq!(reply.data().status.value(), 409);
    let json = reply.json();
    assert_eq!(json["needsComment"], true);
    assert_eq!(json["needsAgent"], false);
    assert_eq!(json["echo"]["to"], "blocked");
    assert!(reply.complete());

    // Octet-stream upload bigger than one frame (multi-frame body).
    let upload = pattern(5 * 1024 * 1024 + 3);
    let reply = board_http(
        &host,
        "POST",
        "/api/tasks/p-1/P-1/attachments?name=shot%201.png",
        Some("application/octet-stream"),
        &upload,
    )
    .await;
    assert_eq!(reply.data().status.value(), 200);
    assert_eq!(reply.json()["len"], upload.len());
    assert_eq!(reply.json()["sum"], checksum(&upload).to_string());

    // Binary download split across many output frames.
    let reply = board_http(&host, "GET", "/api/files/p-1/P-1/big.bin", None, b"").await;
    assert_eq!(reply.data().content_type, "application/octet-stream");
    assert_eq!(reply.body, pattern(3 * 1024 * 1024 + 17));
    assert!(reply.complete());

    let requests = drain(&mut seen).await;
    let upload_seen = requests
        .iter()
        .find(|s| s.path.contains("attachments"))
        .unwrap();
    assert_eq!(upload_seen.body_len, upload.len());
    assert_eq!(
        header(upload_seen, "content-type"),
        Some("application/octet-stream")
    );
    for request in &requests {
        assert_eq!(header(request, "origin"), None, "no Origin header");
        assert_eq!(header(request, "authorization"), None, "loopback: no token");
    }

    // SSE: framing parsed (CRLF, comments, multi-line, split chunks), then
    // the daemon ends the stream → exit{code:null}.
    let (first, mut sse) = Sse::open(&host, "p-1").await;
    assert!(matches!(first, Control::Ok { .. }), "{first:?}");
    assert_eq!(
        event(sse.next().await),
        (Some("revision".into()), "1".into())
    );
    assert_eq!(event(sse.next().await), (None, "line one\nline two".into()));
    assert_eq!(
        event(sse.next().await),
        (Some("revision".into()), "2".into())
    );
    assert!(matches!(
        sse.next().await,
        Some(Control::Exit { code: None })
    ));
    sse.close().await;

    // The client unsubscribing ends a live stream on the host.
    let (first, mut sse) = Sse::open(&host, "forever").await;
    assert!(matches!(first, Control::Ok { .. }));
    assert_eq!(
        event(sse.next().await),
        (Some("revision".into()), "5".into())
    );
    sse.close().await;

    // A refused stream is an E_BOARD_REJECTED error.
    let (first, sse) = Sse::open(&host, "missing").await;
    match first {
        Control::Err { error } => {
            assert_eq!(error.code, AppErrorCode::E_BOARD_REJECTED);
            assert!(error.message.as_str().contains("404"));
        }
        other => panic!("expected err, got {other:?}"),
    }
    sse.close().await;
}

#[tokio::test]
async fn token_is_sent_as_bearer() {
    let kb = tempfile::tempdir().unwrap();
    let mut seen = fake_daemon(kb.path(), Some("s3cret")).await;
    let host = host(Board::new(config(kb.path(), BinaryLookup::default())), true);
    let reply = board_http(&host, "GET", "/api/projects", None, b"").await;
    assert_eq!(reply.data().status.value(), 200);
    let requests = drain(&mut seen).await;
    assert!(!requests.is_empty());
    for request in requests {
        assert_eq!(header(&request, "authorization"), Some("Bearer s3cret"));
        assert_eq!(header(&request, "origin"), None);
    }
}

#[tokio::test]
async fn board_ops_require_a_paired_device_and_allowed_paths() {
    let kb = tempfile::tempdir().unwrap();
    let mut seen = fake_daemon(kb.path(), None).await;
    let unpaired = host(
        Board::new(config(kb.path(), BinaryLookup::default())),
        false,
    );
    let reply = board_http(&unpaired, "GET", "/api/projects", None, b"").await;
    assert_eq!(reply.error_code(), AppErrorCode::E_UNAUTHORIZED);
    let (first, sse) = Sse::open(&unpaired, "p-1").await;
    assert!(matches!(first, Control::Err { error } if error.code == AppErrorCode::E_UNAUTHORIZED));
    sse.close().await;

    let paired = host(Board::new(config(kb.path(), BinaryLookup::default())), true);
    for path in [
        "/events?project=p-1",
        "/index.html",
        "/api/../index.html",
        "/api/%2e%2e/assets/app.js",
    ] {
        let reply = raw_http(
            &paired,
            hello(serde_json::json!({"op":"board_http","method":"GET","path":path})),
        )
        .await;
        assert_eq!(reply.error_code(), AppErrorCode::E_SCHEMA, "{path}");
    }
    // A body longer than body_len is refused.
    let mut bytes = hello(serde_json::json!({
        "op":"board_http","method":"POST","path":"/api/projects","content_type":"application/json","body_len":2
    }));
    bytes.extend(encode(FrameKind::Input, b"{}{}"));
    assert_eq!(
        raw_http(&paired, bytes).await.error_code(),
        AppErrorCode::E_SCHEMA
    );
    assert!(
        drain(&mut seen).await.is_empty(),
        "nothing reached the daemon"
    );
}

#[tokio::test]
async fn missing_binary_reports_not_installed() {
    let kb = tempfile::tempdir().unwrap();
    let empty = tempfile::tempdir().unwrap();
    let lookup = BinaryLookup {
        override_bin: None,
        path_env: Some(empty.path().as_os_str().to_owned()),
        home: Some(empty.path().to_path_buf()),
        platform: "linux-x64".into(),
    };
    let host = host(Board::new(config(kb.path(), lookup)), true);
    let reply = board_http(&host, "GET", "/api/health", None, b"").await;
    match reply.first {
        Control::Err { error } => {
            assert_eq!(error.code, AppErrorCode::E_BOARD_UNAVAILABLE);
            assert_eq!(error.detail.unwrap()["reason"], "not_installed");
            assert!(error
                .hint
                .unwrap()
                .contains("pi install npm:@pi-unipi/unipi"));
        }
        other => panic!("expected err, got {other:?}"),
    }
}

// ─── real unipi-kanboard ──────────────────────────────────────────────────

fn real_binary() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("UNIPI_KANBOARD_TEST_BIN") {
        return Some(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME")?;
    let path = PathBuf::from(home).join(format!(
        ".pi/agent/npm/node_modules/@pi-unipi/kanboard-{}/bin/unipi-kanboard",
        unipi_host::board::discovery::platform_key()
    ));
    path.exists().then_some(path)
}

fn daemon_pid(home: &Path) -> Option<u32> {
    unipi_host::board::discovery::DaemonInfo::read(home).map(|info| info.pid)
}

#[tokio::test]
async fn real_kanboard_round_trips_and_restarts_after_idle() {
    let Some(bin) = real_binary() else {
        eprintln!("skipping: unipi-kanboard binary not installed");
        return;
    };
    let temp = tempfile::tempdir().unwrap();
    let kb_home = temp.path().join("kb");
    let root = temp.path().join("root");
    std::fs::create_dir_all(&root).unwrap();
    let output = std::process::Command::new(&bin)
        .args([
            "project", "add", "--name", "relay", "--prefix", "RL", "--json",
        ])
        .arg("--root")
        .arg(&root)
        .env("UNIPI_KANBOARD_HOME", &kb_home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let project: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    let slug = project["slug"].as_str().unwrap().to_string();

    let mut cfg = config(
        &kb_home,
        BinaryLookup {
            override_bin: Some(bin.clone()),
            ..BinaryLookup::default()
        },
    );
    cfg.serve_args = vec!["--idle-secs".into(), "2".into()];
    cfg.serve_env = vec![("UNIPI_KANBOARD_NO_WATCH".into(), "1".into())];
    let host = host(Board::new(cfg), true);
    assert!(daemon_pid(&kb_home).is_none(), "no daemon yet");

    // First request auto-starts the daemon.
    let reply = board_http(&host, "GET", "/api/health", None, b"").await;
    assert_eq!(reply.data().status.value(), 200, "{reply:?}");
    assert_eq!(reply.json()["ok"], true);
    let first_pid = daemon_pid(&kb_home).expect("daemon.json written");

    // SSE: initial revision, then a write through the relay bumps it.
    let (first, mut sse) = Sse::open(&host, &slug).await;
    assert!(matches!(first, Control::Ok { .. }), "{first:?}");
    let (name, initial) = event(sse.next().await);
    assert_eq!(name.as_deref(), Some("revision"));

    let create = format!("/api/tasks/{slug}/create");
    let reply = board_http(
        &host,
        "POST",
        &create,
        Some("application/json"),
        br#"{"title":"relay task","priority":"high"}"#,
    )
    .await;
    assert_eq!(
        reply.data().status.value(),
        200,
        "{:?}",
        String::from_utf8_lossy(&reply.body)
    );
    let task = reply.json();
    assert_eq!(task["title"], "relay task");
    assert_eq!(task["creator"], "user");
    let id = task["id"].as_str().unwrap().to_string();

    let (name, bumped) = event(sse.next().await);
    assert_eq!(name.as_deref(), Some("revision"));
    assert_ne!(bumped, initial);

    // 4xx JSON passes through verbatim (illegal move).
    let reply = board_http(
        &host,
        "POST",
        &format!("/api/tasks/{slug}/{id}/move"),
        Some("application/json"),
        br#"{"status":"done"}"#,
    )
    .await;
    assert!(
        (400..500).contains(&reply.data().status.value()),
        "{reply:?}"
    );
    assert!(reply.json()["error"].is_string(), "{reply:?}");

    // Octet-stream upload (multi-frame) and binary download.
    let upload = pattern(3 * 1024 * 1024 + 5);
    let reply = board_http(
        &host,
        "POST",
        &format!("/api/tasks/{slug}/{id}/attachments?name=blob.bin"),
        Some("application/octet-stream"),
        &upload,
    )
    .await;
    assert_eq!(
        reply.data().status.value(),
        200,
        "{:?}",
        String::from_utf8_lossy(&reply.body)
    );
    let attachment_json = reply.json();
    let stored = attachment_json["name"]
        .as_str()
        .or_else(|| attachment_json["attachment"]["name"].as_str())
        .unwrap_or("blob.bin")
        .to_string();
    let reply = board_http(
        &host,
        "GET",
        &format!("/api/files/{slug}/{id}/{stored}"),
        None,
        b"",
    )
    .await;
    assert_eq!(reply.data().status.value(), 200, "{attachment_json}");
    assert_eq!(reply.body.len(), upload.len());
    assert!(reply.body == upload);
    sse.close().await;

    // Idle shutdown (2 s, no clients) → the next request restarts it.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    while daemon_pid(&kb_home).is_some() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "daemon did not idle out"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let reply = board_http(&host, "GET", &format!("/api/tasks/{slug}/{id}"), None, b"").await;
    assert_eq!(reply.data().status.value(), 200, "{reply:?}");
    assert_eq!(reply.json()["id"], id.as_str());
    let second_pid = daemon_pid(&kb_home).expect("restarted daemon");
    assert_ne!(first_pid, second_pid);

    // Leave nothing running: stop our private daemon.
    let _ = std::process::Command::new(&bin)
        .arg("stop")
        .env("UNIPI_KANBOARD_HOME", &kb_home)
        .output();
}

// ─── over real iroh streams ───────────────────────────────────────────────

#[tokio::test]
async fn iroh_streams_carry_board_http_and_sse() {
    use iroh::endpoint::{presets, RelayMode};
    let kb = tempfile::tempdir().unwrap();
    let _seen = fake_daemon(kb.path(), None).await;
    let host_ep = iroh::Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .alpns(vec![unipi_host::endpoint::ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let phone = iroh::Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    DeviceStore::new(paths.devices())
        .add(&phone.id(), "Phone".into())
        .unwrap();
    let handler = Handler {
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        paths,
        host_id: host_ep.id(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(Vec::new()),
        board: Board::new(config(kb.path(), BinaryLookup::default())),
    };
    let router = iroh::protocol::Router::builder(host_ep.clone())
        .accept(unipi_host::endpoint::ALPN, handler)
        .spawn();
    let conn = phone
        .connect(host_ep.addr(), unipi_host::endpoint::ALPN)
        .await
        .unwrap();

    // Upload over QUIC: hello + kind-2 body frames, finish, read to the end.
    let upload = pattern(2 * 1024 * 1024 + 9);
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    let mut bytes = hello(serde_json::json!({
        "op":"board_http","method":"POST","path":"/api/tasks/p-1/P-1/attachments?name=x.bin",
        "content_type":"application/octet-stream","body_len":upload.len()
    }));
    for piece in upload.chunks(MAX_FRAME) {
        bytes.extend(encode(FrameKind::Input, piece));
    }
    send.write_all(&bytes).await.unwrap();
    send.finish().unwrap();
    let (_, first) = read_frame(&mut recv).await.unwrap();
    let Control::Ok { data } = serde_json::from_slice(&first).unwrap() else {
        panic!("expected ok");
    };
    let data: BoardHttpData = serde_json::from_value(data.unwrap()).unwrap();
    assert_eq!(data.status.value(), 200);
    let mut body = Vec::new();
    loop {
        let (kind, payload) = read_frame(&mut recv).await.unwrap();
        if kind == 1 {
            body.extend(payload);
        } else {
            assert!(matches!(
                serde_json::from_slice(&payload).unwrap(),
                Control::Exit { code: Some(0) }
            ));
            break;
        }
    }
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["len"], upload.len());
    assert_eq!(json["sum"], checksum(&upload).to_string());

    // SSE over QUIC: the phone finishing its send half unsubscribes.
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(
        serde_json::json!({"op":"board_sse","project":"forever"}),
    ))
    .await
    .unwrap();
    let (_, first) = read_frame(&mut recv).await.unwrap();
    assert!(matches!(
        serde_json::from_slice(&first).unwrap(),
        Control::Ok { .. }
    ));
    let (_, ev) = read_frame(&mut recv).await.unwrap();
    assert_eq!(
        event(Some(serde_json::from_slice(&ev).unwrap())),
        (Some("revision".into()), "5".into())
    );
    send.finish().unwrap();
    let end = tokio::time::timeout(Duration::from_secs(5), read_frame(&mut recv))
        .await
        .expect("host kept the stream open after unsubscribe");
    assert!(end.is_err(), "stream ends without further frames");
    router.shutdown().await.unwrap();
}

/// Manual check against a live daemon (e.g. the coffee tunnel on 7788):
/// `UNIPI_BOARD_LIVE_PORT=7788 UNIPI_BOARD_LIVE_PROJECT=<scratch slug>
/// cargo test -p unipi-host --test board_relay live -- --ignored --nocapture`.
/// Writes one task + attachment into the given (scratch) project.
#[tokio::test]
#[ignore]
async fn live_daemon_smoke() {
    let port: u16 = std::env::var("UNIPI_BOARD_LIVE_PORT")
        .expect("UNIPI_BOARD_LIVE_PORT")
        .parse()
        .unwrap();
    let slug = std::env::var("UNIPI_BOARD_LIVE_PROJECT").expect("UNIPI_BOARD_LIVE_PROJECT");
    let kb = tempfile::tempdir().unwrap();
    std::fs::write(
        kb.path().join("daemon.json"),
        serde_json::json!({"pid": 1, "port": port, "host": "127.0.0.1"}).to_string(),
    )
    .unwrap();
    let host = host(Board::new(config(kb.path(), BinaryLookup::default())), true);

    let reply = board_http(&host, "GET", "/api/health", None, b"").await;
    println!("health: {}", String::from_utf8_lossy(&reply.body));
    assert_eq!(reply.data().status.value(), 200);
    let reply = board_http(&host, "GET", "/api/projects", None, b"").await;
    let projects = reply.json();
    assert!(projects
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["slug"] == slug.as_str()));
    println!("projects: {}", projects.as_array().unwrap().len());

    let (first, mut sse) = Sse::open(&host, &slug).await;
    assert!(matches!(first, Control::Ok { .. }), "{first:?}");
    let (_, initial) = event(sse.next().await);
    println!("initial revision {initial}");

    let reply = board_http(
        &host,
        "POST",
        &format!("/api/tasks/{slug}/create"),
        Some("application/json"),
        br#"{"title":"UNI-129 relay smoke test (safe to delete)"}"#,
    )
    .await;
    assert_eq!(reply.data().status.value(), 200, "{reply:?}");
    let id = reply.json()["id"].as_str().unwrap().to_string();
    println!("created {id}");
    let (_, bumped) = event(sse.next().await);
    println!("revision {initial} -> {bumped}");

    let reply = board_http(
        &host,
        "POST",
        &format!("/api/tasks/{slug}/{id}/move"),
        Some("application/json"),
        br#"{"status":"done"}"#,
    )
    .await;
    println!(
        "illegal move: {} {}",
        reply.data().status.value(),
        String::from_utf8_lossy(&reply.body)
    );
    assert!((400..500).contains(&reply.data().status.value()));

    let png = pattern(300 * 1024);
    let reply = board_http(
        &host,
        "POST",
        &format!("/api/tasks/{slug}/{id}/attachments?name=relay.bin"),
        Some("application/octet-stream"),
        &png,
    )
    .await;
    println!(
        "upload: {} {}",
        reply.data().status.value(),
        String::from_utf8_lossy(&reply.body)
    );
    assert_eq!(reply.data().status.value(), 200);
    let task = board_http(&host, "GET", &format!("/api/tasks/{slug}/{id}"), None, b"").await;
    let task = task.json();
    let name = task["attachments"][0]["name"]
        .as_str()
        .unwrap_or("relay.bin")
        .to_string();
    let reply = board_http(
        &host,
        "GET",
        &format!("/api/files/{slug}/{id}/{name}"),
        None,
        b"",
    )
    .await;
    assert_eq!(reply.data().status.value(), 200, "{task}");
    assert!(reply.body == png, "download matches upload");
    println!(
        "download ok ({} bytes, {})",
        reply.body.len(),
        reply.data().content_type
    );
    sse.close().await;
}
