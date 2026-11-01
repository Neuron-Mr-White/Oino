//! Provider readiness and the new-session default model (v2 QA D-01/D-37).
//!
//! Two questions, one answer each:
//!
//! - **"Which model will a new session use?"** ([`choose_default_model`]) — the configured
//!   default (`defaultProvider`/`defaultModel`) when its provider is *available* (registered,
//!   with credentials, and not known-broken), else the first usable model of a provider whose
//!   credential **was actually validated** — never again "a model that does not exist on this
//!   host" silently starting on whatever dead key sits in the environment. The choice is what
//!   `session_new` starts the runtime on and what `host_cmd › default_model` reports for the
//!   draft chip.
//! - **"Does this provider's credential actually work?"** ([`ReadinessCache`]) — a cached,
//!   lazily validated per-provider answer from a real authenticated request (the provider's
//!   own `GET /models`): `ready`, `invalid_key` (the API rejected the key — a dead
//!   `OPENROUTER_API_KEY` says `Ready` no more) or `unreachable`. Probing is bounded, runs in
//!   parallel and only for APIs with a known shape (`openai-*` bearer endpoints, openrouter's
//!   authenticated `/models/user`, anthropic's `x-api-key` `/v1/models`); everything else
//!   honestly stays `unknown` instead of guessing.
//!
//! The cache is per hub ([`SessionHub`]); `set_api_key`/`logout` invalidate the provider.

use std::collections::HashMap;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use oino_ai::Model;
use oino_models::resolver::DEFAULT_MODEL_PER_PROVIDER;
use oino_models::{AuthOverrides, ModelRegistry, ProviderRegistration, ResolvedAuth};

/// Upper bound of one readiness probe (a local server that never answers must not
/// hold a `session_new`).
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
/// How long a `ready` answer stands (revalidation is cheap and backgrounded).
const READY_TTL: Duration = Duration::from_secs(10 * 60);
/// How long a rejected key stands (a replaced key also invalidates explicitly).
const INVALID_TTL: Duration = Duration::from_secs(5 * 60);
/// How long an unreachable answer stands (local servers come and go).
const UNREACHABLE_TTL: Duration = Duration::from_secs(60);

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// One provider's cached validation result.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readiness {
    /// What the probe found.
    pub state: ReadinessState,
    /// The provider's own words when it refused (HTTP status line / error text).
    pub detail: Option<String>,
    /// When the probe ran.
    pub checked_at: SystemTime,
}

/// What a probe concluded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReadinessState {
    /// The endpoint answered and accepted the credential.
    Ready,
    /// The endpoint answered and rejected the credential (401/403).
    InvalidKey,
    /// The endpoint could not be reached (offline, down, timeout).
    Unreachable,
}

impl ReadinessState {
    /// Wire name (`auth_providers` rows, `default_model` reports).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::InvalidKey => "invalid_key",
            Self::Unreachable => "unreachable",
        }
    }

    /// How long this answer stands.
    #[must_use]
    const fn ttl(self) -> Duration {
        match self {
            Self::Ready => READY_TTL,
            Self::InvalidKey => INVALID_TTL,
            Self::Unreachable => UNREACHABLE_TTL,
        }
    }

    /// Whether a *new session* may start on a provider in this state (D-01: a rejected key
    /// never gets picked silently; an unreachable endpoint is indeterminate, not refused —
    /// offline new chats still work).
    #[must_use]
    const fn may_start_on(self) -> bool {
        !matches!(self, Self::InvalidKey)
    }
}

/// The per-hub cache of [`Readiness`] by provider id. Empty/absent = not validated yet.
#[derive(Default)]
pub struct ReadinessCache {
    entries: Mutex<HashMap<String, Readiness>>,
}

impl ReadinessCache {
    /// A fresh entry, read through by tests and [`SessionHub`].
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    fn entries(&self) -> MutexGuard<'_, HashMap<String, Readiness>> {
        lock(&self.entries)
    }

    /// The provider's fresh (within its TTL) readiness, when it was validated.
    #[must_use]
    pub fn get(&self, provider: &str) -> Option<Readiness> {
        self.entries()
            .get(provider)
            .filter(|readiness| {
                readiness
                    .checked_at
                    .elapsed()
                    .is_ok_and(|age| age < readiness.state.ttl())
            })
            .cloned()
    }

    /// Records a probe result.
    pub fn store(&self, provider: &str, state: ReadinessState, detail: Option<String>) {
        self.entries().insert(
            provider.to_owned(),
            Readiness {
                state,
                detail,
                checked_at: SystemTime::now(),
            },
        );
    }

    /// Drops a provider's answer (a new key was stored, a credential removed).
    pub fn invalidate(&self, provider: &str) {
        self.entries().remove(provider);
    }
}

