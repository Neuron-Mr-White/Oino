//! Bundled provider catalogs (`catalog.json`, generated from pi-ai 1.4.0 by
//! `scripts/pi-oracle/providers-v1/catalog.mjs`), provider registrations and model listing.

use serde::Deserialize;
use serde_json::{Value, json};

/// A bundled catalog.
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Catalog {
    /// Providers this component registers.
    pub providers: Vec<CatalogProvider>,
}

/// One provider.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogProvider {
    /// Provider id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Primary wire API.
    pub api: String,
    /// Default base URL.
    pub base_url: String,
    /// API-key environment variables, in priority order.
    #[serde(default)]
    pub auth_env: Vec<String>,
    /// Local server: no credential needed.
    #[serde(default)]
    pub local: bool,
    /// The component implements `oauth-login` / `oauth-refresh` for this provider.
    #[serde(default)]
    pub oauth: bool,
    /// Static pi `Model` objects, or `"dynamic"` (listed from `GET {baseUrl}/models`).
    pub models: Value,
}

impl Catalog {
    /// Parses catalog JSON.
    ///
    /// # Errors
    /// Invalid JSON.
    pub fn parse(text: &str) -> Result<Self, String> {
        serde_json::from_str(text).map_err(|e| format!("invalid catalog: {e}"))
    }

    /// Provider by id.
    #[must_use]
    pub fn provider(&self, id: &str) -> Option<&CatalogProvider> {
        self.providers.iter().find(|p| p.id == id)
    }
}

impl CatalogProvider {
    /// Registration JSON (`registry.register-provider`, extension-api §4.4).
    #[must_use]
    pub fn registration(&self) -> Value {
        let auth_header = if self.api == "anthropic-messages" {
            "x-api-key"
        } else {
            "bearer"
        };
        json!({
            "id": self.id,
            "name": self.name,
            "api": self.api,
            "baseUrl": self.base_url,
            "authEnv": self.auth_env,
            "authHeader": if self.local { "none" } else { auth_header },
            "oauth": self.oauth || self.id == "anthropic",
            "headers": {},
            "compat": {},
            "models": "dynamic",
        })
    }

    /// Static models (empty for dynamic providers).
    #[must_use]
    pub fn static_models(&self) -> Vec<Value> {
        self.models.as_array().cloned().unwrap_or_default()
    }

    /// Whether models come from the endpoint.
    #[must_use]
    pub fn is_dynamic(&self) -> bool {
        !self.models.is_array()
    }

    /// A pi `Model` for an OpenAI-compatible `/models` row (defaults where the endpoint is
    /// silent).
    #[must_use]
    pub fn model_from_listing(&self, row: &Value) -> Option<Value> {
        let id = row["id"].as_str()?;
        let context = row["context_length"]
            .as_u64()
            .or_else(|| row["context_window"].as_u64())
            .or_else(|| row["max_model_len"].as_u64())
            .unwrap_or(128_000);
        let max_tokens = row["max_output_tokens"]
            .as_u64()
            .or_else(|| row["top_provider"]["max_completion_tokens"].as_u64())
            .unwrap_or(16_384)
            .min(context);
        let vision = row["input_modalities"]
            .as_array()
            .is_some_and(|m| m.iter().any(|v| v == "image"))
            || row["capabilities"]["vision"] == true;
        let reasoning = row["capabilities"]["reasoning"] == true
            || row["supported_parameters"]
                .as_array()
                .is_some_and(|p| p.iter().any(|v| v == "reasoning"));
        Some(json!({
            "id": id,
            "name": row["name"].as_str().unwrap_or(id),
            "api": self.api,
            "provider": self.id,
            "baseUrl": self.base_url,
            "reasoning": reasoning,
            "input": if vision { json!(["text", "image"]) } else { json!(["text"]) },
            "cost": row.get("pricing").map_or_else(
                || json!({"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0}),
                open_router_cost,
            ),
            "contextWindow": context,
            "maxTokens": max_tokens,
        }))
    }
}

fn per_million(value: &Value, fallback: f64) -> f64 {
    let parsed = match value {
        Value::String(s) => s.trim().parse::<f64>().ok(),
        Value::Number(n) => n.as_f64(),
        _ => None,
    };
    match parsed {
        Some(per_token) if per_token >= 0.0 => {
            let scaled = per_token * 1_000_000.0;
            // Number(x.toFixed(6))
            (scaled * 1e6).round() / 1e6
        }
        _ => fallback,
    }
}

fn number(value: f64) -> Value {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::float_cmp,
        reason = "integral prices serialise like JavaScript numbers"
    )]
    if value.fract() == 0.0 && value.abs() < 1e15 {
        return json!(value as i64);
    }
    json!(value)
}

/// pi `openRouterCost`: `$/token` pricing → `$/1M` `ModelCost` JSON.
#[must_use]
pub fn open_router_cost(pricing: &Value) -> Value {
    let input = per_million(&pricing["prompt"], 0.0);
    let output = per_million(&pricing["completion"], 0.0);
    let read = per_million(&pricing["input_cache_read"], 0.0);
    let write = per_million(&pricing["input_cache_write"], 0.0);
    let tiers: Vec<Value> = pricing["overrides"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|o| {
            o.get("min_prompt_tokens").is_some()
                && o.get("utc_start").is_none()
                && o.get("utc_end").is_none()
                && o.get("utc_days").is_none()
        })
        .map(|o| {
            json!({
                "inputTokensAbove": o["min_prompt_tokens"],
                "input": number(per_million(&o["prompt"], input)),
                "output": number(per_million(&o["completion"], output)),
                "cacheRead": number(per_million(&o["input_cache_read"], read)),
                "cacheWrite": number(per_million(&o["input_cache_write"], write)),
            })
        })
        .collect();
    let mut cost = json!({
        "input": number(input), "output": number(output),
        "cacheRead": number(read), "cacheWrite": number(write),
    });
    if !tiers.is_empty() {
        cost["tiers"] = Value::Array(tiers);
    }
    cost
}

/// pi `personalizeOpenRouterCatalog`: restrict chat models to the key's `/models/user` list and
/// apply its limits and prices.
#[must_use]
pub fn personalize_open_router(models: Vec<Value>, user_models: &Value) -> Vec<Value> {
    let Some(rows) = user_models["data"].as_array() else {
        return models;
    };
    models
        .into_iter()
        .filter_map(|mut model| {
            let row = rows.iter().find(|r| r["id"] == model["id"])?;
            let positive = |v: &Value| v.as_u64().filter(|n| *n > 0);
            if let Some(context) = positive(&row["top_provider"]["context_length"])
                .or_else(|| positive(&row["context_length"]))
            {
                model["contextWindow"] = json!(context);
            }
            if let Some(max) = positive(&row["top_provider"]["max_completion_tokens"]) {
                model["maxTokens"] = json!(max);
            }
            if let Some(pricing) = row.get("pricing").filter(|p| p.is_object()) {
                model["cost"] = open_router_cost(pricing);
            }
            Some(model)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn open_router_prices() {
        let cost = open_router_cost(
            &json!({"prompt": "0.000003", "completion": "0.000015", "input_cache_read": "-1"}),
        );
        assert_eq!(
            cost,
            json!({"input": 3, "output": 15, "cacheRead": 0, "cacheWrite": 0})
        );
    }
}
