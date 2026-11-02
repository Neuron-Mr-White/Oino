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
        services_cap: None,
        host_data_dir: Some(dir.path().join("host")),
    })
    .unwrap()
}

/// A hub whose registry holds one openai-shaped provider with a static model, so the
/// readiness probe has a real endpoint to hit (G-05).
fn hub_with_probeable(dir: &tempfile::TempDir, id: &str, base: &str) -> Arc<SessionHub> {
    let provider = FauxProvider::new(
        (0..4)
            .map(|_| oino_core::testing::FauxResponse::Text("ok".into()))
            .collect(),
    );
    let cwd = dir.path().to_path_buf();
    let agent_dir = dir.path().join("agent");
    std::fs::create_dir_all(&agent_dir).unwrap();
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.register_api_implementor("openai-completions", "ext");
    let _ = models.register_provider(
        "ext",
        oino_models::ProviderRegistration {
            id: id.to_owned(),
            name: Some(id.to_owned()),
            api: "openai-completions".to_owned(),
            base_url: Some(base.to_owned()),
            auth_env: vec![format!("{}_API_KEY", id.to_uppercase())],
            auth_header: oino_models::AuthHeaderMode::None,
            oauth: false,
            headers: indexmap::IndexMap::new(),
            compat: None,
            models: oino_models::ProviderModels::Static(vec![oino_models::ProviderModelConfig {
                id: format!("{id}-m"),
                name: None,
                api: None,
                base_url: None,
                reasoning: false,
                thinking_level_map: None,
                input: vec![oino_ai::InputModality::Text],
                input_limits: None,
                cost: oino_ai::ModelCost::default(),
                prompt_cache: None,
                context_window: 128_000,
                max_tokens: 16_384,
                sampling_params: None,
                sampling_params_by_thinking_level: None,
                headers: None,
                compat: None,
            }]),
        },
    );
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
        models: Some(models),
        tools: ToolSelection::Default,
        exclude_tools: Vec::new(),
        services_cap: None,
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
    let providers = run_on(&hub, HostCmd::AuthProviders { validate: None })
        .await
        .unwrap();
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

/// G-05: a saved key is validated right away — the first `auth_providers` read after the
/// save already carries the probe's verdict (a dead key is `invalid_key`, never `ready`
/// by assumption), without a first send failing first.
#[tokio::test]
async fn saving_a_dead_key_reports_invalid_right_away() {
    // One canned 401 endpoint the probe can hit.
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let mut buf = [0u8; 4096];
        let _ = socket.read(&mut buf).await;
        let body = "[]";
        let reply = format!(
            "HTTP/1.1 401 Unauthorized\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
            body.len()
        );
        let _ = socket.write_all(reply.as_bytes()).await;
        let _ = socket.shutdown().await;
    });
    let dir = tempfile::tempdir().unwrap();
    let hub = hub_with_probeable(&dir, "probe", &format!("http://{addr}"));

    run_on(
        &hub,
        HostCmd::SetApiKey {
            provider: "probe".into(),
            key: "sk-dead".into(),
        },
    )
    .await
    .unwrap();
    let _ = server.await;

    // The next read (the UI's refetch after Save) sees the probe's answer.
    let providers = run_on(&hub, HostCmd::AuthProviders { validate: None })
        .await
        .unwrap();
    let row = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "probe")
        .unwrap_or_else(|| panic!("{providers:?}"));
    assert_eq!(row["state"], serde_json::json!("invalid_key"), "{row:?}");
    assert_eq!(row["ready"], serde_json::json!(false));
    assert!(
        row["detail"].as_str().unwrap_or_default().contains("401"),
        "{row:?}"
    );
}

/// v2 QA D-01/D-37: `default_model` reports the model a new session will start on and
/// `auth_providers` rows carry the hub's credential validation — `unknown` for an API the
/// host cannot probe (never a fake `ready`), and a cached answer rides every row.
#[tokio::test]
async fn default_model_and_provider_readiness_without_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let hub = hub(&dir);

    // No default configured: the only credentialed provider (faux — not probeable) is the
    // choice, reported as an unvalidated fallback with its provider state.
    let reply = run_on(&hub, HostCmd::DefaultModel).await.unwrap();
    assert_eq!(reply["source"], serde_json::json!("fallback"));
    assert_eq!(reply["model"]["provider"], serde_json::json!("faux"));
    assert_eq!(reply["model"]["id"], serde_json::json!("faux-1"));
    assert_eq!(reply["defaultConfigured"], serde_json::Value::Null);

    // Configure the faux default: the choice is the user's, source `settings`.
    let settings = hub.core_services().await.unwrap();
    settings
        .settings
        .lock()
        .unwrap()
        .set_default_model_and_provider("faux", "faux-1")
        .unwrap();
    let reply = run_on(&hub, HostCmd::DefaultModel).await.unwrap();
    assert_eq!(reply["source"], serde_json::json!("settings"));
    assert_eq!(reply["model"]["id"], serde_json::json!("faux-1"));
    assert_eq!(
        reply["defaultConfigured"]["provider"],
        serde_json::json!("faux")
    );

    // A stored validation rides the provider row: state, ready, reason, checkedAt.
    hub.readiness().store(
        "faux",
        unipi_host::models_readiness::ReadinessState::InvalidKey,
        Some("HTTP 401".into()),
    );
    let providers = run_on(&hub, HostCmd::AuthProviders { validate: None })
        .await
        .unwrap();
    let faux = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "faux")
        .unwrap_or_else(|| panic!("{providers:?}"));
    assert_eq!(faux["state"], serde_json::json!("invalid_key"));
    assert_eq!(faux["ready"], serde_json::json!(false));
    assert!(faux["reason"].as_str().unwrap().contains("rejected"));
    assert!(faux["checkedAt"].as_u64().is_some());

    // With the key rejected, `default_model` falls back (unverified) and says why.
    let reply = run_on(&hub, HostCmd::DefaultModel).await.unwrap();
    assert_eq!(reply["source"], serde_json::json!("fallback-unverified"));
    assert!(reply["reason"].as_str().unwrap().contains("was rejected"));

    // A fresh key clears the verdict.
    run_on(
        &hub,
        HostCmd::SetApiKey {
            provider: "faux".into(),
            key: "sk-2".into(),
        },
    )
    .await
    .unwrap();
    let providers = run_on(&hub, HostCmd::AuthProviders { validate: None })
        .await
        .unwrap();
    let faux = providers["providers"]
        .as_array()
        .unwrap()
        .iter()
        .find(|row| row["id"] == "faux")
        .unwrap();
    assert_eq!(faux["state"], serde_json::json!("unknown"));
    assert!(
        faux.get("ready").is_none(),
        "unvalidated rows carry no `ready`"
    );
}
