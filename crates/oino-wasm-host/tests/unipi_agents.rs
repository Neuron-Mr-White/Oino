//! End-to-end: the real unipi components (`extensions/dist`, built by `cargo xtask wasm`)
//! on nested agents and the turn arbiter, answered by the native faux provider.
//!
//! Skips (passes without running) when `extensions/dist` is missing.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]
#![cfg(feature = "core")]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use futures::future::BoxFuture;
use indexmap::IndexMap;
use oino_ai::{AgentMessage, JsonValue};
use oino_core::builtin::{SessionToolContext, builtin_tool_entries};
use oino_core::extensions::api::{CallKind, ExtensionInstance, ExtensionMode};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ExtensionLoader, ResourceFlags, Services, ServicesOptions};
use oino_core::session::{AgentSession, PromptOptions};
use oino_core::testing::{FauxComponent, FauxProvider, FauxResponse};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::json;

fn dist() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/dist")
}

fn available(ids: &[&str]) -> bool {
    ids.iter()
        .all(|id| dist().join(id).join("component.wasm").is_file())
}

struct Loader {
    faux: Arc<FauxProvider>,
    wasm: CoreLoader,
}

impl ExtensionLoader for Loader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        Box::pin(async move {
            let mut out: Vec<Arc<dyn ExtensionInstance>> =
                vec![Arc::new(FauxComponent(Arc::clone(&self.faux)))];
            out.extend(self.wasm.load(host, trusted).await);
            out
        })
    }
}

struct Env {
    dir: tempfile::TempDir,
    services: Arc<Services>,
    runtime: AgentSessionRuntime,
    provider: Arc<FauxProvider>,
}

impl Env {
    async fn new(ids: &[&str], responses: Vec<FauxResponse>, settings: JsonValue) -> Self {
        Self::with_files(ids, responses, settings, &[]).await
    }

