//! UNI-229: the host's local listener `app.sock` — same frames and Hello as a
//! QUIC bi-stream, one op per connection, `Peer::Local` = this PC's owner:
//! no pairing needed (a stranger over iroh is refused), per-device allow
//! lists treat it as allowed, pairing over it is refused. Temp dirs only.
#![cfg(unix)]

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use unipi_host::{data_dir::HostPaths, protocol::{read_frame, Handler, Peer}};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, Control, PROTO_VERSION,
};

fn handler(paths: HostPaths) -> Handler {
    Handler {
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        paths,
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(vec![]),
        board: Default::default(),
    }
}

fn hello(req: serde_json::Value) -> Vec<u8> {
    encode(FrameKind::Control, &serde_json::to_vec(&serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req})).unwrap())
}

async fn next_control<R: tokio::io::AsyncRead + Unpin>(read: &mut R) -> Control {
    let (kind, bytes) = tokio::time::timeout(Duration::from_secs(10), read_frame(read)).await.expect("timed out").expect("frame");
    assert_eq!(kind, 0);
    serde_json::from_slice(&bytes).unwrap()
}

/// One op over app.sock: send the hello, return the first control reply (+ the stream).
async fn op(paths: &HostPaths, req: serde_json::Value) -> (Control, UnixStream) {
    let mut stream = UnixStream::connect(paths.app_socket()).await.expect("connect app.sock");
    stream.write_all(&hello(req)).await.unwrap();
    let reply = next_control(&mut stream).await;
    (reply, stream)
}

fn err_code(c: &Control) -> Option<AppErrorCode> {
    match c {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::test]
async fn local_socket_serves_ops_as_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let server = unipi_host::local::serve(handler(paths.clone())).unwrap();

    // The socket is private.
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(std::fs::metadata(paths.app_socket()).unwrap().permissions().mode() & 0o777, 0o600);

    // No pairing needed: fs_list of the data dir works with an EMPTY devices.json.
    std::fs::write(dir.path().join("note.txt"), "hi").unwrap();
    let (reply, _) = op(&paths, serde_json::json!({"op":"fs_list","path":dir.path(),"outside":true})).await;
    match reply {
        Control::Ok { data: Some(d) } => assert!(d.to_string().contains("note.txt"), "{d}"),
        other => panic!("fs_list over app.sock: {other:?}"),
    }

    // Pairing over the local socket is refused (nothing to pair).
    let (reply, _) = op(&paths, serde_json::json!({"op":"pair","code":"ABCDEFGH"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED));

    // Forwarding on, but only for a named phone: Local is still allowed (owner).
    std::fs::write(paths.config(), r#"{"forwarding":{"enabled":true,"ports":"any","devices":["Pixel"]}}"#).unwrap();
    let (reply, _) = op(&paths, serde_json::json!({"op":"ports_list"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "ports_list for the local owner: {reply:?}");

    // Bad first frame → same error as over iroh.
    let mut s = UnixStream::connect(paths.app_socket()).await.unwrap();
    s.write_all(&encode(FrameKind::Input, b"{}")).await.unwrap();
    assert_eq!(err_code(&next_control(&mut s).await), Some(AppErrorCode::E_SCHEMA));

    server.abort();
}

#[tokio::test]
async fn iroh_peers_still_need_pairing() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let h = handler(paths.clone());
    let (client, srv) = tokio::io::duplex(1 << 16);
    let (r, w) = tokio::io::split(srv);
    let stranger = iroh::SecretKey::generate().public();
    let task = tokio::spawn(async move { h.handle(r, w, Peer::Iroh(stranger)).await });
    let (mut cr, mut cw) = tokio::io::split(client);
    cw.write_all(&hello(serde_json::json!({"op":"fs_roots"}))).await.unwrap();
    assert_eq!(err_code(&next_control(&mut cr).await), Some(AppErrorCode::E_UNAUTHORIZED));
    task.await.unwrap().unwrap();
    // And the forwarding allow list still applies to a real phone.
    std::fs::write(paths.config(), r#"{"forwarding":{"enabled":true,"ports":"any","devices":["Pixel"]}}"#).unwrap();
    let config = unipi_host::forward::HostConfig::load(&paths.config()).unwrap().forwarding;
    assert!(config.check_device(&stranger.to_string(), Some("Redmi")).is_err());
    assert!(config.check_device("local", Some("This computer")).is_ok());
}

#[tokio::test]
async fn chat_relays_over_the_local_socket() {
    let bridge_dir = tempfile::tempdir().unwrap();
    let pid = std::process::id();
    let socket = bridge_dir.path().join(format!("{pid}.sock"));
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    std::fs::write(
        bridge_dir.path().join(format!("{pid}.json")),
        serde_json::json!({"v":1,"pid":pid,"socket":socket,"sessionFile":"/s.jsonl"}).to_string(),
    )
    .unwrap();
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("UNIPI_BRIDGE_DIR", bridge_dir.path());
    let pi = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        write.write_all(b"{\"t\":\"hello\",\"v\":1}\n").await.unwrap();
        let mut lines = BufReader::new(read).lines();
        if let Ok(Some(line)) = lines.next_line().await {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            write.write_all(format!("{}\n", serde_json::json!({"t":"ack","ref":v["ref"]})).as_bytes()).await.unwrap();
        }
    });

    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let server = unipi_host::local::serve(handler(paths.clone())).unwrap();
    let (reply, mut stream) = op(&paths, serde_json::json!({"op":"chat_open","pid":pid})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    match next_control(&mut stream).await {
        Control::Chat { line } => assert_eq!(line, r#"{"t":"hello","v":1}"#),
        other => panic!("{other:?}"),
    }
    let line = serde_json::to_vec(&Control::Chat { line: r#"{"t":"prompt","text":"hi","ref":"r1"}"#.into() }).unwrap();
    stream.write_all(&encode(FrameKind::Control, &line)).await.unwrap();
    match next_control(&mut stream).await {
        Control::Chat { line } => assert!(line.contains("\"r1\""), "{line}"),
        other => panic!("{other:?}"),
    }
    pi.await.unwrap();
    server.abort();
    std::env::remove_var("UNIPI_BRIDGE_DIR");
}

#[tokio::test]
async fn tmux_list_over_the_local_socket() {
    if std::process::Command::new("tmux").arg("-V").output().is_err() {
        eprintln!("SKIP: no tmux");
        return;
    }
    let name = format!("unipi-local-test-{}", std::process::id());
    let _g = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("UNIPI_TMUX_SOCKET", &name);
    let ok = std::process::Command::new("tmux").args(["-L", &name, "new-session", "-d", "-s", "localtest", "sleep 30"]).status().unwrap().success();
    assert!(ok);
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let server = unipi_host::local::serve(handler(paths.clone())).unwrap();
    let (reply, _) = op(&paths, serde_json::json!({"op":"tree"})).await;
    let _ = std::process::Command::new("tmux").args(["-L", &name, "kill-server"]).status();
    std::env::remove_var("UNIPI_TMUX_SOCKET");
    server.abort();
    match reply {
        Control::Ok { data: Some(d) } => assert!(d.to_string().contains("localtest"), "{d}"),
        other => panic!("{other:?}"),
    }
}
