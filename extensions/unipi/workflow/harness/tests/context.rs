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
        let file = CRATES.iter().find(|(d, _)| *d == dir).unwrap().1;
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join(format!("unipi/{dir}/extension.json")))
                .unwrap(),
        )
        .unwrap();
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
        ("unipi:permission", false),
        ("unipi:plan", false),
        ("unipi:permission-clear-rules", true),
        ("unipi:plan-off", true),
        ("unipi:plan-enter", true),
        ("unipi:plan-exit", true),
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

    env.command(&ext, "unipi:plan", "on").await;
    let entries = env.appended("unipi:plan-mode");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0]["data"]["active"], true);
    assert!(
        env.notifies()
            .iter()
            .any(|n| n.starts_with("Plan mode on — plan file docs/plans/")),
        "{:?}",
        env.notifies()
    );
    let write = env.event(&ext, EventKind::ToolCall, json!({"type": "tool_call", "toolName": "write", "toolCallId": "c3", "input": {"path": "src/main.rs", "content": "x"}})).await.unwrap();
    assert_eq!(write["block"], true);
    assert!(
        write["reason"]
            .as_str()
            .unwrap()
            .starts_with("Plan mode: only the plan file")
    );
    let reminder = env.event(&ext, EventKind::BeforeAgentStart, json!({"type": "before_agent_start", "prompt": "go", "systemPrompt": "", "systemPromptOptions": {}})).await.unwrap();
    assert_eq!(reminder["message"]["customType"], "unipi:plan-mode-message");
    assert_eq!(reminder["message"]["display"], false);
}

#[tokio::test(flavor = "multi_thread")]
async fn skill_registry_exposure() {
    let env = Env::new();
    let (ext, reg) = env.load("skill-registry").await;
    assert_eq!(
        names(&reg.commands, |c| (c.name.as_str(), c.hidden)),
        [
            ("unipi:skills".to_owned(), false),
            ("unipi:skills-editor".to_owned(), true)
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
    env.command(&ext, "unipi:skills", "").await;
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
        ["unipi:dream"]
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
    env.command(&ext, "unipi:dream", "status").await;
    assert!(
        env.notifies()[0].starts_with("dream: off — turn on in /unipi:settings → Dream"),
        "{:?}",
        env.notifies()
    );
    env.command(&ext, "unipi:dream", "bogus").await;
    assert_eq!(
        env.notifies()[1],
        "usage: /unipi:dream status|tray|report|run|stop|approve <n>|reject <n>"
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
            "unipi:compact-vcc",
            "unipi:compact-by-llm",
            "unipi:session-recall",
            "unipi:compact-stats",
            "unipi:compact-doctor",
            "unipi:compact-help"
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
    let then = env.event(&ext, EventKind::Input, json!({"type": "input", "text": "/unipi:compact-then next", "source": "interactive", "streamingBehavior": "followUp"})).await;
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
            "unipi:memory",
            "unipi:memory-process",
            "unipi:memory-consolidate",
            "unipi:memory-search",
            "unipi:global-memory-search",
            "unipi:memory-forget",
            "unipi:global-memory-list"
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
