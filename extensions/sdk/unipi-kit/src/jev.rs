//! jev decision-model client (unipi `src/jev/{client,settings}.ts`, unipi-port §2.4).
//!
//! Transports: `TypeSafe` native `/v1/systemone` (1 000 ms) and the `OpenRouter` decisions endpoint
//! `/api/alpha/decisions` (6 000 ms). Fail-open: any error, timeout, missing key or bad answer
//! is `None`. Shared settings live in namespace `decision-model` (registered by `unipi.core`);
//! each consumer may carry a `decisionModel` block with `source: inherit|custom`.
//!
//! Key order (first hit wins): the stored key of the resolved settings → secret
//! `decision-model/apiKey` in the caller's own secret store → the shared `decision-model`
//! `apiKey` secret field through the host settings registry (the namespace lets `unipi.*` read
//! it) → env `TYPESAFE_API_KEY` / `OPENROUTER_API_KEY` (permission `env`) → `credential-for`
//! (provider matching the base URL).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::settings::{Choice, Field, Kind, Namespace, Section, Unit, choice, get_field};

/// Settings namespace of the shared decision model.
pub const NAMESPACE: &str = "decision-model";
/// Secret name of the shared key in a consumer's own secret store.
pub const SECRET_API_KEY: &str = "decision-model/apiKey";
/// Native transport time box.
pub const TIMEOUT_NATIVE_MS: u32 = 1_000;
/// Decisions-endpoint time box.
pub const TIMEOUT_DECISIONS_MS: u32 = 6_000;
const DEFAULT_TYPESAFE_BASE: &str = "https://api.typesafe.ai";
const DEFAULT_OPENROUTER_BASE: &str = "https://openrouter.ai";

/// Transport provider.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    /// Native systemone.
    Typesafe,
    /// `OpenRouter` decisions endpoint (default).
    #[default]
    Openrouter,
    /// Own OpenRouter-shape gateway (needs a base URL).
    Custom,
}

impl Provider {
    fn parse(v: Option<&Value>) -> Option<Self> {
        match v?.as_str()? {
            "typesafe" => Some(Self::Typesafe),
            "openrouter" => Some(Self::Openrouter),
            "custom" => Some(Self::Custom),
            _ => None,
        }
    }

    /// Effective transport: only `typesafe` is native.
    #[must_use]
    pub fn native(self) -> bool {
        self == Self::Typesafe
    }
}

/// Decision model settings (unipi `DecisionModelSettings`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Transport.
    pub provider: Provider,
    /// Model id.
    pub model: String,
    /// Gateway base URL (`""` = provider default).
    pub base_url: String,
    /// Stored key (`""` = look elsewhere).
    pub api_key: String,
    /// Explicit time box; `0` = provider default.
    pub timeout_ms: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            provider: Provider::Openrouter,
            model: "typesafe/jev-1.13".into(),
            base_url: String::new(),
            api_key: String::new(),
            timeout_ms: 0,
        }
    }
}

