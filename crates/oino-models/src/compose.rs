//! pi `provider-composer.ts` model layering: registration models, then `models.json`
//! (provider fields and `models[]` upserts), then config-only overrides, then `modelOverrides`.
//!
//! Errors are pi's messages (`Provider x: …`); callers record them per provider and keep the
//! base models.

use indexmap::IndexMap;
use oino_ai::{ImageInputLimits, InputLimits, InputModality, JsonValue, Model, ModelCost};
use oino_ai::{PromptCache, ThinkingLevel, ThinkingLevelMap};

use crate::models_json::{
    ModelsJsonModel, ModelsJsonModelOverride, ModelsJsonProvider, SamplingByLevel,
};
use crate::registration::{
    DEFAULT_CONTEXT_WINDOW, DEFAULT_MAX_TOKENS, ProviderConfigOverride, ProviderModelConfig,
};

/// Nested compat objects that merge per key (pi `mergeCompat`).
const NESTED_COMPAT_KEYS: [&str; 4] = [
    "openRouterRouting",
    "vercelGatewayRouting",
    "chatTemplateKwargs",
    "chatTemplateArgs",
];

/// API whose models serve as defaults for custom models when nothing closer matches.
const DEFAULT_TEMPLATE_API: &str = "openai-completions";

/// pi `mergeCompat`: shallow merge, nested routing objects merged per key.
#[must_use]
pub(crate) fn merge_compat(
    base: Option<&JsonValue>,
    override_compat: Option<&JsonValue>,
) -> Option<JsonValue> {
    let Some(override_compat) = override_compat else {
        return base.cloned();
    };
    let empty = serde_json::Map::new();
    let base_object = base.and_then(JsonValue::as_object).unwrap_or(&empty);
    let override_object = override_compat.as_object().unwrap_or(&empty);
    let mut merged = base_object.clone();
    for (key, value) in override_object {
        merged.insert(key.clone(), value.clone());
    }
    for key in NESTED_COMPAT_KEYS {
        let base_value = base_object.get(key).and_then(JsonValue::as_object);
        let override_value = override_object.get(key).and_then(JsonValue::as_object);
        if base_value.is_some() || override_value.is_some() {
            let mut nested = base_value.cloned().unwrap_or_default();
            for (k, v) in override_value.into_iter().flatten() {
                nested.insert(k.clone(), v.clone());
            }
            merged.insert(key.to_owned(), JsonValue::Object(nested));
        }
    }
    Some(JsonValue::Object(merged))
}

fn merge_input_limits(
    base: Option<&InputLimits>,
    over: Option<&InputLimits>,
) -> Option<InputLimits> {
    let Some(over) = over else {
        return base.cloned();
    };
    let base_images = base.and_then(|b| b.images.as_ref());
    let images = match &over.images {
        Some(over_images) => {
            let base_resize = base_images.and_then(|i| i.resize.as_ref());
            let resize = match &over_images.resize {
                Some(over_resize) => {
                    let base_resize = base_resize.cloned().unwrap_or_default();
                    Some(oino_ai::ImageResize {
                        max_width: over_resize.max_width.or(base_resize.max_width),
                        max_height: over_resize.max_height.or(base_resize.max_height),
                        max_bytes: over_resize.max_bytes.or(base_resize.max_bytes),
                        jpeg_quality: over_resize.jpeg_quality.or(base_resize.jpeg_quality),
                    })
                }
                None => base_resize.cloned(),
            };
            Some(ImageInputLimits {
                resize,
                max_per_message: over_images
                    .max_per_message
                    .or(base_images.and_then(|i| i.max_per_message)),
                max_per_request: over_images
                    .max_per_request
                    .or(base_images.and_then(|i| i.max_per_request)),
            })
        }
        None => base_images.cloned(),
    };
    Some(InputLimits {
        max_request_bytes: over
            .max_request_bytes
            .or(base.and_then(|b| b.max_request_bytes)),
        images,
    })
}

