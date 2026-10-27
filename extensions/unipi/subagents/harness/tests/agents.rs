//! The unipi-agents components as real components in the wasm host: registration of every
//! package plus one end-to-end behaviour each (background task round trip with the
//! notification text, session caps, subagent tool errors, btw service, fusion gating and
//! autocomplete, a ralph loop through the gate).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson, Registration,
    ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

const CRATES: &[(&str, &str, &str)] = &[
    (
        "unipi-background-tasks",
        "background-tasks",
        "unipi_background_tasks",
    ),
    ("unipi-watchdog", "watchdog", "unipi_watchdog"),
    ("unipi-subagents", "subagents", "unipi_subagents"),
    ("unipi-btw", "btw", "unipi_btw"),
    ("unipi-fusion", "fusion", "unipi_fusion"),
    ("unipi-long-horizon", "long-horizon", "unipi_long_horizon"),
];

/// Builds every component once.
fn built() -> &'static PathBuf {
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
        ]);
        for (pkg, _, _) in CRATES {
            cmd.args(["-p", pkg]);
        }
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
        extensions_dir().join("target/wasm32-wasip2/release")
    })
}

struct Env {
    runtime: WasmRuntime,
    host: Arc<RecordingHostServices>,
    project: tempfile::TempDir,
    _agent: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: RecordingHostServices::new(),
            project,
            _agent: agent,
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

