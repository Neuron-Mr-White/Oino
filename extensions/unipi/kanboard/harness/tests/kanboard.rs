//! `unipi.kanboard` as a real component in the wasm host, driving the real `oino-kanboard`
//! binary (built by `run.sh` into the root `target/debug`).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    unsafe_code,
    reason = "tests assert by unwrapping and set process env before any host thread reads it"
)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionId,
    ExtensionInstance, ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
    Registration,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

fn root_dir() -> PathBuf {
    extensions_dir().join("..").canonicalize().unwrap()
}

fn built() -> &'static PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let ws = std::env::var("KANBOARD_EXT_WORKSPACE")
            .map_or_else(|_| extensions_dir(), PathBuf::from);
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(&ws).args([
            "build",
            "-j",
            "2",
            "--release",
            "--target",
            "wasm32-wasip2",
            "-p",
            "unipi-kanboard",
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
        extensions_dir().join("target/wasm32-wasip2/release/unipi_kanboard.wasm")
    })
}

struct Env {
    runtime: WasmRuntime,
    host: Arc<RecordingHostServices>,
    _agent: tempfile::TempDir,
    project: tempfile::TempDir,
    _home: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let bin = root_dir().join("target/debug/oino-kanboard");
        assert!(bin.exists(), "build oino-kanboard first (run.sh does)");
        let home = tempfile::tempdir().unwrap();
        // SAFETY: set before the runtime starts; tests run with --test-threads=1.
        unsafe {
            std::env::set_var("OINO_KANBOARD_BIN", &bin);
            std::env::set_var("OINO_KANBOARD_HOME", home.path());
            std::env::set_var("OINO_KANBOARD_NO_WATCH", "1");
            std::env::remove_var("OINO_KANBOARD_CHILD");
        }
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: RecordingHostServices::new(),
            _agent: agent,
            project,
            _home: home,
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

