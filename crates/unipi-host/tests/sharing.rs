//! UNI-248 end to end over real iroh (temp host dir, never `~/.unipi`): a
//! paired phone asks with `feature_request`, sees it with `feature_requests`,
//! the owner approves (the same `sharing::approve` the CLI runs) and the very
//! next `ports_list` passes — the host re-reads
//! config.json per request, no restart. Also: unpaired refused, bad scope
//! refused by the schema, deny, time-limited approval switching back off,
//! and an older-proto client still accepted.
use std::time::Duration;

use iroh::{
    endpoint::{presets, Connection, RelayMode},
    Endpoint,
};
use unipi_host::{
    data_dir::HostPaths,
    endpoint::ALPN,
    pairing::{now_unix, PairingStore},
    protocol::{read_frame, Handler},
    sharing,
};
use unipi_proto::{
    AppErrorCode, Control, FeatureRequestItem, FeatureRequestStatus, FeatureRequestsData,
};

fn hello(proto: &str, req: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"hello","proto":proto,"req":req});
    unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&value).unwrap(),
    )
}

async fn request_as(conn: &Connection, proto: &str, req: serde_json::Value) -> Control {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(proto, req)).await.unwrap();
    send.finish().unwrap();
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(20), read_frame(&mut recv))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

async fn request(conn: &Connection, req: serde_json::Value) -> Control {
    request_as(conn, unipi_proto::PROTO_VERSION, req).await
}

fn data<T: serde::de::DeserializeOwned>(reply: Control) -> T {
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    serde_json::from_value(data).unwrap()
}

