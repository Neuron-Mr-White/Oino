//! Image model catalog, routing and keys (unipi `src/image/models.ts`).
//!
//! Catalog: OpenRouter's image models (public `/models?output_modalities=image`, refreshed at
//! most daily into `/cache/image-models-cache.json`), a short list of fal models (listed when
//! a fal key is available) and the host's chat models (vision = `input` includes `image`).
//!
//! Routing a `provider/model-id` reference:
//!
//! | ref | endpoint | key |
//! |---|---|---|
//! | `openrouter/…` | OpenRouter chat + modalities | setting → secret → `credential-for` → `OPENROUTER_API_KEY` |
//! | `fal/…` | fal.run | setting → secret → `FAL_KEY` / `FAL_API_KEY` |
//! | other provider | its `baseUrl`, OpenAI images shape | `credential-for(provider)` → `<PROVIDER>_API_KEY` |
//! | custom `baseUrl` | that endpoint, model id as typed | its key |

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::settings::{Endpoint, ImageApi};

/// Cache file.
pub const CACHE_FILE: &str = "/cache/image-models-cache.json";
/// Refresh at most this often.
pub const CACHE_TTL_MS: i64 = 24 * 3_600_000;
/// OpenRouter's public image model list.
pub const OPENROUTER_MODELS_URL: &str =
    "https://openrouter.ai/api/v1/models?output_modalities=image";

/// One catalog entry (unipi `CachedModel`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CachedModel {
    /// Provider id.
    pub provider: String,
    /// Model id.
    pub id: String,
    /// Display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Input modalities.
    #[serde(default)]
    pub input: Vec<String>,
    /// Output modalities.
    #[serde(default)]
    pub output: Vec<String>,
}

impl CachedModel {
    fn fal(id: &str, name: &str, edit: bool) -> Self {
        Self {
            provider: "fal".into(),
            id: id.into(),
            name: Some(name.into()),
            input: if edit {
                vec!["text".into(), "image".into()]
            } else {
                vec!["text".into()]
            },
            output: vec!["image".into()],
        }
    }

    /// `provider/id`.
    #[must_use]
    pub fn reference(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }
}

/// The fal models offered when a fal key is available.
#[must_use]
pub fn fal_models() -> Vec<CachedModel> {
    vec![
        CachedModel::fal("fal-ai/flux-2/klein/4b", "FLUX.2 [klein] 4B", false),
        CachedModel::fal(
            "fal-ai/flux-2/klein/4b/edit",
            "FLUX.2 [klein] 4B edit",
            true,
        ),
        CachedModel::fal("fal-ai/flux-2", "FLUX.2 [dev]", false),
        CachedModel::fal("fal-ai/flux-2/edit", "FLUX.2 [dev] edit", true),
        CachedModel::fal("fal-ai/nano-banana-pro", "Nano Banana Pro", false),
        CachedModel::fal("fal-ai/nano-banana-pro/edit", "Nano Banana Pro edit", true),
    ]
}

/// The cached OpenRouter image list.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Cache {
    /// Epoch ms of the last refresh.
    #[serde(default)]
    pub updated_at: i64,
    /// Models.
    #[serde(default)]
    pub models: Vec<CachedModel>,
}

impl Cache {
    /// Reads the cache file (empty when missing or corrupt).
    #[must_use]
    pub fn read() -> Self {
        oino_extension::fs::read_json::<Self>(CACHE_FILE)
            .ok()
            .flatten()
            .unwrap_or_default()
    }

    /// Needs a refresh.
    #[must_use]
    pub fn stale(&self, now: i64) -> bool {
        self.models.is_empty() || now - self.updated_at >= CACHE_TTL_MS
    }

    /// Output modalities of a reference (`None` = unknown).
    #[must_use]
    pub fn output_of(&self, reference: &str) -> Option<Vec<String>> {
        self.models
            .iter()
            .find(|m| m.reference() == reference)
            .map(|m| m.output.clone())
    }
}

