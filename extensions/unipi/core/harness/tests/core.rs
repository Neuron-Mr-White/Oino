//! `unipi.core` as a real component in the wasm host: registration, hints (startup, clear on
//! input, cycle, browser panel, reset, persisted store) and the work tray against a producer.

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

use oino_extension_api::manifest::Manifest;
use oino_extension_api::testing::RecordingHostServices;
use oino_extension_api::{
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionId,
    ExtensionInstance, ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn extensions_dir() -> PathBuf {
    harness_dir().join("../../..").canonicalize().unwrap()
}

fn cargo_build(dir: &Path, extra: &[&str]) {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let mut cmd = Command::new(cargo);
    cmd.current_dir(dir)
        .args(["build", "-j", "2", "--release", "--target", "wasm32-wasip2"])
        .args(extra);
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
}

/// Builds `unipi-core` and the tray probe once.
fn components() -> &'static (PathBuf, PathBuf) {
    static BUILT: OnceLock<(PathBuf, PathBuf)> = OnceLock::new();
    BUILT.get_or_init(|| {
        cargo_build(&extensions_dir(), &["-p", "unipi-core"]);
        cargo_build(&harness_dir().join("probe"), &[]);
        let out = extensions_dir().join("target/wasm32-wasip2/release");
        (
            out.join("unipi_core.wasm"),
            out.join("unipi_core_tray_probe.wasm"),
        )
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

    async fn load(&self, manifest: &Path, wasm: &Path) -> Arc<dyn ExtensionInstance> {
        let manifest = Manifest::parse(&std::fs::read_to_string(manifest).unwrap()).unwrap();
        let package = LoadedPackage {
            manifest,
            component: ComponentSource::Path(wasm.to_path_buf()),
            root: None,
            bundled: true,
        };
        let host: Arc<dyn HostServices> = Arc::clone(&self.host) as Arc<dyn HostServices>;
        let ext = self.runtime.load(package, host).await.unwrap();
        ext.init(InitInput::default(), self.cx(CallKind::Init))
            .await
            .unwrap();
        ext
    }

    async fn core(&self) -> Arc<dyn ExtensionInstance> {
        let (wasm, _) = components();
        self.load(&extensions_dir().join("unipi/core/extension.json"), wasm)
            .await
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

    /// `ui_fire` requests of `method` (and `widgetKey`, when given), in order.
    fn fired(&self, method: &str, widget: Option<&str>) -> Vec<Value> {
        self.host
            .calls_named("ui_fire")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["method"] == method && widget.is_none_or(|w| d["widgetKey"] == w))
            .collect()
    }

    fn panels(&self) -> Vec<Value> {
        self.host
            .calls_named("ui_panel")
            .into_iter()
            .map(|c| c.detail)
            .collect()
    }

    fn hint_store(&self) -> Value {
        let path = self.agent.path().join("state/unipi.core/global/hints.json");
        serde_json::from_slice(&std::fs::read(&path).unwrap_or_default()).unwrap_or(Value::Null)
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn registration() {
    let env = Env::new();
    let (wasm, _) = components();
    let manifest = Manifest::parse(
        &std::fs::read_to_string(extensions_dir().join("unipi/core/extension.json")).unwrap(),
    )
    .unwrap();
    let package = LoadedPackage {
        manifest,
        component: ComponentSource::Path(wasm.clone()),
        root: None,
        bundled: true,
    };
    let host: Arc<dyn HostServices> = Arc::clone(&env.host) as Arc<dyn HostServices>;
    let ext = env.runtime.load(package, host).await.unwrap();
    let reg = ext
        .init(InitInput::default(), env.cx(CallKind::Init))
        .await
        .unwrap();

    let commands: Vec<(&str, bool)> = reg
        .commands
        .iter()
        .map(|c| (c.name.as_str(), c.hidden))
        .collect();
    assert_eq!(
        commands,
        [("unipi:hint", false), ("unipi:hint-reset", true)]
    );
    assert!(reg.commands[0].completions);
    let shortcuts: Vec<(&str, &[String])> = reg
        .shortcuts
        .iter()
        .map(|s| (s.id.as_str(), s.default_keys.as_slice()))
        .collect();
    assert_eq!(
        shortcuts,
        [
            ("unipi.hint.next", &["alt+h".to_owned()][..]),
            ("unipi.hint.prev", &["alt+shift+h".to_owned()][..]),
            ("unipi.tray.open", &["shift+down".to_owned()][..]),
        ]
    );
    assert_eq!(reg.services, ["core.open-tray"]);
    let namespaces: Vec<String> = reg
        .settings
        .iter()
        .map(|n| n.to_value()["namespace"].as_str().unwrap().to_owned())
        .collect();
    assert_eq!(namespaces, ["hints", "decision-model"]);
    for kind in [
        EventKind::SessionStart,
        EventKind::Input,
        EventKind::ToolCall,
        EventKind::TurnEnd,
        EventKind::OinoBus,
        EventKind::OinoEditor,
        EventKind::OinoUiAction,
        EventKind::OinoSettingsChanged,
    ] {
        assert!(reg.events.contains(&kind), "{kind:?}");
    }
    assert_eq!(reg.tools.len(), 0);
    let items = ext
        .complete_command("unipi:hint", "n", env.cx(CallKind::Command))
        .await;
    assert_eq!(
        items.iter().map(|i| i.value.as_str()).collect::<Vec<_>>(),
        ["next"]
    );
    let _ = ExtensionId::new("unipi.core").unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn hints_lifecycle() {
    let env = Env::new();
    let core = env.core().await;
    env.event(
        &core,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;

    let header = env.fired("setHeader", None);
    assert_eq!(header.len(), 1);
    assert_eq!(header[0]["spec"]["t"], "responsive");
    assert!(header[0]["spec"].to_string().contains("Hi, I'm Unicrab."));
    let shown = env.fired("setWidget", Some("unipi-hints"));
    assert_eq!(shown.len(), 1);
    assert_eq!(shown[0]["widgetPlacement"], "aboveEditor");
    let line = shown[0]["widgetLines"][0].as_str().unwrap();
    assert!(
        line.contains("/unipi:settings") && line.ends_with("command · alt+h ›"),
        "{line}"
    );
    let store = env.hint_store();
    assert_eq!(store["counts"]["cmd.settings"]["count"], 1);
    assert_eq!(store["lastSeenVersion"], "3.0.0");
    let strip = env.fired("setWidget", Some("work-tray-strip"));
    assert_eq!(
        strip.last().map(|s| s.get("widgetLines").is_none()),
        Some(true),
        "no producers → no tray line"
    );

    env.event(
        &core,
        EventKind::Input,
        json!({"type": "input", "text": "/unipi:settings", "source": "interactive"}),
    )
    .await;
    let cleared = env.fired("setWidget", Some("unipi-hints"));
    assert_eq!(cleared.len(), 2);
    assert!(
        cleared[1].get("widgetLines").is_none(),
        "input clears the hint"
    );
    // Every hint teaching `/unipi:settings` is learned (three in the catalog).
    let learned = env.hint_store()["learned"].clone();
    assert!(
        learned.as_array().unwrap().contains(&json!("cmd.settings")),
        "{learned}"
    );

    env.command(&core, "unipi:hint", "next").await;
    let next = env.fired("setWidget", Some("unipi-hints"));
    assert_eq!(next.len(), 3);
    assert!(
        !next[2]["widgetLines"][0]
            .as_str()
            .unwrap()
            .contains("/unipi:settings")
    );

    env.command(&core, "unipi:hint", "").await;
    let panels = env.panels();
    assert_eq!(panels.len(), 1);
    let open = &panels[0]["open"];
    assert_eq!(open["panelId"], "unipi.core/hints");
    assert_eq!(open["placement"], "overlay");
    env.event(
        &core,
        EventKind::OinoUiAction,
        json!({"type": "oino_ui_action", "panelId": "panel-1", "actionId": "filter", "value": "kanboard"}),
    )
    .await;
    let update = &env.panels()[1];
    assert_eq!(update["update"], "panel-1");
    assert!(
        update["spec"].to_string().contains("kanboard")
            && !update["spec"].to_string().contains("/unipi:btw")
    );
    env.event(
        &core,
        EventKind::OinoUiAction,
        json!({"type": "oino_ui_action", "panelId": "panel-1", "actionId": "show", "value": "cmd.hint"}),
    )
    .await;
    assert_eq!(env.panels()[2], json!({"close": "panel-1"}));
    let picked = env.fired("setWidget", Some("unipi-hints"));
    assert!(
        picked.last().unwrap()["widgetLines"][0]
            .as_str()
            .unwrap()
            .contains("/unipi:hint opens the full hint browser")
    );

    env.command(&core, "unipi:hint-reset", "").await;
    assert_eq!(
        env.fired("notify", None).last().unwrap()["message"],
        "Hint history cleared — every hint is eligible again."
    );
    let store = env.hint_store();
    assert_eq!(store["counts"], json!({}));
    assert_eq!(store["learned"], json!([]));

    // An event hint from a bus topic (unipi:memory:stored) in a fresh turn.
    env.event(
        &core,
        EventKind::Input,
        json!({"type": "input", "text": "go", "source": "interactive"}),
    )
    .await;
    let before = env.fired("setWidget", Some("unipi-hints")).len();
    env.event(
        &core,
        EventKind::OinoBus,
        json!({"type": "oino_bus", "topic": "unipi:memory:stored", "data": {"id": "m", "title": "t", "type": "decision", "project": "p", "action": "created"}, "sticky": false, "publisher": "unipi.memory"}),
    )
    .await;
    let after = env.fired("setWidget", Some("unipi-hints"));
    assert_eq!(after.len(), before + 1, "memory hint shown");
    assert!(
        after.last().unwrap()["widgetLines"][0]
            .as_str()
            .unwrap()
            .contains("Memory stored!")
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn work_tray_with_a_producer() {
    let env = Env::new();
    let (_, probe_wasm) = components();
    let probe = env
        .load(&harness_dir().join("probe/extension.json"), probe_wasm)
        .await;
    let core = env.core().await;
    env.event(
        &core,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;

    let strip = env.fired("setWidget", Some("work-tray-strip"));
    assert_eq!(
        strip.last().unwrap()["widgetLines"],
        json!([" Background tasks 2 (1 running) · ↓ open"])
    );
    assert_eq!(strip.last().unwrap()["widgetPlacement"], "belowEditor");

    let other = env
        .event(
            &core,
            EventKind::OinoEditor,
            json!({"type": "oino_editor", "intent": "changed", "text": "x"}),
        )
        .await;
    assert_eq!(other, None);
    let consumed = env
        .event(
            &core,
            EventKind::OinoEditor,
            json!({"type": "oino_editor", "intent": "emptyArrowDown", "text": ""}),
        )
        .await;
    assert_eq!(consumed, Some(json!({"consume": true})));
    let panels = env.panels();
    let open = &panels.last().unwrap()["open"];
    assert_eq!(open["panelId"], "unipi.core/tray");
    assert_eq!(open["placement"], "editorReplace");
    let tabs = &open["spec"]["children"][0];
    assert_eq!(tabs["active"], "bg");
    assert_eq!(tabs["tabs"][0]["title"], "Background tasks (2 · 1 running)");
    assert_eq!(tabs["tabs"][0]["body"]["selectable"], "bg:open");
    assert!(
        env.fired("setWidget", Some("work-tray-strip"))
            .last()
            .unwrap()
            .get("widgetLines")
            .is_none(),
        "line hidden while open"
    );

    let again = env
        .event(
            &core,
            EventKind::OinoEditor,
            json!({"type": "oino_editor", "intent": "emptyArrowDown", "text": ""}),
        )
        .await;
    assert_eq!(again, None, "already open: ↓ is not stolen");

    env.event(
        &core,
        EventKind::OinoUiAction,
        json!({"type": "oino_ui_action", "panelId": "panel-1", "actionId": "bg:open", "value": "t1"}),
    )
    .await;
    let entry = env
        .host
        .entries()
        .into_iter()
        .find(|e| e["customType"] == "probe.tray-action")
        .unwrap();
    assert_eq!(
        entry["data"],
        json!({"tab": "bg", "actionId": "open", "value": "t1"})
    );
    assert_eq!(
        env.panels().last().unwrap(),
        &json!({"close": "panel-1"}),
        "owner asked to close"
    );
    assert_eq!(
        env.fired("setWidget", Some("work-tray-strip"))
            .last()
            .unwrap()["widgetLines"],
        json!([" Background tasks 2 (1 running) · ↓ open"])
    );

    let reply = core
        .handle_service(
            "core.open-tray",
            RawJson::from(json!({"tab": "bg"})),
            probe.id(),
            env.cx(CallKind::Service),
        )
        .await
        .unwrap()
        .to_value();
    assert_eq!(reply, json!({"opened": true}));
    assert_eq!(
        env.panels().last().unwrap()["open"]["panelId"],
        "unipi.core/tray"
    );
    core.run_shortcut("unipi.tray.open", env.cx(CallKind::Shortcut))
        .await
        .unwrap();
    assert!(
        env.panels().last().unwrap().get("update").is_some(),
        "shortcut while open updates in place"
    );
}