/// The answer of [`choose_default_model`].
#[derive(Clone, Debug)]
pub struct DefaultModelChoice {
    /// The model a new session should start on (`None`: nothing usable).
    pub model: Option<Model>,
    /// `"settings"` (the configured default) | `"fallback"` (a validated provider) |
    /// `"fallback-unverified"` (a credentialed provider that could not be validated) |
    /// `"none"`.
    pub source: &'static str,
    /// Why the configured default was not used (reported to the UI).
    pub reason: Option<String>,
}

/// Whether the host knows how to authenticate a readiness probe for `api`, and against what.
enum ProbeShape {
    /// `GET {base}/models` with `Authorization: Bearer …` — the OpenAI-compatible listing,
    /// which rejects a bad key on every provider **except** openrouter.
    OpenAiModels,
    /// `POST {base}/chat/completions` (1 token, `openrouter/auto`): openrouter's `/models`
    /// and `/models/user` answer the public catalog for *any* key, so a minimal completion
    /// is the only request that actually says whether the key works (D-37).
    OpenRouterCompletion,
    /// `GET {base}/v1/models` with `x-api-key` (+ `anthropic-version`).
    Anthropic,
}

/// The probe shape for a provider registration, when there is one.
fn probe_shape(provider: &ProviderRegistration) -> Option<ProbeShape> {
    match provider.api.as_str() {
        api if api.starts_with("openai") => Some(if provider.id == "openrouter" {
            ProbeShape::OpenRouterCompletion
        } else {
            ProbeShape::OpenAiModels
        }),
        "anthropic-messages" if provider.id == "anthropic" => Some(ProbeShape::Anthropic),
        // OAuth-refreshed and signed APIs (google, bedrock, azure): no generic probe;
        // readiness stays honestly `unknown`.
        _ => None,
    }
}

/// Whether the provider's readiness can be probed at all.
#[must_use]
pub fn probeable(provider: &ProviderRegistration) -> bool {
    probe_shape(provider).is_some()
}

/// Runs one provider's readiness probe and caches it. Never fails: an error *is* the
/// answer (`unreachable`, with the text).
pub async fn probe_and_cache(cache: &ReadinessCache, models: &ModelRegistry, provider: &str) {
    let Some(registration) = models.registration(provider) else {
        return;
    };
    if !probeable(&registration) {
        return;
    }
    let (state, detail) = probe_provider(models, provider).await;
    cache.store(provider, state, detail);
}

/// Probes one provider (no caching): the classification plus the refusing status/error as
/// detail. `NotProbeable` collapses to `unreachable`-with-`None` only in the caller that
/// already checked [`probeable`].
async fn probe_provider(
    models: &ModelRegistry,
    provider: &str,
) -> (ReadinessState, Option<String>) {
    let Some(registration) = models.registration(provider) else {
        return (
            ReadinessState::Unreachable,
            Some("unknown provider".to_owned()),
        );
    };
    let Some(shape) = probe_shape(&registration) else {
        return (ReadinessState::Unreachable, None);
    };
    let auth = models
        .get_auth(provider, &AuthOverrides::default())
        .await
        .ok()
        .flatten();
    let base = registration.base_url.as_deref().unwrap_or_default();
    if base.is_empty() {
        return (ReadinessState::Unreachable, Some("no base URL".to_owned()));
    }
    let base = base.trim_end_matches('/');
    let client = match reqwest::Client::builder().timeout(PROBE_TIMEOUT).build() {
        Ok(client) => client,
        Err(error) => {
            return (
                ReadinessState::Unreachable,
                Some(format!("could not build the probe client: {error}")),
            );
        }
    };
    match &shape {
        ProbeShape::OpenAiModels => {
            let mut request = client.get(format!("{base}/models")).timeout(PROBE_TIMEOUT);
            if let Some(header) = bearer_header(auth.as_ref()) {
                request = request.header("authorization", header);
            }
            classify(request.send().await).await
        }
        ProbeShape::OpenRouterCompletion => {
            let mut request = client
                .post(format!("{base}/chat/completions"))
                .timeout(PROBE_TIMEOUT)
                .header("content-type", "application/json")
                .body(
                    serde_json::json!({
                        "model": "openrouter/auto",
                        "max_tokens": 1,
                        "messages": [{ "role": "user", "content": "ping" }],
                    })
                    .to_string(),
                );
            if let Some(header) = bearer_header(auth.as_ref()) {
                request = request.header("authorization", header);
            }
            classify(request.send().await).await
        }
        ProbeShape::Anthropic => {
            let mut request = client
                .get(format!("{base}/v1/models"))
                .timeout(PROBE_TIMEOUT)
                .header("anthropic-version", "2023-06-01");
            if let Some(key) = auth.as_ref().and_then(|auth| auth.api_key.as_deref()) {
                request = request.header("x-api-key", key);
            }
            classify(request.send().await).await
        }
    }
}