impl Settings {
    /// unipi `normalize(raw, fallback)`.
    #[must_use]
    pub fn normalize(raw: &Value, fallback: &Self) -> Self {
        let provider = Provider::parse(raw.get("provider")).unwrap_or(fallback.provider);
        let text = |k: &str, d: &str| {
            raw.get(k)
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .unwrap_or(d)
                .to_owned()
        };
        let default_model = if provider == fallback.provider {
            fallback.model.as_str()
        } else if provider.native() {
            "jev-latest"
        } else {
            "typesafe/jev-1.13"
        };
        Self {
            provider,
            model: text("model", default_model),
            base_url: text("baseUrl", &fallback.base_url),
            api_key: text("apiKey", &fallback.api_key),
            timeout_ms: raw
                .get("timeoutMs")
                .and_then(Value::as_f64)
                .filter(|t| *t > 0.0)
                .map_or(fallback.timeout_ms, |t| {
                    #[allow(
                        clippy::cast_possible_truncation,
                        clippy::cast_sign_loss,
                        reason = "clamped"
                    )]
                    let ms = t.min(f64::from(u32::MAX)) as u32;
                    ms
                }),
        }
    }

    /// Shared settings from `extensionSettings` (pi settings object or namespace map).
    #[must_use]
    pub fn shared(extension_settings: &Value) -> Self {
        Self::normalize(
            extension_settings.get(NAMESPACE).unwrap_or(&Value::Null),
            &Self::default(),
        )
    }

    /// unipi `resolveDecisionModel`: the consumer's `decisionModel` block when `custom`, else
    /// the shared settings.
    #[must_use]
    pub fn resolve(extension_settings: &Value, namespace: Option<&str>) -> Self {
        let shared = Self::shared(extension_settings);
        let Some(ns) = namespace else { return shared };
        match extension_settings
            .get(ns)
            .and_then(|v| get_field(v, "decisionModel"))
        {
            Some(block) if block.get("source").and_then(Value::as_str) == Some("custom") => {
                Self::normalize(block, &shared)
            }
            _ => shared,
        }
    }

    /// Time box: explicit override, else provider default.
    #[must_use]
    pub fn timeout_ms(&self) -> u32 {
        if self.timeout_ms > 0 {
            self.timeout_ms
        } else if self.provider.native() {
            TIMEOUT_NATIVE_MS
        } else {
            TIMEOUT_DECISIONS_MS
        }
    }

    fn base(&self) -> String {
        let configured = self.base_url.trim();
        if configured.is_empty() {
            if self.provider.native() {
                DEFAULT_TYPESAFE_BASE
            } else {
                DEFAULT_OPENROUTER_BASE
            }
            .to_owned()
        } else {
            configured.trim_end_matches('/').to_owned()
        }
    }

    /// Native systemone endpoint.
    #[must_use]
    pub fn systemone_url(&self) -> String {
        format!("{}/v1/systemone", self.base())
    }

    /// Decisions endpoint; a base already ending in `/api/alpha`, `/vN` or `/api/vN` is the root.
    #[must_use]
    pub fn decisions_url(&self) -> String {
        let base = self.base();
        if ends_with_version_root(&base) {
            format!("{base}/decisions")
        } else {
            format!("{base}/api/alpha/decisions")
        }
    }

    /// Endpoint to call, or `None` when unsupported (custom without base, non-jev gateway model).
    #[must_use]
    pub fn endpoint(&self) -> Option<String> {
        if self.provider == Provider::Custom && self.base_url.trim().is_empty() {
            return None;
        }
        if self.provider.native() {
            Some(self.systemone_url())
        } else if is_decisions_model(&self.model) {
            Some(self.decisions_url())
        } else {
            None
        }
    }

    /// Env var consulted for the key.
    #[must_use]
    pub fn env_var(&self) -> &'static str {
        if self.provider.native() {
            "TYPESAFE_API_KEY"
        } else {
            "OPENROUTER_API_KEY"
        }
    }
}

/// unipi regex `/\/(?:api\/alpha|api\/)?v\d+$/` (plus `/api/alpha`).
fn ends_with_version_root(base: &str) -> bool {
    if base.ends_with("/api/alpha") {
        return true;
    }
    let Some(idx) = base.rfind('/') else {
        return false;
    };
    let last = &base[idx + 1..];
    let digits = last
        .strip_prefix('v')
        .filter(|d| !d.is_empty() && d.chars().all(|c| c.is_ascii_digit()));
    digits.is_some()
}

/// Decision-capable model on the openrouter-shape transports.
#[must_use]
pub fn is_decisions_model(model: &str) -> bool {
    let m = model.to_ascii_lowercase();
    m.starts_with("typesafe/") || m.contains("jev")
}

/// One answer: `choice`+`confidence` or `noul` (0–1), plus anything else jev returns.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Answer {
    /// Chosen label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub choice: Option<String>,
    /// Confidence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Number answer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub noul: Option<f64>,
    /// Other fields.
    #[serde(flatten)]
    pub extra: BTreeMap<String, Value>,
}

/// `answers.<questionId>`.
pub type Answers = BTreeMap<String, Answer>;

/// A prepared HTTP call (pure; tested natively).
#[derive(Clone, Debug, PartialEq)]
pub struct Prepared {
    /// URL.
    pub url: String,
    /// Headers.
    pub headers: Vec<(String, String)>,
    /// JSON body.
    pub body: Value,
    /// Time box.
    pub timeout_ms: u32,
}