fn merge_thinking_map(
    base: Option<&ThinkingLevelMap>,
    over: Option<&ThinkingLevelMap>,
) -> Option<ThinkingLevelMap> {
    let Some(over) = over else {
        return base.cloned();
    };
    let base = base.cloned().unwrap_or_default();
    Some(ThinkingLevelMap {
        off: over.off.clone().or(base.off),
        minimal: over.minimal.clone().or(base.minimal),
        low: over.low.clone().or(base.low),
        medium: over.medium.clone().or(base.medium),
        high: over.high.clone().or(base.high),
        xhigh: over.xhigh.clone().or(base.xhigh),
        max: over.max.clone().or(base.max),
    })
}

fn merge_sampling_by_level(
    base: Option<&SamplingByLevel>,
    over: Option<&SamplingByLevel>,
) -> Option<SamplingByLevel> {
    let Some(over) = over else {
        return base.cloned();
    };
    let mut merged = base.cloned().unwrap_or_default();
    for level in ThinkingLevel::ALL {
        if let Some(params) = over.get(&level) {
            let entry = merged.entry(level).or_default();
            for (key, value) in params {
                entry.insert(key.clone(), value.clone());
            }
        }
    }
    Some(merged)
}

#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "token limits are validated > 0 by the schema; fractional values truncate"
)]
fn token_limit(value: f64) -> u64 {
    value as u64
}

/// pi `applyModelOverride`.
#[must_use]
pub(crate) fn apply_model_override(model: &Model, over: &ModelsJsonModelOverride) -> Model {
    let mut next = model.clone();
    if let Some(name) = &over.name {
        next.name.clone_from(name);
    }
    if let Some(reasoning) = over.reasoning {
        next.reasoning = reasoning;
    }
    next.thinking_level_map = merge_thinking_map(
        model.thinking_level_map.as_ref(),
        over.thinking_level_map.as_ref(),
    );
    if let Some(input) = &over.input {
        next.input.clone_from(input);
    }
    next.input_limits = merge_input_limits(model.input_limits.as_ref(), over.input_limits.as_ref());
    if let Some(cost) = &over.cost {
        next.cost = ModelCost {
            input: cost.input.unwrap_or(model.cost.input),
            output: cost.output.unwrap_or(model.cost.output),
            cache_read: cost.cache_read.unwrap_or(model.cost.cache_read),
            cache_write: cost.cache_write.unwrap_or(model.cost.cache_write),
            tiers: cost.tiers.clone().or_else(|| model.cost.tiers.clone()),
        };
    }
    if let Some(cache) = &over.prompt_cache {
        let base = model.prompt_cache.clone().unwrap_or_default();
        next.prompt_cache = Some(PromptCache {
            short: cache.short.or(base.short),
            long: cache.long.or(base.long),
        });
    }
    if let Some(window) = over.context_window {
        next.context_window = token_limit(window);
    }
    if let Some(max) = over.max_tokens {
        next.max_tokens = token_limit(max);
    }
    if let Some(params) = &over.sampling_params {
        let mut merged = model.sampling_params.clone().unwrap_or_default();
        for (key, value) in params {
            merged.insert(key.clone(), value.clone());
        }
        next.sampling_params = Some(merged);
    }
    next.sampling_params_by_thinking_level = merge_sampling_by_level(
        model.sampling_params_by_thinking_level.as_ref(),
        over.sampling_params_by_thinking_level.as_ref(),
    );
    next.compat = merge_compat(model.compat.as_ref(), over.compat.as_ref());
    next
}

/// pi `findModelDefaults`.
fn find_model_defaults<'a>(
    models: &'a [Model],
    model_id: &str,
    api: Option<&str>,
) -> Option<&'a Model> {
    models
        .iter()
        .find(|m| m.id == model_id)
        .or_else(|| api.and_then(|api| models.iter().find(|m| m.api == api)))
        .or_else(|| models.iter().find(|m| m.api == DEFAULT_TEMPLATE_API))
        .or_else(|| models.first())
}

