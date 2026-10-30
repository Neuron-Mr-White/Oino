//! UNI-253 end to end over real iroh (temp host dir, never `~/.unipi`): a
//! paired phone drops a shared feature again with `feature_leave` — for
//! itself or for everyone — and nothing is ever switched on by it. Two
//! phones are paired so one leaving can be seen not to disturb the other.
//! (v2 re-targets the flow from desktop streaming, which the host dropped,
//! to push — the leave mechanics under test are feature-independent.)
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
use unipi_proto::{AppErrorCode, Control, FeatureLeaveData, FeatureLeaveScope, PROTO_VERSION};

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
    request_as(conn, PROTO_VERSION, req).await
}

/// Push (like every feature) reports "off for you" instead of an error.
async fn push_enabled_for(conn: &Connection) -> bool {
    data::<serde_json::Value>(request(conn, serde_json::json!({"op":"push_list"})).await)["enabled"]
        .as_bool()
        .unwrap()
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

/// What the PC's config says (the owner's switch and allow-list).
fn push_config(host: &Host) -> (bool, Vec<String>) {
    let config = unipi_host::forward::HostConfig::load(&host.paths.config())
        .unwrap()
        .push;
    (config.enabled, config.devices)
}

async fn leave(
    conn: &Connection,
    feature: &str,
    scope: Option<FeatureLeaveScope>,
) -> FeatureLeaveData {
    let mut req = serde_json::json!({"op":"feature_leave","feature":feature});
    if let Some(scope) = scope {
        req["scope"] = serde_json::to_value(scope).unwrap();
    }
    data(request(conn, req).await)
}

struct Host {
    _temp: tempfile::TempDir,
    paths: HostPaths,
    _router: iroh::protocol::Router,
    _host: Endpoint,
    _phones: Vec<Endpoint>,
    a: Connection,
    b: Connection,
    a_id: String,
    b_id: String,
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
        host_name: "leave-test-pc".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron::Scheduler::new(paths.clone()),
    };
    let router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler)
        .spawn();
    let mut phones = Vec::new();
    let mut conns = Vec::new();
    for _ in 0..2 {
        let phone = Endpoint::builder(presets::N0)
            .relay_mode(RelayMode::Disabled)
            .clear_address_lookup()
            .bind()
            .await
            .unwrap();
        conns.push(phone.connect(host.addr(), ALPN).await.unwrap());
        phones.push(phone);
    }
    let [a, b]: [Connection; 2] = conns.try_into().unwrap();
    let ids: Vec<String> = phones.iter().map(|p| p.id().to_string()).collect();
    Host {
        _temp: temp,
        paths,
        _router: router,
        _host: host,
        _phones: phones,
        a,
        b,
        a_id: ids[0].clone(),
        b_id: ids[1].clone(),
    }
}

impl Host {
    /// Pairs `name` on the given connection (each pairing code is single-use).
    async fn pair(&self, conn: &Connection, name: &str) {
        let state = PairingStore::new(self.paths.pairing())
            .start(now_unix())
            .unwrap();
        let reply = request(
            conn,
            serde_json::json!({"op":"pair","code":state.code,"name":name}),
        )
        .await;
        assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    }

    /// Push on for the given devices (empty = every paired device).
    fn push_on(&self, devices: &[String]) {
        sharing::push_enable(&self.paths, Some(devices.to_vec())).unwrap();
    }
}

#[tokio::test]
async fn a_phone_leaves_and_the_other_phone_keeps_push() {
    let host = setup().await;
    host.pair(&host.a, "Pixel").await;
    host.pair(&host.b, "Tab").await;
    host.push_on(&[host.a_id.clone(), host.b_id.clone()]);

    // A leaves itself out: B is untouched and push stays on for it.
    let left = leave(&host.a, "push", Some(FeatureLeaveScope::OnlySelf)).await;
    assert!(left.left);
    assert!(left.now_enabled);
    assert_eq!(left.devices, vec![host.b_id.clone()]);
    assert_eq!(push_config(&host), (true, vec![host.b_id.clone()]));
    assert!(push_enabled_for(&host.b).await);
    // A is refused from now on ("off for you"); B still lists.
    assert!(!push_enabled_for(&host.a).await);
    // Asking again changes nothing and says so (`left` reflects reality).
    let again = leave(&host.a, "push", None).await;
    assert!(!again.left && again.now_enabled);
    assert_eq!(again.devices, vec![host.b_id.clone()]);

    // The last phone allowed leaves: off for everyone.
    let left = leave(&host.b, "push", None).await;
    assert!(left.left);
    assert!(!left.now_enabled);
    assert_eq!(left.devices, vec![host.b_id.clone()]);
    assert!(!push_config(&host).0);
}

