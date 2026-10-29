//! The unipi-integrations components in the real wasm host: registration, and one end-to-end
//! behaviour per package (notify dispatch + finished/input-needed flows, MCP stdio server and
//! dynamic tools, web-api smart fetch + wigolo search over a local HTTP server, updater docs
//! browsers).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionId,
    ExtensionInstance, ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
    ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn extensions_dir() -> PathBuf {
    harness_dir().join("../../..").canonicalize().unwrap()
}

/// Builds the components once (release, wasm32-wasip2).
fn components() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(extensions_dir())
            .args(["build", "-j", "2", "--release", "--target", "wasm32-wasip2"])
            .args(["-p", "unipi-notify", "-p", "unipi-updater", "-p", "unipi-mcp", "-p", "unipi-web-api"]);
        for var in ["CARGO_TARGET_DIR", "RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS", "CARGO_BUILD_TARGET", "CARGO_MANIFEST_DIR"] {
            cmd.env_remove(var);
        }
        cmd.env("CARGO_TARGET_DIR", extensions_dir().join("target"));
        let out = cmd.output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        extensions_dir().join("target/wasm32-wasip2/release")
    })
}

struct Env {
    runtime: WasmRuntime,
    host: Arc<RecordingHostServices>,
    agent: tempfile::TempDir,
    project: tempfile::TempDir,
    forwarded: Mutex<usize>,
}

