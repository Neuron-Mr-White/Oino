//! Acceptance against a real Chromium (browser-control.md WP2). Runs only when
//! `OINO_TEST_BROWSER=/path/to/chrome` is set; otherwise every test returns early.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    clippy::print_stderr,
    clippy::case_sensitive_file_extension_comparisons,
    clippy::redundant_closure_for_method_calls,
    missing_docs,
    reason = "integration test"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use futures::StreamExt as _;
use oino_browser::config::EngineChoice;
use oino_browser::protocol::{
    BrowserCommand, BrowserResult, BrowserScope, BrowserViewEvent, FrontendRequest,
};
use oino_browser::{BrowserOptions, BrowserService, BrowserSettings};
use oino_extension_api::browser::{Approval, BrowserHost, BrowserRequest};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio_util::sync::CancellationToken;

fn browser() -> Option<PathBuf> {
    std::env::var_os("OINO_TEST_BROWSER").map(PathBuf::from)
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Minimal static HTTP server on `host:0`; returns the port.
async fn serve(host: &str) -> u16 {
    let listener = tokio::net::TcpListener::bind((host, 0)).await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            tokio::spawn(async move {
                let mut buf = vec![0u8; 8192];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let req = String::from_utf8_lossy(&buf[..n]);
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_owned();
                let file = path
                    .split('?')
                    .next()
                    .unwrap_or("/")
                    .trim_start_matches('/');
                let file = if file.is_empty() { "index.html" } else { file };
                let (status, body, extra) = match std::fs::read(fixtures().join(file)) {
                    Ok(b) if file.ends_with(".txt") => (
                        "200 OK",
                        b,
                        "Content-Type: text/plain\r\nContent-Disposition: attachment; filename=\"file.txt\"\r\n",
                    ),
                    Ok(b) => ("200 OK", b, "Content-Type: text/html; charset=utf-8\r\n"),
                    Err(_) => (
                        "404 Not Found",
                        b"nope".to_vec(),
                        "Content-Type: text/plain\r\n",
                    ),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\n{extra}Content-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    port
}

struct Rig {
    svc: BrowserService,
    scope: BrowserScope,
    _root: tempfile::TempDir,
    base: String,
}

impl Rig {
    async fn new(exe: PathBuf) -> Self {
        let root = tempfile::tempdir().unwrap();
        let settings = BrowserSettings {
            engine: EngineChoice::Path,
            executable_path: Some(exe),
            max_tabs: 4,
            ..BrowserSettings::default()
        };
        let svc = BrowserService::new(BrowserOptions {
            root: root.path().to_path_buf(),
            settings,
            ephemeral: false,
        });
        let a = serve("127.0.0.1").await;
        let b = serve("localhost").await;
        let xo = format!("http://localhost:{b}/xo.html");
        Self {
            svc,
            scope: BrowserScope::session("sess-1", root.path().display().to_string()),
            _root: root,
            base: format!("http://127.0.0.1:{a}/index.html?xo={}", urlencode(&xo)),
        }
    }

    async fn run(&self, tool: &str, args: Value) -> BrowserResult {
        self.run_approved(tool, args, Approval::None).await
    }

    async fn run_approved(&self, tool: &str, args: Value, approval: Approval) -> BrowserResult {
        let command = BrowserCommand::from_tool(tool, &args).unwrap();
        let r = self
            .svc
            .execute(
                self.scope.clone(),
                BrowserRequest { command, approval },
                CancellationToken::new(),
            )
            .await;
        eprintln!("── {tool} {args}\n{}\n", r.text);
        r
    }

    async fn ok(&self, tool: &str, args: Value) -> BrowserResult {
        let r = self.run(tool, args).await;
        assert!(!r.is_error, "{tool} failed: {}", r.text);
        r
    }

    fn ref_of(snapshot: &str, needle: &str) -> String {
        let line = snapshot
            .lines()
            .find(|l| l.contains(needle) && l.contains("[ref="))
            .unwrap_or_else(|| panic!("no ref for {needle} in\n{snapshot}"));
        let start = line.find("[ref=").unwrap() + 5;
        line[start..].split(']').next().unwrap().to_owned()
    }
}

fn urlencode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn drives_a_real_browser() {
    let Some(exe) = browser() else {
        eprintln!("OINO_TEST_BROWSER not set; skipping");
        return;
    };
    let rig = Rig::new(exe).await;

    // navigate + snapshot (shadow DOM, same-origin iframe, cross-origin OOPIF).
    let r = rig.ok("browser_navigate", json!({"url": rig.base})).await;
    assert!(r.text.contains("Opened t"), "{}", r.text);
    let d = r.details.browser.clone().unwrap();
    assert_eq!(d.tab.as_deref(), Some("t1"));
    assert!(!d.url.unwrap().contains('?'), "details URL is redacted");
    tokio::time::sleep(Duration::from_millis(800)).await;
    let snap = rig.ok("browser_snapshot", json!({})).await.text;
    assert!(snap.starts_with("# t1 · Fixture Home"), "{snap}");
    assert!(snap.contains("heading \"Fixture Home\" [level=1]"));
    assert!(
        snap.contains("button \"Shadow button\" [ref="),
        "shadow DOM: {snap}"
    );
    assert!(
        snap.contains("button \"Frame button\" [ref="),
        "same-origin frame: {snap}"
    );
    assert!(snap.contains("button \"XO button\" [ref="), "OOPIF: {snap}");
    assert!(snap.contains("••••"), "password masked");
    assert!(!snap.contains("secret123"));

    // reuse: same host ⇒ same tab.
    let r = rig
        .ok(
            "browser_navigate",
            json!({"url": rig.base.replace("index.html", "second.html"), "snapshot": false}),
        )
        .await;
    assert!(r.text.contains("Opened t1"), "reused: {}", r.text);
    let r = rig.ok("browser_navigate", json!({"history": "back"})).await;
    assert!(r.text.contains("Fixture Home"), "{}", r.text);
    let snap = rig.ok("browser_snapshot", json!({})).await.text;

    // click + events, animation (waits until stable), obscured, disabled.
    let count = Rig::ref_of(&snap, "button \"Count\"");
    let r = rig
        .ok("browser_click", json!({"ref": count, "snapshot": true}))
        .await;
    assert!(
        r.text.contains("paragraph: clicked 1") || r.text.contains("clicked 1"),
        "{}",
        r.text
    );
    let mover = Rig::ref_of(&snap, "button \"Moving\"");
    rig.ok("browser_click", json!({"ref": mover})).await;
    let under = Rig::ref_of(&snap, "button \"Hidden target\"");
    let r = rig.run("browser_click", json!({"ref": under})).await;
    assert_eq!(r.code().map(|c| c.as_str()), Some("obscured"), "{}", r.text);
    assert!(
        r.text.contains("Cookie banner") || r.text.contains("div"),
        "{}",
        r.text
    );
    let dis = Rig::ref_of(&snap, "button \"Disabled button\"");
    let r = rig.run("browser_click", json!({"ref": dis})).await;
    assert!(r.is_error);

    // frames: click inside the OOPIF.
    let xo = Rig::ref_of(&snap, "button \"XO button\"");
    rig.ok("browser_click", json!({"ref": xo})).await;
    let fb = Rig::ref_of(&snap, "button \"Frame button\"");
    rig.ok("browser_click", json!({"ref": fb})).await;
    let snap2 = rig.ok("browser_snapshot", json!({})).await.text;
    assert!(snap2.contains("XO done"), "{snap2}");
    assert!(snap2.contains("Frame done"), "{snap2}");

    // typing: input (clear), textarea, contenteditable, focused element, submit-less.
    let email = Rig::ref_of(&snap2, "textbox \"Email\"");
    rig.ok("browser_type", json!({"ref": email, "text": "new@x.dev"}))
        .await;
    let notes = Rig::ref_of(&snap2, "textbox \"Notes\"");
    rig.ok("browser_type", json!({"ref": notes, "text": "line one"}))
        .await;
    let rich = Rig::ref_of(&snap2, "textbox \"Rich editor\"");
    rig.ok("browser_type", json!({"ref": rich, "text": "rich text"}))
        .await;
    rig.ok("browser_press_key", json!({"key": "End"})).await;
    let values = rig
        .ok("browser_evaluate", json!({"function": "() => [document.getElementById('email').value, document.getElementById('notes').value, document.getElementById('rich').textContent]"}))
        .await;
    assert!(
        values.text.contains("new@x.dev")
            && values.text.contains("line one")
            && values.text.contains("rich text"),
        "{}",
        values.text
    );
    assert!(!values.text.contains("old@x.dev"));
    let xo_input = Rig::ref_of(&snap2, "textbox \"Card number\"");
    rig.ok("browser_type", json!({"ref": xo_input, "text": "4242"}))
        .await;

    // select + checkbox.
    let color = Rig::ref_of(&snap2, "combobox \"Color\"");
    let r = rig
        .ok(
            "browser_select_option",
            json!({"ref": color, "values": ["Blue"]}),
        )
        .await;
    assert!(r.text.contains("Blue"));
    let agree = Rig::ref_of(&snap2, "checkbox \"I agree\"");
    rig.ok("browser_click", json!({"ref": agree})).await;
    let snap3 = rig.ok("browser_snapshot", json!({})).await.text;
    assert!(
        snap3.contains("checkbox \"I agree\"") && snap3.contains("[checked]"),
        "{snap3}"
    );

    // upload needs approval.
    let up = Rig::ref_of(&snap3, "Upload");
    let file = fixtures().join("file.txt");
    let r = rig
        .run("browser_file_upload", json!({"ref": up, "paths": [file]}))
        .await;
    assert_eq!(r.code().map(|c| c.as_str()), Some("approval_required"));
    let r = rig
        .run_approved(
            "browser_file_upload",
            json!({"ref": up, "paths": [file]}),
            Approval::Once,
        )
        .await;
    assert!(!r.is_error, "{}", r.text);
    rig.ok("browser_wait_for", json!({"text": "file file.txt"}))
        .await;

    // wait_for text appearing later.
    let late = Rig::ref_of(&snap3, "button \"Later\"");
    rig.ok("browser_click", json!({"ref": late})).await;
    rig.ok(
        "browser_wait_for",
        json!({"text": "Late text arrived", "timeout_ms": 3000}),
    )
    .await;
    let r = rig
        .run(
            "browser_wait_for",
            json!({"text": "never there", "timeout_ms": 300}),
        )
        .await;
    assert_eq!(r.code().map(|c| c.as_str()), Some("timeout"));

    // dialog: blocks driving, then handled.
    let alert = Rig::ref_of(&snap3, "button \"Alert me\"");
    let r = rig.ok("browser_click", json!({"ref": alert})).await;
    assert!(r.text.contains("alert dialog opened"), "{}", r.text);
    let r = rig.run("browser_click", json!({"ref": count})).await;
    assert_eq!(r.code().map(|c| c.as_str()), Some("dialog_open"));
    rig.ok("browser_handle_dialog", json!({"accept": true}))
        .await;

    // errors and console logs.
    let throw = Rig::ref_of(&snap3, "button \"Throw\"");
    let r = rig.ok("browser_click", json!({"ref": throw})).await;
    assert!(r.text.contains("page error"), "{}", r.text);
    let r = rig.ok("browser_logs", json!({"kind": "errors"})).await;
    assert!(r.text.contains("boom from fixture"), "{}", r.text);
    let r = rig
        .ok(
            "browser_logs",
            json!({"kind": "console", "filter": "count"}),
        )
        .await;
    assert!(r.text.contains("count 1"), "{}", r.text);
    let r = rig
        .ok(
            "browser_logs",
            json!({"kind": "network", "filter": "second.html"}),
        )
        .await;
    assert!(r.text.contains("200"), "{}", r.text);

    // download.
    let get = Rig::ref_of(&snap3, "link \"Get file\"");
    rig.ok("browser_click", json!({"ref": get})).await;
    let r = rig
        .ok(
            "browser_downloads",
            json!({"action": "wait", "timeout_ms": 10000}),
        )
        .await;
    assert!(r.text.contains("file.txt"), "{}", r.text);

    // popup reported, then tabs list.
    let popup = Rig::ref_of(&snap3, "link \"Popup page\"");
    let r = rig.ok("browser_click", json!({"ref": popup})).await;
    assert!(r.text.contains("(popup)"), "popup reported: {}", r.text);
    let r = rig.ok("browser_tabs", json!({"action": "list"})).await;
    assert!(r.text.contains("t2"), "{}", r.text);

    // screenshot: image block + saved path.
    let r = rig
        .ok("browser_take_screenshot", json!({"tab": "t1"}))
        .await;
    let img = r.image.clone().unwrap();
    assert!(img.data.len() * 4 / 3 <= 200 * 1024 + 4);
    assert!(Path::new(img.path.as_deref().unwrap()).is_file());

    // stale ref after navigation.
    rig.ok("browser_navigate", json!({"tab": "t1", "url": rig.base.replace("index.html", "second.html"), "snapshot": false})).await;
    let r = rig
        .run("browser_click", json!({"tab": "t1", "ref": count}))
        .await;
    assert!(
        matches!(
            r.code().map(|c| c.as_str()),
            Some("ref_not_found" | "ref_stale")
        ),
        "{}",
        r.text
    );

    // take-over: viewer input while the agent works refuses driving commands.
    let mut view = rig.svc.attach_view(Some("sess-1"), "t1", None).unwrap();
    let frame = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(e) = view.events.next().await {
            if let BrowserViewEvent::Frame { jpeg, .. } = e {
                return Some(jpeg.0.len());
            }
        }
        None
    })
    .await
    .unwrap();
    assert!(frame.unwrap() > 1000, "screencast frame");
    rig.svc
        .frontend(
            Some("sess-1"),
            "",
            FrontendRequest::Takeover {
                tab: "t1".into(),
                on: true,
            },
        )
        .await
        .unwrap();
    let r = rig
        .run("browser_navigate", json!({"tab": "t1", "url": rig.base}))
        .await;
    assert_eq!(
        r.code().map(|c| c.as_str()),
        Some("taken_over"),
        "{}",
        r.text
    );
    let r = rig.ok("browser_snapshot", json!({"tab": "t1"})).await;
    assert!(r.text.contains("Second"));
    rig.svc
        .frontend(
            Some("sess-1"),
            "",
            FrontendRequest::Takeover {
                tab: "t1".into(),
                on: false,
            },
        )
        .await
        .unwrap();
    rig.ok(
        "browser_navigate",
        json!({"tab": "t1", "url": rig.base, "snapshot": false}),
    )
    .await;

    // isolation: another session sees none of these tabs.
    let other = BrowserScope::session("sess-2", rig.scope.workspace.clone());
    let r = rig
        .svc
        .execute(
            other.clone(),
            BrowserRequest::from(
                BrowserCommand::from_tool("browser_snapshot", &json!({"tab": "t1"})).unwrap(),
            ),
            CancellationToken::new(),
        )
        .await;
    assert_eq!(
        r.code().map(|c| c.as_str()),
        Some("tab_not_found"),
        "{}",
        r.text
    );
    let r = rig
        .svc
        .execute(
            other,
            BrowserRequest::from(
                BrowserCommand::from_tool("browser_tabs", &json!({"action": "list"})).unwrap(),
            ),
            CancellationToken::new(),
        )
        .await;
    assert!(r.text.contains("No tabs"), "{}", r.text);

    // navigation policy.
    let r = rig
        .run("browser_navigate", json!({"url": "chrome://settings"}))
        .await;
    assert_eq!(r.code().map(|c| c.as_str()), Some("navigation_blocked"));

    // residency: max_tabs = 4 ⇒ opening more closes the oldest safe tab.
    drop(view);
    for _ in 0..4 {
        rig.ok("browser_tabs", json!({"action": "new"})).await;
    }
    let tabs = rig.svc.status(Some(&rig.scope)).tabs;
    assert!(tabs.len() <= 4, "{tabs:?}");

    // handoff answered by the user.
    let svc = rig.svc.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(500)).await;
        let tab = svc
            .status(Some(&BrowserScope::session("sess-1", "")))
            .tabs
            .iter()
            .find(|t| t.lifecycle.as_str() == "handoff")
            .map(|t| t.id.clone());
        if let Some(tab) = tab {
            let _ = svc
                .frontend(
                    Some("sess-1"),
                    "",
                    FrontendRequest::HandoffDone { tab, done: true },
                )
                .await;
        }
    });
    let r = rig
        .ok(
            "browser_handoff",
            json!({"message": "Solve the CAPTCHA", "timeout_s": 10}),
        )
        .await;
    assert!(r.text.contains("Handoff done"), "{}", r.text);

    // cleanup: no browser process survives shutdown.
    let pids = rig.svc.pids();
    assert_ne!(pids, Vec::<u32>::new());
    rig.svc.shutdown().await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    for pid in pids {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists() || is_zombie(pid),
            "browser {pid} still runs"
        );
    }
}

fn is_zombie(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat")).is_ok_and(|s| s.contains(") Z "))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn port_transport_and_drop_cleanup() {
    let Some(exe) = browser() else {
        return;
    };
    let root = tempfile::tempdir().unwrap();
    let svc = BrowserService::new(BrowserOptions {
        root: root.path().to_path_buf(),
        settings: BrowserSettings {
            engine: EngineChoice::Path,
            executable_path: Some(exe),
            transport: oino_browser::config::TransportChoice::Port,
            ..BrowserSettings::default()
        },
        ephemeral: false,
    });
    let scope = BrowserScope::session("s", root.path().display().to_string());
    let r = svc
        .execute(
            scope,
            BrowserRequest::from(
                BrowserCommand::from_tool("browser_navigate", &json!({"url": "about:blank"}))
                    .unwrap(),
            ),
            CancellationToken::new(),
        )
        .await;
    assert!(!r.is_error, "{}", r.text);
    let pids = svc.pids();
    drop(svc);
    tokio::time::sleep(Duration::from_millis(800)).await;
    for pid in pids {
        assert!(
            !Path::new(&format!("/proc/{pid}")).exists() || is_zombie(pid),
            "browser {pid} survived drop"
        );
    }
}
