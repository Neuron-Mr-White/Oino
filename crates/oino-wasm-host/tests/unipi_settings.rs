//! End-to-end: the real unipi components (`extensions/dist`) on the extension settings write
//! path — one-time import of their pre-write-path state files into `~/.oino/settings.json`,
//! command writes into `.oino/settings.json`, the schema for frontends and the shared
//! `decision-model` secret.
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

use base64::Engine as _;
use indexmap::IndexMap;
use oino_ai::JsonValue;
use oino_core::extensions::api::{CallKind, ExtensionMode};
use oino_core::extensions::host::{HostApi, HostOp};
use oino_core::extensions::settings::{SECRET_MASK, SettingsLayer};
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions};
use oino_settings::{SecretStore, SettingsManager, SettingsScope};
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};
use serde_json::json;

const IDS: [&str; 5] = [
    "unipi.core",
    "unipi.workflow",
    "unipi.skill-registry",
    "unipi.notify",
    "unipi.footer",
];

fn dist() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../extensions/dist")
}

fn read_json(path: &Path) -> JsonValue {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap_or_else(|_| "{}".into())).unwrap()
}

fn write(path: &Path, value: &JsonValue) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, value.to_string()).unwrap();
}

/// A global `kv.json` as the wasm host stores it (base64 values).
fn kv(entries: &[(&str, JsonValue)]) -> JsonValue {
    let b64 = base64::engine::general_purpose::STANDARD;
    JsonValue::Object(
        entries
            .iter()
            .map(|(k, v)| ((*k).to_owned(), json!(b64.encode(v.to_string()))))
            .collect(),
    )
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unipi_state_files_move_into_settings() {
    if !IDS
        .iter()
        .all(|id| dist().join(id).join("component.wasm").is_file())
    {
        return;
    }
    let dir = tempfile::Builder::new()
        .prefix("oino-unipi-settings-")
        .tempdir_in(env!("CARGO_TARGET_TMPDIR"))
        .unwrap();
    let agent = dir.path().join("agent");
    let cwd = dir.path().join("work");
    std::fs::create_dir_all(agent.join("cache")).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let cache = Path::new(env!("CARGO_TARGET_TMPDIR")).join("oino-wasmtime-cache");
    std::fs::create_dir_all(&cache).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&cache, agent.join("cache/wasmtime")).unwrap();

    let state = |id: &str| agent.join("state").join(id).join("global");
    let permission = state("unipi.workflow").join("permission.json");
    write(
        &permission,
        &json!({"rules": [{"tool": "bash", "pattern": "make deploy*", "decision": "deny", "scope": "global"}]}),
    );
    let skill_states = state("unipi.skill-registry").join("states.json");
    write(&skill_states, &json!({"alpha": {"mustShow": true}}));
    write(
        &state("unipi.notify").join("kv.json"),
        &kv(&[(
            "settings-overlay",
            json!({"ntfy": {"enabled": true, "topic": "builds"}}),
        )]),
    );
    write(
        &state("unipi.footer").join("kv.json"),
        &kv(&[("enabledOverride", json!(false))]),
    );

    let wasm =
        Arc::new(WasmRuntime::new(WasmRuntimeOptions::new(HostConfig::new(&agent, &cwd))).unwrap());
    let packages = IDS
        .iter()
        .map(|id| SourcedPackage {
            package: oino_wasm_host::packages::load_dir(&dist().join(id), true).unwrap(),
            project: false,
        })
        .collect();
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
        loader: Arc::new(CoreLoader::new(wasm, packages)),
        approve: Some(true),
        models: Some(oino_models::ModelRegistry::in_memory(Arc::new(
            oino_models::MapEnv::default(),
        ))),
        settings: Some(SettingsManager::create(&cwd, &agent, true)),
    })
    .await
    .unwrap();
    assert!(
        services
            .diagnostics
            .iter()
            .all(|d| !d.contains("init failed") && !d.contains("settings:")),
        "{:?}",
        services.diagnostics
    );
    let runtime = AgentSessionRuntime::create(
        Arc::clone(&services),
        SessionOptions {
            source: SessionSource::New,
            ..SessionOptions::default()
        },
    )
    .await
    .unwrap();

    // Frontend schema: every registered namespace with its owner.
    let registry = runtime.extension_settings();
    let owners: Vec<(String, String)> = registry
        .schema()
        .iter()
        .map(|ns| {
            (
                ns["namespace"].as_str().unwrap().to_owned(),
                ns["extensionId"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    for (ns, owner) in [
        ("decision-model", "unipi.core"),
        ("permission", "unipi.workflow"),
        ("skills", "unipi.skill-registry"),
        ("notify", "unipi.notify"),
        ("footer", "unipi.footer"),
    ] {
        assert!(
            owners.contains(&(ns.to_owned(), owner.to_owned())),
            "{ns} in {owners:?}"
        );
    }

    // `session_start` imported the state files into the global settings file.
    let global_file = agent.join("settings.json");
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    loop {
        let global = read_json(&global_file);
        let es = &global["extensionSettings"];
        if es["permission"]["rules"].is_array()
            && es["skills"]["states"].is_object()
            && es["notify"]["ntfy"]["topic"].is_string()
            && es["footer"]["enabled"].is_boolean()
        {
            assert_eq!(es["permission"]["rules"][0]["pattern"], "make deploy*");
            assert_eq!(es["skills"]["states"]["alpha"]["mustShow"], true);
            assert_eq!(
                es["notify"]["ntfy"],
                json!({"enabled": true, "topic": "builds"})
            );
            assert_eq!(es["footer"]["enabled"], false);
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "global: {global}");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(!permission.exists());
    assert!(!skill_states.exists());

    // A command writes the project file.
    let session = runtime.session();
    let cx = session.runner().call_context().with_kind(CallKind::Command);
    services
        .host
        .call(
            "unipi.workflow",
            &cx,
            HostOp::InvokeCommand {
                name: "permission".into(),
                args: "ask".into(),
            },
        )
        .await
        .unwrap();
    let project = read_json(&cwd.join(".oino/settings.json"));
    assert_eq!(project["extensionSettings"]["permission"]["mode"], "ask");
    assert_eq!(
        registry.values("permission", SettingsLayer::Effective)["mode"],
        "ask"
    );

    // Shared secret: a hub write of the decision-model key lands in secrets.json and is
    // readable by other unipi components, masked for frontends.
    registry
        .set(
            "decision-model",
            "apiKey",
            Some(json!("jev-key")),
            SettingsScope::Global,
        )
        .unwrap();
    assert!(
        !std::fs::read_to_string(&global_file)
            .unwrap()
            .contains("jev-key")
    );
    assert_eq!(
        SecretStore::new(&agent)
            .get("unipi.core/decision-model/apiKey")
            .unwrap()
            .as_deref(),
        Some("jev-key")
    );
    assert_eq!(
        registry.values("decision-model", SettingsLayer::Effective)["apiKey"],
        SECRET_MASK
    );
    assert_eq!(
        registry.values_for("unipi.workflow", "decision-model", SettingsLayer::Effective)["apiKey"],
        "jev-key"
    );
}