/// Maps one probe response: 2xx (and 429 — authenticated, only throttled) is `ready`,
/// 401/403/402 is a rejected key, anything else is indeterminate (`unreachable`).
async fn classify(
    response: Result<reqwest::Response, reqwest::Error>,
) -> (ReadinessState, Option<String>) {
    match response {
        Ok(response) => {
            let status = response.status();
            if status.is_success() || status.as_u16() == 429 {
                (ReadinessState::Ready, None)
            } else if (401..=403).contains(&status.as_u16()) {
                (
                    ReadinessState::InvalidKey,
                    Some(format!("HTTP {} {status}", status.as_u16())),
                )
            } else {
                (
                    ReadinessState::Unreachable,
                    Some(format!("HTTP {} {status}", status.as_u16())),
                )
            }
        }
        Err(error) => (ReadinessState::Unreachable, Some(error.to_string())),
    }
}

/// `Authorization: Bearer …` for the resolved credential (an OAuth token arrives as the
/// resolved `Authorization` header, an API key as `api_key`).
fn bearer_header(auth: Option<&ResolvedAuth>) -> Option<String> {
    let auth = auth?;
    if let Some(headers) = &auth.headers {
        if let Some((_, value)) = headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))
        {
            return Some(value.clone());
        }
    }
    auth.api_key
        .as_deref()
        .filter(|key| !key.is_empty())
        .map(|key| format!("Bearer {key}"))
}

/// The candidate order of the fallback: the engine's own preference
/// ([`DEFAULT_MODEL_PER_PROVIDER`] first, then catalog order) over the models that have
/// credentials and a loaded provider implementation.
fn fallback_candidates(models: &ModelRegistry) -> Vec<Model> {
    let available = models.available();
    let mut preferred: Vec<Model> = Vec::new();
    let mut rest = available.clone();
    for (provider, id) in DEFAULT_MODEL_PER_PROVIDER {
        if let Some(position) = rest
            .iter()
            .position(|model| model.provider == *provider && model.id == *id)
        {
            preferred.push(rest.remove(position));
        }
    }
    preferred.extend(rest);
    preferred
}

