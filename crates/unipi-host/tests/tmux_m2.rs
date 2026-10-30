//! M2 tmux integration: tree, grouped attach, preview, on a private
//! socket (`UNIPI_TMUX_SOCKET`). Never touches the user's default
//! tmux server. The whole test runs under a 30 s timeout so a hung
//! PTY read fails fast.

use iroh::{
    endpoint::{presets, RelayMode},
    Endpoint,
};
use unipi_host::{
    data_dir::HostPaths,
    devices::DeviceStore,
    endpoint::ALPN,
    mux::tmux::TMUX_SOCKET_ENV,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, Handler},
};
use unipi_proto::{AppErrorCode, Control};

/// Serialises tests that touch the process-global `TMUX_SOCKET_ENV`.
pub static ENV_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// Kills the private tmux server on drop (panic, timeout or success).
struct TmuxServer {
    socket: String,
}
impl TmuxServer {
    fn start() -> Self {
        let socket = format!("unipi-m2-test-{}", std::process::id());
        std::env::set_var(TMUX_SOCKET_ENV, &socket);
        let output = std::process::Command::new("tmux")
            .args([
                "-L",
                &socket,
                "new-session",
                "-d",
                "-s",
                "work",
                "-x",
                "100",
                "-y",
                "30",
            ])
            .output()
            .expect("start tmux server");
        assert!(output.status.success(), "tmux new-session failed");
        // Second window, split into two panes: 2 windows / 3 panes.
        for args in [
            vec!["new-window", "-t", "work", "-n", "second"],
            vec!["split-window", "-t", "work:1"],
            vec!["rename-window", "-t", "work:1", "w:::2:"],
            vec!["select-pane", "-t", "work:1.0"],
            vec!["select-pane", "-T", "t:"],
        ] {
            let output = std::process::Command::new("tmux")
                .args(["-L", &socket])
                .args(&args)
                .output()
                .expect("tmux window/pane setup");
            assert!(output.status.success(), "tmux setup failed: {args:?}");
        }
        TmuxServer { socket }
    }
}
impl Drop for TmuxServer {
    fn drop(&mut self) {
        let _ = std::process::Command::new("tmux")
            .args(["-L", &self.socket, "kill-server"])
            .output();
        std::env::remove_var(TMUX_SOCKET_ENV);
    }
}

fn code(control: &Control) -> AppErrorCode {
    match control {
        Control::Err { error } => error.code,
        other => panic!("expected error, got {other:?}"),
    }
}

async fn request(conn: &iroh::endpoint::Connection, value: serde_json::Value) -> Control {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&value).unwrap(),
    ))
    .await
    .unwrap();
    send.finish().unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn hello(req: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type": "hello", "proto": "1.1.0", "req": req})
}

#[tokio::test]
async fn tmux_tree_attach_and_preview() {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tmux_tree_attach_and_preview_inner(),
    )
    .await;
    assert!(result.is_ok(), "tmux integration test timed out after 30 s");
}

