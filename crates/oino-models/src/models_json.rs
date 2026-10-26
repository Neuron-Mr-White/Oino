//! pi `model-config.ts`: one immutable, credential-blind load of `models.json`.
//!
//! Load problems never fail hard: like pi, they become a diagnostic string
//! ([`ModelsJson::error`]) and the file contributes no providers.

use std::path::{Path, PathBuf};

use indexmap::IndexMap;
use oino_ai::{InputLimits, InputModality, JsonObject, JsonValue, ModelCost, ModelCostTier};
use oino_ai::{PromptCache, ThinkingLevel, ThinkingLevelMap};
use serde::{Deserialize, Deserializer};

use crate::json_text::{strip_bom, strip_json_comments};
use crate::schema::{MODELS_CONFIG, validate};

/// `samplingParamsByThinkingLevel`: known pi levels only (unknown keys are ignored like pi).
pub type SamplingByLevel = IndexMap<ThinkingLevel, JsonObject>;

/// Deserializes `samplingParamsByThinkingLevel`, ignoring keys that are not pi levels.
pub(crate) fn sampling_by_level<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<SamplingByLevel>, D::Error> {
    let raw = Option::<IndexMap<String, JsonValue>>::deserialize(deserializer)?;
    Ok(raw.map(|raw| {
        raw.into_iter()
            .filter_map(|(key, value)| {
                let level = key.parse::<ThinkingLevel>().ok()?;
                match value {
                    JsonValue::Object(object) => Some((level, object)),
                    _ => None,
                }
            })
            .collect()
    }))
}

/// A `models[]` entry (pi `ModelsJsonModel`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsJsonModel {
    /// Model id sent to the API.
    pub id: String,
    /// Display name (default: `id`).
    pub name: Option<String>,
    /// Overrides the provider `api`.
    pub api: Option<String>,
    /// Overrides the provider `baseUrl`.
    pub base_url: Option<String>,
    /// Thinking support (default `false`).
    pub reasoning: Option<bool>,
    /// Provider value per thinking level.
    pub thinking_level_map: Option<ThinkingLevelMap>,
    /// Input modalities (default `["text"]`).
    pub input: Option<Vec<InputModality>>,
    /// Request/image limits.
    pub input_limits: Option<InputLimits>,
    /// Pricing (default all zero).
    pub cost: Option<ModelCost>,
    /// Prompt cache lifetimes.
    pub prompt_cache: Option<PromptCache>,
    /// Context window (default 128 000).
    pub context_window: Option<f64>,
    /// Maximum output tokens (default 16 384).
    pub max_tokens: Option<f64>,
    /// Sampling defaults.
    pub sampling_params: Option<JsonObject>,
    /// Sampling per thinking level.
    #[serde(default, deserialize_with = "sampling_by_level")]
    pub sampling_params_by_thinking_level: Option<SamplingByLevel>,
    /// Request headers (values resolve like `apiKey`).
    pub headers: Option<IndexMap<String, String>>,
    /// Compatibility flags (opaque to core).
    pub compat: Option<JsonValue>,
}

/// Partial cost in a model override.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PartialCost {
    /// Input price.
    pub input: Option<f64>,
    /// Output price.
    pub output: Option<f64>,
    /// Cache-read price.
    pub cache_read: Option<f64>,
    /// Cache-write price.
    pub cache_write: Option<f64>,
    /// Pricing tiers (replace the base tiers).
    pub tiers: Option<Vec<ModelCostTier>>,
}

/// A `modelOverrides` entry (pi `ModelsJsonModelOverride`): every field merges over the
/// existing model.
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsJsonModelOverride {
    /// Display name.
    pub name: Option<String>,
    /// Thinking support.
    pub reasoning: Option<bool>,
    /// Merged per level.
    pub thinking_level_map: Option<ThinkingLevelMap>,
    /// Replaces the modalities.
    pub input: Option<Vec<InputModality>>,
    /// Merged field-wise.
    pub input_limits: Option<InputLimits>,
    /// Merged field-wise.
    pub cost: Option<PartialCost>,
    /// Merged field-wise.
    pub prompt_cache: Option<PromptCache>,
    /// Context window.
    pub context_window: Option<f64>,
    /// Maximum output tokens.
    pub max_tokens: Option<f64>,
    /// Merged per key.
    pub sampling_params: Option<JsonObject>,
    /// Merged per level and key.
    #[serde(default, deserialize_with = "sampling_by_level")]
    pub sampling_params_by_thinking_level: Option<SamplingByLevel>,
    /// Request-time headers for this model.
    pub headers: Option<IndexMap<String, String>>,
    /// Merged like provider compat.
    pub compat: Option<JsonValue>,
}

