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
use std::time::{Duration, Instant};

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
        let runtime = Self::runtime(agent.path(), project.path());
        Self {
            runtime,
            host: RecordingHostServices::new(),
            project,
            _agent: agent,
        }
    }

    fn runtime(agent: &Path, project: &Path) -> WasmRuntime {
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent, project));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        WasmRuntime::new(options).unwrap()
    }

    /// SIGKILL of the host process for the guest: the runtime (guest instances, epoch
    /// ticker, in-memory hub) dies and is rebuilt over the same agent and project dirs —
    /// everything on disk (kv, workspaces, task store) and the recorded session entries
    /// survive exactly as across a real process kill (core.md §2.1.8 restart gate).
    fn restart(&mut self) {
        let agent = self._agent.path().to_path_buf();
        let project = self.project.path().to_path_buf();
        self.runtime = Self::runtime(&agent, &project);
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
        let host: Arc<dyn HostServices> = Arc::clone(&self.host) as Arc<dyn HostServices>;
        self.load_with(dir, host).await
    }

    /// [`Self::load`] around another host: a second services set (another cwd) loading
    /// the same extension into the same process-wide hub.
    async fn load_with(
        &self,
        dir: &str,
        host: Arc<dyn HostServices>,
    ) -> (Arc<dyn ExtensionInstance>, Registration) {
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
    // The recording host now spawns fake nested agents (one prompt, clean completion):
    // the run completes with the staged report instead of failing to spawn.
    assert_eq!(
        failed["isError"], false,
        "the fake nested agent completes: {failed}"
    );
    assert_eq!(failed["details"]["status"], json!("completed"), "{failed}");
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

/// Graph mode cold start: the first add_work opens a graph and spawns the first worker;
/// the graph is persisted before that spawn, and the worker's child session start
/// (the real host delivers it to the same extension instance ~50 ms later) must not
/// wipe the owner's in-memory graph. Regression for add-work answering "no graph"
/// right after opening one (reported as "subagents disabled" by the model).
#[tokio::test(flavor = "multi_thread")]
async fn long_horizon_graph_survives_child_session_start() {
    let env = Env::new();
    let (sa, _) = env.load("subagents").await;
    let (lh, _) = env.load("long-horizon").await;
    for ext in [&sa, &lh] {
        env.event(
            ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
    }

    // Fresh graph with one work item: opens the graph and starts the first worker
    // (the recording host cannot nest sessions — the start fails and the step parks,
    // which is fine for this assertion).
    let opened = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({
                "operation": "add_work",
                "task": "Demo graph",
                "add_work": [
                    {"work_id": "w1", "instruction": "map the code", "profile": "subagent_explore"}
                ]
            }),
        )
        .await;
    let t = text_of(&opened);
    assert!(!t.contains("no graph"), "{t}");
    assert!(t.starts_with("Opened graph"), "{t}");

    // Persisted before the spawn: graph.json on disk already carries the fresh graph.
    let ws_dir = env._agent.path().join("state/unipi.long-horizon/workspace");
    let graph_json = std::fs::read_dir(&ws_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("graph.json");
    let disk: Value = serde_json::from_slice(&std::fs::read(&graph_json).unwrap()).unwrap();
    let graph_id = disk["current"]["graphId"].as_str().unwrap().to_owned();
    assert_eq!(disk["current"]["task"], "Demo graph");
    assert_eq!(disk["current"]["work"][0]["workId"], "w1");

    // The worker's child session start lands on the same instance. The workspace
    // copy may lag memory mid-run — here it is explicitly stale (pre-fix the child's
    // reload-from-disk wiped the fresh graph out of memory).
    std::fs::write(&graph_json, r#"{"v":1,"history":[]}"#).unwrap();
    let mut child = env.cx(CallKind::Event);
    child.is_child = true;
    child.session_id = "child-1".into();
    lh.on_event(
        EventKind::SessionStart,
        RawJson::from(json!({"type": "session_start", "reason": "new"})),
        child,
    )
    .await
    .unwrap_or_else(|e| panic!("child session start: {e}"));

    // The owner's graph survived the child session start: adding work keeps the SAME
    // graph (same id, revision bump) instead of reporting "no graph" or opening a new one.
    let more = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({
                "operation": "add_work",
                "add_work": [
                    {"work_id": "w2", "instruction": "write it up", "profile": "subagent_general"}
                ]
            }),
        )
        .await;
    let t = text_of(&more);
    assert!(!t.contains("no graph"), "{t}");
    assert!(t.contains(&graph_id), "same graph survives: {t}");
    let disk: Value = serde_json::from_slice(&std::fs::read(&graph_json).unwrap()).unwrap();
    assert_eq!(disk["current"]["graphId"], graph_id);
    let ids: Vec<&str> = disk["current"]["work"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|w| w["workId"].as_str())
        .collect();
    assert!(ids.contains(&"w1") && ids.contains(&"w2"), "{disk}");
}

/// F-06: the lead's long-horizon mode survives a child worker's turn. Before the fix the
/// shared per-instance mode state was clobbered by the child's gate (children resolve
/// `none`), so the lead's `yield_agent_graph` / `view_agent_graph` were refused mid-graph
/// ("not available in none mode"). Now the mode state is per session: the lead stays in
/// graph mode while its worker runs.
#[tokio::test(flavor = "multi_thread")]
async fn lead_keeps_graph_mode_while_a_child_turn_runs() {
    let env = Env::new();
    let (sa, _) = env.load("subagents").await;
    let (lh, _) = env.load("long-horizon").await;
    for ext in [&sa, &lh] {
        env.event(
            ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
    }

    // The lead opens a graph: `/graph …` marks the chat's mode explicit-graph.
    env.command(&lh, "graph", "Demo graph").await;
    let lead = env.cx(CallKind::Event);
    let resolved = lh
        .on_event(
            EventKind::BeforeAgentStart,
            RawJson::from(json!({
                "type": "before_agent_start",
                "prompt": "run it",
                "systemPrompt": "",
                "systemPromptOptions": {}
            })),
            lead,
        )
        .await
        .unwrap_or_else(|e| panic!("lead turn start: {e}"));
    let resolved = resolved.map(|r| r.to_value()).unwrap_or_default();
    let fragment = resolved["systemPromptOptions"]["sections"]["long-horizon"]
        .as_str()
        .unwrap_or_default();
    assert!(
        fragment.contains("Mode: Graph"),
        "the lead turn starts in graph mode: {fragment}"
    );

    // A child worker's turn starts on the same instance (session_start + gate).
    let child = {
        let mut cx = env.cx(CallKind::Event);
        cx.is_child = true;
        cx.session_id = "child-1".into();
        cx
    };
    lh.on_event(
        EventKind::SessionStart,
        RawJson::from(json!({"type": "session_start", "reason": "new"})),
        child.clone(),
    )
    .await
    .unwrap_or_else(|e| panic!("child session start: {e}"));
    let child_resolved = lh
        .on_event(
            EventKind::BeforeAgentStart,
            RawJson::from(json!({
                "type": "before_agent_start",
                "prompt": "do the work",
                "systemPrompt": "",
                "systemPromptOptions": {}
            })),
            child,
        )
        .await
        .unwrap_or_else(|e| panic!("child turn start: {e}"));
    let child_value = child_resolved
        .map(|r| r.to_value())
        .unwrap_or_default();
    let child_fragment = child_value["systemPromptOptions"]["sections"]["long-horizon"]
        .as_str()
        .unwrap_or_default();
    assert!(
        child_fragment.contains("Mode: Plain"),
        "children run plain: {child_fragment}"
    );

    // The lead's graph control tools still work mid-graph (pre-fix: refused in "none
    // mode" because the child's gate clobbered the shared turn mode).
    for name in ["view_agent_graph", "yield_agent_graph"] {
        let blocked = env
            .event(
                &lh,
                EventKind::ToolCall,
                json!({
                    "type": "tool_call",
                    "toolName": name,
                    "toolCallId": format!("t-{name}"),
                    "input": {}
                }),
            )
            .await;
        assert!(
            blocked.is_none(),
            "{name} must not be refused while a child runs: {blocked:?}"
        );
    }
}

/// The §2.1.8 restart gate for this suite's components, at both kill levels:
///
/// * instance kill (the session-eviction path: the guest's WASM state dies, the host process,
///   its task store and the session file live on) — background-tasks rebuilds its `local`
///   map from the `unipi:bg-tasks` entry and merges it with the still-alive host task list;
/// * process kill ([`Env::restart`]: runtime, guest instances and the in-memory hub die; the
///   agent dir and the session entries survive) — subagents rebuilds its records from the run
///   index (Running → Failed with the repair-compatible text and a cold-resume hint when the
///   child session file exists), fusion re-attaches its pairing from `unipi:fusion-sidekick`,
///   and every extension stays usable. (The host task store itself is per-process today: its
///   `tasks.json` is written but not re-read — a known gap outside this suite.)
#[allow(
    clippy::too_many_lines,
    reason = "one scripted crash, three extensions"
)]
#[tokio::test(flavor = "multi_thread")]
async fn kill_and_restart_restores_extension_state() {
    let mut env = Env::new();
    let agent = env._agent.path().to_path_buf();
    // ── the scripted session (what the process leaves behind when it dies) ────────────
    let (sleep_id, quick_id) = {
        let (bg, _) = env.load("background-tasks").await;
        // A running task and a finished one whose notification was already delivered.
        let sleep = env
            .tool(
                &bg,
                "bg_run",
                json!({"name": "Long build", "command": "sleep 30", "isAgent": false}),
            )
            .await;
        let sleep_id = sleep["details"]["task"]["id"].as_str().unwrap().to_owned();
        let quick = env
            .tool(
                &bg,
                "bg_run",
                json!({"name": "Quick echo", "command": "echo hi", "isAgent": false}),
            )
            .await;
        let quick_id = quick["details"]["task"]["id"].as_str().unwrap().to_owned();
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
        notified.expect("quick task completion notification");
        // The settle rewrite of the durable snapshot also runs clean.
        env.event(&bg, EventKind::TurnEnd, json!({"type": "turn_end"}))
            .await;
        let entry = env
            .host
            .entries()
            .into_iter()
            .rev()
            .find(|e| e["customType"] == "unipi:bg-tasks")
            .expect("unipi:bg-tasks entry appended");
        assert_eq!(
            entry["data"]["local"][&quick_id]["notified"], true,
            "the durable local map carries the delivered notification"
        );
        assert!(
            entry["data"]["local"]
                .as_object()
                .unwrap()
                .contains_key(&sleep_id)
        );

        // The previous process died while a subagent ran: index + child file on disk.
        let (sa, _) = env.load("subagents").await;
        env.event(
            &sa,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
        let ws_root = agent.join("state/unipi.subagents/workspace");
        let index_dir = std::fs::read_dir(&ws_root)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path()
            .join("sessions/s1");
        std::fs::write(
            index_dir.join("index.json"),
            serde_json::to_vec(&json!([{
                "id": "ab12cd34", "title": "Map auth", "profile": "subagent_explore",
                "model": "faux/faux-1", "status": "running", "background": true,
                "startedAt": 1000, "toolCalls": 3, "lastActivity": 5000,
                "task": "map the auth flows",
                "sessionFile": "/ext-workspace/x/sessions/s1/ab12cd34.jsonl", "depth": 1
            }]))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            index_dir.join("ab12cd34.jsonl"),
            "{\"type\":\"session\",\"version\":3,\"id\":\"child\"}\n",
        )
        .unwrap();
        // And while a Fusion pair was active (the crashed process appended this).
        env.host.seed_entries(vec![json!({
            "type": "custom", "customType": "unipi:fusion-sidekick",
            "data": {"active": true, "selection": {
                "kind": "fusion", "lead": "faux/lead", "sidekick": "faux/cheap",
                "leadEffort": "high", "sidekickEffort": "low"
            }}
        })]);
        (sleep_id, quick_id)
    };

    // ── instance kill: guest state dies, the host task store lives on ────────────────
    {
        let (bg, _) = env.load("background-tasks").await;
        env.event(
            &bg,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
        let status = env.tool(&bg, "bg_status", json!({})).await;
        let tasks = status["details"]["tasks"].as_array().unwrap().clone();
        let find = |id: &str| {
            tasks
                .iter()
                .find(|t| t["id"].as_str().is_some_and(|t| t.starts_with(id)))
                .unwrap_or_else(|| panic!("task {id} in {tasks:?}"))
                .clone()
        };
        assert_eq!(
            find(&quick_id)["notified"],
            true,
            "local map restored from unipi:bg-tasks and merged with the host list"
        );
        assert_eq!(find(&sleep_id)["status"], "running");
        let killed = env.tool(&bg, "bg_kill", json!({"taskId": sleep_id})).await;
        assert!(
            killed["content"][0]["text"]
                .as_str()
                .unwrap()
                .starts_with("Killed background task Long build"),
            "{killed}"
        );
    }

    // ── process kill: runtime + hub die; agent dir and session entries survive ──────
    env.restart();
    let (bg, _) = env.load("background-tasks").await;
    let (sa, _) = env.load("subagents").await;
    let (fusion, _) = env.load("fusion").await;
    for ext in [&bg, &sa, &fusion] {
        env.event(
            ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
    }
    // background-tasks stays usable (the per-process task store starts empty — known gap).
    let fresh = env
        .tool(
            &bg,
            "bg_run",
            json!({"name": "After restart", "command": "echo ok", "isAgent": false}),
        )
        .await;
    assert!(
        fresh["content"][0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Started background task After restart"),
        "{fresh}"
    );

    // subagents: the interrupted run reads as failed with the repair-compatible text and a
    // cold-resume hint (the child session file exists).
    let records = sa
        .handle_service(
            "subagents.list",
            RawJson::from(json!({})),
            &oino_extension_api::ExtensionId::new("unipi.core").unwrap(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(records[0]["status"], "failed");
    assert_eq!(
        records[0]["error"].as_str().unwrap(),
        "interrupted — Oino exited while it ran (outcome unknown; see the child session)"
    );
    assert!(
        records[0]["resumeHint"]
            .as_str()
            .unwrap()
            .contains("resume:ab12cd34"),
        "{records:?}"
    );
    let read = env
        .tool(
            &sa,
            "read_subagent",
            json!({"agent_id": "ab12cd34", "block": false}),
        )
        .await;
    assert!(
        text_of(&read).contains("resume:ab12cd34"),
        "read_subagent offers the cold resume: {}",
        text_of(&read)
    );

    // fusion: the pairing re-attaches (the lead is set, the status names both models).
    let status = env
        .calls("bus_publish")
        .into_iter()
        .rfind(|d| d["topic"] == "unipi:fusion:status")
        .expect("fusion status published");
    assert_eq!(status["data"]["leadKey"], "faux/lead");
    assert_eq!(status["data"]["sidekickKey"], "faux/cheap");
    assert_eq!(status["data"]["leadEffort"], "high");
}

/// E-02 (QA pass 2): a graph with two dependent nodes must run BOTH, and `bg_run` must
/// keep working in the owner chat afterwards. In the real host the worker's child session
/// contaminates every extension instance's last-known call context, so the run-done
/// delivery (and everything it triggers) arrives as a nested session; that used to make
/// the dependent node's `graph_run` answer "subagents are disabled in this session" and
/// the child's session shutdown set background-tasks' process-global `shutting_down`
/// flag — every later bg_run refused until restart.
///
/// Here the contamination is reproduced for real: a child `session_start` fires through
/// the same wasm instances before the first worker settles, and the run-done is then
/// delivered by the real in-process hub with that child context.
#[tokio::test(flavor = "multi_thread")]
async fn graph_runs_two_dependent_nodes_and_bg_run_survives_the_child_session() {
    let env = Env::new();
    env.host.stage_agent_reports(vec![
        "r-mk-a: created a.txt with A".to_owned(),
        "r-mk-b: created b.txt with AB".to_owned(),
    ]);
    let (bg, _) = env.load("background-tasks").await;
    let (sa, _) = env.load("subagents").await;
    let (lh, _) = env.load("long-horizon").await;
    for ext in [&bg, &sa, &lh] {
        env.event(
            ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
    }
    // bg_run works before the graph (sanity for the "after" assertion below).
    let warm = env
        .tool(&bg, "bg_run", json!({"name": "warm", "command": "echo warm", "isAgent": false}))
        .await;
    assert!(
        !text_of(&warm).contains("shutting down"),
        "{}",
        text_of(&warm)
    );

    // The worker's child session fires through the SAME instances: from here on, every
    // instance's last-known context is the nested session (what the real host does).
    let mut child = env.cx(CallKind::Event);
    child.is_child = true;
    child.session_id = "worker-1".into();
    for ext in [&bg, &sa, &lh] {
        ext.on_event(
            EventKind::SessionStart,
            RawJson::from(json!({"type": "session_start", "reason": "new"})),
            child.clone(),
        )
        .await
        .unwrap_or_else(|e| panic!("child session start: {e}"));
    }

    // The lead opens a graph with a dependent node. mk-a starts at once (a spawn was
    // recorded, under the OWNER session s1).
    let opened = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({
                "operation": "add_work",
                "task": "Two dependent steps",
                "add_work": [
                    {"work_id": "mk-a", "instruction": "create a.txt = A", "profile": "subagent_general"},
                    {"work_id": "mk-b", "instruction": "create b.txt from a.txt + B", "profile": "subagent_general", "input_ids": ["mk-a"]}
                ]
            }),
        )
        .await;
    let t = text_of(&opened);
    assert!(!t.contains("no graph"), "{t}");
    assert!(!t.contains("Agent graph is unavailable"), "{t}");

    // mk-a runs and settles on its own; the run-done then arrives (through the real hub)
    // with the child session context, and mk-b must STILL start — and run to completion.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let spawns = env.host.calls_named("agent_spawn").len();
        let graph = graph_json(&env);
        let done = graph["current"]["work"].as_array().is_some_and(|work| {
            work.iter()
                .filter(|w| w["workId"] == json!("mk-b"))
                .all(|w| w["status"] == json!("completed"))
        });
        if spawns >= 2 && done {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the dependent node never ran (spawns: {spawns}, graph: {graph})"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let graph = graph_json(&env);
    for id in ["mk-a", "mk-b"] {
        let work = graph["current"]["work"]
            .as_array()
            .unwrap()
            .iter()
            .find(|w| w["workId"] == json!(id))
            .unwrap_or_else(|| panic!("{id} missing: {graph}"));
        assert_eq!(work["status"], json!("completed"), "{id}: {graph}");
    }

    // The child session ends (the worker's session shutdown — delivered as a child
    // context, exactly like the real host does). The owner's bg_run must STILL work.
    bg.on_event(
        EventKind::SessionShutdown,
        RawJson::from(json!({"type": "session_shutdown"})),
        child,
    )
    .await
    .unwrap_or_else(|e| panic!("child session shutdown: {e}"));
    let after = env
        .tool(&bg, "bg_run", json!({"name": "after", "command": "echo after", "isAgent": false}))
        .await;
    let t = text_of(&after);
    assert!(
        !t.contains("shutting down"),
        "bg_run must work after a child session ends: {t}"
    );
}

/// The graph file on disk (fresh workspace: one directory).
fn graph_json(env: &Env) -> Value {
    let ws_dir = env._agent.path().join("state/unipi.long-horizon/workspace");
    let path = std::fs::read_dir(&ws_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("graph.json");
    serde_json::from_slice(&std::fs::read(&path).unwrap_or_default()).unwrap_or(Value::Null)
}

/// The calls `ext` made of `method` on `host`.
fn calls_by(host: &RecordingHostServices, method: &str, ext: &str) -> usize {
    host.calls_named(method)
        .into_iter()
        .filter(|c| c.ext.as_deref() == Some(ext))
        .count()
}

/// ext-restart (owner report 2026-11-03): the hub disposing an idle services set shuts
/// its instances down; the next call woke them with `init` only, so the subagents
/// instance had no per-session state (profiles, records) and `update_agent_graph
/// add_work` answered "subagents are not loaded or are disabled". Now a wake replays
/// `session_start` too, never counts toward the trap limit (here: far more wakes than
/// the 3-in-10-minutes trap budget), and the graph keeps working across wakes.
#[tokio::test(flavor = "multi_thread")]
async fn add_work_after_subagents_hibernates_mid_session() {
    let env = Env::new();
    let (sa, _) = env.load("subagents").await;
    let (lh, _) = env.load("long-horizon").await;
    for ext in [&sa, &lh] {
        env.event(
            ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
    }
    // Mid-session the services set goes idle and is disposed: both instances hibernate.
    let caller = oino_extension_api::ExtensionId::new("unipi.long-horizon").unwrap();
    sa.shutdown("idle").await;
    lh.shutdown("idle").await;
    // A plain read wakes subagents; the wake replays its session start (whose refresh
    // reads the active tools — the read itself does not).
    let refreshes = calls_by(&env.host, "control", "unipi.subagents");
    sa.handle_service(
        "subagents.list",
        RawJson::from(json!({})),
        &caller,
        env.cx(CallKind::Service),
    )
    .await
    .unwrap();
    assert!(
        calls_by(&env.host, "control", "unipi.subagents") > refreshes,
        "session_start was not replayed on wake"
    );
    sa.shutdown("idle").await;
    let opened = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({
                "operation": "add_work",
                "task": "After a wake",
                "add_work": [
                    {"work_id": "w1", "instruction": "map the code", "profile": "subagent_explore"}
                ]
            }),
        )
        .await;
    let t = text_of(&opened);
    assert!(!t.contains("Agent graph is unavailable"), "{t}");
    assert!(t.starts_with("Opened graph"), "{t}");
    let graph_id = graph_json(&env)["current"]["graphId"]
        .as_str()
        .unwrap()
        .to_owned();

    // Many more wakes than the trap budget: never "disabled after repeated traps", and the
    // profile registry is there after every one of them.
    for round in 0..6 {
        sa.shutdown("idle").await;
        let profiles = sa
            .handle_service(
                "subagents.profiles",
                RawJson::from(json!({})),
                &caller,
                env.cx(CallKind::Service),
            )
            .await
            .unwrap_or_else(|e| panic!("wake {round}: {e}"))
            .to_value();
        let ids: Vec<&str> = profiles
            .as_array()
            .unwrap()
            .iter()
            .filter_map(Value::as_str)
            .collect();
        assert!(ids.contains(&"subagent_explore"), "wake {round}: {profiles}");
    }

    // The long-horizon graph state survives its own wake: the next add_work extends the
    // SAME graph (reloaded by the replayed session start), it does not open a new one.
    lh.shutdown("idle").await;
    sa.shutdown("idle").await;
    let more = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({
                "operation": "add_work",
                "add_work": [
                    {"work_id": "w2", "instruction": "write it up", "profile": "subagent_general"}
                ]
            }),
        )
        .await;
    let t = text_of(&more);
    assert!(!t.contains("Agent graph is unavailable"), "{t}");
    assert!(t.contains(&graph_id), "same graph after the wake: {t}");
}

/// ext-restart, the routing half: the process-wide hub routed a service call to the
/// NEWEST load of the target extension, whatever services set (cwd) the caller was in.
/// A session in cwd A therefore asked cwd B's subagents for its profiles — and once B
/// was disposed, woke B's hibernated instance with no state for A's session. Calls now
/// stay in the caller's own set; the disposed set is never woken.
#[tokio::test(flavor = "multi_thread")]
async fn service_calls_stay_in_the_callers_services_set() {
    let env = Env::new();
    let (sa, _) = env.load("subagents").await;
    let (lh, _) = env.load("long-horizon").await;
    for ext in [&sa, &lh] {
        env.event(
            ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
    }
    // A second services set (another cwd) loads the same extensions later (newest load),
    // runs its own session, then goes idle and is disposed.
    let other = RecordingHostServices::new();
    let other_host: Arc<dyn HostServices> = Arc::clone(&other) as Arc<dyn HostServices>;
    let (sa_b, _) = env.load_with("subagents", Arc::clone(&other_host)).await;
    let (lh_b, _) = env.load_with("long-horizon", other_host).await;
    let mut cx_b = env.cx(CallKind::Event);
    cx_b.session_id = "s-other".into();
    for ext in [&sa_b, &lh_b] {
        ext.on_event(
            EventKind::SessionStart,
            RawJson::from(json!({"type": "session_start", "reason": "startup"})),
            cx_b.clone(),
        )
        .await
        .unwrap();
    }
    sa_b.shutdown("idle").await;
    lh_b.shutdown("idle").await;
    let other_calls = other.calls().len();

    let opened = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({
                "operation": "add_work",
                "task": "Own set",
                "add_work": [
                    {"work_id": "w1", "instruction": "map the code", "profile": "subagent_explore"}
                ]
            }),
        )
        .await;
    let t = text_of(&opened);
    assert!(t.starts_with("Opened graph"), "{t}");
    // The graph's first worker is spawned by THIS set's subagents (through its own host),
    // and nothing reached — or woke — the disposed set.
    let deadline = Instant::now() + Duration::from_secs(10);
    while env.host.calls_named("agent_spawn").is_empty() {
        assert!(
            Instant::now() < deadline,
            "no worker spawned through the caller's own set (other set: {:?})",
            other.calls_named("agent_spawn")
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        other.calls_named("agent_spawn").is_empty(),
        "the graph run went to another services set"
    );
    assert_eq!(
        other.calls().len(),
        other_calls,
        "the disposed services set was woken by another set's call"
    );
}

/// The graph file path of the (single) long-horizon workspace, created by a first save.
fn graph_path(env: &Env) -> PathBuf {
    let ws_dir = env._agent.path().join("state/unipi.long-horizon/workspace");
    std::fs::read_dir(&ws_dir)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path()
        .join("graph.json")
}

/// A graph the way a host left it when it died: `w1` was running on operator `op`
/// (restart resumes used: `resumes`), `w2` waits on it.
fn crashed_graph(op: &str, resumes: u32) -> Value {
    json!({"v": 1, "history": [], "current": {
        "v": 1, "graphId": "graph-crash", "mode": "graph", "task": "Crashed graph",
        "createdAt": 1, "revision": 1, "updatedAt": 1,
        "events": [{"ts": 1, "kind": "opened", "by": "lead", "detail": "Crashed graph"}],
        "work": [
            {"workId": "w1", "instruction": "map the code", "target": {"kind": "new_preset", "subagentId": "subagent_general"},
             "inputIds": [], "status": "running", "operatorId": op, "restartResumes": resumes, "attempts": 1},
            {"workId": "w2", "instruction": "write it up", "target": {"kind": "new_preset", "subagentId": "subagent_general"},
             "inputIds": ["r-w1"], "status": "queued"}
        ]
    }})
}

async fn lh_service(env: &Env, lh: &Arc<dyn ExtensionInstance>, payload: Value) -> Value {
    lh.handle_service(
        "long-horizon.graph",
        RawJson::from(payload),
        &oino_extension_api::ExtensionId::new("oino.frontend").unwrap(),
        env.cx(CallKind::Service),
    )
    .await
    .unwrap()
    .to_value()
}

fn work_of<'a>(graph: &'a Value, id: &str) -> &'a Value {
    graph["current"]["work"]
        .as_array()
        .and_then(|w| w.iter().find(|w| w["workId"] == json!(id)))
        .unwrap_or_else(|| panic!("{id} missing: {graph}"))
}

fn event_pairs(work: &Value) -> Vec<(String, String)> {
    work["events"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            (
                e["status"].as_str().unwrap_or_default().to_owned(),
                e["by"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// [lh-errored] owner 2026-11-04: an interrupted / upstream-errored node is `errored`, not
/// failed — it blocks its dependents (they wait), wakes the lead, refuses a stalling
/// yield, shows `canRetry` in the app snapshot, and the user's Retry (the app's
/// `long-horizon.graph` `retry`) continues it on its own session (here the operator
/// session is gone, so the one-time fresh operator runs) — after which the dependent
/// runs and the graph completes. Only the lead fails work.
#[tokio::test(flavor = "multi_thread")]
async fn long_horizon_errored_node_user_retry_runs_the_graph_on() {
    let env = Env::new();
    env.host
        .stage_agent_reports(vec!["r-w1: mapped".to_owned(), "r-w2: written".to_owned()]);
    let (sa, _) = env.load("subagents").await;
    let (lh, reg) = env.load("long-horizon").await;
    assert!(reg.services.iter().any(|s| s == "long-horizon.graph"));
    env.event(
        &sa,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    // First save creates the workspace dir; then the crashed graph is "left on disk":
    // w1 already used all its restart resumes.
    env.tool(&lh, "view_agent_graph", json!({})).await;
    std::fs::write(graph_path(&env), crashed_graph("op-dead", 3).to_string()).unwrap();
    env.event(
        &lh,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let g = graph_json(&env);
    let w1 = work_of(&g, "w1");
    assert_eq!(w1["status"], "errored", "{g}");
    assert!(
        w1["failure"]
            .as_str()
            .unwrap()
            .contains("interrupted by a restart 4 times")
    );
    assert_eq!(w1["operatorId"], "op-dead", "operator kept");
    assert_eq!(work_of(&g, "w2")["status"], "queued", "dependent waits");
    assert!(
        env.notifications()
            .iter()
            .any(|n| n.contains("w1 stopped on an error") && n.contains("not failed")),
        "{:?}",
        env.notifications()
    );
    // A stalling yield is refused with the levers.
    let y = env.tool(&lh, "yield_agent_graph", json!({})).await;
    assert!(
        text_of(&y).starts_with("Nothing can progress: w1 (") && text_of(&y).contains("operation=continue"),
        "{y}"
    );
    // The app snapshot: errored + canRetry + the event log; history as data.
    let snap = lh_service(&env, &lh, json!({"op": "snapshot"})).await;
    assert_eq!(snap["ok"], true, "{snap}");
    assert_eq!(snap["graph"]["work"][0]["status"], "errored");
    assert_eq!(snap["graph"]["work"][0]["canRetry"], true);
    assert_eq!(snap["graph"]["work"][0]["events"][0]["status"], "errored");
    assert_eq!(snap["graph"]["counts"]["errored"], 1);
    let hist = lh_service(&env, &lh, json!({"op": "history"})).await;
    assert_eq!(hist["graphs"][0]["graphId"], "graph-crash");
    // Refusals carry central codes.
    let bad = lh_service(&env, &lh, json!({"op": "retry", "workId": "w2"})).await;
    assert_eq!(bad["code"], "lh.work.not_continuable", "{bad}");
    let bad = lh_service(&env, &lh, json!({"op": "retry", "workId": "zz"})).await;
    assert_eq!(bad["code"], "lh.work.not_found", "{bad}");
    let bad = lh_service(&env, &lh, json!({"op": "nope"})).await;
    assert_eq!(bad["code"], "lh.request.invalid", "{bad}");
    // The user's Retry.
    let r = lh_service(&env, &lh, json!({"op": "retry", "workId": "w1", "note": "go on"})).await;
    assert_eq!(r, json!({"ok": true, "workId": "w1"}));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let g = graph_json(&env);
        if work_of(&g, "w2")["status"] == json!("completed") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the retried graph never ran on: {g}"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let g = graph_json(&env);
    let w1 = work_of(&g, "w1");
    assert_eq!(w1["status"], "completed", "{g}");
    let ev = event_pairs(w1);
    assert!(
        ev.contains(&("continued".to_owned(), "user".to_owned())),
        "{ev:?}"
    );
    assert_eq!(ev.last().unwrap().0, "completed", "{ev:?}");
    let out = lh_service(&env, &lh, json!({"op": "output", "workId": "w1"})).await;
    assert_eq!(out["ok"], true, "{out}");
    assert_eq!(out["output"]["status"], "completed");
    assert_eq!(out["output"]["resultId"], "r-w1");
    // Continue / fail of settled work is refused by the tool.
    let c = env
        .tool(&lh, "update_agent_graph", json!({"operation": "continue", "continue": [{"target_id": "w1"}]}))
        .await;
    assert!(
        text_of(&c).contains("only errored or failed work can be continued"),
        "{c}"
    );
    let f = env
        .tool(&lh, "update_agent_graph", json!({"operation": "fail", "fail": [{"target_id": "w2", "reason": "x"}]}))
        .await;
    assert!(text_of(&f).contains("w2 is already completed"), "{f}");
    let v = env.tool(&lh, "view_agent_graph", json!({})).await;
    assert!(text_of(&v).contains("\"events\""), "{v}");
}

/// [lh-errored] restart: work that was running when the host died resumes its own
/// operator after a short grace (by `host`), instead of staying queued forever (D6-03).
#[tokio::test(flavor = "multi_thread")]
async fn long_horizon_restart_resumes_interrupted_work() {
    let env = Env::new();
    let (sa, _) = env.load("subagents").await;
    let (lh, _) = env.load("long-horizon").await;
    env.event(
        &sa,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    env.tool(&lh, "view_agent_graph", json!({})).await;
    std::fs::write(graph_path(&env), crashed_graph("op-gone", 0).to_string()).unwrap();
    env.event(
        &lh,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let g = graph_json(&env);
    let w1 = work_of(&g, "w1");
    assert_eq!(w1["status"], "queued", "{g}");
    assert_eq!(w1["restartResumes"], 1);
    assert_eq!(w1["resumeOperatorId"], "op-gone");
    assert!(w1["nextRetryAt"].as_i64().is_some());
    assert_eq!(
        event_pairs(w1).last().unwrap(),
        &("continued".to_owned(), "host".to_owned())
    );
    // The resume fires by itself (no lead turn, no user action) and the graph runs on.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let g = graph_json(&env);
        if work_of(&g, "w2")["status"] == json!("completed") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the interrupted graph stalled: {g}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
}

async fn sa_list(env: &Env, sa: &Arc<dyn ExtensionInstance>) -> Vec<Value> {
    sa.handle_service(
        "subagents.list",
        RawJson::from(json!({})),
        &oino_extension_api::ExtensionId::new("oino.frontend").unwrap(),
        env.cx(CallKind::Service),
    )
    .await
    .unwrap()
    .to_value()
    .as_array()
    .cloned()
    .unwrap_or_default()
}

/// [lh-orphan] owner 2026-11-04 (graph wl-fix-2): a worker parked on a wake-up `bg_run`
/// whose task died under it (killed by a hibernation) never sends another event. The lead
/// stopped + replaced it, but the cancel only flagged the record and waited for an event
/// that never came: the run stayed `running` for 1.5 h in the Running list while the graph
/// hid it among the replaced steps. Now (1) while the worker runs on, the app snapshot
/// lists the replaced item as `running` + `orphaned` + `canStop`; (2) the cancel settles
/// the silent run after its grace (record `cancelled`, ended), after which the item is
/// the replaced step it is; (3) the app's `stop_operator` op cancels an orphan.
#[tokio::test(flavor = "multi_thread")]
async fn a_stopped_worker_that_hangs_is_cancelled_and_surfaces_until_it_ends() {
    let env = Env::new();
    env.host.stage_hung_agents(1);
    env.host
        .stage_agent_reports(vec!["fixed".to_owned(), "r-fix-verify: verified".to_owned()]);
    let (sa, _) = env.load("subagents").await;
    let (lh, _) = env.load("long-horizon").await;
    for ext in [&sa, &lh] {
        env.event(
            ext,
            EventKind::SessionStart,
            json!({"type": "session_start", "reason": "startup"}),
        )
        .await;
    }
    let opened = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({"operation": "add_work", "task": "Fix + verify",
                   "add_work": [{"work_id": "fix", "instruction": "fix the findings", "profile": "subagent_general"}]}),
        )
        .await;
    assert!(!text_of(&opened).contains("refused"), "{opened}");
    // The worker runs and parks on its bg_run (the fake hangs after agent_end).
    let deadline = Instant::now() + Duration::from_secs(20);
    let op = loop {
        let g = graph_json(&env);
        let w = work_of(&g, "fix");
        if w["status"] == "running"
            && let Some(op) = w["operatorId"].as_str()
        {
            break op.to_owned();
        }
        assert!(Instant::now() < deadline, "fix never started: {g}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    tokio::time::sleep(Duration::from_millis(500)).await;
    let rec = sa_list(&env, &sa).await;
    let rec = rec.iter().find(|r| r["id"] == json!(op)).unwrap();
    assert_eq!(rec["status"], "running", "{rec}");

    // The lead stops it and replaces it.
    let stop = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({"operation": "stop", "stop": [{"target_id": "fix", "reason": "idle 86 min"}]}),
        )
        .await;
    assert!(!text_of(&stop).contains("refused"), "{stop}");
    let add = env
        .tool(
            &lh,
            "update_agent_graph",
            json!({"operation": "add_work", "add_work": [{"work_id": "fix-verify", "instruction": "verify", "profile": "subagent_general", "replaces": "fix"}]}),
        )
        .await;
    assert!(!text_of(&add).contains("refused"), "{add}");
    // Right after the stop the silent worker still runs: the graph shows it.
    let snap = lh_service(&env, &lh, json!({"op": "snapshot"})).await;
    let fix = snap["graph"]["work"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["workId"] == "fix")
        .unwrap()
        .clone();
    assert_eq!(fix["status"], "running", "{snap}");
    assert_eq!(fix["orphaned"], true, "{snap}");
    assert_eq!(fix["canStop"], true);
    assert_eq!(fix["stoppedAs"], "superseded");

    // The cancel settles the silent run after its grace (5 s).
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let list = sa_list(&env, &sa).await;
        let rec = list.iter().find(|r| r["id"] == json!(op)).unwrap();
        if rec["status"] == "cancelled" {
            assert!(rec["endedAt"].as_i64().is_some(), "{rec}");
            assert_eq!(rec["cancelledBy"], "user");
            break;
        }
        assert!(
            Instant::now() < deadline,
            "the stopped worker never ended: {rec}"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    let snap = lh_service(&env, &lh, json!({"op": "snapshot"})).await;
    let fix = snap["graph"]["work"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["workId"] == "fix")
        .unwrap()
        .clone();
    assert_eq!(fix["status"], "superseded", "{snap}");
    assert!(fix.get("orphaned").is_none(), "{snap}");
    // The replacement ran to completion; stop_operator refuses work the graph runs and
    // answers for a settled orphan.
    let deadline = Instant::now() + Duration::from_secs(20);
    while work_of(&graph_json(&env), "fix-verify")["status"] != "completed" {
        assert!(Instant::now() < deadline, "fix-verify never completed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let r = lh_service(&env, &lh, json!({"op": "stop_operator", "workId": "fix"})).await;
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(r["cancelled"], false, "already ended: {r}");
    let r = lh_service(&env, &lh, json!({"op": "stop_operator", "workId": "nope"})).await;
    assert_eq!(r["code"], "lh.work.not_found", "{r}");
}
