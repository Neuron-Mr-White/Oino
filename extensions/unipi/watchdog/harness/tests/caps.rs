//! `unipi.watchdog` as a real component in the wasm host — the WP-A17 session-caps
//! acceptance: caps set through the `caps.set` service stop the run at the limit, and
//! the pins persist as a `watchdog.session-caps` custom entry across a restart.

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
    CallContext, CallKind, ComponentSource, EventKind, ExtensionId, ExtensionInstance,
    ExtensionRuntime, HostServices, InitInput, LoadedPackage, Mode, RawJson,
};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::{Value, json};

fn harness_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn extensions_dir() -> PathBuf {
    harness_dir().join("../../..").canonicalize().unwrap()
}

/// Builds the component once (in `EXT_WORKSPACE` or this checkout's `extensions/`).
fn built() -> &'static PathBuf {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let ws = std::env::var_os("EXT_WORKSPACE").map_or_else(extensions_dir, PathBuf::from);
        let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
        let mut cmd = Command::new(cargo);
        cmd.current_dir(&ws).args([
            "build",
            "-j",
            "2",
            "--release",
            "--target",
            "wasm32-wasip2",
            "-p",
            "unipi-watchdog",
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
        extensions_dir().join("target/wasm32-wasip2/release/unipi_watchdog.wasm")
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
        let base = extensions_dir().join("target/watchdog-harness");
        std::fs::create_dir_all(&base).unwrap();
        let project = tempfile::Builder::new()
            .prefix("wdproj")
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

    /// SIGKILL of the host process for the guest (core.md §2.1.8): the runtime and its
    /// guest instances die; the recorded session entries survive, and a fresh runtime is
    /// rebuilt over them.
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

    async fn load(&self) -> (Arc<dyn ExtensionInstance>, oino_extension_api::Registration) {
        let manifest = Manifest::parse(
            &std::fs::read_to_string(extensions_dir().join("unipi/watchdog/extension.json"))
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

    async fn event(&self, ext: &Arc<dyn ExtensionInstance>, kind: EventKind, payload: Value) {
        ext.on_event(kind, RawJson::from(payload), self.cx(CallKind::Event))
            .await
            .unwrap();
    }

    /// Calls a service as the app frontend does (`caller = "oino.frontend"`).
    async fn service(&self, ext: &Arc<dyn ExtensionInstance>, name: &str, payload: Value) -> Value {
        let caller = ExtensionId::new("oino.frontend").unwrap();
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

    fn control_ops(&self) -> Vec<String> {
        self.host
            .calls_named("control")
            .into_iter()
            .map(|c| c.detail.as_str().unwrap_or_default().to_owned())
            .collect()
    }

    fn appended(&self, custom_type: &str) -> Vec<Value> {
        self.host
            .calls_named("session_write")
            .into_iter()
            .map(|c| c.detail)
            .filter(|d| d["appendEntry"] == custom_type)
            .map(|d| d["data"].clone())
            .collect()
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
}

async fn start(ext: &Arc<dyn ExtensionInstance>, env: &Env) {
    env.event(
        ext,
        EventKind::SessionStart,
        json!({"type": "session_start", "reason": "startup"}),
    )
    .await;
}

fn turn_end(cost: f64) -> Value {
    json!({
        "type": "turn_end",
        "message": {"usage": {"cost": {"total": cost}}},
        "toolResults": [],
    })
}

#[tokio::test(flavor = "multi_thread")]
async fn registers_the_caps_services() {
    let env = Env::new();
    let (_ext, reg) = env.load().await;
    for svc in ["caps.get", "caps.set"] {
        assert!(reg.services.iter().any(|s| s == svc), "{svc}");
    }
    for flag in ["max-turns", "max-cost", "max-wall", "max-tool-errors"] {
        assert!(reg.flags.iter().any(|f| f.name == flag), "flag {flag} gone");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn caps_set_via_service_stop_the_run_at_the_limit() {
    let env = Env::new();
    let (ext, _) = env.load().await;
    start(&ext, &env).await;

    // Before: everything off.
    assert_eq!(
        env.service(&ext, "caps.get", json!({})).await,
        json!({"caps": {"maxTurns": null, "maxCostUsd": null, "maxWallMinutes": null,
                         "maxConsecutiveToolErrors": null, "summaryTurn": true},
                "pinned": false})
    );

    // The app's Limits section pins two turns for this session.
    let set = env
        .service(
            &ext,
            "caps.set",
            json!({"maxTurns": 2, "summaryTurn": false}),
        )
        .await;
    assert_eq!(set["caps"]["maxTurns"], json!(2));
    assert_eq!(set["pinned"], json!(true));
    let pins = env.appended("watchdog.session-caps");
    assert_eq!(
        pins.as_slice(),
        [
            json!({"maxTurns": 2, "maxCostUsd": null, "maxWallMinutes": null,
                 "maxConsecutiveToolErrors": null, "summaryTurn": false})
        ],
        "the pins persist as a custom entry"
    );

    // A run crosses the limit on its second turn: abort + notice + cap entry.
    env.event(&ext, EventKind::AgentStart, json!({"type": "agent_start"}))
        .await;
    env.event(&ext, EventKind::TurnEnd, turn_end(0.1)).await;
    assert!(
        !env.control_ops().iter().any(|op| op == "abort"),
        "turn 1 of 2 continues"
    );
    env.event(&ext, EventKind::TurnEnd, turn_end(0.1)).await;
    assert!(
        env.control_ops().iter().any(|op| op == "abort"),
        "the run stops at the pinned limit"
    );
    assert!(
        env.notifies()
            .iter()
            .any(|n| n.contains("session cap reached") && n.contains("2 turns")),
        "{:?}",
        env.notifies()
    );
    assert_eq!(env.appended("watchdog.session-cap").len(), 1);
}

#[tokio::test(flavor = "multi_thread")]
async fn pinned_caps_survive_a_host_restart() {
    let mut env = Env::new();
    let (ext, _) = env.load().await;
    start(&ext, &env).await;
    env.service(&ext, "caps.set", json!({"maxCostUsd": 0.5}))
        .await;

    // The host dies; the session file (the recorded entries) survives.
    env.restart();
    let (ext, _) = env.load().await;
    start(&ext, &env).await;
    let got = env.service(&ext, "caps.get", json!({})).await;
    assert_eq!(got["caps"]["maxCostUsd"], json!(0.5));
    assert_eq!(got["pinned"], json!(true));
}
