//! Registry behaviour tests (catalog, availability, routing, credentials, virtual models).

use std::sync::atomic::{AtomicUsize, Ordering};

use futures::executor::block_on;
use futures::future::BoxFuture;
use serde_json::json;

use super::*;
use crate::auth::oauth::now_ms;
use crate::auth_check::{AuthCheckReason, AuthCheckStatus, AuthCheckTarget, check_provider_auth};
use crate::env::MapEnv;
use crate::virtual_models::{RouteTarget, VIRTUAL_MODEL_API};
use crate::{AuthCredential, ModelsJson};

fn registration(value: serde_json::Value) -> ProviderRegistration {
    serde_json::from_value(value).unwrap()
}

fn anthropic() -> ProviderRegistration {
    registration(json!({
        "id": "anthropic", "name": "Anthropic", "api": "anthropic-messages",
        "baseUrl": "https://api.anthropic.com", "authEnv": ["ANTHROPIC_API_KEY"], "oauth": true,
        "models": [
            {"id": "claude-sonnet-4-5", "name": "Claude Sonnet 4.5", "reasoning": true},
            {"id": "claude-haiku-4-5"}
        ]
    }))
}

fn openrouter() -> ProviderRegistration {
    registration(json!({
        "id": "openrouter", "api": "openai-completions", "baseUrl": "https://openrouter.ai/api/v1",
        "authEnv": ["OPENROUTER_API_KEY"], "models": "dynamic"
    }))
}

fn model(provider: &str, id: &str, api: &str) -> Model {
    serde_json::from_value(json!({
        "id": id, "name": id, "api": api, "provider": provider, "baseUrl": "https://x",
        "input": ["text"], "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
        "reasoning": true, "contextWindow": 1000, "maxTokens": 100
    }))
    .unwrap()
}

fn registry(env: MapEnv) -> ModelRegistry {
    ModelRegistry::in_memory(Arc::new(env))
}

fn models_json(value: &serde_json::Value) -> ModelsJson {
    ModelsJson::parse_at(&value.to_string(), Path::new("/m.json"))
}

fn keys(models: &[Model]) -> Vec<String> {
    models
        .iter()
        .map(|m| format!("{}/{}", m.provider, m.id))
        .collect()
}

#[test]
fn availability_needs_credentials_and_an_implementation() {
    let registry = registry(MapEnv::new().with("ANTHROPIC_API_KEY", "sk"));
    registry
        .register_provider("ext.anthropic", anthropic())
        .unwrap();
    registry
        .register_provider("ext.openrouter", openrouter())
        .unwrap();
    registry.register_api_implementor("openai-completions", "ext.openai");
    registry.set_models_json(models_json(&json!({"providers": {
        "ollama": {"baseUrl": "http://localhost:11434/v1", "api": "openai-completions", "apiKey": "ollama",
                    "models": [{"id": "qwen"}]},
        "weird": {"baseUrl": "http://x", "api": "unknown-api", "apiKey": "k", "models": [{"id": "w"}]}
    }})));
    assert_eq!(
        keys(&registry.all()),
        vec![
            "anthropic/claude-sonnet-4-5",
            "anthropic/claude-haiku-4-5",
            "ollama/qwen",
            "weird/w"
        ]
    );
    assert_eq!(
        keys(&registry.available()),
        vec![
            "anthropic/claude-sonnet-4-5",
            "anthropic/claude-haiku-4-5",
            "ollama/qwen"
        ]
    );
    let ollama = registry.find("ollama", "qwen").unwrap();
    assert_eq!(
        registry.route_target(&ollama).as_deref(),
        Some("ext.openai")
    );
    let sonnet = registry.find("anthropic", "claude-sonnet-4-5").unwrap();
    assert_eq!(
        registry.route_target(&sonnet).as_deref(),
        Some("ext.anthropic")
    );
    assert_eq!(sonnet.base_url, "https://api.anthropic.com");
    assert_eq!(
        registry.diagnostics(),
        vec![RegistryDiagnostic {
            provider: "weird".into(),
            message: "No provider extension implements api \"unknown-api\"".into()
        }]
    );
    assert_eq!(registry.provider_display_name("anthropic"), "Anthropic");
    assert_eq!(registry.provider_display_name("ollama"), "ollama");
}

