//! v3-perf §4.5: app hosts answer the model catalogue from the on-disk cache, never probe a
//! dynamic provider on services creation or wake, and back off a dead provider.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "tests assert by unwrapping"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use futures::future::BoxFuture;
use futures::stream::BoxStream;
use indexmap::IndexMap;
use oino_ai::{JsonValue, Model, ProviderEvent};
use oino_core::extensions::api::{
    CallContext, ExtensionCallError, ExtensionInstance, ExtensionKind, ExtensionMode,
    ProviderInstance, Registration,
};
use oino_core::services::{ResourceFlags, Services, ServicesOptions, StaticLoader};
use oino_models::{ListSchedule, ModelCache, ModelRegistry, ModelRegistryOptions};
use oino_settings::SettingsManager;
use serde_json::json;

/// `list-models` calls seen by the dead provider.
static CALLS: AtomicUsize = AtomicUsize::new(0);

/// A dynamic provider whose server refuses connections, slowly.
struct Dead;

impl ProviderInstance for Dead {
    fn stream(
        &self,
        _request: JsonValue,
        _cancel: oino_extension_api::CancellationToken,
    ) -> BoxFuture<
        '_,
        Result<BoxStream<'static, Result<ProviderEvent, ExtensionCallError>>, ExtensionCallError>,
    > {
        Box::pin(async { Err(ExtensionCallError::new("unused")) })
    }

    fn list_models(
        &self,
        _provider_id: &str,
        credential: Option<JsonValue>,
    ) -> BoxFuture<'_, Result<Vec<Model>, ExtensionCallError>> {
        assert_eq!(
            credential.as_ref().and_then(|c| c["baseUrl"].as_str()),
            Some("http://localhost:18969/v1")
        );
        CALLS.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            Err(ExtensionCallError::new("connection refused"))
        })
    }
}

struct DeadComponent;

impl ExtensionInstance for DeadComponent {
    fn id(&self) -> &'static str {
        "local-dead"
    }
    fn kind(&self) -> ExtensionKind {
        ExtensionKind::Provider
    }
    fn init(&self, _cx: CallContext) -> BoxFuture<'_, Result<Registration, ExtensionCallError>> {
        Box::pin(async {
            Ok(Registration {
                apis: vec!["dead-api".into()],
                providers: vec![json!({
                    "id": "deadlocal", "name": "Dead", "api": "dead-api", "models": "dynamic",
                })],
                ..Registration::default()
            })
        })
    }
    fn on_event(
        &self,
        _kind: &str,
        _payload: JsonValue,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        Box::pin(async { Ok(None) })
    }
    fn run_tool(
        &self,
        _tool: &str,
        _invocation: oino_agent::ToolInvocation,
        _updates: oino_agent::UpdateSink,
        _cancel: oino_extension_api::CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<oino_agent::AgentToolResult, ExtensionCallError>> {
        Box::pin(async { Err(ExtensionCallError::new("unused")) })
    }
    fn run_command(
        &self,
        _name: &str,
        _args: &str,
        _cancel: oino_extension_api::CancellationToken,
        _cx: CallContext,
    ) -> BoxFuture<'_, Result<Option<JsonValue>, ExtensionCallError>> {
        Box::pin(async { Ok(None) })
    }
    fn provider(&self) -> Option<&dyn ProviderInstance> {
        Some(&Dead)
    }
}

async fn services(dir: &std::path::Path, cache: &ModelCache) -> Arc<Services> {
    let models = ModelRegistry::new(
        Arc::new(oino_models::AuthStorage::in_memory(IndexMap::new())),
        ModelRegistryOptions {
            model_cache: Some(cache.clone()),
            ..ModelRegistryOptions::default()
        },
    );
    models.set_models_json(oino_models::ModelsJson::parse_at(
        &json!({"providers": {"deadlocal": {"baseUrl": "http://localhost:18969/v1"}}}).to_string(),
        &dir.join("models.json"),
    ));
    Services::create(ServicesOptions {
        cwd: dir.to_path_buf(),
        agent_dir: dir.join("agent"),
        docs_root: dir.join("docs"),
        mode: ExtensionMode::Desktop,
        has_ui: false,
        flags: ResourceFlags {
            no_context_files: true,
            ..ResourceFlags::default()
        },
        extension_flag_values: IndexMap::new(),
        loader: Arc::new(StaticLoader(vec![Arc::new(DeadComponent)])),
        approve: Some(false),
        models: Some(models),
        settings: Some(SettingsManager::in_memory(serde_json::Map::new())),
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn a_dead_dynamic_provider_is_probed_once_and_the_cache_answers() {
    let dir = tempfile::tempdir().unwrap();
    let cache = ModelCache::new(dir.path().join("cache"));
    // A stale cached list (older than an hour): the catalogue shows it, a refresh is due.
    let model: Model = serde_json::from_value(json!({
        "id": "llama-cached", "name": "Llama", "api": "dead-api", "provider": "deadlocal",
        "baseUrl": "http://127.0.0.1:1", "reasoning": false, "input": ["text"],
        "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "contextWindow": 8192, "maxTokens": 1024,
    }))
    .unwrap();
    std::fs::create_dir_all(dir.path().join("cache")).unwrap();
    std::fs::write(
        cache.path("deadlocal"),
        json!({"models": [model], "checkedAt": 0}).to_string(),
    )
    .unwrap();
    ListSchedule::reset("deadlocal");

    // Ten wakes in a row (ten projects' services sets): none waits on the provider.
    let mut sets = Vec::new();
    for _ in 0..10 {
        let start = Instant::now();
        let set = services(dir.path(), &cache).await;
        assert!(
            start.elapsed() < Duration::from_millis(250),
            "services waited on list-models: {:?}",
            start.elapsed()
        );
        let all = set.models.all();
        assert!(
            all.iter().any(|m| m.id == "llama-cached"),
            "the cached list answers at once: {all:?}"
        );
        sets.push(set);
    }
    // The background refresh probes once for the whole process, then backs off.
    tokio::time::sleep(Duration::from_millis(800)).await;
    assert_eq!(CALLS.load(Ordering::SeqCst), 1, "probed exactly once");
    assert_eq!(ListSchedule::failures("deadlocal"), 1);
    assert_eq!(
        sets[0].models.last_list_error("deadlocal").as_deref(),
        Some("connection refused")
    );
    // A failed refresh keeps the cached list.
    assert!(sets[0].models.all().iter().any(|m| m.id == "llama-cached"));
    // More wakes during the 1-minute backoff: still no probe.
    for _ in 0..5 {
        sets.push(services(dir.path(), &cache).await);
    }
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(CALLS.load(Ordering::SeqCst), 1, "no probe during backoff");
    ListSchedule::reset("deadlocal");
}
