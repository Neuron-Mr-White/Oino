//! pi `auth-storage.ts`: credentials in `<agent>/auth.json`.
//!
//! - Format: `{ "<provider>": {"type":"api_key",…} | {"type":"oauth",…} }`, written with
//!   two-space indentation like `JSON.stringify(data, null, 2)`.
//! - Read-modify-write runs under an exclusive lock on `auth.json.lock` (pi locks
//!   `auth.json.lock` too, through proper-lockfile). The file is replaced atomically (temp file +
//!   rename) so lock-free readers never see a partial write; new files are created `0600`, an
//!   existing file keeps its mode.
//! - Reads are cached by file revision (modification time + size).
//! - Runtime API keys (`--api-key`) overlay stored credentials without touching the file.

use std::fs::{File, OpenOptions, TryLockError};
use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::{Duration, Instant, SystemTime};

use indexmap::IndexMap;
use oino_ai::{JsonObject, JsonValue};

use super::credential::{AuthCredential, CredentialInfo};
use crate::config_value::resolve_config_value;
use crate::env::{EnvSource, ProcessEnv};
use crate::json_text::strip_bom;

/// How long a writer waits for another holder of the lock (pi: 30 s stale window).
const LOCK_TIMEOUT: Duration = Duration::from_secs(30);

/// Errors reading or writing `auth.json` (pi texts where pi has them).
#[derive(Debug, thiserror::Error)]
pub enum AuthStorageError {
    /// The file could not be read or parsed.
    #[error("Failed to read auth.json: {0}")]
    Read(String),
    /// The top level is not an object.
    #[error("Invalid auth.json: expected an object")]
    NotAnObject,
    /// An entry has the wrong shape.
    #[error("Invalid auth.json credential for provider \"{0}\"")]
    InvalidCredential(String),
    /// Mutation of a read-only store.
    #[error("Read-only credential storage cannot modify auth.json")]
    ReadOnly,
    /// The file could not be written.
    #[error("Failed to write auth.json: {0}")]
    Write(#[source] std::io::Error),
    /// Another process held the lock for too long.
    #[error("Timed out waiting for auth.json lock: {}", .0.display())]
    LockTimeout(PathBuf),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Revision {
    modified: SystemTime,
    len: u64,
}

fn revision(path: &Path) -> Option<Revision> {
    let metadata = std::fs::metadata(path).ok()?;
    Some(Revision {
        modified: metadata.modified().ok()?,
        len: metadata.len(),
    })
}

#[derive(Debug)]
enum Backend {
    File { path: PathBuf, read_only: bool },
    Memory(Mutex<JsonObject>),
}

#[derive(Debug, Default)]
struct Snapshot {
    data: JsonObject,
    revision: Option<Revision>,
    loaded: bool,
}

/// Credential storage (pi `AuthStorage` / `ReadOnlyAuthStorage` / in-memory backend) with
/// pi's `RuntimeCredentials` overlay.
pub struct AuthStorage {
    backend: Backend,
    runtime: RwLock<IndexMap<String, String>>,
    snapshot: Mutex<Snapshot>,
    async_gate: futures::lock::Mutex<()>,
    env: Arc<dyn EnvSource>,
}

impl std::fmt::Debug for AuthStorage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AuthStorage")
            .field("backend", &self.backend)
            .finish_non_exhaustive()
    }
}

/// Exclusive lock held for one read-modify-write; released on drop.
struct LockGuard {
    file: File,
}

impl Drop for LockGuard {
    fn drop(&mut self) {
        // Unlock failures are harmless: closing the file releases the lock as well.
        let _ = self.file.unlock();
    }
}

fn parse_data(content: &str) -> Result<JsonObject, AuthStorageError> {
    let content = strip_bom(content);
    if content.is_empty() {
        return Ok(JsonObject::new());
    }
    match serde_json::from_str::<JsonValue>(content) {
        Ok(JsonValue::Object(map)) => Ok(map),
        Ok(_) => Err(AuthStorageError::NotAnObject),
        Err(error) => Err(AuthStorageError::Read(error.to_string())),
    }
}

fn validate_entries(data: &JsonObject) -> Result<(), AuthStorageError> {
    for (provider, value) in data {
        if AuthCredential::from_value(value).is_none() {
            return Err(AuthStorageError::InvalidCredential(provider.clone()));
        }
    }
    Ok(())
}