async fn tmux_tree_attach_and_preview_inner() {
    let _env = ENV_LOCK.lock().await;
    let _server = TmuxServer::start();
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    let host = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let handler = Handler {
        paths: paths.clone(),
        host_id: host.id(),
        host_name: "Test host".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(vec!["default".to_string()]),
        board: Default::default(),
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
    };
    let router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler)
        .spawn();
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();

    // Pair first.
    let store = PairingStore::new(paths.pairing());
    let state = store.start(now_unix()).unwrap();
    assert!(matches!(
        request(
            &conn,
            hello(serde_json::json!({"op":"pair","code":state.code,"name":"Test phone"}))
        )
        .await,
        Control::Ok { .. }
    ));

    // Tree: one session, two windows, three panes, active flags set.
    let tree_reply = request(&conn, hello(serde_json::json!({"op":"tree"}))).await;
    let Control::Ok { data: Some(tree) } = tree_reply else {
        panic!("tree failed: {tree_reply:?}");
    };
    // herdr may or may not be available (other tests create real
    // sessions in parallel); only the tmux side is pinned here.
    assert_eq!(tree["tmux"]["available"], true);
    let sessions = tree["tmux"]["sessions"].as_array().unwrap();
    assert_eq!(sessions.len(), 1, "grouped sessions hidden from the tree");
    assert_eq!(sessions[0]["name"], "work");
    assert_eq!(sessions[0]["window_count"], 2);
    let windows = sessions[0]["windows"].as_array().unwrap();
    assert_eq!(windows.len(), 2);
    assert_eq!(windows[1]["panes"].as_array().unwrap().len(), 2);
    assert!(windows[0]["panes"][0]["active"].as_bool().unwrap());
    // Ambiguous names survive the U+241F separator exactly.
    assert_eq!(windows[1]["name"], "w:::2:");
    assert_eq!(windows[1]["panes"][0]["title"], "t:");

    // Protected tmux session name.
    let reply = request(
        &conn,
        hello(serde_json::json!({"op":"attach","target":{"mux":"tmux","session":"default","window_id":"@0"},"cols":80,"rows":24,"size_policy":"fit_phone"})),
    ).await;
    assert_eq!(code(&reply), AppErrorCode::E_UNAUTHORIZED);

    // Attach to window 2 (`@1`) with keep_pc. The attach consumes its
    // stream: Ok reply, then raw frames on the same stream.
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&hello(serde_json::json!({"op":"attach","target":{"mux":"tmux","session":"work","window_id":"@1"},"cols":100,"rows":30,"size_policy":"keep_pc"}))).unwrap(),
    ))
    .await
    .unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    let attach_reply = serde_json::from_slice::<Control>(&bytes).unwrap();
    let Control::Ok { data: Some(data) } = attach_reply else {
        panic!("attach failed: {attach_reply:?}");
    };
    assert_eq!(data["effective_size_policy"], "keep_pc");

    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    send.write_all(&unipi_proto::frame::encode(
        unipi_proto::FrameKind::Input,
        b"echo m2-ok\r",
    ))
    .await
    .unwrap();
    let mut collected = Vec::new();
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let (kind, bytes) = read_frame(&mut recv).await.unwrap();
            if kind == 1 {
                collected.extend(bytes);
                if String::from_utf8_lossy(&collected).contains("m2-ok") {
                    break;
                }
            }
        }
    })
    .await
    .expect("expected m2-ok in the attached pane output");

    // Preview the split panes of window 2 on a fresh stream.
    let preview_reply = request(
        &conn,
        hello(serde_json::json!({"op":"preview","target":{"mux":"tmux","session":"work","window_id":"@1"},"lines":100})),
    ).await;
    let Control::Ok {
        data: Some(preview),
    } = preview_reply
    else {
        panic!("preview failed: {preview_reply:?}");
    };
    assert!(
        preview["text"].as_str().unwrap().contains("m2-ok"),
        "preview should contain m2-ok"
    );

    // Detach: closing the stream ends the client; the grouped session
    // is killed by the host (destroy-unattached would fire too early:
    // the session is created detached).
    send.finish().unwrap();
    drop(recv);
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    let leftover_output = std::process::Command::new("tmux")
        .args(["-L", &_server.socket, "list-sessions"])
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default();
    let leftover = String::from_utf8_lossy(&leftover_output);
    assert!(
        !leftover.lines().any(|line| line.starts_with("unipi-")),
        "grouped sessions leaked: {leftover}"
    );

    DeviceStore::new(paths.devices())
        .revoke(&phone.id())
        .unwrap();
    conn.close(0u32.into(), b"done");
    phone.close().await;
    router.shutdown().await.unwrap();
}