#[test]
fn dynamic_models_and_cache() {
    let dir = tempfile::tempdir_in(test_tmp()).unwrap();
    let options = ModelRegistryOptions {
        model_cache: Some(ModelCache::new(dir.path())),
        env: Arc::new(MapEnv::new().with("OPENROUTER_API_KEY", "k")),
        ..ModelRegistryOptions::default()
    };
    let auth = Arc::new(AuthStorage::in_memory(IndexMap::new()));
    let registry = ModelRegistry::new(auth.clone(), options.clone());
    registry.register_provider("ext.or", openrouter()).unwrap();
    assert_eq!(registry.all(), Vec::<Model>::new());
    registry
        .set_provider_models("openrouter", &[model("other", "m1", "openai-completions")])
        .unwrap();
    assert_eq!(keys(&registry.all()), vec!["openrouter/m1"]);
    assert_eq!(keys(&registry.available()), vec!["openrouter/m1"]);
    // A new registry starts from the cache.
    let second = ModelRegistry::new(auth, options);
    second.register_provider("ext.or", openrouter()).unwrap();
    assert_eq!(keys(&second.all()), vec!["openrouter/m1"]);
    assert!(matches!(
        second.set_provider_models("nope", &[]),
        Err(RegistryError::UnknownProvider(_))
    ));
    second.register_provider("ext.a", anthropic()).unwrap();
    assert!(matches!(
        second.set_provider_models("anthropic", &[]),
        Err(RegistryError::StaticModels(_))
    ));
}

fn test_tmp() -> PathBuf {
    let root = std::env::var_os("OINO_TEST_TMP").map_or_else(
        || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp"),
        PathBuf::from,
    );
    std::fs::create_dir_all(&root).unwrap();
    root
}

#[test]
fn credential_precedence() {
    let registry = registry(MapEnv::new().with("ANTHROPIC_API_KEY", "env"));
    registry.register_provider("ext.a", anthropic()).unwrap();
    let key = || block_on(registry.get_api_key("anthropic"));
    assert_eq!(key().as_deref(), Some("env"));
    registry
        .auth_storage()
        .set("anthropic", &AuthCredential::api_key("stored"))
        .unwrap();
    assert_eq!(key().as_deref(), Some("stored"));
    registry.set_runtime_api_key("anthropic", "runtime");
    assert_eq!(key().as_deref(), Some("runtime"));
    assert_eq!(
        registry.auth_status("anthropic").source,
        Some(AuthStatusSource::Runtime)
    );
    registry.remove_runtime_api_key("anthropic");
    assert_eq!(
        registry.auth_status("anthropic").source,
        Some(AuthStatusSource::Stored)
    );
    let overridden = block_on(registry.get_auth(
        "anthropic",
        &AuthOverrides {
            api_key: Some("per-request".into()),
            ..AuthOverrides::default()
        },
    ))
    .unwrap()
    .unwrap();
    assert_eq!(overridden.api_key.as_deref(), Some("per-request"));
}

#[test]
fn request_headers_and_auth_header() {
    let registry = registry(MapEnv::new().with("ORG", "org-1"));
    registry.register_api_implementor("openai-completions", "ext.openai");
    registry.set_models_json(models_json(&json!({"providers": {
        "proxy": {"baseUrl": "https://proxy", "api": "openai-completions", "apiKey": "key",
                   "authHeader": true, "headers": {"X-Org": "$ORG"},
                   "models": [{"id": "m", "headers": {"x-org": "model-org", "X-Model": "1"}}]},
        "nokey": {"baseUrl": "https://nokey", "api": "openai-completions", "authHeader": true,
                   "models": [{"id": "m"}]},
        "open": {"baseUrl": "https://open", "api": "openai-completions", "headers": {"X-A": "a"},
                  "models": [{"id": "m"}]}
    }})));
    let proxy = registry.find("proxy", "m").unwrap();
    let auth =
        block_on(registry.get_api_key_and_headers(&proxy, &AuthOverrides::default())).unwrap();
    assert_eq!(auth.api_key.as_deref(), Some("key"));
    let headers = auth.headers.unwrap();
    assert_eq!(
        headers
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect::<Vec<_>>(),
        vec!["Authorization=Bearer key", "x-org=model-org", "X-Model=1"]
    );
    let nokey = registry.find("nokey", "m").unwrap();
    assert_eq!(
        block_on(registry.get_api_key_and_headers(&nokey, &AuthOverrides::default()))
            .unwrap_err()
            .message,
        "No API key found for \"nokey\""
    );
    let open = registry.find("open", "m").unwrap();
    let auth =
        block_on(registry.get_api_key_and_headers(&open, &AuthOverrides::default())).unwrap();
    assert_eq!(auth.api_key, None);
    assert_eq!(
        auth.headers.unwrap().get("X-A").map(String::as_str),
        Some("a")
    );
    assert!(!registry.has_configured_auth("open"));
}

struct Refresher(AtomicUsize);