fn serialize_data(data: &JsonObject) -> String {
    serde_json::to_string_pretty(data).unwrap_or_else(|_| "{}".to_owned())
}

impl AuthStorage {
    fn with_backend(backend: Backend) -> Self {
        Self {
            backend,
            runtime: RwLock::new(IndexMap::new()),
            snapshot: Mutex::new(Snapshot::default()),
            async_gate: futures::lock::Mutex::new(()),
            env: Arc::new(ProcessEnv),
        }
    }

    /// pi `AuthStorage.create(path)`: file-backed, read-write.
    #[must_use]
    pub fn open(path: impl Into<PathBuf>) -> Self {
        Self::with_backend(Backend::File {
            path: path.into(),
            read_only: false,
        })
    }

    /// pi `ReadOnlyAuthStorage`: validates every entry and refuses mutations.
    #[must_use]
    pub fn open_read_only(path: impl Into<PathBuf>) -> Self {
        Self::with_backend(Backend::File {
            path: path.into(),
            read_only: true,
        })
    }

    /// pi `AuthStorage.inMemory(data)`.
    #[must_use]
    pub fn in_memory(data: IndexMap<String, AuthCredential>) -> Self {
        let object = data
            .into_iter()
            .map(|(provider, credential)| (provider, credential.to_value()))
            .collect();
        Self::with_backend(Backend::Memory(Mutex::new(object)))
    }

    /// Replaces the environment used to resolve `$ENV` keys (default: the process env).
    #[must_use]
    pub fn with_env(mut self, env: Arc<dyn EnvSource>) -> Self {
        self.env = env;
        self
    }

    /// The `auth.json` path for file-backed storage.
    #[must_use]
    pub fn path(&self) -> Option<&Path> {
        match &self.backend {
            Backend::File { path, .. } => Some(path),
            Backend::Memory(_) => None,
        }
    }

    /// Whether mutations are refused.
    #[must_use]
    pub fn is_read_only(&self) -> bool {
        matches!(
            self.backend,
            Backend::File {
                read_only: true,
                ..
            }
        )
    }

    fn read_file(path: &Path, read_only: bool) -> Result<JsonObject, AuthStorageError> {
        let content = match std::fs::read_to_string(path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(AuthStorageError::Read(error.to_string())),
        };
        let data = parse_data(&content)?;
        if read_only {
            validate_entries(&data)?;
        }
        Ok(data)
    }