/// pi `modelFromJson`.
fn model_from_json(
    provider_id: &str,
    definition: &ModelsJsonModel,
    config: &ModelsJsonProvider,
    defaults: Option<&Model>,
) -> Result<Model, String> {
    let api = definition
        .api
        .clone()
        .or_else(|| config.api.clone())
        .or_else(|| defaults.map(|d| d.api.clone()))
        .ok_or_else(|| {
            format!(
                "Provider {provider_id}, model {}: no \"api\" specified. Set at provider or model level.",
                definition.id
            )
        })?;
    let base_url = definition
        .base_url
        .clone()
        .or_else(|| config.base_url.clone())
        .or_else(|| defaults.map(|d| d.base_url.clone()))
        .ok_or_else(|| {
            format!("Provider {provider_id}: \"baseUrl\" is required when defining custom models.")
        })?;
    if definition.context_window.is_some_and(|w| w <= 0.0) {
        return Err(format!(
            "Provider {provider_id}, model {}: invalid contextWindow",
            definition.id
        ));
    }
    if definition.max_tokens.is_some_and(|m| m <= 0.0) {
        return Err(format!(
            "Provider {provider_id}, model {}: invalid maxTokens",
            definition.id
        ));
    }
    Ok(Model {
        id: definition.id.clone(),
        name: definition
            .name
            .clone()
            .unwrap_or_else(|| definition.id.clone()),
        api,
        provider: provider_id.to_owned(),
        base_url,
        input: definition
            .input
            .clone()
            .unwrap_or_else(|| vec![InputModality::Text]),
        input_limits: definition.input_limits.clone(),
        cost: definition.cost.clone().unwrap_or_default(),
        headers: None,
        kind: None,
        reasoning: definition.reasoning.unwrap_or(false),
        thinking_level_map: definition.thinking_level_map.clone(),
        prompt_cache: definition.prompt_cache.clone(),
        context_window: definition
            .context_window
            .map_or(DEFAULT_CONTEXT_WINDOW, token_limit),
        max_tokens: definition
            .max_tokens
            .map_or(DEFAULT_MAX_TOKENS, token_limit),
        sampling_params: definition.sampling_params.clone(),
        sampling_params_by_thinking_level: definition.sampling_params_by_thinking_level.clone(),
        compat: merge_compat(config.compat.as_ref(), definition.compat.as_ref()),
    })
}

fn has_models_json_content(config: &ModelsJsonProvider) -> bool {
    let has_overrides = config
        .model_overrides
        .as_ref()
        .is_some_and(|o| !o.is_empty());
    config.models.as_ref().is_some_and(|m| !m.is_empty())
        || config.base_url.is_some()
        || config.headers.is_some()
        || config.compat.is_some()
        || has_overrides
        || config.api_key.is_some()
        || config.oauth.is_some()
        || config.auth_header.is_some()
}

/// pi `applyModelsJson`.
///
/// # Errors
///
/// pi's structural messages (missing `baseUrl`/`api`, empty provider entry).
pub(crate) fn apply_models_json(
    provider_id: &str,
    base: &[Model],
    config: Option<&ModelsJsonProvider>,
) -> Result<Vec<Model>, String> {
    let Some(config) = config else {
        return Ok(base.to_vec());
    };
    if config.oauth.is_some() && config.base_url.is_none() {
        return Err(format!(
            "Provider {provider_id}: \"baseUrl\" is required when \"oauth\" is set."
        ));
    }
    if !has_models_json_content(config) {
        return Err(format!(
            "Provider {provider_id}: must specify \"baseUrl\", \"headers\", \"compat\", \"modelOverrides\", or \"models\"."
        ));
    }
    let radius = config.oauth.as_deref() == Some("radius");
    let mut models: Vec<Model> = base
        .iter()
        .map(|model| {
            let mut next = model.clone();
            if !radius && let Some(base_url) = &config.base_url {
                next.base_url.clone_from(base_url);
            }
            next.compat = merge_compat(model.compat.as_ref(), config.compat.as_ref());
            next
        })
        .collect();
    for definition in config.models.iter().flatten() {
        let api = definition.api.as_deref().or(config.api.as_deref());
        let defaults = find_model_defaults(&models, &definition.id, api);
        let model = model_from_json(provider_id, definition, config, defaults)?;
        match models.iter().position(|m| m.id == definition.id) {
            Some(index) => models[index] = model,
            None => models.push(model),
        }
    }
    Ok(models)
}