impl OAuthRefresher for Refresher {
    fn refresh<'a>(
        &'a self,
        provider: &'a str,
        credential: serde_json::Value,
    ) -> BoxFuture<'a, Result<serde_json::Value, String>> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Box::pin(async move {
            assert_eq!(provider, "anthropic");
            assert_eq!(credential["accountId"], "acct");
            Ok(
                json!({"access": "new-access", "refresh": "r2", "expires": now_ms() + 3_600_000, "accountId": "acct"}),
            )
        })
    }
}

#[test]
fn oauth_credentials_refresh_through_the_registry() {
    let registry = registry(MapEnv::new());
    registry.register_provider("ext.a", anthropic()).unwrap();
    let credential = AuthCredential::from_value(&json!({
        "type": "oauth", "access": "old", "refresh": "r1", "expires": now_ms() + 1000, "accountId": "acct"
    }))
    .unwrap();
    registry
        .auth_storage()
        .set("anthropic", &credential)
        .unwrap();
    assert!(registry.is_using_oauth("anthropic"));
    // Without a refresher, the expiring token cannot be used.
    let error = block_on(registry.get_auth("anthropic", &AuthOverrides::default())).unwrap_err();
    assert_eq!(error.to_string(), "OAuth refresh failed for anthropic");
    let refresher = Arc::new(Refresher(AtomicUsize::new(0)));
    registry.set_oauth_refresher(refresher.clone());
    assert_eq!(
        block_on(registry.get_api_key("anthropic")).as_deref(),
        Some("new-access")
    );
    assert_eq!(
        block_on(registry.get_api_key("anthropic")).as_deref(),
        Some("new-access")
    );
    assert_eq!(refresher.0.load(Ordering::SeqCst), 1);
    // OAuth stored for a provider without OAuth support: not configured.
    registry.set_models_json(models_json(&json!({"providers": {
        "plain": {"baseUrl": "https://p", "api": "openai-completions", "models": [{"id": "m"}]}
    }})));
    registry.auth_storage().set("plain", &credential).unwrap();
    assert!(!registry.has_configured_auth("plain"));
}

