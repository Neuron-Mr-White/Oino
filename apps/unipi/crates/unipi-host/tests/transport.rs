use iroh::{
    endpoint::{presets, RelayMode},
    Endpoint,
};
use unipi_host::{
    data_dir::HostPaths,
    devices::DeviceStore,
    endpoint::ALPN,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, Handler},
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
fn hello(req: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req})
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
        cron: unipi_host::cron_serve::CronHost::new(&paths),
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
    let list = hello(serde_json::json!({"op":"ports_list"}));
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
    bad["proto"] = serde_json::json!("3.0.0");
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
