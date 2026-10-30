//! WP-A24 acceptance (app spec §3.14): `host_cmd` — the hub-level `_oino/*`
//! subset against core services, **without a session runtime**:
//!
//! - `extensions` lists the loaded extensions; `set_extension_enabled` writes
//!   `disabledExtensions` and the next `extensions` round-trips it;
//! - `get_settings` reads the merged pi settings; `set_setting` writes one
//!   dot-path key (global scope) and the merged view reflects it;
//! - `get_settings_schema` answers `{namespaces}` (empty with no declaring
//!   extension); unknown namespaces and bad scopes are errors, not silent
//!   no-ops;
//! - `set_api_key` stores a credential `auth_providers` then reports;
//! - `login` is refused (the OAuth flow needs a session frontend).

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;

use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::ToolSelection;
use oino_core::services::{ResourceFlags, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;
use unipi_host::host_cmd;
use unipi_host::sessions::hub::{HostHubOptions, SessionHub};
use unipi_proto::settings::HostCmd;

fn hub(dir: &tempfile::TempDir) -> Arc<SessionHub> {
    let provider = FauxProvider::new(
        (0..4)
            .map(|_| oino_core::testing::FauxResponse::Text("ok".into()))
            .collect(),
    );
    let cwd = dir.path().to_path_buf();
    let agent_dir = dir.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    SessionHub::create(HostHubOptions {
        cwd,
        agent_dir,
        docs_root: None,
        session_dir: None,
        mode: ExtensionMode::Desktop,
        has_ui: false,
        approve: Some(false),
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        loader: Arc::new(StaticLoader(vec![Arc::new(FauxComponent(provider))
            as Arc<dyn oino_core::extensions::api::ExtensionInstance>])),
        models: {
            let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
            models.set_runtime_api_key("faux", "test-key");
            Some(models)
        },
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        host_data_dir: Some(dir.path().join("host")),
    })
    .unwrap()
}

async fn run(dir: &tempfile::TempDir, cmd: HostCmd) -> Result<serde_json::Value, String> {
    let hub = hub(dir);
    run_on(&hub, cmd).await
}

async fn run_on(hub: &Arc<SessionHub>, cmd: HostCmd) -> Result<serde_json::Value, String> {
    match host_cmd::run(hub, cmd).await {
        Ok(value) => Ok(value),
        Err(error) => Err(error.message.to_string()),
    }
}

#[tokio::test]
async fn extension_toggle_round_trips_without_a_session() {
    let dir = tempfile::tempdir().unwrap();
    // The faux provider extension is loaded and enabled.
    let listed = run(&dir, HostCmd::Extensions).await.unwrap();
    let list = listed["extensions"].as_array().unwrap();
    assert!(!list.is_empty(), "{list:?}");
    assert!(list
        .iter()
        .all(|row| row["enabled"] == serde_json::json!(true)));

    // Toggle off: the write lands in `disabledExtensions`…
    run(
        &dir,
        HostCmd::SetExtensionEnabled {
            id: "faux".into(),
            enabled: false,
        },
    )
    .await
    .unwrap();
    // …and a *fresh* hub (a restarted host, same agent dir) sees it disabled.
    let relisted = run(&dir, HostCmd::Extensions).await.unwrap();
    let faux = relisted["extensions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "faux")
        .unwrap();
    assert_eq!(faux["enabled"], serde_json::json!(false));
    assert_eq!(faux["loaded"], serde_json::json!(false));
}

#[tokio::test]
async fn pi_settings_read_and_write_through_the_hub() {
    let dir = tempfile::tempdir().unwrap();
    let merged = run(&dir, HostCmd::GetSettings { key: None }).await.unwrap();
    assert!(merged["settings"].is_object(), "{merged:?}");

    run(
        &dir,
        HostCmd::SetSetting {
            key: "ui.density".into(),
            value: Some(serde_json::json!("compact")),
            scope: "global".into(),
        },
    )
    .await
    .unwrap();
    let one = run(
        &dir,
        HostCmd::GetSettings {
            key: Some("ui.density".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(one["value"], serde_json::json!("compact"), "{one:?}");

    // A fresh hub reads the same persisted global setting (no session ever ran).
    let reread = run(
        &dir,
        HostCmd::GetSettings {
            key: Some("ui.density".into()),
        },
    )
    .await
    .unwrap();
    assert_eq!(reread["value"], serde_json::json!("compact"));
}

#[tokio::test]
async fn schema_and_errors_match_the_session_commands() {
    let dir = tempfile::tempdir().unwrap();
    let schema = run(&dir, HostCmd::GetSettingsSchema).await.unwrap();
    assert!(schema["namespaces"].is_array(), "{schema:?}");

    // Unknown namespace: the session command's text.
    let err = run(
        &dir,
        HostCmd::GetExtensionSettings {
            namespace: "nope".into(),
            layer: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(err, "Unknown settings namespace: nope");

    // Bad scope: a schema error, never a silent write.
    let err = run(
        &dir,
        HostCmd::SetExtensionSetting {
            namespace: "ns".into(),
            key: "k".into(),
            value: None,
            scope: "somewhere".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(err.contains("scope"), "{err}");

    // Empty key: refused.
    let err = run(
        &dir,
        HostCmd::SetSetting {
            key: String::new(),
            value: None,
            scope: "global".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(err.contains("empty"), "{err}");
}

#[tokio::test]
async fn api_key_and_providers_without_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(&dir);
    run_on(
        &hub,
        HostCmd::SetApiKey {
            provider: "faux".into(),
            key: "sk-hub-level".into(),
        },
    )
    .await
    .unwrap();
    let providers = run_on(&hub, HostCmd::AuthProviders).await.unwrap();
    let faux = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "faux")
        .unwrap_or_else(|| panic!("{providers:?}"));
    assert_eq!(faux["stored"], serde_json::json!("api_key"));
    assert_eq!(faux["configured"], serde_json::json!(true));

    // The OAuth flow needs a session frontend; the hub-level twin says so.
    let err = run_on(
        &hub,
        HostCmd::Login {
            provider: "faux".into(),
        },
    )
    .await
    .unwrap_err();
    assert!(err.contains("session"), "{err}");
}
