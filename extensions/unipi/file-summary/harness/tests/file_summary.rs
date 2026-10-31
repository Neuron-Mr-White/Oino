//! `unipi.file-summary` as a real component in the wasm host: one snapshot per path per turn,
//! `existed:false` for new files, `skipped` over 5 MB, no capture of `bash`, the `agent_end`
//! custom entry with `+X −Y`, kill-mid-turn resume from kv + entries, `undo`/`undoFrom`, blob
//! dedup and the turn-footer node at 40/100 columns.

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

use oino_extension_api::manifest::{FsAccess, Manifest};
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, ComponentSource, EventKind, ExtensionId, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

const ID: &str = "unipi.file-summary";
const TURN_CUSTOM_TYPE: &str = "unipi.file-summary/turn";
const UNDO_CUSTOM_TYPE: &str = "unipi.file-summary/undo";

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn extensions_dir() -> PathBuf {
    harness_dir().join("../../..").canonicalize().unwrap()
}

/// Builds the component once (release, wasm32-wasip2).
fn wasm() -> &'static PathBuf {
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
            "unipi-file-summary",
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
        extensions_dir().join("target/wasm32-wasip2/release/unipi_file_summary.wasm")
    })
}

fn user_entry(id: &str) -> Value {
    json!({
        "type": "message",
        "id": id,
        "parentId": null,
        "timestamp": "2026-01-01T00:00:00.000Z",
        "message": {"role": "user", "content": "go", "timestamp": 1},
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
        let project = tempfile::tempdir().unwrap();
        let mut options = WasmRuntimeOptions::new(HostConfig::new(agent.path(), project.path()));
        options.cache_dir = Some(extensions_dir().join("target/oino-wasm-test-cache"));
        let host = RecordingHostServices::new();
        host.seed_entries(vec![user_entry("u1")]);
        Self {
            runtime: WasmRuntime::new(options).unwrap(),
            host,
            agent,
            project,
        }
    }

    /// SIGKILL of the host process (core.md §2.1.8): the runtime and every guest instance die;
    /// the agent dir (kv, blobs) and the recorded session entries survive.
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

    fn package(&self) -> LoadedPackage {
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join("unipi/file-summary/extension.json"))
                .unwrap(),
        )
        .unwrap();
        LoadedPackage {
            manifest,
            component: ComponentSource::Path(wasm().clone()),
            root: None,
            bundled: true,
        }
    }

    async fn load_without_init(&self) -> Arc<dyn ExtensionInstance> {
        let host: Arc<dyn HostServices> = Arc::clone(&self.host) as Arc<dyn HostServices>;
        self.runtime.load(self.package(), host).await.unwrap()
    }

    async fn load(&self) -> Arc<dyn ExtensionInstance> {
        let ext = self.load_without_init().await;
        ext.init(InitInput::default(), self.cx(CallKind::Init))
            .await
            .unwrap();
        ext
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
            .map(|result| result.to_value())
    }

    async fn tool_call(&self, ext: &Arc<dyn ExtensionInstance>, tool: &str, path: &str) {
        self.event(
            ext,
            EventKind::ToolCall,
            json!({
                "type": "tool_call",
                "toolName": tool,
                "toolCallId": format!("call-{tool}"),
                "input": {"path": path},
            }),
        )
        .await;
    }

    async fn agent_end(&self, ext: &Arc<dyn ExtensionInstance>) {
        self.event(
            ext,
            EventKind::AgentEnd,
            json!({"type": "agent_end", "messages": []}),
        )
        .await;
    }

    async fn service(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, payload: Value) -> Value {
        ext.handle_service(
            name,
            RawJson::from(payload),
            &ExtensionId::new("app").unwrap(),
            self.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value()
    }

    fn render(&self, ext: &Arc<dyn ExtensionInstance>, request: Value) -> Option<Value> {
        ext.render(RawJson::from(request), self.cx(CallKind::Render))
            .map(|node| node.to_value())
    }

    fn project_path(&self, rel: &str) -> PathBuf {
        self.project.path().join(rel)
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.project_path(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.project_path(rel)).unwrap()
    }

    fn session_dir(&self) -> PathBuf {
        self.agent
            .path()
            .join("state")
            .join(ID)
            .join("session")
            .join("s1")
    }

    fn blobs(&self) -> Vec<PathBuf> {
        let dir = self.session_dir().join("blobs");
        let Ok(entries) = std::fs::read_dir(dir) else {
            return Vec::new();
        };
        let mut files: Vec<PathBuf> = entries
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "zst"))
            .collect();
        files.sort();
        files
    }

    fn kv(&self) -> Value {
        std::fs::read(self.session_dir().join("kv.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_else(|| json!({}))
    }

    /// `unipi.file-summary/turn` payloads, in append order.
    fn turns(&self) -> Vec<Value> {
        self.host
            .entries()
            .into_iter()
            .filter(|entry| entry["customType"] == TURN_CUSTOM_TYPE)
            .filter_map(|entry| {
                let data = entry.get("data")?.clone();
                (data.is_object()).then_some(data)
            })
            .collect()
    }

    /// `unipi.file-summary/undo` payloads, in append order.
    fn undos(&self) -> Vec<Value> {
        self.host
            .entries()
            .into_iter()
            .filter(|entry| entry["customType"] == UNDO_CUSTOM_TYPE)
            .filter_map(|entry| {
                let data = entry.get("data")?.clone();
                (data.is_object()).then_some(data)
            })
            .collect()
    }

    fn footer_invalidations(&self) -> Vec<Value> {
        self.host
            .calls_named("ui_fire")
            .into_iter()
            .map(|call| call.detail)
            .filter(|detail| detail["method"] == "invalidateTurnFooter")
            .collect()
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_owned))
                .collect()
        })
        .unwrap_or_default()
}

