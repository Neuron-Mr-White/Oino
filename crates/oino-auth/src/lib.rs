#![doc = r#"Credential storage and resolution for Oino providers.

This crate stores provider credentials and resolves API keys for provider adapters. It
intentionally does not know provider HTTP protocols or depend on the harness, TUI, or app
crates.

## Boundary

`oino-auth` owns the local credential file shape, provider credential lookup order,
typed auth errors, default auth path, best-effort file permissions for stored
secrets, and provider-neutral auth status records. Provider adapters supply a
[`ProviderAuthSpec`] and receive a credential; they own HTTP headers, request signing,
provider-specific error handling, refresh protocol details, and model semantics.
`oino-app` owns user-facing setup flows and runtime overrides.

## Public API map

- [`AuthConfig`] selects the auth file, runtime overrides, explicit environment
  overrides, and whether process environment variables are consulted.
- [`AuthStorage`] loads/saves legacy [`AuthFile`] entries and versioned
  [`AuthDocument`] data. API-key resolution remains compatible with the original
  lookup order: runtime override by provider id, auth file by auth key, explicit
  environment override by environment variable name, then process environment variable.
- [`AuthCredential`] supports API keys plus provider-neutral OAuth, device-code,
  external, and local-endpoint credential records. Secret values are redacted from
  `Debug` output.
- [`ProviderAuthSpec`] maps one provider id to its auth-file key and environment
  variable. [`ProviderAuthSpec::openrouter`] is the built-in OpenRouter mapping.
- [`ProviderAuthAssessment`] and related enums describe provider readiness and source
  attribution for future account/auth UX without embedding provider protocols here.
- [`ExternalAuthTrustRecord`] stores explicit user trust decisions for future safe
  external credential imports without reading provider-specific files in this crate.
- [`AuthError`] keeps missing, malformed, and I/O failures typed so the app can show
  actionable setup messages without provider code parsing strings.

## Contributor rules

Do not add provider HTTP protocol details, TUI prompts, model settings, or harness
state here. Keep secret formatting redacted, avoid logging raw credential values, and
update read/write/permission tests when changing the file format. The auth file is a
plain JSON convenience store protected by filesystem permissions where possible; do not
document it as encryption or an OS keychain unless the storage contract changes.
"#]
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::PathBuf};
use thiserror::Error;
use tokio::fs;

pub const OPENROUTER_PROVIDER_ID: &str = "openrouter";
pub const OPENROUTER_ENV_VAR: &str = "OPENROUTER_API_KEY";
pub const OPENROUTER_AUTH_KEY: &str = "openrouter";
pub const AUTH_DOCUMENT_VERSION: u32 = 2;

#[derive(Debug, Error)]
pub enum AuthError {
    #[error("missing credential for provider `{provider}`; set {env_var} or add {auth_key} to auth file")]
    MissingCredential {
        provider: String,
        auth_key: String,
        env_var: String,
    },
    #[error("credential for provider `{provider}` is not an API key")]
    NotApiKey { provider: String },
    #[error("could not determine home directory for default auth path")]
    HomeDirectoryUnavailable,
    #[error("auth file I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("auth file JSON error at {path}: {source}")]
    Json {
        path: PathBuf,
        #[source]
        source: serde_json::Error,
    },
}

pub type AuthResult<T> = Result<T, AuthError>;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthState {
    Available,
    Expired,
    #[default]
    NotConfigured,
}

