//! M2 H3b integration: `mux_action` on a private tmux socket
//! (`UNIPI_TMUX_SOCKET`) and a throwaway herdr session
//! (`unipi-m1`, via the e2e helper). Never touches the user's
//! `default` sessions; `--protect` rejections are asserted.

use iroh::{
    endpoint::{presets, RelayMode},
    Endpoint,
};
use unipi_host::{
    data_dir::HostPaths,
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
        let socket = format!("unipi-m3-action-{}", std::process::id());
        std::env::set_var(TMUX_SOCKET_ENV, &socket);
        for args in [
            vec!["new-session", "-d", "-s", "work", "-x", "100", "-y", "30"],
            vec!["new-window", "-t", "work", "-n", "second"],
            vec!["split-window", "-t", "work:1"],
            vec!["select-window", "-t", "work:0"],
            vec!["select-pane", "-t", "work:0.0"],
        ] {
            let output = std::process::Command::new("tmux")
                .args(["-L", &socket])
                .args(&args)
                .output()
                .expect("tmux setup");
            assert!(output.status.success(), "tmux setup failed: {args:?}");
        }
        TmuxServer { socket }
    }

    fn tmux(&self, args: &[&str]) -> String {
        let output = std::process::Command::new("tmux")
            .args(["-L", &self.socket])
            .args(args)
            .output()
            .expect("tmux command");
        assert!(
            output.status.success(),
            "tmux {args:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
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

struct HerdrGuard;
impl HerdrGuard {
    fn create() -> Self {
        let script = concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/m1/e2e-herdr.py");
        let status = std::process::Command::new("python3")
            .arg(script)
            .status()
            .expect("run herdr helper");
        assert!(status.success());
        Self
    }
}
impl Drop for HerdrGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("python3")
            .args([
                concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/m1/e2e-herdr.py"),
                "cleanup",
            ])
            .status();
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

/// Spawns the host handler + a paired phone connection.
async fn setup(protected: &[&str]) -> iroh::endpoint::Connection {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    // Leak the temp dir: dropping it would delete the pairing store
    // the connection still uses.
    std::mem::forget(temp);
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
        protected: std::sync::Arc::new(protected.iter().map(|s| s.to_string()).collect()),
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
    assert!(matches!(
        request(
            &conn,
            hello(serde_json::json!({"op":"pair","code":state.code,"name":"Test phone"}))
        )
        .await,
        Control::Ok { .. }
    ));
    // The endpoint drivers must outlive `setup`, so leak them (the
    // test process is short-lived).
    std::mem::forget(router);
    std::mem::forget(host);
    std::mem::forget(phone);
    std::mem::forget(paths);
    conn
}

#[tokio::test]
async fn tmux_mux_action_session_and_nav() {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        tmux_mux_action_session_and_nav_inner(),
    )
    .await;
    assert!(result.is_ok(), "tmux mux_action test timed out after 30 s");
}

async fn tmux_mux_action_session_and_nav_inner() {
    let _env = ENV_LOCK.lock().await;
    let server = TmuxServer::start();
    let conn = setup(&["default"]).await;

    async fn act(
        conn: &iroh::endpoint::Connection,
        name: &str,
        extra: serde_json::Value,
    ) -> Control {
        let mut req = serde_json::json!({"op": "mux_action", "action": name});
        if let Some(map) = extra.as_object() {
            for (key, value) in map {
                req[key.as_str()] = value.clone();
            }
        }
        request(conn, hello(req)).await
    }

    // session_create → visible in list-sessions.
    let reply = act(
        &conn,
        "tmux_session_create",
        serde_json::json!({"name": "scratch", "cwd": "/tmp"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "session_create failed: {reply:?}"
    );
    let list = server.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(list.contains("scratch"), "scratch missing: {list}");

    // session_rename.
    let reply = act(
        &conn,
        "tmux_session_rename",
        serde_json::json!({"session": "scratch", "name": "renamed"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "session_rename failed: {reply:?}"
    );
    let list = server.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(list.contains("renamed") && !list.contains("scratch"));

    // window_next / window_prev cycle work's current window.
    assert_eq!(
        server.tmux(&["display-message", "-p", "-t", "work", "#{window_index}"]),
        "0"
    );
    let reply = act(&conn, "window_next", serde_json::json!({"session": "work"})).await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "window_next failed: {reply:?}"
    );
    assert_eq!(
        server.tmux(&["display-message", "-p", "-t", "work", "#{window_index}"]),
        "1"
    );
    let reply = act(&conn, "window_prev", serde_json::json!({"session": "work"})).await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "window_prev failed: {reply:?}"
    );
    assert_eq!(
        server.tmux(&["display-message", "-p", "-t", "work", "#{window_index}"]),
        "0"
    );

    // pane_next / pane_prev cycle the split window's two panes.
    server.tmux(&["select-window", "-t", "work:1"]);
    let before = server.tmux(&["display-message", "-p", "-t", "work", "#{pane_index}"]);
    let reply = act(
        &conn,
        "tmux_pane_next",
        serde_json::json!({"session": "work"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "pane_next failed: {reply:?}"
    );
    let after = server.tmux(&["display-message", "-p", "-t", "work", "#{pane_index}"]);
    assert_eq!(
        after,
        if before == "0" { "1" } else { "0" },
        "pane_next must move"
    );
    let reply = act(
        &conn,
        "tmux_pane_prev",
        serde_json::json!({"session": "work"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "pane_prev failed: {reply:?}"
    );
    assert_eq!(
        server.tmux(&["display-message", "-p", "-t", "work", "#{pane_index}"]),
        before
    );

    // Navigation acts on the phone's grouped session when one shares
    // the group: work's own window stays put.
    let grouped = unipi_host::mux::tmux::create_grouped("work", None, None)
        .await
        .unwrap();
    server.tmux(&["select-window", "-t", "work:0"]);
    let reply = act(&conn, "window_next", serde_json::json!({"session": "work"})).await;
    assert!(matches!(reply, Control::Ok { .. }));
    let grouped_index = server.tmux(&["display-message", "-p", "-t", &grouped, "#{window_index}"]);
    assert_eq!(grouped_index, "1", "grouped session must move");
    unipi_host::mux::tmux::kill_grouped(&grouped).await;

    // --protect: every action on a protected session is rejected, and
    // creating one with a protected name too.
    let reply = act(
        &conn,
        "window_next",
        serde_json::json!({"session": "default"}),
    )
    .await;
    assert_eq!(code(&reply), AppErrorCode::E_UNAUTHORIZED);
    let reply = act(
        &conn,
        "tmux_session_create",
        serde_json::json!({"name": "default"}),
    )
    .await;
    assert_eq!(code(&reply), AppErrorCode::E_UNAUTHORIZED);
    let reply = act(
        &conn,
        "tmux_session_kill",
        serde_json::json!({"session": "default"}),
    )
    .await;
    assert_eq!(code(&reply), AppErrorCode::E_UNAUTHORIZED);

    // session_kill removes the session.
    let reply = act(
        &conn,
        "tmux_session_kill",
        serde_json::json!({"session": "renamed"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "session_kill failed: {reply:?}"
    );
    let list = server.tmux(&["list-sessions", "-F", "#{session_name}"]);
    assert!(!list.contains("renamed"), "renamed must be gone: {list}");
}

#[tokio::test]
async fn herdr_mux_action_workspace_lifecycle() {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        herdr_mux_action_workspace_lifecycle_inner(),
    )
    .await;
    assert!(result.is_ok(), "herdr mux_action test timed out after 30 s");
}

async fn herdr_mux_action_workspace_lifecycle_inner() {
    let _guard = HerdrGuard::create();
    let conn = setup(&["default"]).await;

    async fn act(
        conn: &iroh::endpoint::Connection,
        name: &str,
        extra: serde_json::Value,
    ) -> Control {
        let mut req = serde_json::json!({"op": "mux_action", "action": name});
        if let Some(map) = extra.as_object() {
            for (key, value) in map {
                req[key.as_str()] = value.clone();
            }
        }
        request(conn, hello(req)).await
    }

    let tree_labels = async |conn: &iroh::endpoint::Connection| -> Vec<String> {
        let tree = request(conn, hello(serde_json::json!({"op": "tree"}))).await;
        let Control::Ok { data: Some(tree) } = tree else {
            panic!("tree failed: {tree:?}");
        };
        tree["herdr"]["sessions"]
            .as_array()
            .unwrap()
            .iter()
            .find(|s| s["name"] == "unipi-m1")
            .and_then(|s| s["workspaces"].as_array())
            .map(|workspaces| {
                workspaces
                    .iter()
                    .map(|w| w["label"].as_str().unwrap_or("").to_string())
                    .collect()
            })
            .unwrap_or_default()
    };

    // workspace_create (label + cwd).
    let reply = act(
        &conn,
        "workspace_create",
        serde_json::json!({"session": "unipi-m1", "label": "na", "cwd": "/tmp"}),
    )
    .await;
    let Control::Ok {
        data: Some(created),
    } = reply
    else {
        panic!("workspace_create failed: {reply:?}");
    };
    let labels = tree_labels(&conn).await;
    assert!(
        labels.contains(&"na".to_string()),
        "workspace missing after create: {labels:?}"
    );

    // The create reply carries the new workspace id.
    let workspace_id = created["workspace"]["workspace_id"]
        .as_str()
        .expect("workspace.create reply carries the workspace id")
        .to_string();

    // workspace_rename.
    let reply = act(&conn,
        "workspace_rename",
        serde_json::json!({"session": "unipi-m1", "workspace_id": workspace_id, "label": "renamed"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "workspace_rename failed: {reply:?}"
    );
    let labels = tree_labels(&conn).await;
    assert!(
        labels.contains(&"renamed".to_string()),
        "rename not reflected: {labels:?}"
    );

    // tab_next / herdr_pane_next succeed on the live session.
    let reply = act(
        &conn,
        "tab_next",
        serde_json::json!({"session": "unipi-m1"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "tab_next failed: {reply:?}"
    );
    let reply = act(
        &conn,
        "tab_prev",
        serde_json::json!({"session": "unipi-m1"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "tab_prev failed: {reply:?}"
    );
    let reply = act(
        &conn,
        "herdr_pane_next",
        serde_json::json!({"session": "unipi-m1"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "pane_next failed: {reply:?}"
    );
    let reply = act(
        &conn,
        "herdr_pane_prev",
        serde_json::json!({"session": "unipi-m1"}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "pane_prev failed: {reply:?}"
    );

    // workspace_close removes it.
    let reply = act(
        &conn,
        "workspace_close",
        serde_json::json!({"session": "unipi-m1", "workspace_id": workspace_id}),
    )
    .await;
    assert!(
        matches!(reply, Control::Ok { .. }),
        "workspace_close failed: {reply:?}"
    );
    let labels = tree_labels(&conn).await;
    assert!(
        !labels.contains(&"renamed".to_string()),
        "workspace still there: {labels:?}"
    );

    // --protect: every action on the protected `default` is rejected.
    for (name, extra) in [
        ("tab_next", serde_json::json!({"session": "default"})),
        (
            "workspace_create",
            serde_json::json!({"session": "default", "label": "x"}),
        ),
    ] {
        let reply = act(&conn, name, extra).await;
        assert_eq!(
            code(&reply),
            AppErrorCode::E_UNAUTHORIZED,
            "{name} must be rejected"
        );
    }
}
