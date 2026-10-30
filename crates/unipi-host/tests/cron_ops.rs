//! WP-H8 acceptance (spec-host §9.8; ext-abi §6): the `cron_*` ops end to end — this PC's
//! own app (`Peer::Local`, over `app.sock`) always has them; a phone needs the `cron`
//! sharing feature first (UNI-248 pattern), for read-only ops too. Add/edit/fire run
//! against the unipi-cron job files with a faux `$OINO_CRON_BIN`, pause/resume flip the
//! scheduler settings, and `test-notify` sends through the host's own channels.
//! Temp dirs only.
#![cfg(unix)]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::await_holding_lock,
    reason = "tests assert by unwrapping; the binary override is process-global"
)]

use std::os::unix::fs::PermissionsExt;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::net::UnixStream;
use unipi_cron::{model::RunStatus, scheduler::Scheduler, store, CronPaths};
use unipi_host::cron_serve::CronHost;
use unipi_host::{
    data_dir::HostPaths,
    devices::DeviceStore,
    protocol::{read_frame, Handler, Peer},
};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, Control, PROTO_VERSION,
};

static ENV_LOCK: Mutex<()> = Mutex::new(());

/// One test at a time for the process-global env overrides (a panicking test must not
/// poison the rest).
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn handler(paths: &HostPaths, cron: CronHost) -> Handler {
    Handler {
        cron,
        paths: paths.clone(),
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
    }
}

/// A host data dir with its agent dir beside it (tempdir). `$OINO_AGENT_DIR` is pointed at
/// it for the test's lifetime, so nothing ever reads or writes the real `~/.oino`; the
/// tests serialise on `ENV_LOCK` for that reason.
fn fixture() -> (tempfile::TempDir, HostPaths, CronPaths) {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path().join("host"));
    paths.ensure_dir().unwrap();
    let cron_paths = CronPaths::new(dir.path().join("agent"));
    std::env::set_var("OINO_AGENT_DIR", &cron_paths.agent_dir);
    (dir, paths, cron_paths)
}

fn restore_agent_dir() {
    std::env::remove_var("OINO_AGENT_DIR");
}

fn hello(req: serde_json::Value) -> Vec<u8> {
    encode(
        FrameKind::Control,
        &serde_json::to_vec(&serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req}))
            .unwrap(),
    )
}

async fn next_control<R: AsyncRead + Unpin>(read: &mut R) -> Control {
    let (kind, bytes) = tokio::time::timeout(Duration::from_secs(10), read_frame(read))
        .await
        .expect("timed out")
        .expect("frame");
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
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    serde_json::from_value(data).unwrap()
}

