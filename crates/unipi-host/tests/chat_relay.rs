//! Chat relay through the host handler (DESIGN §3.4): a fake bridge on a
//! unix socket (always), and — when `UNIPI_CHAT_E2E_DIR` points at a live
//! app-bridge dir — a real pi session (`chat_list` must show it and
//! `chat_open` must greet with its snapshot).

use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use unipi_host::{
    board::{Board, BoardConfig},
    data_dir::HostPaths,
    devices::DeviceStore,
    protocol::{read_frame, Handler},
};
use unipi_proto::{
    frame::{encode, FrameKind},
    AppErrorCode, Control, PROTO_VERSION,
};

struct Host {
    handler: Handler,
    phone: iroh::EndpointId,
    _dir: tempfile::TempDir,
}

fn host() -> Host {
    let dir = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(dir.path());
    paths.ensure_dir().unwrap();
    let phone = iroh::SecretKey::generate().public();
    DeviceStore::new(paths.devices())
        .add(&phone, "Test phone".into())
        .unwrap();
    Host {
        handler: Handler {
            cron: unipi_host::cron::Scheduler::new(paths.clone()),
            paths,
            host_id: iroh::SecretKey::generate().public(),
            host_name: "Test".into(),
            lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
            protected: std::sync::Arc::new(vec![]),
            board: Board::new(BoardConfig::from_env()),
        },
        phone,
        _dir: dir,
    }
}

fn hello(req: serde_json::Value) -> Vec<u8> {
    encode(
        FrameKind::Control,
        &serde_json::to_vec(&serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req}))
            .unwrap(),
    )
}

