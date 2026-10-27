//! `unipi.utility` in the real wasm host: registration, commands (continue, settings, answer
//! reply/questions/web, doctor, cleanup, move dry run, summarize), auto-rename through the
//! `models.complete` fallback, attachments (editor scan + submit transform), render specs and
//! the image tools against a local endpoint.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use oino_extension_api::manifest::Manifest;
use oino_extension_api::ops::{
    ControlOp, ControlReply, DeviceOp, EditorOp, HostContextInfo, ModelsOp, ModelsReply, PanelOp,
    SecretOp, SessionQuery, SessionWrite,
};
use oino_extension_api::registry::{CommandSpec, FlagSpec, RendererSpec, ShortcutSpec, ToolSpec};
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    BoxFut, CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionId,
    ExtensionInstance, ExtensionRuntime, HostError, HostServices, InitInput, LoadedPackage, Mode,
    RawJson, Registration, ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// Builds the component once (release, wasm32-wasip2).
fn component() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(extensions_dir()).args([
            "build",
            "-j",
            "2",
            "--release",
            "--target",
            "wasm32-wasip2",
            "-p",
            "unipi-utility",
        ]);
        for var in [
            "CARGO_TARGET_DIR",
            "RUSTFLAGS",
            "CARGO_ENCODED_RUSTFLAGS",
            "CARGO_BUILD_TARGET",
            "CARGO_MANIFEST_DIR",
        ] {
            cmd.env_remove(var);
        }
        cmd.env("CARGO_TARGET_DIR", extensions_dir().join("target"));
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        extensions_dir().join("target/wasm32-wasip2/release/unipi_utility.wasm")
    })
}

/// The recorder plus the session/model state the utility reads.
#[derive(Default)]
struct World {
    branch: Mutex<Vec<Value>>,
    name: Mutex<Option<String>>,
    models: Mutex<Vec<Value>>,
    completion: Mutex<Option<Value>>,
    idle: Mutex<bool>,
    commands: Mutex<Vec<Value>>,
}

struct TestHost {
    rec: Arc<RecordingHostServices>,
    world: Arc<World>,
}

fn raw(v: Value) -> RawJson {
    RawJson::from(v)
}

