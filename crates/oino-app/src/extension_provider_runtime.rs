use std::{fs, path::PathBuf, time::Duration};

use oino_agent_loop::LoopError;
use oino_auth::ProviderAuthSpec;
use oino_extension_core::{
    ProviderContribution, ProviderRuntimeContribution, ProviderRuntimeModelIdPolicy,
    ProviderRuntimeProtocol, ProviderRuntimeSecret,
};
use oino_extension_manager::ExtensionManagerSnapshot;
use oino_provider_openrouter::{OpenAiCompatibleAuth, OpenAiCompatibleConfig};

pub(crate) fn extension_runtime_providers(
    snapshot: &ExtensionManagerSnapshot,
) -> Vec<ProviderContribution> {
    snapshot
        .registries
        .providers
        .active
        .iter()
        .filter(|active| active.entry.contribution.runtime.is_some())
        .map(|active| active.entry.contribution.clone())
        .collect()
}

pub(crate) fn extension_runtime_config_for_provider_id(
    provider_id: &str,
    extension_providers: &[ProviderContribution],
) -> Result<Option<OpenAiCompatibleConfig>, LoopError> {
    let Some(provider) = extension_providers
        .iter()
        .find(|provider| provider.provider_id == provider_id)
    else {
        return Ok(None);
    };
    let Some(runtime) = &provider.runtime else {
        return Ok(None);
    };
    if runtime.protocol != ProviderRuntimeProtocol::OpenAiChatCompletions {
        return Err(LoopError::Stream(format!(
            "extension provider `{provider_id}` uses an unsupported runtime protocol"
        )));
    }
    if runtime.model_id == ProviderRuntimeModelIdPolicy::PreserveFullIdentifier {
        return Err(LoopError::Stream(format!(
            "extension provider `{provider_id}` requests full model identifiers, which are not wired yet"
        )));
    }

    let display_name = if provider.display_name.trim().is_empty() {
        provider.provider_id.clone()
    } else {
        provider.display_name.clone()
    };
    let base_url = resolved_extension_runtime_base_url(&provider.provider_id, runtime)
        .map_err(LoopError::Stream)?;
    let mut config =
        OpenAiCompatibleConfig::new(provider.provider_id.clone(), display_name, base_url);
    config.auth = match &runtime.api_key {
        ProviderRuntimeSecret::None => OpenAiCompatibleAuth::None,
        ProviderRuntimeSecret::EnvVar { name } => OpenAiCompatibleAuth::OptionalBearer {
            spec: ProviderAuthSpec::new(
                provider.provider_id.clone(),
                provider.provider_id.clone(),
                name.clone(),
            ),
        },
        ProviderRuntimeSecret::ExtensionConfig { key } => {
            let secret = extension_config_string(&provider.provider_id, key)
                .map_err(LoopError::Stream)?
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(|| {
                    LoopError::Stream(format!(
                        "extension provider `{provider_id}` references extension config secret `{key}`, but no non-empty value was found in {}",
                        extension_config_path_display(&provider.provider_id)
                    ))
                })?;
            config = config.with_header("Authorization", format!("Bearer {secret}"));
            OpenAiCompatibleAuth::None
        }
    };
    for (name, value) in &runtime.headers {
        config = config.with_header(name.clone(), value.clone());
    }
    Ok(Some(config))
}

pub(crate) fn resolved_extension_runtime_base_url(
    provider_id: &str,
    runtime: &ProviderRuntimeContribution,
) -> Result<String, String> {
    for env_name in extension_runtime_base_url_env_candidates_for_runtime(provider_id, runtime) {
        if let Ok(value) = std::env::var(&env_name) {
            let value = value.trim();
            if !value.is_empty() {
                return Ok(value.into());
            }
        }
    }
    let base_url_key = runtime.config.base_url_key.as_deref().unwrap_or("base_url");
    if let Some(value) = extension_config_string(provider_id, base_url_key)? {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.into());
        }
    }
    Ok(runtime.base_url.clone())
}

#[cfg(test)]
pub(crate) fn extension_runtime_base_url_env_candidates(provider_id: &str) -> Vec<String> {
    extension_runtime_url_env_candidates(provider_id, "BASE_URL")
}