#[test]
fn config_overrides_merge_and_validate() {
    let registry = registry(MapEnv::new().with("ANTHROPIC_API_KEY", "sk"));
    registry.register_provider("ext.a", anthropic()).unwrap();
    registry
        .register_provider_config(
            "anthropic",
            ProviderConfigOverride {
                base_url: Some("https://proxy".into()),
                ..ProviderConfigOverride::default()
            },
        )
        .unwrap();
    assert!(registry.all().iter().all(|m| m.base_url == "https://proxy"));
    let error = registry
        .register_provider_config(
            "fresh",
            ProviderConfigOverride {
                models: Some(vec![serde_json::from_value(json!({"id": "x"})).unwrap()]),
                ..ProviderConfigOverride::default()
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Provider fresh, model x: no \"api\" specified. Set it at model level or provider level."
    );
    assert_eq!(
        registry.registered_providers(),
        vec!["anthropic".to_owned()]
    );
    registry.unregister_provider("anthropic");
    assert_eq!(registry.all(), Vec::<Model>::new());
    assert_eq!(registry.registered_providers(), Vec::<String>::new());
}

#[test]
fn models_json_errors_are_reported() {
    let registry = registry(MapEnv::new());
    registry.set_models_json(models_json(&json!({"providers": {"x": {}}})));
    assert_eq!(
        registry.error().as_deref(),
        Some(
            "Provider \"x\": Provider x: must specify \"baseUrl\", \"headers\", \"compat\", \"modelOverrides\", or \"models\"."
        )
    );
    registry.set_models_json(models_json(&json!({"providers": 5})));
    assert!(
        registry
            .error()
            .unwrap()
            .starts_with("Invalid models.json schema:\n  - providers: must be object")
    );
}

fn router_definition() -> VirtualModelDefinition {
    serde_json::from_value(json!({"provider": "router", "id": "auto", "name": "Auto", "thinkingLevels": ["low", "high"]}))
        .unwrap()
}

#[test]
fn virtual_models_route_and_validate() {
    let registry = registry(MapEnv::new().with("ANTHROPIC_API_KEY", "sk"));
    registry.register_provider("ext.a", anthropic()).unwrap();
    registry
        .register_virtual_model("ext.router", router_definition())
        .unwrap();
    let auto = registry.find("router", "auto").unwrap();
    assert_eq!(auto.api, VIRTUAL_MODEL_API);
    assert!(registry.has_configured_auth("router"));
    assert!(keys(&registry.available()).contains(&"router/auto".to_owned()));
    assert_eq!(registry.find_physical("router", "auto"), None);

    // Conflict with a physical model.
    let mut conflicting = router_definition();
    conflicting.provider = "anthropic".into();
    conflicting.id = "claude-haiku-4-5".into();
    assert!(
        registry
            .register_virtual_model("ext.router", conflicting)
            .is_err()
    );

    // A virtual model under a physical provider hides nothing else and follows its auth.
    let mut listed = router_definition();
    listed.provider = "anthropic".into();
    registry
        .register_virtual_model("ext.router", listed)
        .unwrap();
    assert!(registry.find("anthropic", "auto").is_some());

    let route = registry
        .validate_route(
            &auto,
            RouteResult {
                model: RouteTarget {
                    provider: "anthropic".into(),
                    id: "claude-sonnet-4-5".into(),
                },
                thinking_level: ThinkingLevel::Max,
                state: Some(json!({"phase": "plan"})),
            },
        )
        .unwrap();
    assert_eq!(route.thinking_level, ThinkingLevel::High);
    assert_eq!(route.model.id, "claude-sonnet-4-5");
    let error = registry
        .validate_route(
            &auto,
            RouteResult {
                model: RouteTarget {
                    provider: "router".into(),
                    id: "auto".into(),
                },
                thinking_level: ThinkingLevel::Low,
                state: None,
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Virtual model router/auto routed to router/auto, which is not a physical model."
    );

    let request = registry
        .route_request(
            &auto,
            Vec::new(),
            RouteReason::Direct,
            ThinkingLevel::Low,
            None,
            Some(json!(1)),
        )
        .unwrap();
    assert_eq!(request.state, None);
    assert_eq!(
        serde_json::to_value(&request).unwrap()["reason"],
        json!("direct")
    );

    registry.unregister_virtual_model("router", "auto");
    assert!(registry.find("router", "auto").is_none());
    assert_eq!(
        registry
            .route_request(
                &auto,
                Vec::new(),
                RouteReason::User,
                ThinkingLevel::Low,
                None,
                None
            )
            .unwrap_err()
            .to_string(),
        "Virtual model router/auto is not registered."
    );
    registry.unregister_extension("ext.router");
    assert!(registry.find("anthropic", "auto").is_none());
}

#[test]
fn no_credentials_route_error() {
    let registry = registry(MapEnv::new());
    registry.register_provider("ext.a", anthropic()).unwrap();
    registry
        .register_virtual_model("ext.router", router_definition())
        .unwrap();
    let auto = registry.find("router", "auto").unwrap();
    let error = registry
        .validate_route(
            &auto,
            RouteResult {
                model: RouteTarget {
                    provider: "anthropic".into(),
                    id: "claude-haiku-4-5".into(),
                },
                thinking_level: ThinkingLevel::Off,
                state: None,
            },
        )
        .unwrap_err();
    assert_eq!(
        error.to_string(),
        "Virtual model router/auto routed to anthropic/claude-haiku-4-5, which has no credentials."
    );
}

#[test]
fn auth_check_exit_codes() {
    let registry = registry(MapEnv::new().with("ANTHROPIC_API_KEY", "sk"));
    registry.register_provider("ext.a", anthropic()).unwrap();
    registry.register_provider("ext.or", openrouter()).unwrap();
    let check = |provider: Option<&str>, model: Option<&str>| {
        block_on(check_provider_auth(
            AuthCheckTarget { provider, model },
            &registry,
            true,
        ))
    };
    let ready = check(Some("anthropic"), None);
    assert_eq!(ready.status.exit_code(), 0);
    assert_eq!(
        ready.render(true, None),
        r#"{"status":"ready","provider":"anthropic","authType":"api_key"}"#
    );
    let not_ready = check(Some("openrouter"), None);
    assert_eq!(
        not_ready.reason,
        Some(AuthCheckReason::CredentialsNotConfigured)
    );
    assert_eq!(not_ready.status.exit_code(), 1);
    assert_eq!(
        check(Some("missing"), None).reason,
        Some(AuthCheckReason::ProviderNotFound)
    );
    let by_model = check(None, Some("sonnet"));
    assert_eq!(
        (by_model.status, by_model.provider.as_str()),
        (AuthCheckStatus::Ready, "anthropic")
    );
    let invalid = check(None, Some("nothing-matches"));
    assert_eq!(
        (invalid.status.exit_code(), invalid.provider.as_str()),
        (2, "nothing-matches")
    );
    registry.set_models_json(models_json(&json!({"providers": {"x": {}}})));
    assert_eq!(
        check(Some("anthropic"), None).status,
        AuthCheckStatus::Invalid
    );
}
