//! The provider stream request (`oino_protocol::ext::ProviderStreamRequest` JSON) and the
//! outbound HTTP request a wire builds from it.

use oino_ai::{Context, JsonValue, Model, StreamOptions, ThinkingLevel};
use serde::Deserialize;
use serde_json::Value;

/// `provider.stream` request: `{requestId, model, context, options, apiKey?, headers?,
/// wantStreamEvents?}`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderRequest {
    /// Request id for `provider-host` hooks (empty when the host sends none).
    #[serde(default)]
    pub request_id: String,
    /// Target model.
    pub model: Model,
    /// Normalised transcript.
    #[serde(default)]
    pub context: Context,
    /// pi `SimpleStreamOptions` subset.
    #[serde(default)]
    pub options: StreamOptions,
    /// Resolved credential.
    #[serde(default)]
    pub api_key: Option<String>,
    /// Extra request headers (`{name: value|null}` or `[[name, value]]`).
    #[serde(default)]
    pub headers: Option<JsonValue>,
    /// Whether to report raw provider events (`provider-host.stream-event`).
    #[serde(default)]
    pub want_stream_events: bool,
    /// Options pi-ai reads that `StreamOptions` does not model.
    #[serde(skip)]
    pub extra: ExtraOptions,
    /// `OINO_CACHE_RETENTION` / `PI_CACHE_RETENTION` is `long` in the host environment.
    #[serde(skip)]
    pub env_long_retention: bool,
}

/// Provider options outside `StreamOptions` (read from the raw `options` object).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExtraOptions {
    /// `OpenAI` `service_tier`.
    pub service_tier: Option<String>,
    /// `OpenAI` Responses `reasoning.summary`.
    pub reasoning_summary: Option<String>,
    /// Anthropic `thinking.display`.
    pub thinking_display: Option<String>,
    /// Anthropic interleaved-thinking beta (default on).
    pub interleaved_thinking: Option<bool>,
}

impl ProviderRequest {
    /// Parses the request JSON.
    ///
    /// # Errors
    /// When the JSON does not match the request shape.
    pub fn parse(value: &Value) -> Result<Self, String> {
        let mut request: Self = serde_json::from_value(value.clone())
            .map_err(|error| format!("invalid provider request: {error}"))?;
        let options = value.get("options");
        let text = |key: &str| {
            options
                .and_then(|o| o.get(key))
                .and_then(Value::as_str)
                .map(str::to_owned)
        };
        request.extra = ExtraOptions {
            service_tier: text("serviceTier"),
            reasoning_summary: text("reasoningSummary"),
            thinking_display: text("thinkingDisplay"),
            interleaved_thinking: options
                .and_then(|o| o.get("interleavedThinking"))
                .and_then(Value::as_bool),
        };
        // pi never passes "off" as a reasoning level; Oino hosts may.
        if request.options.reasoning == Some(ThinkingLevel::Off) {
            request.options.reasoning = None;
        }
        Ok(request)
    }

    /// Request-level header overrides in order (`None` deletes).
    #[must_use]
    pub fn header_overrides(&self) -> Vec<(String, Option<String>)> {
        let mut out: Vec<(String, Option<String>)> = self
            .options
            .headers
            .iter()
            .flatten()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        match &self.headers {
            Some(Value::Object(map)) => out.extend(
                map.iter()
                    .map(|(k, v)| (k.clone(), v.as_str().map(str::to_owned))),
            ),
            Some(Value::Array(list)) => out.extend(list.iter().filter_map(|pair| {
                let name = pair.get(0)?.as_str()?;
                Some((
                    name.to_owned(),
                    pair.get(1).and_then(Value::as_str).map(str::to_owned),
                ))
            })),
            _ => {}
        }
        out
    }

    /// pi `hasHeader`: a non-empty override for `name` exists.
    #[must_use]
    pub fn has_header(&self, name: &str) -> bool {
        self.header_overrides().iter().any(|(key, value)| {
            key.eq_ignore_ascii_case(name) && value.as_deref().is_some_and(|v| !v.trim().is_empty())
        })
    }
}

/// Ordered, case-insensitive header list (`null` overrides delete).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Headers(Vec<(String, String)>);

impl Headers {
    /// Empty list.
    #[must_use]
    pub const fn new() -> Self {
        Self(Vec::new())
    }

    /// Sets `name` (replacing any case variant).
    pub fn set(&mut self, name: &str, value: impl Into<String>) {
        self.remove(name);
        self.0.push((name.to_owned(), value.into()));
    }

    /// Removes `name`.
    pub fn remove(&mut self, name: &str) {
        self.0.retain(|(key, _)| !key.eq_ignore_ascii_case(name));
    }

    /// Applies an override (`None` deletes).
    pub fn apply(&mut self, name: &str, value: Option<&str>) {
        match value {
            Some(value) => self.set(name, value),
            None => self.remove(name),
        }
    }

    /// Value of `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&str> {
        self.0
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }

    /// The pairs.
    #[must_use]
    pub fn pairs(&self) -> &[(String, String)] {
        &self.0
    }

    /// Owned pairs.
    #[must_use]
    pub fn into_pairs(self) -> Vec<(String, String)> {
        self.0
    }
}

impl From<Vec<(String, String)>> for Headers {
    fn from(pairs: Vec<(String, String)>) -> Self {
        let mut headers = Self::new();
        for (name, value) in pairs {
            headers.set(&name, value);
        }
        headers
    }
}

/// An outbound request built by a wire.
#[derive(Clone, Debug, PartialEq)]
pub struct HttpRequest {
    /// Absolute URL.
    pub url: String,
    /// Headers (auth included).
    pub headers: Headers,
    /// JSON payload as pi's `onPayload` sees it (Anthropic `betas` still inline).
    pub payload: Value,
    /// Request timeout.
    pub timeout_ms: Option<u64>,
}

/// `User-Agent` sent by Oino provider components (pi `pi/<version> …`, product-substituted).
pub const USER_AGENT: &str = concat!("oino/", env!("CARGO_PKG_VERSION"));

/// Joins a base URL and a path like the provider SDKs.
#[must_use]
pub fn join_url(base: &str, path: &str) -> String {
    format!("{}{path}", base.trim_end_matches('/'))
}
