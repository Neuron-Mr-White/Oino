#![doc = r#"Credential storage and resolution for Oino providers.

This crate stores provider credentials and resolves API keys for provider adapters. It
intentionally does not know provider HTTP protocols or depend on the harness, TUI, or app
crates.
"#]
#![forbid(unsafe_code)]

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, fmt, path::PathBuf};
use thiserror::Error;
use tokio::fs;

pub const OPENROUTER_PROVIDER_ID: &str = "openrouter";
pub const OPENROUTER_ENV_VAR: &str = "OPENROUTER_API_KEY";
pub const OPENROUTER_AUTH_KEY: &str = "openrouter";

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

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthCredential {
    ApiKey { key: String },
}

impl AuthCredential {
    #[must_use]
    pub fn api_key(key: impl Into<String>) -> Self {
        Self::ApiKey { key: key.into() }
    }

    #[must_use]
    pub fn as_api_key(&self) -> Option<&str> {
        match self {
            Self::ApiKey { key } => Some(key.as_str()),
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
        match fs::read_to_string(&self.config.auth_path).await {
            Ok(text) => serde_json::from_str(&text).map_err(|source| AuthError::Json {
                path: self.config.auth_path.clone(),
                source,
            }),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(source) => Err(AuthError::Io {
                path: self.config.auth_path.clone(),
                source,
            }),
        }
    }

    pub async fn save_all(&self, entries: &AuthFile) -> AuthResult<()> {
        if let Some(parent) = self.config.auth_path.parent() {
            fs::create_dir_all(parent)
                .await
                .map_err(|source| AuthError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
        }
        let text = serde_json::to_string_pretty(entries).map_err(|source| AuthError::Json {
            path: self.config.auth_path.clone(),
            source,
        })?;
        write_secret_file(&self.config.auth_path, text.as_bytes()).await
    }

    pub async fn set_credential(
        &self,
        auth_key: impl Into<String>,
        credential: AuthCredential,
    ) -> AuthResult<()> {
        let mut entries = self.load().await?;
        entries.insert(auth_key.into(), credential);
        self.save_all(&entries).await
    }

    pub async fn delete_credential(&self, auth_key: &str) -> AuthResult<bool> {
        let mut entries = self.load().await?;
        let existed = entries.remove(auth_key).is_some();
        self.save_all(&entries).await?;
        Ok(existed)
    }

    pub async fn resolve(&self, spec: &ProviderAuthSpec) -> AuthResult<Option<AuthCredential>> {
        if let Some(credential) = self.config.runtime_overrides.get(&spec.provider_id) {
            return Ok(Some(credential.clone()));
        }
        let entries = self.load().await?;
        if let Some(credential) = entries.get(&spec.auth_key) {
            return Ok(Some(credential.clone()));
        }
        if let Some(value) = self.config.env_overrides.get(&spec.env_var) {
            return Ok(Some(AuthCredential::api_key(value.clone())));
        }
        if !self.config.process_env {
            return Ok(None);
        }
        match std::env::var(&spec.env_var) {
            Ok(value) if !value.trim().is_empty() => Ok(Some(AuthCredential::api_key(value))),
            Ok(_) | Err(std::env::VarError::NotPresent) => Ok(None),
            Err(std::env::VarError::NotUnicode(_)) => Ok(None),
        }
    }

    pub async fn resolve_api_key(&self, spec: &ProviderAuthSpec) -> AuthResult<String> {
        let Some(credential) = self.resolve(spec).await? else {
            return Err(AuthError::MissingCredential {
                provider: spec.provider_id.clone(),
                auth_key: spec.auth_key.clone(),
                env_var: spec.env_var.clone(),
            });
        };
        match credential.as_api_key() {
            Some(key) => Ok(key.to_string()),
            None => Err(AuthError::NotApiKey {
                provider: spec.provider_id.clone(),
            }),
        }
    }

    pub async fn resolve_openrouter_api_key(&self) -> AuthResult<String> {
        self.resolve_api_key(&ProviderAuthSpec::openrouter()).await
    }
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
}
