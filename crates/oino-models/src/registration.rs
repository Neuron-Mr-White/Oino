//! Provider registrations (extension-api §4.4) and config-only provider overrides
//! (pi `registerProvider(name, config)`).

use indexmap::IndexMap;
use oino_ai::{InputLimits, InputModality, JsonObject, JsonValue, Model, ModelCost};
use oino_ai::{PromptCache, ThinkingLevelMap};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::models_json::{SamplingByLevel, sampling_by_level};

/// How a registration sends the API key (`authHeader`).
///
/// pi uses a boolean; extension-api §4.4 uses `"bearer"`. Both spellings are accepted. Any
/// other header name (e.g. Anthropic's `"x-api-key"`) is informational: the provider
/// component places the key itself, so it maps to [`AuthHeaderMode::None`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum AuthHeaderMode {
    /// The provider component places the key itself.
    #[default]
    None,
    /// `Authorization: Bearer <key>`.
    Bearer,
}

impl Serialize for AuthHeaderMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::None => serializer.serialize_str("none"),
            Self::Bearer => serializer.serialize_str("bearer"),
        }
    }
}

impl<'de> Deserialize<'de> for AuthHeaderMode {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Flag(bool),
            Name(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Flag(true) => Ok(Self::Bearer),
            Raw::Name(name) if name.eq_ignore_ascii_case("bearer") => Ok(Self::Bearer),
            Raw::Flag(false) | Raw::Name(_) => Ok(Self::None),
        }
    }
}

/// A model in a registration or override (pi `ProviderChatModelConfig`).
///
/// pi's TypeScript type requires most fields; Oino applies the models.json defaults when
/// they are missing (`name = id`, `input = ["text"]`, zero cost, 128 000 / 16 384 tokens).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderModelConfig {
    /// Model id.
    pub id: String,
    /// Display name (default: `id`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// API (default: the provider's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// Base URL (default: the provider's).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Thinking support.
    #[serde(default)]
    pub reasoning: bool,
    /// Provider value per thinking level.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thinking_level_map: Option<ThinkingLevelMap>,
    /// Input modalities.
    #[serde(default = "default_input")]
    pub input: Vec<InputModality>,
    /// Request/image limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_limits: Option<InputLimits>,
    /// Pricing.
    #[serde(default)]
    pub cost: ModelCost,
    /// Prompt cache lifetimes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_cache: Option<PromptCache>,
    /// Context window.
    #[serde(default = "default_context_window")]
    pub context_window: u64,
    /// Maximum output tokens.
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u64,
    /// Sampling defaults.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sampling_params: Option<JsonObject>,
    /// Sampling per thinking level.
    #[serde(
        default,
        deserialize_with = "sampling_by_level",
        skip_serializing_if = "Option::is_none"
    )]
    pub sampling_params_by_thinking_level: Option<SamplingByLevel>,
    /// Model headers. On a registration they stay on the model; on an override they are
    /// request-time headers (pi `rawModelHeaders`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<IndexMap<String, String>>,
    /// Compatibility flags.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<JsonValue>,
}

fn default_input() -> Vec<InputModality> {
    vec![InputModality::Text]
}

/// models.json default context window.
pub(crate) const DEFAULT_CONTEXT_WINDOW: u64 = 128_000;
/// models.json default output limit.
pub(crate) const DEFAULT_MAX_TOKENS: u64 = 16_384;

fn default_context_window() -> u64 {
    DEFAULT_CONTEXT_WINDOW
}

fn default_max_tokens() -> u64 {
    DEFAULT_MAX_TOKENS
}

impl ProviderModelConfig {
    /// Builds the catalog model with resolved `provider`, `api` and `baseUrl`.
    #[must_use]
    pub fn into_model(self, provider: &str, api: String, base_url: String) -> Model {
        Model {
            name: self.name.unwrap_or_else(|| self.id.clone()),
            id: self.id,
            api,
            provider: provider.to_owned(),
            base_url,
            input: self.input,
            input_limits: self.input_limits,
            cost: self.cost,
            headers: self.headers,
            kind: None,
            reasoning: self.reasoning,
            thinking_level_map: self.thinking_level_map,
            prompt_cache: self.prompt_cache,
            context_window: self.context_window,
            max_tokens: self.max_tokens,
            sampling_params: self.sampling_params,
            sampling_params_by_thinking_level: self.sampling_params_by_thinking_level,
            compat: self.compat,
        }
    }
}

/// Model list of a registration: `"dynamic"` (host calls the component's `list-models`) or a
/// static list.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum ProviderModels {
    /// Models come from `list-models` ([`crate::ModelRegistry::set_provider_models`]).
    #[default]
    Dynamic,
    /// Static models.
    Static(Vec<ProviderModelConfig>),
}