/// One op over `app.sock` (`Peer::Local`: this PC's own app — always allowed).
async fn local_op(paths: &HostPaths, req: serde_json::Value) -> (Control, UnixStream) {
    let mut stream = UnixStream::connect(paths.app_socket())
        .await
        .expect("connect app.sock");
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

/// Writes a fake `oino` and points `$OINO_CRON_BIN` at it.
fn fake_oino(script: &str) -> (tempfile::TempDir, Option<String>) {
    let bin_dir = tempfile::tempdir().unwrap();
    let bin = bin_dir.path().join("oino");
    std::fs::write(&bin, script).unwrap();
    std::fs::set_permissions(&bin, std::fs::Permissions::from_mode(0o755)).unwrap();
    let previous = std::env::var("OINO_CRON_BIN").ok();
    std::env::set_var("OINO_CRON_BIN", &bin);
    (bin_dir, previous)
}

fn restore_oino(previous: Option<String>) {
    match previous {
        Some(value) => std::env::set_var("OINO_CRON_BIN", value),
        None => std::env::remove_var("OINO_CRON_BIN"),
    }
}

async fn wait_for<F: Fn() -> bool>(cond: F, timeout: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if cond() {
            return true;
        }
        if tokio::time::Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn local_app_cron_ops_work_against_the_job_files() {
    let _guard = env_lock();
    let (_dir, paths, cron_paths) = fixture();
    let server = unipi_host::local::serve(handler(&paths, CronHost::new(&paths))).unwrap();
    let cwd = _dir.path().to_string_lossy().into_owned();

    // cron is off by default for phones; this PC's app is always fine.
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
    // The job is one file under <agent>/cron/jobs, in the v2 schema.
    let file = cron_paths.job_file(&id);
    assert!(file.is_file(), "{}", file.display());
    let stored: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();

    assert_eq!(stored["v"], 2);
    assert_eq!(stored["id"], id);

    // Edit keeps what was not given.
    let (reply, _) = local_op(
        &paths,
        serde_json::json!({
            "op": "cron_edit",
            "id": id,
            "patch": {
                "name": "morning digest",
                "schedule": "every day at 8:00",
            },
        }),
    )
    .await;
    let edited: serde_json::Value = data(reply);
    assert_eq!(edited["name"], "morning digest");
    assert_eq!(edited["schedule"], "0 0 8 * * *");

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_show","id":id})).await;
    let shown: serde_json::Value = data(reply);
    assert_eq!(shown["job"]["id"], id);
    assert_eq!(shown["runs"].as_array().unwrap().len(), 0);

    let (reply, _) = local_op(
        &paths,
        serde_json::json!({"op":"cron_parse","schedule":"every day at 9:00","tz":"UTC"}),
    )
    .await;
    let parsed: serde_json::Value = data(reply);
    assert_eq!(parsed["expression"], "0 0 9 * * *");
    assert_eq!(parsed["next_fires"].as_array().unwrap().len(), 3);

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_settings_get"})).await;
    let settings: serde_json::Value = data(reply);
    assert_eq!(settings["concurrency"], 2);
    assert_eq!(settings["enabled"], false);

    let (reply, _) = local_op(
        &paths,
        serde_json::json!({"op":"cron_settings_set","concurrency":5,"paused":true}),
    )
    .await;
    let settings: serde_json::Value = data(reply);
    assert_eq!(settings["concurrency"], 5);
    assert_eq!(settings["paused"], true);
    // ... stored in the library's own settings file, not the host config.
    assert!(cron_paths.settings().is_file());

    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_rm","id":id})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    assert!(!cron_paths.job_file(&id).exists());

    server.abort();
    restore_agent_dir();
}

#[tokio::test]
async fn run_now_fires_the_faux_binary_and_pause_blocks_new_runs() {
    let _guard = env_lock();
    let (_dir, paths, cron_paths) = fixture();
    let cwd = _dir.path().to_string_lossy().into_owned();
    let (_bin, previous) = fake_oino(
        r#"#!/bin/sh
echo '{"type":"turn_end","message":{"role":"assistant","content":[{"type":"text","text":"DIGEST: 3 emails"}],"usage":{"cost":{"total":0.01}}}}'
exit 0
"#,
    );
    let scheduler = Arc::new(Scheduler::with_paths(cron_paths.clone()));
    let server =
        unipi_host::local::serve(handler(&paths, CronHost::with_scheduler(&paths, scheduler)))
            .unwrap();

    let (reply, _) = local_op(
        &paths,
        serde_json::json!({
            "op": "cron_add",
            "name": "digest",
            "prompt": "Summarize today's inbox",
            "schedule": "at:2000-01-01T00:00:00Z",
            "tz": "UTC",
            "cwd": cwd,
        }),
    )
    .await;
    let job: serde_json::Value = data(reply);
    let id = job["id"].as_str().unwrap().to_string();

    // Run now: the embedded scheduler fires the faux binary.
    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_run","id":id})).await;
    let started: serde_json::Value = data(reply);
    let run_id = started["run_id"].as_str().unwrap().to_string();
    let done = wait_for(
        || {
            store::load_run(&cron_paths, &id, &run_id)
                .unwrap()
                .is_some_and(|r| r.status == RunStatus::Ok)
        },
        Duration::from_secs(10),
    )
    .await;
    assert!(done, "the faux run should finish ok");
    let run = store::load_run(&cron_paths, &id, &run_id).unwrap().unwrap();
    assert!(
        run.summary.as_deref().unwrap_or("").contains("3 emails"),
        "summary: {:?}",
        run.summary
    );

    // Run history over the wire.
    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_runs","id":id})).await;
    let runs: serde_json::Value = data(reply);
    assert_eq!(runs["runs"][0]["id"], run_id);
    assert_eq!(runs["runs"][0]["status"], "ok");

    // Pause/resume is the scheduler's own settings file.
    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_pause"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_status"})).await;
    let status: serde_json::Value = data(reply);
    assert_eq!(status["paused"], true);
    assert_eq!(status["concurrency"], 2);
    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_resume"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    let (reply, _) = local_op(&paths, serde_json::json!({"op":"cron_status"})).await;
    let status: serde_json::Value = data(reply);
    assert_eq!(status["paused"], false);

    server.abort();
    restore_oino(previous);
    restore_agent_dir();
}