/// A `providers.<id>` entry (pi `ModelsJsonProvider`).
#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelsJsonProvider {
    /// Display name.
    pub name: Option<String>,
    /// API base URL; also applies to the provider's registered models.
    pub base_url: Option<String>,
    /// `literal | $ENV | ${ENV} | !command`.
    pub api_key: Option<String>,
    /// API of the provider's models.
    pub api: Option<String>,
    /// pi's `"radius"` OAuth flow marker (validated, no flow in core).
    pub oauth: Option<String>,
    /// Request headers (values resolve like `apiKey`).
    pub headers: Option<IndexMap<String, String>>,
    /// Compatibility flags.
    pub compat: Option<JsonValue>,
    /// Send the resolved key as `Authorization: Bearer` (default `false`).
    pub auth_header: Option<bool>,
    /// Added or replaced models.
    pub models: Option<Vec<ModelsJsonModel>>,
    /// Metadata overrides keyed by model id.
    pub model_overrides: Option<IndexMap<String, ModelsJsonModelOverride>>,
}

#[derive(Deserialize)]
struct RawConfig {
    providers: IndexMap<String, ModelsJsonProvider>,
}

/// One load of `models.json` (pi `ModelConfig`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ModelsJson {
    providers: IndexMap<String, ModelsJsonProvider>,
    error: Option<String>,
}

impl ModelsJson {
    /// An empty configuration (no file).
    #[must_use]
    pub fn empty() -> Self {
        Self::default()
    }

    /// pi `ModelConfig.load`: a missing file is empty; read, parse and schema errors become
    /// [`ModelsJson::error`] with pi's texts.
    #[must_use]
    pub fn load(path: Option<&Path>) -> Self {
        let Some(path) = path else {
            return Self::empty();
        };
        match std::fs::read_to_string(path) {
            Ok(content) => Self::parse_at(&content, path),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::empty(),
            Err(error) => Self::failed(format!(
                "Failed to load models.json: {error}\n\nFile: {}",
                path.display()
            )),
        }
    }

    /// Parses `content` as if read from `path` (used for diagnostics).
    #[must_use]
    pub fn parse_at(content: &str, path: &Path) -> Self {
        let file = PathBuf::from(path);
        let parsed: JsonValue = match serde_json::from_str(&strip_json_comments(strip_bom(content)))
        {
            Ok(value) => value,
            Err(error) => {
                return Self::failed(format!(
                    "Failed to parse models.json: {error}\n\nFile: {}",
                    file.display()
                ));
            }
        };
        let errors = validate(&MODELS_CONFIG, &parsed);
        if !errors.is_empty() {
            let lines: Vec<String> = errors
                .iter()
                .map(|error| format!("  - {}: {}", error.path, error.message))
                .collect();
            return Self::failed(format!(
                "Invalid models.json schema:\n{}\n\nFile: {}",
                lines.join("\n"),
                file.display()
            ));
        }
        match serde_json::from_value::<RawConfig>(parsed) {
            Ok(raw) => Self {
                providers: raw.providers,
                error: None,
            },
            // Unreachable after validation in practice; still reported, never panics.
            Err(error) => Self::failed(format!(
                "Invalid models.json schema:\n  - root: {error}\n\nFile: {}",
                file.display()
            )),
        }
    }

    fn failed(error: String) -> Self {
        Self {
            providers: IndexMap::new(),
            error: Some(error),
        }
    }

    /// Provider entry by id.
    #[must_use]
    pub fn provider(&self, id: &str) -> Option<&ModelsJsonProvider> {
        self.providers.get(id)
    }

    /// Provider ids in file order.
    pub fn provider_ids(&self) -> impl Iterator<Item = &str> {
        self.providers.keys().map(String::as_str)
    }

    /// The load diagnostic, if the file could not be used.
    #[must_use]
    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> ModelsJson {
        ModelsJson::parse_at(text, Path::new("/x/models.json"))
    }

    #[test]
    fn parses_lenient_json() {
        let config = parse("{ // c\n \"providers\": {\"a\": {\"baseUrl\": \"u\",}},}");
        assert_eq!(config.error(), None);
        assert_eq!(
            config.provider("a").and_then(|p| p.base_url.as_deref()),
            Some("u")
        );
    }

    #[test]
    fn schema_error_text() {
        let config = parse(r#"{"providers":{"a":{"baseUrl":5}}}"#);
        assert_eq!(
            config.error(),
            Some(
                "Invalid models.json schema:\n  - providers.a.baseUrl: must be string\n\nFile: /x/models.json"
            )
        );
        assert_eq!(config.provider_ids().count(), 0);
    }

    #[test]
    fn missing_file_is_empty() {
        let config = ModelsJson::load(Some(Path::new("/definitely/missing/models.json")));
        assert_eq!(config, ModelsJson::empty());
    }
}