/// The model a new session will start on (D-01).
///
/// 1. The configured default (`defaultProvider` + `defaultModel`) when its provider is
///    registered, credentialed, and not known-broken (`invalid_key`; `unreachable` is
///    indeterminate and keeps the user's choice).
/// 2. Else the first candidate (engine preference order) whose provider validates `ready`,
///    probing (bounded, in parallel) when `allow_probing` and nothing is cached.
/// 3. Else the first candidate of a provider that cannot be probed generically (its state is
///    honestly `unknown`, not wrong).
/// 4. Else the first credentialed candidate anyway (`fallback-unverified` — the session
///    still gets a model; the UI shows the rejected key instead of pretending).
///
/// `allow_probing` is `false` for the instant `default_model` host command (the cache and a
/// background refresh inform the next read) and `true` on the `session_new` path, where the
/// choice must be right the first time.
pub async fn choose_default_model(
    models: &ModelRegistry,
    settings: &oino_settings::Settings,
    cache: &ReadinessCache,
    allow_probing: bool,
) -> DefaultModelChoice {
    // When probing is allowed (the `session_new` path), validate the default's provider and
    // every fallback candidate in ONE bounded parallel batch — never probe-then-probe
    // sequentially on the click path. Fresh cache entries are not probed again.
    if allow_probing {
        let mut providers: Vec<String> = Vec::new();
        let mut push = |provider: &str| {
            if cache.get(provider).is_none() && !providers.iter().any(|p| p == provider) {
                providers.push(provider.to_owned());
            }
        };
        if let Some(provider) = settings
            .default_provider()
            .filter(|provider| models.has_configured_auth(provider))
        {
            push(provider);
        }
        for model in fallback_candidates(models) {
            push(&model.provider);
        }
        probe_all(cache, models, &providers).await;
    }
    // 1. The configured default.
    let default_provider = settings.default_provider();
    let default_model_id = settings.default_model();
    if let (Some(provider), Some(id)) = (&default_provider, &default_model_id) {
        if let Some(found) = models.find(provider, id) {
            if !models.has_configured_auth(&found.provider) {
                return fallback(models, cache, "no credentials")
                    .await
                    .with_default(format!(
                        "the default model {provider}/{id} has no credentials"
                    ));
            }
            if let Some(state) = cache.get(&found.provider).map(|readiness| readiness.state) {
                if !state.may_start_on() {
                    return fallback(models, cache, state.as_str())
                        .await
                        .with_default(format!(
                            "the default model {provider}/{id} was rejected ({})",
                            state.as_str()
                        ));
                }
            }
            return DefaultModelChoice {
                model: Some(found),
                source: "settings",
                reason: None,
            };
        }
    }
    let why = match (&default_provider, &default_model_id) {
        (Some(provider), Some(id)) => format!("the default model {provider}/{id} is not available"),
        _ => "no default model is configured".to_owned(),
    };
    fallback(models, cache, "none").await.with_default(why)
}

/// Steps 2–4 of [`choose_default_model`], over whatever the cache holds (the up-front batch
/// in [`choose_default_model`] already probed when probing was allowed); `skipped` names the
/// readiness that ruled out the earlier candidates of the current pass (reported only when
/// nothing usable remains).
async fn fallback(
    models: &ModelRegistry,
    cache: &ReadinessCache,
    skipped: &'static str,
) -> DefaultModelChoice {
    let candidates = fallback_candidates(models);
    if candidates.is_empty() {
        return DefaultModelChoice {
            model: None,
            source: "none",
            reason: Some("no model on this PC has credentials".to_owned()),
        };
    }
    // 2. First validated-ready candidate.
    let mut unprobeable: Option<&Model> = None;
    let mut unverified: Option<&Model> = None;
    for model in &candidates {
        let registration = models.registration(&model.provider);
        let state = cache.get(&model.provider).map(|ready| ready.state);
        match state {
            Some(ReadinessState::Ready) => {
                return DefaultModelChoice {
                    model: Some(model.clone()),
                    source: "fallback",
                    reason: None,
                };
            }
            Some(ReadinessState::InvalidKey | ReadinessState::Unreachable) => {}
            None => {
                if registration
                    .as_ref()
                    .is_none_or(|registration| !probeable(registration))
                {
                    unprobeable.get_or_insert(model);
                } else {
                    unverified.get_or_insert(model);
                }
            }
        }
    }
    // 3. A provider the host cannot probe (OAuth/signing) is unknown, not wrong.
    if let Some(model) = unprobeable {
        return DefaultModelChoice {
            model: Some(model.clone()),
            source: "fallback",
            reason: None,
        };
    }
    // 4. Anything credentialed — with the state named for the UI.
    if let Some(model) = unverified.or(candidates.first()) {
        let state = cache
            .get(&model.provider)
            .map_or("unknown", |ready| ready.state.as_str());
        return DefaultModelChoice {
            model: Some(model.clone()),
            source: "fallback-unverified",
            reason: Some(format!(
                "no provider could be validated; using {}/{} ({state})",
                model.provider, model.id
            )),
        };
    }
    DefaultModelChoice {
        model: None,
        source: "none",
        reason: Some(format!("every credentialed provider was {skipped}")),
    }
}