#[tokio::test]
async fn test_notify_sends_through_the_hosts_channels() {
    let _guard = env_lock();
    let (_dir, paths, cron_paths) = fixture();
    let cwd = _dir.path().to_string_lossy().into_owned();
    let (addr, rx) = fake_server().await;
    let config_path = _dir.path().join("notify-settings.json");
    std::fs::write(
        &config_path,
        format!(
            r#"{{"gotify":{{"enabled":true,"serverUrl":"http://{addr}","appToken":"tok_g"}}}}"#
        ),
    )
    .unwrap();
    let previous_config = std::env::var("UNIPI_NOTIFY_CONFIG_JSON").ok();
    std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", &config_path);

    let job = unipi_cron::model::Job {
        id: "digest".into(),
        name: "digest".into(),
        prompt: "p".into(),
        schedule: "0 0 9 * * *".into(),
        tz: "UTC".into(),
        cwd,
        model: None,
        thinking: None,
        enabled: true,
        time_limit_s: 10,
        cost_cap_usd: None,
        overlap: unipi_cron::model::OverlapPolicy::default(),
        missed: unipi_cron::model::MissedPolicy::default(),
        notify: Some(unipi_cron::model::NotifySpec {
            channels: Some(vec!["gotify".into()]),
            events: None,
        }),
        session_link: None,
        created: 0,
        updated: 0,
    };
    store::save_jobs(&cron_paths, &[job]).unwrap();

    let cron = CronHost::new(&paths);
    let reply = cron
        .control(unipi_cron::control::CronControl::TestNotify {
            id: "digest".into(),
        })
        .await
        .unwrap();
    assert_eq!(reply["channels"][0], "gotify");
    assert_eq!(reply["errors"].as_array().unwrap().len(), 0, "{reply}");
    let request = tokio::time::timeout(Duration::from_secs(5), rx)
        .await
        .expect("the test notification should arrive")
        .unwrap();
    assert!(request.starts_with("POST /message"), "request: {request}");
    assert!(request.contains("Cron: digest ok"), "request: {request}");

    match previous_config {
        Some(value) => std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", value),
        None => std::env::remove_var("UNIPI_NOTIFY_CONFIG_JSON"),
    }
    restore_agent_dir();
}

/// One-shot fake HTTP server: captures the raw request and replies 200.
async fn fake_server() -> (std::net::SocketAddr, tokio::sync::oneshot::Receiver<String>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut buf = vec![0u8; 16384];
        let n = socket.read(&mut buf).await.unwrap_or(0);
        buf.truncate(n);
        let body =
            "HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}".to_string();
        let _ = socket.write_all(body.as_bytes()).await;
        let _ = socket.shutdown().await;
        let _ = tx.send(String::from_utf8_lossy(&buf).into_owned());
    });
    (addr, rx)
}

#[tokio::test]
async fn phone_needs_the_cron_feature_for_every_op_including_read_only() {
    let _guard = env_lock();
    let (_dir, paths, _cron_paths) = fixture();
    let phone = iroh::SecretKey::generate().public();
    DeviceStore::new(paths.devices())
        .add(&phone, "Pixel".into())
        .unwrap();
    let h = handler(&paths, CronHost::new(&paths));

    // Off by default: every op, including the read-only ones, is refused.
    for req in [
        serde_json::json!({"op":"cron_list"}),
        serde_json::json!({"op":"cron_status"}),
        serde_json::json!({"op":"cron_parse","schedule":"hourly"}),
        serde_json::json!({"op":"cron_settings_get"}),
    ] {
        let reply = phone_op(&h, phone, req.clone()).await;
        assert_eq!(
            err_code(&reply),
            Some(AppErrorCode::E_UNAUTHORIZED),
            "{req}: {reply:?}"
        );
    }

    // The owner shares cron with this phone (same pattern as push/forward/browser).
    unipi_host::sharing::cron_enable(&paths, Some(vec!["Pixel".into()])).unwrap();
    let reply = phone_op(&h, phone, serde_json::json!({"op":"cron_list"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");

    // A second, unlisted phone still can't.
    let other = iroh::SecretKey::generate().public();
    DeviceStore::new(paths.devices())
        .add(&other, "Other".into())
        .unwrap();
    let reply = phone_op(&h, other, serde_json::json!({"op":"cron_list"})).await;
    assert_eq!(
        err_code(&reply),
        Some(AppErrorCode::E_UNAUTHORIZED),
        "{reply:?}"
    );
    restore_agent_dir();
}
