//! App-local provider config surface. After the native
//! `oino-provider-openrouter` crate was removed (P3c cutover), these config
//! types live app-side: they describe how declarative provider contributions
//! resolve into `oino.provider_openai` (openai-compat component) requests.
//! The OpenRouter usage-report parsing is app-owned too.

use oino_auth::ProviderAuthSpec;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{collections::BTreeMap, time::Duration};

const DEFAULT_OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
const MODELS_PATH: &str = "/models";

/// OpenRouter account config (referer/title are forwarded as headers).
#[derive(Debug, Clone)]
pub struct OpenRouterConfig {
    pub base_url: String,
    pub referer: Option<String>,
    pub title: Option<String>,
    pub timeout: Duration,
}

impl Default for OpenRouterConfig {
    fn default() -> Self {
        Self {
            base_url: DEFAULT_OPENROUTER_BASE_URL.into(),
            referer: None,
            title: None,
            timeout: Duration::from_secs(120),
        }
    }
}

impl OpenRouterConfig {
    #[must_use]
    pub fn openai_compatible_config(&self) -> OpenAiCompatibleConfig {
        let mut config = OpenAiCompatibleConfig::new(
            oino_auth::OPENROUTER_PROVIDER_ID,
            "OpenRouter",
            self.base_url.clone(),
        )
        .with_auth(OpenAiCompatibleAuth::Bearer {
            spec: ProviderAuthSpec::openrouter(),
        })
        .with_timeout(self.timeout);
        if let Some(referer) = &self.referer {
            config = config.with_header("HTTP-Referer", referer.clone());
        }
        if let Some(title) = &self.title {
            config = config.with_header("X-OpenRouter-Title", title.clone());
        }
        config
    }
}

/// Auth spec for an OpenAI-compatible provider endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenAiCompatibleAuth {
    None,
    Bearer { spec: ProviderAuthSpec },
    OptionalBearer { spec: ProviderAuthSpec },
}

/// Resolved config for an OpenAI-compatible provider contribution.
#[derive(Debug, Clone)]
pub struct OpenAiCompatibleConfig {
    pub provider_id: String,
    pub display_name: String,
    pub base_url: String,
    pub timeout: Duration,
    pub auth: OpenAiCompatibleAuth,
    pub headers: BTreeMap<String, String>,
    /// Send `prompt_cache_key` derived from `StreamRequest::session_id`.
    /// `None` = auto: enabled only for `openrouter.ai` / `api.openai.com` hosts.
    pub send_prompt_cache_key: Option<bool>,
    /// Session-affinity header format. `None` = auto: `OpenRouter` when the
    /// host is `openrouter.ai`, disabled otherwise (matches pi compat detect).
    pub session_affinity: Option<SessionAffinityFormat>,
    /// Anthropic-style `cache_control` breakpoints. `None` = auto: enabled for
    /// models whose name starts with `anthropic/` or `~anthropic/`.
    pub cache_control_format: Option<CacheControlFormat>,
}

/// Anthropic-style cache breakpoint format for OpenAI-compatible requests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // config surface: set by declarative provider configs/tests
pub enum CacheControlFormat {
    Anthropic,
}

/// Session-affinity header format (mirrors pi `sessionAffinityFormat`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // config surface: set by declarative provider configs/tests
pub enum SessionAffinityFormat {
    /// `session_id`, `x-client-request-id`, `x-session-affinity`.
    OpenAi,
    /// `x-session-id`.
    OpenRouter,
}

impl OpenAiCompatibleConfig {
    #[must_use]
    pub fn new(
        provider_id: impl Into<String>,
        display_name: impl Into<String>,
        base_url: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            display_name: display_name.into(),
            base_url: base_url.into(),
            timeout: Duration::from_secs(120),
            auth: OpenAiCompatibleAuth::None,
            headers: BTreeMap::new(),
            send_prompt_cache_key: None,
            session_affinity: None,
            cache_control_format: None,
        }
    }

    #[must_use]
    pub fn from_profile(profile: oino_provider_catalog::OpenAiCompatibleProfile) -> Self {
        let credential = profile.credential_spec();
        let auth = credential
            .env_var
            .map(|env_var| {
                ProviderAuthSpec::new(credential.provider_id, credential.auth_key, env_var)
            })
            .map(|spec| {
                if profile.requires_api_key {
                    OpenAiCompatibleAuth::Bearer { spec }
                } else {
                    OpenAiCompatibleAuth::OptionalBearer { spec }
                }
            })
            .unwrap_or(OpenAiCompatibleAuth::None);
        Self::new(profile.id, profile.display_name, profile.api_base).with_auth(auth)
    }

    #[must_use]
    pub fn from_provider(provider: oino_provider_catalog::ProviderDescriptor) -> Option<Self> {
        match provider.target {
            oino_provider_catalog::ProviderTarget::OpenRouter => {
                Some(OpenRouterConfig::default().openai_compatible_config())
            }
            oino_provider_catalog::ProviderTarget::OpenAiApiKey => {
                let credential = provider.credential_spec();
                credential.env_var.map(|env_var| {
                    Self::new(
                        provider.id,
                        provider.display_name,
                        "https://api.openai.com/v1",
                    )
                    .with_auth(OpenAiCompatibleAuth::Bearer {
                        spec: ProviderAuthSpec::new(
                            credential.provider_id,
                            credential.auth_key,
                            env_var,
                        ),
                    })
                })
            }
            oino_provider_catalog::ProviderTarget::OpenAiCompatible { profile_id } => {
                oino_provider_catalog::openai_compatible_profile_by_id(profile_id)
                    .map(|profile| Self::from_profile(*profile))
            }
            _ => None,
        }
    }

    #[must_use]
    pub fn models_endpoint(&self) -> String {
        format!("{}{}", self.base_url.trim_end_matches('/'), MODELS_PATH)
    }

    #[must_use]
    pub fn with_auth(mut self, auth: OpenAiCompatibleAuth) -> Self {
        self.auth = auth;
        self
    }

    #[must_use]
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    #[must_use]
    pub fn with_header(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.headers.insert(name.into(), value.into());
        self
    }
}

