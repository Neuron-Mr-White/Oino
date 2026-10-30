//! Data directory resolution and atomic, permission-restricted file
//! I/O for `unipi-host`.
//!
//! Layout (see `docs/m1/host-notes.md`):
//! ```text
//! $UNIPI_HOST_DIR/ (default: ~/.unipi/app-host)
//!   secret.key     0600  raw 32-byte iroh secret key
//!   devices.json   0600  paired device allowlist
//!   pairing.json   0600  single in-flight pairing attempt (if any)
//!   endpoint.json  0600  cached iroh ticket/address for `pair` display
//!   config.json    0600  owner settings (port forwarding, UNI-213; off by default;
//!                        `fs` allow/deny extra Files roots, UNI-220)
//! ```
//!
//! All writes are atomic: content is written to a sibling `.tmp-*`
//! file, `fsync`ed, then renamed into place. On Unix, file
//! permissions are tightened to `0600` right after creation.

use std::io;
use std::path::{Path, PathBuf};

/// Env var overriding the data directory. See `docs/m1/host-notes.md`.
pub const UNIPI_HOST_DIR_ENV: &str = "UNIPI_HOST_DIR";

/// Resolved paths for all files `unipi-host` persists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPaths {
    pub root: PathBuf,
}

impl HostPaths {
    /// Builds `HostPaths` rooted at an explicit directory. Prefer this
    /// in tests (with a tempdir) to avoid ever touching the real
    /// default directory.
    pub fn from_dir(root: impl Into<PathBuf>) -> Self {
        HostPaths { root: root.into() }
    }

    /// Resolves the data directory from `$UNIPI_HOST_DIR`, falling
    /// back to `~/.unipi/app-host`. This reads the environment and
    /// `$HOME`; it performs no I/O itself.
    pub fn resolve() -> Self {
        if let Ok(dir) = std::env::var(UNIPI_HOST_DIR_ENV) {
            if !dir.is_empty() {
                return HostPaths::from_dir(dir);
            }
        }
        HostPaths::from_dir(default_data_dir())
    }

    pub fn secret_key(&self) -> PathBuf {
        self.root.join("secret.key")
    }

    pub fn devices(&self) -> PathBuf {
        self.root.join("devices.json")
    }

    pub fn pairing(&self) -> PathBuf {
        self.root.join("pairing.json")
    }

    pub fn endpoint(&self) -> PathBuf {
        self.root.join("endpoint.json")
    }

    /// The per-pairing E2E notify key (UNI-161 §1): 32 raw bytes, generated
    /// at first use. Also documented in `docs/m6/PROTOCOL.md` and
    /// `packages/notify/README.md` since the pi side reads this same file
    /// to encrypt notification payloads.
    pub fn notify_key(&self) -> PathBuf {
        self.root.join("notify-key")
    }

    /// Host settings the owner edits (`crate::forward::HostConfig`):
    /// `{"forwarding":{"enabled":true,"ports":"any"|[..],"devices":[..]}}`.
    pub fn config(&self) -> PathBuf {
        self.root.join("config.json")
    }

    /// Where uploaded/cached blobs live (`crate::blob`).
    pub fn blobs(&self) -> PathBuf {
        self.root.join("blobs")
    }

    /// Ensures the data directory exists (created with restrictive
    /// permissions on Unix).
    pub fn ensure_dir(&self) -> io::Result<()> {
        std::fs::create_dir_all(&self.root)?;
        restrict_dir_permissions(&self.root)?;
        Ok(())
    }
}

/// `~/.unipi/app-host`, or `.unipi/app-host` relative to the current
/// directory if `$HOME` is unset (should not happen on supported
/// platforms, but we never panic over it).
fn default_data_dir() -> PathBuf {
    // UNI-298: the shared rule (also used by the desktop app).
    let env = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty()).map(PathBuf::from);
    unipi_proto::paths::host_dir_from(cfg!(windows), None, env("HOME"), env("USERPROFILE"), env("LOCALAPPDATA"), &|d| d.join("secret.key").exists())
}

/// UNI-266: Windows has no `HOME` outside ssh/Git Bash, so the logon task's
/// `serve` (cwd `%LOCALAPPDATA%\UniPi`) and a PowerShell `unipi-host pair`
/// used to resolve *different* relative dirs. Windows now falls back to
/// `%USERPROFILE%`; an identity already created in the old place
/// (`%LOCALAPPDATA%\UniPi\.unipi\app-host`) is kept so paired phones stay paired.
pub fn default_data_dir_from(windows: bool, home: Option<PathBuf>, profile: Option<PathBuf>, local_app_data: Option<PathBuf>) -> PathBuf {
    let sub = |p: PathBuf| p.join(".unipi").join("app-host");
    if !windows {
        return home.map(sub).unwrap_or_else(|| sub(PathBuf::new()));
    }
    let preferred = home.or(profile).map(sub);
    let legacy = local_app_data.map(|l| sub(l.join("UniPi")));
    match (preferred, legacy) {
        (Some(p), Some(l)) if !p.join("secret.key").exists() && l.join("secret.key").exists() => l,
        (Some(p), _) => p,
        (None, Some(l)) => l,
        (None, None) => sub(PathBuf::new()),
    }
}

