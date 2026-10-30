//! UNI-317: `cron_*` ops end to end — this PC's own app (`Peer::Local`,
//! over `app.sock`) always has them; a phone needs the `cron` sharing
//! feature first (UNI-248 pattern), for read-only ops too. Temp dirs only.
#![cfg(unix)]

use std::time::Duration;

use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use unipi_host::{
    data_dir::HostPaths,
    devices::DeviceStore,
    protocol::{read_frame, Handler, Peer},
};
use unipi_proto::{frame::{encode, FrameKind}, AppErrorCode, Control, PROTO_VERSION};

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

async fn next_control<R: AsyncRead + Unpin>(read: &mut R) -> Control {
    let (kind, bytes) = tokio::time::timeout(Duration::from_secs(10), read_frame(read)).await.expect("timed out").expect("frame");
    assert_eq!(kind, 0);
    serde_json::from_slice(&bytes).unwrap()
}

fn err_code(c: &Control) -> Option<AppErrorCode> {
    match c {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

fn data<T: serde::de::DeserializeOwned>(reply: Control) -> T {
    let Control::Ok { data: Some(data) } = reply else { panic!("{reply:?}") };
    serde_json::from_value(data).unwrap()
}

/// One op over `app.sock` (`Peer::Local`: this PC's own app — always allowed).
async fn local_op(paths: &HostPaths, req: serde_json::Value) -> (Control, UnixStream) {
    let mut stream = UnixStream::connect(paths.app_socket()).await.expect("connect app.sock");
    stream.write_all(&hello(req)).await.unwrap();
    let reply = next_control(&mut stream).await;
    (reply, stream)
}

/// One op from a paired phone, straight through the handler (an in-memory
/// duplex pipe, like the other relay tests — no real iroh needed to prove
/// the gating).
async fn phone_op(handler: &Handler, phone: iroh::EndpointId, req: serde_json::Value) -> Control {
    let (client, server) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(server);
    let h = handler.clone();
    let task = tokio::spawn(async move { h.handle(read, write, Peer::Iroh(phone)).await });
    let (mut cread, mut cwrite): (_, Box<dyn AsyncWrite + Unpin + Send>) = {
        let (r, w) = tokio::io::split(client);
        (r, Box::new(w))
    };
    cwrite.write_all(&hello(req)).await.unwrap();
    let reply = next_control(&mut cread).await;
    task.await.unwrap().unwrap();
    reply
}

#[tokio::test]
async fn local_app_always_has_cron_even_when_the_feature_is_off_for_phones() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let server = unipi_host::local::serve(handler(paths.clone())).unwrap();
    let cwd = dir.path().to_string_lossy().into_owned();

    // cron is off by default (CronSettings::enabled == false) -- still fine for Local.
    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_list"})).await;
    let listed: serde_json::Value = data(reply);
    assert_eq!(listed["jobs"].as_array().unwrap().len(), 0);
    assert_eq!(listed["paused"], false);

    let (reply, _) = local_op(
        &paths,
        serde_json::json!({
            "op": "cron_add",
            "name": "daily digest",
            "prompt": "Summarize today's inbox",
            "schedule": "0 0 9 * * *",
            "tz": "UTC",
            "cwd": cwd,
        }),
    )
    .await;
    let job: serde_json::Value = data(reply);
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!(job["name"], "daily digest");
    assert!(job["next_fire"].is_string(), "{job}");

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_show","id":id})).await;
    let shown: serde_json::Value = data(reply);
    assert_eq!(shown["job"]["id"], id);
    assert_eq!(shown["runs"].as_array().unwrap().len(), 0);

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_parse","schedule":"every day at 9:00","tz":"UTC"})).await;
    let parsed: serde_json::Value = data(reply);
    assert_eq!(parsed["expression"], "0 0 9 * * *");
    assert_eq!(parsed["next_fires"].as_array().unwrap().len(), 3);

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_settings_get"})).await;
    let settings: serde_json::Value = data(reply);
    assert_eq!(settings["concurrency"], 2);
    assert_eq!(settings["enabled"], false);

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_settings_set","concurrency":5,"paused":true})).await;
    let settings: serde_json::Value = data(reply);
    assert_eq!(settings["concurrency"], 5);
    assert_eq!(settings["paused"], true);

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_rm","id":id})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");

    server.abort();
}

#[tokio::test]
async fn phone_needs_the_cron_feature_for_every_op_including_read_only() {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let phone = iroh::SecretKey::generate().public();
    DeviceStore::new(paths.devices()).add(&phone, "Pixel".into()).unwrap();
    let h = handler(paths.clone());

    // Off by default: every op, including the read-only ones, is refused.
    for req in [
        serde_json::json!({"op":"cron_list"}),
        serde_json::json!({"op":"cron_status"}),
        serde_json::json!({"op":"cron_parse","schedule":"hourly"}),
        serde_json::json!({"op":"cron_settings_get"}),
    ] {
        let reply = phone_op(&h, phone, req.clone()).await;
        assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED), "{req}: {reply:?}");
    }

    // The owner shares cron with this phone (same pattern as push/forward/browser).
    unipi_host::sharing::cron_enable(&paths, Some(vec!["Pixel".into()])).unwrap();
    let reply = phone_op(&h, phone, serde_json::json!({"op":"cron_list"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");

    // A second, unlisted phone still can't.
    let other = iroh::SecretKey::generate().public();
    DeviceStore::new(paths.devices()).add(&other, "Other".into()).unwrap();
    let reply = phone_op(&h, other, serde_json::json!({"op":"cron_list"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED), "{reply:?}");
}
