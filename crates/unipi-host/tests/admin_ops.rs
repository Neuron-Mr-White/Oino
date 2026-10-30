//! UNI-232: local-only admin ops. Over `app.sock` this PC's app reads
//! host_info, starts a pairing, sees a REAL in-process iroh phone pair with
//! that code, lists and revokes the phone; the same ops over iroh (even from
//! the paired phone) answer E_UNAUTHORIZED. Temp dirs, no relay.
#![cfg(unix)]

use std::time::Duration;

use iroh::{endpoint::{presets, RelayMode}, Endpoint};
use tokio::io::AsyncWriteExt;
use unipi_host::{data_dir::HostPaths, endpoint::{EndpointCache, ALPN}, protocol::{read_frame, Handler}};
use unipi_proto::{frame::{encode, FrameKind}, AppErrorCode, Control, PROTO_VERSION};

fn hello(req: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"type":"hello","proto":PROTO_VERSION,"req":req})
}

async fn local(paths: &HostPaths, req: serde_json::Value) -> Control {
    let mut s = tokio::net::UnixStream::connect(paths.app_socket()).await.unwrap();
    s.write_all(&encode(FrameKind::Control, &serde_json::to_vec(&hello(req)).unwrap())).await.unwrap();
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(10), read_frame(&mut s)).await.unwrap().unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn over_iroh(conn: &iroh::endpoint::Connection, req: serde_json::Value) -> Control {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&encode(FrameKind::Control, &serde_json::to_vec(&hello(req)).unwrap())).await.unwrap();
    send.finish().unwrap();
    let (_, bytes) = read_frame(&mut recv).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn data(c: Control) -> serde_json::Value {
    match c {
        Control::Ok { data } => data.unwrap_or(serde_json::Value::Null),
        other => panic!("expected ok, got {other:?}"),
    }
}

fn code(c: &Control) -> Option<AppErrorCode> {
    match c {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

#[tokio::test]
async fn admin_ops_are_local_only_and_pairing_works_end_to_end() {
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    paths.ensure_dir().unwrap();
    let host = Endpoint::builder(presets::N0).relay_mode(RelayMode::Disabled).clear_address_lookup().alpns(vec![ALPN.to_vec()]).bind().await.unwrap();
    // serve publishes its address here; pair_begin reads it.
    unipi_host::data_dir::write_json(&paths.endpoint(), &EndpointCache::from_addr(host.addr())).unwrap();
    let handler = Handler {
        paths: paths.clone(),
        host_id: host.id(),
        host_name: "Test host".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        protected: std::sync::Arc::new(vec![]),
        board: Default::default(),
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
    };
    let local_srv = unipi_host::local::serve(handler.clone()).unwrap();
    let router = iroh::protocol::Router::builder(host.clone()).accept(ALPN, handler).spawn();

    // host_info over the local socket.
    let info = data(local(&paths, serde_json::json!({"op":"host_info"})).await);
    assert_eq!(info["host_id"], host.id().to_string());
    assert_eq!(info["host_name"], "Test host");
    assert_eq!(info["proto"], PROTO_VERSION);
    assert_eq!(info["devices"], 0);
    serde_json::from_value::<unipi_proto::HostInfoData>(info).unwrap();

    // pair_begin → a link + code; pair_status waiting.
    let begun = data(local(&paths, serde_json::json!({"op":"pair_begin","ttl_secs":120})).await);
    let pair_code = begun["code"].as_str().unwrap().to_string();
    assert!(begun["link"].as_str().unwrap().starts_with("unipi://pair?"));
    let st = data(local(&paths, serde_json::json!({"op":"pair_status","code":pair_code})).await);
    assert_eq!(st["state"], "waiting");

    // A real phone (iroh) pairs with that code.
    let phone = Endpoint::builder(presets::N0).relay_mode(RelayMode::Disabled).clear_address_lookup().bind().await.unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();
    // Before pairing, even admin ops say "pair first".
    assert_eq!(code(&over_iroh(&conn, serde_json::json!({"op":"host_info"})).await), Some(AppErrorCode::E_UNAUTHORIZED));
    assert!(matches!(over_iroh(&conn, serde_json::json!({"op":"pair","code":pair_code,"name":"Redmi"})).await, Control::Ok { .. }));

    let st = data(local(&paths, serde_json::json!({"op":"pair_status","code":pair_code})).await);
    assert_eq!(st, serde_json::json!({"state":"paired","device":"Redmi"}));

    // Paired phone still can't manage the host.
    for req in [serde_json::json!({"op":"host_info"}), serde_json::json!({"op":"devices_list"}), serde_json::json!({"op":"pair_begin"})] {
        let reply = over_iroh(&conn, req.clone()).await;
        assert_eq!(code(&reply), Some(AppErrorCode::E_UNAUTHORIZED), "{req} over iroh: {reply:?}");
    }
    // …but normal ops work for it.
    assert!(matches!(over_iroh(&conn, serde_json::json!({"op":"fs_roots"})).await, Control::Ok { .. }));

    // devices_list + revoke (by prefix) over the local socket.
    let list = data(local(&paths, serde_json::json!({"op":"devices_list"})).await);
    assert_eq!(list["devices"].as_array().unwrap().len(), 1);
    let id = list["devices"][0]["id"].as_str().unwrap().to_string();
    assert_eq!(id, phone.id().to_string());
    assert_eq!(code(&local(&paths, serde_json::json!({"op":"device_revoke","id":"00000000"})).await), Some(AppErrorCode::E_NOT_FOUND));
    data(local(&paths, serde_json::json!({"op":"device_revoke","id":&id[..12]})).await);
    assert_eq!(data(local(&paths, serde_json::json!({"op":"devices_list"})).await)["devices"], serde_json::json!([]));
    // Revoked: the phone is a stranger again.
    assert_eq!(code(&over_iroh(&conn, serde_json::json!({"op":"fs_roots"})).await), Some(AppErrorCode::E_UNAUTHORIZED));

    // Cancel: a fresh attempt closed by the dialog stops working at once.
    let again = data(local(&paths, serde_json::json!({"op":"pair_begin"})).await);
    let c2 = again["code"].as_str().unwrap();
    let st = data(local(&paths, serde_json::json!({"op":"pair_status","code":c2,"cancel":true})).await);
    assert_eq!(st, serde_json::json!({"state":"expired","reason":"cancelled"}));
    assert_eq!(code(&over_iroh(&conn, serde_json::json!({"op":"pair","code":c2,"name":"Late"})).await), Some(AppErrorCode::E_PAIRING_INVALID));
    // A code that was replaced by a newer attempt reads as replaced.
    let st = data(local(&paths, serde_json::json!({"op":"pair_status","code":pair_code})).await);
    assert_eq!(st["state"], "expired");

    local_srv.abort();
    router.shutdown().await.unwrap();
}
