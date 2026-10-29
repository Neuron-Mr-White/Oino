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
use unipi_proto::Control;
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
struct HerdrGuard;
impl HerdrGuard {
    fn create() -> Self {
        let status = std::process::Command::new("python3")
            .arg(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../docs/m1/e2e-herdr.py"
            ))
            .status()
            .unwrap();
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
fn snapshot_size() -> (u64, u64) {
    let output = std::process::Command::new("python3").arg("-c").arg(r#"
import os,json,socket,subprocess
E={k:v for k,v in os.environ.items() if not k.startswith('HERDR_')};E['TERM']='xterm-256color'
sessions=json.loads(subprocess.check_output(['herdr','session','list','--json'],env=E))['sessions']
p=next(s['socket_path'] for s in sessions if s['name']=='unipi-m1')
s=socket.socket(socket.AF_UNIX);s.connect(p);s.sendall(b'{"id":"test","method":"session.snapshot","params":{}}\n');buf=b''
while b'\n' not in buf:buf+=s.recv(65536)
a=json.loads(buf.split(b'\n')[0])['result']['snapshot']['layouts'][0]['area'];print(json.dumps([a['width'],a['height']]))
"#).output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let values: Vec<u64> = serde_json::from_slice(&output.stdout).unwrap();
    (values[0], values[1])
}
fn hello(req: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type":"hello","proto":"1.1.0","req":req})
}
fn code(control: Control) -> unipi_proto::AppErrorCode {
    match control {
        Control::Err { error } => error.code,
        _ => panic!("expected error"),
    }
}
#[tokio::test]
async fn pairing_auth_revoke_and_versions() {
    tracing_subscriber::fmt()
        .with_writer(std::sync::Mutex::new(std::io::stderr()))
        .with_max_level(tracing::Level::DEBUG)
        .init();
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
    let list = hello(serde_json::json!({"op":"list","mux":"herdr"}));
    assert_eq!(
        code(request(&conn, list.clone()).await),
        unipi_proto::AppErrorCode::E_UNAUTHORIZED
    );
    let store = PairingStore::new(paths.pairing());
    let state = store.start(now_unix()).unwrap();
    let pair = |c: &str| hello(serde_json::json!({"op":"pair","code":c,"name":"Test phone"}));
    assert_eq!(
        code(request(&conn, pair("WRONGCODE1")).await),
        unipi_proto::AppErrorCode::E_PAIRING_INVALID
    );
    assert!(matches!(
        request(&conn, pair(&state.code)).await,
        Control::Ok { .. }
    ));
    assert_eq!(
        code(request(&conn, pair(&state.code)).await),
        unipi_proto::AppErrorCode::E_PAIRING_INVALID
    );
    if std::process::Command::new("herdr")
        .arg("--version")
        .output()
        .is_ok()
    {
        let _session_guard = HerdrGuard::create();
        let pair_request = hello(
            serde_json::json!({"op":"attach","target":{"mux":"herdr","session":"unipi-m1","workspace_id":"w1"},"cols":100,"rows":30,"size_policy":"fit_phone"}),
        );
        let (mut send, mut recv) = conn.open_bi().await.unwrap();
        send.write_all(&unipi_proto::frame::encode(
            unipi_proto::FrameKind::Control,
            &serde_json::to_vec(&pair_request).unwrap(),
        ))
        .await
        .unwrap();
        let (_, bytes) = read_frame(&mut recv).await.unwrap();
        let reply = serde_json::from_slice::<Control>(&bytes).unwrap();
        if let Control::Err { error } = &reply {
            panic!("attach errored: {error:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
        send.write_all(&unipi_proto::frame::encode(
            unipi_proto::FrameKind::Input,
            b"echo m1-ok\r",
        ))
        .await
        .unwrap();
        let mut collected = Vec::new();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let (kind, bytes) = read_frame(&mut recv).await.unwrap();
                if kind == 1 {
                    collected.extend(bytes);
                    if String::from_utf8_lossy(&collected).contains("m1-ok") {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap();
        let before = snapshot_size();
        let resize = Control::Resize {
            cols: 120_i64.try_into().unwrap(),
            rows: 40_i64.try_into().unwrap(),
        };
        write_control(&mut send, &resize).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        let after = snapshot_size();
        assert_ne!(before, after);
        send.write_all(&unipi_proto::frame::encode(
            unipi_proto::FrameKind::Input,
            b"stty size\r",
        ))
        .await
        .unwrap();
        let mut dimensions = String::new();
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            loop {
                let (kind, bytes) = read_frame(&mut recv).await.unwrap();
                if kind == 1 {
                    dimensions.push_str(&String::from_utf8_lossy(&bytes));
                    if dimensions.contains(&after.1.to_string())
                        && dimensions.contains(&(after.0 - 1).to_string())
                    {
                        break;
                    }
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("Expected resized pane dimensions: {dimensions:?}"));
        send.finish().unwrap();
        drop(recv);
        let tree_reply = request(&conn, hello(serde_json::json!({"op":"tree"}))).await;
        assert!(
            matches!(tree_reply, Control::Ok { .. }),
            "tree failed: {tree_reply:?}"
        );
        let response = request(&conn, list.clone()).await;
        if let Control::Ok { data: Some(data) } = response {
            assert!(data["sessions"]
                .as_array()
                .unwrap()
                .iter()
                .any(|s| s["name"] == "unipi-m1" && s["running"] == true));
        } else {
            panic!("list failed");
        }
    }
    let mut expired = store.start(now_unix()).unwrap();
    expired.expires_at = 0;
    unipi_host::data_dir::write_json(&paths.pairing(), &expired).unwrap();
    assert_eq!(
        code(request(&conn, pair(&expired.code)).await),
        unipi_proto::AppErrorCode::E_PAIRING_INVALID
    );
    let pending = store.start(now_unix()).unwrap();
    for _ in 0..5 {
        assert_eq!(
            code(request(&conn, pair("WRONGCODE1")).await),
            unipi_proto::AppErrorCode::E_PAIRING_INVALID
        );
    }
    assert_eq!(
        code(request(&conn, pair(&pending.code)).await),
        unipi_proto::AppErrorCode::E_PAIRING_INVALID
    );
    let mut bad = list.clone();
    bad["proto"] = serde_json::json!("2.0.0");
    assert_eq!(
        code(request(&conn, bad).await),
        unipi_proto::AppErrorCode::E_PROTO_VERSION
    );
    DeviceStore::new(paths.devices())
        .revoke(&phone.id())
        .unwrap();
    assert_eq!(
        code(request(&conn, list).await),
        unipi_proto::AppErrorCode::E_UNAUTHORIZED
    );
    conn.close(0u32.into(), b"done");
    phone.close().await;
    router.shutdown().await.unwrap();
}