impl HostServices for TestHost {
    fn subscribe(&self, ext: &ExtensionId, events: &[EventKind]) {
        self.rec.subscribe(ext, events);
    }
    fn register_tool(&self, ext: &ExtensionId, spec: ToolSpec) -> Result<(), HostError> {
        self.rec.register_tool(ext, spec)
    }
    fn register_command(&self, ext: &ExtensionId, spec: CommandSpec) -> Result<String, HostError> {
        self.rec.register_command(ext, spec)
    }
    fn register_shortcut(&self, ext: &ExtensionId, spec: ShortcutSpec) -> Result<(), HostError> {
        self.rec.register_shortcut(ext, spec)
    }
    fn register_flag(&self, ext: &ExtensionId, spec: FlagSpec) -> Result<(), HostError> {
        self.rec.register_flag(ext, spec)
    }
    fn register_renderer(&self, ext: &ExtensionId, spec: RendererSpec) {
        self.rec.register_renderer(ext, spec);
    }
    fn register_service(&self, ext: &ExtensionId, name: &str) -> Result<(), HostError> {
        self.rec.register_service(ext, name)
    }
    fn register_settings(&self, ext: &ExtensionId, ns: RawJson) -> Result<(), HostError> {
        self.rec.register_settings(ext, ns)
    }
    fn session_read(&self, cx: &CallContext, q: SessionQuery) -> Result<RawJson, HostError> {
        match q {
            SessionQuery::Branch { .. } | SessionQuery::ContextEntries => {
                let mut all = self.world.branch.lock().unwrap().clone();
                all.extend(self.rec.entries());
                Ok(raw(Value::Array(all)))
            }
            SessionQuery::SessionName => Ok(raw(self
                .world
                .name
                .lock()
                .unwrap()
                .clone()
                .map_or(Value::Null, Value::String))),
            other => self.rec.session_read(cx, other),
        }
    }
    fn session_write(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        w: SessionWrite,
    ) -> BoxFut<'_, Result<Option<String>, HostError>> {
        if let SessionWrite::SetSessionName(n) = &w {
            self.world.name.lock().unwrap().clone_from(n);
        }
        self.rec.session_write(ext, cx, w)
    }
    fn control(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: ControlOp,
    ) -> BoxFut<'_, Result<ControlReply, HostError>> {
        let reply = match &op {
            ControlOp::ModelsList => Some(ControlReply::Json(raw(Value::Array(
                self.world.models.lock().unwrap().clone(),
            )))),
            ControlOp::IsIdle => Some(ControlReply::Bool(*self.world.idle.lock().unwrap())),
            ControlOp::CommandsList => Some(ControlReply::Json(raw(Value::Array(
                self.world.commands.lock().unwrap().clone(),
            )))),
            ControlOp::GetSettings => {
                Some(ControlReply::Json(raw(json!({"extensionSettings": {}}))))
            }
            ControlOp::Model => Some(ControlReply::MaybeJson(Some(raw(
                json!({"provider": "p", "id": "text", "input": ["text"]}),
            )))),
            ControlOp::WaitForIdle => Some(ControlReply::Unit),
            _ => None,
        };
        match reply {
            Some(r) => Box::pin(async move { Ok(r) }),
            None => self.rec.control(ext, cx, op),
        }
    }
    fn models(
        &self,
        _ext: &ExtensionId,
        _cx: &CallContext,
        op: ModelsOp,
    ) -> BoxFut<'_, Result<ModelsReply, HostError>> {
        let world = Arc::clone(&self.world);
        let rec = Arc::clone(&self.rec);
        Box::pin(async move {
            match op {
                ModelsOp::Complete { request, .. } => {
                    rec.ui_fire(
                        &ExtensionId::new("harness").unwrap(),
                        &CallContext::default(),
                        raw(json!({"method": "harness.complete", "request": request.to_value()})),
                    )
                    .ok();
                    world
                        .completion
                        .lock()
                        .unwrap()
                        .clone()
                        .map(|v| ModelsReply::Json(raw(v)))
                        .ok_or(HostError::Failed("no completion scripted".into()))
                }
                ModelsOp::CredentialFor(_) => Ok(ModelsReply::Credential(None)),
                _ => Err(HostError::unsupported("models")),
            }
        })
    }
    fn ui_dialog(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        req: RawJson,
        cancel: Option<CancellationToken>,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        self.rec.ui_dialog(ext, cx, req, cancel)
    }
    fn ui_fire(&self, ext: &ExtensionId, cx: &CallContext, req: RawJson) -> Result<(), HostError> {
        self.rec.ui_fire(ext, cx, req)
    }
    fn ui_panel(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: PanelOp,
    ) -> Result<Option<String>, HostError> {
        self.rec.ui_panel(ext, cx, op)
    }
    fn ui_editor(&self, cx: &CallContext, op: EditorOp) -> Result<Option<String>, HostError> {
        self.rec.ui_editor(cx, op)
    }
    fn ui_device(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        op: DeviceOp,
    ) -> BoxFut<'_, Result<Option<RawJson>, HostError>> {
        self.rec.ui_device(ext, cx, op)
    }
    fn bus_publish(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        topic: &str,
        data: RawJson,
        sticky: bool,
    ) {
        self.rec.bus_publish(ext, cx, topic, data, sticky);
    }
    fn bus_subscribe(&self, ext: &ExtensionId, cx: &CallContext, pattern: &str, on: bool) {
        self.rec.bus_subscribe(ext, cx, pattern, on);
    }
    fn bus_get(&self, cx: &CallContext, topic: &str) -> Option<RawJson> {
        self.rec.bus_get(cx, topic)
    }
    fn service_call(
        &self,
        ext: &ExtensionId,
        cx: &CallContext,
        target: Option<&ExtensionId>,
        name: &str,
        payload: RawJson,
        timeout: Duration,
    ) -> BoxFut<'_, Result<RawJson, HostError>> {
        self.rec
            .service_call(ext, cx, target, name, payload, timeout)
    }
    fn secret(&self, ext: &ExtensionId, op: SecretOp) -> Result<Option<String>, HostError> {
        self.rec.secret(ext, op)
    }
    fn host_context(&self, cx: &CallContext) -> HostContextInfo {
        self.rec.host_context(cx)
    }
    fn deliver_event(
        &self,
        ext: &ExtensionId,
        session_id: &str,
        kind: EventKind,
        payload: RawJson,
    ) {
        self.rec.deliver_event(ext, session_id, kind, payload);
    }
}