fn err_code(reply: &Control) -> Option<AppErrorCode> {
    match reply {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

struct Host {
    _temp: tempfile::TempDir,
    paths: HostPaths,
    _router: iroh::protocol::Router,
    _phone: Endpoint,
    conn: Connection,
    phone_id: String,
}

async fn setup() -> Host {
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
        host_name: "share-test-pc".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
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
    let phone_id = phone.id().to_string();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();
    Host {
        _temp: temp,
        paths,
        _router: router,
        _phone: phone,
        conn,
        phone_id,
    }
}

async fn pair(host: &Host, name: &str) {
    let state = PairingStore::new(host.paths.pairing())
        .start(now_unix())
        .unwrap();
    let reply = request(
        &host.conn,
        serde_json::json!({"op":"pair","code":state.code,"name":name}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
}

#[tokio::test]
async fn ask_approve_and_use_without_restart() {
    let host = setup().await;
    let conn = &host.conn;

    // Unpaired: refused like every op.
    let reply = request(
        conn,
        serde_json::json!({"op":"feature_request","feature":"forward"}),
    )
    .await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_UNAUTHORIZED));
    pair(&host, "Pixel").await;

    // Off by default.
    let reply = request(conn, serde_json::json!({"op":"ports_list"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_FORWARD_DISABLED));
    let none: FeatureRequestsData =
        data(request(conn, serde_json::json!({"op":"feature_requests"})).await);
    assert!(none.requests.is_empty());

    // A bad scope is a schema error, nothing stored.
    let reply = request(
        conn,
        serde_json::json!({"op":"feature_request","feature":"forward","scope":{"ports":[0]}}),
    )
    .await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_SCHEMA));

    // Ask: pending, named after the paired device, never switched on.
    let item: FeatureRequestItem = data(
        request(conn, serde_json::json!({"op":"feature_request","feature":"forward","scope":{"ports":[5173]},"note":"  vite  "})).await,
    );
    assert_eq!(item.status, FeatureRequestStatus::Pending);
    assert_eq!(item.device_name, "Pixel");
    assert_eq!(item.note.as_deref(), Some("vite"));
    assert_eq!(
        err_code(&request(conn, serde_json::json!({"op":"ports_list"})).await),
        Some(AppErrorCode::E_FORWARD_DISABLED)
    );
    let mine: FeatureRequestsData =
        data(request(conn, serde_json::json!({"op":"feature_requests"})).await);
    assert_eq!(mine.requests.len(), 1);
    assert_eq!(mine.requests[0].id, item.id);

    // The owner approves (what `unipi-host requests approve` runs) →
    // the next request passes, no restart.
    sharing::approve(&host.paths, &item.id, None, unipi_host::push::now_ms()).unwrap();
    let ports = request(conn, serde_json::json!({"op":"ports_list"})).await;
    assert!(matches!(ports, Control::Ok { .. }), "{ports:?}");
    let mine: FeatureRequestsData =
        data(request(conn, serde_json::json!({"op":"feature_requests"})).await);
    assert_eq!(mine.requests[0].status, FeatureRequestStatus::Approved);
    let forwarding = unipi_host::forward::HostConfig::load(&host.paths.config())
        .unwrap()
        .forwarding;
    assert_eq!(forwarding.devices, vec![host.phone_id.clone()]);

    // Asking for something that is already on → approved at once.
    let again: FeatureRequestItem = data(
        request(
            conn,
            serde_json::json!({"op":"feature_request","feature":"forward"}),
        )
        .await,
    );
    assert_eq!(again.status, FeatureRequestStatus::Approved);

    // Browser: deny → the phone sees it, still off.
    let ask: FeatureRequestItem = data(
        request(
            conn,
            serde_json::json!({"op":"feature_request","feature":"browser"}),
        )
        .await,
    );
    sharing::deny(&host.paths, &ask.id, unipi_host::push::now_ms()).unwrap();
    let mine: FeatureRequestsData =
        data(request(conn, serde_json::json!({"op":"feature_requests"})).await);
    let ask_now = mine
        .requests
        .iter()
        .find(|r| r.feature == unipi_proto::FeatureKind::Browser)
        .unwrap();
    assert_eq!(ask_now.status, FeatureRequestStatus::Denied);
    let status = request(conn, serde_json::json!({"op":"browser_status"})).await;
    let Control::Ok { data: Some(status) } = status else {
        panic!()
    };
    assert_eq!(status["enabled"], false);

    // Push for a short while: on, then the sweep switches it off again.
    let push: FeatureRequestItem = data(
        request(
            conn,
            serde_json::json!({"op":"feature_request","feature":"push"}),
        )
        .await,
    );
    let now = unipi_host::push::now_ms();
    sharing::approve(&host.paths, &push.id, Some(now + 1_000), now).unwrap();
    let list = request(conn, serde_json::json!({"op":"push_list"})).await;
    let Control::Ok { data: Some(list) } = list else {
        panic!()
    };
    assert_eq!(list["enabled"], true);
    sharing::sweep(&host.paths, now + 1_000).unwrap();
    let list = request(conn, serde_json::json!({"op":"push_list"})).await;
    let Control::Ok { data: Some(list) } = list else {
        panic!()
    };
    assert_eq!(list["enabled"], false);
    let mine: FeatureRequestsData =
        data(request(conn, serde_json::json!({"op":"feature_requests"})).await);
    assert_eq!(
        mine.requests
            .iter()
            .find(|r| r.feature == unipi_proto::FeatureKind::Push)
            .unwrap()
            .status,
        FeatureRequestStatus::Expired
    );

    // v2 bumped the protocol major: a 1.x client is refused, a 2.x one talks.
    let old = request_as(conn, "1.10.0", serde_json::json!({"op":"ports_list"})).await;
    assert!(
        matches!(old, Control::Err { ref error } if error.code == unipi_proto::AppErrorCode::E_PROTO_VERSION),
        "{old:?}"
    );
    let same_major = request_as(conn, "2.0.1", serde_json::json!({"op":"ports_list"})).await;
    assert!(matches!(same_major, Control::Ok { .. }), "{same_major:?}");
}

/// Against a separately started test host (never the real one):
/// `UNIPI_HOST_DIR=/tmp/qa-248 unipi-host serve --relay-only`, then
/// `SHARE_HOST_DIR=/tmp/qa-248 SHARE_HOST_BIN=target/debug/unipi-host cargo test -p unipi-host --test sharing external_serve_ask -- --ignored --nocapture`.
/// A phone asks, the real CLI approves it (`requests approve --for 1h`), the
/// running serve honours it on the next request, and `forward disable` ends it.
#[tokio::test]
#[ignore]
async fn external_serve_ask() {
    let dir = std::env::var("SHARE_HOST_DIR").unwrap_or_else(|_| "/tmp/qa-248".into());
    assert!(dir.starts_with("/tmp/"), "a throwaway host dir only");
    let bin = std::env::var("SHARE_HOST_BIN").expect("SHARE_HOST_BIN=path/to/unipi-host");
    let paths = HostPaths::from_dir(&dir);
    let cli = |args: &[&str]| {
        let out = std::process::Command::new(&bin)
            .args(args)
            .env("UNIPI_HOST_DIR", &dir)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        println!("$ unipi-host {}\n{text}", args.join(" "));
        assert!(out.status.success(), "{text}");
        text
    };
    cli(&["push", "disable"]);
    let cache: unipi_host::endpoint::EndpointCache =
        unipi_host::data_dir::read_json(&paths.endpoint())
            .unwrap()
            .expect("serve running");
    let phone = Endpoint::builder(presets::N0).bind().await.unwrap();
    let conn = phone.connect(cache.addr.clone(), ALPN).await.unwrap();
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    assert!(matches!(
        request(
            &conn,
            serde_json::json!({"op":"pair","code":state.code,"name":"qa-phone"})
        )
        .await,
        Control::Ok { .. }
    ));
    let item: FeatureRequestItem = data(
        request(
            &conn,
            serde_json::json!({"op":"feature_request","feature":"push","note":"QA run"}),
        )
        .await,
    );
    assert_eq!(item.status, FeatureRequestStatus::Pending);
    assert!(cli(&["requests"]).contains(&item.id));
    cli(&["requests", "approve", &item.id, "--for", "1h"]);
    let status = request(&conn, serde_json::json!({"op":"push_list"})).await;
    let Control::Ok { data: Some(status) } = status else {
        panic!("{status:?}")
    };
    assert_eq!(status["enabled"], true);
    let mine: FeatureRequestsData =
        data(request(&conn, serde_json::json!({"op":"feature_requests"})).await);
    assert!(mine.requests[0].until.is_some());
    cli(&["push", "disable"]);
    let mine: FeatureRequestsData =
        data(request(&conn, serde_json::json!({"op":"feature_requests"})).await);
    assert_eq!(mine.requests[0].status, FeatureRequestStatus::Expired);
    // A second ask → deny via the CLI.
    let again: FeatureRequestItem = data(
        request(
            &conn,
            serde_json::json!({"op":"feature_request","feature":"browser"}),
        )
        .await,
    );
    cli(&["requests", "deny", &again.id]);
    let mine: FeatureRequestsData =
        data(request(&conn, serde_json::json!({"op":"feature_requests"})).await);
    assert_eq!(
        mine.requests
            .iter()
            .find(|r| r.id == again.id)
            .unwrap()
            .status,
        FeatureRequestStatus::Denied
    );
    cli(&["sharing", "status"]);
}