impl Env {
    fn new() -> Self {
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        // Everything goes through the recorder (timers arrive as `deliver_event`, bus as
        // `bus_publish`) so the tests can observe and pump them.
        options.local_services = false;
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: RecordingHostServices::new(),
            agent,
            project,
            forwarded: Mutex::new(0),
        }
    }

    fn cx(&self, kind: CallKind) -> CallContext {
        CallContext {
            session_id: "s1".into(),
            mode: Mode::Rpc,
            has_ui: true,
            cwd: self.project.path().to_path_buf(),
            kind,
            ..CallContext::default()
        }
    }

    async fn load(&self, dir: &str, wasm: &str, settings: Value) -> (Arc<dyn ExtensionInstance>, oino_extension_api::Registration) {
        let manifest = Manifest::parse(&std::fs::read_to_string(extensions_dir().join("unipi").join(dir).join("extension.json")).unwrap()).unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(components().join(wasm)),
            root: None,
            bundled: true,
        };
        let host: Arc<dyn HostServices> = Arc::clone(&self.host) as Arc<dyn HostServices>;
        let ext = self.runtime.load(package, host).await.unwrap();
        // The host serves the same values on later reads (`registry.extension-settings`).
        for (ns, values) in settings.as_object().into_iter().flatten() {
            for (key, value) in values.as_object().into_iter().flatten() {
                self.host.preset_extension_setting(
                    oino_extension_api::ops::SettingsScope::Global,
                    ns,
                    key,
                    value.clone(),
                );
            }
        }
        let input = InitInput {
            settings: RawJson::from(settings),
            ..InitInput::default()
        };
        let reg = ext.init(input, self.cx(CallKind::Init)).await.unwrap();
        (ext, reg)
    }

    async fn event(&self, ext: &Arc<dyn ExtensionInstance>, kind: EventKind, payload: Value) -> Option<Value> {
        ext.on_event(kind, RawJson::from(payload), self.cx(CallKind::Event))
            .await
            .unwrap()
            .map(|r| r.to_value())
    }

    async fn command(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, args: &str) {
        ext.run_command(name, args, CancellationToken::new(), self.cx(CallKind::Command))
            .await
            .unwrap();
    }

    async fn tool(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, args: Value) -> Value {
        let inv = ToolInvocation {
            tool_call_id: "call-1".into(),
            name: name.into(),
            args: RawJson::from(args),
            parent_tool_call_id: None,
        };
        let mut cx = self.cx(CallKind::Tool);
        cx.tool_call_id = Some("call-1".into());
        ext.run_tool(inv, CancellationToken::new(), cx).await.unwrap().to_value()
    }

    async fn tool_result(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, args: Value) -> Result<Value, String> {
        let inv = ToolInvocation {
            tool_call_id: "call-2".into(),
            name: name.into(),
            args: RawJson::from(args),
            parent_tool_call_id: None,
        };
        let mut cx = self.cx(CallKind::Tool);
        cx.tool_call_id = Some("call-2".into());
        ext.run_tool(inv, CancellationToken::new(), cx).await.map(|v| v.to_value()).map_err(|e| e.to_string())
    }

    fn details(&self, method: &str) -> Vec<Value> {
        self.host.calls_named(method).into_iter().map(|c| c.detail).collect()
    }

    fn fired(&self, method: &str) -> Vec<Value> {
        self.details("ui_fire").into_iter().filter(|d| d["method"] == method).collect()
    }

    fn published(&self, topic: &str) -> Vec<Value> {
        self.details("bus_publish").into_iter().filter(|d| d["topic"] == topic).map(|d| d["data"].clone()).collect()
    }

    /// Forwards host timer deliveries (`oino_timer`) with `prefix` to `ext`, waiting up to `ms`.
    async fn pump_timers(&self, ext: &Arc<dyn ExtensionInstance>, prefix: &str, ms: u64) -> usize {
        let deadline = tokio::time::Instant::now() + Duration::from_millis(ms);
        let mut fired = 0;
        loop {
            let all = self.details("deliver_event");
            let start = *self.forwarded.lock().unwrap();
            let fresh: Vec<Value> = all.iter().skip(start).cloned().collect();
            *self.forwarded.lock().unwrap() = all.len();
            for d in fresh {
                if d["kind"] == "oino_timer" && d["payload"]["timerId"].as_str().is_some_and(|t| t.starts_with(prefix)) {
                    fired += 1;
                    self.event(ext, EventKind::OinoTimer, d["payload"].clone()).await;
                }
            }
            if fired > 0 || tokio::time::Instant::now() > deadline {
                return fired;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

/// A tiny HTTP/1.1 server: `(port, recorded requests)`; `route(method, path, body)` →
/// `(status, content-type, body)`.
async fn http_server(route: fn(&str, &str, &str, u16) -> (u16, &'static str, String)) -> (u16, Arc<Mutex<Vec<(String, String, String, String)>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<(String, String, String, String)>>> = Arc::default();
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else { return };
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let mut buf = Vec::new();
                let mut chunk = [0u8; 8192];
                let (head, body) = loop {
                    let n = sock.read(&mut chunk).await.unwrap_or(0);
                    if n == 0 {
                        return;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    if let Some(pos) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                        let head = String::from_utf8_lossy(&buf[..pos]).into_owned();
                        let len = head
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap_or(0)))
                            .unwrap_or(0);
                        while buf.len() < pos + 4 + len {
                            let n = sock.read(&mut chunk).await.unwrap_or(0);
                            if n == 0 {
                                break;
                            }
                            buf.extend_from_slice(&chunk[..n]);
                        }
                        let body = String::from_utf8_lossy(&buf[pos + 4..]).into_owned();
                        break (head, body);
                    }
                };
                let mut first = head.lines().next().unwrap_or("").split_whitespace();
                let method = first.next().unwrap_or("").to_owned();
                let path = first.next().unwrap_or("").to_owned();
                log.lock().unwrap().push((method.clone(), path.clone(), head.clone(), body.clone()));
                let (status, ctype, text) = route(&method, &path, &body, port);
                let resp = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: {ctype}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{text}",
                    text.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
                let _ = sock.shutdown().await;
            });
        }
    });
    (port, seen)
}

fn names(reg: &oino_extension_api::Registration) -> (Vec<String>, Vec<(String, bool)>, Vec<String>) {
    let tools = reg.tools.iter().map(|t| t.name.clone()).collect();
    let commands = reg.commands.iter().map(|c| (c.name.clone(), c.hidden)).collect();
    let ns = reg.settings.iter().map(|n| n.to_value()["namespace"].as_str().unwrap_or("").to_owned()).collect();
    (tools, commands, ns)
}

// ── notify ──────────────────────────────────────────────────────────────────