/// pi `extensionModelFromDefinition`.
fn model_from_override_definition(
    provider_id: &str,
    models: &[Model],
    config: &ProviderConfigOverride,
    definition: &ProviderModelConfig,
) -> Result<Model, String> {
    let defaults = find_model_defaults(models, &definition.id, definition.api.as_deref());
    let api = definition
        .api
        .clone()
        .or_else(|| config.api.clone())
        .or_else(|| defaults.map(|d| d.api.clone()))
        .ok_or_else(|| {
            format!(
                "Provider {provider_id}, model {}: no \"api\" specified. Set it at model level or provider level.",
                definition.id
            )
        })?;
    let base_url = definition
        .base_url
        .clone()
        .or_else(|| config.base_url.clone())
        .or_else(|| defaults.map(|d| d.base_url.clone()))
        .ok_or_else(|| {
            format!("Provider {provider_id}: \"baseUrl\" is required when defining custom models.")
        })?;
    let mut model = definition.clone().into_model(provider_id, api, base_url);
    // Override model headers are request-time headers (pi `rawModelHeaders`).
    model.headers = None;
    Ok(model)
}

/// pi `applyExtension` for config-only overrides.
///
/// # Errors
///
/// pi's messages for override models without `api`/`baseUrl`.
pub(crate) fn apply_override(
    provider_id: &str,
    models: Vec<Model>,
    config: Option<&ProviderConfigOverride>,
) -> Result<Vec<Model>, String> {
    let Some(config) = config else {
        return Ok(models);
    };
    let Some(definitions) = &config.models else {
        return Ok(match &config.base_url {
            Some(base_url) => models
                .into_iter()
                .map(|mut m| {
                    m.base_url.clone_from(base_url);
                    m
                })
                .collect(),
            None => models,
        });
    };
    definitions
        .iter()
        .map(|definition| model_from_override_definition(provider_id, &models, config, definition))
        .collect()
}

/// Full composition (pi `composeModelProvider().getAllModels()`): `models.json`, then the
/// override, then `modelOverrides`.
///
/// # Errors
///
/// The first structural error, as pi reports it.
pub(crate) fn compose_models(
    provider_id: &str,
    base: &[Model],
    config: Option<&ModelsJsonProvider>,
    override_config: Option<&ProviderConfigOverride>,
) -> Result<Vec<Model>, String> {
    let models = apply_override(
        provider_id,
        apply_models_json(provider_id, base, config)?,
        override_config,
    )?;
    let overrides = config.and_then(|c| c.model_overrides.as_ref());
    Ok(models
        .into_iter()
        .map(|model| match overrides.and_then(|o| o.get(&model.id)) {
            Some(over) => apply_model_override(&model, over),
            None => model,
        })
        .collect())
}

/// pi `rawModelHeaders`: request-time headers configured for one model
/// (`modelOverrides` < `models[]` < override `models[]`).
#[must_use]
pub(crate) fn raw_model_headers(
    model_id: &str,
    config: Option<&ModelsJsonProvider>,
    override_config: Option<&ProviderConfigOverride>,
) -> Option<IndexMap<String, String>> {
    let mut headers = IndexMap::new();
    let layers = [
        config
            .and_then(|c| c.model_overrides.as_ref())
            .and_then(|o| o.get(model_id))
            .and_then(|o| o.headers.as_ref()),
        config
            .and_then(|c| c.models.as_ref())
            .and_then(|models| models.iter().find(|m| m.id == model_id))
            .and_then(|m| m.headers.as_ref()),
        override_config
            .and_then(|c| c.models.as_ref())
            .and_then(|models| models.iter().find(|m| m.id == model_id))
            .and_then(|m| m.headers.as_ref()),
    ];
    for layer in layers.into_iter().flatten() {
        for (key, value) in layer {
            headers.insert(key.clone(), value.clone());
        }
    }
    (!headers.is_empty()).then_some(headers)
}