struct Env {
    runtime: WasmRuntime,
    rec: Arc<RecordingHostServices>,
    world: Arc<World>,
    _agent: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let tmp = extensions_dir().join("target/tmp-tests");
        std::fs::create_dir_all(&tmp).unwrap();
        let agent = tempfile::tempdir_in(&tmp).unwrap();
        let project = tempfile::tempdir_in(&tmp).unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        options.local_services = false;
        let world = Arc::new(World::default());
        *world.idle.lock().unwrap() = true;
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            rec: RecordingHostServices::new(),
            world,
            _agent: agent,
            project,
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

    async fn load(&self, settings: Value) -> (Arc<dyn ExtensionInstance>, Registration) {
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join("unipi/utility/extension.json"))
                .unwrap(),
        )
        .unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(component().clone()),
            root: None,
            bundled: true,
        };
        let host: Arc<dyn HostServices> = Arc::new(TestHost {
            rec: Arc::clone(&self.rec),
            world: Arc::clone(&self.world),
        });
        let ext = self.runtime.load(package, host).await.unwrap();
        let input = InitInput {
            settings: RawJson::from(settings),
            ..InitInput::default()
        };
        let reg = ext.init(input, self.cx(CallKind::Init)).await.unwrap();
        (ext, reg)
    }

    async fn event(
        &self,
        ext: &Arc<dyn ExtensionInstance>,
        kind: EventKind,
        payload: Value,
    ) -> Option<Value> {
        ext.on_event(kind, RawJson::from(payload), self.cx(CallKind::Event))
            .await
            .unwrap()
            .map(|r| r.to_value())
    }

    async fn command(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, args: &str) {
        ext.run_command(
            name,
            args,
            CancellationToken::new(),
            self.cx(CallKind::Command),
        )
        .await
        .unwrap();
    }

    async fn tool(
        &self,
        ext: &Arc<dyn ExtensionInstance>,
        name: &str,
        args: Value,
    ) -> Result<Value, String> {
        let inv = ToolInvocation {
            tool_call_id: "call-1".into(),
            name: name.into(),
            args: RawJson::from(args),
            parent_tool_call_id: None,
        };
        let mut cx = self.cx(CallKind::Tool);
        cx.tool_call_id = Some("call-1".into());
        ext.run_tool(inv, CancellationToken::new(), cx)
            .await
            .map(|v| v.to_value())
            .map_err(|e| e.to_string())
    }

    fn details(&self, method: &str) -> Vec<Value> {
        self.rec
            .calls_named(method)
            .into_iter()
            .map(|c| c.detail)
            .collect()
    }

    fn fired(&self, method: &str) -> Vec<Value> {
        self.details("ui_fire")
            .into_iter()
            .filter(|d| d["method"] == method)
            .collect()
    }

    fn writes(&self, key: &str) -> Vec<Value> {
        self.details("session_write")
            .into_iter()
            .filter(|d| d.get(key).is_some())
            .collect()
    }

    fn say(&self, text: &str) {
        self.world.branch.lock().unwrap().push(json!({"type": "message", "id": "a1", "message": {"role": "assistant", "content": [{"type": "text", "text": text}], "stopReason": "stop"}}));
    }

    async fn wait_for(&self, what: impl Fn(&Self) -> bool) -> bool {
        for _ in 0..200 {
            if what(self) {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn registers_the_utility_surface() {
    let env = Env::new();
    let (_ext, reg) = env.load(json!({})).await;
    let mut cmds: Vec<(String, bool)> = reg
        .commands
        .iter()
        .map(|c| (c.name.clone(), c.hidden))
        .collect();
    cmds.sort();
    assert_eq!(
        cmds,
        [
            ("answer", false),
            ("cleanup", false),
            ("continue", false),
            ("doctor", false),
            ("move", false),
            ("rename-now", true),
            ("retry", false),
            ("summarize", false),
        ]
        .map(|(n, h)| (n.to_owned(), h))
    );
    let tools: Vec<&str> = reg.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        tools,
        [
            "image_generate",
            "image_edit",
            "image_recognize",
            "rename_session"
        ]
    );
    let rename = reg
        .tools
        .iter()
        .find(|t| t.name == "rename_session")
        .unwrap();
    assert!(
        !rename.default_active,
        "the main agent never has a rename tool"
    );
    assert_eq!(reg.renderers.len(), 7 + 10 + 1);
    assert_eq!(
        reg.services,
        [
            "image.models",
            "utility.render-style",
            "utility.render-delegated"
        ]
    );
    let namespaces: Vec<String> = reg
        .settings
        .iter()
        .map(|s| {
            s.to_value()["namespace"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(namespaces, ["utility", "image"]);
    let answer = reg.commands.iter().find(|c| c.name == "answer").unwrap();
    assert!(answer.completions);

    let env2 = Env::new();
    let (_ext, reg2) = env2
        .load(json!({"image": {"generate": {"enabled": false}, "recognize": {"enabled": false}}}))
        .await;
    let tools: Vec<&str> = reg2.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(tools, ["image_edit", "rename_session"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn continue_settings_summarize_and_responses() {
    let env = Env::new();
    let (ext, _) = env.load(json!({})).await;
    env.command(&ext, "continue", "").await;
    let sent = env.writes("sendMessage");
    assert_eq!(sent[0]["sendMessage"]["customType"], "unipi-continue");
    assert_eq!(sent[0]["sendMessage"]["display"], false);
    assert_eq!(sent[0]["options"]["triggerTurn"], true);

    env.command(&ext, "summarize", "focus on risks").await;
    assert!(env.fired("notify").iter().any(|n| {
        n["message"]
            .as_str()
            .unwrap_or_default()
            .contains("summarize skill is off")
    }));
    env.world
        .commands
        .lock()
        .unwrap()
        .push(json!({"name": "skill:summarize"}));
    env.command(&ext, "summarize", "focus on risks").await;
    let user = env.writes("sendUserMessage");
    assert_eq!(
        user[0]["sendUserMessage"],
        "/skill:summarize focus on risks"
    );
    assert_eq!(user[0]["options"]["expandPromptTemplates"], true);
    assert_eq!(user[0]["options"]["provenance"]["source"], "Utility");

    env.command(&ext, "doctor", "").await;
    env.command(&ext, "cleanup", "--dry-run").await;
    let responses: Vec<String> = env
        .writes("sendMessage")
        .into_iter()
        .filter(|w| w["sendMessage"]["customType"] == "unipi-response")
        .map(|w| {
            w["sendMessage"]["content"]
                .as_str()
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert!(
        responses[0].starts_with("## Diagnostics"),
        "{}",
        responses[0]
    );
    assert!(responses[0].contains("dir_ext_global"));
    assert!(
        responses[1].starts_with("## Cleanup (dry run)"),
        "{}",
        responses[1]
    );
    let opts = env
        .writes("sendMessage")
        .into_iter()
        .find(|w| w["sendMessage"]["customType"] == "unipi-response")
        .unwrap();
    assert_eq!(opts["options"]["triggerTurn"], false);

    *env.world.idle.lock().unwrap() = false;
    env.command(&ext, "doctor", "").await;
    assert!(env.fired("notify").iter().any(|n| {
        n["message"]
            .as_str()
            .unwrap_or_default()
            .contains("Agent is busy")
    }));
}

#[tokio::test(flavor = "multi_thread")]
async fn answer_reply_questions_and_hint() {
    let env = Env::new();
    let (ext, _) = env.load(json!({})).await;
    env.command(&ext, "answer", "").await;
    assert!(
        env.fired("notify")
            .iter()
            .any(|n| n["message"] == "No agent reply to answer yet.")
    );

    env.say("Plan ready.\n\n- Which port?\n- Should I use TLS?");
    env.event(
        &ext,
        EventKind::AgentEnd,
        json!({"type": "agent_end", "messages": []}),
    )
    .await;
    let hint = env
        .fired("setWidget")
        .into_iter()
        .find(|w| w["widgetKey"] == "unipi-answer-hint")
        .unwrap();
    assert!(
        hint["widgetLines"][0]
            .as_str()
            .unwrap()
            .contains("2 questions in the reply")
    );
    env.event(
        &ext,
        EventKind::Input,
        json!({"type": "input", "text": "x", "source": "interactive"}),
    )
    .await;
    assert!(
        env.fired("setWidget")
            .iter()
            .any(|w| w["widgetKey"] == "unipi-answer-hint" && w["widgetLines"].is_null())
    );

    env.rec
        .queue_dialog_answer(json!({"value": {"actionId": "send", "value": "8080 and yes"}}));
    env.command(&ext, "answer", "reply").await;
    let dialogs = env.details("ui_dialog");
    assert_eq!(dialogs[0]["method"], "custom");
    let sent = env.writes("sendUserMessage");
    assert_eq!(sent[0]["sendUserMessage"], "8080 and yes");
    assert_eq!(sent[0]["options"]["provenance"]["title"], "Your answers");

    env.rec.queue_dialog_answer(
        json!({"value": "Q1. Which port?\nA1: 8080\n\nQ2. Should I use TLS?\nA2: "}),
    );
    env.command(&ext, "answer", "questions").await;
    assert_eq!(env.details("ui_dialog")[1]["method"], "editor");
    let sent = env.writes("sendUserMessage");
    assert_eq!(
        sent[1]["sendUserMessage"],
        "1. > Which port?\n\n8080\n\n(Not answered: #2)"
    );

    let items = ext
        .complete_command("answer", "w", env.cx(CallKind::Command))
        .await;
    assert_eq!(
        items.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(),
        ["web"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn answer_web_form_over_the_loopback_listener() {
    let env = Env::new();
    let (ext, _) = env.load(json!({})).await;
    env.say("Which port? And should I use TLS?");
    let ext2 = Arc::clone(&ext);
    let cx = env.cx(CallKind::Command);
    let run = tokio::spawn(async move {
        ext2.run_command("answer", "web", CancellationToken::new(), cx)
            .await
    });
    assert!(
        env.wait_for(|e| !e.details("ui_panel").is_empty()).await,
        "waiting panel opened"
    );
    let panel = env.details("ui_panel")[0]["open"].clone();
    let url = panel["spec"]["children"][1]["value"][0]["text"]
        .as_str()
        .unwrap()
        .to_owned();
    let (port, path) = url
        .strip_prefix("http://127.0.0.1:")
        .unwrap()
        .split_once('/')
        .unwrap();
    let port: u16 = port.parse().unwrap();
    let path = format!("/{path}");
    let get = http(port, "GET", &path, "").await;
    assert!(get.starts_with("HTTP/1.1 200"), "{get}");
    assert!(get.contains("Which port?"));
    assert!(
        http(port, "GET", "/a/wrong", "")
            .await
            .starts_with("HTTP/1.1 404")
    );
    let submitted = http(
        port,
        "POST",
        &path,
        r#"{"answers":["8080","yes"],"note":"thanks"}"#,
    )
    .await;
    assert!(submitted.starts_with("HTTP/1.1 200"), "{submitted}");
    run.await.unwrap().unwrap();
    let sent = env.writes("sendUserMessage");
    assert_eq!(
        sent[0]["sendUserMessage"],
        "1. > Which port?\n\n8080\n\n2. > And should I use TLS?\n\nyes\n\nthanks"
    );
    assert!(
        env.details("ui_panel")
            .iter()
            .any(|p| p.get("close").is_some())
    );
}

async fn http(port: u16, method: &str, path: &str, body: &str) -> String {
    let mut s = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nhost: 127.0.0.1\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    s.write_all(req.as_bytes()).await.unwrap();
    let mut out = Vec::new();
    let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut out)).await;
    String::from_utf8_lossy(&out).into_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn auto_rename_falls_back_to_one_completion() {
    let env = Env::new();
    let (ext, _) = env.load(json!({})).await;
    *env.world.completion.lock().unwrap() = Some(
        json!({"role": "assistant", "content": [{"type": "toolCall", "id": "t1", "name": "rename_session", "arguments": {"name": "\"Kanboard Lane Ordering.\""}}], "stopReason": "toolUse"}),
    );
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    env.event(
        &ext,
        EventKind::Input,
        json!({"type": "input", "text": "hi", "source": "interactive"}),
    )
    .await;
    env.event(&ext, EventKind::AgentEnd, json!({"type": "agent_end", "messages": [{"role": "assistant", "stopReason": "stop", "content": [{"type": "text", "text": "Hello!"}]}]})).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        env.writes("setSessionName").is_empty(),
        "chatter never renames"
    );

    env.event(&ext, EventKind::Input, json!({"type": "input", "text": "fix the kanboard lane ordering bug", "source": "interactive"})).await;
    env.event(&ext, EventKind::AgentEnd, json!({"type": "agent_end", "messages": [{"role": "assistant", "stopReason": "stop", "content": [{"type": "text", "text": "Fixed."}]}]})).await;
    assert!(
        env.wait_for(|e| !e.writes("setSessionName").is_empty())
            .await,
        "renamed after the detached gate"
    );
    assert_eq!(
        env.writes("setSessionName")[0]["setSessionName"],
        "Kanboard Lane Ordering"
    );
    assert!(
        env.writes("appendEntry")
            .iter()
            .any(|w| w["appendEntry"] == "unipi:auto-name"
                && w["data"]["name"] == "Kanboard Lane Ordering")
    );
    let complete = env.fired("harness.complete");
    assert_eq!(
        complete[0]["request"]["context"]["tools"][0]["name"],
        "rename_session"
    );

    // A user-set name stops auto-rename.
    *env.world.name.lock().unwrap() = Some("Mine".into());
    env.event(&ext, EventKind::Input, json!({"type": "input", "text": "now switch to the image module work", "source": "interactive"})).await;
    env.event(
        &ext,
        EventKind::AgentEnd,
        json!({"type": "agent_end", "messages": []}),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(env.writes("setSessionName").len(), 1);

    // The runner bypasses the gate.
    env.command(&ext, "rename-now", "").await;
    assert_eq!(env.writes("setSessionName").len(), 2);
    assert!(
        env.fired("notify")
            .iter()
            .any(|n| n["message"] == "Session named \"Kanboard Lane Ordering\"")
    );

    // The tool only works inside the naming child.
    let r = env
        .tool(&ext, "rename_session", json!({"name": "X"}))
        .await
        .unwrap();
    assert_eq!(r["isError"], true);
}

#[tokio::test(flavor = "multi_thread")]
async fn attachments_scan_the_editor_and_expand_on_submit() {
    let env = Env::new();
    let (ext, _) = env.load(json!({})).await;
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start"}),
    )
    .await;
    let png = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0];
    let shot = env.project.path().join("shot.png");
    std::fs::write(&shot, png).unwrap();
    let canonical = shot.canonicalize().unwrap();
    let text = format!("look at {} please", canonical.display());
    let r = env
        .event(
            &ext,
            EventKind::OinoEditor,
            json!({"type": "oino_editor", "intent": "changed", "text": text}),
        )
        .await;
    assert!(r.is_none());
    let set: Vec<String> = env
        .details("ui_editor")
        .into_iter()
        .map(|d| d.as_str().unwrap_or_default().to_owned())
        .filter(|d| d.starts_with("Set"))
        .collect();
    assert_eq!(set, ["Set(\"look at [Image #1] please\")"]);
    let widget = env
        .fired("setWidget")
        .into_iter()
        .rev()
        .find(|w| w["widgetKey"] == "unipi-attachments")
        .unwrap();
    assert!(
        widget["widgetLines"][0]
            .as_str()
            .unwrap()
            .contains("[Image #1] shot.png")
    );

    let consumed = env
        .event(
            &ext,
            EventKind::OinoEditor,
            json!({"type": "oino_editor", "intent": "tokenBackspace", "text": "x [Image #1]"}),
        )
        .await;
    assert_eq!(consumed, Some(json!({"consume": true})));
    env.event(
        &ext,
        EventKind::OinoEditor,
        json!({"type": "oino_editor", "intent": "changed", "text": text}),
    )
    .await;

    let out = env
        .event(
            &ext,
            EventKind::Input,
            json!({"type": "input", "text": "look at [Image #1] please", "source": "interactive"}),
        )
        .await
        .unwrap();
    assert_eq!(out["action"], "transform");
    assert_eq!(out["images"][0]["mimeType"], "image/png");
    env.event(&ext, EventKind::AgentStart, json!({"type": "agent_start"}))
        .await;
    assert!(
        env.writes("appendEntry")
            .iter()
            .any(|w| w["appendEntry"] == "unipi:attachments"
                && w["data"]["items"][0]["name"] == "shot.png")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn render_specs_follow_the_style() {
    let env = Env::new();
    let (ext, _) = env.load(json!({})).await;
    let req = json!({"target": "tool", "key": "bash", "density": "regular", "expanded": false, "width": "normal", "data": {"args": {"command": "npm test"}, "result": {"content": [{"type": "text", "text": "ok"}]}}});
    let simple = ext
        .render(RawJson::from(req.clone()), env.cx(CallKind::Render))
        .unwrap()
        .to_value();
    assert_eq!(simple["t"], "text");
    let msg = json!({"target": "customMessage", "key": "unipi-watchdog", "density": "regular", "expanded": false, "width": "normal", "data": {"customType": "unipi-watchdog", "content": "slow tool"}});
    let card = ext
        .render(RawJson::from(msg), env.cx(CallKind::Render))
        .unwrap()
        .to_value();
    assert_eq!(card["rail"], "warning");

    let env2 = Env::new();
    let (ext2, _) = env2
        .load(json!({"utility": {"render": {"style": "regular"}}}))
        .await;
    assert!(
        ext2.render(RawJson::from(req), env2.cx(CallKind::Render))
            .is_none()
    );
    let style = ext2
        .handle_service(
            "utility.render-style",
            RawJson::from(json!({})),
            &ExtensionId::new("unipi.fusion").unwrap(),
            env2.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(style["style"], "regular");
    let panel = ext2
        .handle_service("utility.render-delegated", RawJson::from(json!({"steps": [{"kind": "text", "text": "hi"}], "label": "sidekick", "style": "simple"})), &ExtensionId::new("unipi.fusion").unwrap(), env2.cx(CallKind::Service))
        .await
        .unwrap()
        .to_value();
    assert_eq!(panel["t"], "card");
}

#[tokio::test(flavor = "multi_thread")]
async fn image_tools_against_a_local_endpoint_and_models_complete() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let seen: Arc<Mutex<Vec<String>>> = Arc::default();
    let log = Arc::clone(&seen);
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let log = Arc::clone(&log);
            tokio::spawn(async move {
                let mut buf = vec![0u8; 65_536];
                let mut got = Vec::new();
                loop {
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    got.extend_from_slice(&buf[..n]);
                    let text = String::from_utf8_lossy(&got).into_owned();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if body.len() >= len {
                            log.lock().unwrap().push(text.clone());
                            break;
                        }
                    }
                    if n == 0 {
                        break;
                    }
                }
                let body = json!({"data": [{"b64_json": "iVBORw0KGgo=", "revised_prompt": "a better cube"}]}).to_string();
                let resp = format!(
                    "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = sock.write_all(resp.as_bytes()).await;
            });
        }
    });
    let env = Env::new();
    let settings = json!({"image": {"generate": {"baseUrl": format!("http://127.0.0.1:{port}/v1"), "apiKey": "k", "api": "openai-images", "model": "flux"}}});
    let (ext, _) = env.load(settings).await;
    let r = env
        .tool(&ext, "image_generate", json!({"prompt": "A red cube!"}))
        .await
        .unwrap();
    let text = r["content"][0]["text"].as_str().unwrap();
    assert!(
        text.starts_with("Generated 1 image with flux @ http://127.0.0.1:"),
        "{text}"
    );
    assert!(text.contains("Revised prompt: a better cube"));
    assert_eq!(r["content"][1]["type"], "image");
    let saved = r["details"]["paths"][0].as_str().unwrap();
    assert!(
        saved.contains("unipi.utility") && saved.ends_with("-a-red-cube.png"),
        "{saved}"
    );
    assert!(std::path::Path::new(saved).exists());
    let req = seen.lock().unwrap()[0].clone();
    assert!(req.starts_with("POST /v1/images/generations"));
    assert!(req.to_lowercase().contains("authorization: bearer k"));

    if std::env::var_os("OPENROUTER_API_KEY").is_none() {
        let missing = env
            .tool(
                &ext,
                "image_generate",
                json!({"prompt": "p", "model": "openrouter/x"}),
            )
            .await;
        assert!(missing.is_err_and(|e| e.contains("No OpenRouter key")));
    }
    let unknown = env
        .tool(
            &ext,
            "image_generate",
            json!({"prompt": "p", "model": "nowhere/x"}),
        )
        .await;
    assert!(unknown.is_err_and(|e| e.contains("not configured in Oino")));

    // image_recognize through models.complete with a vision model.
    env.world
        .models
        .lock()
        .unwrap()
        .push(json!({"provider": "p", "id": "eyes", "input": ["text", "image"]}));
    *env.world.completion.lock().unwrap() = Some(
        json!({"role": "assistant", "content": [{"type": "text", "text": "A tiny PNG."}], "stopReason": "stop"}),
    );
    let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNkYPhfDwAChwGA60e6kgAAAABJRU5ErkJggg==";
    let r = env
        .tool(
            &ext,
            "image_recognize",
            json!({"image": format!("data:image/png;base64,{png}"), "model": "p/eyes"}),
        )
        .await
        .unwrap();
    assert_eq!(
        r["content"][0]["text"],
        "A tiny PNG.\n\n— analyzed with p/eyes"
    );
    let call = env.fired("harness.complete").pop().unwrap();
    assert_eq!(call["request"]["model"], "p/eyes");
    assert_eq!(
        call["request"]["context"]["messages"][0]["content"][0]["type"],
        "image"
    );
    let blind = env
        .tool(
            &ext,
            "image_recognize",
            json!({"image": format!("data:image/png;base64,{png}")}),
        )
        .await;
    assert!(blind.is_err_and(
        |e| e.contains("Unknown vision model") || e.contains("does not accept image input")
    ));

    // Gating: a text-only session model gets image_recognize back.
    env.event(&ext, EventKind::ModelSelect, json!({"type": "model_select", "model": {"provider": "p", "id": "eyes", "input": ["text", "image"]}})).await;
    let options = ext
        .handle_service(
            "image.models",
            RawJson::from(json!({})),
            &ExtensionId::new("oino").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert!(
        options
            .as_array()
            .unwrap()
            .iter()
            .any(|o| o["value"] == "p/eyes")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn move_dry_run_and_argument_errors() {
    let env = Env::new();
    let (ext, _) = env.load(json!({})).await;
    env.command(&ext, "move", "--bogus").await;
    assert!(
        env.fired("notify")
            .iter()
            .any(|n| n["message"] == "Unknown option: --bogus" && n["notifyType"] == "error")
    );
    env.command(&ext, "move", "/definitely/not/here/anymore --dry-run")
        .await;
    let notes: Vec<String> = env
        .fired("notify")
        .into_iter()
        .map(|n| n["message"].as_str().unwrap_or_default().to_owned())
        .collect();
    assert!(
        notes
            .iter()
            .any(|n| n.starts_with("Re-link /definitely/not/here/anymore → ")),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|n| n.starts_with("Dry run: ")),
        "{notes:?}"
    );
    let here = env.project.path().canonicalize().unwrap();
    env.command(&ext, "move", &here.display().to_string()).await;
    assert!(env.fired("notify").iter().any(|n| {
        n["message"].as_str().unwrap_or_default().contains("same")
            || n["message"]
                .as_str()
                .unwrap_or_default()
                .contains("still exists")
    }));
}
