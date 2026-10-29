//! UNI-319: a job's notify/wake from the app, and opening a run's session.
//! Temp dirs only; the "live chat" is a bridge record pointing at this test
//! process's own pid (never a real pi). Its own test binary, so the
//! process-wide `UNIPI_BRIDGE_DIR` set here can't leak into other tests.
#![cfg(unix)]

use std::time::Duration;

use tokio::io::AsyncWriteExt;
use tokio::net::UnixStream;
use unipi_host::{data_dir::HostPaths, protocol::Handler};
use unipi_proto::{frame::{encode, FrameKind}, AppErrorCode, Control, PROTO_VERSION};

fn handler(paths: HostPaths) -> Handler {
    Handler {
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
        paths,
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(vec!["work".to_string()]),
        board: Default::default(),
    }
}

async fn op(paths: &HostPaths, req: serde_json::Value) -> Control {
    let mut stream = UnixStream::connect(paths.app_socket()).await.expect("connect app.sock");
    let hello = serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req});
    stream.write_all(&encode(FrameKind::Control, &serde_json::to_vec(&hello).unwrap())).await.unwrap();
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(10), unipi_host::protocol::read_frame(&mut stream))
        .await
        .expect("timed out")
        .expect("frame");
    serde_json::from_slice(&bytes).unwrap()
}

fn ok(reply: Control) -> serde_json::Value {
    let Control::Ok { data } = reply else { panic!("{reply:?}") };
    data.unwrap_or(serde_json::Value::Null)
}

fn err(reply: &Control) -> (AppErrorCode, String) {
    let Control::Err { error } = reply else { panic!("expected an error, got {reply:?}") };
    (error.code, error.message.to_string())
}

/// A bridge record for this test process (alive), on `session_file`.
fn live_chat(bridge_dir: &std::path::Path, session_file: &str) -> u32 {
    let pid = std::process::id();
    std::fs::write(bridge_dir.join("none.sock"), b"").unwrap(); // read_bridges wants the socket path to exist
    let record = serde_json::json!({
        "pid": pid,
        "socket": bridge_dir.join("none.sock").to_string_lossy(),
        "sessionFile": session_file,
        "sessionId": "sess-1",
        "sessionName": "planning",
    });
    std::fs::write(bridge_dir.join(format!("{pid}.json")), record.to_string()).unwrap();
    pid
}

fn save_run(paths: &HostPaths, job_id: &str, run_id: &str, session_path: Option<&str>) {
    let run = unipi_host::cron::model::Run {
        id: run_id.into(),
        job_id: job_id.into(),
        status: unipi_host::cron::model::RunStatus::Ok,
        fired_at: 1,
        started_at: Some(1),
        ended_at: Some(2),
        exit_code: Some(0),
        summary: Some("done".into()),
        reason: None,
        session_path: session_path.map(str::to_string),
        cost_usd: None,
        missed: false,
        notify_errors: Vec::new(),
        wake: None,
    };
    unipi_host::cron::store::save_run(paths, &run).unwrap();
}