/// Builds the call, or `None` when unsupported or the key is empty.
#[must_use]
pub fn prepare(settings: &Settings, state: &str, questions: &Value, key: &str) -> Option<Prepared> {
    let url = settings.endpoint()?;
    if key.trim().is_empty() {
        return None;
    }
    let mut headers = vec![
        ("content-type".to_owned(), "application/json".to_owned()),
        ("authorization".to_owned(), format!("Bearer {}", key.trim())),
    ];
    if !settings.provider.native() {
        headers.push(("x-title".to_owned(), "unipi-jev".to_owned()));
    }
    Some(Prepared {
        url,
        headers,
        body: json!({"state": state, "model": settings.model, "questions": questions}),
        timeout_ms: settings.timeout_ms(),
    })
}

/// Parses a response: non-2xx, bad JSON or a missing `answers` object → `None`.
#[must_use]
pub fn parse_answers(status: u16, body: &[u8]) -> Option<Answers> {
    if !(200..300).contains(&status) {
        return None;
    }
    let parsed: Value = serde_json::from_slice(body).ok()?;
    let answers = parsed.get("answers")?.as_object()?;
    Some(
        answers
            .iter()
            .filter_map(|(k, v)| Some((k.clone(), serde_json::from_value(v.clone()).ok()?)))
            .collect(),
    )
}

/// The key for `settings` in unipi-port §2.4 order; `None` when nothing is configured.
pub async fn resolve_key(settings: &Settings) -> Option<String> {
    let stored = settings.api_key.trim();
    if !stored.is_empty() {
        return Some(stored.to_owned());
    }
    if let Ok(Some(k)) = oino_extension::secrets::get(SECRET_API_KEY)
        && !k.trim().is_empty()
    {
        return Some(k);
    }
    if let Ok(shared) =
        oino_extension::settings::get(NAMESPACE, oino_extension::settings::Layer::Effective)
        && let Some(k) = shared.get("apiKey").and_then(Value::as_str)
        && !k.trim().is_empty()
    {
        return Some(k.to_owned());
    }
    if let Ok(Some(k)) = oino_extension::host::env(settings.env_var())
        && !k.trim().is_empty()
    {
        return Some(k);
    }
    let target = if settings.base_url.trim().is_empty() {
        if settings.provider.native() {
            DEFAULT_TYPESAFE_BASE.to_owned()
        } else {
            DEFAULT_OPENROUTER_BASE.to_owned()
        }
    } else {
        settings.base_url.trim().to_owned()
    };
    let cred = oino_extension::models::credential_for(&target)
        .await
        .ok()??;
    cred.headers.iter().find_map(|(k, v)| {
        k.eq_ignore_ascii_case("authorization")
            .then(|| v.strip_prefix("Bearer ").unwrap_or(v).to_owned())
    })
}

/// unipi `askJev`: one System One call; `None` = fail-open.
pub async fn ask(settings: &Settings, state: &str, questions: &Value) -> Option<Answers> {
    let key = resolve_key(settings).await?;
    let p = prepare(settings, state, questions, &key)?;
    let mut req = oino_extension::http::post(&p.url).timeout_ms(p.timeout_ms);
    for (k, v) in &p.headers {
        req = req.header(k, v);
    }
    let resp = req.body(p.body.to_string()).send().await.ok()?;
    let status = resp.status();
    let bytes = resp.bytes().await;
    parse_answers(status, &bytes)
}

fn provider_options() -> Vec<Choice> {
    vec![
        choice(
            "openrouter",
            "openrouter",
            "decisions endpoint for jev and gateway models",
        ),
        choice("typesafe", "typesafe", "native systemone transport"),
        choice(
            "custom",
            "custom",
            "your own OpenRouter-shape gateway (Base URL + key)",
        ),
    ]
}

