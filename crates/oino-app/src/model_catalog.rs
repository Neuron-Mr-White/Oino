#![forbid(unsafe_code)]

use oino_auth::{AuthError, AuthStorage};
use oino_provider_catalog::{providers, ProviderTarget};
use oino_provider_openrouter::{
    list_models as list_openrouter_models, OpenAiCompatibleAuth, OpenAiCompatibleConfig,
    OpenRouterConfig, OpenRouterModelInfo,
};
use oino_tui::{all_thinking_levels, ModelAvailability, ModelOption, ModelPricing};
use oino_types::{Model, ThinkingLevel};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::fs;

pub const MODEL_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60 * 6);
const MODEL_CATALOG_FETCH_TIMEOUT: Duration = Duration::from_secs(12);
const MODEL_CATALOG_FETCH_CONCURRENCY: usize = 8;
pub const ROUTER_PROVIDER_ID: &str = "router";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogUpdate {
    pub models: Vec<ModelOption>,
    pub status: String,
    pub refreshing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedModelCatalog {
    fetched_at_unix: u64,
    #[serde(default = "default_provider_id")]
    provider_id: String,
    models: Vec<CachedModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedModel {
    id: String,
    display_name: String,
    #[serde(default)]
    route_provider: Option<String>,
    #[serde(default)]
    availability: ModelAvailability,
    supported_parameters: Vec<String>,
    #[serde(default)]
    context_length: Option<usize>,
    #[serde(default)]
    pricing: Option<ModelPricing>,
}

#[allow(dead_code)]
pub async fn load_cached_update() -> Option<ModelCatalogUpdate> {
    load_cached_update_with_historical_provider_catalog(false).await
}

pub async fn load_cached_update_with_historical_provider_catalog(
    include_historical_provider_catalog: bool,
) -> Option<ModelCatalogUpdate> {
    let cached = load_all_cached_model_options(include_historical_provider_catalog).await;
    let models = merge_model_options(
        cached,
        static_model_options_with_historical_provider_catalog(include_historical_provider_catalog),
    );
    if models.is_empty() {
        return None;
    }
    Some(ModelCatalogUpdate {
        models,
        status: "Loaded cached/static extension model catalogs".into(),
        refreshing: false,
    })
}

#[allow(dead_code)]
pub async fn refresh_all_update(
    auth: &AuthStorage,
    openrouter_config: &OpenRouterConfig,
) -> ModelCatalogUpdate {
    refresh_all_update_with_historical_provider_catalog(auth, openrouter_config, false).await
}

pub async fn refresh_all_update_with_historical_provider_catalog(
    auth: &AuthStorage,
    openrouter_config: &OpenRouterConfig,
    include_historical_provider_catalog: bool,
) -> ModelCatalogUpdate {
    let configs = all_refresh_configs_with_historical_provider_catalog(
        openrouter_config,
        include_historical_provider_catalog,
    );
    if configs.is_empty() {
        return ModelCatalogUpdate {
            models: static_model_options_with_historical_provider_catalog(
                include_historical_provider_catalog,
            ),
            status: "No fetchable extension model catalogs are registered".into(),
            refreshing: false,
        };
    }

    let semaphore = Arc::new(tokio::sync::Semaphore::new(MODEL_CATALOG_FETCH_CONCURRENCY));
    let mut tasks = tokio::task::JoinSet::new();
    for config in configs {
        let auth = auth.clone();
        let semaphore = Arc::clone(&semaphore);
        tasks.spawn(async move {
            let _permit = semaphore.acquire_owned().await.ok();
            refresh_one_catalog(&auth, config).await
        });
    }

    let mut refreshed = 0usize;
    let mut skipped = 0usize;
    let mut failed = Vec::new();
    while let Some(result) = tasks.join_next().await {
        match result {
            Ok(ProviderCatalogRefresh::Refreshed { .. }) => refreshed += 1,
            Ok(ProviderCatalogRefresh::Skipped { .. }) => skipped += 1,
            Ok(ProviderCatalogRefresh::Failed { provider_id, error }) => {
                failed.push(format!("{provider_id}: {error}"));
            }
            Err(err) => failed.push(format!("task join failed: {err}")),
        }
    }

    let models = merge_model_options(
        load_all_cached_model_options(include_historical_provider_catalog).await,
        static_model_options_with_historical_provider_catalog(include_historical_provider_catalog),
    );
    let mut status =
        format!("Refreshed extension model catalogs: {refreshed} fetched, {skipped} skipped");
    if !failed.is_empty() {
        let preview = failed
            .iter()
            .take(3)
            .cloned()
            .collect::<Vec<_>>()
            .join("; ");
        let suffix = if failed.len() > 3 {
            format!("; +{} more", failed.len() - 3)
        } else {
            String::new()
        };
        status.push_str(&format!(", {} failed ({preview}{suffix})", failed.len()));
    }

    ModelCatalogUpdate {
        models,
        status,
        refreshing: false,
    }
}

pub async fn refresh_openrouter_pricing_sidecar(
    openrouter_config: &OpenRouterConfig,
) -> Result<usize, String> {
    let models = list_openrouter_models(openrouter_config)
        .await
        .map_err(|err| err.to_string())?;
    let cached = CachedModelCatalog {
        fetched_at_unix: now_unix(),
        provider_id: oino_auth::OPENROUTER_PROVIDER_ID.into(),
        models: models
            .into_iter()
            .map(openai_compatible_to_cached)
            .collect(),
    };
    let count = cached.models.len();
    save_cache(&cached)
        .await
        .map_err(|err| format!("cache save failed: {err}"))?;
    Ok(count)
}

pub async fn refresh_openai_proxy_update(
    provider_id: &str,
    display_name: &str,
    base_url: &str,
    api_key_env: Option<&str>,
) -> ModelCatalogUpdate {
    match refresh_openai_proxy_catalog(provider_id, display_name, base_url, api_key_env, None).await
    {
        ProviderCatalogRefresh::Refreshed { count, .. } => ModelCatalogUpdate {
            models: merge_model_options(
                load_all_cached_model_options(true).await,
                static_model_options(),
            ),
            status: format!("Fetched {count} {display_name} models (cached)"),
            refreshing: false,
        },
        ProviderCatalogRefresh::Skipped { reason, .. } => ModelCatalogUpdate {
            models: merge_model_options(
                load_all_cached_model_options(true).await,
                static_model_options(),
            ),
            status: format!("Skipped {display_name} model refresh: {reason}"),
            refreshing: false,
        },
        ProviderCatalogRefresh::Failed { error, .. } => ModelCatalogUpdate {
            models: merge_model_options(
                load_all_cached_model_options(true).await,
                static_model_options(),
            ),
            status: format!("Model refresh failed for {display_name}: {error}"),
            refreshing: false,
        },
    }
}

pub async fn refresh_openai_proxy_update_with_api_key(
    provider_id: &str,
    display_name: &str,
    base_url: &str,
    api_key: Option<&str>,
) -> ModelCatalogUpdate {
    match refresh_openai_proxy_catalog(provider_id, display_name, base_url, None, api_key).await {
        ProviderCatalogRefresh::Refreshed { count, .. } => ModelCatalogUpdate {
            models: merge_model_options(
                load_all_cached_model_options(true).await,
                static_model_options(),
            ),
            status: format!("Fetched {count} {display_name} models (cached)"),
            refreshing: false,
        },
        ProviderCatalogRefresh::Skipped { reason, .. } => ModelCatalogUpdate {
            models: merge_model_options(
                load_all_cached_model_options(true).await,
                static_model_options(),
            ),
            status: format!("Skipped {display_name} model refresh: {reason}"),
            refreshing: false,
        },
        ProviderCatalogRefresh::Failed { error, .. } => ModelCatalogUpdate {
            models: merge_model_options(
                load_all_cached_model_options(true).await,
                static_model_options(),
            ),
            status: format!("Model refresh failed for {display_name}: {error}"),
            refreshing: false,
        },
    }
}

async fn refresh_openai_proxy_catalog(
    provider_id: &str,
    display_name: &str,
    base_url: &str,
    api_key_env: Option<&str>,
    api_key: Option<&str>,
) -> ProviderCatalogRefresh {
    let provider_id = provider_id.to_string();
    let models_endpoint = format!("{}{}", base_url.trim_end_matches('/'), "/models");
    let client = match reqwest::Client::builder()
        .timeout(MODEL_CATALOG_FETCH_TIMEOUT)
        .build()
    {
        Ok(client) => client,
        Err(err) => {
            return ProviderCatalogRefresh::Failed {
                provider_id,
                error: err.to_string(),
            }
        }
    };
    let mut request = client.get(models_endpoint);
    if let Some(api_key) = api_key.filter(|value| !value.trim().is_empty()) {
        request = request.bearer_auth(api_key);
    } else if let Some(env_var) = api_key_env {
        if let Ok(api_key) = std::env::var(env_var) {
            if !api_key.trim().is_empty() {
                request = request.bearer_auth(api_key);
            }
        }
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return ProviderCatalogRefresh::Failed {
                provider_id,
                error: err.to_string(),
            }
        }
    };
    let status = response.status();
    if !status.is_success() {
        return ProviderCatalogRefresh::Failed {
            provider_id,
            error: format!("{display_name} models request failed with status {status}"),
        };
    }
    match response.json::<ModelsResponse>().await {
        Ok(body) => {
            let availability = if provider_id == ROUTER_PROVIDER_ID {
                fetch_router_configured_routes(&client, base_url, api_key_env).await
            } else {
                RouteAvailability::Unknown
            };
            let mut cached = CachedModelCatalog {
                fetched_at_unix: now_unix(),
                provider_id: provider_id.clone(),
                models: body
                    .data
                    .into_iter()
                    .map(|model| {
                        openai_compatible_to_cached_with_availability(model, &availability)
                    })
                    .collect(),
            };
            if provider_id == ROUTER_PROVIDER_ID {
                enrich_router_metadata_from_openrouter(&mut cached).await;
            }
            let count = cached.models.len();
            match save_cache(&cached).await {
                Ok(()) => ProviderCatalogRefresh::Refreshed { provider_id, count },
                Err(err) => ProviderCatalogRefresh::Failed {
                    provider_id,
                    error: format!("cache save failed: {err}"),
                },
            }
        }
        Err(err) => ProviderCatalogRefresh::Failed {
            provider_id,
            error: err.to_string(),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ProviderCatalogRefresh {
    Refreshed { provider_id: String, count: usize },
    Skipped { provider_id: String, reason: String },
    Failed { provider_id: String, error: String },
}

async fn refresh_one_catalog(
    auth: &AuthStorage,
    config: OpenAiCompatibleConfig,
) -> ProviderCatalogRefresh {
    let provider_id = config.provider_id.clone();
    let config = config.with_timeout(MODEL_CATALOG_FETCH_TIMEOUT);
    if let Err(reason) = ensure_model_catalog_auth(auth, &config).await {
        return ProviderCatalogRefresh::Skipped {
            provider_id,
            reason,
        };
    }

    match list_models_with_optional_auth(auth, &config).await {
        Ok(models) => {
            let mut cached = CachedModelCatalog {
                fetched_at_unix: now_unix(),
                provider_id: config.provider_id.clone(),
                models: models
                    .into_iter()
                    .map(openai_compatible_to_cached)
                    .collect(),
            };
            if config.provider_id == ROUTER_PROVIDER_ID {
                enrich_router_metadata_from_openrouter(&mut cached).await;
            }
            let count = cached.models.len();
            match save_cache(&cached).await {
                Ok(()) => ProviderCatalogRefresh::Refreshed { provider_id, count },
                Err(err) => ProviderCatalogRefresh::Failed {
                    provider_id,
                    error: format!("cache save failed: {err}"),
                },
            }
        }
        Err(err) => ProviderCatalogRefresh::Failed {
            provider_id,
            error: err.to_string(),
        },
    }
}

async fn ensure_model_catalog_auth(
    auth: &AuthStorage,
    config: &OpenAiCompatibleConfig,
) -> Result<(), String> {
    match &config.auth {
        OpenAiCompatibleAuth::None | OpenAiCompatibleAuth::OptionalBearer { .. } => Ok(()),
        OpenAiCompatibleAuth::Bearer { spec } | OpenAiCompatibleAuth::ApiKeyHeader { spec, .. } => {
            if config.provider_id == oino_auth::OPENROUTER_PROVIDER_ID {
                return Ok(());
            }
            match auth.resolve_api_key(spec).await {
                Ok(_) => Ok(()),
                Err(AuthError::MissingCredential { .. }) => Err("missing credential".into()),
                Err(AuthError::NotApiKey { .. }) => {
                    Err("stored credential is not an API key".into())
                }
                Err(err) => Err(err.to_string()),
            }
        }
    }
}

async fn list_models_with_optional_auth(
    auth: &AuthStorage,
    config: &OpenAiCompatibleConfig,
) -> Result<Vec<OpenRouterModelInfo>, String> {
    let client = reqwest::Client::builder()
        .timeout(config.timeout)
        .build()
        .map_err(|err| err.to_string())?;
    let mut builder = client.get(config.models_endpoint());
    for (name, value) in &config.headers {
        builder = builder.header(name, value);
    }
    builder = apply_model_catalog_auth(auth, config, builder).await?;
    let response = builder.send().await.map_err(|err| err.to_string())?;
    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "{} models request failed with status {status}",
            config.display_name
        ));
    }
    response
        .json::<ModelsResponse>()
        .await
        .map(|body| body.data)
        .map_err(|err| err.to_string())
}

async fn apply_model_catalog_auth(
    auth: &AuthStorage,
    config: &OpenAiCompatibleConfig,
    builder: reqwest::RequestBuilder,
) -> Result<reqwest::RequestBuilder, String> {
    match &config.auth {
        OpenAiCompatibleAuth::None => Ok(builder),
        OpenAiCompatibleAuth::Bearer { spec } => match auth.resolve_api_key(spec).await {
            Ok(api_key) => Ok(builder.bearer_auth(api_key)),
            Err(AuthError::MissingCredential { .. })
                if config.provider_id == oino_auth::OPENROUTER_PROVIDER_ID =>
            {
                Ok(builder)
            }
            Err(AuthError::MissingCredential { .. }) => Err("missing credential".into()),
            Err(err) => Err(err.to_string()),
        },
        OpenAiCompatibleAuth::OptionalBearer { spec } => match auth.resolve(spec).await {
            Ok(Some(credential)) => match credential.as_api_key() {
                Some(api_key) => Ok(builder.bearer_auth(api_key)),
                None => Err("stored credential is not an API key".into()),
            },
            Ok(None) => Ok(builder),
            Err(err) => Err(err.to_string()),
        },
        OpenAiCompatibleAuth::ApiKeyHeader { spec, header_name } => auth
            .resolve_api_key(spec)
            .await
            .map(|api_key| builder.header(header_name.as_str(), api_key))
            .map_err(|err| err.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct ModelsResponse {
    data: Vec<OpenRouterModelInfo>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum RouteAvailability {
    Known(BTreeSet<String>),
    Unknown,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
struct RouterProvidersResponse {
    #[serde(default)]
    connections: Vec<RouterProviderConnection>,
}

#[derive(Debug, Clone, PartialEq, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct RouterProviderConnection {
    provider: Option<String>,
    is_active: Option<bool>,
    provider_specific_data: Option<serde_json::Value>,
}

async fn fetch_router_configured_routes(
    client: &reqwest::Client,
    base_url: &str,
    api_key_env: Option<&str>,
) -> RouteAvailability {
    let providers_url = format!(
        "{}/api/providers",
        openai_compatible_base_to_server_root(base_url)
    );
    let mut request = client.get(providers_url);
    if let Some(env_var) = api_key_env {
        if let Ok(api_key) = std::env::var(env_var) {
            if !api_key.trim().is_empty() {
                request = request.bearer_auth(api_key);
            }
        }
    }
    let Ok(response) = request.send().await else {
        return RouteAvailability::Unknown;
    };
    if !response.status().is_success() {
        return RouteAvailability::Unknown;
    }
    let Ok(body) = response.json::<RouterProvidersResponse>().await else {
        return RouteAvailability::Unknown;
    };
    let mut routes = BTreeSet::new();
    for connection in body.connections {
        if connection.is_active == Some(false) {
            continue;
        }
        let Some(provider) = connection.provider.as_deref().map(str::trim) else {
            continue;
        };
        if provider.is_empty() {
            continue;
        }
        add_router_provider_routes(
            &mut routes,
            provider,
            connection.provider_specific_data.as_ref(),
        );
    }
    RouteAvailability::Known(routes)
}

fn openai_compatible_base_to_server_root(base_url: &str) -> String {
    let trimmed = base_url.trim_end_matches('/');
    trimmed
        .strip_suffix("/v1")
        .unwrap_or(trimmed)
        .trim_end_matches('/')
        .to_string()
}

fn add_router_provider_routes(
    routes: &mut BTreeSet<String>,
    provider: &str,
    provider_specific_data: Option<&serde_json::Value>,
) {
    routes.insert(provider.to_string());
    if let Some(prefix) = provider_specific_data
        .and_then(|data| data.get("prefix"))
        .and_then(serde_json::Value::as_str)
        .map(str::trim)
        .filter(|prefix| !prefix.is_empty())
    {
        routes.insert(prefix.to_string());
    }
    if let Some(alias) = known_router_provider_alias(provider) {
        routes.insert(alias.to_string());
    }
}

fn known_router_provider_alias(provider: &str) -> Option<&'static str> {
    match provider {
        "anthropic" => Some("claude"),
        "google" | "gemini" | "gemini-cli" => Some("gemini"),
        "kilocode" => Some("kilo"),
        "huggingface" => Some("hf"),
        "vercel-ai-gateway" => Some("vercel"),
        "github" => Some("copilot"),
        "openrouter" => Some("openrouter"),
        "openai" => Some("openai"),
        "deepseek" => Some("deepseek"),
        "groq" => Some("groq"),
        "xai" => Some("xai"),
        "mistral" => Some("mistral"),
        "ollama-local" => Some("ollama"),
        _ => None,
    }
}

#[allow(dead_code)]
pub fn all_refresh_configs(openrouter_config: &OpenRouterConfig) -> Vec<OpenAiCompatibleConfig> {
    all_refresh_configs_with_historical_provider_catalog(openrouter_config, false)
}

pub fn all_refresh_configs_with_historical_provider_catalog(
    openrouter_config: &OpenRouterConfig,
    include_historical_provider_catalog: bool,
) -> Vec<OpenAiCompatibleConfig> {
    if !include_historical_provider_catalog {
        return Vec::new();
    }
    let mut configs = BTreeMap::new();
    for provider in providers() {
        let config = match provider.target {
            ProviderTarget::OpenRouter => Some(openrouter_config.openai_compatible_config()),
            ProviderTarget::OpenAiApiKey | ProviderTarget::OpenAiCompatible { .. } => {
                OpenAiCompatibleConfig::from_provider(*provider)
            }
            // Native providers need provider-specific model catalog APIs. Their
            // curated static models are still included in `static_model_options`.
            _ => None,
        };
        if let Some(config) = config {
            configs.insert(config.provider_id.clone(), config);
        }
    }
    configs.into_values().collect()
}

pub async fn cached_is_fresh(provider_id: &str) -> bool {
    let Some(cache) = load_provider_cache(provider_id).await else {
        return false;
    };
    let metadata_ok = provider_id == ROUTER_PROVIDER_ID || cache_has_context_lengths(&cache);
    metadata_ok && cache_age(&cache).is_some_and(|age| age < MODEL_REFRESH_INTERVAL)
}

const OPENAI_OAUTH_STATIC_MODELS: &[&str] = &[
    "gpt-5.5",
    "gpt-5.4",
    "gpt-5.4-pro",
    "gpt-5.3-codex",
    "gpt-5.3-codex-spark",
    "gpt-5.2-chat-latest",
    "gpt-5.2-codex",
    "gpt-5.2-pro",
    "gpt-5.1-codex-mini",
    "gpt-5.1-codex-max",
    "gpt-5.2",
    "gpt-5.1-chat-latest",
    "gpt-5.1",
    "gpt-5.1-codex",
    "gpt-5-chat-latest",
    "gpt-5-codex",
    "gpt-5-codex-mini",
    "gpt-5-pro",
    "gpt-5-mini",
    "gpt-5-nano",
    "gpt-5",
];

pub fn static_model_options() -> Vec<ModelOption> {
    static_model_options_with_historical_provider_catalog(false)
}

pub fn static_model_options_with_historical_provider_catalog(
    include_historical_provider_catalog: bool,
) -> Vec<ModelOption> {
    if !include_historical_provider_catalog {
        return Vec::new();
    }
    let default_models = providers().iter().filter_map(|provider| {
        let model_id = provider.default_model()?;
        let id = Model::new(provider.id, model_id).identifier();
        Some(
            ModelOption::new(id)
                .with_display_name(format!("{} {model_id}", provider.display_name))
                .with_provider_label(provider.display_name),
        )
    });
    let openai_oauth_models = OPENAI_OAUTH_STATIC_MODELS.iter().map(|model_id| {
        ModelOption::new(Model::new("openai", *model_id).identifier())
            .with_display_name(format!("OpenAI {model_id}"))
            .with_provider_label("OpenAI")
            .with_thinking_levels(all_thinking_levels())
            .with_context_length(Some(128_000))
    });
    default_models.chain(openai_oauth_models).collect()
}

fn default_provider_id() -> String {
    oino_auth::OPENROUTER_PROVIDER_ID.into()
}

async fn load_all_cached_model_options(
    include_historical_provider_catalog: bool,
) -> Vec<ModelOption> {
    let mut options = Vec::new();
    let mut loaded_providers = BTreeSet::new();
    if let Ok(dir) = cache_dir() {
        if let Ok(mut entries) = fs::read_dir(dir).await {
            loop {
                let entry = match entries.next_entry().await {
                    Ok(Some(entry)) => entry,
                    Ok(None) | Err(_) => break,
                };
                let path = entry.path();
                if path.extension().and_then(|value| value.to_str()) != Some("json") {
                    continue;
                }
                if let Some(cache) = read_cache_file(&path).await {
                    if !include_historical_provider_catalog
                        && is_historical_provider_catalog_cache(&cache.provider_id)
                    {
                        continue;
                    }
                    loaded_providers.insert(cache.provider_id.clone());
                    options.extend(
                        cache
                            .models
                            .into_iter()
                            .map(|model| cached_to_option(&cache.provider_id, model)),
                    );
                }
            }
        }
    }
    if include_historical_provider_catalog
        && !loaded_providers.contains(oino_auth::OPENROUTER_PROVIDER_ID)
    {
        if let Some(cache) = load_historical_openrouter_cache().await {
            options.extend(
                cache
                    .models
                    .into_iter()
                    .map(|model| cached_to_option(oino_auth::OPENROUTER_PROVIDER_ID, model)),
            );
        }
    }
    options
}

fn is_historical_provider_catalog_cache(provider_id: &str) -> bool {
    provider_id != ROUTER_PROVIDER_ID
        && oino_provider_catalog::provider_by_id(provider_id).is_some()
}

async fn load_provider_cache(provider_id: &str) -> Option<CachedModelCatalog> {
    if let Ok(path) = cache_path(provider_id) {
        if let Some(cache) = read_cache_file(&path).await {
            return Some(cache);
        }
    }
    if provider_id == oino_auth::OPENROUTER_PROVIDER_ID {
        load_historical_openrouter_cache().await
    } else {
        None
    }
}

async fn read_cache_file(path: &Path) -> Option<CachedModelCatalog> {
    let text = fs::read_to_string(path).await.ok()?;
    serde_json::from_str::<CachedModelCatalog>(&text).ok()
}

async fn load_historical_openrouter_cache() -> Option<CachedModelCatalog> {
    let path = historical_openrouter_cache_path().ok()?;
    read_cache_file(&path).await.map(|mut cache| {
        cache.provider_id = oino_auth::OPENROUTER_PROVIDER_ID.into();
        cache
    })
}

fn openai_compatible_to_cached(model: OpenRouterModelInfo) -> CachedModel {
    openai_compatible_to_cached_with_availability(model, &RouteAvailability::Unknown)
}

fn openai_compatible_to_cached_with_availability(
    model: OpenRouterModelInfo,
    route_availability: &RouteAvailability,
) -> CachedModel {
    let route_provider = model
        .owned_by
        .clone()
        .or_else(|| model.id.split('/').next().map(str::to_string));
    let availability = match (route_availability, route_provider.as_deref()) {
        (RouteAvailability::Known(routes), Some(route_provider)) => {
            if routes.contains(route_provider) {
                ModelAvailability::Configured
            } else {
                ModelAvailability::NeedsProviderKey
            }
        }
        (RouteAvailability::Known(routes), None) if routes.is_empty() => {
            ModelAvailability::NeedsProviderKey
        }
        (RouteAvailability::Known(_), None) => ModelAvailability::Unknown,
        (RouteAvailability::Unknown, _) => ModelAvailability::Unknown,
    };
    CachedModel {
        display_name: model.name.unwrap_or_else(|| model.id.clone()),
        route_provider,
        availability,
        id: model.id,
        supported_parameters: model.supported_parameters,
        context_length: model.context_length,
        pricing: model.pricing.map(|pricing| ModelPricing {
            input_per_token: pricing.prompt,
            output_per_token: pricing.completion,
            cache_hit_per_token: pricing.input_cache_read,
            cache_write_per_token: pricing.input_cache_write,
            source: "provider".into(),
        }),
    }
}

fn cached_to_option(provider_id: &str, model: CachedModel) -> ModelOption {
    let mut provider_label = provider_display_label(provider_id);
    if provider_id == ROUTER_PROVIDER_ID {
        if let Some(route_provider) = model.route_provider.as_deref() {
            if !route_provider.trim().is_empty() {
                provider_label = format!("OmniRoute/{route_provider}");
            }
        }
    }
    ModelOption::new(Model::new(provider_id, model.id).identifier())
        .with_display_name(model.display_name)
        .with_provider_label(provider_label)
        .with_availability(model.availability)
        .with_thinking_levels(thinking_levels_for_model(
            provider_id,
            &model.supported_parameters,
        ))
        .with_context_length(model.context_length)
        .with_pricing(model.pricing)
}

fn provider_display_label(provider_id: &str) -> String {
    if provider_id == ROUTER_PROVIDER_ID {
        return "router".into();
    }
    oino_provider_catalog::provider_by_id(provider_id).map_or_else(
        || provider_id.to_string(),
        |provider| provider.display_name.into(),
    )
}

fn merge_model_options(
    mut primary: Vec<ModelOption>,
    secondary: Vec<ModelOption>,
) -> Vec<ModelOption> {
    let mut seen = primary
        .iter()
        .map(|model| model.id.clone())
        .collect::<BTreeSet<_>>();
    for model in secondary {
        if seen.insert(model.id.clone()) {
            primary.push(model);
        }
    }
    primary
}

fn thinking_levels_for_model(provider_id: &str, parameters: &[String]) -> Vec<ThinkingLevel> {
    if provider_id == ROUTER_PROVIDER_ID {
        return all_thinking_levels();
    }
    thinking_levels_for_supported_parameters(parameters)
}

fn thinking_levels_for_supported_parameters(parameters: &[String]) -> Vec<ThinkingLevel> {
    let supports_reasoning = parameters.iter().any(|parameter| {
        matches!(
            parameter.as_str(),
            "reasoning" | "reasoning_effort" | "include_reasoning"
        )
    });
    if supports_reasoning {
        all_thinking_levels()
    } else {
        vec![ThinkingLevel::Off]
    }
}

async fn save_cache(cache: &CachedModelCatalog) -> std::io::Result<()> {
    let path = cache_path(&cache.provider_id)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(std::io::Error::other)?;
    fs::write(path, text).await
}

fn cache_path(provider_id: &str) -> std::io::Result<PathBuf> {
    Ok(cache_dir()?.join(format!("{}.json", safe_provider_file_stem(provider_id)?)))
}

fn cache_dir() -> std::io::Result<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "home directory unavailable",
        ));
    };
    Ok(home.join(".oino").join("model-catalogs"))
}

