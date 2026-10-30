//! UNI-229 + v2 (spec host.md §6.1): the host's local listener — the same
//! frames and Hello as a QUIC bi-stream, one op per connection, `Peer::Local`
//! = this PC's owner: no pairing needed (a stranger over iroh is refused),
//! per-device allow lists treat it as allowed, pairing over it is refused.
//!
//! The same body runs on both transports: `<dir>/app.sock` on Unix and the
//! named pipe `\\.\pipe\unipi-oino-app` on Windows (`HostPaths::local_addr`).
//! Temp dirs only.
//!
//! Caveat for the Windows run: every host on a machine shares one pipe name,
//! so a second `local::serve` in the same test binary would race the first —
//! `serve` claims the name with `first_pipe_instance`, and the pipe tests
//! below run serialised through `PIPE_LOCK`.

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWriteExt};

use unipi_host::{
    data_dir::HostPaths,
    ipc,
    protocol::{read_frame, Handler, Peer},
};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, Control, PROTO_VERSION,
};

#[cfg(windows)]
static PIPE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// One host listener at a time on Windows (one pipe name per machine).
#[cfg(windows)]
fn pipe_guard() -> std::sync::MutexGuard<'static, ()> {
    PIPE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn handler(paths: HostPaths) -> Handler {
    Handler {
        cron: unipi_host::cron_serve::CronHost::new(&paths),
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

/// Connects to the listener of `paths` over whichever transport this OS uses.
async fn connect(paths: &HostPaths) -> ipc::Stream {
    ipc::connect(&paths.local_addr())
        .await
        .expect("connect the local listener")
}

async fn next_control<R: AsyncRead + Unpin>(read: &mut R) -> Control {
    let (kind, bytes) = tokio::time::timeout(Duration::from_secs(10), read_frame(read))
        .await
        .expect("timed out")
        .expect("frame");
    assert_eq!(kind, 0);
    serde_json::from_slice(&bytes).unwrap()
}

/// One op over the local listener: send the hello, return the first control
/// reply.
async fn op(paths: &HostPaths, req: serde_json::Value) -> Control {
    let mut stream = connect(paths).await;
    stream.write_all(&hello(req)).await.unwrap();
    next_control(&mut stream).await
}

fn err_code(c: &Control) -> Option<AppErrorCode> {
    match c {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

#[tokio::test]
async fn local_socket_serves_ops_as_the_owner() {
    #[cfg(windows)]
    let _pipe = pipe_guard();
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let server = unipi_host::local::serve(handler(paths.clone())).unwrap();

    // The socket is private (Windows: the pipe's DACL, checked by `local.rs`).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(paths.app_socket())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }

    // No pairing needed: fs_list of the data dir works with an EMPTY devices.json.
    std::fs::write(dir.path().join("note.txt"), "hi").unwrap();
    match op(
        &paths,
        serde_json::json!({"op":"fs_list","path":dir.path(),"outside":true}),
    )
    .await
    {
        Control::Ok { data: Some(d) } => assert!(d.to_string().contains("note.txt"), "{d}"),
        other => panic!("fs_list over the local listener: {other:?}"),
    }

    // Pairing over the local listener is refused (nothing to pair).
    let reply = op(&paths, serde_json::json!({"op":"pair","code":"ABCDEFGH"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED));

    // Forwarding on, but only for a named phone: Local is still allowed (owner).
    std::fs::write(
        paths.config(),
        r#"{"forwarding":{"enabled":true,"ports":"any","devices":["Pixel"]}}"#,
    )
    .unwrap();
    let reply = op(&paths, serde_json::json!({"op":"ports_list"})).await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "ports_list for the local owner: {reply:?}"
    );

    // Bad first frame → same error as over iroh.
    let mut s = connect(&paths).await;
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

/// Windows only (spec host.md risk R2: "H2 delivers it first with a dedicated
/// test"; uid-equivalent = owner-only DACL + `GetNamedPipeClientProcessId`):
/// the pipe name is the v2 one, the host claims it exclusively, and a
/// same-user client is accepted by the SID check.
#[cfg(windows)]
#[tokio::test]
async fn pipe_is_owner_only_and_exclusive() {
    let _pipe = pipe_guard();
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    assert_eq!(
        paths.local_addr(),
        unipi_host::local::PIPE_NAME.to_string(),
        "the v2 pipe name (spec §5.1)"
    );

    let server = unipi_host::local::serve(handler(paths.clone())).unwrap();
    // One host per machine: a second listener can't claim the name.
    let second = unipi_host::local::serve(handler(paths.clone()));
    assert!(second.is_err(), "a second listener must not claim the pipe");

    // A client of the same user gets an op answered (owner-only DACL + SID
    // check in the accept loop).
    std::fs::write(dir.path().join("note.txt"), "hi").unwrap();
    let reply = op(
        &paths,
        serde_json::json!({"op":"fs_list","path":dir.path(),"outside":true}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "same-user client over the pipe: {reply:?}"
    );

    server.abort();
}