impl DefaultModelChoice {
    fn with_default(mut self, reason: String) -> Self {
        if self.reason.is_none() {
            self.reason = Some(reason);
        } else {
            self.reason = Some(format!(
                "{reason}; {}",
                self.reason.take().unwrap_or_default()
            ));
        }
        self
    }
}

/// Probes every provider **concurrently** (each bounded by [`PROBE_TIMEOUT`]; the futures
/// share this task — they are network waits, not CPU) and caches the answers. Used by the
/// `session_new` fallback and `auth_providers{validate:true}`.
pub async fn probe_all(cache: &ReadinessCache, models: &ModelRegistry, providers: &[String]) {
    futures_util::future::join_all(
        providers
            .iter()
            .map(|provider| probe_and_cache(cache, models, provider)),
    )
    .await;
}

/// Epoch milliseconds of a readiness check (wire `checkedAt`).
#[must_use]
pub fn epoch_ms(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, |duration| {
        u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use oino_models::{MapEnv, ModelRegistry};
    use oino_models::{ProviderModelConfig, ProviderRegistration};
    use std::sync::Arc;

    fn registry(providers: &[(&str, &str, &str)]) -> ModelRegistry {
        // (id, api, base_url); every provider gets one static model named `<id>-m` and an
        // env-var auth plan, so a provider is configured exactly when its key is set.
        let models = ModelRegistry::in_memory(Arc::new(MapEnv::default()));
        for (id, api, base) in providers {
            models.register_api_implementor(api, "ext");
            let _ = models.register_provider(
                "ext",
                ProviderRegistration {
                    id: (*id).to_owned(),
                    name: Some((*id).to_owned()),
                    api: (*api).to_owned(),
                    base_url: Some((*base).to_owned()),
                    auth_env: vec![format!("{}_API_KEY", id.to_uppercase())],
                    auth_header: oino_models::AuthHeaderMode::None,
                    oauth: false,
                    headers: indexmap::IndexMap::new(),
                    compat: None,
                    models: oino_models::ProviderModels::Static(vec![model_config(id)]),
                },
            );
        }
        models
    }

    /// One static model `<id>-m` on provider `id` (only the fields the chooser reads).
    fn model_config(id: &str) -> ProviderModelConfig {
        ProviderModelConfig {
            id: format!("{id}-m"),
            name: None,
            api: None,
            base_url: None,
            reasoning: false,
            thinking_level_map: None,
            input: vec![oino_ai::InputModality::Text],
            input_limits: None,
            cost: oino_ai::ModelCost::default(),
            prompt_cache: None,
            context_window: 128_000,
            max_tokens: 16_384,
            sampling_params: None,
            sampling_params_by_thinking_level: None,
            headers: None,
            compat: None,
        }
    }

    fn settings_with_default(
        provider: Option<&str>,
        model: Option<&str>,
    ) -> oino_settings::Settings {
        let dir = tempfile::TempDir::new().unwrap();
        let mut manager =
            oino_settings::SettingsManager::create(std::path::Path::new("/"), dir.path(), false);
        if let (Some(provider), Some(model)) = (provider, model) {
            manager
                .set_default_model_and_provider(provider, model)
                .unwrap();
        }
        manager.snapshot()
    }

    #[test]
    fn cache_answers_within_the_ttl_and_expires_after() {
        let cache = ReadinessCache::new();
        assert!(cache.get("p").is_none());
        cache.store("p", ReadinessState::Ready, None);
        assert_eq!(cache.get("p").unwrap().state, ReadinessState::Ready);
        // A long-past check does not count as fresh anymore.
        lock(&cache.entries).insert(
            "old".to_owned(),
            Readiness {
                state: ReadinessState::Ready,
                detail: None,
                checked_at: SystemTime::UNIX_EPOCH,
            },
        );
        assert!(cache.get("old").is_none());
        cache.invalidate("p");
        assert!(cache.get("p").is_none());
    }

    #[test]
    fn bearer_header_prefers_the_resolved_header_then_the_api_key() {
        let mut headers = indexmap::IndexMap::new();
        headers.insert("Authorization".to_owned(), "Bearer tok".to_owned());
        let oauth = ResolvedAuth {
            api_key: None,
            headers: Some(headers),
            env: None,
            source: String::new(),
        };
        assert_eq!(bearer_header(Some(&oauth)).as_deref(), Some("Bearer tok"));
        let key = ResolvedAuth {
            api_key: Some("sk-1".to_owned()),
            headers: None,
            env: None,
            source: String::new(),
        };
        assert_eq!(bearer_header(Some(&key)).as_deref(), Some("Bearer sk-1"));
        assert_eq!(bearer_header(None), None);
    }

    #[test]
    fn only_known_api_shapes_are_probeable() {
        let registration = |id: &str, api: &str| ProviderRegistration {
            id: id.to_owned(),
            name: None,
            api: api.to_owned(),
            base_url: None,
            auth_env: Vec::new(),
            auth_header: oino_models::AuthHeaderMode::None,
            oauth: false,
            headers: indexmap::IndexMap::new(),
            compat: None,
            models: oino_models::ProviderModels::Dynamic,
        };
        assert!(probeable(&registration("deepseek", "openai-completions")));
        assert!(probeable(&registration("openai", "openai-responses")));
        // openrouter is probeable …
        assert!(probeable(&registration("openrouter", "openai-completions")));
        // … but the signed/oauth APIs stay honestly unknown.
        assert!(!probeable(&registration("google", "google-gemini")));
        assert!(!probeable(&registration("faux", "faux")));
        assert!(!probeable(&registration("vercel", "anthropic-messages")));
    }

    #[tokio::test]
    async fn chooses_the_configured_default_when_available() {
        let models = registry(&[("faux", "faux", "")]);
        models.set_runtime_api_key("faux", "k");
        let settings = settings_with_default(Some("faux"), Some("faux-m"));
        let cache = ReadinessCache::new();
        let choice = choose_default_model(&models, &settings, &cache, false).await;
        assert_eq!(choice.source, "settings");
        assert_eq!(choice.model.as_ref().unwrap().id, "faux-m");
        assert!(choice.reason.is_none());
    }

    #[tokio::test]
    async fn a_rejected_default_falls_back_to_a_validated_provider() {
        let models = registry(&[
            ("dead", "openai-completions", "https://dead.invalid"),
            ("live", "faux", ""),
        ]);
        models.set_runtime_api_key("dead", "k");
        models.set_runtime_api_key("live", "k");
        let settings = settings_with_default(Some("dead"), Some("dead-m"));
        let cache = ReadinessCache::new();
        // Without validation the rejected key is not known yet: the default still wins
        // (give the user's choice the benefit of the doubt).
        let choice = choose_default_model(&models, &settings, &cache, false).await;
        assert_eq!(choice.source, "settings");
        // Once rejected, the default is skipped and the validated provider is picked.
        cache.store(
            "dead",
            ReadinessState::InvalidKey,
            Some("HTTP 401".to_owned()),
        );
        let choice = choose_default_model(&models, &settings, &cache, false).await;
        assert_eq!(choice.source, "fallback");
        assert_eq!(choice.model.as_ref().unwrap().provider, "live");
        assert!(choice.reason.as_deref().unwrap().contains("was rejected"));
    }

    #[tokio::test]
    async fn without_a_default_a_ready_provider_wins_over_an_earlier_unknown_one() {
        let models = registry(&[
            ("first", "openai-completions", "https://first.invalid"),
            ("second", "openai-completions", "https://second.invalid"),
        ]);
        models.set_runtime_api_key("first", "k");
        models.set_runtime_api_key("second", "k");
        let settings = settings_with_default(None, None);
        let cache = ReadinessCache::new();
        cache.store("second", ReadinessState::Ready, None);
        let choice = choose_default_model(&models, &settings, &cache, false).await;
        assert_eq!(choice.source, "fallback");
        assert_eq!(choice.model.as_ref().unwrap().provider, "second");
    }

    #[tokio::test]
    async fn a_provider_the_host_cannot_probe_is_picked_as_unknown() {
        let models = registry(&[("faux", "faux", "")]);
        models.set_runtime_api_key("faux", "k");
        let settings = settings_with_default(None, None);
        let choice = choose_default_model(&models, &settings, &ReadinessCache::new(), false).await;
        assert_eq!(choice.source, "fallback");
        assert_eq!(choice.model.as_ref().unwrap().id, "faux-m");
    }

    #[tokio::test]
    async fn an_unverifiable_probeable_provider_is_reported_as_such() {
        let models = registry(&[("far", "openai-completions", "https://far.invalid")]);
        models.set_runtime_api_key("far", "k");
        let settings = settings_with_default(None, None);
        let choice = choose_default_model(&models, &settings, &ReadinessCache::new(), false).await;
        assert_eq!(choice.source, "fallback-unverified");
        assert!(choice.model.is_some());
        assert!(choice
            .reason
            .as_deref()
            .unwrap()
            .contains("no provider could be validated"));
    }

    #[tokio::test]
    async fn nothing_credentialed_means_no_model() {
        let models = registry(&[("faux", "faux", "")]);
        let settings = settings_with_default(None, None);
        let choice = choose_default_model(&models, &settings, &ReadinessCache::new(), false).await;
        assert_eq!(choice.source, "none");
        assert!(choice.model.is_none());
    }

    /// A local HTTP endpoint answering one canned status; the task returns the request
    /// headers it saw, to prove the probe's classification and auth shape end to end.
    async fn answering(status: u16) -> (String, tokio::task::JoinHandle<Vec<(String, String)>>) {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move {
            let mut seen: Vec<(String, String)> = Vec::new();
            if let Ok((mut socket, _)) = listener.accept().await {
                use tokio::io::{AsyncReadExt, AsyncWriteExt};
                let mut buf = [0u8; 4096];
                let _n = socket.read(&mut buf).await.unwrap_or(0);
                let text = String::from_utf8_lossy(&buf).to_string();
                if let Some(request_line) = text.lines().next() {
                    // `"post", "\/chat\/completions HTTP\/1.1"`-style record of the method+path.
                    if let Some((method, rest)) = request_line.split_once(' ') {
                        seen.push((method.to_lowercase(), rest.trim().to_owned()));
                    }
                }
                for line in text.lines().skip(1) {
                    if let Some((name, value)) = line.split_once(':') {
                        seen.push((name.trim().to_lowercase(), value.trim().to_owned()));
                    }
                }
                let body = "[]";
                let reply = format!(
                    "HTTP/1.1 {status} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\r\n{body}",
                    body.len()
                );
                let _ = socket.write_all(reply.as_bytes()).await;
                let _ = socket.shutdown().await;
            }
            seen
        });
        (format!("http://{}", addr), task)
    }

    #[tokio::test]
    async fn probe_classifies_a_rejected_key_and_sends_the_bearer_header() {
        let (base, task) = answering(401).await;
        let models = registry(&[("probe", "openai-completions", &base)]);
        models.set_runtime_api_key("probe", "sk-dead");
        let cache = ReadinessCache::new();
        probe_and_cache(&cache, &models, "probe").await;
        let seen = task.await.unwrap();
        let readiness = cache.get("probe").unwrap();
        assert_eq!(readiness.state, ReadinessState::InvalidKey);
        assert!(readiness.detail.as_deref().unwrap().contains("401"));
        let auth = seen
            .iter()
            .find(|(name, _)| name == "authorization")
            .map(|(_, value)| value.clone())
            .unwrap_or_default();
        assert_eq!(auth, "Bearer sk-dead");
    }

    #[tokio::test]
    async fn probe_accepts_a_2xx_and_openrouter_posts_a_minimal_completion() {
        let (base, task) = answering(200).await;
        let models = registry(&[("openrouter", "openai-completions", &base)]);
        models.set_runtime_api_key("openrouter", "sk-live");
        let cache = ReadinessCache::new();
        probe_and_cache(&cache, &models, "openrouter").await;
        let seen = task.await.unwrap();
        assert_eq!(
            cache.get("openrouter").unwrap().state,
            ReadinessState::Ready
        );
        // openrouter's model lists answer the public catalog for any key, so the probe is a
        // 1-token completion with the key (D-37).
        let request_line = seen
            .iter()
            .find(|(name, _)| name == "post")
            .map(|(_, v)| v.clone());
        assert_eq!(
            request_line.as_deref(),
            Some("/chat/completions HTTP/1.1"),
            "the openrouter probe must be a chat completion, not a model list"
        );
    }
}