#[tokio::test]
async fn notify_wake_and_open_session_from_the_app() {
    let dir = tempfile::tempdir().unwrap();
    let bridges = dir.path().join("bridge");
    std::fs::create_dir_all(&bridges).unwrap();
    std::env::set_var("UNIPI_BRIDGE_DIR", &bridges);
    std::env::set_var("UNIPI_NOTIFY_CONFIG_JSON", dir.path().join("notify.json"));
    std::fs::write(dir.path().join("notify.json"), r#"{"ntfy":{"enabled":true,"serverUrl":"http://127.0.0.1:1","topic":"t"},"gotify":{"enabled":false},"telegram":{"enabled":true,"botToken":"x","chatId":"1"}}"#).unwrap();
    let paths = HostPaths::from_dir(dir.path().join("host"));
    paths.ensure_dir().unwrap();
    let _server = unipi_host::local::serve(handler(paths.clone())).unwrap();
    let cwd = dir.path().to_string_lossy().into_owned();
    let session_file = dir.path().join("chat.jsonl");
    std::fs::write(&session_file, "{\"type\":\"session\"}\n").unwrap();
    let session_file = session_file.to_string_lossy().into_owned();
    let pid = live_chat(&bridges, &session_file);

    // The settings say which channels a job can pick.
    let settings = ok(op(&paths, serde_json::json!({"op":"cron_settings_get"})).await);
    assert_eq!(settings["notify_channels"], serde_json::json!(["ntfy", "telegram"]));

    // Add with notify + wake on a live chat: stored as its session file.
    let job = ok(op(
        &paths,
        serde_json::json!({
            "op": "cron_add", "name": "digest", "prompt": "p", "schedule": "0 0 9 * * *", "tz": "UTC", "cwd": cwd,
            "notify": {"events": ["done", "failed"], "channels": ["Telegram"]},
            "wake_pid": pid,
        }),
    )
    .await);
    let id = job["id"].as_str().unwrap().to_string();
    assert_eq!(job["notify"], serde_json::json!({"channels": ["telegram"], "events": ["done", "failed"]}));
    assert_eq!(job["wake"]["session_file"], session_file);
    assert_eq!(job["wake"]["pid"], pid, "the live chat is found");
    assert_eq!(job["wake"]["session_name"], "planning");
    let stored = unipi_host::cron::store::load_jobs(&paths).unwrap();
    assert_eq!(stored[0].session_link.as_ref().unwrap().session_file.as_deref(), Some(session_file.as_str()));

    // The form sends what the user typed: plain language resolves like the CLI.
    let plain = ok(op(&paths, serde_json::json!({"op":"cron_add","name":"plain","prompt":"p","schedule":"every day at 9:00","tz":"UTC","cwd":cwd})).await);
    assert_eq!(plain["schedule"], "0 0 9 * * *");
    let plain_id = plain["id"].as_str().unwrap().to_string();
    let edited = ok(op(&paths, serde_json::json!({"op":"cron_edit","id":plain_id,"patch":{"schedule":"weekdays at 18:30"}})).await);
    assert_eq!(edited["schedule"], "0 30 18 * * MON-FRI");
    let once = ok(op(&paths, serde_json::json!({"op":"cron_add","name":"once","prompt":"p","schedule":"in 2 hours","cwd":cwd})).await);
    assert!(once["schedule"].as_str().unwrap().starts_with("at:"), "{once}");
    let (code, _) = err(&op(&paths, serde_json::json!({"op":"cron_add","name":"bad","prompt":"p","schedule":"whenever you like","cwd":cwd})).await);
    assert_eq!(code, AppErrorCode::E_SCHEMA);

    // A gone chat can't be picked.
    let (code, _) = err(&op(&paths, serde_json::json!({"op":"cron_edit","id":id,"patch":{"wake_pid": 4_000_000}})).await);
    assert_eq!(code, AppErrorCode::E_NOT_FOUND);

    // Edit: an empty notify goes back to the defaults; wake_clear drops the wake.
    let job = ok(op(&paths, serde_json::json!({"op":"cron_edit","id":id,"patch":{"notify":{}, "wake_clear": true}})).await);
    assert!(job.get("notify").is_none(), "{job}");
    assert!(job.get("wake").is_none(), "{job}");
    // Leaving them out keeps them.
    ok(op(&paths, serde_json::json!({"op":"cron_edit","id":id,"patch":{"notify":{"events":["skipped"]},"wake_pid":pid}})).await);
    let job = ok(op(&paths, serde_json::json!({"op":"cron_edit","id":id,"patch":{"name":"digest 2"}})).await);
    assert_eq!(job["notify"]["events"], serde_json::json!(["skipped"]));
    assert_eq!(job["wake"]["pid"], pid);

    // Open a run's session: a pi already on that file is reused (nothing started).
    save_run(&paths, &id, "r1", Some(&session_file));
    let target = serde_json::json!({"kind":"tmux_new_session","name":"cron"});
    let opened = ok(op(&paths, serde_json::json!({"op":"cron_open_session","run_id":"r1","target":target})).await);
    assert_eq!(opened["pid"], pid);
    assert_eq!(opened["already_open"], true);
    assert!(opened.get("started").is_none());

    // No saved session / a deleted file / an unknown run → not found, in plain words.
    save_run(&paths, &id, "r2", None);
    let (code, message) = err(&op(&paths, serde_json::json!({"op":"cron_open_session","run_id":"r2","target":target})).await);
    assert_eq!(code, AppErrorCode::E_NOT_FOUND);
    assert!(message.contains("no saved session"), "{message}");
    save_run(&paths, &id, "r3", Some("/nope/gone.jsonl"));
    let (code, message) = err(&op(&paths, serde_json::json!({"op":"cron_open_session","run_id":"r3","target":target})).await);
    assert_eq!(code, AppErrorCode::E_NOT_FOUND);
    assert!(message.contains("gone"), "{message}");
    let (code, _) = err(&op(&paths, serde_json::json!({"op":"cron_open_session","run_id":"r9","target":target})).await);
    assert_eq!(code, AppErrorCode::E_NOT_FOUND);

    // A protected mux session refuses, like chat_new.
    let (code, _) = err(&op(&paths, serde_json::json!({"op":"cron_open_session","run_id":"r1","target":{"kind":"tmux_window","session":"work"}})).await);
    assert_eq!(code, AppErrorCode::E_UNAUTHORIZED);
}
