//! `unipi.memory` as a real component in the wasm host: the store/list/search
//! tools, the `/memory` command, and the **browse panel** (WP-A25, G-9) —
//! list → search → open → forget, all through `_oino/ui_action`.
//!
//! No MemPalace is installed in the harness, so the extension takes its
//! markdown-only path: memories are files under
//! `<agent>/state/unipi.memory/global/<project>/…` and search is the local
//! (keyword) search. That is exactly the degraded mode a user without
//! `mempalace` gets, so the assertions cover a real configuration.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, OnceLock};

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson, Registration,
    ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

/// The extension under test.
const DIR: &str = "memory";
const CRATE: &str = "unipi_memory";
/// The panel id the browse panel uses.
const PANEL: &str = "unipi.memory/browse";

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// Builds the component once (release, wasm32-wasip2).
fn built() -> &'static PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let ws =
            std::env::var("UNIPI_MEMORY_WORKSPACE").map_or_else(|_| extensions_dir(), PathBuf::from);
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(&ws)
            .args(["build", "-j", "2", "--release", "--target", "wasm32-wasip2"])
            .args(["-p", &format!("unipi-{DIR}")]);
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
    /// A folder named `unipi`: the project name the memory files are filed under.
    project: tempfile::TempDir,
}

impl Env {
    fn new() -> Self {
        let agent = tempfile::tempdir().unwrap();
        let project = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(project.path().join("unipi")).unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host: RecordingHostServices::new(),
            agent,
            project,
        }
    }

    fn cwd(&self) -> PathBuf {
        self.project.path().join("unipi")
    }

    fn cx(&self, kind: CallKind) -> CallContext {
        CallContext {
            session_id: "s1".into(),
            mode: Mode::Rpc,
            has_ui: true,
            cwd: self.cwd(),
            kind,
            ..CallContext::default()
        }
    }

    async fn load(&self) -> Arc<dyn ExtensionInstance> {
        let wasm = built().join(format!("{CRATE}.wasm"));
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join(format!("unipi/{DIR}/extension.json")))
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
        let settings = json!({"memory": {"write": true, "recallAtStart": false, "saveMode": "off"}});
        let input = InitInput {
            settings: RawJson::from(settings),
            ..InitInput::default()
        };
        let _reg: Registration = ext.init(input, self.cx(CallKind::Init)).await.unwrap();
        ext
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

    async fn store(&self, ext: &Arc<dyn ExtensionInstance>, title: &str, content: &str) {
        let inv = ToolInvocation {
            tool_call_id: format!("c-{title}"),
            name: "memory_store".into(),
            args: RawJson::from(json!({"title": title, "content": content, "type": "decision"})),
            parent_tool_call_id: None,
        };
        let result = ext
            .run_tool(inv, CancellationToken::new(), self.cx(CallKind::Tool))
            .await
            .unwrap()
            .to_value();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.starts_with("Stored memory:"), "{text}");
    }

    /// One `_oino/ui_action` from the panel.
    async fn action(&self, ext: &Arc<dyn ExtensionInstance>, action: &str, extra: Value) {
        let mut payload = json!({"panelId": PANEL, "actionId": action});
        if let Some(object) = extra.as_object() {
            for (key, value) in object {
                payload[key] = value.clone();
            }
        }
        ext.on_event(
            EventKind::OinoUiAction,
            RawJson::from(payload),
            self.cx(CallKind::Event),
        )
        .await
        .unwrap();
    }

    /// Every `ui_panel` call, oldest first.
    fn panels(&self) -> Vec<Value> {
        self.host
            .calls_named("ui_panel")
            .into_iter()
            .map(|c| c.detail)
            .collect()
    }

    /// The last panel spec sent (open or update).
    fn last_spec(&self) -> Value {
        let calls = self.panels();
        let last = calls.last().unwrap();
        last.get("open")
            .or_else(|| last.get("spec"))
            .cloned()
            .unwrap_or(Value::Null)
    }

    fn memory_dir(&self) -> PathBuf {
        self.agent
            .path()
            .join("state/unipi.memory/global/memory/unipi")
    }

    /// One extension event in session `s1`.
    async fn event(&self, ext: &Arc<dyn ExtensionInstance>, kind: EventKind, payload: Value) {
        ext.on_event(kind, RawJson::from(payload), self.cx(CallKind::Event))
            .await
            .unwrap();
    }

    /// How many wake-up cards were appended to the session file.
    fn cards(&self) -> usize {
        self.host
            .calls_named("session_write")
            .iter()
            .filter(|c| c.detail["appendEntry"] == json!("unipi-memory-session-card"))
            .count()
    }
}