fn file<'a>(turn: &'a Value, path: &str) -> &'a Value {
    turn["files"]
        .as_array()
        .unwrap()
        .iter()
        .find(|file| file["path"] == path)
        .unwrap_or_else(|| panic!("{path} in {turn}"))
}

fn lines(node: &Value, width: u16) -> Vec<String> {
    let node: oino_ui::node::Node = serde_json::from_value(node.clone()).unwrap();
    oino_ui::plain::to_lines(&node, width)
}

#[tokio::test(flavor = "multi_thread")]
async fn registration() {
    let env = Env::new();
    let ext = env.load_without_init().await;
    let registration = ext
        .init(InitInput::default(), env.cx(CallKind::Init))
        .await
        .unwrap();
    let renderers: Vec<(String, String)> = registration
        .renderers
        .iter()
        .map(|spec| (format!("{:?}", spec.target), spec.key.clone()))
        .collect();
    assert!(
        renderers.contains(&("TurnFooter".into(), "files".into())),
        "{renderers:?}"
    );
    assert_eq!(registration.services, ["turn", "diff", "undo", "undoFrom"]);
    for kind in [
        EventKind::SessionStart,
        EventKind::ToolCall,
        EventKind::AgentEnd,
    ] {
        assert!(registration.events.contains(&kind), "{kind:?}");
    }
    let manifest = ext.manifest();
    assert_eq!(manifest.id.as_str(), ID);
    assert_eq!(manifest.ui.reserved, ["unipi.file-summary/turn-files"]);
    // Ownership derives from the host registration (the SDK table starts empty).
    oino_extension_api::manifest::register_reserved_owner(
        "unipi.file-summary/turn-files",
        "unipi.file-summary",
    );
    assert!(manifest.owns_reserved("unipi.file-summary/turn-files"));
    assert_eq!(manifest.permissions.fs.project, FsAccess::Write);
    let _ = ExtensionId::new(ID).unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn one_snapshot_per_path_per_turn_with_counts() {
    let env = Env::new();
    env.write("src/a.txt", "a\nb\nc\n");
    let ext = env.load().await;
    env.tool_call(&ext, "write", "src/a.txt").await;
    env.tool_call(&ext, "edit", "./src/a.txt").await;
    env.tool_call(&ext, "write", "src/a.txt").await;

    let running = env
        .service(&ext, "turn", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(running["running"], true);
    assert_eq!(running["files"].as_array().unwrap().len(), 1, "{running}");
    assert_eq!(file(&running, "src/a.txt")["existed"], true);
    assert!(file(&running, "src/a.txt")["beforeSha"].is_string());
    assert_eq!(env.blobs().len(), 1);

    env.write("src/a.txt", "a\nB\nc\nd\n");
    env.tool_call(&ext, "write", "src/new.txt").await;
    env.write("src/new.txt", "x\ny\n");
    let diff = env
        .service(
            &ext,
            "diff",
            json!({"turnEntryId": "u1", "path": "src/a.txt"}),
        )
        .await;
    let text = diff["diff"].as_str().unwrap();
    assert!(
        text.starts_with("--- a/src/a.txt\n+++ b/src/a.txt\n"),
        "{text}"
    );
    assert!(text.contains("-b\n+B\n"), "{text}");
    assert!(text.contains("+d\n"), "{text}");

    env.agent_end(&ext).await;
    let turns = env.turns();
    assert_eq!(turns.len(), 1);
    let turn = &turns[0];
    assert_eq!(turn["turnEntryId"], "u1");
    assert_eq!(turn["files"].as_array().unwrap().len(), 2);
    let a = file(turn, "src/a.txt");
    assert_eq!((a["add"].as_u64(), a["del"].as_u64()), (Some(2), Some(1)));
    assert!(a["afterSha"].is_string());
    let new = file(turn, "src/new.txt");
    assert_eq!(new["existed"], false);
    assert_eq!(
        (new["add"].as_u64(), new["del"].as_u64()),
        (Some(2), Some(0))
    );
    assert_eq!(env.blobs().len(), 3, "a before, a after, new after");

    let finished = env
        .service(&ext, "turn", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(finished["running"], false);
    assert_eq!(finished["files"], turn["files"]);
    assert_eq!(env.kv(), json!({}), "the in-progress index is deleted");
    let fired = env.footer_invalidations();
    assert_eq!(fired.len(), 1, "{fired:?}");
    assert_eq!(fired[0]["key"], "files");
    assert_eq!(fired[0]["turnEntryIds"], json!(["u1"]));
}

#[tokio::test(flavor = "multi_thread")]
async fn large_file_is_listed_but_skipped() {
    let env = Env::new();
    let big = "x".repeat(6 * 1024 * 1024);
    env.write("big.bin", &big);
    let ext = env.load().await;
    env.tool_call(&ext, "write", "big.bin").await;
    let running = env
        .service(&ext, "turn", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(file(&running, "big.bin")["skipped"], "too_large");
    assert_eq!(file(&running, "big.bin")["existed"], true);
    assert!(file(&running, "big.bin")["beforeSha"].is_null());
    assert!(env.blobs().is_empty(), "a skipped file stores no blob");

    env.agent_end(&ext).await;
    let turn = &env.turns()[0];
    assert_eq!(file(turn, "big.bin")["skipped"], "too_large");
    let undone = env
        .service(&ext, "undo", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(undone["found"], true);
    assert_eq!(strings(&undone["restored"]), Vec::<String>::new());
    assert_eq!(strings(&undone["deleted"]), Vec::<String>::new());
    assert_eq!(env.read("big.bin"), big, "skipped files are never restored");
}

#[tokio::test(flavor = "multi_thread")]
async fn bash_and_extension_tools_are_not_captured() {
    let env = Env::new();
    let ext = env.load().await;
    env.tool_call(&ext, "bash", "src/a.txt").await;
    env.tool_call(&ext, "write_file", "src/a.txt").await;
    let running = env
        .service(&ext, "turn", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(running["files"], json!([]));
    env.agent_end(&ext).await;
    assert!(env.turns().is_empty());
    assert!(env.footer_invalidations().is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn kill_mid_turn_resumes_from_kv_and_entries() {
    let mut env = Env::new();
    env.write("a.txt", "one\n");
    let ext = env.load().await;
    env.tool_call(&ext, "write", "a.txt").await;
    assert!(env.kv().get("turn:u1").is_some(), "{}", env.kv());

    // Kill the host mid-turn.
    env.restart();
    let resumed = env.load().await;
    let running = env
        .service(&resumed, "turn", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(running["running"], true);
    assert_eq!(file(&running, "a.txt")["existed"], true);
    assert!(file(&running, "a.txt")["beforeSha"].is_string());

    // A second touch after the restart must not snapshot the path again.
    env.tool_call(&resumed, "write", "a.txt").await;
    env.write("a.txt", "one\ntwo\n");
    env.agent_end(&resumed).await;
    let turns = env.turns();
    assert_eq!(turns.len(), 1, "{turns:?}");
    assert_eq!(turns[0]["files"].as_array().unwrap().len(), 1);
    assert_eq!(
        (
            file(&turns[0], "a.txt")["add"].as_u64(),
            file(&turns[0], "a.txt")["del"].as_u64()
        ),
        (Some(1), Some(0))
    );
    assert_eq!(env.kv(), json!({}));

    // Kill again: the entry (not the guest memory) answers the summary.
    env.restart();
    let again = env.load().await;
    let summary = env
        .service(&again, "turn", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(summary["running"], false);
    assert_eq!(summary["files"], turns[0]["files"]);
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_restores_deletes_and_needs_force_on_conflict() {
    let env = Env::new();
    env.write("a.txt", "one\n");
    let ext = env.load().await;
    env.tool_call(&ext, "write", "a.txt").await;
    env.write("a.txt", "two\n");
    env.tool_call(&ext, "write", "new.txt").await;
    env.write("new.txt", "x\n");
    env.agent_end(&ext).await;

    env.write("a.txt", "changed by hand\n");
    let conflict = env
        .service(
            &ext,
            "undo",
            json!({"turnEntryId": "u1", "paths": ["a.txt"]}),
        )
        .await;
    assert_eq!(strings(&conflict["conflicts"]), ["a.txt"]);
    assert_eq!(strings(&conflict["restored"]), Vec::<String>::new());
    assert_eq!(env.read("a.txt"), "changed by hand\n");

    let forced = env
        .service(
            &ext,
            "undo",
            json!({"turnEntryId": "u1", "paths": ["a.txt"], "force": true}),
        )
        .await;
    assert_eq!(strings(&forced["restored"]), ["a.txt"]);
    assert_eq!(strings(&forced["conflicts"]), Vec::<String>::new());
    assert_eq!(env.read("a.txt"), "one\n");

    let deleted = env
        .service(
            &ext,
            "undo",
            json!({"turnEntryId": "u1", "paths": ["new.txt"]}),
        )
        .await;
    assert_eq!(strings(&deleted["deleted"]), ["new.txt"]);
    assert!(!env.project_path("new.txt").exists());

    let undos = env.undos();
    assert_eq!(undos.len(), 3, "{undos:?}");
    assert_eq!(undos[0]["turnEntryId"], "u1");
    assert_eq!(undos[2]["deleted"], json!(["new.txt"]));
    let summary = env
        .service(&ext, "turn", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(summary["undone"], true);
    assert_eq!(summary["undonePaths"], json!(["a.txt", "new.txt"]));
    assert_eq!(
        env.footer_invalidations().len(),
        4,
        "one at agent_end, one per undo"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn undo_from_undoes_newest_first() {
    let env = Env::new();
    env.write("a.txt", "v0\n");
    let ext = env.load().await;
    env.tool_call(&ext, "write", "a.txt").await;
    env.write("a.txt", "v1\n");
    env.agent_end(&ext).await;

    env.host.seed_entries(vec![user_entry("u2")]);
    env.tool_call(&ext, "write", "a.txt").await;
    env.write("a.txt", "v2\n");
    env.agent_end(&ext).await;
    assert_eq!(env.turns().len(), 2);

    let undone = env
        .service(&ext, "undoFrom", json!({"turnEntryId": "u1"}))
        .await;
    assert_eq!(strings(&undone["turns"]), ["u2", "u1"]);
    assert_eq!(strings(&undone["restored"]), ["a.txt", "a.txt"]);
    assert_eq!(env.read("a.txt"), "v0\n");
    assert_eq!(env.undos().len(), 2, "one undo entry per turn");
}

#[tokio::test(flavor = "multi_thread")]
async fn identical_content_shares_one_blob() {
    let env = Env::new();
    env.write("one.txt", "same\n");
    env.write("two.txt", "same\n");
    let ext = env.load().await;
    env.tool_call(&ext, "write", "one.txt").await;
    env.tool_call(&ext, "write", "two.txt").await;
    assert_eq!(env.blobs().len(), 1, "before content stored once");
    env.write("one.txt", "changed\n");
    env.write("two.txt", "changed\n");
    env.agent_end(&ext).await;
    assert_eq!(env.blobs().len(), 2, "after content stored once");

    let turn = &env.turns()[0];
    assert_eq!(
        file(turn, "one.txt")["beforeSha"],
        file(turn, "two.txt")["beforeSha"]
    );
    assert_eq!(
        file(turn, "one.txt")["afterSha"],
        file(turn, "two.txt")["afterSha"]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn footer_node_golden_at_40_and_100_columns() {
    let env = Env::new();
    env.write("a.txt", "one\ntwo\nthree\nfour\n");
    env.write(
        "src/components/timeline/TurnFilesFooter.tsx",
        "// a\n// b\n// c\n",
    );
    let ext = env.load().await;
    env.tool_call(&ext, "write", "a.txt").await;
    env.tool_call(&ext, "write", "src/components/timeline/TurnFilesFooter.tsx")
        .await;
    env.tool_call(&ext, "write", "b.txt").await;
    env.write("a.txt", "one\nTWO\nthree\nfour\nfive\n");
    env.write(
        "src/components/timeline/TurnFilesFooter.tsx",
        "// a\n// B\n// c\n",
    );
    env.write("b.txt", "x\n");

    // A live turn has no entry id yet: the footer renders the kv index.
    let live = env
        .render(
            &ext,
            json!({"target": "turnFooter", "key": "files", "turnIndex": 0, "running": true}),
        )
        .expect("live footer");
    assert_eq!(lines(&live, 100)[0], "3 files +04 −02");

    env.agent_end(&ext).await;
    let node = env
        .render(
            &ext,
            json!({
                "target": "turnFooter",
                "key": "files",
                "turnEntryId": "u1",
                "turnIndex": 0,
                "running": false,
            }),
        )
        .expect("turn footer");
    assert_eq!(
        lines(&node, 100),
        [
            "3 files +04 −02",
            "• a.txt  +2 −1  modified",
            "• src/components/timeline/TurnFilesFooter.tsx  +1 −1  modified",
            "• b.txt  +1 −0  new",
        ]
    );
    assert_eq!(
        lines(&node, 40),
        [
            "3 files +04 −02",
            "• a.txt  +2 −1  modified",
            "•",
            "src/components/timeline/TurnFilesFooter.",
            "tsx  +1 −1  modified",
            "• b.txt  +1 −0  new",
        ]
    );
    assert!(
        env.render(
            &ext,
            json!({"target": "turnFooter", "key": "files", "turnEntryId": "missing"}),
        )
        .is_none()
    );
    assert!(
        env.render(&ext, json!({"target": "turnFooter", "key": "other"}))
            .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn diff_of_a_deleted_file_shows_removal() {
    let env = Env::new();
    env.write("gone.txt", "a\nb\n");
    let ext = env.load().await;
    env.tool_call(&ext, "edit", "gone.txt").await;
    std::fs::remove_file(env.project_path("gone.txt")).unwrap();
    env.agent_end(&ext).await;
    let turn = &env.turns()[0];
    assert_eq!(file(turn, "gone.txt")["afterSha"], Value::Null);
    assert_eq!(file(turn, "gone.txt")["del"], 2);
    let diff = env
        .service(
            &ext,
            "diff",
            json!({"turnEntryId": "u1", "path": "gone.txt"}),
        )
        .await;
    let text = diff["diff"].as_str().unwrap();
    assert!(text.contains("-a\n-b\n"), "{text}");
}
