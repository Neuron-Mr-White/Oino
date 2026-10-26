//! pi `ModelRegistry` / `ModelRuntime` (registry and auth parts): the shared catalog of
//! provider registrations, `models.json`, config overrides and virtual models, plus credential
//! resolution per request.
//!
//! [`ModelRegistry`] is a cheap clonable handle (`Arc` + `RwLock`); core and extension host
//! calls use it concurrently. Mutations recompose the catalog synchronously, so reads always
//! see a consistent snapshot. Availability is computed on demand from `auth.json`, runtime
//! keys and the environment (no network).

mod auth;
mod state;

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard};

use indexmap::IndexMap;
use oino_ai::{AssistantMessage, JsonValue, Message, Model, ThinkingLevel};

pub use self::auth::{
    ApiKeyError, AuthCheck, AuthOverrides, AuthResolveError, AuthStatus, AuthStatusSource,
    ResolvedAuth,
};
use self::auth::{AuthPlan, Resolver};
use self::state::{RegisteredProvider, State};
use crate::auth::credential::CredentialType;
use crate::auth::oauth::OAuthRefresher;
use crate::auth::storage::AuthStorage;
use crate::cache::{CacheError, ModelCache};
use crate::compose::{configured_headers, merge_headers, raw_model_headers};
use crate::config_value::resolve_headers_strict;
use crate::env::{EnvSource, ProcessEnv};
use crate::models_json::ModelsJson;
use crate::registration::{ProviderConfigOverride, ProviderModels, ProviderRegistration};
use crate::virtual_models::{
    ModelRoute, RegisteredVirtualModel, RouteError, RouteFailed, RoutePrevious, RouteReason,
    RouteRequest, RouteResult, VirtualModelDefinition, VirtualModelError, find_latest_response,
    is_virtual_api,
};

/// Registry construction options.
#[derive(Clone)]
pub struct ModelRegistryOptions {
    /// `models.json` path (`None`: no file).
    pub models_json_path: Option<PathBuf>,
    /// Model list cache for dynamic registrations (`None`: no cache).
    pub model_cache: Option<ModelCache>,
    /// Environment for provider env vars and `$ENV` config values.
    pub env: Arc<dyn EnvSource>,
}

impl Default for ModelRegistryOptions {
    fn default() -> Self {
        Self {
            models_json_path: None,
            model_cache: None,
            env: Arc::new(ProcessEnv),
        }
    }
}

impl std::fmt::Debug for ModelRegistryOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRegistryOptions")
            .field("models_json_path", &self.models_json_path)
            .field("model_cache", &self.model_cache)
            .finish_non_exhaustive()
    }
}

impl ModelRegistryOptions {
    /// `<agent>/models.json` and `<agent>/cache/models/` with the process environment.
    #[must_use]
    pub fn for_agent_dir(agent_dir: &Path) -> Self {
        Self {
            models_json_path: Some(agent_dir.join("models.json")),
            model_cache: Some(ModelCache::for_agent_dir(agent_dir)),
            env: Arc::new(ProcessEnv),
        }
    }
}

/// Registry mutation failures.
#[derive(Debug, thiserror::Error)]
pub enum RegistryError {
    /// Empty provider id (pi text).
    #[error("Provider id must not be empty.")]
    EmptyProviderId,
    /// The provider is not registered by a component.
    #[error("Provider {0} is not registered.")]
    UnknownProvider(String),
    /// The provider's models are static.
    #[error("Provider {0} has a static model list.")]
    StaticModels(String),
    /// A config override does not compose (pi message).
    #[error("{0}")]
    InvalidConfig(String),
    /// Virtual model registration failed.
    #[error(transparent)]
    VirtualModel(#[from] VirtualModelError),
    /// Writing the model cache failed (the models were still applied).
    #[error(transparent)]
    Cache(#[from] CacheError),
}

/// Credentials for one request (pi `ResolvedRequestAuth` with `ok: true`).
#[derive(Clone, Default, PartialEq, Eq)]
pub struct RequestAuth {
    /// `StreamOptions.apiKey`.
    pub api_key: Option<String>,
    /// `StreamOptions.headers`.
    pub headers: Option<IndexMap<String, String>>,
}

impl std::fmt::Debug for RequestAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RequestAuth")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field(
                "headers",
                &self.headers.as_ref().map(|h| h.keys().collect::<Vec<_>>()),
            )
            .finish()
    }
}

