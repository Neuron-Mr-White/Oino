//! UNI-215 end to end: a phone endpoint talks to a host `Handler` over real
//! iroh streams (temp identity + data dir, never `~/.unipi`), and the host
//! launches its OWN headless Chromium-family browser (temp profile) against
//! a local `python3 -m http.server` page. We get JPEG frames, click a button
//! by coordinates and see the DOM change (the page retitles itself, which
//! arrives as a `page` event), type text, use history, open/close tabs, and
//! shut the browser down.
//!
//! Skips (passes with a note) when no browser or python3 is installed.
//! `UNIPI_TEST_BROWSER=/path/to/chrome` picks a specific binary.
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
};
use unipi_proto::{AppErrorCode, BrowserEvent, BrowserOpenData, BrowserStatusData, Control};

fn hello(req: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"hello","proto":unipi_proto::PROTO_VERSION,"req":req});
    unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&value).unwrap(),
    )
}

fn cmd(cmd: serde_json::Value) -> Vec<u8> {
    let value = serde_json::json!({"type":"browser_cmd","cmd":cmd});
    unipi_proto::frame::encode(
        unipi_proto::FrameKind::Control,
        &serde_json::to_vec(&value).unwrap(),
    )
}

async fn request(conn: &Connection, req: serde_json::Value) -> Control {
    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(req)).await.unwrap();
    send.finish().unwrap();
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(60), read_frame(&mut recv))
        .await
        .unwrap()
        .unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

fn err_code(control: &Control) -> Option<AppErrorCode> {
    match control {
        Control::Err { error } => Some(error.code),
        _ => None,
    }
}

struct Child(std::process::Child);
impl Drop for Child {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

const PAGE: &str = r#"<!doctype html><html><head><title>start</title>
<meta name="viewport" content="width=device-width, initial-scale=1">
<style>body{margin:0;font:20px sans-serif} button{position:absolute;left:20px;top:20px;width:200px;height:80px}
input{position:absolute;left:20px;top:140px;width:250px;height:40px}</style></head>
<body><button id="b" onclick="document.title='clicked-'+(++window.n||(window.n=1));document.body.style.background='#f00'">Click me</button>
<input id="i" oninput="document.title='typed:'+this.value">
<a id="next" href="/two.html" style="position:absolute;left:20px;top:220px">next</a></body></html>"#;
const PAGE2: &str = "<!doctype html><title>two</title><p>second page</p>";
// UNI-293: counts mousemove/keydown so the humanized relay can be checked
// against a real browser (more than one mousemove before a click lands,
// one keydown per typed character).
const COUNTER_PAGE: &str = r#"<!doctype html><title>counter</title>
<body style="margin:0"><button id="b" style="position:absolute;left:300px;top:300px;width:120px;height:50px">click</button>
<input id="i" style="position:absolute;left:20px;top:20px;width:250px;height:40px">
<script>
window.moves = 0; window.downs = 0;
addEventListener('mousemove', () => window.moves++);
addEventListener('keydown', () => window.downs++);
</script></body>"#;

/// Reads host → phone frames until `want` matches (returns it) or timeout.
/// Acks every frame like the phone does; counts JPEG bytes seen.
struct Viewer {
    send: iroh::endpoint::SendStream,
    recv: iroh::endpoint::RecvStream,
    frames: Vec<Vec<u8>>,
    last_page: Option<(String, String, bool, bool)>,
}

impl Viewer {
    async fn until(
        &mut self,
        what: &str,
        mut want: impl FnMut(&BrowserEvent, &Self) -> bool,
    ) -> BrowserEvent {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        loop {
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            let (kind, bytes) = tokio::time::timeout(left, read_frame(&mut self.recv))
                .await
                .unwrap_or_else(|_| {
                    panic!("timed out waiting for {what} (page {:?})", self.last_page)
                })
                .unwrap();
            assert_eq!(kind, 0, "expected a control frame");
            let control: Control = serde_json::from_slice(&bytes).unwrap();
            let Control::BrowserEvent { event } = control else {
                panic!("unexpected {control:?}")
            };
            if let BrowserEvent::Frame(meta) = &event {
                let (kind, jpeg) = read_frame(&mut self.recv).await.unwrap();
                assert_eq!(kind, 1, "a frame event is followed by the JPEG");
                assert_eq!(&jpeg[..3], &[0xFF, 0xD8, 0xFF], "JPEG magic");
                self.frames.push(jpeg);
                self.send
                    .write_all(&cmd(serde_json::json!({"op":"ack","seq":meta.seq})))
                    .await
                    .unwrap();
            }
            if let BrowserEvent::Page {
                url,
                title,
                can_back,
                can_forward,
                ..
            } = &event
            {
                self.last_page = Some((url.clone(), title.clone(), *can_back, *can_forward));
            }
            if want(&event, self) {
                return event;
            }
        }
    }

