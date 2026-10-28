//! Extension settings registry through `Services`: namespaces from `init` registrations,
//! guest reads and writes over `HostApi`, frontend values with masked secrets, ownership and
//! `oino_settings_changed` delivery.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::too_many_lines,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexMap;
use oino_ai::JsonObject;
use oino_core::extensions::api::{
    CallContext, CallKind, ExtensionInstance, ExtensionMode, Registration,
};
use oino_core::extensions::host::{HostApi, HostError, HostOp};
use oino_core::extensions::settings::{SECRET_MASK, SettingsLayer};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_core::testing::FakeExtension;
use oino_models::ModelRegistry;
use oino_settings::{SettingsManager, SettingsScope};
use serde_json::{Value, json};

fn schema() -> Value {
    json!({
        "namespace": "acme", "title": "Acme",
        "defaults": {"level": "low"},
        "sections": [{"id": "main", "title": "Main", "fields": [
            {"key": "level", "type": "enum", "label": "Level", "options": [{"value": "low", "label": "Low"}, {"value": "high", "label": "High"}]},
            {"key": "token", "type": "secret", "label": "Token", "scope": "global"},
            {"key": "ui.compact", "type": "boolean", "label": "Compact", "default": false}
        ]}]
    })
}

async fn services(dir: &std::path::Path, owner: Arc<FakeExtension>) -> Arc<Services> {
    let other = Arc::new(FakeExtension::new("acme.other", Registration::default()));
    let instances: Vec<Arc<dyn ExtensionInstance>> = vec![owner, other];
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: dir.join("agent"),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Print,
        has_ui: false,
        flags: ResourceFlags::default(),
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(instances)),
        approve: Some(false),
        models: Some(ModelRegistry::in_memory(Arc::new(
            oino_models::MapEnv::default(),
        ))),
        settings: Some(SettingsManager::in_memory(JsonObject::new())),
    })
    .await
    .unwrap()
}

fn cx() -> CallContext {
    CallContext {
        session_id: String::new(),
        mode: ExtensionMode::Print,
        has_ui: false,
        cwd: std::path::PathBuf::new(),
        is_child: false,
        child_role: None,
        call_kind: CallKind::Event,
    }
}

async fn call(services: &Services, ext: &str, op: HostOp) -> Result<Value, HostError> {
    services.host.call(ext, &cx(), op).await
}

fn set(namespace: &str, key: &str, value: Option<Value>) -> HostOp {
    HostOp::SetExtensionSetting {
        namespace: namespace.into(),
        key: key.into(),
        value,
        scope: SettingsScope::Global,
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn registry_serves_guests_and_frontends() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Arc::new(FakeExtension::new(
        "acme.ext",
        Registration {
            events: vec!["oino_settings_changed".into()],
            settings: vec![schema()],
            ..Registration::default()
        },
    ));
    let services = services(dir.path(), Arc::clone(&owner)).await;
    let registry = &services.extension_settings;
    assert_eq!(registry.schema().len(), 1);
    assert_eq!(registry.owner("acme").as_deref(), Some("acme.ext"));
    assert_eq!(
        registry.values("acme", SettingsLayer::Effective),
        json!({"level": "low", "ui": {"compact": false}})
    );

    // Guest writes through HostApi (sessionless, like during init or from timers).
    call(
        &services,
        "acme.ext",
        set("acme", "level", Some(json!("high"))),
    )
    .await
    .unwrap();
    call(
        &services,
        "acme.ext",
        set("acme", "token", Some(json!("s3cret"))),
    )
    .await
    .unwrap();
    let own = call(
        &services,
        "acme.ext",
        HostOp::GetExtensionSettings {
            namespace: "acme".into(),
            layer: SettingsLayer::Effective,
        },
    )
    .await
    .unwrap();
    assert_eq!(own["level"], "high");
    assert_eq!(own["token"], "s3cret");
    let foreign = call(
        &services,
        "acme.other",
        HostOp::GetExtensionSettings {
            namespace: "acme".into(),
            layer: SettingsLayer::Effective,
        },
    )
    .await
    .unwrap();
    assert!(foreign.get("token").is_none(), "{foreign}");
    assert!(matches!(
        call(
            &services,
            "acme.other",
            set("acme", "level", Some(json!("low")))
        )
        .await,
        Err(HostError::Invalid(_))
    ));
    assert!(matches!(
        call(
            &services,
            "acme.ext",
            set("acme", "level", Some(json!("max")))
        )
        .await,
        Err(HostError::Invalid(_))
    ));

    // Frontend view: masked secret, nothing secret in the settings object.
    assert_eq!(
        registry.values("acme", SettingsLayer::Effective)["token"],
        SECRET_MASK
    );
    let merged = call(&services, "acme.ext", HostOp::GetSettings)
        .await
        .unwrap();
    assert_eq!(
        merged["extensionSettings"]["acme"],
        json!({"level": "high"})
    );
    assert!(!merged.to_string().contains("s3cret"));
    let all = call(&services, "acme.ext", HostOp::GetAllExtensionSettings)
        .await
        .unwrap();
    assert_eq!(all["acme"]["token"], "s3cret");

    // Frontend write → owner event.
    let mut changes = registry.subscribe();
    registry
        .set(
            "acme",
            "ui.compact",
            Some(json!(true)),
            SettingsScope::Global,
        )
        .unwrap();
    assert_eq!(changes.recv().await.unwrap().keys, ["ui.compact"]);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        let keys: Vec<Value> = owner
            .received()
            .into_iter()
            .filter(|(kind, _)| kind == "oino_settings_changed")
            .map(|(_, payload)| payload["keys"].clone())
            .collect();
        if keys.contains(&json!(["ui.compact"])) {
            assert!(keys.contains(&json!(["level"])));
            assert!(keys.contains(&json!(["token"])));
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "events: {keys:?}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }

    // The same writes reach frontends as sticky `oino:settings:<namespace>` topics.
    let topic = services
        .host
        .topics()
        .get(&oino_core::topics::settings_topic("acme"))
        .expect("settings topic");
    assert_eq!(topic.publisher, oino_core::topics::CORE_PUBLISHER);
    assert!(topic.sticky);
    assert_eq!(topic.data["namespace"], "acme");
    assert_eq!(topic.data["keys"], json!(["ui.compact"]));
    assert_eq!(topic.data["scope"], "global");
    assert_eq!(topic.data["extensionId"], "acme.ext");

    // Dynamic registration of a second namespace; names are owned.
    call(
        &services,
        "acme.other",
        HostOp::RegisterSettings(json!({"namespace": "other", "title": "Other", "sections": []})),
    )
    .await
    .unwrap();
    assert!(matches!(
        call(&services, "acme.other", HostOp::RegisterSettings(schema())).await,
        Err(HostError::Invalid(_))
    ));
    assert_eq!(registry.owned_by("acme.other"), ["other"]);
}

#[tokio::test]
async fn untrusted_project_writes_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let owner = Arc::new(FakeExtension::new(
        "acme.ext",
        Registration {
            settings: vec![schema()],
            ..Registration::default()
        },
    ));
    let services = services(dir.path(), owner).await;
    let result = call(
        &services,
        "acme.ext",
        HostOp::SetExtensionSetting {
            namespace: "acme".into(),
            key: "level".into(),
            value: Some(json!("high")),
            scope: SettingsScope::Project,
        },
    )
    .await;
    assert!(matches!(result, Err(HostError::Failed(m)) if m.contains("not trusted")));
}