/// `getApiKeyAndHeaders` failure (pi `{ok: false, error}`); the message is pi's text.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct RequestAuthError {
    /// User-facing message.
    pub message: String,
}

/// A non-fatal catalog problem outside pi's `getError()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryDiagnostic {
    /// Provider id.
    pub provider: String,
    /// Message.
    pub message: String,
}

struct Inner {
    state: RwLock<State>,
    auth: Arc<AuthStorage>,
    env: Arc<dyn EnvSource>,
    models_json_path: Option<PathBuf>,
    cache: Option<ModelCache>,
    refresher: RwLock<Option<Arc<dyn OAuthRefresher>>>,
}

/// The shared model registry (see module docs).
#[derive(Clone)]
pub struct ModelRegistry {
    inner: Arc<Inner>,
}

impl std::fmt::Debug for ModelRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModelRegistry")
            .field("models_json_path", &self.inner.models_json_path)
            .finish_non_exhaustive()
    }
}

impl ModelRegistry {
    /// Creates a registry and loads `models.json`.
    #[must_use]
    pub fn new(auth: Arc<AuthStorage>, options: ModelRegistryOptions) -> Self {
        let mut state = State {
            models_json: ModelsJson::load(options.models_json_path.as_deref()),
            ..State::default()
        };
        state.recompose();
        Self {
            inner: Arc::new(Inner {
                state: RwLock::new(state),
                auth,
                env: options.env,
                models_json_path: options.models_json_path,
                cache: options.model_cache,
                refresher: RwLock::new(None),
            }),
        }
    }

    /// In-memory credentials, no `models.json`, no cache, the given environment.
    #[must_use]
    pub fn in_memory(env: Arc<dyn EnvSource>) -> Self {
        let auth = Arc::new(AuthStorage::in_memory(IndexMap::new()).with_env(env.clone()));
        Self::new(
            auth,
            ModelRegistryOptions {
                env,
                ..ModelRegistryOptions::default()
            },
        )
    }

