//! The unipi-context components as real components in the wasm host: registration plus one
//! end-to-end behaviour each (workflow gate + plan mode, skill exposure, dream tray, lossless
//! compaction + recall, memory store/list/search/delete).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionId,
    ExtensionInstance, ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
    ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

const CRATES: [(&str, &str); 5] = [
    ("workflow", "unipi_workflow"),
    ("skill-registry", "unipi_skill_registry"),
    ("dream", "unipi_dream"),
    ("compactor", "unipi_compactor"),
    ("memory", "unipi_memory"),
];

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn extensions_dir() -> PathBuf {
    harness_dir().join("../../..").canonicalize().unwrap()
}

/// Builds the five components once (in `EXT_WORKSPACE` or this checkout's `extensions/`).
fn wasm_dir() -> &'static PathBuf {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        let ws = std::env::var_os("EXT_WORKSPACE").map_or_else(extensions_dir, PathBuf::from);
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(&ws)
            .args(["build", "-j", "2", "--release", "--target", "wasm32-wasip2"]);
        for (dir, _) in CRATES {
            cmd.args(["-p", &format!("unipi-{dir}")]);
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
    agent: tempfile::TempDir,
    project: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let agent = tempfile::tempdir().unwrap();
        // Not under TMPDIR: the permission gate treats the temp dir as writable.
        let base = extensions_dir().join("target/context-harness");
        std::fs::create_dir_all(&base).unwrap();
        let project = tempfile::Builder::new()
            .prefix("ctxproj")
            .tempdir_in(&base)
            .unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: RecordingHostServices::new(),
            agent,
            project,
        }
    }

    /// SIGKILL of the host process for the guest (core.md §2.1.8 restart gate): the runtime
    /// and its guest instances die, the agent/project dirs and the recorded session entries
    /// survive, and a fresh runtime is rebuilt over them.
    fn restart(&mut self) {
        let mut options =
            WasmRuntimeOptions::new(HostConfig::new(self.agent.path(), self.project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
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

    async fn load(
        &self,
        dir: &str,
    ) -> (Arc<dyn ExtensionInstance>, oino_extension_api::Registration) {
        self.load_with_memory(dir, None).await
    }

    /// Like [`Self::load`], with the manifest's linear-memory cap replaced.
    async fn load_with_memory(
        &self,
        dir: &str,
        memory_mib: Option<u32>,
    ) -> (Arc<dyn ExtensionInstance>, oino_extension_api::Registration) {
        let file = CRATES.iter().find(|(d, _)| *d == dir).unwrap().1;
        let mut manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join(format!("unipi/{dir}/extension.json")))
                .unwrap(),
        )
        .unwrap();
        if let Some(mib) = memory_mib {
            manifest.limits.memory_mib = mib;
        }
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(wasm_dir().join(format!("{file}.wasm"))),
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

    /// Like [`Self::event`], with an explicit call context (a second chat's session id).
    async fn event_with(
        &self,
        ext: &Arc<dyn ExtensionInstance>,
        kind: EventKind,
        payload: Value,
        cx: CallContext,
    ) -> Option<Value> {
        ext.on_event(kind, RawJson::from(payload), cx)
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

    async fn tool(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, args: Value) -> Value {
        let mut cx = self.cx(CallKind::Tool);
        cx.tool_call_id = Some("t1".into());
        let inv = ToolInvocation {
            tool_call_id: "t1".into(),
            name: name.into(),
            args: RawJson::from(args),
            parent_tool_call_id: None,
        };
        ext.run_tool(inv, CancellationToken::new(), cx)
            .await
            .unwrap()
            .to_value()
    }

    async fn service(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, payload: Value) -> Value {
        let caller = ExtensionId::new("unipi.core").unwrap();
        ext.handle_service(
            name,
            RawJson::from(payload),
            &caller,
            self.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value()
    }

    fn notifies(&self) -> Vec<String> {
        self.host
            .calls_named("ui_fire")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["method"] == "notify")
            .map(|d| d["message"].as_str().unwrap_or_default().to_owned())
            .collect()
    }

    /// Session messages the extension sent (`sendMessage`/`sendUserMessage`).
    fn sent_messages(&self) -> Vec<Value> {
        self.host
            .calls_named("session_write")
            .into_iter()
            .map(|c| c.detail)
            .filter_map(|d| {
                if d["sendMessage"].is_object() {
                    Some(d["sendMessage"].clone())
                } else {
                    d["sendUserMessage"]
                        .is_object()
                        .then(|| d["sendUserMessage"].clone())
                }
            })
            .collect()
    }

    fn appended(&self, custom_type: &str) -> Vec<Value> {
        self.host
            .calls_named("session_write")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["appendEntry"] == custom_type)
            .collect()
    }
}

fn names<T>(items: &[T], f: impl Fn(&T) -> (&str, bool)) -> Vec<(String, bool)> {
    items
        .iter()
        .map(|i| {
            let (n, h) = f(i);
            (n.to_owned(), h)
        })
        .collect()
}

/// Fires `before_agent_start` as the host does for a user prompt.
async fn prompt_turn(env: &Env, ext: &Arc<dyn ExtensionInstance>, text: &str) -> Option<Value> {
    env.event(
        ext,
        EventKind::BeforeAgentStart,
        json!({"type": "before_agent_start", "prompt": text, "systemPrompt": "", "systemPromptOptions": {}}),
    )
    .await
}

fn text(result: &Value) -> String {
    result["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test(flavor = "multi_thread")]
async fn workflow_gate_and_plan_mode() {
    let env = Env::new();
    let (ext, reg) = env.load("workflow").await;
    let cmds = names(&reg.commands, |c| (c.name.as_str(), c.hidden));
    for want in [
        ("permission", false),
        ("plan", false),
        ("permission-clear-rules", true),
        ("plan-off", true),
        ("plan-enter", true),
        ("plan-exit", true),
    ] {
        assert!(
            cmds.contains(&(want.0.to_owned(), want.1)),
            "{want:?} in {cmds:?}"
        );
    }
    assert_eq!(
        reg.tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["plan_submit"]
    );
    let keys: Vec<(&str, &str)> = reg
        .shortcuts
        .iter()
        .map(|s| (s.id.as_str(), s.default_keys[0].as_str()))
        .collect();
    assert_eq!(
        keys,
        [
            ("unipi.plan.toggle", "alt+p"),
            ("unipi.permission.cycle", "alt+m")
        ]
    );
    assert!(reg.events.contains(&EventKind::ToolCall));
    assert_eq!(reg.settings[0].to_value()["namespace"], "permission");

    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let safe = env.event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c1", "input": {"command": "git status && ls"}})).await;
    assert_eq!(safe, None, "read-only bash runs");
    env.host
        .queue_dialog_answer(json!({"value": {"optionId": "deny_note", "feedback": "not here"}}));
    let blocked = env.event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "c2", "input": {"command": "rm -rf /tmp/x"}})).await.unwrap();
    assert_eq!(
        blocked,
        json!({"block": true, "reason": "Blocked by permission (user denied): not here"})
    );
    let dialog = env.host.calls_named("ui_dialog").pop().unwrap().detail;
    assert_eq!(dialog["method"], "approval");
    assert_eq!(
        dialog["options"][1]["label"],
        "Always allow `rm -rf /tmp/x`"
    );

    env.command(&ext, "plan", "on").await;
    let entries = env.appended("unipi:plan-mode");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["data"]["active"], true);
    assert_eq!(
        env.notifies().last().map(String::as_str),
        Some("Plan mode on"),
        "{:?}",
        env.notifies()
    );
    // The toggle itself sends nothing: no session message, no turn start.
    assert!(env.sent_messages().is_empty(), "{:?}", env.sent_messages());
    let write = env.event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "write", "toolCallId": "c3", "input": {"path": "src/main.rs", "content": "x"}})).await.unwrap();
    assert_eq!(write["block"], true);
    assert!(
        write["reason"]
            .as_str()
            .unwrap()
            .starts_with("Plan mode: only the plan file")
    );
    // First prompt: the instructions attach once (visible) …
    let instructions = env.event(&ext, EventKind::BeforeAgentStart, json!({"type": "before_agent_start", "prompt": "go", "systemPrompt": "", "systemPromptOptions": {}})).await.unwrap();
    assert_eq!(
        instructions["message"]["customType"],
        "unipi:plan-mode-message"
    );
    assert_eq!(instructions["message"]["display"], true);
    assert!(
        instructions["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Plan mode is ON")
    );
    // … later prompts only get the hidden reminder.
    let reminder = env.event(&ext, EventKind::BeforeAgentStart, json!({"type": "before_agent_start", "prompt": "again", "systemPrompt": "", "systemPromptOptions": {}})).await.unwrap();
    assert_eq!(reminder["message"]["customType"], "unipi:plan-mode-message");
    assert_eq!(reminder["message"]["display"], false);
    assert!(
        reminder["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .starts_with("[plan mode on")
    );
}

/// Plan-mode toggles never start a turn and never put anything in the chat by themselves:
/// the instructions (and later the "[plan mode off]" note) attach to the next prompt at
/// `before_agent_start`, and an on→off flip before that prompt is a net no-op.
#[tokio::test(flavor = "multi_thread")]
async fn plan_toggle_attaches_to_next_prompt() {
    let env = Env::new();
    let (ext, _) = env.load("workflow").await;
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;

    // on → prompt: instructions attached once, by the hook, not sent.
    env.command(&ext, "plan", "on").await;
    assert_eq!(env.sent_messages().len(), 0, "toggle sends nothing");
    let msg = prompt_turn(&env, &ext, "draft the plan")
        .await
        .expect("instructions attach");
    assert!(
        msg["message"]["content"]
            .as_str()
            .unwrap_or_default()
            .starts_with("Plan mode is ON")
    );
    assert_eq!(msg["message"]["display"], true);
    assert_eq!(env.sent_messages().len(), 0, "attach is a hook return");

    // on, prompt, off, prompt: "[plan mode off]" attaches to the second prompt.
    env.command(&ext, "plan", "off").await;
    assert_eq!(
        env.notifies().last().map(String::as_str),
        Some("Plan mode off")
    );
    let note = prompt_turn(&env, &ext, "go ahead")
        .await
        .expect("off note attaches");
    assert_eq!(note["message"]["content"], "[plan mode off]");
    assert_eq!(note["message"]["display"], true);
    assert!(
        prompt_turn(&env, &ext, "and again").await.is_none(),
        "off note consumed once; no reminder while off"
    );

    // on, off, prompt: nothing attached (net no-op).
    env.command(&ext, "plan", "on").await;
    env.command(&ext, "plan", "off").await;
    assert_eq!(env.sent_messages().len(), 0, "no message was ever sent");
    assert!(prompt_turn(&env, &ext, "plain question").await.is_none());

    // No toggle ever started a turn: nothing left the extension but state entries.
    assert!(env.sent_messages().is_empty());
    assert_eq!(
        env.appended("unipi:plan-mode").len(),
        4,
        "state entries only"
    );
}

/// Rules, the project mode and skill states go through `registry.set-extension-setting`; the
/// pre-write-path state files are imported once and removed.
#[tokio::test(flavor = "multi_thread")]
async fn settings_write_path_and_legacy_import() {
    use oino_extension_api::ops::SettingsScope;
    let env = Env::new();
    let state = |id: &str| env.agent.path().join("state").join(id).join("global");
    std::fs::create_dir_all(state("unipi.workflow")).unwrap();
    let legacy = state("unipi.workflow").join("permission.json");
    std::fs::write(
        &legacy,
        json!({"rules": [{"tool": "bash", "pattern": "cargo publish*", "decision": "deny", "scope": "global"}]}).to_string(),
    )
    .unwrap();
    std::fs::create_dir_all(state("unipi.skill-registry")).unwrap();
    let states = state("unipi.skill-registry").join("states.json");
    std::fs::write(&states, json!({"alpha": {"enabled": false}}).to_string()).unwrap();
    std::fs::write(
        state("unipi.skill-registry").join("proxy.json"),
        json!({"proxy": true}).to_string(),
    )
    .unwrap();

    let (wf, _) = env.load("workflow").await;
    env.event(
        &wf,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    assert!(!legacy.exists(), "imported store is removed");
    let global = env
        .host
        .extension_setting_layer(SettingsScope::Global, "permission");
    assert_eq!(global["rules"][0]["pattern"], "cargo publish*");
    assert_eq!(global["rules"][0]["decision"], "deny");
    let denied = env.event(&wf, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "d1", "input": {"command": "cargo publish --dry-run"}})).await.unwrap();
    assert_eq!(
        denied["block"], true,
        "imported deny rule applies: {denied}"
    );

    env.host
        .queue_dialog_answer(json!({"value": {"optionId": "allow_always"}}));
    let first = env.event(&wf, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "a1", "input": {"command": "rm -rf /tmp/x"}})).await;
    assert_eq!(first, None);
    let project = env
        .host
        .extension_setting_layer(SettingsScope::Project, "permission");
    assert_eq!(project["rules"][0]["pattern"], "rm -rf /tmp/x");
    assert_eq!(project["rules"][0]["scope"], "project");
    let dialogs = env.host.calls_named("ui_dialog").len();
    let again = env.event(&wf, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "a2", "input": {"command": "rm -rf /tmp/x"}})).await;
    assert_eq!(again, None);
    assert_eq!(
        env.host.calls_named("ui_dialog").len(),
        dialogs,
        "saved rule allows without asking"
    );

    // E-18/F-09: a permission mode change is per chat — nothing is written into the
    // project, and the same chat then runs dangerous bash without asking (full).
    env.command(&wf, "permission", "full").await;
    let project = env
        .host
        .extension_setting_layer(SettingsScope::Project, "permission");
    assert!(
        project.get("mode").is_none(),
        "no mode in the project layer: {project}"
    );
    let full = env.event(&wf, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "bash", "toolCallId": "f1", "input": {"command": "rm -rf /tmp/x"}})).await;
    assert_eq!(full, None, "this chat's full mode lets it run");
    assert_eq!(
        env.host.calls_named("ui_dialog").len(),
        dialogs,
        "no ask under full"
    );

    // The explicit "make default" writes the GLOBAL default in the agent dir — and never
    // the project (the repo's settings.json must never learn a permission mode).
    env.command(&wf, "permission", "make-default ask").await;
    assert_eq!(
        env.host
            .extension_setting_layer(SettingsScope::Global, "permission")["defaultMode"],
        "ask"
    );
    assert!(
        env.host
            .extension_setting_layer(SettingsScope::Project, "permission")
            .get("mode")
            .is_none(),
    );
    // Another chat (a fresh session id) starts at the default, unaffected by the first
    // chat's session mode — the mode never leaks across chats.
    let other = {
        let mut cx = env.cx(oino_extension_api::CallKind::Tool);
        cx.session_id = "s2".into();
        cx.tool_call_id = Some("f2".into());
        cx
    };
    env.host
        .queue_dialog_answer(json!({"value": {"optionId": "allow_once"}}));
    let refused = env
        .event_with(
            &wf,
            EventKind::ToolCall,
            json!({"type": "tool_call", "toolName": "bash", "toolCallId": "f2", "input": {"command": "rm -rf /tmp/x"}}),
            other,
        )
        .await;
    assert_eq!(refused, None, "the new chat asks again (its own mode is the default)");

    env.command(&wf, "permission-clear-rules", "").await;
    assert!(
        env.host
            .extension_setting_layer(SettingsScope::Project, "permission")
            .get("rules")
            .is_none()
    );

    let (sr, _) = env.load("skill-registry").await;
    env.event(
        &sr,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    assert!(!states.exists());
    assert!(!state("unipi.skill-registry").join("proxy.json").exists());
    let skills = env
        .host
        .extension_setting_layer(SettingsScope::Global, "skills");
    assert_eq!(skills["states"], json!({"alpha": {"enabled": false}}));
    assert_eq!(skills["proxy"], true);
    let writes: Vec<String> = env
        .host
        .calls_named("set_extension_setting")
        .iter()
        .map(|c| c.ext.clone().unwrap_or_default())
        .collect();
    assert!(
        writes
            .iter()
            .all(|e| e == "unipi.workflow" || e == "unipi.skill-registry")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn skill_registry_exposure() {
    let env = Env::new();
    let (ext, reg) = env.load("skill-registry").await;
    assert_eq!(
        names(&reg.commands, |c| (c.name.as_str(), c.hidden)),
        [
            ("skills".to_owned(), false),
            ("skills-editor".to_owned(), true)
        ]
    );
    for k in [
        EventKind::ResourcesDiscover,
        EventKind::BeforeAgentStart,
        EventKind::Input,
        EventKind::SessionStart,
    ] {
        assert!(reg.events.contains(&k), "{k:?}");
    }
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let skills = json!([
        {"name": "alpha", "description": "a", "filePath": "/x/alpha/SKILL.md", "baseDir": "/x/alpha"},
        {"name": "beta", "description": "b", "filePath": "/x/beta/SKILL.md", "baseDir": "/x/beta"},
    ]);
    let out = env.event(&ext, EventKind::BeforeAgentStart, json!({"type": "before_agent_start", "prompt": "fix the parser", "systemPrompt": "", "systemPromptOptions": {"skills": skills, "sections": {"unipi-skills-hidden": "stale"}}})).await.unwrap();
    assert_eq!(
        out["systemPromptOptions"]["skills"]
            .as_array()
            .unwrap()
            .len(),
        2,
        "≤ maxSkills: everything listed"
    );
    assert!(
        out["systemPromptOptions"]["sections"]
            .get("unipi-skills-hidden")
            .is_none()
    );
    env.command(&ext, "skills", "").await;
    let fired: Vec<Value> = env
        .host
        .calls_named("ui_fire")
        .into_iter()
        .map(|c| c.detail)
        .filter(|d| d["method"] == "openSettings")
        .collect();
    assert_eq!(
        fired,
        [json!({"method": "openSettings", "namespace": "skills"})]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dream_tray_and_status() {
    let env = Env::new();
    let (ext, reg) = env.load("dream").await;
    assert_eq!(
        reg.commands
            .iter()
            .map(|c| c.name.as_str())
            .collect::<Vec<_>>(),
        ["dream"]
    );
    for s in [
        "work-tray.tab",
        "work-tray.action",
        "dream.status",
        "dream.run",
        "dream.stop",
    ] {
        assert!(reg.services.iter().any(|x| x == s), "{s}");
    }
    let tab = env
        .service(&ext, "work-tray.tab", json!({"open": true}))
        .await;
    assert_eq!(tab["id"], "dream");
    assert_eq!(tab["visible"], false, "no runs, dreaming off");
    assert!(
        tab["body"]
            .to_string()
            .contains("Background dreaming is off")
    );
    let status = env.service(&ext, "dream.status", json!({})).await;
    assert_eq!(status["enabled"], false);
    assert_eq!(status["runs"], json!([]));
    env.command(&ext, "dream", "status").await;
    assert!(
        env.notifies()[0].starts_with("dream: off — turn on in /settings → Dream"),
        "{:?}",
        env.notifies()
    );
    env.command(&ext, "dream", "bogus").await;
    assert_eq!(
        env.notifies()[1],
        "usage: /dream status|tray|report|run|stop|approve <n>|reject <n>"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn compactor_lossless_and_recall() {
    let env = Env::new();
    let (ext, reg) = env.load("compactor").await;
    let cmds: Vec<&str> = reg.commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        cmds,
        [
            "compact-vcc",
            "compact-by-llm",
            "session-recall",
            "compact-stats",
            "compact-doctor",
            "compact-help"
        ]
    );
    assert_eq!(
        reg.tools
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["session_recall", "context_budget"]
    );
    let params = reg.tools[0].parameters.to_value();
    assert!(params["properties"].get("expand").is_some());
    let u = |id: &str, t: &str| json!({"id": id, "type": "message", "message": {"role": "user", "content": t}});
    let a = |id: &str, t: &str| json!({"id": id, "type": "message", "message": {"role": "assistant", "content": [{"type": "text", "text": t}]}});
    let branch = json!([
        u("e1", "Build a login page. Keep the orange theme."),
        a("e2", "Working on it."),
        u("e3", "Continue"),
        a("e4", "Done with the form."),
        u("e5", "Now add a reset link."),
        a("e6", "Adding it.")
    ]);
    let out = env.event(&ext, EventKind::SessionBeforeCompact, json!({"type": "session_before_compact", "reason": "threshold", "willRetry": false, "branchEntries": branch, "preparation": {"tokensBefore": 50000, "firstKeptEntryId": "e5", "messagesToSummarize": [], "turnPrefixMessages": [], "isSplitTurn": false, "fileOps": {"read": [], "written": [], "edited": []}, "settings": {}}})).await.unwrap();
    let c = &out["compaction"];
    assert_eq!(c["details"]["method"], "vcc");
    assert_eq!(c["details"]["compactor"], "@pi-unipi/compactor");
    assert!(
        c["summary"]
            .as_str()
            .unwrap()
            .contains("[Your Requests]\n- Build a login page.")
    );
    assert!(
        c["summary"]
            .as_str()
            .unwrap()
            .ends_with("Do not redo work that is already done.")
    );
    assert_eq!(c["tokensBefore"], 50000);
    let budget = env.tool(&ext, "context_budget", json!({})).await;
    assert_eq!(
        text(&budget),
        "Context budget: unknown (no token data available yet)."
    );
    let recall = env
        .tool(&ext, "session_recall", json!({"query": "redis"}))
        .await;
    assert_eq!(text(&recall), "No matches for \"redis\".");
    let then = env.event(&ext, EventKind::Input, json!({"type": "input", "text": "/compact-then next", "source": "interactive", "streamingBehavior": "followUp"})).await;
    assert_eq!(then, Some(json!({"action": "handled"})));
    assert!(
        env.notifies()
            .iter()
            .any(|n| n == "Compact then: queued — will compact and continue after this run.")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_store_list_search_delete() {
    let env = Env::new();
    let (ext, reg) = env.load("memory").await;
    let tools: Vec<&str> = reg.tools.iter().map(|t| t.name.as_str()).collect();
    assert_eq!(
        tools,
        [
            "memory_store",
            "memory_search",
            "global_memory_search",
            "memory_delete",
            "memory_list",
            "global_memory_list"
        ]
    );
    let cmds: Vec<&str> = reg.commands.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(
        cmds,
        [
            "memory",
            "memory-process",
            "memory-consolidate",
            "memory-search",
            "global-memory-search",
            "memory-forget",
            "global-memory-list"
        ]
    );
    let stored = env.tool(&ext, "memory_store", json!({"title": "redis_cache_ttl", "content": "Use a 60 s TTL for the session cache.", "type": "decision", "tags": ["cache"]})).await;
    let t = text(&stored);
    assert!(t.starts_with("Stored memory: redis_cache_ttl\n"), "{t}");
    assert!(
        t.contains("› decision · markdown only — daemon unreachable"),
        "{t}"
    );
    let project = env
        .project
        .path()
        .file_name()
        .unwrap()
        .to_string_lossy()
        .to_lowercase();
    let file: PathBuf = env.agent.path().join(format!(
        "state/unipi.memory/global/memory/{project}/decision/redis_cache_ttl.md"
    ));
    let doc = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
    assert!(
        doc.starts_with("---\nid: redis_cache_ttl\ntitle: redis_cache_ttl\ntags:\n  - cache\n"),
        "{doc}"
    );
    let dup = env.tool(&ext, "memory_store", json!({"title": "redis_cache_ttl", "content": "Use a 60 s TTL for the session cache.", "type": "decision"})).await;
    assert!(text(&dup).starts_with("⚠️ Memory already exists"));
    let list = env.tool(&ext, "memory_list", json!({})).await;
    assert_eq!(
        text(&list),
        "Project memories (1):\n\n- redis_cache_ttl (decision)"
    );
    let found = env
        .tool(
            &ext,
            "memory_search",
            json!({"query": "session cache", "scope": "project"}),
        )
        .await;
    assert!(
        text(&found).starts_with("Found 1 memories:\n\n1. ["),
        "{}",
        text(&found)
    );
    let deleted = env
        .tool(&ext, "memory_delete", json!({"title": "redis_cache_ttl"}))
        .await;
    assert_eq!(
        text(&deleted),
        "Deleted memory: redis_cache_ttl (markdown-only)"
    );
    assert!(!file.exists());
}

/// The §2.1.8 restart gate for this suite: a scripted session is SIGKILLed mid-turn
/// ([`Env::restart`]) and a fresh host reloads the extensions over the same agent dir and
/// session file. Workflow restores plan mode from its `unipi:plan-mode` entry (and the gate
/// still enforces); compactor restores the pending `compact-vcc` from its
/// `unipi:compactor-pending` entry — the restarted session's first compaction runs it as a
/// command compaction and clears the entry when it executes.
#[allow(clippy::too_many_lines, reason = "one scripted crash, two extensions")]
#[tokio::test(flavor = "multi_thread")]
async fn kill_and_restart_restores_plan_mode_and_pending_compaction() {
    let mut env = Env::new();
    {
        let (wf, _) = env.load("workflow").await;
        env.command(&wf, "plan", "on").await;
        assert_eq!(env.appended("unipi:plan-mode").len(), 1);
        // The process died right after arming a /compact-vcc that never ran.
        env.host.seed_entries(vec![json!({
            "type": "custom", "customType": "unipi:compactor-pending",
            "data": {"method": "vcc", "command": "compact-vcc"}
        })]);
    }
    env.restart();

    // workflow: plan mode rebuilt from the session entry, the gate still blocks writes.
    let (wf, _) = env.load("workflow").await;
    env.event(
        &wf,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let published: Vec<_> = env
        .host
        .calls_named("bus_publish")
        .into_iter()
        .map(|c| c.detail)
        .filter(|d| d["topic"] == "unipi:plan:mode:changed")
        .collect();
    let changed = published
        .last()
        .expect("plan mode change published after restart");
    assert_eq!(changed["data"]["active"], true, "{changed:?}");
    let blocked = env
        .event(
            &wf,
            EventKind::ToolCall,
            json!({"type": "tool_call", "toolName": "write", "toolCallId": "c1", "input": {"path": "src/main.rs", "content": "x"}}),
        )
        .await
        .unwrap();
    assert_eq!(blocked["block"], true, "plan gate usable after restart");

    // compactor: the pending command compaction is armed again and consumed by the next
    // compaction as a command compaction, which clears the durable entry.
    let (cp, _) = env.load("compactor").await;
    env.event(
        &cp,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let u = |id: &str, t: &str| json!({"id": id, "type": "message", "message": {"role": "user", "content": t}});
    let a = |id: &str, t: &str| json!({"id": id, "type": "message", "message": {"role": "assistant", "content": [{"type": "text", "text": t}]}});
    let branch = json!([
        u("e1", "Build a login page."),
        a("e2", "Working on it."),
        u("e3", "Now add a reset link."),
        a("e4", "Adding it.")
    ]);
    let before = env
        .event(
            &cp,
            EventKind::SessionBeforeCompact,
            json!({"type": "session_before_compact", "reason": "threshold", "willRetry": false, "branchEntries": branch, "preparation": {"tokensBefore": 50000, "firstKeptEntryId": "e3", "messagesToSummarize": [], "turnPrefixMessages": [], "isSplitTurn": false, "fileOps": {"read": [], "written": [], "edited": []}, "settings": {}}}),
        )
        .await
        .unwrap();
    assert_eq!(
        before["compaction"]["details"]["method"], "vcc",
        "the restored pending method drives the compaction"
    );
    assert!(
        env.appended("unipi:compactor-pending")
            .iter()
            .any(|e| e["data"]["cleared"] == true),
        "executing the pending compaction clears the durable entry"
    );
    env.event(
        &cp,
        EventKind::SessionCompact,
        json!({"type": "session_compact", "compactionEntry": before["compaction"].clone()}),
    )
    .await;
    let cards = env.appended("unipi-compaction");
    assert!(
        cards.iter().any(|c| c["data"]["command"] == "compact-vcc"),
        "the restarted compaction is labelled with the restored command: {cards:?}"
    );
}

/// G-02/G-01/G-10: the per-chat permission mode is persisted as a `unipi:permission-mode`
/// entry, restored EXACTLY after a host restart (an Ask chat must never come back as
/// auto), published on this chat's own `unipi:workflow:status:<session>` topic on every
/// change (the global topic is shared, so chats overwrite each other), and both status
/// commands answer with one readable line.
#[tokio::test(flavor = "multi_thread")]
async fn permission_mode_survives_restart_and_reports_per_session() {
    use oino_extension_api::CancellationToken;
    const PERMISSION_ENTRY: &str = "unipi:permission-mode";
    let mut env = Env::new();
    let (wf, _) = env.load("workflow").await;
    env.event(
        &wf,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let topic = |env: &Env, sid: &str| -> Vec<Value> {
        env.host
            .calls_named("bus_publish")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["topic"] == format!("unipi:workflow:status:{sid}"))
            .collect()
    };

    env.command(&wf, "permission", "ask").await;
    let persisted = env.appended(PERMISSION_ENTRY);
    assert_eq!(persisted.len(), 1, "the mode is persisted: {persisted:?}");
    assert_eq!(persisted[0]["data"]["mode"], "ask");
    let published = topic(&env, "s1");
    assert_eq!(published.last().unwrap()["sticky"], json!(true));
    assert_eq!(
        published.last().unwrap()["data"]["permissionMode"],
        "ask",
        "this chat's topic carries this chat's mode"
    );

    // G-10: one readable line each, with this chat's mode and the global default.
    env.command(&wf, "permission", "status").await;
    assert_eq!(
        env.notifies().last().map(String::as_str),
        Some("Permission mode: ask (this chat: ask · default for new chats: auto)"),
        "{:?}",
        env.notifies()
    );
    env.command(&wf, "plan", "status").await;
    assert_eq!(
        env.notifies().last().map(String::as_str),
        Some("Plan mode: off (/plan to plan before acting)"),
        "{:?}",
        env.notifies()
    );

    // A second chat sets full: its own topic says full, and s1's topic is untouched.
    let other = {
        let mut cx = env.cx(oino_extension_api::CallKind::Command);
        cx.session_id = "s2".into();
        cx
    };
    wf.run_command("permission", "full", CancellationToken::new(), other)
        .await
        .unwrap();
    assert_eq!(
        topic(&env, "s2").last().unwrap()["data"]["permissionMode"],
        "full"
    );
    assert_eq!(
        topic(&env, "s1").last().unwrap()["data"]["permissionMode"],
        "ask",
        "chat two's change does not leak into chat one's topic"
    );

    // Restart: the chat's own session file comes back as ask — never auto. (The recording
    // host pools both chats' entries, so the restarted chat is a fresh Env seeded with
    // s1's entries only — in production each session's jsonl holds its own.)
    let mut env2 = Env::new();
    env2
        .host
        .seed_entries(vec![json!({
            "type": "custom", "customType": PERMISSION_ENTRY,
            "data": {"mode": "ask"}
        })]);
    let (wf2, _) = env2.load("workflow").await;
    env2.event(
        &wf2,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let restored = topic(&env2, "s1").last().unwrap().clone();
    assert_eq!(
        restored["data"]["permissionMode"], "ask",
        "{restored:?}"
    );
    // The restored mode is the gate's: an Ask chat still asks after the restart.
    env2
        .host
        .queue_dialog_answer(json!({"value": {"optionId": "deny"}}));
    let blocked = env2
        .event(
            &wf2,
            EventKind::ToolCall,
            json!({"type": "tool_call", "toolName": "bash", "toolCallId": "r1", "input": {"command": "rm -rf /tmp/x"}}),
        )
        .await
        .unwrap();
    assert_eq!(blocked["block"], true, "an Ask chat still asks after a restart");

    // Plan mode also reports a readable on-state (G-10) after the restore.
    env2.command(&wf2, "plan", "on").await;
    env2.command(&wf2, "plan", "status").await;
    let notifies = env2.notifies();
    let status = notifies.last().map(String::as_str).unwrap_or_default();
    assert!(
        status.starts_with("Plan mode: on — the plan goes to "),
        "{status}"
    );
}

/// F-06: child workers get the tools their profile grants. A background child's dialogs
/// auto-deny, so an `ask` decision would always be a denial — when the tool is in the
/// child's granted set (`control › active_tools`, the AgentSpec allowlist), the ask is
/// waived. The lead's asks still prompt, and a child without the grant still can't.
#[tokio::test(flavor = "multi_thread")]
async fn a_child_worker_runs_its_profile_granted_bash_without_asking() {
    let env = Env::new();
    let (wf, _) = env.load("workflow").await;
    env.event(
        &wf,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    // auto mode: jev judges ambiguous bash; without a decision model the call asks.
    let child = {
        let mut cx = env.cx(oino_extension_api::CallKind::Event);
        cx.is_child = true;
        cx.session_id = "worker-1".into();
        cx
    };
    let bash = |id: &str| {
        json!({
            "type": "tool_call",
            "toolName": "bash",
            "toolCallId": id,
            "input": {"command": "python3 calc.py"}
        })
    };

    // Without the grant: the ask auto-denies in production; here the dialog is answered
    // with a denial → blocked.
    env.host
        .queue_dialog_answer(json!({"value": {"optionId": "deny"}}));
    let denied = env
        .event_with(&wf, EventKind::ToolCall, bash("c1"), child.clone())
        .await;
    assert!(
        denied.as_ref().is_some_and(|b| b["block"] == true),
        "an ungranted child tool still asks (auto-denies): {denied:?}"
    );

    // With the profile grant (subagent_general grants bash): the child runs it.
    env.host.set_active_tools(&["read", "bash", "edit", "write"]);
    let allowed = env
        .event_with(&wf, EventKind::ToolCall, bash("c2"), child)
        .await;
    assert_eq!(allowed, None, "granted child bash runs without asking");
    env.host.set_active_tools(&[]);

    // The lead (not a child) still gets the dialog for the same call.
    env.host
        .queue_dialog_answer(json!({"value": {"optionId": "allow_once"}}));
    let lead = env.event(&wf, EventKind::ToolCall, bash("c3")).await;
    assert_eq!(lead, None, "the lead approves via dialog");
    assert!(
        env.host
            .calls_named("ui_dialog")
            .iter()
            .any(|c| { c.detail["toolName"] == "bash" }),
        "the lead was asked"
    );
}

/// compactor-fix regression: core caps `session_before_compact` at 4 MiB of `branchEntries`
/// plus 4 MiB of preparation (`oino_core::compaction::HOOK_PAYLOAD_LIMITS`). A payload at
/// both caps must compact inside a 96 MiB linear memory (the 213 MB chat trapped the
/// compactor at 455 → 910 MiB), three times in a row (memory does not ratchet per call).
#[tokio::test(flavor = "multi_thread")]
async fn compactor_lossless_at_the_hook_payload_cap_fits_96_mib() {
    const CAP: usize = 4 << 20;
    let env = Env::new();
    let (ext, _) = env.load_with_memory("compactor", Some(96)).await;
    let file_text = "fn route() { navigate('/home') }\n".repeat(60);
    let mut branch = Vec::new();
    let mut size = 0;
    let mut i = 0;
    while size < CAP - (64 << 10) {
        let e = match i % 3 {
            0 => {
                json!({"id": format!("e{i}"), "type": "message", "message": {"role": "user", "content": format!("turn {i}: fix the router, keep the theme")}})
            }
            1 => {
                json!({"id": format!("e{i}"), "type": "message", "message": {"role": "assistant", "content": [
                {"type": "text", "text": format!("Editing router.tsx ({i}).")},
                {"type": "toolCall", "id": format!("c{i}"), "name": "edit", "arguments": {"path": "src/router.tsx", "oldText": file_text}}]}})
            }
            _ => {
                json!({"id": format!("e{i}"), "type": "message", "message": {"role": "toolResult", "toolCallId": format!("c{}", i - 1), "toolName": "edit",
                "content": [{"type": "text", "text": file_text}], "isError": false}})
            }
        };
        size += e.to_string().len() + 1;
        branch.push(e);
        i += 1;
    }
    let messages: Vec<Value> = branch
        .iter()
        .take(branch.len() - 30)
        .map(|e| e["message"].clone())
        .collect();
    let kept = branch[branch.len() - 30]["id"].clone();
    let payload = json!({"type": "session_before_compact", "reason": "threshold", "willRetry": false,
        "branchEntries": branch,
        "preparation": {"tokensBefore": 180_000, "firstKeptEntryId": kept, "messagesToSummarize": messages,
            "turnPrefixMessages": [], "isSplitTurn": false, "fileOps": {"read": [], "written": [], "edited": ["src/router.tsx"]},
            "settings": {"enabled": true, "reserveTokens": 16384, "keepRecentTokens": 20000}}});
    assert!(payload.to_string().len() > 7 << 20);
    for _ in 0..3 {
        let out = env
            .event(&ext, EventKind::SessionBeforeCompact, payload.clone())
            .await
            .expect("compactor answered (no trap)");
        let c = &out["compaction"];
        assert_eq!(c["details"]["method"], "vcc", "{out}");
        assert!(c["summary"].as_str().unwrap().contains("fix the router"));
        assert!(!c["firstKeptEntryId"].as_str().unwrap().is_empty());
    }
}