fn modalities(list: Option<&Value>) -> Vec<String> {
    list.and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .filter(|m| matches!(*m, "text" | "image"))
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// OpenRouter `/models` (image output) → cache entries.
#[must_use]
pub fn parse_openrouter_image_models(json: &Value) -> Vec<CachedModel> {
    let Some(data) = json.get("data").and_then(Value::as_array) else {
        return Vec::new();
    };
    data.iter()
        .filter_map(|m| {
            let id = m.get("id")?.as_str()?;
            if id.starts_with("openrouter/") {
                return None;
            }
            let arch = m.get("architecture");
            let output = modalities(arch.and_then(|a| a.get("output_modalities")));
            if !output.iter().any(|o| o == "image") {
                return None;
            }
            let input = modalities(arch.and_then(|a| a.get("input_modalities")));
            Some(CachedModel {
                provider: "openrouter".into(),
                id: id.to_owned(),
                name: m.get("name").and_then(Value::as_str).map(ToOwned::to_owned),
                input: if input.is_empty() {
                    vec!["text".into()]
                } else {
                    input
                },
                output,
            })
        })
        .collect()
}

/// Image entries of the catalog (first wins on duplicates).
#[must_use]
pub fn catalog(cache: &Cache, fal_available: bool) -> Vec<CachedModel> {
    let mut out = cache.models.clone();
    if fal_available {
        out.extend(fal_models());
    }
    let mut seen = std::collections::HashSet::new();
    out.retain(|m| seen.insert(m.reference()));
    out
}

/// `provider/model-id` split at the first `/`.
#[must_use]
pub fn split_ref(r: &str) -> Option<(&str, &str)> {
    let t = r.trim();
    let (p, id) = t.split_once('/')?;
    (!p.is_empty() && !id.is_empty()).then_some((p, id))
}

/// A resolved credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cred {
    /// A raw key (`Authorization: Bearer <key>`, fal: `Key <key>`).
    Key(String),
    /// Headers from the host credential store.
    Headers(Vec<(String, String)>),
}

/// Where and how to send a generate/edit request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Route {
    /// Wire shape.
    pub api: ImageApi,
    /// Endpoint base (`""` = the api's default host).
    pub base_url: String,
    /// Credential.
    pub cred: Cred,
    /// Model id sent on the wire.
    pub model_id: String,
    /// Label in results.
    pub label: String,
    /// Output modalities (OpenRouter).
    pub output: Option<Vec<String>>,
}

/// Base URL of a host provider (from `control.models-list`).
#[must_use]
pub fn provider_base_url(models: &[Value], provider: &str) -> Option<String> {
    models
        .iter()
        .filter(|m| m.get("provider").and_then(Value::as_str) == Some(provider))
        .find_map(|m| {
            m.get("baseUrl")
                .and_then(Value::as_str)
                .filter(|u| !u.is_empty())
        })
        .map(ToOwned::to_owned)
}

/// Env var name of a provider's key (`gateway-x` → `GATEWAY_X_API_KEY`).
#[must_use]
pub fn provider_env_var(provider: &str) -> String {
    let up: String = provider
        .to_ascii_uppercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .collect();
    format!("{up}_API_KEY")
}