/// `/models` response entry (OpenRouter-shaped).
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct OpenRouterModelInfo {
    pub id: String,
    pub name: Option<String>,
    #[serde(default)]
    pub owned_by: Option<String>,
    #[serde(default)]
    pub supported_parameters: Vec<String>,
    #[serde(default)]
    pub context_length: Option<usize>,
}

/// OpenRouter `/key` + `/credits` usage report (moved from
/// `oino-provider-openrouter` — same shape, same semantics).
#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterUsageReport {
    pub limits: Vec<OpenRouterUsageLimit>,
    pub extra_info: Vec<(String, String)>,
    pub balance: Option<OpenRouterCreditBalance>,
    pub hard_limit_reached: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterUsageLimit {
    pub name: String,
    pub usage_percent: f64,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpenRouterCreditBalance {
    pub amount: f64,
    pub currency: String,
    pub total_credits: f64,
    pub total_usage: f64,
}

#[allow(dead_code)] // exercised by usage tests
pub fn parse_openrouter_usage_payloads(
    key_payload: Option<&str>,
    credits_payload: Option<&str>,
) -> Result<OpenRouterUsageReport, String> {
    let key = key_payload
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|err| format!("invalid key usage fixture: {err}"))?;
    let credits = credits_payload
        .map(serde_json::from_str::<Value>)
        .transpose()
        .map_err(|err| format!("invalid credits usage fixture: {err}"))?;
    Ok(parse_openrouter_usage_values(
        key.as_ref(),
        credits.as_ref(),
    ))
}

pub fn parse_openrouter_usage_values(
    key_payload: Option<&Value>,
    credits_payload: Option<&Value>,
) -> OpenRouterUsageReport {
    let mut report = OpenRouterUsageReport {
        limits: Vec::new(),
        extra_info: Vec::new(),
        balance: None,
        hard_limit_reached: false,
    };

    if let Some(data) = credits_payload.and_then(openrouter_data_object) {
        let total_credits = data
            .get("total_credits")
            .and_then(openrouter_f64)
            .unwrap_or(0.0);
        let total_usage = data
            .get("total_usage")
            .and_then(openrouter_f64)
            .unwrap_or(0.0);
        let balance = total_credits - total_usage;
        if total_credits > 0.0 {
            report.limits.push(OpenRouterUsageLimit {
                name: "Credits".into(),
                usage_percent: usage_percent_from_used_limit(total_usage, total_credits),
                resets_at: None,
            });
            report.balance = Some(OpenRouterCreditBalance {
                amount: balance,
                currency: "USD".into(),
                total_credits,
                total_usage,
            });
            report.extra_info.push((
                "Balance".into(),
                format!("${balance:.2} / ${total_credits:.2}"),
            ));
        }
    }

    if let Some(data) = key_payload.and_then(openrouter_data_object) {
        for (label, key) in [
            ("Today", "usage_daily"),
            ("This week", "usage_weekly"),
            ("This month", "usage_monthly"),
        ] {
            if let Some(value) = data.get(key).and_then(openrouter_f64) {
                report
                    .extra_info
                    .push((label.into(), format!("${value:.2}")));
            }
        }
        if let Some(limit) = data
            .get("limit")
            .and_then(openrouter_f64)
            .filter(|value| *value > 0.0)
        {
            let remaining = data
                .get("limit_remaining")
                .and_then(openrouter_f64)
                .unwrap_or(0.0);
            let used = (limit - remaining).max(0.0);
            report.hard_limit_reached = remaining <= 0.0;
            report.limits.push(OpenRouterUsageLimit {
                name: "Key limit".into(),
                usage_percent: usage_percent_from_used_limit(used, limit),
                resets_at: None,
            });
            report.extra_info.push((
                "Key limit".into(),
                format!("${remaining:.2} remaining / ${limit:.2}"),
            ));
        }
    }

    report
}

fn openrouter_data_object(value: &Value) -> Option<&serde_json::Map<String, Value>> {
    value.get("data").unwrap_or(value).as_object()
}

fn openrouter_f64(value: &Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str()?.trim().parse().ok())
}

fn usage_percent_from_used_limit(used: f64, limit: f64) -> f64 {
    if !used.is_finite() || !limit.is_finite() || limit <= 0.0 {
        return 0.0;
    }
    (used / limit * 100.0).clamp(0.0, 100.0)
}
