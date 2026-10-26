//! Component-side driver (feature `guest`): runs an exchange over the host's `http` import,
//! with `provider-host` hooks, provider-level retries and cancellation, and lists models.
//! [`oauth`] holds the login-flow plumbing (loopback callbacks, device-code polling).

pub mod oauth;

use std::time::Duration;

use oino_ai::ProviderEvent;
use oino_extension::http::{self, Method, Response};
use oino_extension::provider::{self, EventSink, ProviderStream};
use oino_extension::{Error, Result, host, timers, wit_bindgen};
use serde_json::Value;

use crate::catalog::{Catalog, CatalogProvider, personalize_open_router};
use crate::emit::Emitter;
use crate::request::{Headers, ProviderRequest};
use crate::retry::{is_retryable, retry_delay_ms};
use crate::{PrepareOptions, Wire, anthropic, finalize, prepare, sigv4};

/// Provider environment the components read when the host grants it (`permissions.env`) and
/// the request's `options.env` does not set it (pi `getProviderEnvValue`).
pub const PROVIDER_ENV: &[&str] = &[
    "CLOUDFLARE_ACCOUNT_ID",
    "CLOUDFLARE_GATEWAY_ID",
    "AZURE_OPENAI_BASE_URL",
    "AZURE_OPENAI_RESOURCE_NAME",
    "AZURE_OPENAI_API_VERSION",
    "AZURE_OPENAI_DEPLOYMENT_NAME_MAP",
    "AWS_REGION",
    "AWS_DEFAULT_REGION",
    "AWS_PROFILE",
    "AWS_ACCESS_KEY_ID",
    "AWS_SECRET_ACCESS_KEY",
    "AWS_SESSION_TOKEN",
    "AWS_BEARER_TOKEN_BEDROCK",
    "AWS_BEDROCK_SKIP_AUTH",
    "AWS_BEDROCK_FORCE_CACHE",
    "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI",
    "AWS_CONTAINER_CREDENTIALS_FULL_URI",
    "AWS_WEB_IDENTITY_TOKEN_FILE",
    "GOOGLE_CLOUD_PROJECT",
    "GCLOUD_PROJECT",
    "GOOGLE_CLOUD_LOCATION",
    "GOOGLE_CLOUD_API_KEY",
    "GOOGLE_APPLICATION_CREDENTIALS",
];

/// Default request timeout (the provider SDKs use 600 s).
const DEFAULT_TIMEOUT_MS: u64 = 600_000;
const READ_CHUNK: usize = 16 * 1024;

/// How a component streams one request.
#[derive(Clone, Copy, Debug, Default)]
pub struct StreamOptions {
    /// Wire to use instead of the model's `api` (Oino Router).
    pub wire: Option<Wire>,
    /// Allow requests without a credential (local servers).
    pub allow_missing_key: bool,
}

fn env_long_retention() -> bool {
    ["OINO_CACHE_RETENTION", "PI_CACHE_RETENTION"]
        .iter()
        .any(|name| host::env(name).ok().flatten().as_deref() == Some("long"))
}

/// Parses a stream request and merges the granted host environment into `env`.
///
/// # Errors
/// Invalid request JSON.
pub fn parse(request: &Value) -> Result<ProviderRequest> {
    let mut request = ProviderRequest::parse(request).map_err(Error::msg)?;
    request.env_long_retention = env_long_retention();
    for name in PROVIDER_ENV {
        if request.env.contains_key(*name) {
            continue;
        }
        if let Some(value) = host::env(name).ok().flatten().filter(|v| !v.is_empty()) {
            request.env.insert((*name).to_owned(), value);
        }
    }
    Ok(request)
}

/// `provider.stream`: parses the request and spawns the producer. Every failure after parsing
/// is reported as an `error` event (pi semantics).
///
/// # Errors
/// Invalid request JSON or a model whose API this component does not implement.
pub fn stream(request: &Value, options: StreamOptions) -> Result<ProviderStream> {
    stream_request(parse(request)?, options)
}

/// A stream that fails before any request (pi setup errors): a lone `error` event.
#[must_use]
pub fn fail_stream(request: &ProviderRequest, message: String) -> ProviderStream {
    let mut out = Emitter::new(&request.model, timers::now_ms());
    out.fail(message, false);
    let events = out.drain();
    ProviderStream::spawn(move |sink| async move {
        let mut sink = Sink {
            inner: sink,
            open: true,
        };
        sink.send(events).await;
    })
}

/// [`stream`] for an already parsed (and possibly component-adjusted) request.
///
/// # Errors
/// A model whose API this component does not implement.
pub fn stream_request(request: ProviderRequest, options: StreamOptions) -> Result<ProviderStream> {
    let wire = options
        .wire
        .or_else(|| Wire::from_api(&request.model.api))
        .ok_or_else(|| {
            Error::msg(format!(
                "No provider extension implements API \"{}\"",
                request.model.api
            ))
        })?;
    Ok(ProviderStream::spawn(move |sink| async move {
        run(wire, request, options, sink).await;
    }))
}