/// Resolves a route; `cred(provider)` is the prefetched credential per provider.
///
/// # Errors
///
/// A message that says exactly what to set when a key or endpoint is missing.
pub fn resolve_route(
    reference: &str,
    endpoint: &Endpoint,
    models: &[Value],
    cache: &Cache,
    cred: &dyn Fn(&str) -> Option<Cred>,
) -> Result<Route, String> {
    let model = reference.trim();
    if model.is_empty() {
        return Err("No image model configured — pick one in /settings → Image.".into());
    }
    let base = endpoint.base_url.trim();
    if !base.is_empty() {
        if endpoint.api_key.is_empty() {
            return Err(format!(
                "The custom image endpoint {base} has no key — set it in /settings → Image."
            ));
        }
        return Ok(Route {
            api: endpoint.api,
            base_url: base.to_owned(),
            cred: Cred::Key(endpoint.api_key.clone()),
            model_id: model.to_owned(),
            label: format!("{model} @ {base}"),
            output: None,
        });
    }
    let (provider, id) = split_ref(model)
        .ok_or_else(|| format!("\"{model}\" is not a provider/model-id reference."))?;
    match provider {
        "openrouter" => {
            let c = cred("openrouter").ok_or(
                "No OpenRouter key. Set Keys → OpenRouter key in /settings → Image, log in to OpenRouter, or export OPENROUTER_API_KEY.",
            )?;
            Ok(Route {
                api: ImageApi::OpenRouter,
                base_url: String::new(),
                cred: c,
                model_id: id.to_owned(),
                label: model.to_owned(),
                output: cache.output_of(model),
            })
        }
        "fal" => {
            let c = cred("fal")
                .ok_or("No fal key. Set Keys → fal key in /settings → Image, or export FAL_KEY.")?;
            Ok(Route {
                api: ImageApi::Fal,
                base_url: String::new(),
                cred: c,
                model_id: id.to_owned(),
                label: model.to_owned(),
                output: None,
            })
        }
        _ => {
            let base = provider_base_url(models, provider).ok_or_else(|| {
                format!("Provider \"{provider}\" is not configured in Oino, so there is no endpoint for {model}. Pick an openrouter/ or fal/ model, or set a custom endpoint.")
            })?;
            let c = cred(provider).ok_or_else(|| {
                format!(
                    "No key for provider \"{provider}\". Log in to it or set a custom endpoint key."
                )
            })?;
            Ok(Route {
                api: ImageApi::OpenAiImages,
                base_url: base,
                cred: c,
                model_id: id.to_owned(),
                label: model.to_owned(),
                output: None,
            })
        }
    }
}

// ── vision (recognize) ──────────────────────────────────────────────────

/// The model accepts image input.
#[must_use]
pub fn is_vision(model: &Value) -> bool {
    model.get("id").and_then(Value::as_str).is_some()
        && model.get("provider").and_then(Value::as_str).is_some()
        && model
            .get("input")
            .and_then(Value::as_array)
            .is_some_and(|i| i.iter().any(|m| m == "image"))
}

/// Active tools with `tool` hidden for vision models and restored for text-only ones
/// (`None` = unchanged).
#[must_use]
pub fn apply_recognize_gating(active: &[String], model: &Value, tool: &str) -> Option<Vec<String>> {
    let vision = is_vision(model);
    let present = active.iter().any(|t| t == tool);
    if vision != present {
        return None;
    }
    Some(if vision {
        active.iter().filter(|t| *t != tool).cloned().collect()
    } else {
        let mut v = active.to_vec();
        v.push(tool.to_owned());
        v
    })
}

fn full(m: &Value) -> String {
    format!(
        "{}/{}",
        m.get("provider")
            .and_then(Value::as_str)
            .unwrap_or_default(),
        m.get("id").and_then(Value::as_str).unwrap_or_default()
    )
    .to_lowercase()
}

/// A vision model by reference (exact, then id, then substring).
///
/// # Errors
///
/// Why the reference does not name a usable vision model.
pub fn resolve_vision_model<'a>(reference: &str, models: &'a [Value]) -> Result<&'a Value, String> {
    let vision: Vec<&Value> = models.iter().filter(|m| is_vision(m)).collect();
    let q = reference.trim().to_lowercase();
    let id = |m: &Value| {
        m.get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_lowercase()
    };
    if let Some(hit) = vision
        .iter()
        .find(|m| full(m) == q)
        .or_else(|| vision.iter().find(|m| id(m) == q))
        .or_else(|| vision.iter().find(|m| full(m).contains(&q)))
    {
        return Ok(hit);
    }
    let list: Vec<String> = vision.iter().take(5).map(|m| full(m)).collect();
    let known = models.iter().any(|m| full(m) == q || id(m) == q);
    if known {
        return Err(format!(
            "Model \"{reference}\" does not accept image input. Vision models: {}",
            list.join(", ")
        ));
    }
    Err(format!(
        "Unknown vision model \"{reference}\". Vision models: {}",
        if list.is_empty() {
            "none configured".to_owned()
        } else {
            list.join(", ")
        }
    ))
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::assert_is_empty,
    clippy::format_collect,
    clippy::cloned_ref_to_slice_refs,
    clippy::duration_suboptimal_units,
    clippy::too_many_lines,
    clippy::needless_pass_by_value,
    reason = "tests assert by unwrapping"
)]
mod tests {
    use serde_json::json;

