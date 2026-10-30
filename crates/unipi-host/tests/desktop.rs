//! UNI-216 end to end with the synthetic `test_card` capture: a phone
//! endpoint talks to a host `Handler` over real iroh streams (temp identity
//! + data dir, never `~/.unipi`). Covers the gate, status, H.264 frames that
//! decode, acks/flow control, key-frame requests, quality and viewport
//! changes, input forwarding (and its absence in view-only mode), a second
//! viewer replacing the first, and the session log.
//!
//! Never touches the real screen. Real X11 capture is in `desktop_x11.rs`
//! (against a private Xvfb).
mod desktop_common;

use desktop_common::*;
use unipi_host::desktop::capture::{test_card, Input};
use unipi_proto::{AppErrorCode, Control, DesktopEvent, DesktopStatusData, PointerButton};

const OPEN: &str = r#"{"op":"desktop_open","viewport":{"width":1080,"height":2400}}"#;

fn open_req() -> serde_json::Value {
    serde_json::from_str(OPEN).unwrap()
}

/// One desktop stream per host process (a new viewer replaces the old one):
/// the tests in this file share that slot, so they must not overlap (UNI-271:
/// they flaked ~1 in 4 runs with "stream ended while waiting for config").
static ONE_AT_A_TIME: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desktop_gate_status_and_stream() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let rig = Rig::new().await;

    // Off by default: status explains, open refuses with the enable hint.
    let reply = request(&rig.conn, serde_json::json!({"op":"desktop_status"})).await;
    let Control::Ok { data: Some(data) } = reply else { panic!("{reply:?}") };
    let status: DesktopStatusData = serde_json::from_value(data).unwrap();
    assert!(!status.enabled);
    assert!(status.compiled.contains(&"x11".to_string()) || status.compiled.is_empty());
    let reply = rig.open(open_req()).await.err().unwrap();
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_DESKTOP_DISABLED), "{reply:?}");
    let Control::Err { error } = &reply else { unreachable!() };
    assert!(error.hint.as_deref().unwrap().contains("unipi-host desktop enable"));

    // Enabled for another device only.
    rig.config(serde_json::json!({"enabled":true,"devices":["Some other phone"],"backend":"test_card"}));
    let reply = rig.open(open_req()).await.err().unwrap();
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_DESKTOP_DISABLED));
    let reply = request(&rig.conn, serde_json::json!({"op":"desktop_status"})).await;
    assert_eq!(err_code(&reply), Some(AppErrorCode::E_DESKTOP_DISABLED), "status is gated per device once enabled");

    // Enabled for this phone (by name) with the test card.
    rig.config(serde_json::json!({"enabled":true,"devices":["Test phone"],"backend":"test_card"}));
    let reply = request(&rig.conn, serde_json::json!({"op":"desktop_status"})).await;
    let Control::Ok { data: Some(data) } = reply else { panic!("{reply:?}") };
    let status: DesktopStatusData = serde_json::from_value(data).unwrap();
    assert!(status.enabled && !status.view_only);
    assert_eq!(status.backend.as_deref(), Some("test_card"));
    assert!(status.features.iter().any(|f| f == "presets"), "UNI-271 presets advertised: {:?}", status.features);

    let _ = test_card::take_input_log();
    let mut viewer = rig.open(open_req()).await.unwrap_or_else(|c| panic!("{c:?}"));
    let started = viewer.until("started", |e, _| matches!(e, DesktopEvent::Started { .. })).await;
    let DesktopEvent::Started { backend, width, height, view_only, .. } = started else { unreachable!() };
    assert_eq!((backend.as_str(), width, height, view_only), ("test_card", 1280, 720, false));
    viewer.until("config", |e, _| matches!(e, DesktopEvent::Config { .. })).await;
    let (codec, cw, ch) = viewer.config.clone().unwrap();
    assert!(codec.starts_with("avc1.42e0"), "{codec}");
    // `auto` starts at 1600 max side → never upscaled past 1280×720.
    assert_eq!((cw, ch), (1280, 720));
    viewer.until("10 decoded frames", |_, v| v.decoded.len() >= 10).await;
    assert_eq!(viewer.keys, 1.max(viewer.keys));
    assert!(viewer.decoded.iter().all(|&d| d == (1280, 720)));
    // Colour bars survive the round trip: bar 0 light grey, bar 6 red.
    let grey = viewer.pixel(0.03, 0.4);
    assert!(grey.iter().all(|&c| (170..=230).contains(&c)), "grey bar {grey:?}");
    let red = viewer.pixel(6.5 / 8.0, 0.4);
    assert!(red[0] > 150 && red[1] < 70 && red[2] < 70, "red bar {red:?}");

    // Input reaches the capture backend, in order.
    viewer.send(serde_json::json!({"op":"pointer","x":0.25,"y":0.5})).await;
    viewer.send(serde_json::json!({"op":"button","button":"left","down":true})).await;
    viewer.send(serde_json::json!({"op":"button","button":"left","down":false})).await;
    viewer.send(serde_json::json!({"op":"scroll","dy":0.6})).await;
    viewer.send(serde_json::json!({"op":"scroll","dy":0.6})).await;
    viewer.send(serde_json::json!({"op":"key","keysym":0xffe3,"down":true})).await;
    viewer.send(serde_json::json!({"op":"key","keysym":0xffe3,"down":false})).await;
    viewer.send(serde_json::json!({"op":"text","text":"hi"})).await;
    // Bad input is dropped, not fatal.
    viewer.send(serde_json::json!({"op":"pointer","x":4,"y":0})).await;
    viewer.until("a few more frames", |_, v| v.decoded.len() >= 20).await;
    let log = test_card::take_input_log();
    assert_eq!(
        log,
        vec![
            Input::Pointer { x: 0.25, y: 0.5 },
            Input::Button { button: PointerButton::Left, down: true },
            Input::Button { button: PointerButton::Left, down: false },
            Input::Scroll { dx: 0, dy: 1 },
            Input::Key { keysym: 0xffe3, down: true },
            Input::Key { keysym: 0xffe3, down: false },
            Input::Key { keysym: 'h' as u32, down: true },
            Input::Key { keysym: 'h' as u32, down: false },
            Input::Key { keysym: 'i' as u32, down: true },
            Input::Key { keysym: 'i' as u32, down: false },
        ]
    );
    // The test card draws the pointer: a red crosshair at (0.25, 0.5).
    let cross = viewer.pixel(0.25, 0.5);
    assert!(cross[0] > 150 && cross[1] < 90, "crosshair {cross:?}");

    // Key frame on request.
    let keys = viewer.keys;
    viewer.send(serde_json::json!({"op":"keyframe"})).await;
    viewer.until("a requested key frame", |_, v| v.keys > keys).await;

    // UNI-271: legacy `auto` no longer shrinks below 1920 px for a small
    // viewport; `speed` follows the viewer's size → new config + key frame.
    viewer.send(serde_json::json!({"op":"viewport","viewport":{"width":640,"height":360}})).await;
    viewer.send(serde_json::json!({"op":"quality","quality":"speed"})).await;
    viewer.until("a smaller config", |e, _| matches!(e, DesktopEvent::Config { width: 640, height: 360, .. })).await;
    viewer.until("a 640×360 picture", |_, v| v.decoded.last() == Some(&(640, 360))).await;
    // `best` keeps the screen's own size whatever the viewport.
    viewer.send(serde_json::json!({"op":"quality","quality":"best"})).await;
    viewer.until("full size again", |e, _| matches!(e, DesktopEvent::Config { width: 1280, height: 720, .. })).await;
    viewer.until("a 1280×720 picture", |_, v| v.decoded.last() == Some(&(1280, 720))).await;
    viewer.send(serde_json::json!({"op":"quality","quality":"low"})).await;
    let stats = viewer.until("stats", |e, _| matches!(e, DesktopEvent::Stats { quality: unipi_proto::DesktopQuality::Low, .. })).await;
    let DesktopEvent::Stats { kbps, .. } = stats else { unreachable!() };
    assert!(kbps < 20_000, "{kbps} kbps");

    // A second viewer replaces the first.
    let mut second = rig.open(open_req()).await.unwrap_or_else(|c| panic!("{c:?}"));
    viewer.until("replaced", |e, _| matches!(e, DesktopEvent::Closed { reason } if reason == "replaced")).await;
    second.until("frames on the second viewer", |_, v| v.decoded.len() >= 3).await;
    drop(second);

    // View-only: input is ignored.
    rig.config(serde_json::json!({"enabled":true,"view_only":true,"backend":"test_card"}));
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let _ = test_card::take_input_log();
    let mut watcher = rig.open(open_req()).await.unwrap_or_else(|c| panic!("{c:?}"));
    let started = watcher.until("started", |e, _| matches!(e, DesktopEvent::Started { .. })).await;
    assert!(matches!(started, DesktopEvent::Started { view_only: true, .. }));
    watcher.send(serde_json::json!({"op":"pointer","x":0.9,"y":0.9})).await;
    watcher.send(serde_json::json!({"op":"key","keysym":97,"down":true})).await;
    watcher.until("frames", |_, v| v.decoded.len() >= 5).await;
    assert!(test_card::take_input_log().is_empty(), "view-only ignores input");
    // The phone may also ask for view-only itself.
    drop(watcher);

    // Session log: one connect line + one end line per connection.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    let log = std::fs::read_to_string(rig.paths.root.join("desktop.log")).unwrap();
    assert!(log.lines().filter(|l| l.contains(" connect")).count() >= 3, "{log}");
    assert!(log.contains("device=Test phone"), "{log}");
    assert!(log.contains("view_only=true"), "{log}");
    assert!(log.contains("ended after"), "{log}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn desktop_reports_unavailable_backends() {
    let _serial = ONE_AT_A_TIME.lock().await;
    let rig = Rig::new().await;
    // A backend this host can't run: x11 with no display reachable.
    rig.config(serde_json::json!({"enabled":true,"backend":"x11"}));
    // (Detection reads the host process env; with a $DISPLAY it would try
    // to connect — point it at a display that does not exist.)
    std::env::set_var("DISPLAY", ":250");
    // The gate passes (`ok`), then capture fails: one `err` ends the stream.
    let error = match rig.open(open_req()).await {
        Err(reply) => match reply {
            Control::Err { error } => error,
            other => panic!("{other:?}"),
        },
        Ok(mut viewer) => loop {
            match viewer.next().await {
                Next::Err(error) => break error,
                Next::Event(event) => assert!(!matches!(event, DesktopEvent::Started { .. }), "{event:?}"),
                Next::Eof => panic!("ended without an error"),
            }
        },
    };
    assert_eq!(error.code, AppErrorCode::E_DESKTOP_UNAVAILABLE, "{error:?}");
    let reason = error.detail.as_ref().and_then(|d| d["reason"].as_str()).unwrap().to_string();
    assert!(reason == "no_display" || reason == "not_compiled", "{reason}");
    assert!(error.hint.is_some());
    let log = std::fs::read_to_string(rig.paths.root.join("desktop.log")).unwrap_or_default();
    assert!(log.contains("refused"), "{log}");
}
