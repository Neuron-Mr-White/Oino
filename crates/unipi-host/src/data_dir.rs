//! Data directory resolution and atomic, permission-restricted file
//! I/O for `unipi-host`.
//!
//! Layout (v2 spec host.md §5.8):
//! ```text
//! $OINO_HOST_DIR/ (default: ~/.oino/host)
//!   secret.key     0600  raw 32-byte iroh secret key
//!   devices.json   0600  paired device allowlist
//!   pairing.json   0600  single in-flight pairing attempt (if any)
//!   endpoint.json  0600  cached iroh ticket/address for `pair` display
//!   config.json    0600  owner settings (port forwarding, UNI-213; off by default;
//!                        `fs` allow/deny extra Files roots, UNI-220)
//!   host.lock      0600  one host per user (`instance.rs`)
//!   host.log             the daemon's stdout/stderr (app-spawned hosts)
//!   app.sock / pipe     the local listener (`local.rs`)
//!   control.sock  0600  the CLI's control socket (`push.rs`)
//! ```
//!
//! All writes are atomic: content is written to a sibling `.tmp-*`
//! file, `fsync`ed, then renamed into place. On Unix, file
//! permissions are tightened to `0600` right after creation.

use std::io;
use std::path::{Path, PathBuf};

/// Env var overriding the data directory (v2: `$OINO_HOST_DIR`; the old
/// unipi read `$UNIPI_HOST_DIR` and keeps its own `~/.unipi/app-host`).
pub const OINO_HOST_DIR_ENV: &str = "OINO_HOST_DIR";

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

    /// Resolves the data directory from `$OINO_HOST_DIR`, falling
    /// back to `~/.oino/host`. This reads the environment and
    /// `$HOME`; it performs no I/O itself.
    pub fn resolve() -> Self {
        if let Ok(dir) = std::env::var(OINO_HOST_DIR_ENV) {
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

    /// Host settings the owner edits (`crate::forward::HostConfig`,
    /// `crate::fs_scope::FsAccessConfig`, …): see the module docs for the
    /// v2 keys (`relay`, `term`, `update`, …).
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

/// `~/.oino/host`, or `.oino/host` relative to the current
/// directory if `$HOME` is unset (should not happen on supported
/// platforms, but we never panic over it). The v1 `~/.unipi` tree is
/// never touched (coexistence, spec §5.1).
fn default_data_dir() -> PathBuf {
    // The shared rule (also used by the desktop app's `local_host.rs`).
    let env = |k: &str| {
        std::env::var_os(k)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    unipi_proto::paths::host_dir_from(cfg!(windows), None, env("HOME"), env("USERPROFILE"))
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
        let _guard = EnvGuard::unset(OINO_HOST_DIR_ENV);
        let home = std::env::var_os("HOME");
        let paths = HostPaths::resolve();
        if let Some(home) = home {
            assert_eq!(paths.root, PathBuf::from(home).join(".oino").join("host"));
        }
    }

    #[test]
    fn windows_default_dir_has_no_legacy_fallback() {
        // The shared rule is pure and unit-tested in `unipi-proto::paths`;
        // here we only pin that a v1 legacy identity never wins.
        let tmp = tempfile::tempdir().expect("tempdir");
        let profile = tmp.path().join("profile");
        let legacy = tmp
            .path()
            .join("lad")
            .join("UniPi")
            .join(".unipi")
            .join("app-host");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("secret.key"), [0u8; 32]).unwrap();
        // Even with a legacy identity on disk, a v2 host always takes its own
        // dir under USERPROFILE (coexistence with the old unipi, spec §5.1).
        assert_eq!(
            unipi_proto::paths::host_dir_from(true, None, None, Some(profile.clone())),
            profile.join(".oino").join("host")
        );
    }

    #[test]
    fn env_var_overrides_default_dir() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let _guard = EnvGuard::set(OINO_HOST_DIR_ENV, tmp.path());
        let paths = HostPaths::resolve();
        assert_eq!(paths.root, tmp.path());
    }

    /// Spec §5.1 / risk R8: the v2 host never reads, writes or locks anything
    /// under the owner's old `~/.unipi` (a live unipi runs there on this PC).
    #[test]
    fn never_touches_the_old_unipi_tree() {
        let home = tempfile::tempdir().expect("tempdir");
        let old = home.path().join(".unipi").join("app-host");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::write(old.join("secret.key"), [7u8; 32]).unwrap();
        let before: Vec<_> = std::fs::read_dir(&old)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();

        let _env = EnvGuard::change(
            &[
                ("HOME", home.path().as_os_str().to_os_string()),
                ("USERPROFILE", home.path().as_os_str().to_os_string()),
            ],
            &[OINO_HOST_DIR_ENV],
        );
        let paths = HostPaths::resolve();
        assert_eq!(paths.root, home.path().join(".oino").join("host"));
        paths.ensure_dir().unwrap();
        write_atomic(&paths.secret_key(), &[0u8; 32]).unwrap();

        // The old dir is untouched and the new one holds our own identity.
        let after: Vec<_> = std::fs::read_dir(&old)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(before.len(), after.len(), "~/.unipi was modified");
        assert_eq!(std::fs::read(old.join("secret.key")).unwrap(), [7u8; 32]);
        assert!(paths.secret_key().exists());
        assert!(!paths.root.starts_with(home.path().join(".unipi")));
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

    /// Minimal env var guard: serializes access to the vars it touches via a
    /// global mutex so tests can run in parallel without clobbering each
    /// other's `OINO_HOST_DIR`/`HOME`. Several vars share one guard (one lock)
    /// — nesting guards would deadlock.
    pub struct EnvGuard {
        saved: Vec<(&'static str, Option<std::ffi::OsString>)>,
        _lock: std::sync::MutexGuard<'static, ()>,
    }

    static ENV_MUTEX: std::sync::Mutex<()> = std::sync::Mutex::new(());

    impl EnvGuard {
        pub fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
            EnvGuard::change(&[(key, value.as_ref().to_os_string())], &[])
        }

        pub fn unset(key: &'static str) -> Self {
            EnvGuard::change(&[], &[key])
        }

        /// Sets and/or removes several vars under one lock (the mutex is not
        /// reentrant, so a test's env changes must share a single guard).
        pub fn change(vars: &[(&'static str, std::ffi::OsString)], unset: &[&'static str]) -> Self {
            let lock = ENV_MUTEX.lock().unwrap_or_else(|e| e.into_inner());
            let mut saved = Vec::with_capacity(vars.len() + unset.len());
            for (key, value) in vars {
                saved.push((*key, std::env::var_os(key)));
                std::env::set_var(key, value);
            }
            for key in unset {
                saved.push((*key, std::env::var_os(key)));
                std::env::remove_var(key);
            }
            EnvGuard { saved, _lock: lock }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, previous) in &self.saved {
                match previous {
                    Some(v) => std::env::set_var(key, v),
                    None => std::env::remove_var(key),
                }
            }
        }
    }
}