    use super::*;

    fn none() -> Endpoint {
        Endpoint::default()
    }

    fn creds(p: &str) -> Option<Cred> {
        match p {
            "openrouter" => Some(Cred::Key("set".into())),
            "fal" => Some(Cred::Key("fk".into())),
            "gateway" => Some(Cred::Headers(vec![(
                "authorization".into(),
                "Bearer ok".into(),
            )])),
            _ => None,
        }
    }

    #[test]
    fn routes_by_provider_with_the_right_key_source() {
        let models =
            vec![json!({"provider": "gateway", "id": "x", "baseUrl": "https://router/v1"})];
        let cache = Cache::default();
        let or = resolve_route(
            "openrouter/black-forest-labs/flux.2-klein-4b",
            &none(),
            &models,
            &cache,
            &creds,
        )
        .unwrap();
        assert_eq!(
            (or.api, or.cred, or.model_id.as_str()),
            (
                ImageApi::OpenRouter,
                Cred::Key("set".into()),
                "black-forest-labs/flux.2-klein-4b"
            )
        );
        let fal = resolve_route(
            "fal/fal-ai/flux-2/klein/4b",
            &none(),
            &models,
            &cache,
            &creds,
        )
        .unwrap();
        assert_eq!(
            (fal.api, fal.model_id.as_str()),
            (ImageApi::Fal, "fal-ai/flux-2/klein/4b")
        );
        let gw = resolve_route(
            "gateway/openrouter/black-forest-labs/flux.2-pro",
            &none(),
            &models,
            &cache,
            &creds,
        )
        .unwrap();
        assert_eq!(
            (gw.api, gw.base_url.as_str(), gw.model_id.as_str()),
            (
                ImageApi::OpenAiImages,
                "https://router/v1",
                "openrouter/black-forest-labs/flux.2-pro"
            )
        );
    }

    #[test]
    fn custom_endpoint_uses_the_model_id_as_typed() {
        let ep = Endpoint {
            base_url: "https://mine/v1".into(),
            api_key: "mk".into(),
            api: ImageApi::Fal,
        };
        let r = resolve_route("my-model", &ep, &[], &Cache::default(), &creds).unwrap();
        assert_eq!(
            (r.api, r.base_url.as_str(), r.cred, r.model_id.as_str()),
            (
                ImageApi::Fal,
                "https://mine/v1",
                Cred::Key("mk".into()),
                "my-model"
            )
        );
        let nokey = Endpoint {
            api_key: String::new(),
            ..ep
        };
        assert!(
            resolve_route("m", &nokey, &[], &Cache::default(), &creds)
                .unwrap_err()
                .contains("has no key")
        );
    }

    #[test]
    fn says_exactly_what_to_set() {
        let no = |_: &str| None;
        let c = Cache::default();
        assert!(
            resolve_route("openrouter/x", &none(), &[], &c, &no)
                .unwrap_err()
                .contains("OPENROUTER_API_KEY")
        );
        assert!(
            resolve_route("fal/x", &none(), &[], &c, &no)
                .unwrap_err()
                .contains("FAL_KEY")
        );
        assert!(
            resolve_route("nowhere/x", &none(), &[], &c, &no)
                .unwrap_err()
                .contains("not configured in Oino")
        );
        assert!(
            resolve_route("", &none(), &[], &c, &no)
                .unwrap_err()
                .contains("No image model")
        );
        assert!(
            resolve_route("bare", &none(), &[], &c, &no)
                .unwrap_err()
                .contains("not a provider/model-id")
        );
        let models = vec![json!({"provider": "p", "id": "x", "baseUrl": "https://p"})];
        assert!(
            resolve_route("p/x", &none(), &models, &c, &no)
                .unwrap_err()
                .contains("No key for provider")
        );
    }

