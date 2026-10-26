//! Credential checks and resolution per provider (pi `provider-composer.ts`
//! `composeApiKeyAuth`/`composeOAuthAuth`, pi-ai `auth/resolve.ts`, `envApiKeyAuth`).
//!
//! Order: runtime key (`--api-key`) > `auth.json` > `models.json`/override `apiKey` >
//! registration `authEnv` variables. A stored credential owns the provider: there is no
//! fallback to the environment after a failed OAuth refresh.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::auth::credential::{ApiKeyCredential, AuthCredential, CredentialType};
use crate::auth::oauth::{OAuthError, OAuthRefresher, ensure_fresh_oauth};
use crate::auth::storage::{AuthStorage, AuthStorageError};
use crate::config_value::{
    ConfigValueError, config_value_env_var_names, is_command_config_value,
    is_config_value_configured, resolve_config_value_strict, resolve_headers_strict,
};
use crate::env::EnvSource;

/// API-key method of a provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ApiKeyPlan {
    /// Registration env vars (`Some` when the registration itself offers API keys).
    pub(crate) inherited_env: Option<Vec<String>>,
    /// `models.json` / override `apiKey` config value.
    pub(crate) raw_key: Option<String>,
    /// `raw_key` came from an override (pi `fallback` status source).
    pub(crate) raw_key_from_override: bool,
}

/// Auth methods of a composed provider.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StandardAuth {
    pub(crate) api_key: Option<ApiKeyPlan>,
    pub(crate) oauth: bool,
    /// Provider headers from `models.json` and overrides (unresolved).
    pub(crate) raw_headers: Option<IndexMap<String, String>>,
    /// Add `Authorization: Bearer <key>`.
    pub(crate) auth_header: bool,
}

/// How a provider authenticates.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuthPlan {
    /// A provider listing only virtual models: always configured, no credentials.
    Virtual,
    /// Regular provider.
    Standard(StandardAuth),
}

/// pi `AuthCheck`: a provider has usable credentials.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthCheck {
    /// Credential kind.
    #[serde(rename = "type")]
    pub kind: CredentialType,
    /// Where it comes from (`stored credential`, an env var name, `configured API key`,
    /// `OAuth`, `virtual`).
    pub source: String,
}

/// pi `AuthStatus.source`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthStatusSource {
    /// `auth.json`.
    Stored,
    /// `--api-key`.
    Runtime,
    /// Environment variable(s).
    Environment,
    /// Override `apiKey` literal.
    Fallback,
    /// `models.json` literal key.
    ModelsJsonKey,
    /// `models.json` `!command`.
    ModelsJsonCommand,
}

/// pi `AuthStatus` (for `/login` and status displays).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthStatus {
    /// Credentials are configured.
    pub configured: bool,
    /// Their source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<AuthStatusSource>,
    /// Env var names or check source.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// Credentials resolved for one provider (pi `AuthResult`).
#[derive(Clone, PartialEq, Eq)]
pub struct ResolvedAuth {
    /// API key / access token.
    pub api_key: Option<String>,
    /// Provider headers (resolved; `Authorization` when `authHeader`).
    pub headers: Option<IndexMap<String, String>>,
    /// Variables from the stored credential's `env`.
    pub env: Option<IndexMap<String, String>>,
    /// Source label.
    pub source: String,
}

impl std::fmt::Debug for ResolvedAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedAuth")
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field(
                "headers",
                &self.headers.as_ref().map(|h| h.keys().collect::<Vec<_>>()),
            )
            .field("source", &self.source)
            .finish_non_exhaustive()
    }
}

