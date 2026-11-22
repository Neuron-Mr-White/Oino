//! Live services trust changes preserve existing session objects.
#![allow(clippy::unwrap_used, reason = "test assertions")]
use oino_core::extensions::api::ExtensionMode;
use oino_core::runtime::{AgentSessionRuntime, SessionOptions, SessionSource};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_resources::trust::TrustStore;
use serde_json::json;
use std::sync::Arc;

#[tokio::test]
async fn live_session_settings_follow_saved_ancestor_and_child_denial() {
    let temp = tempfile::tempdir().unwrap();
    let project = temp.path().join("parent/project");
    let agent = temp.path().join("agent");
    std::fs::create_dir_all(project.join(".oino")).unwrap();
    std::fs::create_dir_all(&agent).unwrap();
    std::fs::write(project.join(".oino/mcp.json"), "{}").unwrap();
    std::fs::write(
        project.join(".oino/settings.json"),
        r#"{"extensionSettings":{"fixture":{"marker":"project"}}}"#,
    )
    .unwrap();
    let services = Services::create(ServicesOptions {
        cwd: project.clone(),
        agent_dir: agent.clone(),
        docs_root: std::path::PathBuf::default(),
        mode: ExtensionMode::Rpc,
        has_ui: false,
        flags: ResourceFlags {
            offline: true,
            ..Default::default()
        },
        extension_flag_values: indexmap::IndexMap::default(),
        loader: Arc::new(StaticLoader(vec![])),
        approve: None,
        models: None,
        settings: None,
    })
    .await
    .unwrap();
    let runtime = AgentSessionRuntime::create(
        services.clone(),
        SessionOptions {
            source: SessionSource::InMemory,
            ..Default::default()
        },
    )
    .await
    .unwrap();
    let session = runtime.session();
    assert!(!session.with_settings(|s| s.is_project_trusted()));
    TrustStore::new(&agent)
        .set(project.parent().unwrap(), Some(true))
        .unwrap();
    services.reload_project().await.unwrap();
    assert!(session.with_settings(|s| s.is_project_trusted()));
    assert_eq!(
        session.with_settings(|s| serde_json::Value::Object(s.settings().merged().clone()))["extensionSettings"]
            ["fixture"]["marker"],
        json!("project")
    );
    TrustStore::new(&agent).set(&project, Some(false)).unwrap();
    services.reload_project().await.unwrap();
    assert!(!session.with_settings(|s| s.is_project_trusted()));
    assert!(session.with_settings(|s| serde_json::Value::Object(s.settings().merged().clone()))["extensionSettings"]["fixture"].is_null());
    assert!(Arc::ptr_eq(runtime.services(), &services));
}