    async fn send(&mut self, value: serde_json::Value) {
        self.send.write_all(&cmd(value)).await.unwrap();
    }

    async fn title_is(&mut self, title: &str) {
        let t = title.to_string();
        self.until(
            &format!("title {title}"),
            move |e, _| matches!(e, BrowserEvent::Page { title, .. } if *title == t),
        )
        .await;
    }

    async fn click(&mut self, x: f64, y: f64) {
        for kind in ["pressed", "released"] {
            self.send(serde_json::json!({"op":"mouse","kind":kind,"x":x,"y":y,"button":"left","clicks":1})).await;
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn remote_browser_streams_frames_and_takes_input() {
    let config_path = std::env::var("UNIPI_TEST_BROWSER").ok();
    let browser =
        unipi_host::browser::detect(config_path.as_deref(), std::env::var_os("PATH").as_deref());
    let Some(browser) = browser else {
        eprintln!("SKIP: no Chromium-family browser on PATH");
        return;
    };
    if std::process::Command::new("python3")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("SKIP: no python3");
        return;
    }
    eprintln!("using {} at {}", browser.name, browser.path);
    let _ = tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_test_writer()
        .try_init();

    // The page.
    let www = tempfile::tempdir().unwrap();
    std::fs::write(www.path().join("index.html"), PAGE).unwrap();
    std::fs::write(www.path().join("two.html"), PAGE2).unwrap();
    std::fs::write(www.path().join("counter.html"), COUNTER_PAGE).unwrap();
    let port = free_port();
    let _server = Child(
        std::process::Command::new("python3")
            .args([
                "-m",
                "http.server",
                &port.to_string(),
                "--bind",
                "127.0.0.1",
            ])
            .current_dir(www.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // Host + phone over iroh, temp data dir. The profile the host actually
    // uses depends on the browser (UNI-287: a snap gets ~/snap/<name>/common).
    let temp = tempfile::tempdir().unwrap();
    let paths = HostPaths::from_dir(temp.path());
    // Even if an assertion fails: never leave the test's browser running.
    struct KillOnDrop(std::path::PathBuf);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            unipi_host::browser::kill_profile_processes(&self.0);
        }
    }
    // UNI-292: the default profile (Chrome engine here; CloakBrowser is the default engine).
    let home = unipi_host::fs::home_dir().ok();
    let profile = unipi_host::browser::profiles::chrome_dir(
        std::path::Path::new(&browser.path),
        &paths.root,
        "default",
        home.as_deref(),
    );
    let _cleanup = KillOnDrop(profile.clone());
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
        host_name: "Browser test host".into(),
        lock: std::sync::Arc::new(tokio::sync::Mutex::new(())),
        cron: unipi_host::cron_serve::CronHost::new(&paths),
    };
    let _router = iroh::protocol::Router::builder(host.clone())
        .accept(ALPN, handler)
        .spawn();
    let phone = Endpoint::builder(presets::N0)
        .relay_mode(RelayMode::Disabled)
        .clear_address_lookup()
        .bind()
        .await
        .unwrap();
    let conn = phone.connect(host.addr(), ALPN).await.unwrap();
    let state = PairingStore::new(paths.pairing())
        .start(now_unix())
        .unwrap();
    let reply = request(
        &conn,
        serde_json::json!({"op":"pair","code":state.code,"name":"Test phone"}),
    )
    .await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");

    let viewport = serde_json::json!({"width":412,"height":700,"scale":2,"mobile":true});
    let url = format!("http://127.0.0.1:{port}/index.html");

    // Off by default: status explains, open refuses with the enable hint.
    let reply = request(&conn, serde_json::json!({"op":"browser_status"})).await;
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    let status: BrowserStatusData = serde_json::from_value(data).unwrap();
    assert!(!status.enabled && !status.running);
    let reply = request(
        &conn,
        serde_json::json!({"op":"browser_open","viewport":viewport}),
    )
    .await;
    assert_eq!(
        err_code(&reply),
        Some(AppErrorCode::E_BROWSER_DISABLED),
        "{reply:?}"
    );

    // A missing configured binary → not_found with the install hint.
    let mut config = serde_json::json!({"browser":{"enabled":true,"engine":"chrome","path":"/nonexistent/chrome"}});
    std::fs::write(paths.config(), config.to_string()).unwrap();
    let reply = request(
        &conn,
        serde_json::json!({"op":"browser_open","viewport":viewport}),
    )
    .await;
    let Control::Err { error } = &reply else {
        panic!("{reply:?}")
    };
    assert_eq!(error.code, AppErrorCode::E_BROWSER_UNAVAILABLE);
    assert_eq!(error.detail.as_ref().unwrap()["reason"], "not_found");

    // Enabled (read fresh, no restart).
    config["browser"] =
        serde_json::json!({"enabled":true,"engine":"chrome","path":browser.path,"idle_minutes":0});
    std::fs::write(paths.config(), config.to_string()).unwrap();

    let (mut send, mut recv) = conn.open_bi().await.unwrap();
    send.write_all(&hello(
        serde_json::json!({"op":"browser_open","viewport":viewport,"url":url}),
    ))
    .await
    .unwrap();
    let (_, bytes) = tokio::time::timeout(Duration::from_secs(60), read_frame(&mut recv))
        .await
        .unwrap()
        .unwrap();
    let reply: Control = serde_json::from_slice(&bytes).unwrap();
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    let open: BrowserOpenData = serde_json::from_value(data).unwrap();
    assert!(open.headless);
    assert_eq!(
        (
            open.engine.as_deref(),
            open.stealth,
            open.profile.as_deref()
        ),
        (Some("chrome"), Some(false), Some("default"))
    );
    eprintln!("open: {} {:?}", open.browser.name, open.browser.version);
    let mut v = Viewer {
        send,
        recv,
        frames: Vec::new(),
        last_page: None,
    };

    // Tabs, then the page, then frames sized to the phone.
    v.until("tabs", |e, _| matches!(e, BrowserEvent::Tabs { .. }))
        .await;
    v.title_is("start").await;
    let frame = v
        .until("a frame", |e, _| matches!(e, BrowserEvent::Frame(_)))
        .await;
    let BrowserEvent::Frame(meta) = frame else {
        unreachable!()
    };
    let before = v.frames.last().unwrap().clone();
    assert_eq!(meta.css_width.round() as u32, 412, "{meta:?}");
    // A screencast frame of a 412×700 viewport at DPR 2 is 824×1400.
    let (w, h) = jpeg_size(&before).unwrap();
    // 412×700 CSS px at DPR 2 → 824×1400 (some builds capture the first frame at 1×).
    assert!(
        (w, h) == (824, 1400) || (w, h) == (412, 700),
        "frame size {w}x{h}"
    );

    // Click the button: the DOM changes (title + red background).
    v.click(120.0, 60.0).await;
    v.title_is("clicked-1").await;
    let changed = v
        .until("a new frame", |e, _| matches!(e, BrowserEvent::Frame(_)))
        .await;
    assert!(matches!(changed, BrowserEvent::Frame(_)));
    assert_ne!(
        v.frames.last().unwrap(),
        &before,
        "frame changed after the click"
    );

    // Touch tap works too (mobile emulation).
    for kind in ["start", "end"] {
        let points = if kind == "start" {
            serde_json::json!([{"x":120,"y":60,"id":0}])
        } else {
            serde_json::json!([])
        };
        v.send(serde_json::json!({"op":"touch","kind":kind,"points":points}))
            .await;
    }
    v.title_is("clicked-2").await;

    // Focus the input, type with insertText + a special key.
    v.click(100.0, 160.0).await;
    v.send(serde_json::json!({"op":"text","text":"héllo"}))
        .await;
    v.title_is("typed:héllo").await;
    v.send(serde_json::json!({"op":"key","kind":"press","key":"Backspace","code":"Backspace"}))
        .await;
    v.title_is("typed:héll").await;

    // Navigate + history.
    v.send(serde_json::json!({"op":"navigate","url":format!("127.0.0.1:{port}/two.html")}))
        .await;
    v.title_is("two").await;
    assert!(v.last_page.as_ref().unwrap().2, "can go back");
    v.send(serde_json::json!({"op":"back"})).await;
    v.until("back to start", |e, _| matches!(e, BrowserEvent::Page { url, can_forward: true, .. } if url.ends_with("/index.html"))).await;
    v.send(serde_json::json!({"op":"forward"})).await;
    v.title_is("two").await;
    v.send(serde_json::json!({"op":"reload"})).await;
    v.until("reload", |e, _| {
        matches!(e, BrowserEvent::Page { loading: true, .. })
    })
    .await;

    // Rotate: the viewport follows, frames get the new size.
    v.send(serde_json::json!({"op":"viewport","viewport":{"width":700,"height":412,"scale":1,"mobile":true}})).await;
    v.until(
        "landscape frame",
        |e, _| matches!(e, BrowserEvent::Frame(m) if m.css_width.round() as u32 == 700),
    )
    .await;

    // Tabs: new, switch, close.
    v.send(serde_json::json!({"op":"tab_new","url":url})).await;
    let mut seen_start = false;
    let tabs = v
        .until("two tabs", |e, _| {
            seen_start |= matches!(e, BrowserEvent::Page { title, .. } if title == "start");
            matches!(e, BrowserEvent::Tabs { tabs, .. } if tabs.len() == 2)
        })
        .await;
    let BrowserEvent::Tabs { tabs, active } = tabs else {
        unreachable!()
    };
    let first = tabs.iter().find(|t| t.id != active).unwrap().id.clone();
    if !seen_start {
        v.title_is("start").await;
    }
    v.send(serde_json::json!({"op":"tab_close","id":active}))
        .await;
    v.until("back to one tab", |e, _| matches!(e, BrowserEvent::Tabs { tabs, active } if tabs.len() == 1 && *active == first)).await;

    // Status shows it running with its tab.
    let reply = request(&conn, serde_json::json!({"op":"browser_status"})).await;
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    let status: BrowserStatusData = serde_json::from_value(data).unwrap();
    assert!(status.running && status.enabled);
    assert_eq!(status.tabs.len(), 1);

    // UNI-296: an agent through the take-over aware endpoint, against the real browser.
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let manager = unipi_host::browser::Manager::for_paths(&paths);
        let running = manager.current().await.expect("running");
        let gate_ws = running.gate_ws().expect("the agent endpoint is up");
        assert_ne!(gate_ws, running.ws_url);
        let (mut agent, _) = tokio_tungstenite::connect_async(&gate_ws).await.unwrap();
        async fn call(
            agent: &mut tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            id: u64,
            method: &str,
            params: serde_json::Value,
            session: Option<&str>,
        ) -> serde_json::Value {
            let mut msg = serde_json::json!({"id":id,"method":method,"params":params});
            if let Some(s) = session {
                msg["sessionId"] = s.into();
            }
            agent
                .send(Message::Text(msg.to_string().into()))
                .await
                .unwrap();
            loop {
                let Some(Ok(Message::Text(text))) =
                    tokio::time::timeout(Duration::from_secs(20), agent.next())
                        .await
                        .unwrap()
                else {
                    panic!("agent socket closed")
                };
                let v: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
                if v["id"] == id {
                    return v;
                }
            }
        }
        let version = call(
            &mut agent,
            1,
            "Browser.getVersion",
            serde_json::json!({}),
            None,
        )
        .await;
        assert!(version["result"]["product"].is_string(), "{version}");
        let target = status_tabs(&conn).await[0].id.clone();
        let attached = call(
            &mut agent,
            2,
            "Target.attachToTarget",
            serde_json::json!({"targetId":target,"flatten":true}),
            None,
        )
        .await;
        let session = attached["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_string();
        // The viewer hears that an agent connected / drives.
        let key = serde_json::json!({"type":"keyDown","key":"Shift"});
        let ok = call(
            &mut agent,
            3,
            "Input.dispatchKeyEvent",
            key.clone(),
            Some(&session),
        )
        .await;
        assert!(ok.get("error").is_none(), "{ok}");
        v.until(
            "agent driving",
            |e, _| matches!(e, BrowserEvent::Agent { agent } if agent.connected && agent.driving),
        )
        .await;
        // Take over from the app: the agent's input is refused with the plain message; reads pass.
        let reply = request(
            &conn,
            serde_json::json!({"op":"browser_takeover","on":true}),
        )
        .await;
        let Control::Ok { data: Some(data) } = reply else {
            panic!("{reply:?}")
        };
        assert_eq!(data["taken_over"], true);
        v.until(
            "taken over",
            |e, _| matches!(e, BrowserEvent::Agent { agent } if agent.taken_over && !agent.driving),
        )
        .await;
        let refused = call(
            &mut agent,
            4,
            "Input.dispatchKeyEvent",
            key.clone(),
            Some(&session),
        )
        .await;
        assert_eq!(
            refused["error"]["message"], "The user has taken over the browser; wait or ask them",
            "{refused}"
        );
        assert_eq!(refused["sessionId"], session.as_str());
        let nav = call(
            &mut agent,
            5,
            "Page.navigate",
            serde_json::json!({"url":"about:blank"}),
            Some(&session),
        )
        .await;
        assert!(
            nav["error"]["message"]
                .as_str()
                .unwrap()
                .contains("taken over"),
            "{nav}"
        );
        let read = call(
            &mut agent,
            6,
            "Runtime.evaluate",
            serde_json::json!({"expression":"document.title"}),
            Some(&session),
        )
        .await;
        assert!(
            read["result"]["result"]["value"].is_string(),
            "reads still work: {read}"
        );
        let status = request(&conn, serde_json::json!({"op":"browser_status"})).await;
        let Control::Ok { data: Some(data) } = status else {
            panic!()
        };
        assert_eq!(data["agent"]["taken_over"], true);
        assert_eq!(data["agent"]["connected"], true);
        // Give back: input flows again.
        let reply = request(
            &conn,
            serde_json::json!({"op":"browser_takeover","on":false}),
        )
        .await;
        assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
        let ok = call(
            &mut agent,
            7,
            "Input.dispatchKeyEvent",
            serde_json::json!({"type":"keyUp","key":"Shift"}),
            Some(&session),
        )
        .await;
        assert!(ok.get("error").is_none(), "{ok}");
        let _ = call(
            &mut agent,
            8,
            "Target.detachFromTarget",
            serde_json::json!({"sessionId":session}),
            None,
        )
        .await;
        agent.close(None).await.unwrap();
        drop(agent);
        v.until("agent gone", |e, _| matches!(e, BrowserEvent::Agent { agent } if !agent.connected && !agent.taken_over)).await;
        // Back on the page for what follows.
    }

    // UNI-293: the gate humanizes an agent's clicks and typing (default is on).
    {
        use futures_util::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        let manager = unipi_host::browser::Manager::for_paths(&paths);
        let running = manager.current().await.expect("running");
        let gate_ws = running.gate_ws().expect("the agent endpoint is up");
        let (mut agent, _) = tokio_tungstenite::connect_async(&gate_ws).await.unwrap();
        async fn call(
            agent: &mut tokio_tungstenite::WebSocketStream<
                tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
            >,
            id: u64,
            method: &str,
            params: serde_json::Value,
            session: Option<&str>,
        ) -> serde_json::Value {
            let mut msg = serde_json::json!({"id":id,"method":method,"params":params});
            if let Some(s) = session {
                msg["sessionId"] = s.into();
            }
            agent
                .send(Message::Text(msg.to_string().into()))
                .await
                .unwrap();
            loop {
                let Some(Ok(Message::Text(text))) =
                    tokio::time::timeout(Duration::from_secs(20), agent.next())
                        .await
                        .unwrap()
                else {
                    panic!("agent socket closed")
                };
                let v: serde_json::Value = serde_json::from_str(text.as_str()).unwrap();
                if v["id"] == id {
                    return v;
                }
            }
        }
        let target = status_tabs(&conn).await[0].id.clone();
        let attached = call(
            &mut agent,
            1,
            "Target.attachToTarget",
            serde_json::json!({"targetId":target,"flatten":true}),
            None,
        )
        .await;
        let session = attached["result"]["sessionId"]
            .as_str()
            .unwrap()
            .to_string();
        let nav_url = format!("http://127.0.0.1:{port}/counter.html");
        call(
            &mut agent,
            2,
            "Page.navigate",
            serde_json::json!({"url":nav_url}),
            Some(&session),
        )
        .await;
        v.title_is("counter").await;

        // Click the button: real coordinates are inside the button, several
        // mousemoves land before the press (a human path, not a teleport).
        let press = serde_json::json!({"type":"mousePressed","x":360.0,"y":325.0,"button":"left","clickCount":1});
        let release = serde_json::json!({"type":"mouseReleased","x":360.0,"y":325.0,"button":"left","clickCount":1});
        call(
            &mut agent,
            3,
            "Input.dispatchMouseEvent",
            press,
            Some(&session),
        )
        .await;
        call(
            &mut agent,
            4,
            "Input.dispatchMouseEvent",
            release,
            Some(&session),
        )
        .await;
        let moves = call(
            &mut agent,
            5,
            "Runtime.evaluate",
            serde_json::json!({"expression":"window.moves"}),
            Some(&session),
        )
        .await;
        let moves = moves["result"]["result"]["value"].as_u64().unwrap_or(0);
        assert!(
            moves > 5,
            "more than 5 mousemoves before the click: {moves}"
        );

        // Focus the input and type: Input.insertText becomes real key events.
        let focus = serde_json::json!({"expression":"document.getElementById('i').focus()"});
        call(&mut agent, 6, "Runtime.evaluate", focus, Some(&session)).await;
        call(
            &mut agent,
            7,
            "Input.insertText",
            serde_json::json!({"text":"hello"}),
            Some(&session),
        )
        .await;
        let downs = call(
            &mut agent,
            8,
            "Runtime.evaluate",
            serde_json::json!({"expression":"window.downs"}),
            Some(&session),
        )
        .await;
        let downs = downs["result"]["result"]["value"].as_u64().unwrap_or(0);
        // One keydown per typed character — plus, whenever the humanizer's
        // RNG simulates a mistype, one more for the wrong key and one for
        // the Backspace that fixes it (each mistype adds exactly 2). The
        // typed text itself is asserted next, through the input's value.
        assert!(
            downs >= 5 && (downs - 5) % 2 == 0,
            "keydowns must be one per character plus mistype+backspace pairs: {downs}"
        );
        let value = call(
            &mut agent,
            9,
            "Runtime.evaluate",
            serde_json::json!({"expression":"document.getElementById('i').value"}),
            Some(&session),
        )
        .await;
        assert_eq!(value["result"]["result"]["value"], "hello");

        // Browser.close from the agent must not kill the browser.
        let closed = call(&mut agent, 10, "Browser.close", serde_json::json!({}), None).await;
        assert_eq!(closed["result"], serde_json::json!({}));
        let _ = call(
            &mut agent,
            11,
            "Target.detachFromTarget",
            serde_json::json!({"sessionId":session}),
            None,
        )
        .await;
        agent.close(None).await.unwrap();
        drop(agent);
        // The real browser is still up: status still reports it running.
        let status = request(&conn, serde_json::json!({"op":"browser_status"})).await;
        let Control::Ok { data: Some(data) } = status else {
            panic!("{status:?}")
        };
        let status: BrowserStatusData = serde_json::from_value(data).unwrap();
        assert!(
            status.running,
            "Browser.close from an agent did not kill the browser"
        );

        // Back to the start page for what follows.
        v.send(serde_json::json!({"op":"navigate","url":url})).await;
        v.title_is("start").await;
    }

    // UNI-268: show the window on the PC. Only with a screen session (CI has none).
    if unipi_host::browser::can_show_window() && std::env::var_os("UNIPI_TEST_NO_WINDOW").is_none()
    {
        let headed_pids = || -> usize {
            let flag = format!("--user-data-dir={}", profile.display());
            std::fs::read_dir("/proc")
                .unwrap()
                .flatten()
                .filter(|e| {
                    let Ok(cmd) = std::fs::read(e.path().join("cmdline")) else {
                        return false;
                    };
                    let text = String::from_utf8_lossy(&cmd).replace('\0', " ");
                    unipi_host::browser::cmdline_has_arg(&cmd, &flag)
                        && !text.contains("--type=")
                        && !text.contains("--headless")
                })
                .count()
        };
        let tab_url = status_tabs(&conn).await[0].url.clone();
        // a) The phone's "Show on PC": saved + relaunched now, same tab, viewer told to reconnect.
        let reply = request(
            &conn,
            serde_json::json!({"op":"browser_window","show":true}),
        )
        .await;
        let Control::Ok { data: Some(data) } = reply else {
            panic!("{reply:?}")
        };
        let status: BrowserStatusData = serde_json::from_value(data).unwrap();
        assert!(
            status.running && !status.headless && status.can_show_window == Some(true),
            "{status:?}"
        );
        assert_eq!(
            status
                .tabs
                .iter()
                .map(|t| t.url.as_str())
                .collect::<Vec<_>>(),
            [tab_url.as_str()],
            "tabs reopened"
        );
        v.until(
            "relaunch",
            |e, _| matches!(e, BrowserEvent::Closed { reason } if reason == "relaunch"),
        )
        .await;
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
        assert_eq!(saved["browser"]["headless"], false);
        assert!(
            headed_pids() >= 1,
            "a headed browser process runs on our profile"
        );
        // A new viewer lands on the same page.
        let (send, recv) = conn.open_bi().await.unwrap();
        v = Viewer {
            send,
            recv,
            frames: Vec::new(),
            last_page: None,
        };
        v.send
            .write_all(&hello(
                serde_json::json!({"op":"browser_open","viewport":viewport}),
            ))
            .await
            .unwrap();
        let (_, bytes) = tokio::time::timeout(Duration::from_secs(60), read_frame(&mut v.recv))
            .await
            .unwrap()
            .unwrap();
        let Control::Ok { data: Some(data) } = serde_json::from_slice::<Control>(&bytes).unwrap()
        else {
            panic!()
        };
        let open: BrowserOpenData = serde_json::from_value(data).unwrap();
        assert!(!open.headless);
        let want = tab_url.clone();
        v.until(
            "same page",
            move |e, _| matches!(e, BrowserEvent::Page { url, .. } if *url == want),
        )
        .await;
        v.until("a headed frame", |e, _| matches!(e, BrowserEvent::Frame(_)))
            .await;
        // b) The owner's toggle (config.json edited by the desktop app / CLI): applied by the watcher.
        let mut config: serde_json::Value =
            serde_json::from_slice(&std::fs::read(paths.config()).unwrap()).unwrap();
        config["browser"]["headless"] = serde_json::json!(true);
        std::fs::write(paths.config(), config.to_string()).unwrap();
        v.until(
            "relaunch from config",
            |e, _| matches!(e, BrowserEvent::Closed { reason } if reason == "relaunch"),
        )
        .await;
        for _ in 0..100 {
            if headed_pids() == 0 && status_tabs(&conn).await.len() == 1 {
                break;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        assert_eq!(headed_pids(), 0, "back to headless, window gone");
        assert_eq!(status_tabs(&conn).await[0].url, tab_url);
        let (send, recv) = conn.open_bi().await.unwrap();
        v = Viewer {
            send,
            recv,
            frames: Vec::new(),
            last_page: None,
        };
        v.send
            .write_all(&hello(
                serde_json::json!({"op":"browser_open","viewport":viewport}),
            ))
            .await
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(60), read_frame(&mut v.recv))
            .await
            .unwrap()
            .unwrap();
        let want = tab_url.clone();
        v.until(
            "same page again",
            move |e, _| matches!(e, BrowserEvent::Page { url, .. } if *url == want),
        )
        .await;
    } else {
        eprintln!("no screen session: window toggle not exercised");
    }

    // Our own profile, never the user's.
    assert!(
        profile.join("Default").exists() || profile.exists(),
        "profile {}",
        profile.display()
    );

    // browser_close: the viewer is told, the browser quits.
    let reply = request(&conn, serde_json::json!({"op":"browser_close"})).await;
    assert!(matches!(reply, Control::Ok { .. }), "{reply:?}");
    v.until("closed", |e, _| matches!(e, BrowserEvent::Closed { .. }))
        .await;
    let reply = request(&conn, serde_json::json!({"op":"browser_status"})).await;
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    let status: BrowserStatusData = serde_json::from_value(data).unwrap();
    assert!(!status.running);
    eprintln!("frames received: {}", v.frames.len());
    let sizes: Vec<_> = v.frames.iter().filter_map(|f| jpeg_size(f)).collect();
    eprintln!("frame sizes: {:?}", sizes);
    // Nothing of ours is left running.
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        unipi_host::browser::kill_profile_processes(&profile),
        0,
        "browser left running"
    );
}

async fn status_tabs(conn: &Connection) -> Vec<unipi_proto::BrowserTab> {
    let reply = request(conn, serde_json::json!({"op":"browser_status"})).await;
    let Control::Ok { data: Some(data) } = reply else {
        panic!("{reply:?}")
    };
    serde_json::from_value::<BrowserStatusData>(data)
        .unwrap()
        .tabs
}

/// Width/height from a JPEG's SOF0/SOF2 marker.
fn jpeg_size(bytes: &[u8]) -> Option<(u32, u32)> {
    let mut i = 2;
    while i + 9 < bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let marker = bytes[i + 1];
        let len = u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]) as usize;
        if matches!(marker, 0xC0..=0xC2) {
            let h = u16::from_be_bytes([bytes[i + 5], bytes[i + 6]]) as u32;
            let w = u16::from_be_bytes([bytes[i + 7], bytes[i + 8]]) as u32;
            return Some((w, h));
        }
        i += 2 + len;
    }
    None
}
