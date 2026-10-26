//! The unipi-ux components as real components in the wasm host: registration plus one
//! end-to-end behaviour per package.

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
    CallContext, CallKind, CancellationToken, ComponentSource, EventKind, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson, Registration,
    ToolInvocation,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

const CRATES: &[(&str, &str)] = &[
    ("ask-user", "unipi_ask_user"),
    ("command-enchantment", "unipi_command_enchantment"),
    ("input-shortcuts", "unipi_input_shortcuts"),
    ("footer", "unipi_footer"),
    ("info-screen", "unipi_info_screen"),
];

fn extensions_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .unwrap()
}

/// Builds all five components once (release, wasm32-wasip2).
fn built() -> &'static PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let ws =
            std::env::var("UNIPI_UX_WORKSPACE").map_or_else(|_| extensions_dir(), PathBuf::from);
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

    async fn load(&self, dir: &str, settings: Value) -> (Arc<dyn ExtensionInstance>, Registration) {
        let file = CRATES.iter().find(|(d, _)| *d == dir).unwrap().1;
        let wasm = built().join(format!("{file}.wasm"));
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join(format!("unipi/{dir}/extension.json")))
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
        // The host serves the same values on later reads (`registry.extension-settings`).
        for (ns, values) in settings.as_object().into_iter().flatten() {
            for (key, value) in values.as_object().into_iter().flatten() {
                self.host.preset_extension_setting(
                    oino_extension_api::ops::SettingsScope::Global,
                    ns,
                    key,
                    value.clone(),
                );
            }
        }
        let input = InitInput {
            settings: RawJson::from(settings),
            ..InitInput::default()
        };
        let reg = ext.init(input, self.cx(CallKind::Init)).await.unwrap();
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

    async fn shortcut(&self, ext: &Arc<dyn ExtensionInstance>, id: &str) {
        ext.run_shortcut(id, self.cx(CallKind::Shortcut))
            .await
            .unwrap();
    }

    fn fired(&self, method: &str) -> Vec<Value> {
        self.host
            .calls_named("ui_fire")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["method"] == method)
            .collect()
    }

    fn calls(&self, name: &str) -> Vec<Value> {
        self.host
            .calls_named(name)
            .into_iter()
            .map(|c| c.detail)
            .collect()
    }
}

fn find_file(root: &Path, name: &str) -> Option<PathBuf> {
    for e in std::fs::read_dir(root).ok()?.flatten() {
        let p = e.path();
        if p.is_dir() {
            if let Some(f) = find_file(&p, name) {
                return Some(f);
            }
        } else if p.file_name().is_some_and(|n| n == name) {
            return Some(p);
        }
    }
    None
}