fn gotify_route(method: &str, path: &str, _body: &str, _port: u16) -> (u16, &'static str, String) {
    if method == "POST" && path == "/message" {
        (200, "application/json", "{\"id\":1}".into())
    } else {
        (404, "text/plain", "nope".into())
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn notify_dispatch_and_lifecycle() {
    let env = Env::new();
    let (port, seen) = http_server(gotify_route).await;
    let settings = json!({"notify": {
        "native": {"enabled": true},
        "gotify": {"enabled": true, "serverUrl": format!("http://127.0.0.1:{port}"), "appToken": "tok"},
        "events": {"agent_settled": {"enabled": true, "platforms": []}, "input_needed": {"enabled": true, "platforms": ["native"]}},
    }});
    let (ext, reg) = env.load("notify", "unipi_notify.wasm", settings).await;
    let (tools, commands, ns) = names(&reg);
    assert_eq!(tools, ["notify_user"]);
    assert_eq!(
        commands,
        [
            ("notify-event".to_owned(), false),
            ("notify-test".to_owned(), true),
            ("notify-set-gotify".to_owned(), true),
            ("notify-set-tg".to_owned(), true),
            ("notify-set-ntfy".to_owned(), true),
        ]
    );
    assert_eq!(ns, ["notify"]);
    assert_eq!(reg.tools[0].parameters.to_value()["required"], json!(["message"]));
    env.event(&ext, EventKind::SessionStart, json!({"type": "session_start", "reason": "startup"})).await;

    // notify_user: immediate result, dispatch from the queue timer.
    let r = env.tool(&ext, "notify_user", json!({"message": "Build done", "priority": "high", "platforms": ["native", "gotify"]})).await;
    assert_eq!(r["content"][0]["text"], "Notification sending to 2 platform(s): native, gotify");
    assert_eq!(r["details"]["priorityByPlatform"], json!({"gotify": 8}));
    assert!(env.pump_timers(&ext, "notify.queue", 5000).await > 0);
    let posts = seen.lock().unwrap().clone();
    assert_eq!(posts.len(), 1, "{posts:?}");
    assert!(posts[0].2.to_ascii_lowercase().contains("x-gotify-key: tok"));
    let body: Value = serde_json::from_str(&posts[0].3).unwrap();
    assert_eq!(body, json!({"title": "Oino Notification", "message": "Build done", "priority": 8}));
    let devices = env.details("ui_device");
    assert!(devices.iter().any(|d| d.as_str().unwrap_or("").contains("Build done")), "{devices:?}");
    let sent = env.published("unipi:notify:sent");
    assert_eq!(sent.last().unwrap()["eventType"], "agent_tool");
    assert_eq!(sent.last().unwrap()["success"], true);

    // Run finished: one notification after the 1.5 s grace.
    env.event(&ext, EventKind::AgentStart, json!({"type": "agent_start"})).await;
    env.event(&ext, EventKind::AgentEnd, json!({"type": "agent_end", "messages": []})).await;
    env.event(&ext, EventKind::AgentSettled, json!({"type": "agent_settled"})).await;
    assert!(env.pump_timers(&ext, "notify.finish", 6000).await > 0);
    let sent = env.published("unipi:notify:sent");
    assert_eq!(sent.last().unwrap()["eventType"], "agent_settled");
    let devices = env.details("ui_device");
    assert!(devices.iter().any(|d| d.as_str().unwrap_or("").contains("Agent is complete")), "{devices:?}");

    // A chained run cancels the pending "finished".
    let before = env.published("unipi:notify:sent").len();
    env.event(&ext, EventKind::AgentSettled, json!({"type": "agent_settled"})).await;
    env.event(&ext, EventKind::AgentStart, json!({"type": "agent_start"})).await;
    env.pump_timers(&ext, "notify.finish", 2500).await;
    assert_eq!(env.published("unipi:notify:sent").len(), before, "cancelled by agent_start");

    // input_needed while running.
    env.event(&ext, EventKind::UiPromptStart, json!({"type": "ui_prompt_start", "title": "Pick a model", "kind": "select"})).await;
    assert!(env.pump_timers(&ext, "notify.queue", 5000).await > 0);
    let devices = env.details("ui_device");
    assert!(devices.iter().any(|d| d.as_str().unwrap_or("").contains("Waiting for your input: Pick a model")), "{devices:?}");
    env.event(&ext, EventKind::UiPromptEnd, json!({"type": "ui_prompt_end"})).await;

    // /notify-event writes the overlay.
    env.command(&ext, "notify-event", "bogus on").await;
    assert!(env.fired("notify").last().unwrap()["message"].as_str().unwrap().starts_with("Unknown event \"bogus\""));
    env.command(&ext, "notify-event", "input_needed off").await;
    assert_eq!(env.fired("notify").last().unwrap()["message"], "notify: input_needed is now off.");
    env.command(&ext, "notify-event", "x").await;
    assert_eq!(env.fired("notify").last().unwrap()["message"], "Usage: /notify-event <event> <on|off>");
    let items = ext.complete_command("notify-event", "input", env.cx(CallKind::Command)).await;
    assert_eq!(items.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(), ["input_needed"]);
}

// ── mcp ─────────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn mcp_stdio_server_and_tools() {
    let env = Env::new();
    let script = harness_dir().join("fixtures/fake_mcp.py");
    let global = env.agent.path().join("state/unipi.mcp/global");
    std::fs::create_dir_all(&global).unwrap();
    std::fs::write(
        global.join("mcp.json"),
        serde_json::to_vec(&json!({"mcpServers": {
            "fake": {"command": "python3", "args": [script.to_string_lossy()]},
            "broken": {"command": "definitely-not-a-binary-xyz", "args": []},
            "off": {"command": "python3", "args": [script.to_string_lossy()], "enabled": false},
            "bad name!": {"command": "x"},
        }}))
        .unwrap(),
    )
    .unwrap();
    let (ext, reg) = env.load("mcp", "unipi_mcp.wasm", json!({})).await;
    let (tools, commands, ns) = names(&reg);
    assert!(tools.is_empty(), "tools are dynamic");
    assert_eq!(
        commands,
        [
            ("mcp-status".to_owned(), false),
            ("mcp-settings".to_owned(), true),
            ("mcp-add".to_owned(), true),
            ("mcp-sync".to_owned(), true),
            ("mcp-reload".to_owned(), true),
        ]
    );
    assert_eq!(ns, ["mcp"]);
    env.event(&ext, EventKind::SessionStart, json!({"type": "session_start", "reason": "startup"})).await;

    let registered: Vec<Value> = env.details("register_tool");
    let reg_names: Vec<&str> = registered.iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(reg_names, ["fake__add", "fake__echo", "fake__fail"], "sorted by final name");
    let add = &registered[0];
    assert_eq!(add["description"], "MCP tool: add [Server: fake]");
    let params: Value = match add["parameters"].as_str() {
        Some(text) => serde_json::from_str(text).unwrap(),
        None => add["parameters"].clone(),
    };
    assert_eq!(params["required"], json!(["a", "b"]), "canonical schema");
    assert_eq!(params.as_object().unwrap().keys().collect::<Vec<_>>(), ["properties", "required", "type"]);
    assert_eq!(registered[1]["kind"], "read", "readOnlyHint → read");
    assert_eq!(registered[1]["annotations"]["readOnly"], true);
    let started = env.published("unipi:mcp:server:started");
    assert_eq!(started, [json!({"name": "fake", "toolCount": 3})]);
    let errors = env.published("unipi:mcp:server:error");
    assert_eq!(errors.len(), 1);
    assert_eq!(errors[0]["name"], "broken");
    assert_eq!(env.published("unipi:mcp:tools:registered")[0]["toolNames"], json!(["fake__add", "fake__echo", "fake__fail"]));

    let r = env.tool(&ext, "fake__echo", json!({"text": "hi"})).await;
    assert_eq!(r["content"][0]["text"], "echo: hi");
    assert_eq!(r["details"]["server"], "fake");
    assert_eq!(r["details"]["truncated"], false);
    let r = env.tool(&ext, "fake__add", json!({"a": 2, "b": 3})).await;
    assert_eq!(r["content"][0]["text"], "5");
    assert_eq!(r["details"]["structuredContent"], json!({"sum": 5}));
    let r = env.tool(&ext, "fake__fail", json!({})).await;
    assert_eq!(r["content"][0]["text"], "MCP tool error from fake: it broke");
    assert_eq!(r["details"]["error"], true);

    env.command(&ext, "mcp-status", "").await;
    let status = env.fired("notify").last().unwrap()["message"].as_str().unwrap().to_owned();
    assert!(status.starts_with("MCP Server Status:\n\n● fake — running (3 tools)\n✗ broken — error — "), "{status}");
    assert!(status.contains("○ off — stopped"));
    assert!(status.ends_with("1 active, 1 failed, 3 total tools"));

    env.command(&ext, "mcp-settings", "").await;
    let open = env.details("ui_panel").last().unwrap()["open"].clone();
    assert_eq!(open["panelId"], "unipi.mcp/servers");
    assert!(open["spec"].to_string().contains("Config problems"), "invalid entries are reported");

    let groups = ext
        .handle_service("info-screen.groups", RawJson::from(json!({})), &ExtensionId::new("unipi.info-screen").unwrap(), env.cx(CallKind::Service))
        .await
        .unwrap()
        .to_value();
    assert_eq!(groups["id"], "mcp");
    assert_eq!(groups["data"]["active"]["value"], "1");
    assert_eq!(groups["data"]["failed"]["detail"], "broken");

    env.command(&ext, "mcp-reload", "").await;
    assert_eq!(env.fired("notify").last().unwrap()["message"], "Reloaded MCP servers: 1 active, 1 failed, 3 tools.");
    assert!(env.details("unregister_tool").len() >= 3, "reload unregisters the old tools");

    env.event(&ext, EventKind::SessionShutdown, json!({"type": "session_shutdown"})).await;
}

// ── web-api ─────────────────────────────────────────────────────────────────

const ARTICLE: &str = r#"<!doctype html><html lang="en"><head><title>Doc Title</title><meta name="author" content="Jane Doe"></head>
<body><nav>Home | About</nav><article><h1>Big Heading</h1>
<p>This is the first paragraph of a reasonably long article about Rust and WebAssembly, written so that the readability scorer finds enough text to treat it as the main content of the page.</p>
<p>The second paragraph adds more words, with a <a href="https://example.com/x">link</a> and <strong>bold text</strong>, so the extracted markdown keeps inline formatting intact.</p>
<p>A third paragraph, because readability engines like to see several paragraphs of prose before they commit to a candidate container as the article body.</p>
</article><footer>Copyright footer</footer></body></html>"#;

fn web_route(method: &str, path: &str, body: &str, port: u16) -> (u16, &'static str, String) {
    match (method, path) {
        ("GET", "/article") => (200, "text/html; charset=utf-8", ARTICLE.into()),
        ("GET", "/refresh") => (200, "text/html", format!("<html><head><meta http-equiv=\"refresh\" content=\"0;url=http://127.0.0.1:{port}/article\"></head></html>")),
        ("GET", "/data") => (200, "application/json", "{\"b\":1,\"a\":[1,2]}".into()),
        ("GET", "/file.pdf") => (200, "application/pdf", "%PDF".into()),
        ("GET", "/missing") => (404, "text/plain", "no".into()),
        ("POST", "/v1/search") => {
            let q: Value = serde_json::from_str(body).unwrap_or_default();
            (200, "application/json", json!({"results": [
                {"title": "Rust", "url": "https://rust-lang.org", "excerpt": format!("about {}", q["query"].as_str().unwrap_or(""))},
                {"url": "https://docs.rs"},
            ]}).to_string())
        }
        ("POST", "/v1/fetch") => (200, "application/json", json!({"url": "https://w.test/page", "markdown": "# Wigolo page"}).to_string()),
        _ => (404, "text/plain", "not found".into()),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn web_api_smart_fetch_and_search() {
    let env = Env::new();
    let (port, seen) = http_server(web_route).await;
    let base = format!("http://127.0.0.1:{port}");
    let settings = json!({"web-api": {"providers": {"wigolo": {"enabled": true, "url": base}, "duckduckgo": {"enabled": false}, "jina-search": {"enabled": false}, "jina-reader": {"enabled": false}}}});
    let (ext, reg) = env.load("web-api", "unipi_web_api.wasm", settings).await;
    let (tools, commands, ns) = names(&reg);
    assert_eq!(tools, ["web_search", "multi_web_content_read", "web_llm_summarize"]);
    assert_eq!(commands, [("web-cache-clear".to_owned(), true)]);
    assert_eq!(ns, ["web-api"]);
    assert_eq!(reg.tools[1].prompt_guidelines.len(), 5);
    env.event(&ext, EventKind::SessionStart, json!({"type": "session_start", "reason": "startup"})).await;
    let set = env.details("control");
    assert!(set.iter().any(|c| c == "set-active-tools" || c.as_str().unwrap_or("").contains("active")), "tool exposure synced: {set:?}");

    // Smart fetch: readable markdown, metadata header, cached.
    let r = env.tool(&ext, "multi_web_content_read", json!({"url": format!("{base}/article")})).await;
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("# "), "{text}");
    assert!(text.contains(&format!("URL: {base}/article")));
    assert!(text.contains("Author: Jane Doe"));
    assert!(text.contains("first paragraph"));
    assert!(text.contains("[link](https://example.com/x)"));
    assert!(!text.contains("Copyright footer"));
    assert_eq!(r["details"]["url"], format!("{base}/article"));
    let hits_before = seen.lock().unwrap().iter().filter(|r| r.1 == "/article").count();
    let again = env.tool(&ext, "multi_web_content_read", json!({"url": format!("{base}/article")})).await;
    assert_eq!(again["content"], r["content"]);
    assert_eq!(seen.lock().unwrap().iter().filter(|r| r.1 == "/article").count(), hits_before, "served from the cache");
    let cache_dir = env.agent.path().join("cache/ext/unipi.web-api/web");
    assert_eq!(std::fs::read_dir(&cache_dir).unwrap().count(), 1);

    // Meta refresh, JSON, binary, text format, errors, batch.
    let r = env.tool(&ext, "multi_web_content_read", json!({"url": format!("{base}/refresh"), "verbose": false, "format": "text"})).await;
    assert!(r["content"][0]["text"].as_str().unwrap().contains("first paragraph"));
    let r = env.tool(&ext, "multi_web_content_read", json!({"url": format!("{base}/data"), "verbose": false})).await;
    assert_eq!(r["content"][0]["text"], "{\n  \"b\": 1,\n  \"a\": [\n    1,\n    2\n  ]\n}");
    let r = env.tool(&ext, "multi_web_content_read", json!({"url": format!("{base}/file.pdf"), "verbose": false})).await;
    assert_eq!(r["content"][0]["text"], "[Binary file: application/pdf]");
    let err = env.tool_result(&ext, "multi_web_content_read", json!({"url": format!("{base}/missing")})).await;
    assert!(err.as_ref().err().is_some_and(|e| e.contains("Read failed: HTTP error: 404 Not Found")), "{err:?}");
    let err = env.tool_result(&ext, "multi_web_content_read", json!({"url": "ftp://x.test/y"})).await;
    assert!(err.err().is_some_and(|e| e.contains("Unsupported protocol: ftp:")));
    let r = env.tool(&ext, "multi_web_content_read", json!({"url": [format!("{base}/article"), format!("{base}/missing")]})).await;
    let batch = r["content"][0]["text"].as_str().unwrap();
    assert!(batch.starts_with("# Batch Read Results\n\nTotal: 2 · Succeeded: 1 · Failed: 1"), "{batch}");
    assert_eq!(r["details"], json!({"total": 2, "succeeded": 1, "failed": 1}));
    let r = env.tool(&ext, "multi_web_content_read", json!({"url": []})).await;
    assert_eq!(r["content"][0]["text"], "No URLs provided.");

    // Provider read (source 1 = wigolo) and wigolo search.
    let r = env.tool(&ext, "multi_web_content_read", json!({"url": "https://w.test/page", "source": 1})).await;
    assert_eq!(r["content"][0]["text"], "Content from https://w.test/page:\n\n# Wigolo page");
    let r = env.tool(&ext, "web_search", json!({"query": "wasm"})).await;
    assert_eq!(
        r["content"][0]["text"],
        "Found 2 results:\n\n1. **Rust**\n   https://rust-lang.org\n   about wasm\n\n2. **https://docs.rs**\n   https://docs.rs\n   "
    );
    let err = env.tool_result(&ext, "web_search", json!({"query": "x", "source": 4})).await;
    assert!(err.err().is_some_and(|e| e.contains("Search failed: No provider at rank 4 for search.\nAvailable ranks: 1")));
    let err = env.tool_result(&ext, "web_llm_summarize", json!({"url": "https://x"})).await;
    assert!(err.err().is_some_and(|e| e.contains("Summarize failed: No summarize provider configured.")));

    // Cache clear (confirm dialog answered yes).
    env.host.queue_dialog_answer(json!({"confirmed": true}));
    env.command(&ext, "web-cache-clear", "").await;
    let msg = env.fired("notify").last().unwrap()["message"].as_str().unwrap().to_owned();
    assert!(msg.starts_with("Cache cleared: "), "{msg}");
    assert_eq!(std::fs::read_dir(&cache_dir).unwrap().count(), 0);

    let group = ext
        .handle_service("info-screen.groups", RawJson::from(json!({})), &ExtensionId::new("unipi.info-screen").unwrap(), env.cx(CallKind::Service))
        .await
        .unwrap()
        .to_value();
    assert_eq!(group["id"], "web-api");
    assert_eq!(group["data"]["cacheEntries"]["value"], "0");
}

// ── updater ─────────────────────────────────────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn updater_docs_browsers() {
    let env = Env::new();
    let bundled = env.agent.path().join("bundled");
    let docs = bundled.join("oino.docs@1.0.0-aaaaaaaa");
    std::fs::create_dir_all(&docs).unwrap();
    std::fs::write(docs.join("README.md"), "# Oino\n\nThe README.").unwrap();
    std::fs::write(docs.join("CHANGELOG.md"), "# Changelog\n\n## [1.0.0] — 2026-10-01\n\n### Added\n- Everything\n\n## [0.9.0] - 2026-09-01\n\n### Fixed\n- A bug\n").unwrap();
    let wf = bundled.join("unipi.workflow@3.0.0-bbbbbbbb");
    std::fs::create_dir_all(&wf).unwrap();
    std::fs::write(wf.join("README.md"), "# workflow\n\nPermission gate.").unwrap();
    let (ext, reg) = env.load("updater", "unipi_updater.wasm", json!({"updater": {"autoUpdate": "disabled"}})).await;
    let (tools, commands, ns) = names(&reg);
    assert!(tools.is_empty());
    assert_eq!(
        commands,
        [
            ("readme".to_owned(), false),
            // hidden runner the core `/changelog` built-in forwards to (commands.md §2)
            ("changelog".to_owned(), true),
            ("update-check".to_owned(), true)
        ]
    );
    assert_eq!(ns, ["updater"]);
    assert_eq!(reg.services, ["info-screen.groups"]);

    env.event(&ext, EventKind::SessionStart, json!({"type": "session_start", "reason": "startup"})).await;
    assert!(env.published("unipi:update:check").is_empty(), "autoUpdate disabled → no check");
    let state: Value = serde_json::from_slice(&std::fs::read(env.agent.path().join("state/unipi.updater/global/state.json")).unwrap()).unwrap();
    assert!(state["lastRunVersion"].is_string());

    let items = ext.complete_command("readme", "w", env.cx(CallKind::Command)).await;
    assert_eq!(items.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(), ["workflow"]);
    env.command(&ext, "readme", "").await;
    let open = env.details("ui_panel").last().unwrap()["open"].clone();
    assert_eq!(open["panelId"], "unipi.updater/readme");
    let list = open["spec"].to_string();
    assert!(list.contains("\"oino\"") && list.contains("\"workflow\""), "{list}");
    env.event(&ext, EventKind::OinoUiAction, json!({"type": "oino_ui_action", "panelId": "panel-1", "actionId": "open", "value": "workflow"})).await;
    let update = env.details("ui_panel").last().unwrap().clone();
    assert_eq!(update["update"], "panel-1");
    assert!(update["spec"].to_string().contains("Permission gate."));
    env.event(&ext, EventKind::OinoUiAction, json!({"type": "oino_ui_action", "panelId": "panel-1", "actionId": "back"})).await;
    assert!(env.details("ui_panel").last().unwrap()["spec"].to_string().contains("\"workflow\""));

    env.command(&ext, "readme", "workflow").await;
    assert!(env.details("ui_panel").last().unwrap()["open"]["spec"].to_string().contains("Permission gate."));

    let group = ext
        .handle_service("info-screen.groups", RawJson::from(json!({})), &ExtensionId::new("unipi.info-screen").unwrap(), env.cx(CallKind::Service))
        .await
        .unwrap()
        .to_value();
    assert_eq!(group["id"], "updater");
    assert_eq!(group["data"]["status"]["value"], "checking");
}