struct Sink {
    inner: EventSink,
    open: bool,
}

impl Sink {
    async fn send(&mut self, events: Vec<ProviderEvent>) {
        for event in events {
            if !self.open {
                return;
            }
            self.open = self.inner.send(&event).await;
        }
    }
}

fn jitter() -> f64 {
    let millis = timers::now_ms().rem_euclid(1000);
    #[allow(clippy::cast_precision_loss, reason = "value below 1000")]
    let fraction = millis as f64 / 1000.0;
    fraction
}

async fn sleep_ms(ms: u64) {
    timers::sleep(Duration::from_millis(ms)).await;
}

#[allow(clippy::too_many_lines, reason = "one linear request lifecycle")]
async fn run(wire: Wire, request: ProviderRequest, options: StreamOptions, sink: EventSink) {
    let mut sink = Sink {
        inner: sink,
        open: true,
    };
    let now = timers::now_ms();
    let mut prepare_options = PrepareOptions {
        allow_missing_key: options.allow_missing_key,
        disable_preferred_strict: false,
    };
    let mut diagnostics = Vec::new();
    let max_retries = request.options.max_retries.unwrap_or(0);
    'attempt: loop {
        let mut prepared = match prepare(wire, &request, prepare_options, now) {
            Ok(prepared) => prepared,
            Err(message) => {
                let mut out = Emitter::new(&request.model, now);
                out.fail(message, false);
                sink.send(out.drain()).await;
                return;
            }
        };
        for diagnostic in diagnostics.drain(..) {
            prepared.exchange.output().diagnostic(diagnostic);
        }
        let headers = prepared.http.headers.clone().into_pairs();
        let (payload, headers) =
            provider::before_request(&request.request_id, &prepared.http.payload, headers).await;
        let mut headers = Headers::from(headers);
        let body = finalize(wire, payload, &mut headers);
        let body = serde_json::to_vec(&body).unwrap_or_default();
        if let Some(signing) = &prepared.http.signing {
            sigv4::sign(
                &prepared.http.url,
                &mut headers,
                &body,
                signing,
                &sigv4::amz_date(timers::now_ms()),
            );
        }
        let timeout = prepared.http.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS);
        let mut retries = 0u32;
        let response: Response = loop {
            let mut builder = http::request(Method::Post, &prepared.http.url)
                .body(body.clone())
                .timeout_ms(u32::try_from(timeout).unwrap_or(u32::MAX));
            for (name, value) in headers.pairs() {
                builder = builder.header(name, value);
            }
            match builder.send().await {
                Err(error) => {
                    let detail = error.to_string();
                    if retries < max_retries
                        && is_retryable(None, &[])
                        && let Ok(delay) = retry_delay_ms(
                            &[],
                            retries,
                            request.options.max_retry_delay_ms,
                            jitter(),
                            &detail,
                        )
                    {
                        retries += 1;
                        sleep_ms(delay).await;
                        continue;
                    }
                    host::log(
                        host::LogLevel::Warn,
                        &format!("{} request failed: {detail}", request.model.provider),
                    );
                    sink.send(prepared.exchange.connection_error(&detail)).await;
                    return;
                }
                Ok(response) => {
                    let status = response.status();
                    provider::after_response(&request.request_id, status, &response.head.headers);
                    if (200..300).contains(&status) {
                        break response;
                    }
                    let head = response.head.headers.clone();
                    let bytes = response.bytes().await;
                    let text = String::from_utf8_lossy(&bytes).into_owned();
                    if wire == Wire::Anthropic
                        && prepared.strict_fallback
                        && !prepare_options.disable_preferred_strict
                        && anthropic::is_strict_limit_error(status, &text)
                    {
                        let message = prepared
                            .exchange
                            .style()
                            .status_error(status, &text)
                            .message;
                        diagnostics.push(anthropic::strict_fallback_diagnostic(
                            &message,
                            timers::now_ms(),
                        ));
                        prepare_options.disable_preferred_strict = true;
                        continue 'attempt;
                    }
                    if retries < max_retries && is_retryable(Some(status), &head) {
                        let message = prepared
                            .exchange
                            .style()
                            .status_error(status, &text)
                            .message;
                        match retry_delay_ms(
                            &head,
                            retries,
                            request.options.max_retry_delay_ms,
                            jitter(),
                            &message,
                        ) {
                            Ok(delay) => {
                                retries += 1;
                                sleep_ms(delay).await;
                                continue;
                            }
                            Err(too_long) => {
                                sink.send(prepared.exchange.fail(too_long, false)).await;
                                return;
                            }
                        }
                    }
                    sink.send(prepared.exchange.http_error_with(status, &head, &bytes))
                        .await;
                    return;
                }
            }
        };
        sink.send(prepared.exchange.start_with(&response.head.headers))
            .await;
        let mut body = response.body();
        while sink.open && !prepared.exchange.is_finished() {
            let (status, chunk) = body.read(Vec::with_capacity(READ_CHUNK)).await;
            if !chunk.is_empty() {
                let events = prepared.exchange.feed(&chunk);
                if request.want_stream_events {
                    for raw in prepared.exchange.take_raw() {
                        provider::stream_event(&request.request_id, &raw);
                    }
                } else {
                    prepared.exchange.take_raw();
                }
                sink.send(events).await;
            }
            if matches!(status, wit_bindgen::StreamResult::Dropped) {
                break;
            }
        }
        if sink.open {
            let events = prepared.exchange.finish();
            if request.want_stream_events {
                for raw in prepared.exchange.take_raw() {
                    provider::stream_event(&request.request_id, &raw);
                }
            }
            sink.send(events).await;
        }
        return;
    }
}

