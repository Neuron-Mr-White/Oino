//! UNI-229: the host's local listener `app.sock` — same frames and Hello as a
//! QUIC bi-stream, one op per connection, `Peer::Local` = this PC's owner:
//! no pairing needed (a stranger over iroh is refused), per-device allow
//! lists treat it as allowed, pairing over it is refused. Temp dirs only.
#![cfg(unix)]

use std::time::Duration;

use tokio::io::AsyncWriteExt;

use tokio::net::UnixStream;
use unipi_host::{
    data_dir::HostPaths,
    protocol::{read_frame, Handler, Peer},
};
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
    }
}

fn hello(req: serde_json::Value) -> Vec<u8> {
    encode(
        FrameKind::Control,
        &serde_json::to_vec(&serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req}))
            .unwrap(),
    )
}

async fn next_control<R: tokio::io::AsyncRead + Unpin>(read: &mut R) -> Control {
    let (kind, bytes) = tokio::time::timeout(Duration::from_secs(10), read_frame(read))
        .await
        .expect("timed out")
        .expect("frame");
    assert_eq!(kind, 0);
    serde_json::from_slice(&bytes).unwrap()
}

/// One op over app.sock: send the hello, return the first control reply (+ the stream).
async fn op(paths: &HostPaths, req: serde_json::Value) -> (Control, UnixStream) {
    let mut stream = UnixStream::connect(paths.app_socket())
        .await
        .expect("connect app.sock");
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

#[tokio::test]
async fn local_socket_serves_ops_as_the_owner() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let server = unipi_host::local::serve(handler(paths.clone())).unwrap();

    // The socket is private.
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(paths.app_socket())
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );

    // No pairing needed: fs_list of the data dir works with an EMPTY devices.json.
    std::fs::write(dir.path().join("note.txt"), "hi").unwrap();
    let (reply, _) = op(
        &paths,
        serde_json::json!({"op":"fs_list","path":dir.path(),"outside":true}),
    )
    .await;
    match reply {
        Control::Ok { data: Some(d) } => assert!(d.to_string().contains("note.txt"), "{d}"),
        other => panic!("fs_list over app.sock: {other:?}"),
    }

    // Pairing over the local socket is refused (nothing to pair).
    let (reply, _) = op(&paths, serde_json::json!({"op":"pair","code":"ABCDEFGH"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED));

    // Forwarding on, but only for a named phone: Local is still allowed (owner).
    std::fs::write(
        paths.config(),
        r#"{"forwarding":{"enabled":true,"ports":"any","devices":["Pixel"]}}"#,
    )
    .unwrap();
    let (reply, _) = op(&paths, serde_json::json!({"op":"ports_list"})).await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "ports_list for the local owner: {reply:?}"
    );

    // Bad first frame → same error as over iroh.
    let mut s = UnixStream::connect(paths.app_socket()).await.unwrap();
    s.write_all(&encode(FrameKind::Input, b"{}")).await.unwrap();
    assert_eq!(
        err_code(&next_control(&mut s).await),
        Some(AppErrorCode::E_SCHEMA)
    );

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
    cw.write_all(&hello(serde_json::json!({"op":"fs_roots"})))
        .await
        .unwrap();
    assert_eq!(
        err_code(&next_control(&mut cr).await),
        Some(AppErrorCode::E_UNAUTHORIZED)
    );
    task.await.unwrap().unwrap();
    // And the forwarding allow list still applies to a real phone.
    std::fs::write(
        paths.config(),
        r#"{"forwarding":{"enabled":true,"ports":"any","devices":["Pixel"]}}"#,
    )
    .unwrap();
    let config = unipi_host::forward::HostConfig::load(&paths.config())
        .unwrap()
        .forwarding;
    assert!(config
        .check_device(&stranger.to_string(), Some("Redmi"))
        .is_err());
    assert!(config.check_device("local", Some("This computer")).is_ok());
}