    async fn load(&self, dir: &str) -> (Arc<dyn ExtensionInstance>, Registration) {
        let (_, _, file) = CRATES.iter().find(|c| c.1 == dir).unwrap();
        let wasm = built().join(format!("{file}.wasm"));
        let manifest = Manifest::parse(
            &std::fs::read_to_string(
                extensions_dir()
                    .join("unipi")
                    .join(dir)
                    .join("extension.json"),
            )
            .unwrap(),
        )
        .unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(wasm),
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

    async fn tool(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, args: Value) -> Value {
        let inv = ToolInvocation {
            tool_call_id: format!("call-{name}"),
            name: name.into(),
            args: RawJson::from(args),
            parent_tool_call_id: None,
        };
        let mut cx = self.cx(CallKind::Tool);
        cx.tool_call_id = Some(inv.tool_call_id.clone());
        ext.run_tool(inv, CancellationToken::new(), cx)
            .await
            .unwrap()
            .to_value()
    }

    async fn command(
        &self,
        ext: &Arc<dyn ExtensionInstance>,
        name: &str,
        args: &str,
    ) -> Option<Value> {
        ext.run_command(
            name,
            args,
            CancellationToken::new(),
            self.cx(CallKind::Command),
        )
        .await
        .unwrap()
        .map(|v| v.to_value())
    }

    fn calls(&self, method: &str) -> Vec<Value> {
        self.host
            .calls_named(method)
            .into_iter()
            .map(|c| c.detail)
            .collect()
    }

    fn notifications(&self) -> Vec<String> {
        self.calls("ui_fire")
            .into_iter()
            .filter(|d| d["method"] == "notify")
            .filter_map(|d| d["message"].as_str().map(ToOwned::to_owned))
            .collect()
    }
}

fn names<T>(items: &[T], f: impl Fn(&T) -> &str) -> Vec<String> {
    items.iter().map(|i| f(i).to_owned()).collect()
}

fn text_of(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn background_tasks_round_trip() {
    let env = Env::new();
    let (bg, reg) = env.load("background-tasks").await;
    assert_eq!(
        names(&reg.tools, |t| &t.name),
        ["bg_run", "bg_status", "bg_logs", "bg_kill"]
    );
    assert_eq!(names(&reg.commands, |c| &c.name), ["bg", "bg-tasks"]);
    assert_eq!(reg.shortcuts[0].id, "unipi.bg.clear");
    assert_eq!(reg.shortcuts[0].default_keys, ["ctrl+alt+c"]);
    for s in ["work-tray.tab", "work-tray.action", "tasks.notification"] {
        assert!(reg.services.iter().any(|x| x == s), "{s}");
    }
    assert!(reg.arbiter.wait_source);

    let inv = ToolInvocation {
        tool_call_id: "c0".into(),
        name: "bg_run".into(),
        args: RawJson::from(json!({"name": "n", "command": "true"})),
        parent_tool_call_id: None,
    };
    let missing = bg
        .run_tool(inv, CancellationToken::new(), env.cx(CallKind::Tool))
        .await
        .unwrap_err();
    assert!(
        missing
            .message
            .starts_with("bg_run requires isAgent boolean."),
        "{}",
        missing.message
    );
    let started = env
        .tool(
            &bg,
            "bg_run",
            json!({"name": "Say hi", "command": "echo hi; exit 3", "isAgent": false}),
        )
        .await;
    let t = text_of(&started);
    assert!(t.starts_with("Started background task Say hi ("), "{t}");
    assert!(t.contains("Terminal notification: enabled.\nAutomatic follow-up turn: enabled."));
    let id = started["details"]["task"]["id"]
        .as_str()
        .unwrap()
        .to_owned();

    // The host delivers the completion through the extension's `tasks.notification` service.
    let mut notified = None;
    for _ in 0..100 {
        tokio::time::sleep(Duration::from_millis(100)).await;
        notified = env.calls("session_write").into_iter().find(|d| {
            d["sendMessage"]["content"]
                .as_str()
                .is_some_and(|c| c.starts_with("<background-task-notification>"))
        });
        if notified.is_some() {
            break;
        }
    }
    let msg = notified.expect("completion notification");
    let content = msg["sendMessage"]["content"].as_str().unwrap();
    assert!(content.contains(&format!("<task-id>{id}</task-id>")));
    assert!(content.contains("<status>failed</status>\n  <exit-code>3</exit-code>\n  <error>Exited with code 3</error>"), "{content}");
    assert!(
        content.contains("<summary>Background task &quot;Say hi&quot; failed</summary>")
            || content.contains("<summary>Background task \"Say hi\" failed</summary>")
    );
    assert_eq!(
        msg["options"]["trigger_turn"]
            .as_bool()
            .or(msg["options"]["triggerTurn"].as_bool()),
        Some(true)
    );

    let logs = env.tool(&bg, "bg_logs", json!({"taskId": id})).await;
    assert!(
        text_of(&logs).starts_with("hi\n\n\n[Full output: "),
        "{}",
        text_of(&logs)
    );
    let status = env.tool(&bg, "bg_status", json!({})).await;
    assert!(
        text_of(&status).starts_with(&format!("✗ {id} failed")),
        "{}",
        text_of(&status)
    );
    let inv = ToolInvocation {
        tool_call_id: "c9".into(),
        name: "bg_kill".into(),
        args: RawJson::from(json!({"taskId": id})),
        parent_tool_call_id: None,
    };
    let kill = bg
        .run_tool(inv, CancellationToken::new(), env.cx(CallKind::Tool))
        .await
        .unwrap_err();
    assert_eq!(kill.message, format!("Task {id} is failed, not running"));
    let tab = bg
        .handle_service(
            "work-tray.tab",
            RawJson::from(json!({"open": true})),
            &oino_extension_api::ExtensionId::new("unipi.core").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(tab[0]["id"], "bg");
    assert_eq!(tab[0]["counts"]["total"], 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn watchdog_session_caps() {
    let env = Env::new();
    let (wd, reg) = env.load("watchdog").await;
    assert_eq!(names(&reg.commands, |c| &c.name), ["bg-detach"]);
    assert_eq!(
        names(&reg.flags, |f| &f.name),
        ["max-turns", "max-cost", "max-wall", "max-tool-errors"]
    );
    for k in [
        EventKind::ToolExecutionStart,
        EventKind::ToolResult,
        EventKind::BeforeAgentStart,
        EventKind::TurnEnd,
        EventKind::OinoTimer,
    ] {
        assert!(reg.events.contains(&k), "{k:?}");
    }
    assert_eq!(reg.settings[0].to_value()["namespace"], "watchdog");

    env.host.set_flag("max-turns", json!("1"));
    env.event(&wd, EventKind::AgentStart, json!({"type": "agent_start"}))
        .await;
    env.event(&wd, EventKind::TurnEnd, json!({"type": "turn_end", "turnIndex": 0, "message": {"role": "assistant", "usage": {"cost": {"total": 0.01}}}, "toolResults": []})).await;
    let aborted = env
        .host
        .calls_named("control")
        .iter()
        .any(|c| c.detail == json!("abort"));
    assert!(aborted, "cap aborts the run");
    assert!(
        env.notifications()
            .iter()
            .any(|n| n == "watchdog: session cap reached — 1 turns"),
        "{:?}",
        env.notifications()
    );
    let summary = env
        .calls("session_write")
        .into_iter()
        .find(|d| d.get("sendUserMessage").is_some())
        .expect("summary turn");
    assert!(
        summary["sendUserMessage"]
            .as_str()
            .unwrap()
            .starts_with("Session cap reached (1 turns).")
    );

    env.command(&wd, "bg-detach", "").await;
    assert!(
        env.notifications()
            .iter()
            .any(|n| n.starts_with("Could not detach"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn subagents_tools_and_prompt() {
    let env = Env::new();
    let (sa, reg) = env.load("subagents").await;
    assert_eq!(
        names(&reg.tools, |t| &t.name),
        ["run_subagent", "read_subagent"]
    );
    assert!(
        reg.tools[0].parameters.to_value()["properties"]["profile"]["description"]
            .as_str()
            .unwrap()
            .contains("subagent_explore — Read-only codebase exploration and research.")
    );
    assert_eq!(names(&reg.commands, |c| &c.name), ["subagents", "agents"]);
    assert_eq!(reg.shortcuts[0].id, "unipi.subagents.foreground");
    for s in [
        "subagents.list",
        "subagents.graph-run",
        "subagents.register-external",
        "subagents.profiles",
    ] {
        assert!(reg.services.iter().any(|x| x == s), "{s}");
    }
    env.event(
        &sa,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let r = env
        .tool(
            &sa,
            "run_subagent",
            json!({"title": "t", "task": "x", "profile": "nope"}),
        )
        .await;
    assert_eq!(
        text_of(&r),
        "Unknown profile \"nope\". Available: subagent_explore, subagent_general"
    );
    let read = env.tool(&sa, "read_subagent", json!({})).await;
    assert_eq!(text_of(&read), "No subagent has run yet.");
    let out = env.event(&sa, EventKind::BeforeAgentStart, json!({"type": "before_agent_start", "prompt": "hi", "systemPrompt": "", "systemPromptOptions": {"sections": {}}})).await.unwrap();
    assert!(
        out["systemPromptOptions"]["sections"]["subagents"]
            .as_str()
            .unwrap()
            .starts_with("You can delegate self-contained subtasks")
    );
    let profiles = sa
        .handle_service(
            "subagents.profiles",
            RawJson::from(json!({})),
            &oino_extension_api::ExtensionId::new("unipi.long-horizon").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(profiles, json!(["subagent_explore", "subagent_general"]));
    let failed = env
        .tool(
            &sa,
            "run_subagent",
            json!({"title": "t", "task": "x", "profile": "subagent_explore"}),
        )
        .await;
    assert_eq!(
        failed["isError"], true,
        "the recording host has no nested agents: {failed}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn btw_and_fusion() {
    let env = Env::new();
    let (btw, reg) = env.load("btw").await;
    assert_eq!(names(&reg.commands, |c| &c.name), ["btw"]);
    assert!(reg.services.iter().any(|s| s == "btw.ask"));
    let list = btw
        .handle_service(
            "btw.list",
            RawJson::from(json!({})),
            &oino_extension_api::ExtensionId::new("x").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(list, json!([]));

    let (fusion, reg) = env.load("fusion").await;
    assert_eq!(names(&reg.tools, |t| &t.name), ["sidekick"]);
    assert!(!reg.tools[0].default_active);
    assert_eq!(
        names(&reg.commands, |c| &c.name),
        ["fusion", "fusion-stats", "fusion-preset"]
    );
    assert!(reg.commands[2].hidden);
    let r = env
        .tool(&fusion, "sidekick", json!({"message": "do it"}))
        .await;
    assert_eq!(
        text_of(&r),
        "Fusion is not active — pick a Fusion pair with /fusion."
    );
    let groups = fusion
        .complete_input(
            RawJson::from(json!({"text": "/mod", "cursor": 4})),
            env.cx(CallKind::Event),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(groups[0]["items"][0]["value"], "/fusion");
    env.command(&fusion, "fusion-stats", "").await;
    assert!(
        env.notifications()
            .iter()
            .any(|n| n.starts_with("Fusion is not active"))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn long_horizon_ralph_loop() {
    let env = Env::new();
    let (lh, reg) = env.load("long-horizon").await;
    assert_eq!(reg.tools.len(), 11);
    assert!(reg.tools.iter().all(|t| !t.default_active));
    let cmds = names(&reg.commands, |c| &c.name);
    for c in [
        "goal",
        "ralph",
        "swarm",
        "graph",
        "regular",
        "visualize-progress",
        "goal-start",
        "lh-graph",
    ] {
        assert!(cmds.iter().any(|x| x == c), "{c}");
    }
    assert!(reg.arbiter.nudge_provider);
    env.event(
        &lh,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;

    env.command(
        &lh,
        "ralph",
        "start Chores - [ ] one\\n- [ ] two\\n- [x] three",
    )
    .await;
    let file = env.project.path().join(".oino/ralph/chores.md");
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        "- [ ] one\n- [ ] two\n- [x] three"
    );
    let kickoff = env
        .calls("session_write")
        .into_iter()
        .find_map(|d| d["sendUserMessage"].as_str().map(ToOwned::to_owned))
        .expect("kickoff");
    assert!(
        kickoff.contains("🔄 RALPH LOOP: chores | Iteration 1"),
        "{kickoff}"
    );
    assert!(kickoff.contains("- [ ] one\n- [ ] two"));

    let gate = env.event(&lh, EventKind::BeforeAgentStart, json!({"type": "before_agent_start", "prompt": "go", "systemPrompt": "", "systemPromptOptions": {"sections": {}}})).await.unwrap();
    let section = gate["systemPromptOptions"]["sections"]["long-horizon"]
        .as_str()
        .unwrap();
    assert!(section.starts_with("Mode: Ralph — Work through a task file / checklist over many iterations.\nAvailable long-horizon tools: ralph_done, loop_status, todowrite."), "{section}");
    let blocked = env
        .event(
            &lh,
            EventKind::ToolCall,
            json!({"type": "tool_call", "toolCallId": "c", "toolName": "create_goal", "input": {}}),
        )
        .await
        .unwrap();
    assert_eq!(blocked["block"], true);

    std::fs::write(&file, "- [x] one\n- [ ] two\n- [x] three").unwrap();
    let done = env.tool(&lh, "ralph_done", json!({})).await;
    assert!(
        text_of(&done).starts_with("Iteration yielded."),
        "{}",
        text_of(&done)
    );
    let nudge = lh
        .propose_nudge(env.cx(CallKind::Nudge))
        .await
        .unwrap()
        .to_value();
    assert_eq!(nudge["priority"], 100);
    assert!(nudge["content"].as_str().unwrap().contains("Iteration 2"));
    let status = env.tool(&lh, "loop_status", json!({})).await;
    assert!(text_of(&status).contains("\"iteration\": 2"));
    let todo = env
        .tool(
            &lh,
            "todowrite",
            json!({"todos": [{"content": "a", "status": "in_progress", "priority": "high"}]}),
        )
        .await;
    assert_eq!(text_of(&todo), "Plan updated: 0/1 · ▶ a");

    env.command(&lh, "ralph", "stop").await;
    assert!(
        env.notifications()
            .iter()
            .any(|n| n.starts_with("Loop parked (iteration 2).")),
        "{:?}",
        env.notifications()
    );
    let state: Value = serde_json::from_slice(
        &std::fs::read(
            env._agent
                .path()
                .join("state/unipi.long-horizon/workspace")
                .read_dir()
                .unwrap()
                .next()
                .unwrap()
                .unwrap()
                .path()
                .join("state.json"),
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(state["parked"]["kind"], "ralph-loop");
    let runner = env.command(&lh, "goal-status", "{}").await.unwrap();
    assert_eq!(runner["found"], true);
    let _ = Path::new("/");
}
