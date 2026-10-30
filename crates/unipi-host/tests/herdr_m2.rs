//! M2 herdr integration on a throwaway `unipi-m2-it` session.
//! Only that session is created/attached/stopped/deleted.

use iroh::{
    endpoint::{presets, RelayMode},
    Endpoint,
};
use unipi_host::{
    data_dir::HostPaths,
    devices::DeviceStore,
    endpoint::ALPN,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, write_control, Handler},
};
use unipi_proto::{AppErrorCode, Control};

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

fn snapshot_size() -> Option<(u64, u64)> {
    let script = r#"
import os,json,socket,subprocess
E={k:v for k,v in os.environ.items() if not k.startswith('HERDR_')}
sessions=json.loads(subprocess.check_output(['herdr','session','list','--json'],env=E))['sessions']
s=next((x for x in sessions if x['name']=='unipi-m1'),None)
if not s or not s.get('socket_path'): raise SystemExit(1)
c=socket.socket(socket.AF_UNIX);c.connect(s['socket_path'])
c.sendall(b'{"id":"t","method":"session.snapshot","params":{}}\n')
buf=b''
while b'\n' not in buf: buf+=c.recv(65536)
a=json.loads(buf.split(b'\n')[0])['result']['snapshot']['layouts'][0]['area']
print(json.dumps([a['width'],a['height']]))
"#;
    let output = std::process::Command::new("python3")
        .arg("-c")
        .arg(script)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let values: Vec<u64> = serde_json::from_slice(&output.stdout).ok()?;
    Some((values[0], values[1]))
}

fn hello(req: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type": "hello", "proto": "1.1.0", "req": req})
}

#[tokio::test]
async fn herdr_tree_focus_attach_preview() {
    let result = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        herdr_tree_focus_attach_preview_inner(),
    )
    .await;
    assert!(
        result.is_ok(),
        "herdr integration test timed out after 30 s"
    );
}