#[tokio::test]
async fn leaving_everyone_from_a_phone_that_has_it() {
    let host = setup().await;
    host.pair(&host.a, "Pixel").await;
    host.pair(&host.b, "Tab").await;
    host.push_on(&[host.a_id.clone(), host.b_id.clone()]);

    let left = leave(&host.a, "push", Some(FeatureLeaveScope::All)).await;
    assert!(left.left);
    assert!(!left.now_enabled);
    // B loses it too: push is off on the PC.
    assert_eq!(
        push_config(&host),
        (false, vec![host.a_id.clone(), host.b_id.clone()])
    );
    assert!(!push_enabled_for(&host.a).await && !push_enabled_for(&host.b).await);
}

#[tokio::test]
async fn leaving_everyone_needs_the_feature() {
    let host = setup().await;
    host.pair(&host.a, "Pixel").await;
    host.pair(&host.b, "Tab").await;
    // On for B only.
    host.push_on(std::slice::from_ref(&host.b_id));

    // A doesn't have it → refused, and nothing changed on the PC.
    let refused = request(
        &host.a,
        serde_json::json!({"op":"feature_leave","feature":"push","scope":"all"}),
    )
    .await;
    assert_eq!(
        err_code(&refused),
        Some(AppErrorCode::E_UNAUTHORIZED),
        "{refused:?}"
    );
    assert_eq!(push_config(&host), (true, vec![host.b_id.clone()]));
    assert!(push_enabled_for(&host.b).await);

    // Off entirely → nobody may turn it off for everyone.
    sharing::disable(&host.paths, unipi_proto::FeatureKind::Push).unwrap();
    let refused = request(
        &host.b,
        serde_json::json!({"op":"feature_leave","feature":"push","scope":"all"}),
    )
    .await;
    assert_eq!(
        err_code(&refused),
        Some(AppErrorCode::E_UNAUTHORIZED),
        "{refused:?}"
    );
}

#[tokio::test]
async fn leaving_spells_out_the_paired_devices() {
    let host = setup().await;
    host.pair(&host.a, "Pixel").await;
    host.pair(&host.b, "Tab").await;
    // On for every paired device.
    host.push_on(&[]);
    assert_eq!(push_config(&host), (true, Vec::<String>::new()));
    assert!(push_enabled_for(&host.a).await && push_enabled_for(&host.b).await);

    let left = leave(&host.a, "push", None).await;
    assert!(left.left && left.now_enabled);
    // B stays allowed — the list isn't left empty (that would mean everyone).
    assert_eq!(left.devices, vec![host.b_id.clone()]);
    assert_eq!(push_config(&host), (true, vec![host.b_id.clone()]));
    assert!(push_enabled_for(&host.b).await);
    assert!(!push_enabled_for(&host.a).await);
    // A phone that pairs later is not on push any more.
    let late = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = late.connect(host._host.addr(), ALPN).await.unwrap();
    host.pair(&conn, "Later").await;
    assert!(!push_enabled_for(&conn).await);
}

#[tokio::test]
async fn leaving_never_switches_anything_on() {
    let host = setup().await;
    host.pair(&host.a, "Pixel").await;
    host.pair(&host.b, "Tab").await;
    // Off by default for every feature.
    for feature in ["forward", "browser", "push"] {
        let left = leave(&host.a, feature, None).await;
        assert!(!left.left, "{feature}: {left:?}");
    }
}
