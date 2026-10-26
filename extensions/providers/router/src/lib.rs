//! `oino.router`: the Oino Router (`oino` provider, api `oino-router`). One endpoint serves two
//! wires: Anthropic Messages for Claude-family models, `OpenAI` chat completions for the rest.
//! `list-models` reads the router's `/models`, enriches Claude models from the bundled pi-ai
//! Anthropic catalog and records the wire in `compat.oinoWire`; `stream` dispatches on it.

#![allow(
    clippy::unused_async_trait_impl,
    reason = "SDK `Extension::init` is async even when registration is synchronous"
)]

use std::sync::OnceLock;

use oino_extension::futures::future::LocalBoxFuture;
use oino_extension::prelude::*;
use oino_extension::provider::{Provider, ProviderStream};
use oino_provider_kit::Wire;
use oino_provider_kit::catalog::Catalog;
use oino_provider_kit::guest::{self, StreamOptions};

const ANTHROPIC_CATALOG: &str = include_str!("../../anthropic/catalog.json");
const PROVIDER_ID: &str = "oino";
const API: &str = "oino-router";
const DEFAULT_BASE_URL: &str = "https://router.oino.dev/v1";
const WIRE_KEY: &str = "oinoWire";

/// Component state.
#[derive(Default)]
struct Router {
    anthropic: OnceLock<Catalog>,
}

impl Router {
    fn claude(&self, root: &str) -> Option<Value> {
        let catalog = self
            .anthropic
            .get_or_init(|| Catalog::parse(ANTHROPIC_CATALOG).unwrap_or_default());
        catalog
            .provider("anthropic")?
            .static_models()
            .into_iter()
            .find(|m| m["id"] == root)
    }
}

fn wants_anthropic(row: &Value) -> bool {
    let owner = row["owned_by"].as_str().unwrap_or_default();
    owner == "claude"
        || owner == "anthropic"
        || row["root"]
            .as_str()
            .unwrap_or_default()
            .starts_with("claude-")
}

/// `thinkingLevelMap` from the router's `capabilities.effort_tiers`.
fn thinking_level_map(row: &Value) -> Option<Value> {
    let tiers: Vec<&str> = row["capabilities"]["effort_tiers"]
        .as_array()?
        .iter()
        .filter_map(Value::as_str)
        .collect();
    if tiers.is_empty() {
        return None;
    }
    let mut map = serde_json::Map::new();
    for (level, tier) in [
        ("off", "none"),
        ("minimal", "minimal"),
        ("low", "low"),
        ("medium", "medium"),
        ("high", "high"),
        ("xhigh", "xhigh"),
        ("max", "max"),
    ] {
        map.insert(
            level.into(),
            if tiers.contains(&tier) {
                json!(tier)
            } else {
                Value::Null
            },
        );
    }
    Some(Value::Object(map))
}

impl Router {
    fn model(&self, row: &Value) -> Option<Value> {
        let id = row["id"].as_str()?;
        let anthropic = wants_anthropic(row);
        let wire = if anthropic { "anthropic" } else { "openai" };
        let root = row["root"].as_str().unwrap_or(id);
        let mut model = if let Some(base) = self.claude(root).filter(|_| anthropic) {
            base
        } else {
            let vision = row["input_modalities"]
                .as_array()
                .is_some_and(|m| m.iter().any(|v| v == "image"))
                || row["capabilities"]["vision"] == true;
            let mut model = json!({
                "reasoning": row["capabilities"]["reasoning"] == true,
                "input": if vision { json!(["text", "image"]) } else { json!(["text"]) },
                "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0},
                "contextWindow": 128_000,
                "maxTokens": 16_384,
            });
            if let Some(map) = thinking_level_map(row) {
                model["thinkingLevelMap"] = map;
            }
            model
        };
        model["id"] = json!(id);
        model["name"] = json!(row["name"].as_str().unwrap_or(id));
        model["api"] = json!(API);
        model["provider"] = json!(PROVIDER_ID);
        model["baseUrl"] = json!(DEFAULT_BASE_URL);
        if let Some(context) = row["context_length"].as_u64() {
            model["contextWindow"] = json!(context);
        }
        if let Some(max) = row["max_output_tokens"].as_u64() {
            model["maxTokens"] = json!(max);
        }
        if !model["compat"].is_object() {
            model["compat"] = json!({});
        }
        model["compat"][WIRE_KEY] = json!(wire);
        Some(model)
    }
}

fn wire_of(model: &Value) -> Wire {
    let explicit = model["compat"][WIRE_KEY].as_str();
    let id = model["id"].as_str().unwrap_or_default();
    match explicit {
        Some("anthropic") => Wire::Anthropic,
        None if id.contains("claude") => Wire::Anthropic,
        _ => Wire::Completions,
    }
}

impl Provider for Router {
    fn stream(&'static self, request: Value) -> LocalBoxFuture<'static, Result<ProviderStream>> {
        Box::pin(async move {
            let mut request = request;
            let wire = wire_of(&request["model"]);
            if wire == Wire::Anthropic {
                // The Anthropic wire appends `/v1/messages`; the router base already ends in `/v1`.
                let base = request["model"]["baseUrl"]
                    .as_str()
                    .unwrap_or(DEFAULT_BASE_URL)
                    .trim_end_matches('/')
                    .to_owned();
                let base = base.strip_suffix("/v1").unwrap_or(&base).to_owned();
                request["model"]["baseUrl"] = json!(base);
            }
            guest::stream(
                &request,
                StreamOptions {
                    wire: Some(wire),
                    allow_missing_key: false,
                },
            )
        })
    }

    fn list_models(
        &'static self,
        provider_id: String,
        credential: Option<Value>,
    ) -> LocalBoxFuture<'static, Result<Value>> {
        Box::pin(async move {
            if provider_id != PROVIDER_ID {
                return Err(Error::msg(format!("unknown provider {provider_id}")));
            }
            let key = guest::credential_key(credential.as_ref())
                .ok_or_else(|| Error::msg("Oino Router: set OINO_API_KEY (or OMNI_API_KEY)"))?;
            let response = http::get(&format!("{DEFAULT_BASE_URL}/models"))
                .header("accept", "application/json")
                .header("authorization", &format!("Bearer {key}"))
                .timeout_ms(15_000)
                .send()
                .await?;
            let status = response.status();
            let body: Value =
                serde_json::from_slice(&response.bytes().await).unwrap_or(Value::Null);
            if !(200..300).contains(&status) {
                return Err(Error::msg(format!(
                    "Oino Router model list request failed: {status}"
                )));
            }
            let rows = body["data"].as_array().cloned().unwrap_or_default();
            Ok(Value::Array(
                rows.iter().filter_map(|row| self.model(row)).collect(),
            ))
        })
    }
}

impl Extension for Router {
    async fn init(&'static self, _cx: &mut Init<Self>) -> Result<()> {
        oino_extension::wit::registry::register_provider(
            &json!({
                "id": PROVIDER_ID,
                "name": "Oino Router",
                "api": API,
                "baseUrl": DEFAULT_BASE_URL,
                "authEnv": ["OINO_API_KEY", "OMNI_API_KEY"],
                "authHeader": "bearer",
                "oauth": false,
                "headers": {},
                "compat": {},
                "models": "dynamic",
            })
            .to_string(),
        )?;
        Ok(())
    }

    fn provider(&'static self) -> Option<&'static dyn Provider> {
        Some(self)
    }
}

oino_extension::export_extension!(Router);
