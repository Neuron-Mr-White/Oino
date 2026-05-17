#![forbid(unsafe_code)]

use oino_provider_openrouter::{list_models, OpenRouterConfig, OpenRouterModelInfo};
use oino_tui::{all_thinking_levels, ModelOption};
use oino_types::{Model, ThinkingLevel};
use serde::{Deserialize, Serialize};
use std::{
    path::PathBuf,
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::fs;

pub const MODEL_REFRESH_INTERVAL: Duration = Duration::from_secs(60 * 60 * 6);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelCatalogUpdate {
    pub models: Vec<ModelOption>,
    pub status: String,
    pub refreshing: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedModelCatalog {
    fetched_at_unix: u64,
    models: Vec<CachedModel>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CachedModel {
    id: String,
    display_name: String,
    supported_parameters: Vec<String>,
}

pub async fn load_cached_update() -> Option<ModelCatalogUpdate> {
    let path = cache_path().ok()?;
    let text = fs::read_to_string(path).await.ok()?;
    let cache = serde_json::from_str::<CachedModelCatalog>(&text).ok()?;
    let age = cache_age(&cache);
    let models = cache.models.into_iter().map(cached_to_option).collect();
    Some(ModelCatalogUpdate {
        models,
        status: format!("Loaded cached models{}", age_status(age)),
        refreshing: false,
    })
}

pub async fn refresh_update(config: &OpenRouterConfig) -> ModelCatalogUpdate {
    match list_models(config).await {
        Ok(models) => {
            let cached = CachedModelCatalog {
                fetched_at_unix: now_unix(),
                models: models.into_iter().map(openrouter_to_cached).collect(),
            };
            let count = cached.models.len();
            let options = cached
                .models
                .iter()
                .cloned()
                .map(cached_to_option)
                .collect::<Vec<_>>();
            let save_status = save_cache(&cached).await.map_or_else(
                |err| format!("cache save failed: {err}"),
                |_| "cached".into(),
            );
            ModelCatalogUpdate {
                models: options,
                status: format!("Fetched {count} OpenRouter models ({save_status})"),
                refreshing: false,
            }
        }
        Err(err) => ModelCatalogUpdate {
            models: Vec::new(),
            status: format!("Model refresh failed: {err}"),
            refreshing: false,
        },
    }
}

pub async fn cached_is_fresh() -> bool {
    let Some(path) = cache_path().ok() else {
        return false;
    };
    let Ok(text) = fs::read_to_string(path).await else {
        return false;
    };
    let Ok(cache) = serde_json::from_str::<CachedModelCatalog>(&text) else {
        return false;
    };
    cache_age(&cache).is_some_and(|age| age < MODEL_REFRESH_INTERVAL)
}

fn openrouter_to_cached(model: OpenRouterModelInfo) -> CachedModel {
    CachedModel {
        display_name: model.name.unwrap_or_else(|| model.id.clone()),
        id: model.id,
        supported_parameters: model.supported_parameters,
    }
}

fn cached_to_option(model: CachedModel) -> ModelOption {
    ModelOption::new(Model::new("openrouter", model.id).identifier())
        .with_display_name(model.display_name)
        .with_thinking_levels(thinking_levels_for_supported_parameters(
            &model.supported_parameters,
        ))
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
    let path = cache_path()?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).await?;
    }
    let text = serde_json::to_string_pretty(cache).map_err(std::io::Error::other)?;
    fs::write(path, text).await
}

fn cache_path() -> std::io::Result<PathBuf> {
    let Some(home) = dirs::home_dir() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "home directory unavailable",
        ));
    };
    Ok(home.join(".oino").join("openrouter-models.json"))
}

fn age_status(age: Option<Duration>) -> String {
    match age {
        Some(age) if age < MODEL_REFRESH_INTERVAL => " (fresh)".into(),
        Some(_) => " (stale; refresh queued)".into(),
        None => String::new(),
    }
}

fn cache_age(cache: &CachedModelCatalog) -> Option<Duration> {
    let fetched_at = UNIX_EPOCH.checked_add(Duration::from_secs(cache.fetched_at_unix))?;
    SystemTime::now().duration_since(fetched_at).ok()
}

fn now_unix() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
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
}