impl Serialize for ProviderModels {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Dynamic => serializer.serialize_str("dynamic"),
            Self::Static(models) => models.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for ProviderModels {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Name(String),
            List(Vec<ProviderModelConfig>),
        }
        match Raw::deserialize(deserializer)? {
            Raw::Name(name) if name == "dynamic" => Ok(Self::Dynamic),
            Raw::Name(name) => Err(serde::de::Error::custom(format!(
                "models must be \"dynamic\" or an array, got \"{name}\""
            ))),
            Raw::List(models) => Ok(Self::Static(models)),
        }
    }
}

/// A provider component's registration (extension-api §4.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRegistration {
    /// Provider id (`openrouter`).
    pub id: String,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// API the component implements for this provider.
    pub api: String,
    /// Default base URL of the provider's models.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// Environment variables holding an API key, in lookup order.
    #[serde(default)]
    pub auth_env: Vec<String>,
    /// Informational for the component (see crate docs: core only injects
    /// `Authorization` for models.json / override `authHeader: true`, like pi).
    #[serde(default)]
    pub auth_header: AuthHeaderMode,
    /// The component supports OAuth login/refresh.
    #[serde(default)]
    pub oauth: bool,
    /// Provider headers used by the component.
    #[serde(default, skip_serializing_if = "IndexMap::is_empty")]
    pub headers: IndexMap<String, String>,
    /// Provider compatibility flags applied to every model.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub compat: Option<JsonValue>,
    /// Model list.
    #[serde(default)]
    pub models: ProviderModels,
}

impl ProviderRegistration {
    /// Display name (`name`, else `id`).
    #[must_use]
    pub fn display_name(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }

    /// Whether the provider offers API-key auth: every registration does unless it is
    /// OAuth-only (`oauth: true` without `authEnv`).
    #[must_use]
    pub fn has_api_key_auth(&self) -> bool {
        !(self.oauth && self.auth_env.is_empty())
    }
}

/// pi `ProviderConfigInput` minus code: config-only overrides an extension sets for an
/// existing provider id. Re-registration merges defined fields over the previous override.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderConfigOverride {
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Base URL for every model (or the default for `models`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,
    /// API key config value (`literal | $ENV | !command`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    /// Default API for `models`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api: Option<String>,
    /// Request headers (values resolve like `apiKey`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<IndexMap<String, String>>,
    /// Send the resolved key as `Authorization: Bearer`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_header: Option<bool>,
    /// Replaces the provider's model list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub models: Option<Vec<ProviderModelConfig>>,
}

impl ProviderConfigOverride {
    /// pi re-registration: defined values of `next` replace those of `self`.
    #[must_use]
    pub fn merged_with(self, next: Self) -> Self {
        Self {
            name: next.name.or(self.name),
            base_url: next.base_url.or(self.base_url),
            api_key: next.api_key.or(self.api_key),
            api: next.api.or(self.api),
            headers: next.headers.or(self.headers),
            auth_header: next.auth_header.or(self.auth_header),
            models: next.models.or(self.models),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn parses_extension_api_shape() {
        let registration: ProviderRegistration = serde_json::from_value(json!({
            "id": "openrouter", "name": "OpenRouter", "api": "openai-completions",
            "baseUrl": "https://openrouter.ai/api/v1", "authEnv": ["OPENROUTER_API_KEY"],
            "authHeader": "bearer", "oauth": false, "headers": {}, "compat": {},
            "models": "dynamic"
        }))
        .unwrap();
        assert_eq!(registration.models, ProviderModels::Dynamic);
        assert_eq!(registration.auth_header, AuthHeaderMode::Bearer);
        assert!(registration.has_api_key_auth());

        let registration: ProviderRegistration = serde_json::from_value(json!({
            "id": "anthropic", "api": "anthropic-messages", "authHeader": "x-api-key",
            "models": "dynamic"
        }))
        .unwrap();
        assert_eq!(registration.auth_header, AuthHeaderMode::None);

        let registration: ProviderRegistration = serde_json::from_value(json!({
            "id": "x", "api": "a", "oauth": true, "authHeader": true,
            "models": [{"id": "m"}]
        }))
        .unwrap();
        assert!(!registration.has_api_key_auth());
        let ProviderModels::Static(models) = registration.models else {
            panic!("static")
        };
        assert_eq!(models[0].context_window, 128_000);
        assert_eq!(models[0].input, vec![InputModality::Text]);
    }

    #[test]
    fn override_merge_keeps_undefined() {
        let first = ProviderConfigOverride {
            base_url: Some("a".into()),
            api_key: Some("k".into()),
            ..Default::default()
        };
        let merged = first.merged_with(ProviderConfigOverride {
            base_url: Some("b".into()),
            ..Default::default()
        });
        assert_eq!(merged.base_url.as_deref(), Some("b"));
        assert_eq!(merged.api_key.as_deref(), Some("k"));
    }
}