/// pi `configuredHeaders`: provider headers from `models.json` and the override (override wins).
#[must_use]
pub(crate) fn configured_headers(
    config: Option<&ModelsJsonProvider>,
    override_config: Option<&ProviderConfigOverride>,
) -> Option<IndexMap<String, String>> {
    let config_headers = config.and_then(|c| c.headers.as_ref());
    let override_headers = override_config.and_then(|c| c.headers.as_ref());
    if config_headers.is_none() && override_headers.is_none() {
        return None;
    }
    let mut merged = IndexMap::new();
    for layer in [config_headers, override_headers].into_iter().flatten() {
        for (key, value) in layer {
            merged.insert(key.clone(), value.clone());
        }
    }
    Some(merged)
}

/// pi `mergeHeaders`: later names replace earlier ones case-insensitively.
#[must_use]
pub(crate) fn merge_headers(
    base: Option<IndexMap<String, String>>,
    over: Option<IndexMap<String, String>>,
) -> Option<IndexMap<String, String>> {
    if base.is_none() && over.is_none() {
        return None;
    }
    let mut merged = base.unwrap_or_default();
    for (name, value) in over.unwrap_or_default() {
        let lower = name.to_lowercase();
        merged.retain(|existing, _| existing.to_lowercase() != lower);
        merged.insert(name, value);
    }
    Some(merged)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn model(id: &str, api: &str) -> Model {
        serde_json::from_value(json!({
            "id": id, "name": id, "api": api, "provider": "p", "baseUrl": "https://base",
            "input": ["text"], "cost": {"input": 1, "output": 2, "cacheRead": 0, "cacheWrite": 0},
            "reasoning": false, "contextWindow": 1000, "maxTokens": 100
        }))
        .unwrap()
    }

    #[test]
    fn compat_merges_nested_routing() {
        let merged = merge_compat(
            Some(&json!({"a": 1, "openRouterRouting": {"x": 1}})),
            Some(&json!({"b": 2, "openRouterRouting": {"y": 2}})),
        );
        assert_eq!(
            merged,
            Some(json!({"a": 1, "openRouterRouting": {"x": 1, "y": 2}, "b": 2}))
        );
    }

    #[test]
    fn models_json_upserts_and_overrides() {
        let config: ModelsJsonProvider = serde_json::from_value(json!({
            "baseUrl": "https://proxy",
            "models": [{"id": "new"}, {"id": "a", "name": "Replaced"}],
            "modelOverrides": {"new": {"contextWindow": 5, "cost": {"input": 9}}}
        }))
        .unwrap();
        let models = compose_models("p", &[model("a", "x-api")], Some(&config), None).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "Replaced");
        assert_eq!(models[0].base_url, "https://proxy");
        assert_eq!(models[1].api, "x-api");
        assert_eq!(models[1].context_window, 5);
        assert!((models[1].cost.input - 9.0).abs() < f64::EPSILON);
    }

    #[test]
    fn empty_provider_entry_is_an_error() {
        let config = ModelsJsonProvider::default();
        assert_eq!(
            compose_models("p", &[], Some(&config), None).unwrap_err(),
            "Provider p: must specify \"baseUrl\", \"headers\", \"compat\", \"modelOverrides\", or \"models\"."
        );
    }

    #[test]
    fn headers_merge_case_insensitively() {
        let merged = merge_headers(
            Some([("X-A".to_owned(), "1".to_owned())].into()),
            Some([("x-a".to_owned(), "2".to_owned())].into()),
        )
        .unwrap();
        assert_eq!(merged.len(), 1);
        assert_eq!(merged.get("x-a").map(String::as_str), Some("2"));
    }
}