fn chat(line: &str) -> Vec<u8> {
    encode(
        FrameKind::Control,
        &serde_json::to_vec(&Control::Chat { line: line.into() }).unwrap(),
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

/// Serializes the tests that set UNIPI_BRIDGE_DIR (process-global env).
static ENV: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[tokio::test]
async fn chat_open_relays_a_fake_bridge_and_reports_missing_pids() {
    let bridge_dir = tempfile::tempdir().unwrap();
    let pid = std::process::id();
    let socket = bridge_dir.path().join(format!("{pid}.sock"));
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    std::fs::write(
        bridge_dir.path().join(format!("{pid}.json")),
        serde_json::json!({"v":1,"pid":pid,"socket":socket,"sessionFile":"/s.jsonl"}).to_string(),
    )
    .unwrap();
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("UNIPI_BRIDGE_DIR", bridge_dir.path());

    // The fake pi: greets, then echoes each line back as an ack.
    let pi = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let (read, mut write) = stream.into_split();
        write.write_all(b"{\"t\":\"hello\",\"v\":1}\n").await.unwrap();
        let mut lines = BufReader::new(read).lines();
        while let Ok(Some(line)) = lines.next_line().await {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            let ack = serde_json::json!({"t":"ack","ref":v["ref"]});
            write.write_all(format!("{ack}\n").as_bytes()).await.unwrap();
            if v["t"] == "abort" {
                break; // pi "quits"
            }
        }
    });

    let h = host();
    let (client, server) = tokio::io::duplex(1 << 20);
    let (read, write) = tokio::io::split(server);
    let handler = h.handler.clone();
    let phone = h.phone;
    let task = tokio::spawn(async move { handler.handle(read, write, phone).await });
    let (mut c_read, mut c_write) = tokio::io::split(client);
    c_write
        .write_all(&hello(serde_json::json!({"op":"chat_open","pid":pid})))
        .await
        .unwrap();
    assert!(matches!(next_control(&mut c_read).await, Control::Ok { .. }));
    match next_control(&mut c_read).await {
        Control::Chat { line } => assert_eq!(line, r#"{"t":"hello","v":1}"#),
        other => panic!("{other:?}"),
    }
    c_write.write_all(&chat(r#"{"t":"prompt","text":"hi","ref":"r1"}"#)).await.unwrap();
    match next_control(&mut c_read).await {
        Control::Chat { line } => assert_eq!(
            serde_json::from_str::<serde_json::Value>(&line).unwrap(),
            serde_json::json!({"t":"ack","ref":"r1"})
        ),
        other => panic!("{other:?}"),
    }
    c_write.write_all(&chat(r#"{"t":"abort","ref":"r2"}"#)).await.unwrap();
    assert!(matches!(next_control(&mut c_read).await, Control::Chat { .. }));
    assert!(matches!(next_control(&mut c_read).await, Control::Exit { code: None }));
    task.await.unwrap().unwrap();
    pi.await.unwrap();

    // A pid with no bridge.
    let (client, server) = tokio::io::duplex(1 << 16);
    let (read, write) = tokio::io::split(server);
    let handler = h.handler.clone();
    let task = tokio::spawn(async move { handler.handle(read, write, phone).await });
    let (mut c_read, mut c_write) = tokio::io::split(client);
    c_write
        .write_all(&hello(serde_json::json!({"op":"chat_open","pid":1})))
        .await
        .unwrap();
    match next_control(&mut c_read).await {
        Control::Err { error } => assert_eq!(error.code, AppErrorCode::E_NOT_FOUND),
        other => panic!("{other:?}"),
    }
    task.await.unwrap().unwrap();
    std::env::remove_var("UNIPI_BRIDGE_DIR");
    drop(guard);
}

/// Live check against a real pi + app-bridge in a throwaway herdr session.
/// Run with `UNIPI_CHAT_E2E_DIR=<bridge dir> cargo test --test chat_relay -- --ignored`.
#[tokio::test]
#[ignore]
async fn live_pi_bridge_shows_in_chat_list_and_greets() {
    let Ok(dir) = std::env::var("UNIPI_CHAT_E2E_DIR") else {
        return;
    };
    let guard = ENV.lock().unwrap_or_else(|e| e.into_inner());
    std::env::set_var("UNIPI_BRIDGE_DIR", &dir);
    let h = host();
    let phone = h.phone;

    let (client, server) = tokio::io::duplex(1 << 22);
    let (read, write) = tokio::io::split(server);
    let handler = h.handler.clone();
    let task = tokio::spawn(async move { handler.handle(read, write, phone).await });
    let (mut c_read, mut c_write) = tokio::io::split(client);
    c_write
        .write_all(&hello(serde_json::json!({"op":"chat_list"})))
        .await
        .unwrap();
    let data = match next_control(&mut c_read).await {
        Control::Ok { data } => data.unwrap(),
        other => panic!("{other:?}"),
    };
    task.await.unwrap().unwrap();
    let mut pids = Vec::new();
    for session in data["herdr"]["sessions"].as_array().unwrap() {
        for ws in session["workspaces"].as_array().unwrap() {
            for tab in ws["tabs"].as_array().unwrap() {
                for pane in tab["panes"].as_array().unwrap() {
                    if let Some(pid) = pane["chat"]["pid"].as_u64() {
                        eprintln!("chat pane {} {} pid {pid}", session["name"], pane["id"]);
                        pids.push(pid);
                    }
                }
            }
        }
    }
    assert!(!pids.is_empty(), "no pane matched a live bridge: {data}");

    let (client, server) = tokio::io::duplex(1 << 22);
    let (read, write) = tokio::io::split(server);
    let handler = h.handler.clone();
    let task = tokio::spawn(async move { handler.handle(read, write, phone).await });
    let (mut c_read, mut c_write) = tokio::io::split(client);
    c_write
        .write_all(&hello(serde_json::json!({"op":"chat_open","pid":pids[0]})))
        .await
        .unwrap();
    assert!(matches!(next_control(&mut c_read).await, Control::Ok { .. }));
    match next_control(&mut c_read).await {
        Control::Chat { line } => {
            let v: serde_json::Value = serde_json::from_str(&line).unwrap();
            assert_eq!(v["t"], "hello");
            eprintln!(
                "hello: {} entries, {} commands, model {}",
                v["entries"].as_array().unwrap().len(),
                v["commands"].as_array().unwrap().len(),
                v["state"]["model"]["id"]
            );
        }
        other => panic!("{other:?}"),
    }
    // Both halves must go for the duplex to close (the phone hanging up).
    drop(c_write);
    drop(c_read);
    tokio::time::timeout(Duration::from_secs(10), task)
        .await
        .expect("relay did not end after the phone hung up")
        .unwrap()
        .ok();
    std::env::remove_var("UNIPI_BRIDGE_DIR");
    drop(guard);
}