#[cfg(unix)]
fn restrict_dir_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o700);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn restrict_dir_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn restrict_file_permissions(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = std::fs::Permissions::from_mode(0o600);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn restrict_file_permissions(_path: &Path) -> io::Result<()> {
    Ok(())
}

/// Atomically writes `contents` to `path`: write to a sibling temp
/// file, sync, rename, then tighten permissions to `0600` (Unix
/// only). The directory is created if missing.
pub fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let dir = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "path has no parent directory")
    })?;
    std::fs::create_dir_all(dir)?;
    restrict_dir_permissions(dir)?;

    let file_name = path
        .file_name()
        .and_then(|n| n.to_str())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "path has no file name"))?;
    let tmp_path = dir.join(format!(
        ".tmp-{file_name}-{}-{:016x}",
        std::process::id(),
        rand::random::<u64>()
    ));

    {
        use std::io::Write;
        let mut options = std::fs::OpenOptions::new();
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let mut file = options
            .write(true)
            .create(true)
            .truncate(true)
            .open(&tmp_path)?;
        file.write_all(contents)?;
        file.sync_all()?;
    }
    restrict_file_permissions(&tmp_path)?;
    std::fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Reads `path` if it exists, returning `None` for `NotFound`.
pub fn read_optional(path: &Path) -> io::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(err) => Err(err),
    }
}

/// Reads and JSON-deserializes `path`, returning `None` if it
/// doesn't exist.
pub fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> anyhow::Result<Option<T>> {
    match read_optional(path)? {
        Some(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        None => Ok(None),
    }
}

/// JSON-serializes `value` and writes it atomically to `path`.
pub fn write_json<T: serde::Serialize>(path: &Path, value: &T) -> anyhow::Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    write_atomic(path, &bytes)?;
    Ok(())
}

/// Removes `path` if it exists; a missing file is not an error.
pub fn remove_optional(path: &Path) -> io::Result<()> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_default_dir_from_home_without_touching_disk() {
        let _guard = EnvGuard::unset(UNIPI_HOST_DIR_ENV);
        let home = std::env::var_os("HOME");
        let paths = HostPaths::resolve();
        if let Some(home) = home {
            assert_eq!(
                paths.root,
                PathBuf::from(home).join(".unipi").join("app-host")
            );
        }
    }

    #[test]
    fn windows_default_dir_without_home() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let profile = tmp.path().join("profile");
        let lad = tmp.path().join("lad");
        let sub = |p: &Path| p.join(".unipi").join("app-host");
        // Unix: HOME only.
        assert_eq!(default_data_dir_from(false, Some(profile.clone()), None, Some(lad.clone())), sub(&profile));
        // Windows, no HOME: USERPROFILE.
        assert_eq!(default_data_dir_from(true, None, Some(profile.clone()), Some(lad.clone())), sub(&profile));
        // …unless the old logon-task dir already holds the identity.
        let legacy = sub(&lad.join("UniPi"));
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("secret.key"), [0u8; 32]).unwrap();
        assert_eq!(default_data_dir_from(true, None, Some(profile.clone()), Some(lad.clone())), legacy);
        // ssh sets HOME = the profile: same answer.
        assert_eq!(default_data_dir_from(true, Some(profile.clone()), Some(profile.clone()), Some(lad.clone())), legacy);
        // Once the profile dir has its own identity it wins.
        std::fs::create_dir_all(sub(&profile)).unwrap();
        std::fs::write(sub(&profile).join("secret.key"), [0u8; 32]).unwrap();
        assert_eq!(default_data_dir_from(true, None, Some(profile.clone()), Some(lad.clone())), sub(&profile));
    }

    #[test]
    fn env_var_overrides_default_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _guard = EnvGuard::set(UNIPI_HOST_DIR_ENV, tmp.path());
        let paths = HostPaths::resolve();
        assert_eq!(paths.root, tmp.path());
    }

    #[test]
    fn write_atomic_then_read_round_trips() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("nested").join("file.txt");
        write_atomic(&path, b"hello").expect("write");
        let read_back = read_optional(&path).expect("read").expect("some");
        assert_eq!(read_back, b"hello");
    }

    #[cfg(unix)]
    #[test]
    fn write_atomic_sets_0600() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("secret.key");
        write_atomic(&path, b"secret").expect("write");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn read_optional_missing_is_none() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("missing.txt");
        assert_eq!(read_optional(&path).expect("read"), None);
    }

    #[test]
    fn remove_optional_missing_is_ok() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let path = tmp.path().join("missing.txt");
        assert!(remove_optional(&path).is_ok());
    }

    /// Minimal env var guard: serializes access to the one var it
    /// touches via a global mutex so tests can run in parallel
    /// without clobbering each other's `UNIPI_HOST_DIR`.
    pub struct EnvGuard {
        key: &'static str,
        previous: Option<std::ffi::OsString>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    impl EnvGuard {
        pub fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            EnvGuard {
                key,
                previous,
                _lock: lock,
            }
        }

        pub fn unset(key: &'static str) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let previous = std::env::var_os(key);
            std::env::remove_var(key);
            EnvGuard {
                key,
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(v) => std::env::set_var(self.key, v),
                None => std::env::remove_var(self.key),
            }
        }
    }
}
