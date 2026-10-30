//! X-SAFE acceptance: two recorded failed starts make the next `Services::create` run with
//! extensions off and providers on, the banner diagnostic names the recorded suspects, and a
//! successful start resets the state.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;

use indexmap::IndexMap;
use oino_ai::JsonObject;
use oino_core::extensions::api::{ExtensionInstance, ExtensionMode, Registration};
use oino_core::safe_mode::{FailureEntry, FailureStage, SafeMode};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::testing::{FauxComponent, FauxProvider};
use oino_models::ModelRegistry;
use oino_settings::SettingsManager;

fn agent_dir(dir: &std::path::Path) -> std::path::PathBuf {
    dir.join("agent")
}

async fn create_services(
    dir: &std::path::Path,
    extra: Vec<Arc<dyn ExtensionInstance>>,
    safe_mode: Option<oino_core::safe_mode::SafeModePlan>,
) -> Arc<Services> {
    let models = ModelRegistry::in_memory(Arc::new(oino_models::MapEnv::default()));
    models.set_runtime_api_key("faux", "k");
    let mut instances: Vec<Arc<dyn ExtensionInstance>> =
        vec![Arc::new(FauxComponent(Arc::new(FauxProvider::default())))];
    instances.extend(extra);
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: agent_dir(dir),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Print,
        has_ui: false,
        flags: ResourceFlags {
            no_context_files: true,
            safe_mode,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(instances)),
        approve: Some(false),
        models: Some(models),
        settings: Some(SettingsManager::in_memory(JsonObject::new())),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn two_failures_disable_extensions_keep_providers_and_name_suspects() {
    let dir = tempfile::tempdir().unwrap();

    // Two failed starts, the first naming a suspect extension.
    {
        let mut safe = SafeMode::load(&agent_dir(dir.path()));
        safe.note_suspect("unipi.notify", "init failed: boom");
        safe.record_failure(FailureEntry::new(FailureStage::Services, "Error: boom"));
        safe.record_failure(FailureEntry::new(FailureStage::Session, "Error: crash"));
    }
    let plan = SafeMode::load(&agent_dir(dir.path())).plan();
    assert!(plan.disable_extensions, "{plan:?}");

    // Next start: the extension is off, the provider is on, the banner names the suspect.
    let suspect = Arc::new(oino_core::testing::FakeExtension::new(
        "unipi.notify",
        Registration::default(),
    ));
    let services = create_services(
        dir.path(),
        vec![suspect as Arc<dyn ExtensionInstance>],
        Some(plan.clone()),
    )
    .await;
    let ids: Vec<&str> = services
        .extensions
        .extensions()
        .iter()
        .map(|loaded| loaded.instance.id())
        .collect();
    assert_eq!(ids, vec!["faux-provider"], "providers on, extensions off");
    assert!(
        services
            .models
            .registered_providers()
            .iter()
            .any(|provider| provider == "faux"),
        "the provider component still registered its models"
    );
    let banner = services
        .diagnostics
        .iter()
        .find(|line| line.starts_with("Safe mode:"))
        .expect("the banner diagnostic");
    assert!(
        banner.starts_with("Safe mode: extensions are off after 2 failed starts"),
        "{banner}"
    );
    assert!(
        banner.contains("unipi.notify (init failed: boom)"),
        "{banner}"
    );
    assert_eq!(
        services
            .safe_mode
            .as_ref()
            .map(|plan| plan.disable_extensions),
        Some(true),
        "the plan is carried for the `_oino/hello` `safeMode` flag"
    );

    // A successful start resets.
    let mut safe = SafeMode::load(&agent_dir(dir.path()));
    safe.record_success();
    drop(safe);
    let plan = SafeMode::load(&agent_dir(dir.path())).plan();
    assert!(!plan.disable_extensions);
    assert_eq!(plan.suspects.len(), 0);

    // Without the plan the extension loads again (`-ne`-style behavior, nothing latched).
    let suspect = Arc::new(oino_core::testing::FakeExtension::new(
        "unipi.notify",
        Registration::default(),
    ));
    let fresh = create_services(
        dir.path(),
        vec![suspect as Arc<dyn ExtensionInstance>],
        None,
    )
    .await;
    let ids: Vec<&str> = fresh
        .extensions
        .extensions()
        .iter()
        .map(|loaded| loaded.instance.id())
        .collect();
    assert!(ids.contains(&"unipi.notify"), "{ids:?}");
}
