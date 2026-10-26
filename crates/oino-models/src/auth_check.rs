//! pi `oino auth check` (`cli/auth-check.ts` + `main.ts`): provider readiness with exit codes
//! 0 (ready), 1 (not ready), 2 (invalid).
//!
//! For `--no-refresh` the caller builds the registry over
//! [`AuthStorage::open_read_only`](crate::AuthStorage::open_read_only), like pi.

use serde::Serialize;

use crate::auth::credential::{AuthCredential, CredentialType};
use crate::registry::{AuthOverrides, ModelRegistry, ResolvedAuth};
use crate::resolver::{CliModelRequest, resolve_cli_model_in};

/// Readiness (pi `AuthCheckStatus`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCheckStatus {
    /// Usable credentials.
    Ready,
    /// Not configured.
    NotReady,
    /// Broken configuration or credential store.
    Invalid,
}

impl AuthCheckStatus {
    /// Process exit code (0/1/2).
    #[must_use]
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Ready => 0,
            Self::NotReady => 1,
            Self::Invalid => 2,
        }
    }

    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::NotReady => "not_ready",
            Self::Invalid => "invalid",
        }
    }
}

/// Why a provider is not ready (pi `AuthCheckReason`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCheckReason {
    /// Unknown provider.
    ProviderNotFound,
    /// No credentials.
    CredentialsNotConfigured,
    /// `--credentials` requested but none could be produced.
    CredentialNotAvailable,
    /// Configuration or store error.
    InvalidState,
}

/// pi `AuthCheckResult`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthCheckResult {
    /// Readiness.
    pub status: AuthCheckStatus,
    /// Provider (or the requested text when resolution failed).
    pub provider: String,
    /// Reason when not ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<AuthCheckReason>,
    /// Credential kind when ready.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth_type: Option<CredentialType>,
}

impl AuthCheckResult {
    fn new(status: AuthCheckStatus, provider: &str, reason: Option<AuthCheckReason>) -> Self {
        Self {
            status,
            provider: provider.to_owned(),
            reason,
            auth_type: None,
        }
    }

    /// pi output line: JSON (`{…, "credentials"?}`) or the credential / status word.
    #[must_use]
    pub fn render(&self, json: bool, credential: Option<&str>) -> String {
        if json {
            let mut value = serde_json::to_value(self).unwrap_or_default();
            if let (Some(credential), Some(object)) = (credential, value.as_object_mut()) {
                object.insert("credentials".to_owned(), credential.into());
            }
            value.to_string()
        } else {
            credential.map_or_else(|| self.status.as_str().to_owned(), str::to_owned)
        }
    }
}

/// What to check: `--provider` and/or `--model`.
#[derive(Clone, Copy, Debug, Default)]
pub struct AuthCheckTarget<'a> {
    /// `--provider`.
    pub provider: Option<&'a str>,
    /// `--model` (resolved like the CLI).
    pub model: Option<&'a str>,
}

/// pi `checkProviderAuth` with main's error mapping (every failure is `invalid`).
pub async fn check_provider_auth(
    target: AuthCheckTarget<'_>,
    registry: &ModelRegistry,
    refresh: bool,
) -> AuthCheckResult {
    let requested = target.provider.or(target.model).unwrap_or_default();
    let invalid = |provider: &str| {
        AuthCheckResult::new(
            AuthCheckStatus::Invalid,
            provider,
            Some(AuthCheckReason::InvalidState),
        )
    };
    let provider = match target.model {
        Some(model) => match resolve_cli_model_in(
            &CliModelRequest {
                provider: target.provider,
                model: Some(model),
                ..CliModelRequest::default()
            },
            registry,
        ) {
            Ok(Some(resolved)) => resolved.model.provider,
            _ => return invalid(requested),
        },
        None => match target.provider {
            Some(provider) => provider.to_owned(),
            None => return invalid(requested),
        },
    };
    if registry.error().is_some() {
        return invalid(&provider);
    }
    if !registry.provider_ids().contains(&provider) {
        return AuthCheckResult::new(
            AuthCheckStatus::NotReady,
            &provider,
            Some(AuthCheckReason::ProviderNotFound),
        );
    }
    let Some(check) = registry.check_auth(&provider) else {
        return AuthCheckResult::new(
            AuthCheckStatus::NotReady,
            &provider,
            Some(AuthCheckReason::CredentialsNotConfigured),
        );
    };
    if refresh {
        match registry
            .get_auth(&provider, &AuthOverrides::default())
            .await
        {
            Ok(Some(_)) => {}
            Ok(None) => {
                return AuthCheckResult::new(
                    AuthCheckStatus::NotReady,
                    &provider,
                    Some(AuthCheckReason::CredentialsNotConfigured),
                );
            }
            Err(_) => return invalid(&provider),
        }
    }
    AuthCheckResult {
        status: AuthCheckStatus::Ready,
        provider,
        reason: None,
        auth_type: Some(check.kind),
    }
}

/// pi `getAuthCredential`: the API key, else the token of an `Authorization: Bearer` header.
#[must_use]
pub fn auth_credential(auth: Option<&ResolvedAuth>) -> Option<String> {
    let auth = auth?;
    if let Some(key) = auth.api_key.as_deref().filter(|k| !k.is_empty()) {
        return Some(key.to_owned());
    }
    let (_, value) = auth
        .headers
        .as_ref()?
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("authorization"))?;
    let (scheme, token) = value.split_once(char::is_whitespace)?;
    let token = token.trim_start();
    (scheme.eq_ignore_ascii_case("bearer") && !token.is_empty()).then(|| token.to_owned())
}

/// pi `getProviderCredential` (`--credentials`): without refresh a stored OAuth access token is
/// printed as is; otherwise the resolved credential.
pub async fn provider_credential(
    registry: &ModelRegistry,
    provider: &str,
    refresh: bool,
) -> Option<String> {
    if !refresh
        && let Ok(Some(AuthCredential::Oauth(oauth))) = registry.auth_storage().get(provider)
    {
        return Some(oauth.access);
    }
    let auth = registry
        .get_auth(provider, &AuthOverrides::default())
        .await
        .ok()
        .flatten();
    auth_credential(auth.as_ref())
}
