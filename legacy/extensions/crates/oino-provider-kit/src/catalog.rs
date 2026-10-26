//! Provider model catalog ported verbatim from pi's generated model data
//! (`packages/ai/src/providers/data/{anthropic,openai}.json`, embedded here as
//! `assets/*.json`).

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::piutil::CostRates;

/// Flattened catalog row (pi `ChatModelCatalog` entry).
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    pub api: String,
    pub provider: String,
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    #[serde(default)]
    pub reasoning: bool,
    #[serde(default)]
    pub input: Vec<String>,
    #[serde(default)]
    pub cost: CostRates,
    #[serde(default, rename = "contextWindow")]
    pub context_window: u64,
    #[serde(default, rename = "maxTokens")]
    pub max_tokens: u64,
    #[serde(default, rename = "thinkingLevelMap")]
    pub thinking_level_map: Option<BTreeMap<String, Option<String>>>,
    #[serde(default)]
    pub compat: serde_json::Value,
    #[serde(default, rename = "promptCache")]
    pub prompt_cache: Option<PromptCache>,
}

impl Default for CatalogModel {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            api: String::new(),
            provider: String::new(),
            base_url: String::new(),
            reasoning: false,
            input: Vec::new(),
            cost: CostRates::default(),
            context_window: 0,
            max_tokens: 0,
            thinking_level_map: None,
            compat: serde_json::Value::Null,
            prompt_cache: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PromptCache {
    pub short: u64,
    pub long: u64,
}

impl CatalogModel {
    /// pi `model.input.includes("image")`.
    #[must_use]
    pub fn accepts_images(&self) -> bool {
        self.input.iter().any(|m| m == "image")
    }
    /// Compat flag read with pi's `?? default` semantics.
    #[must_use]
    pub fn compat_flag(&self, key: &str, default: bool) -> bool {
        self.compat
            .get(key)
            .and_then(|v| v.as_bool())
            .unwrap_or(default)
    }
    #[must_use]
    pub fn compat_str(&self, key: &str) -> Option<&str> {
        self.compat.get(key).and_then(|v| v.as_str())
    }
    /// Facets for `piutil` transforms.
    #[must_use]
    pub fn facets(&self) -> crate::piutil::ModelFacets<'_> {
        crate::piutil::ModelFacets {
            provider: &self.provider,
            api: &self.api,
            id: &self.id,
            accepts_images: self.accepts_images(),
        }
    }
}

fn load(json_text: &str) -> BTreeMap<String, CatalogModel> {
    let groups: BTreeMap<String, BTreeMap<String, CatalogModel>> =
        serde_json::from_str(json_text).unwrap_or_default();
    groups
        .into_values()
        .flat_map(|models| {
            models
                .into_iter()
                .map(|(key, model)| (key.strip_prefix("chat:").unwrap_or(&key).to_string(), model))
        })
        .collect()
}

/// Anthropic catalog (`anthropic-messages` api).
#[must_use]
pub fn anthropic_models() -> BTreeMap<String, CatalogModel> {
    load(include_str!("../assets/anthropic.json"))
}

/// OpenAI catalog (`openai-responses` api).
#[must_use]
pub fn openai_models() -> BTreeMap<String, CatalogModel> {
    load(include_str!("../assets/openai.json"))
}

/// Lookup with a safe default for unknown ids (mirrors pi's permissive
/// behavior for models not in the catalog).
#[must_use]
pub fn model_or_default(
    catalog: &BTreeMap<String, CatalogModel>,
    id: &str,
    provider: &str,
    api: &str,
) -> CatalogModel {
    catalog.get(id).cloned().unwrap_or_else(|| CatalogModel {
        id: id.to_string(),
        name: id.to_string(),
        api: api.to_string(),
        provider: provider.to_string(),
        ..CatalogModel::default()
    })
}