    /// Like [`Env::new`], copying `(agent-relative path, source)` files in first.
    async fn with_files(
        ids: &[&str],
        responses: Vec<FauxResponse>,
        settings: JsonValue,
        files: &[(&str, PathBuf)],
    ) -> Self {
        let dir = tempfile::Builder::new()
            .prefix("oino-unipi-agents-")
            .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
            .unwrap();
        let agent = dir.path().join("agent");
        let cwd = dir.path().join("work");
        std::fs::create_dir_all(agent.join("cache")).unwrap();
        std::fs::create_dir_all(&cwd).unwrap();
        for (rel, source) in files {
            let target = agent.join(rel);
            std::fs::create_dir_all(target.parent().unwrap()).unwrap();
            std::fs::copy(source, target).unwrap();
        }
        // Share the wasmtime compile cache between runs (debug compiles are slow).
        let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("oino-wasmtime-cache");
        std::fs::create_dir_all(&cache).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&cache, agent.join("cache/wasmtime")).unwrap();
        let provider = FauxProvider::new(responses);
        let wasm = Arc::new(
            WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(&agent, &cwd))).unwrap(),
        );
        let packages = ids
            .iter()
            .map(|id| SourcedPackage {
                package: oino_wasm_host::packages::load_dir(&dist().join(id), true).unwrap(),
                project: false,
            })
            .collect();
        let loader = Loader {
            faux: Arc::clone(&provider),
            wasm: CoreLoader::new(wasm, packages),
        };
        let models =
            oino_models::ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
        models.set_runtime_api_key("faux", "k");
        let JsonValue::Object(settings) = settings else {
            panic!("settings object")
        };
        let services = Services::create(ServicesOptions {
            cwd: cwd.clone(),
            agent_dir: agent.clone(),
            docs_root: dir.path().join("docs"),
            mode: ExtensionMode::Print,
            has_ui: false,
            flags: ResourceFlags {
                no_context_files: true,
                offline: true,
                ..ResourceFlags::default()
            },
            extension_flag_values: IndexMap::new(),
            loader: Arc::new(loader),
            approve: Some(true),
            models: Some(models),
            settings: Some(oino_settings::SettingsManager::in_memory(settings)),
        })
        .await
        .unwrap();
        assert!(
            services
                .diagnostics
                .iter()
                .all(|d| !d.contains("init failed")),
            "{:?}",
            services.diagnostics
        );
        let context = Arc::new(SessionToolContext::default());
        let snapshot = services.settings.lock().unwrap().snapshot();
        let tools = builtin_tool_entries(&cwd, &agent, &snapshot, Arc::clone(&context));
        let runtime = AgentSessionRuntime::create(
            Arc::clone(&services),
            SessionOptions {
                source: SessionSource::New,
                builtin_tools: tools,
                ..SessionOptions::default()
            },
        )
        .await
        .unwrap();
        context.bind(&runtime.session());
        Self {
            dir,
            services,
            runtime,
            provider,
        }
    }

    fn session(&self) -> AgentSession {
        self.runtime.session()
    }

    fn cwd(&self) -> PathBuf {
        self.dir.path().join("work")
    }

    async fn prompt(&self, text: &str) {
        let session = self.session();
        session
            .prompt(text.into(), PromptOptions::default())
            .await
            .unwrap();
        idle(&session).await;
    }

    async fn command(&self, ext: &str, name: &str, args: &str) -> JsonValue {
        let session = self.session();
        let cx = session.runner().call_context().with_kind(CallKind::Command);
        self.services
            .host
            .call(
                ext,
                &cx,
                HostOp::InvokeCommand {
                    name: name.into(),
                    args: args.into(),
                },
            )
            .await
            .unwrap()
    }

    /// Requests the faux provider received whose context mentions `needle`.
    fn requests_with(&self, needle: &str) -> Vec<JsonValue> {
        self.provider
            .requests()
            .into_iter()
            .filter(|r| r["context"].to_string().contains(needle))
            .collect()
    }

    async fn until(&self, what: &str, f: impl Fn(&Self) -> bool) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
        while !f(self) {
            assert!(tokio::time::Instant::now() < deadline, "timed out: {what}");
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

async fn idle(session: &AgentSession) {
    tokio::time::timeout(Duration::from_secs(60), session.wait_for_idle())
        .await
        .unwrap();
}

fn call(id: &str, name: &str, args: JsonValue) -> FauxResponse {
    FauxResponse::ToolCalls(vec![(
        id.into(),
        name.into(),
        serde_json::from_value(args).unwrap(),
    )])
}

fn text(t: &str) -> FauxResponse {
    FauxResponse::Text(t.into())
}

fn tool_result(session: &AgentSession, id: &str) -> oino_ai::ToolResultMessage {
    session
        .messages()
        .into_iter()
        .find_map(|m| match m {
            AgentMessage::ToolResult(r) if r.tool_call_id == id => Some(r),
            _ => None,
        })
        .unwrap_or_else(|| panic!("no tool result {id}"))
}

fn usage_kinds(session: &AgentSession) -> Vec<String> {
    session
        .entries()
        .iter()
        .filter_map(|e| serde_json::to_value(e).ok())
        .filter(|e| e["type"] == "usage")
        .map(|e| e["kind"].as_str().unwrap_or_default().to_owned())
        .collect()
}

const SUBAGENTS: &[&str] = &["unipi.core", "unipi.subagents", "unipi.background-tasks"];

/// The faux answers requests of nested agents whose context contains `marker` with `answer`.
fn route_children(
    provider: &FauxProvider,
    routes: Vec<(&'static str, &'static str, &'static str)>,
) {
    provider.route(move |request| {
        let wire = request["context"].to_string();
        routes
            .iter()
            .find(|(system, marker, _)| wire.contains(system) && wire.contains(marker))
            .map(|(_, _, answer)| text(answer))
    });
}

fn result_text(result: &oino_ai::ToolResultMessage) -> String {
    serde_json::to_string(&result.content).unwrap()
}

const SUBAGENT_SYSTEM: &str = "You are a general-purpose subagent";

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn subagents_run_foreground_background_and_read() {
    if !available(SUBAGENTS) {
        return;
    }
    let env = Env::new(
        SUBAGENTS,
        vec![
            call(
                "c1",
                "run_subagent",
                json!({"title": "scan", "task": "FOREGROUND-TASK", "profile": "subagent_general"}),
            ),
            text("parent saw the report"),
        ],
        json!({}),
    )
    .await;
    route_children(
        &env.provider,
        vec![
            (SUBAGENT_SYSTEM, "FOREGROUND-TASK", "foreground report"),
            (SUBAGENT_SYSTEM, "BACKGROUND-TASK", "background report"),
        ],
    );
    let session = env.session();
    assert!(
        session
            .active_tool_names()
            .iter()
            .any(|t| t == "run_subagent"),
        "{:?}",
        session.active_tool_names()
    );
    env.prompt("delegate").await;
    let result = tool_result(&session, "c1");
    assert!(!result.is_error, "{result:?}");
    assert!(
        result_text(&result).contains("foreground report"),
        "{result:?}"
    );
    assert!(
        usage_kinds(&session)
            .iter()
            .any(|k| k.starts_with("child:"))
    );
    assert_eq!(
        session.last_assistant_text().as_deref(),
        Some("parent saw the report")
    );

    // Background run: the completion notification wakes the parent; read_subagent returns it.
    env.provider.push(call(
        "c2",
        "run_subagent",
        json!({"title": "bg", "task": "BACKGROUND-TASK", "profile": "subagent_general", "is_background": true}),
    ));
    env.provider.push(text("started it"));
    env.provider.push(text("noted the completion"));
    env.prompt("background please").await;
    let started = tool_result(&session, "c2");
    assert!(!started.is_error, "{started:?}");
    let id = started.details.as_ref().unwrap()["id"]
        .as_str()
        .unwrap()
        .to_owned();
    env.until("background completion message", |env| {
        env.session()
            .messages()
            .iter()
            .any(|m| matches!(m, AgentMessage::Custom(c) if c.custom_type == "subagent-completion"))
    })
    .await;
    idle(&session).await;
    env.provider.push(call(
        "c3",
        "read_subagent",
        json!({"agent_id": id, "block": true}),
    ));
    env.provider.push(text("read done"));
    env.prompt("read it").await;
    let read = tool_result(&session, "c3");
    assert!(result_text(&read).contains("background report"), "{read:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn btw_answers_from_a_branch_copy_without_touching_the_session() {
    let ids = ["unipi.core", "unipi.btw"];
    if !available(&ids) {
        return;
    }
    let env = Env::new(&ids, vec![text("MAIN-ANSWER")], json!({})).await;
    route_children(
        &env.provider,
        vec![(
            "one-shot aside conversation",
            "SIDE-QUESTION",
            "aside answer",
        )],
    );
    env.prompt("MAIN-CONTEXT").await;
    let before = env.session().messages().len();
    env.command("unipi.btw", "unipi:btw", "SIDE-QUESTION").await;
    let child = env.requests_with("SIDE-QUESTION");
    assert_eq!(child.len(), 1, "one aside request");
    let wire = child[0]["context"].to_string();
    assert!(
        wire.contains("MAIN-CONTEXT") && wire.contains("MAIN-ANSWER"),
        "{wire}"
    );
    assert_eq!(
        env.session().messages().len(),
        before,
        "aside never reaches the session"
    );
    assert!(usage_kinds(&env.session()).contains(&"child:btw".to_owned()));
    assert!(env.session().child_agents().is_empty(), "aside closed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fusion_sidekick_handoff_runs_a_persistent_child() {
    let ids = ["unipi.core", "unipi.subagents", "unipi.fusion"];
    if !available(&ids) {
        return;
    }
    let preset = Path::new(env!("CARGO_TARGET_TMPDIR")).join("fusion-preset.json");
    std::fs::write(
        &preset,
        json!({"schemaVersion": 1, "active": {"kind": "fusion", "lead": "faux/faux-1", "sidekick": "faux/faux-1"}}).to_string(),
    )
    .unwrap();
    let env = Env::with_files(
        &ids,
        vec![
            call("h1", "sidekick", json!({"message": "HANDOFF-ONE"})),
            text("lead reviewed"),
        ],
        json!({}),
        &[("state/unipi.fusion/global/preset.json", preset)],
    )
    .await;
    route_children(
        &env.provider,
        vec![("Fusion sidekick", "HANDOFF-ONE", "sidekick did it")],
    );
    let session = env.session();
    assert!(
        session.active_tool_names().iter().any(|t| t == "sidekick"),
        "{:?}",
        session.active_tool_names()
    );
    env.prompt("work").await;
    let result = tool_result(&session, "h1");
    assert!(!result.is_error, "{result:?}");
    assert!(
        result_text(&result).contains("sidekick did it"),
        "{result:?}"
    );
    let children = session.child_agents();
    assert_eq!(children.len(), 1, "the sidekick stays open");
    let file = children[0].session_file().unwrap();
    assert!(file.is_file(), "{file:?}");
    assert!(file.to_string_lossy().contains("sidekick"), "{file:?}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn memory_side_save_runs_after_a_writing_task() {
    let ids = ["unipi.core", "unipi.memory"];
    if !available(&ids) {
        return;
    }
    let env = Env::new(
        &ids,
        vec![
            call(
                "w1",
                "write",
                json!({"path": "notes.md", "content": "hello"}),
            ),
            text("wrote it"),
        ],
        json!({}),
    )
    .await;
    route_children(
        &env.provider,
        vec![("Memory save pass", "Memory save pass", "NONE")],
    );
    env.prompt("write a note").await;
    assert!(env.cwd().join("notes.md").is_file());
    env.until("side save request", |env| {
        !env.requests_with("Memory save pass").is_empty()
    })
    .await;
    env.until("side save usage", |env| {
        usage_kinds(&env.session()).contains(&"child:memory save".to_owned())
    })
    .await;
    let request = &env.requests_with("Memory save pass")[0];
    assert!(request["context"].to_string().contains("write a note"));
    let tools: Vec<String> = request["context"]["tools"]
        .as_array()
        .map(|t| {
            t.iter()
                .filter_map(|t| t["name"].as_str().map(ToOwned::to_owned))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        tools
            .iter()
            .all(|t| t.starts_with("memory_") || t.starts_with("global_memory")),
        "{tools:?}"
    );
    env.until("side save closed", |env| {
        env.session().child_agents().is_empty()
    })
    .await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn dream_run_spawns_a_memory_only_agent() {
    let ids = ["unipi.core", "unipi.memory", "unipi.dream"];
    if !available(&ids) {
        return;
    }
    let env = Env::new(&ids, vec![text("first session")], json!({})).await;
    env.provider.route(|request| {
        let wire = request["context"].to_string();
        (wire.contains("dream") && !wire.contains("first prompt")).then(|| text("DREAM-DONE"))
    });
    env.prompt("first prompt").await;
    env.command("unipi.dream", "unipi:dream", "run").await;
    env.until("dream agent usage", |env| {
        usage_kinds(&env.session()).contains(&"child:dream".to_owned())
    })
    .await;
    env.until("dream closed", |env| {
        env.session().child_agents().is_empty()
    })
    .await;
    let runs = env.dir.path().join("agent/state/unipi.dream/global/runs");
    let logs: Vec<String> = walk(&runs)
        .into_iter()
        .filter(|p| p.extension().is_some_and(|e| e == "jsonl"))
        .map(|p| std::fs::read_to_string(p).unwrap())
        .collect();
    assert!(logs.iter().any(|l| l.contains("DREAM-DONE")), "{runs:?}");
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                out.extend(walk(&path));
            } else {
                out.push(path);
            }
        }
    }
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn long_horizon_goal_continues_through_the_arbiter() {
    let ids = ["unipi.core", "unipi.long-horizon"];
    if !available(&ids) {
        return;
    }
    let env = Env::new(&ids, Vec::new(), json!({})).await;
    env.provider.route(|_| Some(text("working on it")));
    let started = env
        .command(
            "unipi.long-horizon",
            "unipi:goal-start",
            &json!({"objective": "GOAL-OBJECTIVE ship the feature"}).to_string(),
        )
        .await;
    assert_eq!(started["ok"], true, "{started}");
    env.prompt("begin").await;
    env.until("a continuation nudge", |env| {
        env.session()
            .messages()
            .iter()
            .any(|m| matches!(m, AgentMessage::Custom(c) if c.custom_type == "unipi:lh-continue"))
    })
    .await;
    let session = env.session();
    assert!(
        env.requests_with("GOAL-OBJECTIVE").len() >= 2,
        "kickoff and continuation reached the model"
    );
    // The goal stops on its own (no-progress stall cap / max turns) or when paused.
    env.command("unipi.long-horizon", "unipi:goal", "pause")
        .await;
    idle(&session).await;
    let requests = env.provider.requests().len();
    assert!(requests >= 2, "{requests}");
}