impl AuthState {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Available => "available",
            Self::Expired => "expired",
            Self::NotConfigured => "not configured",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthCredentialSource {
    #[default]
    None,
    RuntimeOverride,
    EnvironmentVariable,
    OinoAuthFile,
    OinoManagedFile,
    TrustedExternalFile,
    TrustedExternalAppState,
    LocalCliSession,
    AzureDefaultCredential,
    Mixed,
}

impl AuthCredentialSource {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::RuntimeOverride => "runtime override",
            Self::EnvironmentVariable => "environment variable",
            Self::OinoAuthFile => "Oino auth file",
            Self::OinoManagedFile => "Oino-managed file",
            Self::TrustedExternalFile => "trusted external file",
            Self::TrustedExternalAppState => "trusted external app state",
            Self::LocalCliSession => "local CLI session",
            Self::AzureDefaultCredential => "Azure DefaultAzureCredential",
            Self::Mixed => "mixed",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthExpiryConfidence {
    #[default]
    Unknown,
    Exact,
    PresenceOnly,
    ConfigurationOnly,
    NotApplicable,
}

impl AuthExpiryConfidence {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Exact => "exact timestamp",
            Self::PresenceOnly => "presence only",
            Self::ConfigurationOnly => "configuration only",
            Self::NotApplicable => "not applicable",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthRefreshSupport {
    #[default]
    Unknown,
    Automatic,
    Conditional,
    ManualRelogin,
    ExternalManaged,
    NotApplicable,
}

impl AuthRefreshSupport {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Automatic => "automatic",
            Self::Conditional => "conditional",
            Self::ManualRelogin => "manual re-login",
            Self::ExternalManaged => "external/manual",
            Self::NotApplicable => "not applicable",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthValidationMethod {
    #[default]
    Unknown,
    PresenceCheck,
    TimestampCheck,
    ConfigurationCheck,
    TrustedImportScan,
    CommandProbe,
    CompositeProbe,
}

impl AuthValidationMethod {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::PresenceCheck => "presence check",
            Self::TimestampCheck => "timestamp check",
            Self::ConfigurationCheck => "configuration check",
            Self::TrustedImportScan => "trusted import scan",
            Self::CommandProbe => "command probe",
            Self::CompositeProbe => "composite probe",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthReadinessLevel {
    #[default]
    None,
    CredentialPresent,
    Authenticated,
    RequestValid,
    DeploymentValid,
}

impl AuthReadinessLevel {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::None => "not configured",
            Self::CredentialPresent => "credential present",
            Self::Authenticated => "authenticated",
            Self::RequestValid => "request valid",
            Self::DeploymentValid => "deployment valid",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderValidationRecord {
    pub checked_at_ms: i64,
    pub success: bool,
    pub provider_smoke_ok: Option<bool>,
    pub tool_smoke_ok: Option<bool>,
    pub summary: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRefreshRecord {
    pub last_attempt_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_success_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAuthAssessment {
    pub state: AuthState,
    pub readiness: AuthReadinessLevel,
    pub method_detail: String,
    pub credential_source: AuthCredentialSource,
    pub credential_source_detail: String,
    pub expiry_confidence: AuthExpiryConfidence,
    pub refresh_support: AuthRefreshSupport,
    pub validation_method: AuthValidationMethod,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_validation: Option<ProviderValidationRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_refresh: Option<ProviderRefreshRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_account_label: Option<String>,
}

impl ProviderAuthAssessment {
    #[must_use]
    pub fn not_configured(provider_id: &str) -> Self {
        Self {
            state: AuthState::NotConfigured,
            readiness: AuthReadinessLevel::None,
            method_detail: "not configured".into(),
            credential_source: AuthCredentialSource::None,
            credential_source_detail: format!("no credential found for {provider_id}"),
            expiry_confidence: AuthExpiryConfidence::Unknown,
            refresh_support: AuthRefreshSupport::Unknown,
            validation_method: AuthValidationMethod::PresenceCheck,
            last_validation: None,
            last_refresh: None,
            active_account_label: None,
        }
    }

    #[must_use]
    pub fn from_resolved(_provider_id: &str, resolved: &ResolvedCredential) -> Self {
        let credential_is_refreshable = resolved.credential.has_refresh_token();
        Self {
            state: AuthState::Available,
            readiness: AuthReadinessLevel::CredentialPresent,
            method_detail: resolved.credential.kind_label().into(),
            credential_source: resolved.source,
            credential_source_detail: resolved.source_detail.clone(),
            expiry_confidence: resolved.credential.expiry_confidence(),
            refresh_support: if credential_is_refreshable {
                AuthRefreshSupport::Conditional
            } else {
                AuthRefreshSupport::NotApplicable
            },
            validation_method: AuthValidationMethod::PresenceCheck,
            last_validation: None,
            last_refresh: None,
            active_account_label: resolved.credential.account_id().map(ToString::to_string),
        }
    }

    #[must_use]
    pub fn is_available(&self) -> bool {
        self.state == AuthState::Available
    }

    #[must_use]
    pub fn is_configured(&self) -> bool {
        self.state != AuthState::NotConfigured
    }

    #[must_use]
    pub fn health_summary(&self) -> String {
        let mut parts = vec![
            format!("readiness: {}", self.readiness.label()),
            format!("source: {}", self.credential_source_detail),
            format!("expiry: {}", self.expiry_confidence.label()),
            format!("refresh: {}", self.refresh_support.label()),
            format!("probe: {}", self.validation_method.label()),
        ];
        if let Some(record) = &self.last_refresh {
            let label = record.last_error.as_deref().map_or_else(
                || {
                    record
                        .last_success_ms
                        .map_or_else(|| "attempted".to_string(), |ms| format!("ok at {ms}"))
                },
                |error| format!("error: {error}"),
            );
            parts.push(format!("last refresh: {label}"));
        }
        parts.join(" · ")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthCredential {
    ApiKey {
        key: String,
    },
    OAuthToken {
        access_token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_unix: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        scopes: Vec<String>,
    },
    DeviceCodeToken {
        token: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        refresh_token: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        expires_at_unix: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
    },
    External {
        source: AuthCredentialSource,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        command: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_id: Option<String>,
    },
    LocalEndpoint {
        base_url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        api_key: Option<String>,
    },
}

impl AuthCredential {
    #[must_use]
    pub fn api_key(key: impl Into<String>) -> Self {
        Self::ApiKey { key: key.into() }
    }

    #[must_use]
    pub fn oauth_token(
        access_token: impl Into<String>,
        refresh_token: Option<String>,
        expires_at_unix: Option<i64>,
        account_id: Option<String>,
        scopes: Vec<String>,
    ) -> Self {
        Self::OAuthToken {
            access_token: access_token.into(),
            refresh_token,
            expires_at_unix,
            account_id,
            scopes,
        }
    }

    #[must_use]
    pub fn device_code_token(
        token: impl Into<String>,
        refresh_token: Option<String>,
        expires_at_unix: Option<i64>,
        account_id: Option<String>,
    ) -> Self {
        Self::DeviceCodeToken {
            token: token.into(),
            refresh_token,
            expires_at_unix,
            account_id,
        }
    }

    #[must_use]
    pub fn external(
        source: AuthCredentialSource,
        path: Option<String>,
        command: Option<String>,
        account_id: Option<String>,
    ) -> Self {
        Self::External {
            source,
            path,
            command,
            account_id,
        }
    }

    #[must_use]
    pub fn local_endpoint(base_url: impl Into<String>, api_key: Option<String>) -> Self {
        Self::LocalEndpoint {
            base_url: base_url.into(),
            api_key,
        }
    }

    #[must_use]
    pub fn as_api_key(&self) -> Option<&str> {
        match self {
            Self::ApiKey { key } => Some(key.as_str()),
            Self::OAuthToken { .. }
            | Self::DeviceCodeToken { .. }
            | Self::External { .. }
            | Self::LocalEndpoint { .. } => None,
        }
    }

    #[must_use]
    pub const fn kind_label(&self) -> &'static str {
        match self {
            Self::ApiKey { .. } => "API key",
            Self::OAuthToken { .. } => "OAuth token",
            Self::DeviceCodeToken { .. } => "device-code token",
            Self::External { .. } => "external credential",
            Self::LocalEndpoint { .. } => "local endpoint",
        }
    }

    #[must_use]
    pub fn account_id(&self) -> Option<&str> {
        match self {
            Self::OAuthToken { account_id, .. }
            | Self::DeviceCodeToken { account_id, .. }
            | Self::External { account_id, .. } => account_id.as_deref(),
            Self::ApiKey { .. } | Self::LocalEndpoint { .. } => None,
        }
    }

    #[must_use]
    pub fn has_refresh_token(&self) -> bool {
        match self {
            Self::OAuthToken { refresh_token, .. }
            | Self::DeviceCodeToken { refresh_token, .. } => refresh_token
                .as_deref()
                .is_some_and(|token| !token.trim().is_empty()),
            Self::ApiKey { .. } | Self::External { .. } | Self::LocalEndpoint { .. } => false,
        }
    }

    #[must_use]
    pub const fn expiry_confidence(&self) -> AuthExpiryConfidence {
        match self {
            Self::OAuthToken {
                expires_at_unix, ..
            }
            | Self::DeviceCodeToken {
                expires_at_unix, ..
            } => {
                if expires_at_unix.is_some() {
                    AuthExpiryConfidence::Exact
                } else {
                    AuthExpiryConfidence::PresenceOnly
                }
            }
            Self::External { .. } => AuthExpiryConfidence::ConfigurationOnly,
            Self::ApiKey { .. } | Self::LocalEndpoint { .. } => AuthExpiryConfidence::NotApplicable,
        }
    }
}

impl fmt::Debug for AuthCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ApiKey { .. } => f
                .debug_struct("ApiKey")
                .field("key", &"<redacted>")
                .finish(),
            Self::OAuthToken {
                expires_at_unix,
                account_id,
                scopes,
                ..
            } => f
                .debug_struct("OAuthToken")
                .field("access_token", &"<redacted>")
                .field("refresh_token", &"<redacted>")
                .field("expires_at_unix", expires_at_unix)
                .field("account_id", account_id)
                .field("scopes", scopes)
                .finish(),
            Self::DeviceCodeToken {
                expires_at_unix,
                account_id,
                ..
            } => f
                .debug_struct("DeviceCodeToken")
                .field("token", &"<redacted>")
                .field("refresh_token", &"<redacted>")
                .field("expires_at_unix", expires_at_unix)
                .field("account_id", account_id)
                .finish(),
            Self::External {
                source,
                path,
                account_id,
                ..
            } => f
                .debug_struct("External")
                .field("source", source)
                .field("path", path)
                .field("command", &"<redacted>")
                .field("account_id", account_id)
                .finish(),
            Self::LocalEndpoint { base_url, api_key } => f
                .debug_struct("LocalEndpoint")
                .field("base_url", base_url)
                .field("api_key", &api_key.as_ref().map(|_| "<redacted>"))
                .finish(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderAuthSpec {
    pub provider_id: String,
    pub auth_key: String,
    pub env_var: String,
}

impl ProviderAuthSpec {
    #[must_use]
    pub fn new(
        provider_id: impl Into<String>,
        auth_key: impl Into<String>,
        env_var: impl Into<String>,
    ) -> Self {
        Self {
            provider_id: provider_id.into(),
            auth_key: auth_key.into(),
            env_var: env_var.into(),
        }
    }

    #[must_use]
    pub fn openrouter() -> Self {
        Self::new(
            OPENROUTER_PROVIDER_ID,
            OPENROUTER_AUTH_KEY,
            OPENROUTER_ENV_VAR,
        )
    }
}

pub type AuthFile = BTreeMap<String, AuthCredential>;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccount {
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<AuthCredential>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderAccountSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_label: Option<String>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, ProviderAccount>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAuthTrustDecision {
    #[default]
    Trusted,
    Denied,
}

impl ExternalAuthTrustDecision {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Trusted => "trusted",
            Self::Denied => "denied",
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExternalAuthTrustScope {
    #[default]
    Source,
    FilePath,
    Account,
    Environment,
}

impl ExternalAuthTrustScope {
    #[must_use]
    pub const fn label(self) -> &'static str {
        match self {
            Self::Source => "source",
            Self::FilePath => "file path",
            Self::Account => "account",
            Self::Environment => "environment",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalAuthTrustRecord {
    pub source_id: String,
    pub decision: ExternalAuthTrustDecision,
    pub scope: ExternalAuthTrustScope,
    pub granted_at_ms: i64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub provider_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl ExternalAuthTrustRecord {
    #[must_use]
    pub fn trusted_source(
        source_id: impl Into<String>,
        provider_ids: Vec<String>,
        granted_at_ms: i64,
        note: Option<String>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            decision: ExternalAuthTrustDecision::Trusted,
            scope: ExternalAuthTrustScope::Source,
            granted_at_ms,
            provider_ids,
            path: None,
            account_label: None,
            expires_at_ms: None,
            note,
        }
    }

    #[must_use]
    pub fn trusted_file(
        source_id: impl Into<String>,
        provider_ids: Vec<String>,
        path: impl Into<String>,
        granted_at_ms: i64,
        note: Option<String>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            decision: ExternalAuthTrustDecision::Trusted,
            scope: ExternalAuthTrustScope::FilePath,
            granted_at_ms,
            provider_ids,
            path: Some(path.into()),
            account_label: None,
            expires_at_ms: None,
            note,
        }
    }

    #[must_use]
    pub fn denied_source(
        source_id: impl Into<String>,
        provider_ids: Vec<String>,
        granted_at_ms: i64,
        note: Option<String>,
    ) -> Self {
        Self {
            source_id: source_id.into(),
            decision: ExternalAuthTrustDecision::Denied,
            scope: ExternalAuthTrustScope::Source,
            granted_at_ms,
            provider_ids,
            path: None,
            account_label: None,
            expires_at_ms: None,
            note,
        }
    }

    #[must_use]
    pub fn key(&self) -> String {
        external_auth_trust_key(
            &self.source_id,
            self.scope,
            self.path.as_deref(),
            self.account_label.as_deref(),
        )
    }

    #[must_use]
    pub fn is_trusted_at(&self, now_ms: i64) -> bool {
        self.decision == ExternalAuthTrustDecision::Trusted
            && self
                .expires_at_ms
                .map_or(true, |expires_at| expires_at > now_ms)
    }

    #[must_use]
    pub fn summary(&self) -> String {
        let target = self
            .path
            .as_deref()
            .or(self.account_label.as_deref())
            .unwrap_or("all targets");
        format!(
            "{} {} for {} ({})",
            self.decision.label(),
            self.scope.label(),
            self.source_id,
            target
        )
    }
}

#[must_use]
pub fn external_auth_trust_key(
    source_id: &str,
    scope: ExternalAuthTrustScope,
    path: Option<&str>,
    account_label: Option<&str>,
) -> String {
    let target = path.or(account_label).unwrap_or("*");
    format!("{source_id}::{}::{target}", scope.label().replace(' ', "_"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthDocument {
    pub version: u32,
    #[serde(
        default,
        alias = "providers",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub credentials: AuthFile,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub accounts: BTreeMap<String, ProviderAccountSet>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub assessments: BTreeMap<String, ProviderAuthAssessment>,
    #[serde(
        default,
        alias = "external_auth_trust",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub external_trusts: BTreeMap<String, ExternalAuthTrustRecord>,
}

impl Default for AuthDocument {
    fn default() -> Self {
        Self {
            version: AUTH_DOCUMENT_VERSION,
            credentials: BTreeMap::new(),
            accounts: BTreeMap::new(),
            assessments: BTreeMap::new(),
            external_trusts: BTreeMap::new(),
        }
    }
}

impl AuthDocument {
    #[must_use]
    pub fn from_credentials(credentials: AuthFile) -> Self {
        Self {
            credentials,
            ..Self::default()
        }
    }

    #[must_use]
    pub fn active_external_trust_count_at(&self, now_ms: i64) -> usize {
        self.external_trusts
            .values()
            .filter(|record| record.is_trusted_at(now_ms))
            .count()
    }
}

#[derive(Debug, Clone)]
pub struct ResolvedCredential {
    pub credential: AuthCredential,
    pub source: AuthCredentialSource,
    pub source_detail: String,
}

#[derive(Debug, Clone)]
pub struct AuthConfig {
    pub auth_path: PathBuf,
    pub runtime_overrides: BTreeMap<String, AuthCredential>,
    pub env_overrides: BTreeMap<String, String>,
    pub process_env: bool,
}

impl AuthConfig {
    #[must_use]
    pub fn new(auth_path: impl Into<PathBuf>) -> Self {
        Self {
            auth_path: auth_path.into(),
            runtime_overrides: BTreeMap::new(),
            env_overrides: BTreeMap::new(),
            process_env: true,
        }
    }

    pub fn default_path() -> AuthResult<PathBuf> {
        let Some(home) = dirs::home_dir() else {
            return Err(AuthError::HomeDirectoryUnavailable);
        };
        Ok(home.join(".oino").join("auth.json"))
    }

    pub fn default_file() -> AuthResult<Self> {
        Ok(Self::new(Self::default_path()?))
    }

    #[must_use]
    pub fn with_runtime_override(
        mut self,
        provider: impl Into<String>,
        key: impl Into<String>,
    ) -> Self {
        self.runtime_overrides
            .insert(provider.into(), AuthCredential::api_key(key));
        self
    }

    #[must_use]
    pub fn with_env_override(mut self, env_var: impl Into<String>, key: impl Into<String>) -> Self {
        self.env_overrides.insert(env_var.into(), key.into());
        self
    }

    #[must_use]
    pub fn with_process_env(mut self, enabled: bool) -> Self {
        self.process_env = enabled;
        self
    }
}

#[derive(Debug, Clone)]
pub struct AuthStorage {
    config: AuthConfig,
}

impl AuthStorage {
    #[must_use]
    pub fn new(config: AuthConfig) -> Self {
        Self { config }
    }

    pub fn default_file() -> AuthResult<Self> {
        Ok(Self::new(AuthConfig::default_file()?))
    }

    #[must_use]
    pub fn config(&self) -> &AuthConfig {
        &self.config
    }

    pub async fn load(&self) -> AuthResult<AuthFile> {
        self.load_document()
            .await
            .map(|document| document.credentials)
    }

    pub async fn load_document(&self) -> AuthResult<AuthDocument> {
        self.load_document_with_format()
            .await
            .map(|(document, _)| document)
    }

    pub async fn save_all(&self, entries: &AuthFile) -> AuthResult<()> {
        self.write_json_secret(entries).await
    }

    pub async fn save_document(&self, document: &AuthDocument) -> AuthResult<()> {
        self.write_json_secret(document).await
    }

    pub async fn set_credential(
        &self,
        auth_key: impl Into<String>,
        credential: AuthCredential,
    ) -> AuthResult<()> {
        let (mut document, format) = self.load_document_with_format().await?;
        document.credentials.insert(auth_key.into(), credential);
        self.save_preserving_format(&document, format).await
    }

    pub async fn delete_credential(&self, auth_key: &str) -> AuthResult<bool> {
        let (mut document, format) = self.load_document_with_format().await?;
        let existed = document.credentials.remove(auth_key).is_some();
        self.save_preserving_format(&document, format).await?;
        Ok(existed)
    }

    /// Set a named credential for a provider. If `is_active` is true or no
    /// credential exists yet for this provider, it becomes the active one.
    pub async fn set_named_credential(
        &self,
        provider_id: &str,
        label: &str,
        credential: AuthCredential,
        is_active: bool,
    ) -> AuthResult<()> {
        let (mut document, _) = self.load_document_with_format().await?;
        let account_set = document
            .accounts
            .entry(provider_id.to_string())
            .or_insert_with(ProviderAccountSet::default);
        let account = ProviderAccount {
            label: label.to_string(),
            email: None,
            credential_key: Some(format!("{provider_id}:{label}")),
            credential: Some(credential.clone()),
            metadata: BTreeMap::new(),
        };
        account_set.accounts.insert(label.to_string(), account);
        if is_active || account_set.active_label.is_none() {
            account_set.active_label = Some(label.to_string());
        }
        // Also keep the legacy single-credential entry in sync
        document
            .credentials
            .insert(provider_id.to_string(), credential);
        self.save_document(&document).await?;
        Ok(())
    }

    /// Delete a named credential for a provider.
    pub async fn delete_named_credential(
        &self,
        provider_id: &str,
        label: &str,
    ) -> AuthResult<bool> {
        let (mut document, _) = self.load_document_with_format().await?;
        let existed = document
            .accounts
            .get_mut(provider_id)
            .and_then(|set| set.accounts.remove(label))
            .is_some();
        if existed {
            // If we removed the active credential, pick another one
            if document
                .accounts
                .get(provider_id)
                .and_then(|set| set.active_label.as_deref())
                == Some(label)
            {
                let new_active = document
                    .accounts
                    .get(provider_id)
                    .and_then(|set| set.accounts.keys().next().cloned());
                if let Some(set) = document.accounts.get_mut(provider_id) {
                    set.active_label = new_active.clone();
                }
                // Update legacy credential entry
                if let Some(new_label) = new_active {
                    if let Some(account) = document
                        .accounts
                        .get(provider_id)
                        .and_then(|set| set.accounts.get(&new_label))
                    {
                        if let Some(cred) = &account.credential {
                            document
                                .credentials
                                .insert(provider_id.to_string(), cred.clone());
                        }
                    }
                } else {
                    document.credentials.remove(provider_id);
                }
            }
            self.save_document(&document).await?;
        }
        Ok(existed)
    }

    /// Set the active credential label for a provider.
    pub async fn set_active_credential(&self, provider_id: &str, label: &str) -> AuthResult<bool> {
        let (mut document, _) = self.load_document_with_format().await?;
        let Some(account_set) = document.accounts.get_mut(provider_id) else {
            return Ok(false);
        };
        if !account_set.accounts.contains_key(label) {
            return Ok(false);
        }
        account_set.active_label = Some(label.to_string());
        // Update legacy credential entry
        if let Some(account) = account_set.accounts.get(label) {
            if let Some(cred) = &account.credential {
                document
                    .credentials
                    .insert(provider_id.to_string(), cred.clone());
            }
        }
        self.save_document(&document).await?;
        Ok(true)
    }

    /// List all named credentials for a provider.
    pub async fn list_credentials(&self, provider_id: &str) -> AuthResult<Vec<(String, bool)>> {
        let document = self.load_document().await?;
        let active_label = document
            .accounts
            .get(provider_id)
            .and_then(|set| set.active_label.as_deref());
        Ok(document
            .accounts
            .get(provider_id)
            .map(|set| {
                set.accounts
                    .keys()
                    .map(|label| (label.clone(), Some(label.as_str()) == active_label))
                    .collect()
            })
            .unwrap_or_default())
    }

    pub async fn set_external_trust(&self, record: ExternalAuthTrustRecord) -> AuthResult<String> {
        let (mut document, _) = self.load_document_with_format().await?;
        let key = record.key();
        document.external_trusts.insert(key.clone(), record);
        // Trust decisions require the versioned document shape; preserving legacy
        // map format would silently drop the consent record.
        self.save_document(&document).await?;
        Ok(key)
    }

    pub async fn revoke_external_trust(&self, key: &str) -> AuthResult<bool> {
        let (mut document, _) = self.load_document_with_format().await?;
        let existed = document.external_trusts.remove(key).is_some();
        self.save_document(&document).await?;
        Ok(existed)
    }

    pub async fn external_trusts(&self) -> AuthResult<BTreeMap<String, ExternalAuthTrustRecord>> {
        self.load_document()
            .await
            .map(|document| document.external_trusts)
    }

    pub async fn active_external_trust_count_at(&self, now_ms: i64) -> AuthResult<usize> {
        self.load_document()
            .await
            .map(|document| document.active_external_trust_count_at(now_ms))
    }

    pub async fn resolve(&self, spec: &ProviderAuthSpec) -> AuthResult<Option<AuthCredential>> {
        self.resolve_with_source(spec)
            .await
            .map(|resolved| resolved.map(|resolved| resolved.credential))
    }

    pub async fn resolve_with_source(
        &self,
        spec: &ProviderAuthSpec,
    ) -> AuthResult<Option<ResolvedCredential>> {
        if let Some(credential) = self.config.runtime_overrides.get(&spec.provider_id) {
            return Ok(Some(ResolvedCredential {
                credential: credential.clone(),
                source: AuthCredentialSource::RuntimeOverride,
                source_detail: "runtime override".into(),
            }));
        }
        let entries = self.load().await?;
        if let Some(credential) = entries.get(&spec.auth_key) {
            return Ok(Some(ResolvedCredential {
                credential: credential.clone(),
                source: AuthCredentialSource::OinoAuthFile,
                source_detail: self.config.auth_path.display().to_string(),
            }));
        }
        if let Some(value) = self.config.env_overrides.get(&spec.env_var) {
            return Ok(Some(ResolvedCredential {
                credential: AuthCredential::api_key(value.clone()),
                source: AuthCredentialSource::EnvironmentVariable,
                source_detail: spec.env_var.clone(),
            }));
        }
        if !self.config.process_env {
            return Ok(None);
        }
        match std::env::var(&spec.env_var) {
            Ok(value) if !value.trim().is_empty() => Ok(Some(ResolvedCredential {
                credential: AuthCredential::api_key(value),
                source: AuthCredentialSource::EnvironmentVariable,
                source_detail: spec.env_var.clone(),
            })),
            Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Ok(None),
        }
    }

    pub async fn resolve_api_key(&self, spec: &ProviderAuthSpec) -> AuthResult<String> {
        let Some(resolved) = self.resolve_with_source(spec).await? else {
            return Err(AuthError::MissingCredential {
                provider: spec.provider_id.clone(),
                auth_key: spec.auth_key.clone(),
                env_var: spec.env_var.clone(),
            });
        };
        match resolved.credential.as_api_key() {
            Some(key) => Ok(key.to_string()),
            None => Err(AuthError::NotApiKey {
                provider: spec.provider_id.clone(),
            }),
        }
    }

    pub async fn assess_api_key_provider(
        &self,
        spec: &ProviderAuthSpec,
    ) -> AuthResult<ProviderAuthAssessment> {
        let Some(resolved) = self.resolve_with_source(spec).await? else {
            return Ok(ProviderAuthAssessment::not_configured(&spec.provider_id));
        };
        let mut assessment = ProviderAuthAssessment::from_resolved(&spec.provider_id, &resolved);
        if resolved.credential.as_api_key().is_some() {
            assessment.method_detail = "API key".into();
        }
        Ok(assessment)
    }

    pub async fn resolve_openrouter_api_key(&self) -> AuthResult<String> {
        self.resolve_api_key(&ProviderAuthSpec::openrouter()).await
    }

    async fn load_document_with_format(&self) -> AuthResult<(AuthDocument, AuthDocumentFormat)> {
        match fs::read_to_string(&self.config.auth_path).await {
            Ok(text) => parse_auth_document_text(&text, &self.config.auth_path),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
                Ok((AuthDocument::default(), AuthDocumentFormat::Legacy))
            }
            Err(source) => Err(AuthError::Io {
                path: self.config.auth_path.clone(),
                source,
            }),
        }
    }

    async fn save_preserving_format(
        &self,
        document: &AuthDocument,
        format: AuthDocumentFormat,
    ) -> AuthResult<()> {
        match format {
            AuthDocumentFormat::Legacy => self.save_all(&document.credentials).await,
            AuthDocumentFormat::Versioned => self.save_document(document).await,
        }
    }

    async fn write_json_secret<T: Serialize + ?Sized>(&self, value: &T) -> AuthResult<()> {
        if let Some(parent) = self.config.auth_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| AuthError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
        }
        let text = serde_json::to_string_pretty(value).map_err(|source| AuthError::Json {
            path: self.config.auth_path.clone(),
            source,
        })?;
        write_secret_file(&self.config.auth_path, text.as_bytes()).await
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AuthDocumentFormat {
    Legacy,
    Versioned,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum AuthDocumentOnDisk {
    Versioned(VersionedAuthDocument),
    Legacy(AuthFile),
}

#[derive(Debug, Deserialize)]
struct VersionedAuthDocument {
    version: u32,
    #[serde(default, alias = "providers")]
    credentials: AuthFile,
    #[serde(default)]
    accounts: BTreeMap<String, ProviderAccountSet>,
    #[serde(default)]
    assessments: BTreeMap<String, ProviderAuthAssessment>,
    #[serde(default, alias = "external_auth_trust")]
    external_trusts: BTreeMap<String, ExternalAuthTrustRecord>,
}

impl VersionedAuthDocument {
    fn into_document(self) -> AuthDocument {
        AuthDocument {
            version: self.version,
            credentials: self.credentials,
            accounts: self.accounts,
            assessments: self.assessments,
            external_trusts: self.external_trusts,
        }
    }
}

fn parse_auth_document_text(
    text: &str,
    path: &PathBuf,
) -> AuthResult<(AuthDocument, AuthDocumentFormat)> {
    let parsed =
        serde_json::from_str::<AuthDocumentOnDisk>(text).map_err(|source| AuthError::Json {
            path: path.clone(),
            source,
        })?;
    Ok(match parsed {
        AuthDocumentOnDisk::Versioned(document) => {
            (document.into_document(), AuthDocumentFormat::Versioned)
        }
        AuthDocumentOnDisk::Legacy(credentials) => (
            AuthDocument::from_credentials(credentials),
            AuthDocumentFormat::Legacy,
        ),
    })
}

#[cfg(unix)]
async fn write_secret_file(path: &PathBuf, contents: &[u8]) -> AuthResult<()> {
    use std::os::unix::fs::PermissionsExt;
    use tokio::io::AsyncWriteExt;

    let mut file = fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .open(path)
        .await
        .map_err(|source| AuthError::Io {
            path: path.clone(),
            source,
        })?;
    file.write_all(contents)
        .await
        .map_err(|source| AuthError::Io {
            path: path.clone(),
            source,
        })?;
    file.flush().await.map_err(|source| AuthError::Io {
        path: path.clone(),
        source,
    })?;
    fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
        .await
        .map_err(|source| AuthError::Io {
            path: path.clone(),
            source,
        })
}

#[cfg(not(unix))]
async fn write_secret_file(path: &PathBuf, contents: &[u8]) -> AuthResult<()> {
    fs::write(path, contents)
        .await
        .map_err(|source| AuthError::Io {
            path: path.clone(),
            source,
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_auth_path() -> PathBuf {
        let nonce = match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(duration) => duration.as_nanos(),
            Err(err) => panic!("system clock failed: {err}"),
        };
        std::env::temp_dir()
            .join(format!("oino-auth-test-{nonce}"))
            .join("nested")
            .join("auth.json")
    }

    #[tokio::test]
    async fn read_write_round_trip() {
        let storage = AuthStorage::new(AuthConfig::new(temp_auth_path()).with_process_env(false));
        if let Err(err) = storage
            .set_credential(OPENROUTER_AUTH_KEY, AuthCredential::api_key("sk-test"))
            .await
        {
            panic!("set credential failed: {err}");
        }
        let key = match storage.resolve_openrouter_api_key().await {
            Ok(key) => key,
            Err(err) => panic!("resolve failed: {err}"),
        };
        assert_eq!(key, "sk-test");
    }

    #[tokio::test]
    async fn missing_credential_is_typed() {
        let storage = AuthStorage::new(AuthConfig::new(temp_auth_path()).with_process_env(false));
        match storage.resolve_openrouter_api_key().await {
            Err(AuthError::MissingCredential { provider, .. }) => {
                assert_eq!(provider, OPENROUTER_PROVIDER_ID);
            }
            other => panic!("expected missing credential, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn malformed_file_is_typed() {
        let path = temp_auth_path();
        let parent = match path.parent() {
            Some(parent) => parent.to_path_buf(),
            None => panic!("temp path missing parent"),
        };
        if let Err(err) = fs::create_dir_all(parent).await {
            panic!("mkdir failed: {err}");
        }
        if let Err(err) = fs::write(&path, "not-json").await {
            panic!("write failed: {err}");
        }
        let storage = AuthStorage::new(AuthConfig::new(path));
        match storage.load().await {
            Err(AuthError::Json { .. }) => {}
            other => panic!("expected json error, got {other:?}"),
        }
    }

    #[test]
    fn openrouter_provider_spec_mapping() {
        let spec = ProviderAuthSpec::openrouter();
        assert_eq!(spec.provider_id, "openrouter");
        assert_eq!(spec.auth_key, "openrouter");
        assert_eq!(spec.env_var, "OPENROUTER_API_KEY");
    }

    #[tokio::test]
    async fn resolution_order_runtime_file_env() {
        let path = temp_auth_path();
        let file_storage = AuthStorage::new(AuthConfig::new(path.clone()));
        if let Err(err) = file_storage
            .set_credential(OPENROUTER_AUTH_KEY, AuthCredential::api_key("file"))
            .await
        {
            panic!("set credential failed: {err}");
        }
        let storage = AuthStorage::new(
            AuthConfig::new(path)
                .with_runtime_override(OPENROUTER_PROVIDER_ID, "runtime")
                .with_env_override(OPENROUTER_ENV_VAR, "env"),
        );
        let key = match storage.resolve_openrouter_api_key().await {
            Ok(key) => key,
            Err(err) => panic!("resolve failed: {err}"),
        };
        assert_eq!(key, "runtime");
    }

    #[tokio::test]
    async fn file_beats_env_override() {
        let path = temp_auth_path();
        let file_storage = AuthStorage::new(AuthConfig::new(path.clone()));
        if let Err(err) = file_storage
            .set_credential(OPENROUTER_AUTH_KEY, AuthCredential::api_key("file"))
            .await
        {
            panic!("set credential failed: {err}");
        }
        let storage =
            AuthStorage::new(AuthConfig::new(path).with_env_override(OPENROUTER_ENV_VAR, "env"));
        let key = match storage.resolve_openrouter_api_key().await {
            Ok(key) => key,
            Err(err) => panic!("resolve failed: {err}"),
        };
        assert_eq!(key, "file");
    }

    #[tokio::test]
    async fn env_override_is_fallback() {
        let storage = AuthStorage::new(
            AuthConfig::new(temp_auth_path()).with_env_override(OPENROUTER_ENV_VAR, "env"),
        );
        let key = match storage.resolve_openrouter_api_key().await {
            Ok(key) => key,
            Err(err) => panic!("resolve failed: {err}"),
        };
        assert_eq!(key, "env");
    }

    #[test]
    fn debug_redacts_secret() {
        let shown = format!("{:?}", AuthCredential::api_key("secret"));
        assert!(!shown.contains("secret"));
        assert!(shown.contains("redacted"));
    }

    #[test]
    fn debug_redacts_all_secret_variants() {
        let credentials = [
            AuthCredential::oauth_token(
                "access-secret",
                Some("refresh-secret".into()),
                Some(123),
                Some("acct".into()),
                vec!["scope".into()],
            ),
            AuthCredential::device_code_token(
                "device-secret",
                Some("device-refresh".into()),
                Some(456),
                None,
            ),
            AuthCredential::external(
                AuthCredentialSource::TrustedExternalFile,
                Some("/tmp/auth.json".into()),
                Some("print-secret".into()),
                None,
            ),
            AuthCredential::local_endpoint(
                "http://localhost:11434/v1",
                Some("local-secret".into()),
            ),
        ];
        for credential in credentials {
            let shown = format!("{credential:?}");
            assert!(!shown.contains("access-secret"));
            assert!(!shown.contains("refresh-secret"));
            assert!(!shown.contains("device-secret"));
            assert!(!shown.contains("device-refresh"));
            assert!(!shown.contains("print-secret"));
            assert!(!shown.contains("local-secret"));
            assert!(shown.contains("redacted"));
        }
    }

    #[tokio::test]
    async fn versioned_document_loads_credentials_and_accounts() {
        let path = temp_auth_path();
        let parent = match path.parent() {
            Some(parent) => parent.to_path_buf(),
            None => panic!("temp path missing parent"),
        };
        if let Err(err) = fs::create_dir_all(parent).await {
            panic!("mkdir failed: {err}");
        }
        let text = r#"
        {
          "version": 2,
          "credentials": {
            "openrouter": { "type": "api_key", "key": "from-v2" }
          },
          "accounts": {
            "openai": {
              "active_label": "work",
              "accounts": {
                "work": {
                  "label": "work",
                  "email": "dev@example.com",
                  "credential_key": "openai.work"
                }
              }
            }
          },
          "assessments": {
            "openrouter": {
              "state": "available",
              "readiness": "credential_present",
              "method_detail": "API key",
              "credential_source": "oino_auth_file",
              "credential_source_detail": "test",
              "expiry_confidence": "not_applicable",
              "refresh_support": "not_applicable",
              "validation_method": "presence_check"
            }
          },
          "external_trusts": {
            "opencode_auth_json::file_path::~/.local/share/opencode/auth.json": {
              "source_id": "opencode_auth_json",
              "decision": "trusted",
              "scope": "file_path",
              "granted_at_ms": 42,
              "provider_ids": ["openai", "claude"],
              "path": "~/.local/share/opencode/auth.json",
              "note": "fixture consent"
            }
          }
        }
        "#;
        if let Err(err) = fs::write(&path, text).await {
            panic!("write failed: {err}");
        }
        let storage = AuthStorage::new(AuthConfig::new(path).with_process_env(false));
        let key = match storage.resolve_openrouter_api_key().await {
            Ok(key) => key,
            Err(err) => panic!("resolve failed: {err}"),
        };
        assert_eq!(key, "from-v2");
        let document = match storage.load_document().await {
            Ok(document) => document,
            Err(err) => panic!("load document failed: {err}"),
        };
        assert_eq!(document.version, 2);
        assert_eq!(
            document
                .accounts
                .get("openai")
                .and_then(|accounts| accounts.active_label.as_deref()),
            Some("work")
        );
        assert_eq!(
            document
                .assessments
                .get("openrouter")
                .map(ProviderAuthAssessment::is_available),
            Some(true)
        );
        assert_eq!(document.external_trusts.len(), 1);
        assert_eq!(document.active_external_trust_count_at(43), 1);
    }

    #[tokio::test]
    async fn set_credential_preserves_versioned_document_accounts() {
        let path = temp_auth_path();
        let mut document = AuthDocument::default();
        document.credentials.insert(
            OPENROUTER_AUTH_KEY.into(),
            AuthCredential::api_key("before"),
        );
        document.accounts.insert(
            "openai".into(),
            ProviderAccountSet {
                active_label: Some("work".into()),
                accounts: BTreeMap::new(),
            },
        );
        let storage = AuthStorage::new(AuthConfig::new(path).with_process_env(false));
        if let Err(err) = storage.save_document(&document).await {
            panic!("save document failed: {err}");
        }
        if let Err(err) = storage
            .set_credential(OPENROUTER_AUTH_KEY, AuthCredential::api_key("after"))
            .await
        {
            panic!("set credential failed: {err}");
        }
        let loaded = match storage.load_document().await {
            Ok(document) => document,
            Err(err) => panic!("load document failed: {err}"),
        };
        assert_eq!(
            loaded
                .accounts
                .get("openai")
                .and_then(|accounts| accounts.active_label.as_deref()),
            Some("work")
        );
        assert_eq!(
            loaded
                .credentials
                .get(OPENROUTER_AUTH_KEY)
                .and_then(AuthCredential::as_api_key),
            Some("after")
        );
    }

    #[tokio::test]
    async fn external_trust_records_upgrade_legacy_file_to_versioned_document() {
        let path = temp_auth_path();
        let storage = AuthStorage::new(AuthConfig::new(path.clone()).with_process_env(false));
        if let Err(err) = storage
            .set_credential(OPENROUTER_AUTH_KEY, AuthCredential::api_key("before"))
            .await
        {
            panic!("set credential failed: {err}");
        }

        let record = ExternalAuthTrustRecord::trusted_file(
            "cursor_auth_json",
            vec!["cursor".into()],
            "~/.config/cursor/auth.json",
            100,
            Some("user approved exact path".into()),
        );
        let key = match storage.set_external_trust(record).await {
            Ok(key) => key,
            Err(err) => panic!("set external trust failed: {err}"),
        };
        assert_eq!(
            key,
            external_auth_trust_key(
                "cursor_auth_json",
                ExternalAuthTrustScope::FilePath,
                Some("~/.config/cursor/auth.json"),
                None,
            )
        );

        let document = match storage.load_document().await {
            Ok(document) => document,
            Err(err) => panic!("load document failed: {err}"),
        };
        assert_eq!(document.version, AUTH_DOCUMENT_VERSION);
        assert_eq!(document.active_external_trust_count_at(101), 1);
        assert_eq!(
            document
                .credentials
                .get(OPENROUTER_AUTH_KEY)
                .and_then(AuthCredential::as_api_key),
            Some("before")
        );
        let text = match fs::read_to_string(path).await {
            Ok(text) => text,
            Err(err) => panic!("read auth file failed: {err}"),
        };
        assert!(text.contains("external_trusts"));
    }

    #[tokio::test]
    async fn external_trust_records_can_be_revoked_and_denied() {
        let storage = AuthStorage::new(AuthConfig::new(temp_auth_path()).with_process_env(false));
        let trusted = ExternalAuthTrustRecord::trusted_source(
            "opencode_auth_json",
            vec!["openai".into(), "claude".into()],
            200,
            None,
        );
        let trusted_key = match storage.set_external_trust(trusted).await {
            Ok(key) => key,
            Err(err) => panic!("set trusted source failed: {err}"),
        };
        assert_eq!(
            storage.active_external_trust_count_at(201).await.unwrap(),
            1
        );

        let denied = ExternalAuthTrustRecord::denied_source(
            "pi_auth_json",
            vec!["openai".into()],
            201,
            Some("not this installation".into()),
        );
        if let Err(err) = storage.set_external_trust(denied).await {
            panic!("set denied source failed: {err}");
        }
        let trusts = match storage.external_trusts().await {
            Ok(trusts) => trusts,
            Err(err) => panic!("load trusts failed: {err}"),
        };
        assert_eq!(trusts.len(), 2);
        assert_eq!(
            storage.active_external_trust_count_at(202).await.unwrap(),
            1
        );
        assert!(trusts
            .values()
            .any(|record| record.summary().contains("denied source")));

        let revoked = match storage.revoke_external_trust(&trusted_key).await {
            Ok(revoked) => revoked,
            Err(err) => panic!("revoke trust failed: {err}"),
        };
        assert!(revoked);
        assert_eq!(
            storage.active_external_trust_count_at(203).await.unwrap(),
            0
        );
    }

    #[tokio::test]
    async fn non_api_key_credential_is_typed_error() {
        let storage = AuthStorage::new(AuthConfig::new(temp_auth_path()).with_process_env(false));
        if let Err(err) = storage
            .set_credential(
                OPENROUTER_AUTH_KEY,
                AuthCredential::oauth_token("access", None, None, None, Vec::new()),
            )
            .await
        {
            panic!("set credential failed: {err}");
        }
        match storage.resolve_openrouter_api_key().await {
            Err(AuthError::NotApiKey { provider }) => {
                assert_eq!(provider, OPENROUTER_PROVIDER_ID);
            }
            other => panic!("expected not-api-key error, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn resolve_with_source_reports_credential_source() {
        let storage = AuthStorage::new(
            AuthConfig::new(temp_auth_path())
                .with_process_env(false)
                .with_env_override(OPENROUTER_ENV_VAR, "env-key"),
        );
        let resolved = match storage
            .resolve_with_source(&ProviderAuthSpec::openrouter())
            .await
        {
            Ok(Some(resolved)) => resolved,
            Ok(None) => panic!("expected credential"),
            Err(err) => panic!("resolve failed: {err}"),
        };
        assert_eq!(resolved.source, AuthCredentialSource::EnvironmentVariable);
        assert_eq!(resolved.source_detail, OPENROUTER_ENV_VAR);
        let assessment = match storage
            .assess_api_key_provider(&ProviderAuthSpec::openrouter())
            .await
        {
            Ok(assessment) => assessment,
            Err(err) => panic!("assessment failed: {err}"),
        };
        assert!(assessment.is_available());
        assert!(assessment.health_summary().contains("readiness"));
    }
}