    fn read(&self) -> RwLockReadGuard<'_, State> {
        self.inner
            .state
            .read()
            .unwrap_or_else(PoisonError::into_inner)
    }

    fn write(&self) -> RwLockWriteGuard<'_, State> {
        self.inner
            .state
            .write()
            .unwrap_or_else(PoisonError::into_inner)
    }

    /// The credential store.
    #[must_use]
    pub fn auth_storage(&self) -> &Arc<AuthStorage> {
        &self.inner.auth
    }

    /// Installs the OAuth refresher (the host's bridge to `oauth-refresh` exports).
    pub fn set_oauth_refresher(&self, refresher: Arc<dyn OAuthRefresher>) {
        *self
            .inner
            .refresher
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Some(refresher);
    }

    fn refresher(&self) -> Option<Arc<dyn OAuthRefresher>> {
        self.inner
            .refresher
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// pi `refresh`: reloads `models.json` and recomposes the catalog.
    pub fn refresh(&self) {
        let models_json = ModelsJson::load(self.inner.models_json_path.as_deref());
        let mut state = self.write();
        state.models_json = models_json;
        state.recompose();
    }

    /// Replaces the `models.json` configuration without touching the file (tests, embedders).
    pub fn set_models_json(&self, models_json: ModelsJson) {
        let mut state = self.write();
        state.models_json = models_json;
        state.recompose();
    }

    /// pi `getError`: `models.json` load error and per-provider composition errors, joined by
    /// blank lines.
    #[must_use]
    pub fn error(&self) -> Option<String> {
        let state = self.read();
        let mut errors: Vec<String> = Vec::new();
        if let Some(error) = state.models_json.error() {
            errors.push(error.to_owned());
        }
        for (provider, error) in &state.composition_errors {
            errors.push(format!("Provider \"{provider}\": {error}"));
        }
        (!errors.is_empty()).then(|| errors.join("\n\n"))
    }

    /// Oino diagnostics: models whose API no loaded provider component implements (these
    /// models are never [`available`](Self::available)).
    #[must_use]
    pub fn diagnostics(&self) -> Vec<RegistryDiagnostic> {
        let state = self.read();
        let mut diagnostics: Vec<RegistryDiagnostic> = Vec::new();
        for provider in &state.providers {
            for model in &provider.models {
                if is_virtual_api(&model.api) || state.route_target(model).is_some() {
                    continue;
                }
                let message = format!("No provider extension implements api \"{}\"", model.api);
                if !diagnostics
                    .iter()
                    .any(|d| d.provider == provider.id && d.message == message)
                {
                    diagnostics.push(RegistryDiagnostic {
                        provider: provider.id.clone(),
                        message,
                    });
                }
            }
        }
        diagnostics
    }

    // ----- registrations -------------------------------------------------------------------

    /// Registers (or replaces) a provider component's registration (pi
    /// `registerNativeProvider`). A dynamic registration starts from the cached model list.
    ///
    /// # Errors
    ///
    /// [`RegistryError::EmptyProviderId`].
    pub fn register_provider(
        &self,
        extension_id: &str,
        registration: ProviderRegistration,
    ) -> Result<(), RegistryError> {
        if registration.id.trim().is_empty() {
            return Err(RegistryError::EmptyProviderId);
        }
        let dynamic_models = match (&registration.models, &self.inner.cache) {
            (ProviderModels::Dynamic, Some(cache)) => match cache.read(&registration.id) {
                Ok(entry) => entry.map(|e| e.models).unwrap_or_default(),
                Err(error) => {
                    tracing::warn!(%error, provider = %registration.id, "ignoring model cache");
                    Vec::new()
                }
            },
            _ => Vec::new(),
        };
        let mut state = self.write();
        state.registrations.insert(
            registration.id.clone(),
            RegisteredProvider {
                registration,
                extension_id: extension_id.to_owned(),
                dynamic_models,
            },
        );
        state.recompose();
        Ok(())
    }

    /// Declares that `extension_id`'s component implements `api` (manifest `provides.apis`).
    /// `models.json` providers using `api` stream through it.
    pub fn register_api_implementor(&self, api: &str, extension_id: &str) {
        self.write()
            .api_implementors
            .insert(api.to_owned(), extension_id.to_owned());
    }

    /// Sets a dynamic provider's models (after `list-models`) and writes the cache.
    ///
    /// # Errors
    ///
    /// Unknown or static provider; cache write failures (models are applied regardless).
    pub fn set_provider_models(
        &self,
        provider: &str,
        models: &[Model],
    ) -> Result<(), RegistryError> {
        {
            let mut state = self.write();
            let registered = state
                .registrations
                .get_mut(provider)
                .ok_or_else(|| RegistryError::UnknownProvider(provider.to_owned()))?;
            if !matches!(registered.registration.models, ProviderModels::Dynamic) {
                return Err(RegistryError::StaticModels(provider.to_owned()));
            }
            registered.dynamic_models = models.to_vec();
            state.recompose();
        }
        if let Some(cache) = &self.inner.cache {
            cache.write(provider, models)?;
        }
        Ok(())
    }

    /// pi `registerProvider(name, config)`: validates the override on its own, then merges its
    /// defined fields over the previous override. The provider's registration is kept.
    ///
    /// # Errors
    ///
    /// [`RegistryError::InvalidConfig`] with pi's composition message; nothing is stored.
    pub fn register_provider_config(
        &self,
        provider: &str,
        config: ProviderConfigOverride,
    ) -> Result<(), RegistryError> {
        if provider.trim().is_empty() {
            return Err(RegistryError::EmptyProviderId);
        }
        let mut state = self.write();
        let base = state
            .registrations
            .get(provider)
            .map(RegisteredProvider::base_models)
            .unwrap_or_default();
        crate::compose::apply_override(
            provider,
            crate::compose::apply_models_json(
                provider,
                &base,
                state.models_json.provider(provider),
            )
            .map_err(RegistryError::InvalidConfig)?,
            Some(&config),
        )
        .map_err(RegistryError::InvalidConfig)?;
        let merged = match state.overrides.shift_remove(provider) {
            Some(previous) => previous.merged_with(config),
            None => config,
        };
        state.overrides.insert(provider.to_owned(), merged);
        state.recompose();
        Ok(())
    }

    /// pi `unregisterProvider`: removes the registration and the config override (virtual
    /// models stay).
    pub fn unregister_provider(&self, provider: &str) {
        let mut state = self.write();
        state.registrations.shift_remove(provider);
        state.overrides.shift_remove(provider);
        state.recompose();
    }

    /// Removes everything `extension_id` registered (providers, API implementations, virtual
    /// models), e.g. when the extension unloads.
    pub fn unregister_extension(&self, extension_id: &str) {
        let mut state = self.write();
        state
            .registrations
            .retain(|_, registered| registered.extension_id != extension_id);
        state.api_implementors.retain(|_, ext| ext != extension_id);
        state.virtual_models.remove_extension(extension_id);
        state.recompose();
    }

    /// pi `getRegisteredProviderIds`: providers with an override or a component registration.
    #[must_use]
    pub fn registered_providers(&self) -> Vec<String> {
        let state = self.read();
        let mut ids: Vec<String> = state.overrides.keys().cloned().collect();
        for id in state.registrations.keys() {
            if !ids.contains(id) {
                ids.push(id.clone());
            }
        }
        ids
    }

    /// A component registration.
    #[must_use]
    pub fn registration(&self, provider: &str) -> Option<ProviderRegistration> {
        self.read()
            .registrations
            .get(provider)
            .map(|r| r.registration.clone())
    }

    /// pi `getRegisteredProviderConfig`.
    #[must_use]
    pub fn registered_provider_config(&self, provider: &str) -> Option<ProviderConfigOverride> {
        self.read().overrides.get(provider).cloned()
    }

    // ----- catalog -------------------------------------------------------------------------

    /// pi `getAll`: every model, providers in registration order.
    #[must_use]
    pub fn all(&self) -> Vec<Model> {
        self.read()
            .providers
            .iter()
            .flat_map(|p| p.models.iter().cloned())
            .collect()
    }

    /// pi `getAvailable`: models whose provider has credentials and whose API a provider
    /// component implements (virtual models: provider credentials only).
    #[must_use]
    pub fn available(&self) -> Vec<Model> {
        let state = self.read();
        let mut available = Vec::new();
        for provider in &state.providers {
            if self.check_plan(&provider.id, &provider.auth).is_none() {
                continue;
            }
            available.extend(
                provider
                    .models
                    .iter()
                    .filter(|m| is_virtual_api(&m.api) || state.route_target(m).is_some())
                    .cloned(),
            );
        }
        available
    }

    /// pi `find`.
    #[must_use]
    pub fn find(&self, provider: &str, id: &str) -> Option<Model> {
        self.read().find(provider, id).cloned()
    }

    /// pi `getPhysicalModel`: a catalog model that is not virtual.
    #[must_use]
    pub fn find_physical(&self, provider: &str, id: &str) -> Option<Model> {
        self.find(provider, id).filter(|m| !is_virtual_api(&m.api))
    }

    /// Provider ids of the catalog in order.
    #[must_use]
    pub fn provider_ids(&self) -> Vec<String> {
        self.read().providers.iter().map(|p| p.id.clone()).collect()
    }

    /// pi `getProviderDisplayName`.
    #[must_use]
    pub fn provider_display_name(&self, provider: &str) -> String {
        self.read()
            .provider(provider)
            .map_or_else(|| provider.to_owned(), |p| p.name.clone())
    }

    /// The extension whose component streams `model` (`None`: no implementation loaded).
    #[must_use]
    pub fn route_target(&self, model: &Model) -> Option<String> {
        self.read().route_target(model).map(str::to_owned)
    }

    // ----- auth ----------------------------------------------------------------------------

    fn check_plan(&self, provider: &str, plan: &AuthPlan) -> Option<AuthCheck> {
        let credential = match self.inner.auth.get(provider) {
            Ok(credential) => credential,
            Err(error) => {
                tracing::warn!(%error, provider, "auth check failed");
                return None;
            }
        };
        auth::check(plan, credential.as_ref(), self.inner.env.as_ref())
    }

    fn plan(&self, provider: &str) -> Option<AuthPlan> {
        self.read().provider(provider).map(|p| p.auth.clone())
    }

    /// pi `checkAuth`: how a provider is authenticated, `None` when it is not.
    #[must_use]
    pub fn check_auth(&self, provider: &str) -> Option<AuthCheck> {
        let plan = self.plan(provider)?;
        self.check_plan(provider, &plan)
    }

    /// pi `hasConfiguredAuth`.
    #[must_use]
    pub fn has_configured_auth(&self, provider: &str) -> bool {
        self.check_auth(provider).is_some()
    }

    /// pi `isUsingOAuth`.
    #[must_use]
    pub fn is_using_oauth(&self, provider: &str) -> bool {
        self.check_auth(provider)
            .is_some_and(|c| c.kind == CredentialType::Oauth)
    }

    /// pi `getProviderAuthStatus`.
    #[must_use]
    pub fn auth_status(&self, provider: &str) -> AuthStatus {
        let plan = self
            .plan(provider)
            .unwrap_or(AuthPlan::Standard(auth::StandardAuth {
                api_key: None,
                oauth: false,
                raw_headers: None,
                auth_header: false,
            }));
        let check = self.check_plan(provider, &plan);
        auth::status(
            &plan,
            &self.inner.auth,
            provider,
            check.as_ref(),
            self.inner.env.as_ref(),
        )
    }

    /// pi `setRuntimeApiKey` (`--api-key`).
    pub fn set_runtime_api_key(&self, provider: &str, key: impl Into<String>) {
        self.inner.auth.set_runtime_api_key(provider, key);
    }

    /// pi `removeRuntimeApiKey`.
    pub fn remove_runtime_api_key(&self, provider: &str) {
        self.inner.auth.remove_runtime_api_key(provider);
    }

    /// pi `getAuth(provider)`: resolved credentials (refreshing OAuth tokens as needed).
    ///
    /// # Errors
    ///
    /// [`AuthResolveError`] when a configured credential cannot be resolved.
    pub async fn get_auth(
        &self,
        provider: &str,
        overrides: &AuthOverrides,
    ) -> Result<Option<ResolvedAuth>, AuthResolveError> {
        let Some(plan) = self.plan(provider) else {
            return Ok(None);
        };
        let refresher = self.refresher();
        Resolver {
            provider,
            plan: &plan,
            storage: &self.inner.auth,
            env: self.inner.env.as_ref(),
            refresher: refresher.as_deref(),
        }
        .resolve(overrides)
        .await
    }

    /// pi `getApiKeyForProvider`: the key, or `None` when unconfigured or failing.
    pub async fn get_api_key(&self, provider: &str) -> Option<String> {
        self.get_auth(provider, &AuthOverrides::default())
            .await
            .ok()
            .flatten()
            .and_then(|auth| auth.api_key)
    }

    /// pi `getApiKeyAndHeaders`: what core passes to the provider component as
    /// `StreamOptions.apiKey` + `headers`.
    ///
    /// # Errors
    ///
    /// [`RequestAuthError`] with pi's message.
    pub async fn get_api_key_and_headers(
        &self,
        model: &Model,
        overrides: &AuthOverrides,
    ) -> Result<RequestAuth, RequestAuthError> {
        let provider = model.provider.as_str();
        let to_error = |error: &AuthResolveError| RequestAuthError {
            message: error.request_message(provider),
        };
        let resolution = self
            .get_auth(provider, overrides)
            .await
            .map_err(|e| to_error(&e))?;
        let (config_headers, model_headers, auth_header) = {
            let state = self.read();
            let config = state.models_json.provider(provider);
            let override_config = state.overrides.get(provider);
            (
                configured_headers(config, override_config),
                raw_model_headers(&model.id, config, override_config),
                override_config
                    .and_then(|o| o.auth_header)
                    .or_else(|| config.and_then(|c| c.auth_header))
                    .unwrap_or(false),
            )
        };
        let description = format!("model \"{provider}/{}\"", model.id);
        let env = self.inner.env.as_ref();
        if let Some(resolution) = resolution {
            let headers = resolve_headers_strict(
                model_headers.as_ref(),
                &description,
                resolution.env.as_ref(),
                env,
            )
            .map_err(|e| to_error(&AuthResolveError::ModelHeaders(e)))?;
            return Ok(RequestAuth {
                api_key: resolution.api_key,
                headers: merge_headers(resolution.headers, headers),
            });
        }
        // pi compatibility path: no credentials resolved.
        let raw = match (config_headers, model_headers) {
            (None, None) => None,
            (base, over) => {
                let mut merged = base.unwrap_or_default();
                merged.extend(over.unwrap_or_default());
                Some(merged)
            }
        };
        let configured = resolve_headers_strict(raw.as_ref(), &description, None, env)
            .map_err(|e| to_error(&AuthResolveError::ModelHeaders(e)))?;
        if auth_header {
            return Err(RequestAuthError {
                message: format!("No API key found for \"{provider}\""),
            });
        }
        let headers = match (model.headers.clone(), configured) {
            (None, None) => None,
            (base, over) => {
                let mut merged = base.unwrap_or_default();
                merged.extend(over.unwrap_or_default());
                Some(merged)
            }
        };
        Ok(RequestAuth {
            api_key: None,
            headers,
        })
    }

    // ----- virtual models ------------------------------------------------------------------

    /// pi `registerVirtualModel`: registers or replaces a virtual model.
    ///
    /// # Errors
    ///
    /// Empty ids or a physical model with the same id.
    pub fn register_virtual_model(
        &self,
        extension_id: &str,
        definition: VirtualModelDefinition,
    ) -> Result<(), RegistryError> {
        let mut state = self.write();
        let is_physical = state
            .find(&definition.provider, &definition.id)
            .is_some_and(|m| !is_virtual_api(&m.api));
        state
            .virtual_models
            .register(extension_id, definition, is_physical)?;
        state.recompose();
        Ok(())
    }

    /// pi `unregisterVirtualModel`.
    pub fn unregister_virtual_model(&self, provider: &str, id: &str) {
        let mut state = self.write();
        if state.virtual_models.unregister(provider, id) {
            state.recompose();
        }
    }

    /// A registered virtual model (definition + owning extension).
    #[must_use]
    pub fn virtual_model(&self, provider: &str, id: &str) -> Option<RegisteredVirtualModel> {
        self.read().virtual_models.get(provider, id).cloned()
    }

    /// Builds the route request for a virtual model (pi `resolveModel` before the router call):
    /// `previous` from the latest successful response in `messages`, `failed` from a retry.
    ///
    /// # Errors
    ///
    /// [`RouteError::NotRegistered`].
    pub fn route_request(
        &self,
        model: &Model,
        messages: Vec<Message>,
        reason: RouteReason,
        thinking_level: ThinkingLevel,
        failed: Option<AssistantMessage>,
        state: Option<JsonValue>,
    ) -> Result<RouteRequest, RouteError> {
        if self.virtual_model(&model.provider, &model.id).is_none() {
            return Err(RouteError::NotRegistered {
                provider: model.provider.clone(),
                id: model.id.clone(),
            });
        }
        let previous = find_latest_response(&messages).and_then(|latest| {
            self.find_physical(&latest.provider, &latest.model)
                .map(|model| RoutePrevious {
                    model,
                    thinking_level: latest.thinking_level,
                })
        });
        let failed = failed.and_then(|message| {
            self.find_physical(&message.provider, &message.model)
                .map(|model| RouteFailed {
                    model,
                    thinking_level: message.thinking_level,
                    message,
                })
        });
        Ok(RouteRequest {
            model: model.clone(),
            thinking_level,
            reason,
            previous,
            failed,
            state: if reason == RouteReason::Direct {
                None
            } else {
                state
            },
            messages,
        })
    }

    /// Validates a router result (pi `resolveModel` after the call): the target must be a
    /// physical catalog model with credentials; the level is clamped to it.
    ///
    /// # Errors
    ///
    /// [`RouteError::NotPhysical`] / [`RouteError::NoCredentials`].
    pub fn validate_route(
        &self,
        model: &Model,
        result: RouteResult,
    ) -> Result<ModelRoute, RouteError> {
        let target = format!("{}/{}", result.model.provider, result.model.id);
        let physical = self
            .find_physical(&result.model.provider, &result.model.id)
            .ok_or_else(|| RouteError::NotPhysical {
                provider: model.provider.clone(),
                id: model.id.clone(),
                target: target.clone(),
            })?;
        if !self.has_configured_auth(&physical.provider) {
            return Err(RouteError::NoCredentials {
                provider: model.provider.clone(),
                id: model.id.clone(),
                target,
            });
        }
        let thinking_level = result
            .thinking_level
            .clamp(&physical.available_thinking_levels());
        Ok(ModelRoute {
            model: physical,
            thinking_level,
            state: result.state,
        })
    }
}

#[cfg(test)]
mod tests;