/// Credential resolution failures (pi `ModelsError` messages).
#[derive(Debug, thiserror::Error)]
pub enum AuthResolveError {
    /// The credential store could not be read.
    #[error("Credential store read failed for {provider}")]
    Store {
        /// Provider id.
        provider: String,
        /// Underlying error.
        #[source]
        source: AuthStorageError,
    },
    /// An API key, header or `authHeader` could not be resolved.
    #[error("API key auth failed for provider {provider}")]
    ApiKey {
        /// Provider id.
        provider: String,
        /// Underlying error.
        #[source]
        source: ApiKeyError,
    },
    /// OAuth refresh failed.
    #[error(transparent)]
    OAuth(#[from] OAuthError),
    /// A model header could not be resolved.
    #[error(transparent)]
    ModelHeaders(ConfigValueError),
}

/// Causes of [`AuthResolveError::ApiKey`].
#[derive(Debug, thiserror::Error)]
pub enum ApiKeyError {
    /// A config value did not resolve.
    #[error(transparent)]
    ConfigValue(#[from] ConfigValueError),
    /// `authHeader` without a key.
    #[error("authHeader requires a resolved API key")]
    AuthHeaderWithoutKey,
}

impl AuthResolveError {
    /// The message pi's `ModelRegistry.getApiKeyAndHeaders` reports: the cause's message, with
    /// `authHeader requires a resolved API key` shown as `No API key found for "<provider>"`.
    #[must_use]
    pub fn request_message(&self, provider: &str) -> String {
        match self {
            Self::Store { source, .. } => source.to_string(),
            Self::ApiKey {
                source: ApiKeyError::AuthHeaderWithoutKey,
                ..
            } => format!("No API key found for \"{provider}\""),
            Self::ApiKey {
                source: ApiKeyError::ConfigValue(error),
                ..
            }
            | Self::ModelHeaders(error) => error.to_string(),
            Self::OAuth(error) => error.cause_message(),
        }
    }
}

/// Per-request auth inputs.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AuthOverrides {
    /// A request-scoped API key (pi `options.apiKey`).
    pub api_key: Option<String>,
    /// Required remaining OAuth validity (default five minutes).
    pub min_oauth_validity_ms: Option<i64>,
}

fn first_env_key(names: &[String], env: &dyn EnvSource) -> Option<(String, String)> {
    names.iter().find_map(|name| {
        env.var(name)
            .filter(|value| !value.is_empty())
            .map(|value| (value, name.clone()))
    })
}

/// pi `checkProviderAuth` for a composed provider.
pub(crate) fn check(
    plan: &AuthPlan,
    credential: Option<&AuthCredential>,
    env: &dyn EnvSource,
) -> Option<AuthCheck> {
    let AuthPlan::Standard(plan) = plan else {
        return Some(AuthCheck {
            kind: CredentialType::ApiKey,
            source: "virtual".to_owned(),
        });
    };
    let api_key_check = |source: String| AuthCheck {
        kind: CredentialType::ApiKey,
        source,
    };
    match credential {
        Some(AuthCredential::Oauth(_)) => plan.oauth.then(|| AuthCheck {
            kind: CredentialType::Oauth,
            source: "OAuth".to_owned(),
        }),
        Some(AuthCredential::ApiKey(stored)) => {
            let api_key = plan.api_key.as_ref()?;
            if stored.key.as_deref().is_some_and(|k| !k.is_empty()) {
                return Some(api_key_check("stored credential".to_owned()));
            }
            let (_, name) = first_env_key(api_key.inherited_env.as_deref()?, env)?;
            Some(api_key_check(name))
        }
        None => {
            let api_key = plan.api_key.as_ref()?;
            if let Some(raw) = &api_key.raw_key {
                let configured = is_command_config_value(raw)
                    || config_value_env_var_names(raw)
                        .iter()
                        .all(|name| env.var(name).is_some_and(|v| !v.is_empty()));
                return configured.then(|| api_key_check("configured API key".to_owned()));
            }
            let (_, name) = first_env_key(api_key.inherited_env.as_deref()?, env)?;
            Some(api_key_check(name))
        }
    }
}

/// pi `getProviderAuthStatus`.
pub(crate) fn status(
    plan: &AuthPlan,
    storage: &AuthStorage,
    provider: &str,
    check: Option<&AuthCheck>,
    env: &dyn EnvSource,
) -> AuthStatus {
    let configured = |source: AuthStatusSource, label: Option<String>| AuthStatus {
        configured: true,
        source: Some(source),
        label,
    };
    if storage.has_runtime_api_key(provider) {
        return configured(AuthStatusSource::Runtime, None);
    }
    if storage.has_stored(provider) {
        return configured(AuthStatusSource::Stored, None);
    }
    if let AuthPlan::Standard(StandardAuth {
        api_key: Some(api_key),
        ..
    }) = plan
        && let Some(raw) = &api_key.raw_key
    {
        if is_command_config_value(raw) {
            return configured(AuthStatusSource::ModelsJsonCommand, None);
        }
        let names = config_value_env_var_names(raw);
        if !names.is_empty() {
            return if is_config_value_configured(raw, None, env) {
                configured(AuthStatusSource::Environment, Some(names.join(", ")))
            } else {
                AuthStatus {
                    configured: false,
                    source: None,
                    label: None,
                }
            };
        }
        return configured(
            if api_key.raw_key_from_override {
                AuthStatusSource::Fallback
            } else {
                AuthStatusSource::ModelsJsonKey
            },
            None,
        );
    }
    match check {
        Some(check) => configured(AuthStatusSource::Environment, Some(check.source.clone())),
        None => AuthStatus {
            configured: false,
            source: None,
            label: None,
        },
    }
}

/// pi `withConfiguredAuth`.
fn with_configured_auth(
    mut auth: ResolvedAuth,
    headers: Option<IndexMap<String, String>>,
    auth_header: bool,
) -> Result<ResolvedAuth, ApiKeyError> {
    let mut merged = match (auth.headers.take(), headers) {
        (None, None) => None,
        (base, over) => {
            let mut merged = base.unwrap_or_default();
            merged.extend(over.unwrap_or_default());
            Some(merged)
        }
    };
    if auth_header {
        let key = auth
            .api_key
            .as_deref()
            .filter(|k| !k.is_empty())
            .ok_or(ApiKeyError::AuthHeaderWithoutKey)?;
        merged
            .get_or_insert_with(IndexMap::new)
            .insert("Authorization".to_owned(), format!("Bearer {key}"));
    }
    auth.headers = merged;
    Ok(auth)
}

/// Inputs shared by one resolution.
pub(crate) struct Resolver<'a> {
    pub(crate) provider: &'a str,
    pub(crate) plan: &'a AuthPlan,
    pub(crate) storage: &'a AuthStorage,
    pub(crate) env: &'a dyn EnvSource,
    pub(crate) refresher: Option<&'a dyn OAuthRefresher>,
}

