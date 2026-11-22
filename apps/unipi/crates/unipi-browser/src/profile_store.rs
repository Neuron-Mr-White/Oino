//! The profile store: where profile directories live.
//!
//! Standalone mode keeps the service's own root (`<browser root>/profiles`).
//! In attached mode (WP-B7) the embedding host owns the real browsers, so its
//! store is plugged in here ([`ProfileStore`]) — one source of truth instead
//! of two roots that can disagree (the old bug: `browser_profile create`
//! wrote an empty marker under `<browser root>/profiles` while the launcher
//! started browsers from the host's `browser-profiles`, so the created
//! profile existed but never launched and `cookieDomains` never showed).

/// One profile as the store sees it: its directory and the domains whose
/// cookies were imported into it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredProfile {
    /// Name.
    pub name: String,
    /// The browser's `--user-data-dir`.
    pub dir: std::path::PathBuf,
    /// Domains with imported cookies.
    pub cookie_domains: Vec<String>,
}

/// Reads and creates profiles. Standalone: the service's own directory.
pub trait ProfileStore: Send + Sync {
    /// Every profile, sorted (`default` first when the store has one).
    fn list(&self) -> Vec<StoredProfile>;

    /// Creates an empty profile. Errors when the name is invalid or taken.
    ///
    /// # Errors
    /// The store's own validation or I/O.
    fn create(&self, name: &str) -> Result<(), String>;

    /// Marks a marker-only profile (created by an older root) as a real
    /// profile, adopting whatever exists instead of failing on first use.
    /// Must never delete user data. `Ok(false)` when nothing was needed.
    ///
    /// # Errors
    /// Creation or I/O failures.
    fn adopt(&self, name: &str) -> Result<bool, String>;

    /// The profile's `--user-data-dir`; `None` when the store has no fixed
    /// directory for it (the caller then applies its own layout).
    fn dir_of(&self, name: &str) -> Option<std::path::PathBuf> {
        let _ = name;
        None
    }

    /// Domains whose cookies were imported into `name`.
    fn cookie_domains(&self, name: &str) -> Vec<String> {
        let _ = name;
        Vec::new()
    }
}

/// The standalone store: `<browser root>/profiles/<name>` with
/// `oino-profile.json` metadata.
#[derive(Clone, Debug)]
pub struct LocalStore {
    /// `<browser root>/profiles`.
    pub dir: std::path::PathBuf,
}

impl ProfileStore for LocalStore {
    fn list(&self) -> Vec<StoredProfile> {
        crate::profiles::list_of(&self.dir)
    }

    fn create(&self, name: &str) -> Result<(), String> {
        crate::profiles::create_in(&self.dir, name)
    }

    fn adopt(&self, _name: &str) -> Result<bool, String> {
        // The local root has no twin directory a marker could hide in.
        Ok(false)
    }

    fn dir_of(&self, name: &str) -> Option<std::path::PathBuf> {
        Some(self.dir.join(name))
    }

    fn cookie_domains(&self, name: &str) -> Vec<String> {
        crate::profiles::cookie_domains_of(&self.dir.join(name))
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    #[test]
    fn local_store_lists_creates_and_validates() {
        let dir = tempfile::tempdir().unwrap();
        let store = LocalStore {
            dir: dir.path().to_path_buf(),
        };
        assert_eq!(store.list(), Vec::new());
        store.create("work").unwrap();
        assert!(store.create("work").is_err(), "exists");
        assert!(store.create("Bad Name").is_err());
        let list = store.list();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "work");
        assert_eq!(list[0].cookie_domains, Vec::<String>::new());
        assert!(list[0].dir.is_dir());
        // Nothing to adopt in the local root; never an error.
        assert_eq!(store.adopt("work"), Ok(false));
    }
}
