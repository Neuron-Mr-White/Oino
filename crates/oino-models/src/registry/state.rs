//! Registry data and the composed catalog (pi `ModelRuntime` provider composition).

use indexmap::{IndexMap, IndexSet};
use oino_ai::Model;

use super::auth::{ApiKeyPlan, AuthPlan, StandardAuth};
use crate::compose::{compose_models, configured_headers, merge_compat};
use crate::models_json::ModelsJson;
use crate::registration::{ProviderConfigOverride, ProviderModels, ProviderRegistration};
use crate::virtual_models::VirtualModelRegistry;

/// A provider component's registration plus its model list.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RegisteredProvider {
    pub(crate) registration: ProviderRegistration,
    pub(crate) extension_id: String,
    /// Models from `list-models` (or the cache) for dynamic registrations.
    pub(crate) dynamic_models: Vec<Model>,
}

impl RegisteredProvider {
    /// Registration models with provider/api/baseUrl/compat defaults applied.
    pub(crate) fn base_models(&self) -> Vec<Model> {
        let registration = &self.registration;
        match &registration.models {
            ProviderModels::Dynamic => self
                .dynamic_models
                .iter()
                .map(|model| {
                    let mut model = model.clone();
                    model.provider.clone_from(&registration.id);
                    model
                })
                .collect(),
            ProviderModels::Static(models) => models
                .iter()
                .map(|definition| {
                    let api = definition
                        .api
                        .clone()
                        .unwrap_or_else(|| registration.api.clone());
                    let base_url = definition
                        .base_url
                        .clone()
                        .or_else(|| registration.base_url.clone())
                        .unwrap_or_default();
                    let mut model = definition
                        .clone()
                        .into_model(&registration.id, api, base_url);
                    model.compat =
                        merge_compat(registration.compat.as_ref(), model.compat.as_ref());
                    model
                })
                .collect(),
        }
    }

    fn base_auth(&self) -> StandardAuth {
        let registration = &self.registration;
        StandardAuth {
            api_key: registration.has_api_key_auth().then(|| ApiKeyPlan {
                inherited_env: Some(registration.auth_env.clone()),
                raw_key: None,
                raw_key_from_override: false,
            }),
            oauth: registration.oauth,
            raw_headers: None,
            auth_header: false,
        }
    }
}

/// One provider of the composed catalog.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ComposedProvider {
    pub(crate) id: String,
    pub(crate) name: String,
    /// Physical models (minus those hidden by virtual ids) followed by virtual models.
    pub(crate) models: Vec<Model>,
    pub(crate) auth: AuthPlan,
}

/// Mutable registry state behind the `RwLock`.
#[derive(Debug, Default)]
pub(crate) struct State {
    pub(crate) registrations: IndexMap<String, RegisteredProvider>,
    pub(crate) overrides: IndexMap<String, ProviderConfigOverride>,
    pub(crate) api_implementors: IndexMap<String, String>,
    pub(crate) virtual_models: VirtualModelRegistry,
    pub(crate) models_json: ModelsJson,
    pub(crate) providers: Vec<ComposedProvider>,
    pub(crate) composition_errors: IndexMap<String, String>,
}

impl State {
    /// pi `providerIds()` order: registrations, models.json, overrides, virtual providers.
    fn provider_ids(&self) -> IndexSet<String> {
        let mut ids: IndexSet<String> = self.registrations.keys().cloned().collect();
        ids.extend(self.models_json.provider_ids().map(str::to_owned));
        ids.extend(self.overrides.keys().cloned());
        ids.extend(self.virtual_models.provider_ids().map(str::to_owned));
        ids
    }

    /// pi `rebuildProviders`: recomposes every provider.
    pub(crate) fn recompose(&mut self) {
        self.composition_errors.clear();
        let mut providers = Vec::new();
        for id in self.provider_ids() {
            if let Some(provider) = self.compose_provider(&id) {
                providers.push(provider);
            }
        }
        self.providers = providers;
    }

    fn compose_provider(&mut self, id: &str) -> Option<ComposedProvider> {
        let registered = self.registrations.get(id);
        let config = self.models_json.provider(id);
        let override_config = self.overrides.get(id);
        let base_models = registered.map(RegisteredProvider::base_models);
        let untouched =
            || registered.map(|r| (base_models.clone().unwrap_or_default(), r.base_auth()));
        let physical: Option<(Vec<Model>, StandardAuth)> =
            if config.is_none() && override_config.is_none() {
                // No overlays: the registration untouched.
                untouched()
            } else {
                match compose_models(
                    id,
                    base_models.as_deref().unwrap_or_default(),
                    config,
                    override_config,
                ) {
                    Ok(models) => {
                        let inherited = registered
                            .filter(|r| r.registration.has_api_key_auth())
                            .map(|r| r.registration.auth_env.clone());
                        let override_key = override_config.and_then(|o| o.api_key.clone());
                        let raw_key_from_override = override_key.is_some();
                        let raw_key =
                            override_key.or_else(|| config.and_then(|c| c.api_key.clone()));
                        let oauth = registered.is_some_and(|r| r.registration.oauth);
                        let api_key = (inherited.is_some() || raw_key.is_some() || !oauth)
                            .then_some(ApiKeyPlan {
                                inherited_env: inherited,
                                raw_key,
                                raw_key_from_override,
                            });
                        let auth = StandardAuth {
                            api_key,
                            oauth,
                            raw_headers: configured_headers(config, override_config),
                            auth_header: override_config
                                .and_then(|o| o.auth_header)
                                .or_else(|| config.and_then(|c| c.auth_header))
                                .unwrap_or(false),
                        };
                        Some((models, auth))
                    }
                    Err(error) => {
                        self.composition_errors.insert(id.to_owned(), error);
                        untouched()
                    }
                }
            };
        let name = override_config
            .and_then(|o| o.name.clone())
            .or_else(|| config.and_then(|c| c.name.clone()))
            .or_else(|| registered.and_then(|r| r.registration.name.clone()))
            .unwrap_or_else(|| id.to_owned());
        let virtual_models: Vec<Model> = self.virtual_models.models_for(id).cloned().collect();
        match physical {
            Some((models, auth)) => {
                let mut models: Vec<Model> = models
                    .into_iter()
                    .filter(|m| !virtual_models.iter().any(|v| v.id == m.id))
                    .collect();
                models.extend(virtual_models);
                Some(ComposedProvider {
                    id: id.to_owned(),
                    name,
                    models,
                    auth: AuthPlan::Standard(auth),
                })
            }
            None if !virtual_models.is_empty() => Some(ComposedProvider {
                id: id.to_owned(),
                name,
                models: virtual_models,
                auth: AuthPlan::Virtual,
            }),
            None => None,
        }
    }

    pub(crate) fn provider(&self, id: &str) -> Option<&ComposedProvider> {
        self.providers.iter().find(|p| p.id == id)
    }

    pub(crate) fn find(&self, provider: &str, id: &str) -> Option<&Model> {
        self.provider(provider)?.models.iter().find(|m| m.id == id)
    }

    /// Extension whose component streams `model`: the provider's own component when it
    /// implements the model's API, else the component declaring that API.
    pub(crate) fn route_target(&self, model: &Model) -> Option<&str> {
        if let Some(registered) = self.registrations.get(&model.provider) {
            let own = registered.registration.api == model.api
                || self.api_implementors.get(&model.api) == Some(&registered.extension_id);
            if own {
                return Some(&registered.extension_id);
            }
        }
        self.api_implementors.get(&model.api).map(String::as_str)
    }
}