    /// Current data, reloaded when the file revision changed.
    ///
    /// Read-write storage keeps the last valid snapshot when the file is unreadable (pi);
    /// read-only storage reports the error.
    fn data(&self) -> Result<JsonObject, AuthStorageError> {
        match &self.backend {
            Backend::Memory(data) => {
                Ok(data.lock().unwrap_or_else(PoisonError::into_inner).clone())
            }
            Backend::File { path, read_only } => {
                let current = revision(path);
                let mut snapshot = self.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
                if snapshot.loaded && current.is_some() && snapshot.revision == current {
                    return Ok(snapshot.data.clone());
                }
                match Self::read_file(path, *read_only) {
                    Ok(data) => {
                        snapshot.data = data;
                        snapshot.revision = current;
                        snapshot.loaded = true;
                        Ok(snapshot.data.clone())
                    }
                    Err(error) if !*read_only => {
                        tracing::warn!(%error, path = %path.display(), "keeping last auth.json snapshot");
                        Ok(snapshot.data.clone())
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    /// Forces the next read to reload the file (pi `reload`).
    pub fn reload(&self) {
        let mut snapshot = self.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
        snapshot.revision = None;
        snapshot.loaded = false;
    }

    /// The stored credential exactly as written (no runtime overlay, no key resolution;
    /// pi `readStoredCredential`). Entries of unknown shape read as `None`.
    ///
    /// # Errors
    ///
    /// Read-only storage reports unreadable or invalid files.
    pub fn stored(&self, provider: &str) -> Result<Option<AuthCredential>, AuthStorageError> {
        let data = self.data()?;
        Ok(data.get(provider).and_then(AuthCredential::from_value))
    }

    /// pi `read`: the runtime key if set, else the stored credential with an API key's config
    /// value resolved (`!command` cached, `$ENV` with the credential's `env` first). A key that
    /// does not resolve becomes `None`.
    ///
    /// # Errors
    ///
    /// Read-only storage reports unreadable or invalid files.
    pub fn get(&self, provider: &str) -> Result<Option<AuthCredential>, AuthStorageError> {
        if let Some(key) = self.runtime_api_key(provider) {
            return Ok(Some(AuthCredential::api_key(key)));
        }
        Ok(self.stored(provider)?.map(|credential| match credential {
            AuthCredential::ApiKey(mut api_key) => {
                if let Some(key) = &api_key.key {
                    api_key.key =
                        resolve_config_value(key, api_key.env.as_ref(), self.env.as_ref());
                }
                AuthCredential::ApiKey(api_key)
            }
            oauth @ AuthCredential::Oauth(_) => oauth,
        }))
    }

    /// pi `list`: stored entries plus runtime keys (as `api_key`).
    ///
    /// # Errors
    ///
    /// Read-only storage reports unreadable or invalid files.
    pub fn list(&self) -> Result<Vec<CredentialInfo>, AuthStorageError> {
        let mut entries: IndexMap<String, String> = self
            .data()?
            .iter()
            .map(|(provider, value)| {
                let kind = value
                    .get("type")
                    .and_then(JsonValue::as_str)
                    .unwrap_or_default()
                    .to_owned();
                (provider.clone(), kind)
            })
            .collect();
        for provider in self.read_runtime().keys() {
            entries.insert(provider.clone(), "api_key".to_owned());
        }
        Ok(entries
            .into_iter()
            .map(|(provider_id, kind)| CredentialInfo { provider_id, kind })
            .collect())
    }

    /// Whether a credential (stored or runtime) exists for `provider`.
    #[must_use]
    pub fn has(&self, provider: &str) -> bool {
        self.has_runtime_api_key(provider)
            || self.data().is_ok_and(|data| data.contains_key(provider))
    }

    /// Whether `auth.json` holds an entry for `provider` (runtime keys excluded).
    #[must_use]
    pub fn has_stored(&self, provider: &str) -> bool {
        self.data().is_ok_and(|data| data.contains_key(provider))
    }

    /// Stores `credential` for `provider`.
    ///
    /// # Errors
    ///
    /// Lock, write or read-only failures.
    pub fn set(&self, provider: &str, credential: &AuthCredential) -> Result<(), AuthStorageError> {
        self.write_with(|data| {
            data.insert(provider.to_owned(), credential.to_value());
            true
        })
    }

    /// pi `delete`: removes the stored credential and any runtime key.
    ///
    /// # Errors
    ///
    /// Lock, write or read-only failures.
    pub fn remove(&self, provider: &str) -> Result<(), AuthStorageError> {
        self.write_with(|data| {
            data.shift_remove(provider);
            true
        })?;
        self.write_runtime().shift_remove(provider);
        Ok(())
    }

    /// pi `modify` (synchronous): `f` receives the current credential under the lock; `Some`
    /// replaces it, `None` leaves the file untouched. Returns the credential after the call.
    ///
    /// # Errors
    ///
    /// Lock, write or read-only failures.
    pub fn modify(
        &self,
        provider: &str,
        f: impl FnOnce(Option<AuthCredential>) -> Option<AuthCredential>,
    ) -> Result<Option<AuthCredential>, AuthStorageError> {
        let mut result = None;
        self.write_with(|data| {
            let current = data.get(provider).and_then(AuthCredential::from_value);
            if let Some(next) = f(current.clone()) {
                data.insert(provider.to_owned(), next.to_value());
                result = Some(next);
                true
            } else {
                result = current;
                false
            }
        })?;
        Ok(result)
    }

    /// pi `modify` with an asynchronous callback (OAuth refresh). Concurrent calls in this
    /// process are serialized; the file lock is held while `f` runs, so other processes
    /// refresh at most once.
    ///
    /// # Errors
    ///
    /// Storage failures (converted into `E`) or the callback's error; on error nothing is
    /// written.
    pub async fn modify_async<F, Fut, E>(
        &self,
        provider: &str,
        f: F,
    ) -> Result<Option<AuthCredential>, E>
    where
        F: FnOnce(Option<AuthCredential>) -> Fut,
        Fut: Future<Output = Result<Option<AuthCredential>, E>>,
        E: From<AuthStorageError>,
    {
        let _gate = self.async_gate.lock().await;
        if self.is_read_only() {
            return Err(AuthStorageError::ReadOnly.into());
        }
        let _lock = self.acquire_lock()?;
        let mut data = self.read_locked()?;
        let current = data.get(provider).and_then(AuthCredential::from_value);
        match f(current.clone()).await? {
            Some(next) => {
                data.insert(provider.to_owned(), next.to_value());
                self.write_locked(&data)?;
                Ok(Some(next))
            }
            None => Ok(current),
        }
    }

    /// pi `setRuntimeApiKey`: a non-persistent key that wins over `auth.json`.
    pub fn set_runtime_api_key(&self, provider: &str, key: impl Into<String>) {
        self.write_runtime().insert(provider.to_owned(), key.into());
    }

    /// pi `removeRuntimeApiKey`.
    pub fn remove_runtime_api_key(&self, provider: &str) {
        self.write_runtime().shift_remove(provider);
    }

    /// pi `hasRuntimeApiKey`.
    #[must_use]
    pub fn has_runtime_api_key(&self, provider: &str) -> bool {
        self.read_runtime().contains_key(provider)
    }

    fn runtime_api_key(&self, provider: &str) -> Option<String> {
        self.read_runtime().get(provider).cloned()
    }

    fn read_runtime(&self) -> std::sync::RwLockReadGuard<'_, IndexMap<String, String>> {
        self.runtime.read().unwrap_or_else(PoisonError::into_inner)
    }

    fn write_runtime(&self) -> std::sync::RwLockWriteGuard<'_, IndexMap<String, String>> {
        self.runtime.write().unwrap_or_else(PoisonError::into_inner)
    }

    /// Runs one locked read-modify-write; `mutate` returns whether to write.
    fn write_with(
        &self,
        mutate: impl FnOnce(&mut JsonObject) -> bool,
    ) -> Result<(), AuthStorageError> {
        if self.is_read_only() {
            return Err(AuthStorageError::ReadOnly);
        }
        let _lock = self.acquire_lock()?;
        let mut data = self.read_locked()?;
        if mutate(&mut data) {
            self.write_locked(&data)?;
        }
        Ok(())
    }

    fn read_locked(&self) -> Result<JsonObject, AuthStorageError> {
        match &self.backend {
            Backend::Memory(data) => {
                Ok(data.lock().unwrap_or_else(PoisonError::into_inner).clone())
            }
            Backend::File { path, .. } => Self::read_file(path, false),
        }
    }

    fn write_locked(&self, data: &JsonObject) -> Result<(), AuthStorageError> {
        match &self.backend {
            Backend::Memory(stored) => {
                stored
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone_from(data);
                Ok(())
            }
            Backend::File { path, .. } => {
                write_atomic(path, &serialize_data(data)).map_err(AuthStorageError::Write)?;
                let mut snapshot = self.snapshot.lock().unwrap_or_else(PoisonError::into_inner);
                snapshot.data.clone_from(data);
                snapshot.revision = revision(path);
                snapshot.loaded = true;
                Ok(())
            }
        }
    }

    fn acquire_lock(&self) -> Result<Option<LockGuard>, AuthStorageError> {
        let Backend::File { path, .. } = &self.backend else {
            return Ok(None);
        };
        let lock_path = lock_path(path);
        if let Some(parent) = lock_path.parent() {
            create_private_dir(parent).map_err(AuthStorageError::Write)?;
        }
        let file = open_private(&lock_path, false).map_err(AuthStorageError::Write)?;
        let deadline = Instant::now() + LOCK_TIMEOUT;
        let mut delay = Duration::from_millis(10);
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(LockGuard { file })),
                Err(TryLockError::WouldBlock) if Instant::now() < deadline => {
                    std::thread::sleep(delay);
                    delay = (delay * 2).min(Duration::from_secs(1));
                }
                Err(TryLockError::WouldBlock) => {
                    return Err(AuthStorageError::LockTimeout(lock_path));
                }
                Err(TryLockError::Error(error)) => return Err(AuthStorageError::Write(error)),
            }
        }
    }
}