    async fn load(&self) -> (Arc<dyn ExtensionInstance>, Registration) {
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join("unipi/kanboard/extension.json"))
                .unwrap(),
        )
        .unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(built().clone()),
            root: None,
            bundled: true,
        };
        let host: Arc<dyn HostServices> = Arc::clone(&self.host) as Arc<dyn HostServices>;
        let ext = self.runtime.load(package, host).await.unwrap();
        let reg = ext
            .init(InitInput::default(), self.cx(CallKind::Init))
            .await
            .unwrap();
        (ext, reg)
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

    fn notices(&self) -> Vec<String> {
        self.host
            .calls_named("ui_fire")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["method"] == "notify")
            .map(|d| d["message"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    fn writes(&self, key: &str) -> Vec<Value> {
        self.host
            .calls_named("session_write")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d.get(key).is_some())
            .collect()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn registration() {
    let env = Env::new();
    let (_ext, reg) = env.load().await;
    let commands: Vec<(&str, bool)> = reg
        .commands
        .iter()
        .map(|c| (c.name.as_str(), c.hidden))
        .collect();
    assert_eq!(
        commands,
        [
            ("kanboard", false),
            ("kanboard-add", false),
            ("kanboard-do", false),
            ("kanboard-autowork", false),
            ("kanboard-open", true),
            ("kanboard-stop-daemon", true),
            ("kanboard-rotate-token", true),
        ]
    );
    assert!(reg.tools.is_empty());
    assert!(reg.arbiter.nudge_provider);
    for svc in [
        "kanboard.capture",
        "evidence.contribute",
        "compaction-context.provide",
        "move.handle",
    ] {
        assert!(reg.services.iter().any(|s| s == svc), "{svc}");
    }
    assert_eq!(reg.renderers.len(), 4);
    assert_eq!(reg.settings[0].to_value()["namespace"], "kanboard");
}

#[tokio::test(flavor = "multi_thread")]
async fn board_flow_guard_and_env() {
    let env = Env::new();
    let (ext, _) = env.load().await;
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;

    env.command(&ext, "kanboard", "onboard").await;
    assert!(
        env.notices().iter().any(|n| n.contains("registered")),
        "{:?}",
        env.notices()
    );

    env.command(&ext, "kanboard-add", "-p 4 Fix login\nsteps below")
        .await;
    let added = env
        .notices()
        .into_iter()
        .find(|n| n.starts_with('✓'))
        .unwrap();
    assert!(added.ends_with("-1 added to Backlog"), "{added}");
    let id = added
        .trim_start_matches("✓ ")
        .split(' ')
        .next()
        .unwrap()
        .to_owned();

    // No -do budget: a write through bash is refused, a read passes.
    let blocked = env
        .event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c1", "input": {"command": "oino-kanboard --actor agent add x"}}))
        .await
        .unwrap();
    assert_eq!(blocked["block"], true);
    assert!(
        blocked["reason"]
            .as_str()
            .unwrap()
            .starts_with("kanboard write budget used up")
    );
    let read = env
        .event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c2", "input": {"command": format!("oino-kanboard --actor agent show {id}")}}))
        .await
        .unwrap();
    let cmd = read["input"]["command"].as_str().unwrap();
    assert!(
        cmd.starts_with("export OINO_KANBOARD_SESSION='oino-s1' OINO_KANBOARD_PID="),
        "{cmd}"
    );
    assert!(
        cmd.ends_with(&format!("oino-kanboard --actor agent show {id}")),
        "{cmd}"
    );
    // Non-kanboard commands are untouched.
    assert!(env
        .event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c3", "input": {"command": "ls"}}))
        .await
        .is_none());

    // -do grants a budget and sends the harness prompt.
    env.command(&ext, "kanboard-do", "triage the board")
        .await;
    let sent = env.writes("sendUserMessage");
    let text = sent[0]["sendUserMessage"].as_str().unwrap();
    assert!(
        text.starts_with("[kanboard] For this request you may use the kanboard skill on project "),
        "{text}"
    );
    assert!(text.ends_with("Request: triage the board"));
    let allowed = env
        .event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c4", "input": {"command": "oino-kanboard --actor agent add x"}}))
        .await
        .unwrap();
    assert!(allowed.get("block").is_none(), "{allowed}");

    // Badge on a successful start.
    env.event(&ext, EventKind::ToolResult, json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c5", "input": {"command": format!("oino-kanboard start {id}")}, "content": [], "isError": false})).await;
    let badge = env
        .host
        .entries()
        .into_iter()
        .find(|e| e["customType"] == "unipi:kanboard-badge")
        .unwrap();
    assert_eq!(badge["data"]["kind"], "started");

    // In-chat board.
    env.command(&ext, "kanboard", "show --all").await;
    let shown = env.writes("sendMessage");
    let msg = shown
        .iter()
        .find(|m| m["sendMessage"]["customType"] == "unipi:kanboard-show")
        .unwrap();
    assert!(
        msg["sendMessage"]["content"]
            .as_str()
            .unwrap()
            .contains(&format!("{id} ↑ Fix login"))
    );
    let node = ext
        .render(RawJson::from(json!({"target": "customMessage", "key": "unipi:kanboard-show", "data": msg["sendMessage"]})), env.cx(CallKind::Render))
        .unwrap()
        .to_value();
    assert_eq!(node["t"], "stack");

    // Help is display-only and filtered from the context.
    env.command(&ext, "kanboard", "").await;
    let filtered = env
        .event(&ext, EventKind::Context, json!({"type": "context", "messages": [{"role": "custom", "customType": "unipi:kanboard-help"}, {"role": "user"}]}))
        .await
        .unwrap();
    assert_eq!(filtered["messages"].as_array().unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn capture_service_and_claims_nudge() {
    let env = Env::new();
    let (ext, _) = env.load().await;
    let refused = ext
        .handle_service(
            "kanboard.capture",
            RawJson::from(json!({"text": "later"})),
            &ExtensionId::new("unipi.input-shortcuts").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(refused["ok"], false);
    env.command(&ext, "kanboard", "onboard").await;
    let ok = ext
        .handle_service(
            "kanboard.capture",
            RawJson::from(json!({"text": "remember to rotate keys"})),
            &ExtensionId::new("unipi.input-shortcuts").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(ok["ok"], true, "{ok}");
    let id = ok["id"].as_str().unwrap().to_owned();

    // Claim it as this session through the binary (todo first), then settle: the monitor nudges.
    let bin = root_dir().join("target/debug/oino-kanboard");
    let run = |args: &[&str]| {
        let out = Command::new(&bin)
            .args(args)
            .current_dir(env.project.path())
            .env("OINO_KANBOARD_SESSION", "oino-s1")
            .env("OINO_KANBOARD_PID", std::process::id().to_string())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    run(&["move", &id, "todo"]);
    run(&["--actor", "agent", "start", &id]);
    env.event(&ext, EventKind::ToolResult, json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c1", "input": {"command": format!("oino-kanboard start {id}")}, "content": [], "isError": false})).await;
    env.event(&ext, EventKind::AgentStart, json!({"type": "agent_start"}))
        .await;
    env.event(&ext, EventKind::AgentEnd, json!({"type": "agent_end", "messages": [{"role": "assistant", "content": [{"type": "text", "text": "working on it"}], "stopReason": "stop"}]})).await;
    let nudge = ext
        .propose_nudge(env.cx(CallKind::Nudge))
        .await
        .unwrap()
        .to_value();
    assert_eq!(nudge["priority"], 50);
    assert_eq!(nudge["customType"], "unipi:kanboard-continue");
    assert!(
        nudge["content"]
            .as_str()
            .unwrap()
            .starts_with(&format!("↻ {id} still In Progress"))
    );

    let evidence = ext
        .handle_service(
            "evidence.contribute",
            RawJson::from(json!({})),
            &ExtensionId::new("unipi.long-horizon").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(evidence["blocking"].as_array().unwrap().len(), 1);

    // Shutdown releases the claim back to Todo.
    env.event(
        &ext,
        EventKind::SessionShutdown,
        json!({"type": "session_shutdown", "reason": "quit"}),
    )
    .await;
    let out = Command::new(&bin)
        .args(["show", &id, "--json"])
        .current_dir(env.project.path())
        .output()
        .unwrap();
    let task: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(task["status"], "todo");
}