/// Bearer credential from `list-models`' credential JSON (`{key}`, `{access}` or a string).
#[must_use]
pub fn credential_key(credential: Option<&Value>) -> Option<String> {
    let credential = credential?;
    credential
        .as_str()
        .or_else(|| credential["key"].as_str())
        .or_else(|| credential["apiKey"].as_str())
        .or_else(|| credential["access"].as_str())
        .filter(|k| !k.is_empty())
        .map(str::to_owned)
}

async fn get_json(url: &str, key: Option<&str>) -> Result<(u16, Value)> {
    let mut builder = http::get(url)
        .header("accept", "application/json")
        .timeout_ms(15_000);
    if let Some(key) = key {
        builder = builder.header("authorization", &format!("Bearer {key}"));
    }
    let response = builder.send().await?;
    let status = response.status();
    let bytes = response.bytes().await;
    Ok((
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    ))
}

/// `provider.list-models` for a catalog provider: static models (`OpenRouter` personalised with
/// `/models/user` when a key is given) or the endpoint's `/models` for dynamic providers.
///
/// # Errors
/// Unknown provider, or a failing `/models` request of a dynamic provider.
pub async fn list_models(
    catalog: &Catalog,
    provider_id: &str,
    credential: Option<&Value>,
) -> Result<Value> {
    let provider = catalog
        .provider(provider_id)
        .ok_or_else(|| Error::msg(format!("unknown provider {provider_id}")))?;
    let key = credential_key(credential);
    if provider.is_dynamic() {
        return dynamic_models(provider, key.as_deref()).await;
    }
    let mut models = provider.static_models();
    if provider.id == crate::providers::GITHUB_COPILOT {
        models = filter_copilot_models(models, credential);
    }
    if provider.id == "openrouter"
        && let Some(key) = key
    {
        let url = format!("{}/models/user", provider.base_url.trim_end_matches('/'));
        let (status, body) = get_json(&url, Some(&key)).await?;
        return match status {
            200..=299 => Ok(Value::Array(personalize_open_router(models, &body))),
            401 | 403 | 404 => Ok(Value::Array(models)),
            other => Err(Error::msg(format!(
                "OpenRouter model list request failed: {other}"
            ))),
        };
    }
    Ok(Value::Array(models))
}

/// pi `filterModels` for GitHub Copilot: an OAuth credential's `availableModelIds` (when it is
/// a list of strings) restricts the catalog.
#[must_use]
pub fn filter_copilot_models(models: Vec<Value>, credential: Option<&Value>) -> Vec<Value> {
    let Some(credential) = credential.filter(|c| c["type"] == "oauth") else {
        return models;
    };
    let Some(ids) = credential["availableModelIds"].as_array() else {
        return models;
    };
    let Some(ids) = ids.iter().map(Value::as_str).collect::<Option<Vec<&str>>>() else {
        return models;
    };
    models
        .into_iter()
        .filter(|m| m["id"].as_str().is_some_and(|id| ids.contains(&id)))
        .collect()
}

async fn dynamic_models(provider: &CatalogProvider, key: Option<&str>) -> Result<Value> {
    let url = format!("{}/models", provider.base_url.trim_end_matches('/'));
    let (status, body) = get_json(&url, key).await?;
    if !(200..300).contains(&status) {
        return Err(Error::msg(format!(
            "{} model list request failed: {status}",
            provider.name
        )));
    }
    let rows = body["data"]
        .as_array()
        .or_else(|| body["models"].as_array())
        .cloned()
        .unwrap_or_default();
    Ok(Value::Array(
        rows.iter()
            .filter_map(|row| provider.model_from_listing(row))
            .collect(),
    ))
}

/// Registers every catalog provider (`registry.register-provider`).
///
/// # Errors
/// A rejected registration.
pub fn register_all(catalog: &Catalog) -> Result<()> {
    for provider in &catalog.providers {
        oino_extension::wit::registry::register_provider(&provider.registration().to_string())?;
    }
    Ok(())
}