#[tokio::test]
async fn workspace_free_superagent_search_reads_known_memory() {
    let env = Env::new();
    let ext = env.load().await;
    env.store(&ext, "Known workspace memory", "A unique cobalt badger detail beyond the title.").await;
    let mut cx = env.cx(CallKind::Tool);
    cx.cwd = PathBuf::new();
    for name in ["memory_search", "global_memory_search"] {
        let inv = ToolInvocation {
            tool_call_id: format!("workspace-free-{name}"),
            name: name.into(),
            args: RawJson::from(json!({"query":"cobalt badger", "scope":"all"})),
            parent_tool_call_id: None,
        };
        let result = ext.run_tool(inv, CancellationToken::new(), cx.clone()).await.unwrap().to_value();
        let text = result["content"][0]["text"].as_str().unwrap();
        assert!(text.contains("Known workspace memory"), "{name}: {text}");
        assert!(text.contains("A unique cobalt badger detail beyond the title."), "{name}: {text}");
    }
}

/// All text of a node tree (the rows the frontend would draw).
fn text_of(node: &Value) -> String {
    let mut out = String::new();
    collect(node, &mut out);
    out
}

fn collect(node: &Value, out: &mut String) {
    match node {
        Value::String(s) => {
            out.push_str(s);
            out.push('\n');
        }
        Value::Array(items) => {
            for item in items {
                collect(item, out);
            }
        }
        Value::Object(map) => {
            for value in map.values() {
                collect(value, out);
            }
        }
        _ => {}
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_panel_lists_searches_and_forgets() {
    let env = Env::new();
    let ext = env.load().await;
    env.store(&ext, "Decisions use the oino host", "The host owns sessions; the app is a client.")
        .await;
    env.store(&ext, "Graph retries from its own session", "A transient node failure retries in place.")
        .await;
    env.store(&ext, "Prefers short reports", "Short reports with evidence, no preamble.")
        .await;
    assert!(env.memory_dir().exists(), "the store wrote markdown files");
    let files_before = std::fs::read_dir(env.memory_dir().join("decision"))
        .unwrap()
        .count();
    assert_eq!(files_before, 3);

    // `/memory` (no argument) opens the browse panel: header + search + list.
    env.command(&ext, "memory", "").await;
    let opened = env.panels();
    assert_eq!(opened.len(), 1, "one openPanel");
    let spec = opened[0].get("open").cloned().unwrap();
    assert_eq!(spec["panelId"], PANEL);
    assert_eq!(spec["placement"], "overlay");
    let text = text_of(&spec["spec"]);
    assert!(text.contains("Memory · unipi · 3 memories"), "{text}");
    assert!(text.contains("Decisions use the oino host"), "{text}");
    assert!(text.contains("Graph retries from its own session"), "{text}");

    // Search: `values.q` filters through the extension's search path.
    env.action(&ext, "search", json!({"values": {"q": "graph"}}))
        .await;
    let text = text_of(&env.last_spec());
    assert!(text.contains("1 match"), "{text}");
    assert!(text.contains("Graph retries from its own session"), "{text}");
    assert!(!text.contains("Prefers short reports"), "{text}");

    // Search with no hit says so and keeps the panel.
    env.action(&ext, "search", json!({"values": {"q": "zzzz-nothing"}}))
        .await;
    let text = text_of(&env.last_spec());
    assert!(text.contains("No memory matches that search."), "{text}");

    // Open the first memory (the list's `on_activate`), then forget it.
    env.action(&ext, "search", json!({"values": {"q": ""}})).await;
    let open_key = "decisions_use_the_oino_host";
    assert!(env.memory_dir().join("decision").join(format!("{open_key}.md")).exists());
    env.action(&ext, "open", json!({"value": open_key})).await;
    let text = text_of(&env.last_spec());
    assert!(text.contains("The host owns sessions; the app is a client."), "{text}");
    env.action(&ext, "forget", json!({})).await;
    let text = text_of(&env.last_spec());
    assert!(text.contains(&format!("Forgot {open_key}")), "{text}");
    assert!(
        !env.memory_dir().join("decision").join(format!("{open_key}.md")).exists(),
        "the file is gone"
    );
    // The list refreshed: two memories left.
    assert!(text.contains("2 memories"), "{text}");

    // All projects / this project: the scope button flips the header.
    env.action(&ext, "scope", json!({})).await;
    let text = text_of(&env.last_spec());
    assert!(text.contains("all projects"), "{text}");
    env.action(&ext, "scope", json!({})).await;
    assert!(text_of(&env.last_spec()).contains("Memory · unipi"));

    // Close drops the panel entry.
    env.action(&ext, "close", json!({})).await;
    let calls = env.panels();
    assert_eq!(calls.last().unwrap()["close"], PANEL);
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_panel_empty_state_and_no_ui() {
    let env = Env::new();
    let ext = env.load().await;
    env.command(&ext, "memory", "").await;
    let text = text_of(&env.last_spec());
    assert!(text.contains("0 memories"), "{text}");
    assert!(text.contains("Nothing remembered here yet"), "{text}");

    // `/memory status` still answers as text (the panel is the default, not the only command).
    env.command(&ext, "memory", "status").await;
    assert!(
        !env.panels().is_empty(),
        "the panel was opened by the first call"
    );
    let texts: Vec<String> = env
        .host
        .calls_named("ui_fire")
        .into_iter()
        .filter_map(|c| c.detail.get("message").map(|m| m.to_string()))
        .collect();
    assert!(
        texts.iter().any(|t| t.contains("project memories")),
        "status notified: {texts:?}"
    );
}

/// The panel's spec is a valid node tree for the app's NodeView kinds (§4.5).
#[tokio::test(flavor = "multi_thread")]
async fn memory_panel_spec_uses_known_node_kinds() {
    let env = Env::new();
    let ext = env.load().await;
    env.store(&ext, "A memory", "Body").await;
    env.command(&ext, "memory", "").await;
    let spec = env.last_spec();
    let text = spec["spec"].to_string();
    assert!(text.contains("\"t\":\"Layout\""), "{text}");
    assert!(text.contains("\"t\":\"Input\""), "{text}");
    assert!(text.contains("\"t\":\"List\""), "{text}");
    assert!(text.contains("\"t\":\"Button\""), "{text}");
    assert!(text.contains("\"t\":\"KeyHints\""), "{text}");
}

/// E-20: opening (and re-opening) a chat must not append the wake-up card — it is
/// written once, when the session runs its first real turn.
#[tokio::test(flavor = "multi_thread")]
async fn session_card_is_written_once_on_a_real_turn_never_on_view() {
    let env = Env::new();
    let ext = env.load().await;
    let start = json!({"type": "session_start", "reason": "startup"});
    let turn = json!({"type": "before_agent_start"});

    // View the chat: opens fire `session_start` — no entries.
    env.event(&ext, EventKind::SessionStart, start.clone()).await;
    env.event(&ext, EventKind::SessionShutdown, json!({"type": "session_shutdown"})).await;
    env.event(&ext, EventKind::SessionStart, start.clone()).await;
    assert_eq!(env.cards(), 0, "viewing a chat must not write entries");

    // The first real turn writes exactly one card.
    env.event(&ext, EventKind::BeforeAgentStart, turn.clone()).await;
    assert_eq!(env.cards(), 1);

    // Close and re-open, then another turn: still one card for this session.
    env.event(&ext, EventKind::SessionShutdown, json!({"type": "session_shutdown"})).await;
    env.event(&ext, EventKind::SessionStart, start).await;
    env.event(&ext, EventKind::BeforeAgentStart, turn).await;
    assert_eq!(env.cards(), 1, "re-opening a chat must not repeat the card");
}

impl Env {
    /// One tool call in session `s1` (or a child session with `role`).
    async fn tool(
        &self,
        ext: &Arc<dyn ExtensionInstance>,
        name: &str,
        args: Value,
        role: Option<&str>,
    ) -> Value {
        let inv = ToolInvocation {
            tool_call_id: format!("c-{name}"),
            name: name.into(),
            args: RawJson::from(args),
            parent_tool_call_id: None,
        };
        let mut cx = self.cx(CallKind::Tool);
        if let Some(r) = role {
            cx.is_child = true;
            cx.child_role = Some(r.into());
        }
        ext.run_tool(inv, CancellationToken::new(), cx)
            .await
            .unwrap()
            .to_value()
    }

    async fn inventory(&self, ext: &Arc<dyn ExtensionInstance>, payload: Value) -> Value {
        ext.handle_service(
            "memory.inventory",
            RawJson::from(payload),
            &oino_extension_api::ExtensionId::new("unipi.dream").unwrap(),
            self.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value()
    }
}

fn row<'a>(inv: &'a Value, project: &str, id: &str) -> Option<&'a Value> {
    inv["memories"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["project"] == project && m["id"] == id)
}

/// v3 J2: per-memory usage for the dream pass — searches and reads count, dream-agent
/// searches don't, stores record their source session, `merge_from` folds overlapping
/// memories (usage carried over), `project` targets another bucket.
#[tokio::test(flavor = "multi_thread")]
async fn usage_tracking_inventory_merge_and_project_target() {
    let env = Env::new();
    let ext = env.load().await;
    env.store(&ext, "Graph retries from its own session", "A transient node failure retries in place.")
        .await;
    env.store(&ext, "Graph retry keeps the session", "Retries reuse the node's session.")
        .await;
    env.store(&ext, "Prefers short reports", "Short reports with evidence.").await;

    let inv = env.inventory(&ext, json!({"projects": ["unipi"]})).await;
    assert_eq!(inv["memories"].as_array().unwrap().len(), 3, "{inv}");
    let a = row(&inv, "unipi", "graph_retries_from_its_own_session").unwrap();
    assert_eq!(a["useCount"], 0);
    assert_eq!(a["sources"], json!(["s1"]), "the storing session is the source");
    assert_eq!(a["stale"], false, "tracking just began");
    assert!(inv["trackingSince"].as_i64().unwrap() > 0);

    // A search hit is a use; the same search by the dream agent is not.
    let r = env.tool(&ext, "memory_search", json!({"query": "graph"}), None).await;
    assert!(r["content"][0]["text"].as_str().unwrap().contains("Found"), "{r}");
    env.tool(&ext, "memory_search", json!({"query": "graph"}), Some("dream")).await;
    let inv = env.inventory(&ext, json!({"projects": ["unipi"]})).await;
    let a = row(&inv, "unipi", "graph_retries_from_its_own_session").unwrap();
    assert_eq!(a["useCount"], 1, "{a}");
    assert!(a["lastUsed"].as_i64().unwrap() > 0);
    assert_eq!(a["sessions"], json!(["s1"]));
    assert_eq!(row(&inv, "unipi", "prefers_short_reports").unwrap()["useCount"], 0);

    // Reading the memory file directly is a use too.
    let file = env
        .memory_dir()
        .join("decision/graph_retry_keeps_the_session.md")
        .display()
        .to_string();
    env.event(
        &ext,
        EventKind::ToolCall,
        json!({"type": "tool_call", "toolName": "read", "toolCallId": "r1", "input": {"file_path": file}}),
    )
    .await;
    let inv = env.inventory(&ext, json!({"projects": ["unipi"]})).await;
    assert_eq!(row(&inv, "unipi", "graph_retry_keeps_the_session").unwrap()["useCount"], 2);

    // Consolidation: one memory replaces both graph notes; usage carries over.
    let r = env
        .tool(
            &ext,
            "memory_store",
            json!({"title": "Graph retries in place", "content": "A transient failure retries in the node's own session.", "type": "pattern",
                   "merge_from": ["Graph retries from its own session", "graph_retry_keeps_the_session"]}),
            Some("dream"),
        )
        .await;
    assert_eq!(r["details"]["action"], "merged", "{r}");
    assert_eq!(r["details"]["merged"].as_array().unwrap().len(), 2, "{r}");
    assert!(!env.memory_dir().join("decision/graph_retries_from_its_own_session.md").exists());
    let inv = env.inventory(&ext, json!({"projects": ["unipi"]})).await;
    assert_eq!(inv["memories"].as_array().unwrap().len(), 2, "{inv}");
    let m = row(&inv, "unipi", "graph_retries_in_place").unwrap();
    assert_eq!(m["useCount"], 3, "{m}");
    assert_eq!(
        m["mergedFrom"],
        json!(["unipi/graph_retries_from_its_own_session", "unipi/graph_retry_keeps_the_session"])
    );

    // `project` files a memory under another bucket; delete with `project` forgets it.
    let r = env
        .tool(&ext, "memory_store", json!({"title": "Other lesson", "content": "x", "project": "Other Repo"}), None)
        .await;
    assert_eq!(r["details"]["project"], "other_repo", "{r}");
    let inv = env.inventory(&ext, json!({"projects": ["other_repo"]})).await;
    assert_eq!(inv["memories"].as_array().unwrap().len(), 1);
    let listed = env
        .tool(&ext, "memory_list", json!({"project": "other_repo"}), None)
        .await;
    assert!(listed["content"][0]["text"].as_str().unwrap().contains("Other lesson"));
    let r = env
        .tool(&ext, "memory_delete", json!({"title": "Other lesson", "project": "other_repo"}), None)
        .await;
    assert_eq!(r["details"]["deleted"], true, "{r}");
    let inv = env.inventory(&ext, json!({})).await;
    assert!(row(&inv, "other_repo", "other_lesson").is_none());
    let usage: Value = serde_json::from_slice(
        &std::fs::read(env.agent.path().join("state/unipi.memory/global/memory/.usage.json")).unwrap(),
    )
    .unwrap();
    assert!(usage["memories"].get("other_repo/other_lesson").is_none(), "{usage}");
}