impl Resolver<'_> {
    /// pi `resolveProviderAuth` + composition: `None` when nothing is configured.
    pub(crate) async fn resolve(
        &self,
        overrides: &AuthOverrides,
    ) -> Result<Option<ResolvedAuth>, AuthResolveError> {
        let AuthPlan::Standard(plan) = self.plan else {
            return Ok(Some(ResolvedAuth {
                api_key: None,
                headers: None,
                env: None,
                source: "virtual".to_owned(),
            }));
        };
        if let (Some(key), Some(api_key)) = (&overrides.api_key, &plan.api_key) {
            let credential = ApiKeyCredential {
                key: Some(key.clone()),
                ..ApiKeyCredential::default()
            };
            return self.resolve_api_key(plan, api_key, Some(&credential));
        }
        let credential =
            self.storage
                .get(self.provider)
                .map_err(|source| AuthResolveError::Store {
                    provider: self.provider.to_owned(),
                    source,
                })?;
        match credential {
            Some(AuthCredential::Oauth(stored)) if plan.oauth => {
                let Some(fresh) = ensure_fresh_oauth(
                    self.storage,
                    self.provider,
                    self.refresher,
                    stored,
                    overrides.min_oauth_validity_ms,
                )
                .await?
                else {
                    return Ok(None);
                };
                let env = fresh.extra.get("env").and_then(|env| {
                    serde_json::from_value::<IndexMap<String, String>>(env.clone()).ok()
                });
                let auth = ResolvedAuth {
                    api_key: Some(fresh.access),
                    headers: None,
                    env: None,
                    source: "OAuth".to_owned(),
                };
                let headers = self.resolve_provider_headers(plan, env.as_ref())?;
                with_configured_auth(auth, headers, plan.auth_header)
                    .map(Some)
                    .map_err(|source| self.api_key_error(source))
            }
            Some(AuthCredential::ApiKey(stored)) => match &plan.api_key {
                Some(api_key) => self.resolve_api_key(plan, api_key, Some(&stored)),
                None => Ok(None),
            },
            Some(AuthCredential::Oauth(_)) => Ok(None),
            None => match &plan.api_key {
                Some(api_key) => self.resolve_api_key(plan, api_key, None),
                None => Ok(None),
            },
        }
    }

    fn api_key_error(&self, source: ApiKeyError) -> AuthResolveError {
        AuthResolveError::ApiKey {
            provider: self.provider.to_owned(),
            source,
        }
    }

    fn resolve_provider_headers(
        &self,
        plan: &StandardAuth,
        explicit: Option<&IndexMap<String, String>>,
    ) -> Result<Option<IndexMap<String, String>>, AuthResolveError> {
        resolve_headers_strict(
            plan.raw_headers.as_ref(),
            &format!("provider \"{}\"", self.provider),
            explicit,
            self.env,
        )
        .map_err(|error| self.api_key_error(error.into()))
    }

    /// pi `composeApiKeyAuth().resolve`.
    fn resolve_api_key(
        &self,
        plan: &StandardAuth,
        api_key: &ApiKeyPlan,
        credential: Option<&ApiKeyCredential>,
    ) -> Result<Option<ResolvedAuth>, AuthResolveError> {
        let stored_key = |key: String, env: Option<IndexMap<String, String>>| ResolvedAuth {
            api_key: Some(key),
            headers: None,
            env,
            source: "stored credential".to_owned(),
        };
        let from_env = || {
            api_key
                .inherited_env
                .as_deref()
                .and_then(|names| first_env_key(names, self.env))
                .map(|(key, name)| ResolvedAuth {
                    api_key: Some(key),
                    headers: None,
                    env: None,
                    source: name,
                })
        };
        let result = if let Some(credential) = credential {
            match credential.key.as_deref().filter(|k| !k.is_empty()) {
                Some(key) => Some(stored_key(key.to_owned(), credential.env.clone())),
                None if api_key.inherited_env.is_some() => from_env(),
                None => None,
            }
        } else if let Some(raw) = &api_key.raw_key {
            let description = format!("API key for provider \"{}\"", self.provider);
            let key = resolve_config_value_strict(raw, &description, None, self.env)
                .map_err(|error| self.api_key_error(error.into()))?;
            Some(if api_key.inherited_env.is_some() {
                stored_key(key, None)
            } else {
                ResolvedAuth {
                    api_key: Some(key),
                    headers: None,
                    env: None,
                    source: "configured API key".to_owned(),
                }
            })
        } else {
            from_env()
        };
        let Some(result) = result else {
            return Ok(None);
        };
        let headers = self.resolve_provider_headers(plan, result.env.as_ref())?;
        with_configured_auth(result, headers, plan.auth_header)
            .map(Some)
            .map_err(|source| self.api_key_error(source))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    fn plan(raw_key: Option<&str>, inherited: Option<&[&str]>) -> AuthPlan {
        AuthPlan::Standard(StandardAuth {
            api_key: Some(ApiKeyPlan {
                inherited_env: inherited
                    .map(|names| names.iter().map(|n| (*n).to_owned()).collect()),
                raw_key: raw_key.map(str::to_owned),
                raw_key_from_override: false,
            }),
            oauth: false,
            raw_headers: None,
            auth_header: false,
        })
    }

    #[test]
    fn check_sources() {
        let env = MapEnv::new().with("K2", "v");
        assert_eq!(
            check(&plan(None, Some(&["K1", "K2"])), None, &env).map(|c| c.source),
            Some("K2".to_owned())
        );
        assert_eq!(
            check(&plan(Some("$MISSING"), Some(&["K2"])), None, &env),
            None
        );
        assert_eq!(
            check(&plan(Some("!cmd"), None), None, &env).map(|c| c.source),
            Some("configured API key".to_owned())
        );
        let stored = AuthCredential::api_key("k");
        assert_eq!(
            check(&plan(None, None), Some(&stored), &env).map(|c| c.source),
            Some("stored credential".to_owned())
        );
    }
}