#[tokio::test(flavor = "multi_thread")]
async fn command_enchantment_completes_unipi_commands() {
    let env = Env::new();
    let (ext, reg) = env.load("command-enchantment", json!({})).await;
    assert!(reg.tools.is_empty() && reg.commands.is_empty());
    let ns: Vec<Value> = reg.settings.iter().map(RawJson::to_value).collect();
    assert_eq!(ns[0]["namespace"], "command-enchantment");
    let out = ext
        .complete_input(
            RawJson::from(json!({"text": "/unipi:pl", "cursor": 9})),
            env.cx(CallKind::Event),
        )
        .await
        .unwrap()
        .to_value();
    let group = &out[0];
    assert_eq!(group["source"], "unipi.command-enchantment");
    assert_eq!(group["replaces"], "commands");
    assert_eq!(group["items"][0]["value"], "unipi:plan");
    assert_eq!(group["items"][0]["tag"], "[workflow]");
    assert!(
        ext.complete_input(
            RawJson::from(json!({"text": "hello", "cursor": 5})),
            env.cx(CallKind::Event)
        )
        .await
        .is_none()
    );
    // Off ⇒ pure pass-through.
    let env2 = Env::new();
    let (off, _) = env2
        .load(
            "command-enchantment",
            json!({"command-enchantment": {"autocompleteEnhanced": false}}),
        )
        .await;
    assert!(
        off.complete_input(
            RawJson::from(json!({"text": "/unipi:pl"})),
            env2.cx(CallKind::Event)
        )
        .await
        .is_none()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn input_shortcuts_stash_round_trip() {
    let env = Env::new();
    let (ext, reg) = env.load("input-shortcuts", json!({})).await;
    let shortcuts: Vec<(&str, &str)> = reg
        .shortcuts
        .iter()
        .map(|s| (s.id.as_str(), s.default_keys[0].as_str()))
        .collect();
    assert_eq!(
        shortcuts,
        [("unipi.chord", "alt+s"), ("unipi.tab-insert", "alt+i")]
    );
    assert_eq!(reg.services, ["info-screen.groups"]);
    assert_eq!(reg.renderers.len(), 1);

    env.shortcut(&ext, "unipi.tab-insert").await;
    let editor = env.calls("ui_editor");
    assert!(
        editor.iter().any(|e| e.as_str() == Some("Set(\"\\t\")")),
        "{editor:?}"
    );

    env.host
        .queue_dialog_answer(json!({"value": {"actionId": "stash"}}));
    env.shortcut(&ext, "unipi.chord").await;
    let dialog = &env.calls("ui_dialog")[0];
    assert_eq!(dialog["method"], "custom");
    assert_eq!(dialog["title"], "Input Shortcuts");
    let status = env.fired("setStatus");
    assert_eq!(status.last().unwrap()["statusText"], "✓ stash saved");
    let file = find_file(
        &env.agent.path().join("state/unipi.input-shortcuts"),
        "input-shortcuts.json",
    )
    .unwrap();
    let saved: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
    assert_eq!(saved, json!({"stash": "\t"}));

    env.host
        .queue_dialog_answer(json!({"value": {"actionId": "stash"}}));
    env.shortcut(&ext, "unipi.chord").await;
    assert_eq!(
        env.fired("setStatus").last().unwrap()["statusText"],
        "✓ stash restored"
    );

    // Ctrl+C-style clear keeps the text as a custom entry.
    env.event(
        &ext,
        EventKind::OinoEditor,
        json!({"type": "oino_editor", "intent": "changed", "text": "draft idea"}),
    )
    .await;
    env.event(
        &ext,
        EventKind::OinoEditor,
        json!({"type": "oino_editor", "intent": "cleared", "text": "", "reason": "interrupt"}),
    )
    .await;
    let kept = env
        .host
        .entries()
        .into_iter()
        .find(|e| e["customType"] == "unipi-cleared-input")
        .unwrap();
    assert_eq!(kept["data"]["text"], "draft idea");
}

#[tokio::test(flavor = "multi_thread")]
async fn ask_user_asks_and_answers() {
    let env = Env::new();
    let (ext, reg) = env
        .load("ask-user", json!({"ask-user": {"enabled": true}}))
        .await;
    let tool = &reg.tools[0];
    assert_eq!(tool.name, "ask_user");
    assert!(tool.prepare_arguments);
    assert_eq!(reg.commands[0].name, "unipi:ask-user-new-session");
    assert!(reg.commands[0].hidden);

    env.host
        .queue_dialog_answer(json!({"value": {"answers": [{"id": "q0", "selected": ["sqlite"]}]}}));
    let args = json!({"questions": [{"question": "Which DB?", "header": "DB", "options": [{"label": "Postgres"}, {"label": "SQLite", "value": "sqlite"}]}, {"question": "Name?", "header": "Name", "options": []}]});
    let inv = ToolInvocation {
        tool_call_id: "c1".into(),
        name: "ask_user".into(),
        args: RawJson::from(args.clone()),
        parent_tool_call_id: None,
    };
    let result = ext
        .run_tool(inv, CancellationToken::new(), env.cx(CallKind::Tool))
        .await
        .unwrap()
        .to_value();
    let text = result["content"][0]["text"].as_str().unwrap();
    assert!(text.starts_with("User answered your questions:\n{\n  \"Which DB?\": {\n    \"selected\": [\n      \"sqlite\"\n    ],\n    \"skipped\": false"), "{text}");
    assert!(text.ends_with("don't ask them again unless you must."));
    assert_eq!(result["details"]["outcome"], "answered");
    let form = &env.calls("ui_dialog")[0];
    assert_eq!(form["method"], "form");
    assert_eq!(form["title"], "DB · Name");
    // `unipi:ask-user:prompt` goes to the wasm host's in-process hub (not the recording host).

    // Esc ⇒ cancelled + terminate.
    env.host.queue_dialog_answer(json!({"cancelled": true}));
    let inv = ToolInvocation {
        tool_call_id: "c2".into(),
        name: "ask_user".into(),
        args: RawJson::from(args),
        parent_tool_call_id: None,
    };
    let result = ext
        .run_tool(inv, CancellationToken::new(), env.cx(CallKind::Tool))
        .await
        .unwrap()
        .to_value();
    assert_eq!(result["terminate"], true);
    assert_eq!(
        result["content"][0]["text"],
        "The user cancelled the questions and stopped the turn. Wait for their next message."
    );

    // Off by default ⇒ the tool refuses without a dialog.
    let env2 = Env::new();
    let (off, reg2) = env2.load("ask-user", json!({})).await;
    assert!(!reg2.tools[0].default_active);
    let inv = ToolInvocation {
        tool_call_id: "c3".into(),
        name: "ask_user".into(),
        args: RawJson::from(json!({"question": "x?"})),
        parent_tool_call_id: None,
    };
    let result = off
        .run_tool(inv, CancellationToken::new(), env2.cx(CallKind::Tool))
        .await
        .unwrap()
        .to_value();
    assert_eq!(
        result["content"][0]["text"],
        "ask_user is turned off in settings — ask in your reply instead."
    );
    assert_eq!(env2.calls("ui_dialog"), Vec::<Value>::new());
}

#[tokio::test(flavor = "multi_thread")]
async fn footer_paints_and_toggles() {
    let env = Env::new();
    let (ext, reg) = env.load("footer", json!({})).await;
    assert_eq!(reg.commands[0].name, "unipi:footer");
    assert!(
        reg.events.contains(&EventKind::MessageUpdate)
            && reg.events.contains(&EventKind::OinoTimer)
    );
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let frame = env.fired("setFrame");
    assert_eq!(frame[0]["frameKey"], "glance");
    assert_eq!(frame[0]["top"][0]["id"], "brand");
    assert_eq!(frame[0]["bottom"][1]["text"], "?%");
    env.command(&ext, "unipi:footer", "off").await;
    assert_eq!(
        env.fired("notify").last().unwrap()["message"],
        "Footer disabled"
    );
    assert!(
        env.fired("setFrame").last().unwrap().get("top").is_none(),
        "frame cleared"
    );
    env.command(&ext, "unipi:footer", "on").await;
    assert_eq!(
        env.fired("notify").last().unwrap()["message"],
        "Footer enabled"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn info_screen_splash_and_dashboard() {
    let env = Env::new();
    let (ext, reg) = env
        .load("info-screen", json!({"info-screen": {"bootMode": "on"}}))
        .await;
    assert_eq!(reg.commands[0].name, "unipi:info");
    assert!(reg.commands[0].completions);
    let items = ext
        .complete_command("unipi:info", "us", env.cx(CallKind::Command))
        .await;
    assert_eq!(items[0].value, "usage");
    env.event(
        &ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
    let header = env.fired("setHeader");
    assert_eq!(header[0]["key"], "splash");
    assert!(header[0]["spec"].to_string().contains("any key"));
    env.event(
        &ext,
        EventKind::Input,
        json!({"type": "input", "text": "hi", "source": "interactive"}),
    )
    .await;
    assert!(
        env.fired("setHeader").last().unwrap().get("spec").is_none(),
        "splash closed on input"
    );

    env.command(&ext, "unipi:info", "tools").await;
    let panels = env.calls("ui_panel");
    let open = &panels[0]["open"];
    assert_eq!(open["placement"], "overlay");
    assert_eq!(open["spec"]["t"], "tabs");
    assert_eq!(open["spec"]["active"], "tools");
    let ids: Vec<&str> = open["spec"]["tabs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["session", "usage", "tools", "skills", "extensions"]);
    env.event(
        &ext,
        EventKind::OinoUiAction,
        json!({"type": "oino_ui_action", "panelId": "panel-1", "actionId": "page:1"}),
    )
    .await;
    let update = env
        .calls("ui_panel")
        .into_iter()
        .rev()
        .find(|p| p.get("update").is_some())
        .unwrap();
    assert_eq!(update["spec"]["spec"]["active"], "session");
    env.event(
        &ext,
        EventKind::OinoUiAction,
        json!({"type": "oino_ui_action", "panelId": "panel-1", "actionId": "close"}),
    )
    .await;
    assert_eq!(
        env.calls("ui_panel").last().unwrap(),
        &json!({"close": "panel-1"})
    );
}