/// Provider / model / base URL / key / timeout fields under `prefix` (unipi `modelFields`).
#[must_use]
pub fn model_fields(prefix: &str, empty_model: Option<&str>) -> Vec<Field> {
    let k = |name: &str| {
        if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}.{name}")
        }
    };
    let provider_key = k("provider");
    vec![
        Field::enumeration(&k("provider"), "Provider", provider_options())
            .description("Transport the judge call goes through."),
        Field::model(&k("model"), "Model")
            .description("A decision (classifier) model — jev; custom… for others.")
            .with_kind(|kind| {
                if let Kind::Model {
                    presets_by_provider,
                    provider_key: pk,
                    empty_option,
                    empty_label,
                    ..
                } = kind
                {
                    *presets_by_provider = Some(json!({
                        "typesafe": ["jev-latest"],
                        "openrouter": ["typesafe/jev-1.13"],
                        "custom": ["typesafe/jev-1.13"],
                    }));
                    *pk = Some(provider_key);
                    if let Some(e) = empty_model {
                        *empty_option = Some(e.to_owned());
                        *empty_label = Some(e.to_owned());
                    }
                }
            }),
        Field::string(&k("baseUrl"), "Base URL")
            .description("Gateway URL; required when provider = custom.")
            .empty_label("provider default"),
        Field::secret(&k("apiKey"), "API key")
            .description(
                "Stored key wins over the environment (OPENROUTER_API_KEY / TYPESAFE_API_KEY).",
            )
            .empty_label("environment"),
        Field::number(&k("timeoutMs"), "Timeout")
            .description("Abort budget for one judge call.")
            .with_kind(|kind| {
                if let Kind::Number {
                    unit,
                    min,
                    zero_label,
                    ..
                } = kind
                {
                    *unit = Some(Unit::Ms);
                    *min = Some(0.0);
                    *zero_label = Some("auto (1s native / 6s decisions)".into());
                }
            }),
    ]
}

/// The shared `decision-model` namespace (registered once, by `unipi.core`).
#[must_use]
pub fn namespace(extension_id: &str) -> Namespace {
    let d = Settings::default();
    Namespace::new(NAMESPACE, "Decision Model", extension_id)
        .secret_readers(&["unipi.*"])
        .defaults(json!({"provider": "openrouter", "model": d.model, "baseUrl": "", "timeoutMs": 0}))
        .section(
            Section::new("decision-model", "Decision model")
                .description(
                    "A fast calibrated classifier used for mode routing, permission judging, skill exposure, the watchdog, kanboard strategy and compaction pruning. Each of those can inherit this or use its own.",
                )
                .fields(model_fields("", None)),
        )
}

/// The section a consumer adds to its own namespace (unipi `decisionModelSection`).
#[must_use]
pub fn consumer_section(title: Option<&str>, advanced: bool) -> Section {
    Section::new("decision-model", title.unwrap_or("Decision model"))
        .description("inherit = the shared Decision Model settings · custom = override below for this module only (empty = shared value)")
        .advanced(advanced)
        .field(
            Field::enumeration(
                "decisionModel.source",
                "Decision model",
                vec![
                    choice("inherit", "inherit", "use the shared Decision Model settings"),
                    choice("custom", "custom", "override below for this module only"),
                ],
            )
            .description("Whether this module uses the shared Decision Model or its own."),
        )
        .fields(model_fields("decisionModel", Some("shared model")))
}