fn lock_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    name.push(".lock");
    path.with_file_name(name)
}

fn create_private_dir(dir: &Path) -> std::io::Result<()> {
    if dir.as_os_str().is_empty() || dir.exists() {
        return Ok(());
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(dir)
}

fn open_private(path: &Path, truncate: bool) -> std::io::Result<File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(truncate);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path)
}

/// Writes `content` to a sibling temp file and renames it over `path`, keeping an existing
/// file's permissions.
pub(crate) fn write_atomic(path: &Path, content: &str) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        create_private_dir(parent)?;
    }
    let mut temp_name = path
        .file_name()
        .map(std::ffi::OsStr::to_os_string)
        .unwrap_or_default();
    temp_name.push(format!(".tmp-{}", std::process::id()));
    let temp = path.with_file_name(temp_name);
    {
        let mut file = open_private(&temp, true)?;
        if let Ok(metadata) = std::fs::metadata(path) {
            file.set_permissions(metadata.permissions())?;
        }
        file.write_all(content.as_bytes())?;
        file.sync_all()?;
    }
    std::fs::rename(&temp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&temp);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::env::MapEnv;

    fn temp_dir() -> tempfile::TempDir {
        let root = std::env::var_os("OINO_TEST_TMP").map_or_else(
            || PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp"),
            PathBuf::from,
        );
        std::fs::create_dir_all(&root).unwrap();
        tempfile::tempdir_in(root).unwrap()
    }

    #[test]
    fn file_round_trip_and_permissions() {
        let dir = temp_dir();
        let path = dir.path().join("agent/auth.json");
        let storage = AuthStorage::open(&path);
        assert_eq!(storage.get("a").unwrap(), None);
        storage.set("a", &AuthCredential::api_key("k1")).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(
            text,
            "{\n  \"a\": {\n    \"type\": \"api_key\",\n    \"key\": \"k1\"\n  }\n}"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
        let other = AuthStorage::open(&path);
        assert_eq!(other.get("a").unwrap(), Some(AuthCredential::api_key("k1")));
        other.remove("a").unwrap();
        assert!(!storage.has("a"));
        assert_eq!(storage.list().unwrap(), vec![]);
    }

    #[test]
    fn concurrent_writers_do_not_lose_updates() {
        let dir = temp_dir();
        let path = dir.path().join("auth.json");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let storage = AuthStorage::open(&path);
                    storage
                        .set(&format!("p{i}"), &AuthCredential::api_key(format!("k{i}")))
                        .unwrap();
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }
        let storage = AuthStorage::open(&path);
        assert_eq!(storage.list().unwrap().len(), 8);
    }

    #[test]
    fn runtime_keys_overlay() {
        let storage =
            AuthStorage::in_memory([("a".to_owned(), AuthCredential::api_key("stored"))].into());
        storage.set_runtime_api_key("a", "runtime");
        storage.set_runtime_api_key("b", "rb");
        assert_eq!(
            storage.get("a").unwrap(),
            Some(AuthCredential::api_key("runtime"))
        );
        let kinds: Vec<_> = storage
            .list()
            .unwrap()
            .into_iter()
            .map(|i| (i.provider_id, i.kind))
            .collect();
        assert_eq!(
            kinds,
            vec![
                ("a".into(), "api_key".into()),
                ("b".into(), "api_key".into())
            ]
        );
        storage.remove("a").unwrap();
        assert!(!storage.has("a"));
    }

    #[test]
    fn resolves_env_keys() {
        let storage =
            AuthStorage::in_memory([("a".to_owned(), AuthCredential::api_key("$KEY"))].into())
                .with_env(Arc::new(MapEnv::new().with("KEY", "secret")));
        assert_eq!(
            storage.get("a").unwrap(),
            Some(AuthCredential::api_key("secret"))
        );
        assert_eq!(
            storage.stored("a").unwrap(),
            Some(AuthCredential::api_key("$KEY"))
        );
    }

    #[test]
    fn read_only_validates() {
        let dir = temp_dir();
        let path = dir.path().join("auth.json");
        std::fs::write(&path, r#"{"a": {"type": "api_key", "key": 5}}"#).unwrap();
        let storage = AuthStorage::open_read_only(&path);
        assert_eq!(
            storage.get("a").unwrap_err().to_string(),
            "Invalid auth.json credential for provider \"a\""
        );
        assert!(matches!(
            storage.set("b", &AuthCredential::api_key("x")),
            Err(AuthStorageError::ReadOnly)
        ));
        std::fs::write(&path, "[]").unwrap();
        storage.reload();
        assert_eq!(
            storage.list().unwrap_err().to_string(),
            "Invalid auth.json: expected an object"
        );
    }

    #[test]
    fn modify_async_serializes() {
        let storage = AuthStorage::in_memory(IndexMap::new());
        let result: Result<_, AuthStorageError> =
            futures::executor::block_on(storage.modify_async("a", |current| async move {
                assert!(current.is_none());
                Ok(Some(AuthCredential::api_key("x")))
            }));
        assert_eq!(result.unwrap(), Some(AuthCredential::api_key("x")));
        assert!(storage.has("a"));
    }
}