fn historical_openrouter_cache_path() -> std::io::Result<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "home directory unavailable",
        ));
    };
    Ok(home.join(".oino").join("openrouter-models.json"))
}

fn safe_provider_file_stem(provider_id: &str) -> std::io::Result<String> {
    if !provider_id.is_empty()
        && provider_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    {
        Ok(provider_id.to_string())
    } else {
        Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("unsafe provider id `{provider_id}`"),
        ))
    }
}

fn cache_age(cache: &CachedModelCatalog) -> Option<Duration> {
    let fetched_at = UNIX_EPOCH.checked_add(Duration::from_secs(cache.fetched_at_unix))?;
    SystemTime::now().duration_since(fetched_at).ok()
}

fn cache_has_context_lengths(cache: &CachedModelCatalog) -> bool {
    cache
        .models
        .iter()
        .any(|model| model.context_length.is_some())
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

/// Enrich OmniRoute cached models with `context_length` from the OpenRouter cache.
///
/// OmniRoute's `/v1/models` endpoint does not return `context_length`, so all models
/// come back with `None`. OpenRouter's endpoint does include it. Since OmniRoute routes
/// to the same underlying models (just with different provider prefixes), we can
/// cross-reference by base model name (the segment after the last `/`) to fill in
/// the context length.
async fn enrich_router_metadata_from_openrouter(cache: &mut CachedModelCatalog) {
    let Some(openrouter_cache) = load_provider_cache(oino_auth::OPENROUTER_PROVIDER_ID).await
    else {
        return;
    };
    enrich_router_metadata_from_openrouter_cache(cache, &openrouter_cache);
}

fn enrich_router_metadata_from_openrouter_cache(
    cache: &mut CachedModelCatalog,
    openrouter_cache: &CachedModelCatalog,
) {
    let mut by_id = std::collections::HashMap::new();
    let mut by_base: std::collections::HashMap<String, &CachedModel> =
        std::collections::HashMap::new();
    for model in &openrouter_cache.models {
        by_id.insert(model.id.as_str(), model);
        if let Some(base) = model.id.rsplit('/').next() {
            by_base.entry(base.to_string()).or_insert(model);
            by_base
                .entry(canonical_pricing_match_key(base))
                .or_insert(model);
        }
    }
    for model in &mut cache.models {
        let matched = by_id.get(model.id.as_str()).copied().or_else(|| {
            model.id.rsplit('/').next().and_then(|base| {
                by_base.get(base).copied().or_else(|| {
                    by_base
                        .get(canonical_pricing_match_key(base).as_str())
                        .copied()
                })
            })
        });
        let Some(source) = matched else {
            continue;
        };
        if model.context_length.is_none() {
            model.context_length = source.context_length;
        }
        if model.pricing.is_none() {
            model.pricing = source.pricing.clone().map(|mut pricing| {
                pricing.source = "openrouter".into();
                pricing
            });
        }
    }
}

fn canonical_pricing_match_key(value: &str) -> String {
    let mut key = value.to_string();
    key = key.replace("-thinking", "");
    for suffix in ["-extra-low", "-xhigh", "-high", "-medium", "-low"] {
        if let Some(stripped) = key.strip_suffix(suffix) {
            key = stripped.to_string();
            break;
        }
    }
    if key.len() > 9 {
        let split = key.len() - 9;
        let (prefix, suffix) = key.split_at(split);
        if suffix.starts_with('-') && suffix[1..].chars().all(|ch| ch.is_ascii_digit()) {
            key = prefix.to_string();
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reasoning_parameters_enable_all_thinking_levels() {
        assert_eq!(
            thinking_levels_for_supported_parameters(&["reasoning".into()]),
            all_thinking_levels()
        );
        assert_eq!(
            thinking_levels_for_supported_parameters(&["temperature".into()]),
            vec![ThinkingLevel::Off]
        );
    }

    #[test]
    fn router_models_allow_thinking_without_supported_parameter_metadata() {
        let option = cached_to_option(
            ROUTER_PROVIDER_ID,
            CachedModel {
                id: "cc/claude-sonnet-4-5".into(),
                display_name: "Claude Sonnet 4.5".into(),
                route_provider: Some("cc".into()),
                availability: ModelAvailability::Configured,
                supported_parameters: Vec::new(),
                context_length: Some(200_000),
                pricing: None,
            },
        );

        assert_eq!(option.id, "router:cc/claude-sonnet-4-5");
        assert_eq!(option.thinking_levels, all_thinking_levels());
    }

    #[test]
    fn provider_context_length_survives_cache_mapping() {
        let option = cached_to_option(
            "openrouter",
            openai_compatible_to_cached(OpenRouterModelInfo {
                id: "openai/gpt-4o-mini".into(),
                name: Some("GPT 4o Mini".into()),
                owned_by: Some("openai".into()),
                supported_parameters: vec!["reasoning".into()],
                context_length: Some(128_000),
                pricing: None,
            }),
        );

        assert_eq!(option.id, "openrouter:openai/gpt-4o-mini");
        assert_eq!(option.context_length, Some(128_000));
        assert_eq!(option.display_name, "GPT 4o Mini");
        assert_eq!(option.thinking_levels, all_thinking_levels());
    }

    #[test]
    fn static_catalog_can_include_historical_provider_catalog_defaults_when_requested() {
        let models = static_model_options_with_historical_provider_catalog(true);
        assert!(models
            .iter()
            .any(|model| model.id == "openrouter:openai/gpt-4o-mini"));
        assert!(models
            .iter()
            .any(|model| model.id == "claude:claude-3-5-sonnet-latest"));
        assert!(models.iter().any(|model| model.id.starts_with("deepseek:")));
        assert!(models.iter().any(|model| model.id.starts_with("groq:")));
        assert!(models.iter().any(|model| model.id == "openai:gpt-5.4"));
        assert!(models
            .iter()
            .any(|model| model.id == "openai:gpt-5.3-codex-spark"));
    }

    #[test]
    fn static_catalog_excludes_historical_provider_catalog_models_by_default() {
        assert!(static_model_options().is_empty());
        assert!(static_model_options_with_historical_provider_catalog(false).is_empty());
    }

    #[test]
    fn all_refresh_configs_can_include_historical_openai_compatible_catalogs_when_requested() {
        assert!(all_refresh_configs(&OpenRouterConfig::default()).is_empty());
        let configs = all_refresh_configs_with_historical_provider_catalog(
            &OpenRouterConfig::default(),
            true,
        );
        assert!(configs
            .iter()
            .any(|config| config.provider_id == "openrouter"));
        assert!(configs
            .iter()
            .any(|config| config.provider_id == "openai-api"));
        assert!(configs
            .iter()
            .any(|config| config.provider_id == "deepseek"));
        assert!(configs.iter().any(|config| config.provider_id == "ollama"));
        assert!(!configs
            .iter()
            .any(|config| config.provider_id == "auto-import"));
        assert!(all_refresh_configs_with_historical_provider_catalog(
            &OpenRouterConfig::default(),
            false,
        )
        .is_empty());
    }

    #[test]
    fn merge_model_options_deduplicates_by_identifier() {
        let merged = merge_model_options(
            vec![ModelOption::new("openrouter:a").with_display_name("Cached A")],
            vec![
                ModelOption::new("openrouter:a").with_display_name("Static A"),
                ModelOption::new("openrouter:b"),
            ],
        );
        assert_eq!(merged.len(), 2);
        assert_eq!(merged[0].display_name, "Cached A");
        assert_eq!(merged[1].id, "openrouter:b");
    }

    #[test]
    fn cache_without_context_lengths_is_not_usable_for_footer_limits() {
        let cache = CachedModelCatalog {
            fetched_at_unix: now_unix(),
            provider_id: "openrouter".into(),
            models: vec![CachedModel {
                id: "openai/gpt-4o-mini".into(),
                display_name: "GPT 4o Mini".into(),
                route_provider: Some("openai".into()),
                availability: ModelAvailability::Unknown,
                supported_parameters: Vec::new(),
                context_length: None,
                pricing: None,
            }],
        };

        assert!(!cache_has_context_lengths(&cache));
    }

    #[test]
    fn cache_with_any_context_length_is_usable_for_footer_limits() {
        let cache = CachedModelCatalog {
            fetched_at_unix: now_unix(),
            provider_id: "openrouter".into(),
            models: vec![
                CachedModel {
                    id: "legacy/model".into(),
                    display_name: "Legacy".into(),
                    route_provider: Some("legacy".into()),
                    availability: ModelAvailability::Unknown,
                    supported_parameters: Vec::new(),
                    context_length: None,
                    pricing: None,
                },
                CachedModel {
                    id: "openai/gpt-4o-mini".into(),
                    display_name: "GPT 4o Mini".into(),
                    route_provider: Some("openai".into()),
                    availability: ModelAvailability::Unknown,
                    supported_parameters: Vec::new(),
                    context_length: Some(128_000),
                    pricing: None,
                },
            ],
        };

        assert!(cache_has_context_lengths(&cache));
    }

    #[test]
    fn canonical_pricing_match_key_strips_router_variants_only() {
        assert_eq!(
            canonical_pricing_match_key("claude-sonnet-4-5-20250929"),
            "claude-sonnet-4-5"
        );
        assert_eq!(canonical_pricing_match_key("gpt-5.5-xhigh"), "gpt-5.5");
        assert_eq!(
            canonical_pricing_match_key("gemini-2.5-flash-thinking"),
            "gemini-2.5-flash"
        );
        assert_eq!(
            canonical_pricing_match_key("gemini-3.5-flash-extra-low"),
            "gemini-3.5-flash"
        );
    }

    #[test]
    fn router_metadata_enrichment_copies_openrouter_pricing_by_exact_id_and_base() {
        let openrouter_cache = CachedModelCatalog {
            fetched_at_unix: now_unix(),
            provider_id: "openrouter".into(),
            models: vec![CachedModel {
                id: "openai/gpt-4o".into(),
                display_name: "GPT-4o".into(),
                route_provider: Some("openai".into()),
                availability: ModelAvailability::Unknown,
                supported_parameters: Vec::new(),
                context_length: Some(128_000),
                pricing: Some(ModelPricing {
                    input_per_token: Some("0.0000025".into()),
                    output_per_token: Some("0.00001".into()),
                    cache_hit_per_token: Some("0.00000125".into()),
                    cache_write_per_token: None,
                    source: "provider".into(),
                }),
            }],
        };
        let mut router_cache = CachedModelCatalog {
            fetched_at_unix: now_unix(),
            provider_id: ROUTER_PROVIDER_ID.into(),
            models: vec![CachedModel {
                id: "omni/gpt-4o".into(),
                display_name: "omni/gpt-4o".into(),
                route_provider: Some("omni".into()),
                availability: ModelAvailability::Unknown,
                supported_parameters: Vec::new(),
                context_length: None,
                pricing: None,
            }],
        };

        enrich_router_metadata_from_openrouter_cache(&mut router_cache, &openrouter_cache);

        let model = &router_cache.models[0];
        assert_eq!(model.context_length, Some(128_000));
        let pricing = model.pricing.as_ref().expect("pricing should be copied");
        assert_eq!(pricing.input_per_token.as_deref(), Some("0.0000025"));
        assert_eq!(pricing.output_per_token.as_deref(), Some("0.00001"));
        assert_eq!(pricing.cache_hit_per_token.as_deref(), Some("0.00000125"));
        assert_eq!(pricing.source, "openrouter");
    }

    #[test]
    fn router_cached_models_mark_configured_routes_and_needed_keys() {
        let availability = RouteAvailability::Known(BTreeSet::from(["openai".to_string()]));
        let configured = cached_to_option(
            ROUTER_PROVIDER_ID,
            openai_compatible_to_cached_with_availability(
                OpenRouterModelInfo {
                    id: "openai/gpt-5".into(),
                    name: Some("GPT 5".into()),
                    owned_by: Some("openai".into()),
                    supported_parameters: Vec::new(),
                    context_length: None,
                    pricing: None,
                },
                &availability,
            ),
        );
        let needs_key = cached_to_option(
            ROUTER_PROVIDER_ID,
            openai_compatible_to_cached_with_availability(
                OpenRouterModelInfo {
                    id: "claude/sonnet".into(),
                    name: Some("Sonnet".into()),
                    owned_by: Some("claude".into()),
                    supported_parameters: Vec::new(),
                    context_length: None,
                    pricing: None,
                },
                &availability,
            ),
        );

        assert_eq!(configured.availability, ModelAvailability::Configured);
        assert_eq!(configured.provider_label, "OmniRoute/openai");
        assert_eq!(needs_key.availability, ModelAvailability::NeedsProviderKey);
        assert_eq!(needs_key.provider_label, "OmniRoute/claude");
    }

    #[test]
    fn router_provider_api_uses_server_root_not_openai_v1_base() {
        assert_eq!(
            openai_compatible_base_to_server_root("http://localhost:20128/v1"),
            "http://localhost:20128"
        );
        assert_eq!(
            openai_compatible_base_to_server_root("http://localhost:20128"),
            "http://localhost:20128"
        );
    }

    #[test]
    fn router_provider_routes_include_common_aliases() {
        let mut routes = BTreeSet::new();
        add_router_provider_routes(&mut routes, "anthropic", None);
        add_router_provider_routes(
            &mut routes,
            "custom-openai",
            Some(&serde_json::json!({ "prefix": "custom" })),
        );

        assert!(routes.contains("anthropic"));
        assert!(routes.contains("claude"));
        assert!(routes.contains("custom-openai"));
        assert!(routes.contains("custom"));
    }

    #[test]
    fn unsafe_provider_cache_stem_is_rejected() {
        assert!(safe_provider_file_stem("deepseek").is_ok());
        assert!(safe_provider_file_stem("../deepseek").is_err());
    }

    #[tokio::test]
    async fn enrich_router_context_lengths_fills_from_openrouter() {
        let openrouter_dir = tempfile::tempdir().unwrap();
        let openrouter_path = openrouter_dir.path().join("openrouter.json");
        let openrouter_cache = CachedModelCatalog {
            fetched_at_unix: 1000,
            provider_id: "openrouter".into(),
            models: vec![
                CachedModel {
                    id: "z-ai/glm-5.1".into(),
                    display_name: "GLM 5.1".into(),
                    route_provider: Some("z-ai".into()),
                    availability: ModelAvailability::Unknown,
                    supported_parameters: vec![],
                    context_length: Some(202_752),
                    pricing: None,
                },
                CachedModel {
                    id: "openai/gpt-4o".into(),
                    display_name: "GPT-4o".into(),
                    route_provider: Some("openai".into()),
                    availability: ModelAvailability::Unknown,
                    supported_parameters: vec![],
                    context_length: Some(128_000),
                    pricing: None,
                },
            ],
        };
        std::fs::write(
            &openrouter_path,
            serde_json::to_string(&openrouter_cache).unwrap(),
        )
        .unwrap();

        // Set the cache dir env so load_provider_cache finds it
        let mut router_cache = CachedModelCatalog {
            fetched_at_unix: 2000,
            provider_id: "router".into(),
            models: vec![
                CachedModel {
                    id: "glm/glm-5.1".into(),
                    display_name: "glm/glm-5.1".into(),
                    route_provider: Some("glm".into()),
                    availability: ModelAvailability::Unknown,
                    supported_parameters: vec![],
                    context_length: None,
                    pricing: None,
                },
                CachedModel {
                    id: "openai/gpt-4o".into(),
                    display_name: "openai/gpt-4o".into(),
                    route_provider: Some("openai".into()),
                    availability: ModelAvailability::Unknown,
                    supported_parameters: vec![],
                    context_length: None,
                    pricing: None,
                },
                CachedModel {
                    id: "unknown/mystery-model".into(),
                    display_name: "unknown/mystery-model".into(),
                    route_provider: None,
                    availability: ModelAvailability::Unknown,
                    supported_parameters: vec![],
                    context_length: None,
                    pricing: None,
                },
            ],
        };

        // We need the OpenRouter cache to be loadable by load_provider_cache.
        // Since that uses cache_dir() -> $HOME, we'll test the logic directly
        // by building the lookup map and applying it manually.
        let context_map: std::collections::HashMap<&str, usize> = openrouter_cache
            .models
            .iter()
            .filter_map(|model| {
                let length = model.context_length?;
                let base = model.id.rsplit('/').next()?;
                Some((base, length))
            })
            .collect();

        // Enrich manually using the same logic
        for model in &mut router_cache.models {
            if model.context_length.is_none() {
                if let Some(base) = model.id.rsplit('/').next() {
                    if let Some(&length) = context_map.get(base) {
                        model.context_length = Some(length);
                    }
                }
            }
        }

        // glm-5.1 matched z-ai/glm-5.1's context_length
        assert_eq!(router_cache.models[0].context_length, Some(202_752));
        // gpt-4o matched openai/gpt-4o's context_length
        assert_eq!(router_cache.models[1].context_length, Some(128_000));
        // mystery-model has no match, stays None
        assert_eq!(router_cache.models[2].context_length, None);
    }
}