fn extension_runtime_base_url_env_candidates_for_runtime(
    provider_id: &str,
    runtime: &ProviderRuntimeContribution,
) -> Vec<String> {
    extension_runtime_env_candidates_with_overrides(
        provider_id,
        "BASE_URL",
        &runtime.config.base_url_env,
    )
}

fn extension_runtime_health_url_env_candidates_for_runtime(
    provider_id: &str,
    runtime: &ProviderRuntimeContribution,
) -> Vec<String> {
    extension_runtime_env_candidates_with_overrides(
        provider_id,
        "HEALTH_URL",
        &runtime.config.health_url_env,
    )
}

fn extension_runtime_env_candidates_with_overrides(
    provider_id: &str,
    suffix: &str,
    overrides: &[String],
) -> Vec<String> {
    let mut candidates = overrides
        .iter()
        .map(|name| name.trim())
        .filter(|name| !name.is_empty())
        .map(ToString::to_string)
        .collect::<Vec<_>>();
    for candidate in extension_runtime_url_env_candidates(provider_id, suffix) {
        if !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
    candidates
}

pub(crate) fn extension_runtime_url_env_candidates(provider_id: &str, suffix: &str) -> Vec<String> {
    let mut candidates = Vec::new();
    if provider_id == "9router" {
        candidates.push(format!("NINEROUTER_{suffix}"));
    }
    let prefix = provider_id
        .chars()
        .map(|ch| {
            if ch.is_ascii_alphanumeric() {
                ch.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect::<String>();
    let generic = format!("{prefix}_{suffix}");
    if !candidates.contains(&generic) {
        candidates.push(generic);
    }
    candidates
}

pub(crate) fn extension_config_string(
    provider_id: &str,
    key: &str,
) -> Result<Option<String>, String> {
    let path = extension_config_path(provider_id)?;
    if !path.exists() {
        return Ok(None);
    }
    let text = fs::read_to_string(&path)
        .map_err(|err| format!("could not read extension config {}: {err}", path.display()))?;
    let value = serde_json::from_str::<serde_json::Value>(&text)
        .map_err(|err| format!("invalid extension config JSON at {}: {err}", path.display()))?;
    Ok(extension_config_string_from_value(&value, key))
}

pub(crate) fn extension_config_string_from_value(
    value: &serde_json::Value,
    key: &str,
) -> Option<String> {
    let mut current = value;
    for part in key.split('.').filter(|part| !part.is_empty()) {
        current = current.get(part)?;
    }
    current.as_str().map(ToString::to_string)
}

pub(crate) fn extension_config_path(provider_id: &str) -> Result<PathBuf, String> {
    let Some(home) = dirs::home_dir() else {
        return Err("home directory unavailable for extension config".into());
    };
    Ok(home
        .join(".oino/extensions")
        .join(extension_config_dir_name(provider_id)?)
        .join("config.json"))
}

pub(crate) fn extension_config_path_display(provider_id: &str) -> String {
    extension_config_path(provider_id)
        .map(|path| path.display().to_string())
        .unwrap_or_else(|_| format!("~/.oino/extensions/{provider_id}/config.json"))
}

pub(crate) fn extension_config_dir_name(provider_id: &str) -> Result<String, String> {
    let provider_id = provider_id.trim();
    if provider_id.is_empty()
        || matches!(provider_id, "." | "..")
        || provider_id.contains("..")
        || !provider_id
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(format!(
            "invalid extension provider id `{provider_id}` for config path"
        ));
    }
    Ok(provider_id.into())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExtensionRuntimeHealth {
    pub(crate) url: String,
    pub(crate) reachable: bool,
    pub(crate) status: Option<String>,
    pub(crate) model_count: Option<usize>,
    pub(crate) error: Option<String>,
}

pub(crate) async fn check_extension_runtime_health(
    provider_id: &str,
    runtime: &ProviderRuntimeContribution,
) -> ExtensionRuntimeHealth {
    let url = match resolved_extension_runtime_health_url(provider_id, runtime) {
        Ok(url) => url,
        Err(err) => {
            return ExtensionRuntimeHealth {
                url: runtime
                    .health_url
                    .clone()
                    .or_else(|| runtime.models_url.clone())
                    .unwrap_or_else(|| runtime.base_url.clone()),
                reachable: false,
                status: None,
                model_count: None,
                error: Some(err),
            }
        }
    };
    let timeout = Duration::from_millis(1_500);
    let client = match reqwest::Client::builder().timeout(timeout).build() {
        Ok(client) => client,
        Err(err) => {
            return ExtensionRuntimeHealth {
                url,
                reachable: false,
                status: None,
                model_count: None,
                error: Some(err.to_string()),
            }
        }
    };
    let mut request = client.get(&url);
    match extension_runtime_auth_bearer(provider_id, &runtime.api_key) {
        Ok(Some(secret)) => request = request.bearer_auth(secret),
        Ok(None) => {}
        Err(err) => {
            return ExtensionRuntimeHealth {
                url,
                reachable: false,
                status: None,
                model_count: None,
                error: Some(err),
            }
        }
    }
    for (name, value) in &runtime.headers {
        request = request.header(name, value);
    }
    let response = match request.send().await {
        Ok(response) => response,
        Err(err) => {
            return ExtensionRuntimeHealth {
                url,
                reachable: false,
                status: None,
                model_count: None,
                error: Some(err.to_string()),
            }
        }
    };
    let status = response.status().to_string();
    if !response.status().is_success() {
        return ExtensionRuntimeHealth {
            url,
            reachable: false,
            status: Some(status.clone()),
            model_count: None,
            error: Some(format!("health endpoint returned {status}")),
        };
    }
    let model_count = response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|body| openai_model_count(&body));
    ExtensionRuntimeHealth {
        url,
        reachable: true,
        status: Some(status),
        model_count,
        error: None,
    }
}

pub(crate) fn resolved_extension_runtime_health_url(
    provider_id: &str,
    runtime: &ProviderRuntimeContribution,
) -> Result<String, String> {
    for env_name in extension_runtime_health_url_env_candidates_for_runtime(provider_id, runtime) {
        if let Ok(value) = std::env::var(&env_name) {
            let value = value.trim();
            if !value.is_empty() {
                return Ok(value.into());
            }
        }
    }
    let health_url_key = runtime
        .config
        .health_url_key
        .as_deref()
        .unwrap_or("health_url");
    if let Some(value) = extension_config_string(provider_id, health_url_key)? {
        let value = value.trim();
        if !value.is_empty() {
            return Ok(value.into());
        }
    }
    let base_url = resolved_extension_runtime_base_url(provider_id, runtime)?;
    if base_url.trim_end_matches('/') == runtime.base_url.trim_end_matches('/') {
        if let Some(health_url) = &runtime.health_url {
            return Ok(health_url.clone());
        }
        if let Some(models_url) = &runtime.models_url {
            return Ok(models_url.clone());
        }
    }
    Ok(extension_runtime_url_from_base(&base_url, "models"))
}

pub(crate) fn extension_runtime_url_from_base(base_url: &str, path: &str) -> String {
    format!(
        "{}/{}",
        base_url.trim_end_matches('/'),
        path.trim_start_matches('/')
    )
}

pub(crate) fn extension_runtime_auth_bearer(
    provider_id: &str,
    secret: &ProviderRuntimeSecret,
) -> Result<Option<String>, String> {
    match secret {
        ProviderRuntimeSecret::None => Ok(None),
        ProviderRuntimeSecret::EnvVar { name } => Ok(std::env::var(name)
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty())),
        ProviderRuntimeSecret::ExtensionConfig { key } => {
            Ok(extension_config_string(provider_id, key)?
                .map(|value| value.trim().to_string())
                .filter(|value| !value.is_empty()))
        }
    }
}

pub(crate) fn format_extension_runtime_health_detail(health: &ExtensionRuntimeHealth) -> String {
    if health.reachable {
        format!(
            "Live runtime health: reachable at {}{}.",
            health.url,
            health
                .model_count
                .map(|count| format!(" · {count} models"))
                .unwrap_or_default()
        )
    } else {
        format!(
            "Live runtime health: not reachable at {}{}.",
            health.url,
            health
                .error
                .as_deref()
                .map(|error| format!(" ({error})"))
                .or_else(|| health
                    .status
                    .as_deref()
                    .map(|status| format!(" ({status})")))
                .unwrap_or_default()
        )
    }
}

fn openai_model_count(body: &serde_json::Value) -> Option<usize> {
    body.get("data")?.as_array().map(Vec::len)
}