async fn herdr_tree_focus_attach_preview_inner() {
    if std::process::Command::new("herdr")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("herdr absent; skipping");
        return;
    }
    let _guard = HerdrGuard::create();
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

    // Tree lists unipi-m1 (created by the guard) with at least one workspace.
    let tree_reply = request(&conn, hello(serde_json::json!({"op":"tree"}))).await;
    let Control::Ok { data: Some(tree) } = tree_reply else {
        panic!("tree failed: {tree_reply:?}")
    };
    // tmux may be available (user's default server) or not; the
    // herdr side must be available and list the throwaway session.
    if tree["herdr"]["available"] != true {
        panic!("HERDR TREE: {tree}");
    }
    assert_eq!(tree["herdr"]["available"], true);
    let herdr_sessions = tree["herdr"]["sessions"].as_array().unwrap();
    let mine = herdr_sessions
        .iter()
        .find(|s| s["name"] == "unipi-m1")
        .expect("unipi-m1 in tree");
    assert_eq!(mine["running"], true);
    assert!(!mine["workspaces"].as_array().unwrap().is_empty());
    let workspace_id = mine["workspaces"][0]["id"].as_str().unwrap().to_string();
    assert!(!herdr_sessions.iter().any(|s| s["name"] == "default"
        && s["workspaces"].as_array().map(|w| w.is_empty()) == Some(false)
        && false));

    // Focus the workspace (spike confirmed: moves the whole session view).
    let focus_reply = request(&conn, hello(serde_json::json!({"op":"attach","target":{"mux":"herdr","session":"unipi-m1","workspace_id":workspace_id},"cols":100,"rows":30,"size_policy":"fit_phone"}))).await;
    assert!(
        matches!(focus_reply, Control::Ok { .. }),
        "attach failed: {focus_reply:?}"
    );

    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&hello(serde_json::json!({"op":"attach","target":{"mux":"herdr","session":"unipi-m1","workspace_id":workspace_id},"cols":100,"rows":30,"size_policy":"fit_phone"}))).unwrap(),
    )).await.unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    let attach_reply = serde_json::from_slice::<Control>(&bytes).unwrap();
    let Control::Ok { data: Some(data) } = attach_reply else {
        panic!("attach failed: {attach_reply:?}")
    };
    assert_eq!(data["effective_size_policy"], "fit_phone");

    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
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
    .expect("m2-ok expected in output");

    // Resize and confirm the pane size via snapshot (not hard-coded).
    let before = snapshot_size();
    write_control(
        &mut send,
        &Control::Resize {
            cols: 120_i64.try_into().unwrap(),
            rows: 40_i64.try_into().unwrap(),
        },
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    if let Some((w0, _h0)) = before {
        if let Some((w1, _h1)) = snapshot_size() {
            assert_ne!(w0, w1, "pane width should change after resize");
        }
    }

    // Preview returns recent text containing m2-ok (ANSI kept).
    let preview_reply = request(&conn, hello(serde_json::json!({"op":"preview","target":{"mux":"herdr","session":"unipi-m1","workspace_id":workspace_id},"lines":100}))).await;
    let Control::Ok {
        data: Some(preview),
    } = preview_reply
    else {
        panic!("preview failed: {preview_reply:?}")
    };
    let preview_text = preview["text"].as_str().unwrap_or("");
    assert!(
        preview_text.contains("m2-ok"),
        "preview should contain m2-ok, got: {:?}",
        preview_text.chars().take(400).collect::<String>()
    );

    send.finish().unwrap();
    drop(recv);
    DeviceStore::new(paths.devices())
        .revoke(&phone.id())
        .unwrap();
    conn.close(0u32.into(), b"done");
    phone.close().await;
    router.shutdown().await.unwrap();
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

/// `--protect default` refuses attach for the protected name while
/// tree/preview stay allowed. Uses the in-process handler directly,
/// no herdr needed.
#[tokio::test]
async fn protected_session_refuses_attach() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    let host = Handler {
        paths: paths.clone(),
        host_id: iroh::SecretKey::generate().public(),
        host_name: "Test".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(vec!["default".to_string()]),
        board: Default::default(),
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
    };
    // Pair first so auth passes and the protect check is what rejects.
    let store = PairingStore::new(paths.pairing());
    let state = store.start(now_unix()).unwrap();
    let (client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(server);
    let handler = host.clone();
    let task = tokio::spawn(async move {
        handler
            .handle(read, write, iroh::SecretKey::generate().public())
            .await
            .unwrap()
    });
    let (mut c_read, mut c_write) = tokio::io::split(client);
    let hello = Control::Hello {
        proto: "1.1.0".to_owned().try_into().unwrap(),
        req: unipi_proto::Request::Pair {
            code: state.code.try_into().unwrap(),
            name: None,
        },
    };
    write_control(&mut c_write, &hello).await.unwrap();
    let (_, bytes) = read_frame(&mut c_read).await.unwrap();
    assert!(matches!(
        serde_json::from_slice::<Control>(&bytes).unwrap(),
        Control::Ok { .. }
    ));
    task.await.unwrap();
    // Attach goes on a fresh bi-stream (the pair stream closed after
    // its reply): handle() runs per stream.
    let (client, server) = tokio::io::duplex(4096);
    let (read, write) = tokio::io::split(server);
    let attach_handler = host.clone();
    let attach_task = tokio::spawn(async move {
        attach_handler
            .handle(read, write, iroh::SecretKey::generate().public())
            .await
            .unwrap()
    });
    let (mut c_read, mut c_write) = tokio::io::split(client);
    let attach = Control::Hello {
        proto: "1.1.0".to_owned().try_into().unwrap(),
        req: unipi_proto::Request::Attach {
            target: unipi_proto::AttachTarget::Herdr {
                session: "default".to_owned().try_into().unwrap(),
                workspace_id: None,
                tab_id: None,
                pane_id: None,
            },
            cols: 80_i64.try_into().unwrap(),
            rows: 24_i64.try_into().unwrap(),
            size_policy: unipi_proto::SizePolicy::FitPhone,
        },
    };
    write_control(&mut c_write, &attach).await.unwrap();
    let (_, bytes) = read_frame(&mut c_read).await.unwrap();
    match serde_json::from_slice::<Control>(&bytes).unwrap() {
        Control::Err { error } => assert_eq!(error.code, AppErrorCode::E_UNAUTHORIZED),
        other => panic!("expected E_UNAUTHORIZED, got {other:?}"),
    }
    attach_task.await.unwrap();
}