/// Defaults of a consumer's `decisionModel` block.
#[must_use]
pub fn consumer_defaults() -> Value {
    json!({"source": "inherit", "provider": "openrouter", "model": "", "baseUrl": "", "timeoutMs": 0})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(provider: Provider, model: &str, base: &str) -> Settings {
        Settings {
            provider,
            model: model.into(),
            base_url: base.into(),
            ..Settings::default()
        }
    }

    #[test]
    fn url_builders() {
        assert_eq!(
            s(Provider::Typesafe, "jev-latest", "").systemone_url(),
            "https://api.typesafe.ai/v1/systemone"
        );
        assert_eq!(
            s(Provider::Typesafe, "x", "https://gw.example/").systemone_url(),
            "https://gw.example/v1/systemone"
        );
        assert_eq!(
            s(Provider::Openrouter, "m", "").decisions_url(),
            "https://openrouter.ai/api/alpha/decisions"
        );
        assert_eq!(
            s(Provider::Custom, "m", "https://gw/api/alpha").decisions_url(),
            "https://gw/api/alpha/decisions"
        );
        assert_eq!(
            s(Provider::Custom, "m", "https://gw/v1").decisions_url(),
            "https://gw/v1/decisions"
        );
        assert_eq!(
            s(Provider::Custom, "m", "https://gw/api/v2").decisions_url(),
            "https://gw/api/v2/decisions"
        );
        assert_eq!(
            s(Provider::Custom, "m", "https://gw/vx").decisions_url(),
            "https://gw/vx/api/alpha/decisions"
        );
    }

    #[test]
    fn decisions_model_shape() {
        assert!(is_decisions_model("typesafe/jev-1.13"));
        assert!(is_decisions_model("acme/JEV-2"));
        assert!(!is_decisions_model("openai/gpt-5"));
        assert_eq!(
            s(Provider::Openrouter, "openai/gpt-5", "").endpoint(),
            None,
            "chat fallback stays with the caller"
        );
        assert_eq!(
            s(Provider::Custom, "typesafe/jev-1.13", "").endpoint(),
            None,
            "custom needs a base URL"
        );
        assert!(s(Provider::Typesafe, "anything", "").endpoint().is_some());
    }

    #[test]
    fn timeouts() {
        assert_eq!(s(Provider::Typesafe, "m", "").timeout_ms(), 1_000);
        assert_eq!(s(Provider::Openrouter, "m", "").timeout_ms(), 6_000);
        assert_eq!(
            Settings {
                timeout_ms: 250,
                ..Settings::default()
            }
            .timeout_ms(),
            250
        );
    }

    #[test]
    fn prepare_and_parse() {
        let st = s(Provider::Typesafe, "jev-latest", "");
        let p = prepare(&st, "state", &json!({"q": {"type": "noul"}}), " k ");
        let Some(p) = p else { unreachable!("prepared") };
        assert_eq!(p.url, "https://api.typesafe.ai/v1/systemone");
        assert!(
            p.headers
                .contains(&("authorization".into(), "Bearer k".into()))
        );
        assert!(!p.headers.iter().any(|(k, _)| k == "x-title"));
        assert_eq!(
            p.body,
            json!({"state": "state", "model": "jev-latest", "questions": {"q": {"type": "noul"}}})
        );
        assert_eq!(p.timeout_ms, 1_000);
        let or = prepare(
            &s(Provider::Openrouter, "typesafe/jev-1.13", ""),
            "s",
            &json!({}),
            "k",
        );
        assert!(or.is_some_and(|p| p.headers.contains(&("x-title".into(), "unipi-jev".into()))));
        assert!(
            prepare(&st, "s", &json!({}), "  ").is_none(),
            "missing key → no call"
        );

        let a = parse_answers(200, br#"{"answers": {"mode": {"choice": "goal", "confidence": 0.9}, "risk": {"noul": 0.2}}}"#);
        let Some(a) = a else { unreachable!("parsed") };
        assert_eq!(a["mode"].choice.as_deref(), Some("goal"));
        assert_eq!(a["risk"].noul, Some(0.2));
        assert!(parse_answers(500, b"{}").is_none());
        assert!(parse_answers(200, b"not json").is_none());
        assert!(parse_answers(200, br#"{"answers": 3}"#).is_none());
    }

    #[test]
    fn resolve_inherit_and_custom() {
        let ext = json!({
            "decision-model": {"provider": "typesafe", "apiKey": "shared"},
            "watchdog": {"decisionModel": {"source": "custom", "model": "", "timeoutMs": 300}},
            "workflow": {"decisionModel": {"source": "inherit", "model": "x"}},
        });
        let shared = Settings::shared(&ext);
        assert_eq!(shared.provider, Provider::Typesafe);
        assert_eq!(
            shared.model, "jev-latest",
            "provider switch picks its default model"
        );
        assert_eq!(shared.api_key, "shared");
        let w = Settings::resolve(&ext, Some("watchdog"));
        assert_eq!(w.timeout_ms, 300);
        assert_eq!(w.model, "jev-latest", "empty custom field = shared value");
        assert_eq!(w.api_key, "shared");
        assert_eq!(Settings::resolve(&ext, Some("workflow")), shared);
        assert_eq!(Settings::resolve(&json!({}), None), Settings::default());
    }

    #[test]
    fn schema_matches_unipi() {
        let ns = namespace("unipi.core").to_json();
        let fields = &ns["sections"][0]["fields"];
        let keys: Vec<&str> = fields
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|f| f["key"].as_str())
            .collect();
        assert_eq!(
            keys,
            ["provider", "model", "baseUrl", "apiKey", "timeoutMs"]
        );
        assert_eq!(fields[1]["providerKey"], "provider");
        assert_eq!(fields[0]["default"], "openrouter");
        let c = serde_json::to_value(consumer_section(None, true)).unwrap_or_default();
        assert_eq!(c["advanced"], true);
        assert_eq!(c["fields"][0]["key"], "decisionModel.source");
        assert_eq!(c["fields"][2]["emptyOption"], "shared model");
    }
}