/// Orphan sweep: a pre-created `unipi-x-1` with no client is killed
/// by `sweep_orphans`; attached sessions survive.
#[tokio::test]
async fn sweep_kills_orphaned_grouped_sessions() {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    let _env = ENV_LOCK.lock().await;
    let socket = format!("unipi-m2-sweep-{}", std::process::id());
    std::env::set_var(TMUX_SOCKET_ENV, &socket);
    let run = |args: &[&str]| {
        std::process::Command::new("tmux")
            .args(["-L", &socket])
            .args(args)
            .output()
            .unwrap()
            .stdout
    };
    assert!(run(&["new-session", "-d", "-s", "real"]).is_empty());
    assert!(run(&["new-session", "-d", "-s", "unipi-x-1"]).is_empty());
    // A live attached grouped session must survive. Attach through a
    // real PTY (a /dev/null stdin client would be refused).
    assert!(run(&[
        "new-session",
        "-d",
        "-s",
        "unipi-x-2",
        "-x",
        "80",
        "-y",
        "24"
    ])
    .is_empty());
    let attach_script = r#"
import os, pty, subprocess, time, fcntl, termios, struct
master, slave = pty.openpty()
fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 24, 80, 0, 0))
env = dict(os.environ); env["TERM"] = "xterm-256color"
child = subprocess.Popen(["tmux", "-L", os.environ["UNIPI_TMUX_SOCKET"], "attach-session", "-t", "unipi-x-2"], stdin=slave, stdout=slave, stderr=slave, env=env, start_new_session=True)
os.close(slave)
time.sleep(3)
child.terminate()
child.wait()
os.close(master)
"#;
    let attach_done = tokio::task::spawn_blocking(move || {
        let out = std::process::Command::new("python3")
            .arg("-c")
            .arg(attach_script)
            .output()
            .unwrap();
        eprintln!(
            "ATTACH SCRIPT status={} stderr={}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        );
    });
    // Wait until the PTY client is registered as attached.
    for _ in 0..40 {
        let out = run(&[
            "list-sessions",
            "-F",
            "#{session_name}:::#{session_attached}",
        ]);
        if String::from_utf8_lossy(&out).contains("unipi-x-2:::1") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let attached_bytes = run(&[
        "list-sessions",
        "-F",
        "#{session_name}:::#{session_attached}",
    ]);
    let attached = String::from_utf8_lossy(&attached_bytes);
    assert!(
        attached.contains("unipi-x-2:::1"),
        "client should be attached: {attached}"
    );

    eprintln!(
        "PRE-SWEEP: {}",
        String::from_utf8_lossy(&run(&[
            "list-sessions",
            "-F",
            "#{session_name}:::#{session_attached}"
        ]))
    );
    unipi_host::mux::tmux::sweep_orphans().await;

    unipi_host::mux::tmux::sweep_orphans().await;

    let sessions = String::from_utf8_lossy(&run(&["list-sessions", "-F", "#{session_name}"]))
        .lines()
        .map(str::to_string)
        .collect::<Vec<_>>();
    assert!(sessions.contains(&"real".to_string()));
    assert!(
        sessions.contains(&"unipi-x-2".to_string()),
        "attached grouped session must survive: {sessions:?}"
    );
    assert!(
        !sessions.contains(&"unipi-x-1".to_string()),
        "orphan must be swept: {sessions:?}"
    );

    let _ = attach_done.await;
    let _ = run(&["kill-server"]);
    std::env::remove_var(TMUX_SOCKET_ENV);
}

/// An attach whose client drops the stream abruptly mid-session must
/// still clean up its grouped session.
#[tokio::test]
async fn abrupt_stream_drop_cleans_up_grouped_session() {
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), abrupt_inner()).await;
    assert!(result.is_ok(), "abrupt-drop test timed out after 30 s");
}

async fn abrupt_inner() {
    let _env = ENV_LOCK.lock().await;
    let _server = TmuxServer::start();
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    let host = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .alpns(vec![ALPN.to_vec()])
        .bind()
        .await
        .unwrap();
    let handler = Handler {
        paths: paths.clone(),
        host_id: host.id(),
        host_name: "Test host".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(Vec::new()),
        board: Default::default(),
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
    };
    let router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler)
        .spawn();
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();

    let store = PairingStore::new(paths.pairing());
    let state = store.start(now_unix()).unwrap();
    let paired = request(
        &conn,
        hello(serde_json::json!({"op":"pair","code":state.code,"name":"Test phone"})),
    )
    .await;
    assert!(matches!(paired, Control::Ok { .. }));

    // Attach, then drop the whole connection abruptly mid-stream.
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&hello(serde_json::json!({"op":"attach","target":{"mux":"tmux","session":"work"},"cols":80,"rows":24,"size_policy":"fit_phone"}))).unwrap(),
    )).await.unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    assert!(matches!(
        serde_json::from_slice::<Control>(&bytes).unwrap(),
        Control::Ok { .. }
    ));
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    drop(send);
    drop(recv);
    conn.close(2u32.into(), b"abrupt");
    phone.close().await;

    // The host kills the grouped session even though the stream
    // errored. Poll briefly: kill_grouped runs after the PTY child
    // notices the closed pty master.
    let socket = std::env::var(TMUX_SOCKET_ENV).unwrap();
    for _ in 0..40 {
        let out = std::process::Command::new("tmux")
            .args(["-L", &socket, "list-sessions", "-F", "#{session_name}"])
            .output()
            .unwrap();
        let names = String::from_utf8_lossy(&out.stdout);
        if !names.lines().any(|line| line.starts_with("unipi-")) {
            router.shutdown().await.unwrap();
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    }
    let out = std::process::Command::new("tmux")
        .args(["-L", &socket, "list-sessions", "-F", "#{session_name}"])
        .output()
        .unwrap();
    panic!(
        "grouped session leaked after abrupt drop: {}",
        String::from_utf8_lossy(&out.stdout)
    );
}

/// DESIGN.md §5.6: tmux installed but no server running is an
/// available, empty tree (not "unavailable").
#[tokio::test]
async fn tmux_tree_without_server_is_available_and_empty() {
    let _lock = ENV_LOCK.lock().await;
    let socket = format!("unipi-m2-noserver-{}", std::process::id());
    std::env::set_var(TMUX_SOCKET_ENV, &socket);
    let state = unipi_host::mux::tmux::tree().await.expect("tree");
    std::env::remove_var(TMUX_SOCKET_ENV);
    assert!(state.available, "reason: {:?}", state.reason);
    assert_eq!(state.sessions.map(|s| s.len()), Some(0));
}
