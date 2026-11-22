//! `unipi.kanboard` as a real component in the wasm host, driving the real `unipi-kanboard`
//! release binary.

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
use oino_extension_api::ops::{ControlOp, ControlReply};
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind,
    ExtensionId, ExtensionInstance, ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode,
    RawJson, Registration, ToolInvocation,
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
        let mut cmd = Command::new("nice");
        cmd.args(["-n", "10", &cargo]);
        cmd.current_dir(&ws).args([
            "build",
            "-j",
            "4",
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
        cmd.env("CARGO_TARGET_DIR", root_dir().join("target"));
        let out = cmd.output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        root_dir().join("target/wasm32-wasip2/release/unipi_kanboard.wasm")
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
        // The extension spawns the binary through `process.exec` (ext-abi
        // WP-X3): the guest passes `UNIPI_KANBOARD_BIN` (the manifest grants the
        // env read), so every call uses the private release build.
        let bin = root_dir().join("target/release/unipi-kanboard");
        assert!(bin.exists(), "build unipi-kanboard first (run.sh does)");
        let fixtures =
            PathBuf::from(std::env::var("HOME").unwrap()).join(".cache/oino-work/kb-scope/harness");
        std::fs::create_dir_all(&fixtures).unwrap();
        let home = tempfile::tempdir_in(&fixtures).unwrap();
        // SAFETY: set before the runtime starts; tests run with --test-threads=1.
        unsafe {
            std::env::set_var("UNIPI_KANBOARD_BIN", &bin);
            std::env::set_var("UNIPI_KANBOARD_HOME", home.path());
            std::env::set_var("UNIPI_KANBOARD_NO_WATCH", "1");
            std::env::remove_var("UNIPI_KANBOARD_CHILD");
        }
        let agent = tempfile::tempdir_in(&fixtures).unwrap();
        let project = tempfile::tempdir_in(&fixtures).unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(root_dir().join("target/oino-wasm-test-cache"));
        let host = RecordingHostServices::new();
        host.allow_process_command(bin.to_str().unwrap());
        host.allow_process_command("unipi-kanboard");
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host,
            _agent: agent,
            project,
            _home: home,
        }
    }

    /// SIGKILL of the host process for the guest (core.md §2.1.8 restart gate): the runtime
    /// and its guest instances die; the kanboard CLI's locked board files (under
    /// `UNIPI_KANBOARD_HOME`), the agent/project dirs and the recorded host calls survive, and
    /// a fresh runtime is rebuilt over them. The process-global env vars stay as `new` set
    /// them.
    fn restart(&mut self) {
        let mut options =
            WasmRuntimeOptions::new(HostConfig::new(self._agent.path(), self.project.path()));
        options.cache_dir = Some(root_dir().join("target/oino-wasm-test-cache"));
        self.runtime = WasmRuntime::new(options).unwrap();
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

    fn child_cx(&self, kind: CallKind) -> CallContext {
        CallContext {
            session_id: "c1".into(),
            is_child: true,
            ..self.cx(kind)
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

    async fn skill_hidden(&self, ext: &Arc<dyn ExtensionInstance>) -> bool {
        ext.handle_service(
            "skill-condition.check",
            RawJson::from(json!({"skill":"kanboard"})),
            &ExtensionId::new("unipi.skill-registry").unwrap(),
            self.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value()["hidden"]
            .as_bool()
            .unwrap()
    }

    async fn active(&self) -> Vec<String> {
        match self
            .host
            .control(
                &ExtensionId::new("unipi.kanboard").unwrap(),
                &self.cx(CallKind::Event),
                ControlOp::ActiveTools,
            )
            .await
            .unwrap()
        {
            ControlReply::Strings(names) => names,
            other => panic!("unexpected active tools {other:?}"),
        }
    }

    fn cli(&self, args: &[&str]) -> Value {
        let out = Command::new(root_dir().join("target/release/unipi-kanboard"))
            .args(args)
            .arg("--json")
            .current_dir(self.project.path())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        serde_json::from_slice(&out.stdout).unwrap()
    }

    async fn onboard(&self, ext: &Arc<dyn ExtensionInstance>) {
        self.cli(&["project", "add"]);
        self.event(ext, EventKind::BeforeAgentStart, json!({"prompt": "hello"}))
            .await;
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

    async fn event_child(
        &self,
        ext: &Arc<dyn ExtensionInstance>,
        kind: EventKind,
        payload: Value,
    ) -> Option<Value> {
        ext.on_event(kind, RawJson::from(payload), self.child_cx(CallKind::Event))
            .await
            .unwrap()
            .map(|r| r.to_value())
    }

    async fn tool(&self, ext: &Arc<dyn ExtensionInstance>, args: Value) -> Value {
        ext.run_tool(
            ToolInvocation {
                tool_call_id: "t1".into(),
                name: "kanboard".into(),
                args: RawJson::from(args),
                parent_tool_call_id: None,
            },
            CancellationToken::new(),
            self.cx(CallKind::Tool),
        )
        .await
        .unwrap()
        .to_value()
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
            ("kanboard-open", true),
        ]
    );
    let tools: Vec<(&str, &str)> = reg
        .tools
        .iter()
        .map(|t| (t.name.as_str(), t.description.as_str()))
        .collect();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0].0, "kanboard");
    assert!(
        tools[0]
            .1
            .starts_with("Track work on this project's board."),
        "{}",
        tools[0].1
    );
    assert!(
        tools[0].1.ends_with("Set up boards from Superagent."),
        "{}",
        tools[0].1
    );
    let params = reg.tools[0].parameters.to_value();
    assert_eq!(params["required"], json!(["argv"]));
    assert_eq!(params["properties"]["argv"]["minItems"], 1);
    assert_eq!(params["additionalProperties"], false);
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

/// The extension's `kanboard` tool drives an existing board as this session,
/// stamps the ticket's origin, and refuses the flags the
/// tool sets itself and the CLI-only commands before anything runs.
#[tokio::test(flavor = "multi_thread")]
async fn tool_add_start_finish_against_the_real_binary() {
    let env = Env::new();
    let (ext, _) = env.load().await;
    env.onboard(&ext).await;

    let add = env
        .tool(
            &ext,
            json!({"argv": ["add", "Fix login", "--status", "todo"]}),
        )
        .await;
    assert_eq!(add["isError"], false, "{add}");
    let text = add["content"][0]["text"].as_str().unwrap();
    let task: Value = serde_json::from_str(text).unwrap();
    assert_eq!(task["status"], "todo");
    assert_eq!(task["origin"]["kind"], "chat");
    assert_eq!(task["origin"]["session"], "s1");
    let id = task["id"].as_str().unwrap().to_owned();

    let start = env.tool(&ext, json!({"argv": ["start", &id]})).await;
    assert_eq!(start["isError"], false, "{start}");
    let task: Value = serde_json::from_str(start["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(task["status"], "in_progress");
    assert!(task["run"].is_object(), "{task}");

    let finish = env
        .tool(
            &ext,
            json!({"argv": ["finish", &id, "--comment", "Fixed; tests added"]}),
        )
        .await;
    assert_eq!(finish["isError"], false, "{finish}");
    let task: Value = serde_json::from_str(finish["content"][0]["text"].as_str().unwrap()).unwrap();
    assert_eq!(task["status"], "in_review");
    assert_eq!(task["origin"]["kind"], "chat");

    // The board is real on disk: the CLI agrees after the tool's writes.
    let out = Command::new(root_dir().join("target/release/unipi-kanboard"))
        .args(["show", &id, "--json"])
        .current_dir(env.project.path())
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let shown: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(shown["status"], "in_review");

    // Overrides and CLI-only commands never reach the binary.
    let bad = env
        .tool(&ext, json!({"argv": ["list", "--project", "x"]}))
        .await;
    assert_eq!(bad["isError"], true, "{bad}");
    assert_eq!(
        bad["content"][0]["text"],
        "Don't pass --project; identity and project are set for you (use the project field for another board)."
    );
    let only = env.tool(&ext, json!({"argv": ["validate"]})).await;
    assert_eq!(only["isError"], true, "{only}");
    assert_eq!(
        only["content"][0]["text"],
        "`validate` is CLI-only; ask the owner to run it."
    );
    let only = env
        .tool(&ext, json!({"argv": ["project", "archive", "x"]}))
        .await;
    assert_eq!(
        only["content"][0]["text"],
        "`project archive` is CLI-only; ask the owner to run it."
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn bash_many_adds_are_not_blocked() {
    let env = Env::new();
    let (ext, _) = env.load().await;
    env.onboard(&ext).await;
    // 21+ `add`s in one bash call: nothing blocks the call — no per-turn add
    // limit and no write budget is left.
    let many = (0..21)
        .map(|_| "unipi-kanboard add t".to_owned())
        .collect::<Vec<_>>()
        .join(" && ");
    let result = env
        .event(
            &ext,
            EventKind::ToolCall,
            json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c1", "input": {"command": many}}),
        )
        .await
        .unwrap();
    assert_eq!(result["block"], json!(null), "{result}");
    assert_eq!(result["reason"], json!(null), "{result}");
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

    env.onboard(&ext).await;
    assert!(
        env.active().await.contains(&"kanboard".to_owned()),
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

    // No budget: a write through bash passes, with the session env prefix, and so does a
    // child session's write (subagents work the board too).
    let write = env
        .event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c1", "input": {"command": "unipi-kanboard --actor agent add x"}}))
        .await
        .unwrap();
    assert!(write.get("block").is_none(), "{write}");
    let cmd = write["input"]["command"].as_str().unwrap();
    assert!(
        cmd.starts_with("export UNIPI_KANBOARD_SESSION='oino-s1' UNIPI_KANBOARD_PID="),
        "{cmd}"
    );
    assert!(cmd.ends_with("unipi-kanboard --actor agent add x"), "{cmd}");
    env.event_child(
        &ext,
        EventKind::BeforeAgentStart,
        json!({"prompt": "hello"}),
    )
    .await;
    let child = env
        .event_child(
            &ext,
            EventKind::ToolCall,
            json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c1c", "input": {"command": format!("unipi-kanboard --actor agent note {id} hi")}}),
        )
        .await
        .unwrap();
    assert!(
        child.get("block").is_none(),
        "a child session writes too: {child}"
    );
    let cmd = child["input"]["command"].as_str().unwrap();
    assert!(
        cmd.starts_with("export UNIPI_KANBOARD_SESSION='oino-c1' UNIPI_KANBOARD_PID="),
        "{cmd}"
    );
    let read = env
        .event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c2", "input": {"command": format!("unipi-kanboard --actor agent show {id}")}}))
        .await
        .unwrap();
    let cmd = read["input"]["command"].as_str().unwrap();
    assert!(
        cmd.starts_with("export UNIPI_KANBOARD_SESSION='oino-s1' UNIPI_KANBOARD_PID="),
        "{cmd}"
    );
    assert!(
        cmd.ends_with(&format!("unipi-kanboard --actor agent show {id}")),
        "{cmd}"
    );
    // Non-kanboard commands are untouched.
    assert!(env
        .event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c3", "input": {"command": "ls"}}))
        .await
        .is_none());

    // Retired commands are absent; historical badge/render records remain supported.
    assert!(
        !env.writes("sendUserMessage")
            .iter()
            .any(|v| v.to_string().contains("For this request"))
    );

    // Badge on a successful start.
    env.event(&ext, EventKind::ToolResult, json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c5", "input": {"command": format!("unipi-kanboard start {id}")}, "content": [], "isError": false})).await;
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
    assert_eq!(node["t"], "Layout");

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
            &ExtensionId::new("unipi.btw").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(refused["ok"], false);
    env.onboard(&ext).await;
    let ok = ext
        .handle_service(
            "kanboard.capture",
            RawJson::from(json!({"text": "remember to rotate keys"})),
            &ExtensionId::new("unipi.btw").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(ok["ok"], true, "{ok}");
    let id = ok["id"].as_str().unwrap().to_owned();

    // Claim it as this session through the binary (todo first), then settle: the monitor nudges.
    let bin = root_dir().join("target/release/unipi-kanboard");
    let run = |args: &[&str]| {
        let out = Command::new(&bin)
            .args(args)
            .current_dir(env.project.path())
            .env("UNIPI_KANBOARD_SESSION", "oino-s1")
            .env("UNIPI_KANBOARD_PID", std::process::id().to_string())
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
    env.event(&ext, EventKind::ToolResult, json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c1", "input": {"command": format!("unipi-kanboard start {id}")}, "content": [], "isError": false})).await;
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

/// WP-X3: without `UNIPI_KANBOARD_BIN`, the host's process service resolves the
/// bare `unipi-kanboard` name against the calling package's `bin/` dir — the
/// shipped-binary path (`OINO_EXT_BIN_DIR` → package `bin/` → `PATH`).
#[tokio::test(flavor = "multi_thread")]
async fn spawn_resolves_the_package_bin_without_the_env_override() {
    let env = Env::new();
    // A package root shipping this platform's binary (the dist layout).
    let pkg = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(pkg.path().join("bin")).unwrap();
    std::fs::copy(
        root_dir().join("target/release/unipi-kanboard"),
        pkg.path().join("bin/unipi-kanboard"),
    )
    .unwrap();
    // SAFETY: the harness runs with --test-threads=1; restored before the
    // assertions so later tests keep the debug-build override.
    unsafe {
        std::env::remove_var("UNIPI_KANBOARD_BIN");
    }
    let manifest = Manifest::parse(
        &std::fs::read_to_string(extensions_dir().join("unipi/kanboard/extension.json")).unwrap(),
    )
    .unwrap();
    let package = LoadedPackage {
        manifest,
        component: ComponentSource::Path(built().clone()),
        root: Some(pkg.path().to_path_buf()),
        bundled: true,
    };
    env.host
        .allow_process_command(pkg.path().join("bin/unipi-kanboard").to_str().unwrap());
    let host: Arc<dyn HostServices> = Arc::clone(&env.host) as Arc<dyn HostServices>;
    let ext = env.runtime.load(package, host).await.unwrap();
    ext.init(InitInput::default(), env.cx(CallKind::Init))
        .await
        .unwrap();
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    env.onboard(&ext).await;
    // SAFETY: restore before any assertion can early-return.
    unsafe {
        std::env::set_var(
            "UNIPI_KANBOARD_BIN",
            root_dir().join("target/release/unipi-kanboard"),
        );
    }
    assert!(
        env.active().await.contains(&"kanboard".to_owned()),
        "the extension onboarded through the package's bin/ binary: {:?}",
        env.notices()
    );
}

/// §5.3: when nothing resolves (no env override, no package binary, not on
/// PATH), a spawn failure surfaces unipi's `unavailableMessage` instead of a
/// raw I/O error — the tools stay usable and say why.
#[tokio::test(flavor = "multi_thread")]
async fn missing_binary_surfaces_the_unavailable_message() {
    let env = Env::new();
    // SAFETY: the harness runs with --test-threads=1; restored before the
    // assertions so later tests keep the debug-build override.
    unsafe {
        std::env::remove_var("UNIPI_KANBOARD_BIN");
    }
    let manifest = Manifest::parse(
        &std::fs::read_to_string(extensions_dir().join("unipi/kanboard/extension.json")).unwrap(),
    )
    .unwrap();
    let package = LoadedPackage {
        manifest,
        component: ComponentSource::Path(built().clone()),
        root: None,
        bundled: true,
    };
    let host: Arc<dyn HostServices> = Arc::clone(&env.host) as Arc<dyn HostServices>;
    let ext = env.runtime.load(package, host).await.unwrap();
    ext.init(InitInput::default(), env.cx(CallKind::Init))
        .await
        .unwrap();
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    env.command(&ext, "kanboard-add", "later").await;
    // SAFETY: restore before any assertion can early-return.
    unsafe {
        std::env::set_var(
            "UNIPI_KANBOARD_BIN",
            root_dir().join("target/release/unipi-kanboard"),
        );
    }
    let expected = "This project has no board. Set one up from Superagent.";
    assert!(
        env.writes("sendMessage").iter().any(|n| n.to_string().contains(expected)),
        "expected the persistent unavailable message, got {:?}",
        env.writes("sendMessage")
    );
}

/// The §2.1.8 restart gate for kanboard: session state (guard budget, monitor, tracker) is
/// memory-only and safely resets; the board itself is durable in the kanboard CLI's locked
/// files. A scripted session is SIGKILLed mid-turn with a card In Progress ([`Env::restart`]);
/// after the restart the session re-registers, the write gate is armed fresh, and the
/// monitor's nudge is rebuilt from the board — the interrupted card is still In Progress on
/// disk (a hard kill never ran `session_shutdown`, so the claim survives).
#[allow(
    clippy::too_many_lines,
    reason = "one scripted crash, board + gate + nudge"
)]
#[tokio::test(flavor = "multi_thread")]
async fn kill_and_restart_rebuilds_from_the_board() {
    let mut env = Env::new();
    let bin = root_dir().join("target/release/unipi-kanboard");
    let run = |args: &[&str]| {
        let out = Command::new(&bin)
            .args(args)
            .current_dir(env.project.path())
            .env("UNIPI_KANBOARD_SESSION", "oino-s1")
            .env("UNIPI_KANBOARD_PID", std::process::id().to_string())
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    let id = {
        let (ext, _) = env.load().await;
        env.event(
            &ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
        env.onboard(&ext).await;
        env.command(&ext, "kanboard-add", "-p 4 Fix login\nsteps below")
            .await;
        let added = env
            .notices()
            .into_iter()
            .find(|n| n.starts_with('✓'))
            .unwrap();
        let id = added
            .trim_start_matches("✓ ")
            .split(' ')
            .next()
            .unwrap()
            .to_owned();
        // Mid-turn: the card is claimed In Progress when the process dies.
        run(&["move", &id, "todo"]);
        run(&["--actor", "agent", "start", &id]);
        id
    };

    env.restart();
    let (ext, _) = env.load().await;
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    assert!(
        env.active().await.contains(&"kanboard".to_owned()),
        "the restarted session resolves the existing active board"
    );
    // No budget survives or is needed: the restarted session can write through bash again.
    let write = env
        .event(
            &ext,
            EventKind::ToolCall,
            json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c1", "input": {"command": "unipi-kanboard --actor agent add x"}}),
        )
        .await
        .unwrap();
    assert!(
        write.get("block").is_none(),
        "the restarted session writes: {write}"
    );
    // The board is durable: the interrupted card is still In Progress, and the monitor's
    // nudge is rebuilt from it.
    let out = Command::new(&bin)
        .args(["show", &id, "--json"])
        .current_dir(env.project.path())
        .output()
        .unwrap();
    let task: Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        task["status"], "in_progress",
        "a hard kill never released the claim: {task}"
    );
    env.event(
        &ext,
        EventKind::ToolResult,
        json!({"type": "tool_result", "toolName": "bash", "toolCallId": "c2", "input": {"command": format!("unipi-kanboard start {id}")}, "content": [], "isError": false}),
    )
    .await;
    env.event(&ext, EventKind::AgentStart, json!({"type": "agent_start"}))
        .await;
    env.event(&ext, EventKind::AgentEnd, json!({"type": "agent_end", "messages": [{"role": "assistant", "content": [{"type": "text", "text": "working on it"}], "stopReason": "stop"}]})).await;
    let nudge = ext
        .propose_nudge(env.cx(CallKind::Nudge))
        .await
        .unwrap()
        .to_value();
    assert_eq!(nudge["customType"], "unipi:kanboard-continue");
    assert!(
        nudge["content"]
            .as_str()
            .unwrap()
            .starts_with(&format!("↻ {id} still In Progress")),
        "the monitor rebuilt from the durable board: {}",
        nudge["content"]
    );
}

/// Live project state, not cached onboarding, controls every next turn.
#[tokio::test(flavor = "multi_thread")]
async fn board_scope_tracks_creation_archive_and_unarchive() {
    let env = Env::new();
    env.host.set_active_tools(&["read", "bash", "kanboard"]);
    let (ext, _) = env.load().await;
    env.event(&ext, EventKind::SessionStart, json!({})).await;
    assert_eq!(env.active().await, ["read", "bash"]);
    assert!(env.skill_hidden(&ext).await);
    let refused = env
        .tool(&ext, json!({"argv": ["add", "must not create"]}))
        .await;
    assert_eq!(refused["isError"], true);
    assert!(env.cli(&["project", "list"]).as_array().unwrap().is_empty());
    env.command(&ext, "kanboard-add", "also must not create")
        .await;
    assert!(env.writes("sendMessage").iter().any(|v| v.to_string().contains("This project has no board. Set one up from Superagent.")));
    env.command(&ext, "kanboard", "status").await;
    let notices = env.writes("sendMessage").len();
    assert_eq!(notices, 2, "both typed commands leave a persistent reply");
    assert!(
        env.host.entries().is_empty()
            && env.writes("sendMessage").len() == notices
            && env.writes("appendReminder").is_empty(),
        "no board transcript injections"
    );

    let project = env.cli(&["project", "add"]);
    let slug = project["slug"].as_str().unwrap();
    ext.handle_service("kanboard.scope", json!({}).into(),
        &ExtensionId::new("test.ui").unwrap(), env.cx(CallKind::Service)).await.unwrap();
    assert!(env.active().await.contains(&"kanboard".to_owned()));
    assert!(!env.skill_hidden(&ext).await);
    assert_eq!(
        env.tool(&ext, json!({"argv": ["add", "works"]})).await["isError"],
        false
    );
    env.cli(&["project", "archive", slug]);
    ext.handle_service("kanboard.scope", json!({}).into(),
        &ExtensionId::new("test.ui").unwrap(), env.cx(CallKind::Service)).await.unwrap();
    assert_eq!(env.active().await, ["read", "bash"]);
    assert!(env.skill_hidden(&ext).await);
    env.command(&ext, "kanboard", "status").await;
    assert_eq!(env.writes("sendMessage").len(), notices + 1);
    let notices = notices + 1;
    env.event(&ext, EventKind::ToolResult, json!({"toolName": "bash", "input": {"command": "unipi-kanboard start XX-1"}, "isError": false})).await;
    assert!(
        env.host.entries().is_empty()
            && env.writes("sendMessage").len() == notices
            && env.writes("appendReminder").is_empty(),
        "no badge or reminder on archived board"
    );
    env.cli(&["project", "unarchive", slug]);
    ext.handle_service("kanboard.scope", json!({}).into(),
        &ExtensionId::new("test.ui").unwrap(), env.cx(CallKind::Service)).await.unwrap();
    assert!(env.active().await.contains(&"kanboard".to_owned()));
    env.host.seed_entries(vec![
        json!({"type":"session", "rootCwd": env.project.path()}),
    ]);
    let mut child = env.child_cx(CallKind::Event);
    let elsewhere = tempfile::tempdir_in(env._home.path()).unwrap();
    child.cwd = elsewhere.path().to_path_buf();
    ext.on_event(
        EventKind::BeforeAgentStart,
        RawJson::from(json!({"prompt":"child"})),
        child,
    )
    .await
    .unwrap();
    assert!(env.active().await.contains(&"kanboard".to_owned()));
}

/// Local read-only scope refresh latency, including the real CLI subprocess.
#[tokio::test(flavor = "multi_thread")]
async fn scope_refresh_measurement() {
    let env = Env::new();
    let (ext, _) = env.load().await;
    env.event(&ext, EventKind::SessionStart, json!({})).await;
    let caller = ExtensionId::new("test.ui").unwrap();
    for state in ["none", "active", "archived"] {
        if state == "active" { env.cli(&["project", "add"]); }
        if state == "archived" {
            let project = env.cli(&["project", "show"]);
            env.cli(&["project", "archive", project["project"]["slug"].as_str().unwrap()]);
        }
        let mut ms = Vec::new();
        for _ in 0..50 {
            let start = std::time::Instant::now();
            let result = ext.handle_service("kanboard.scope", json!({}).into(), &caller,
                env.cx(CallKind::Service)).await.unwrap().to_value();
            ms.push(start.elapsed().as_secs_f64() * 1000.0);
            assert_eq!(result["available"], state == "active");
        }
        ms.sort_by(f64::total_cmp);
        println!("KB1_SCOPE {state} n=50 p50_ms={:.3} p95_ms={:.3} max_ms={:.3}", ms[24], ms[47], ms[49]);
    }
}