    #[test]
    fn keeps_image_output_models_with_modalities() {
        let models = parse_openrouter_image_models(&json!({"data": [
            {"id": "black-forest-labs/flux.2-klein-4b", "architecture": {"input_modalities": ["text", "image"], "output_modalities": ["image"]}},
            {"id": "google/gemini-3.1-flash-image", "architecture": {"input_modalities": ["image", "text", "file"], "output_modalities": ["image", "text"]}},
            {"id": "deepseek/v4", "architecture": {"input_modalities": ["text"], "output_modalities": ["text"]}},
            {"id": "openrouter/auto", "architecture": {"output_modalities": ["image"]}},
        ]}));
        let got: Vec<(String, Vec<String>, Vec<String>)> = models
            .iter()
            .map(|m| (m.id.clone(), m.input.clone(), m.output.clone()))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    "black-forest-labs/flux.2-klein-4b".into(),
                    vec!["text".into(), "image".into()],
                    vec!["image".into()]
                ),
                (
                    "google/gemini-3.1-flash-image".into(),
                    vec!["image".into(), "text".into()],
                    vec!["image".into(), "text".into()]
                ),
            ]
        );
        assert!(parse_openrouter_image_models(&json!({})).is_empty());
        let cache = Cache {
            updated_at: 10,
            models,
        };
        assert_eq!(
            cache.output_of("openrouter/google/gemini-3.1-flash-image"),
            Some(vec!["image".into(), "text".into()])
        );
        assert!(!cache.stale(10 + CACHE_TTL_MS - 1));
        assert!(cache.stale(10 + CACHE_TTL_MS));
        assert!(Cache::default().stale(0));
        assert_eq!(catalog(&cache, true).len(), 8);
        assert_eq!(catalog(&cache, false).len(), 2);
    }

    #[test]
    fn recognize_gating() {
        let active = vec!["read".to_owned(), "image_recognize".to_owned()];
        let vision = json!({"id": "a", "provider": "p", "input": ["text", "image"]});
        let blind = json!({"id": "a", "provider": "p", "input": ["text"]});
        assert_eq!(
            apply_recognize_gating(&active, &vision, "image_recognize"),
            Some(vec!["read".to_owned()])
        );
        assert_eq!(
            apply_recognize_gating(&["read".to_owned()], &blind, "image_recognize"),
            Some(active.clone())
        );
        assert_eq!(
            apply_recognize_gating(&active, &blind, "image_recognize"),
            None
        );
        assert_eq!(
            apply_recognize_gating(&["read".to_owned()], &vision, "image_recognize"),
            None
        );
    }

    #[test]
    fn resolves_vision_models_and_rejects_blind_ones() {
        let reg = vec![
            json!({"provider": "p", "id": "eyes", "input": ["text", "image"]}),
            json!({"provider": "p", "id": "blind", "input": ["text"]}),
        ];
        assert_eq!(resolve_vision_model("p/eyes", &reg).unwrap()["id"], "eyes");
        assert_eq!(resolve_vision_model("EYES", &reg).unwrap()["id"], "eyes");
        assert_eq!(resolve_vision_model("ey", &reg).unwrap()["id"], "eyes");
        assert!(
            resolve_vision_model("p/blind", &reg)
                .unwrap_err()
                .contains("does not accept image input")
        );
        assert!(
            resolve_vision_model("zzz", &[])
                .unwrap_err()
                .contains("none configured")
        );
    }

    #[test]
    fn helpers() {
        assert_eq!(split_ref(" a/b/c "), Some(("a", "b/c")));
        assert_eq!(split_ref("/x"), None);
        assert_eq!(split_ref("x/"), None);
        assert_eq!(provider_env_var("gateway-x"), "GATEWAY_X_API_KEY");
        assert_eq!(fal_models().len(), 6);
    }
}
