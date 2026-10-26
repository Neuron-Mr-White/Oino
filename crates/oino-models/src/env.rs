//! Environment lookup used by credential and config-value resolution.
//!
//! pi reads `process.env` directly. Oino injects the environment so tests stay deterministic
//! (edition 2024 makes `std::env::set_var` unsafe) and hosts can overlay variables.

use std::collections::HashMap;
use std::fmt;

/// Source of environment variables.
///
/// Implementations return `None` for unset variables. Callers treat empty values as unset,
/// matching pi's `env[name] || process.env[name] || undefined`.
pub trait EnvSource: Send + Sync {
    /// Value of `name`, if set.
    fn var(&self, name: &str) -> Option<String>;
}

/// The real process environment (`std::env::var`).
#[derive(Clone, Copy, Debug, Default)]
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn var(&self, name: &str) -> Option<String> {
        std::env::var(name).ok()
    }
}

/// A fixed environment map, for tests and sandboxed hosts.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct MapEnv(HashMap<String, String>);

impl MapEnv {
    /// Empty environment.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds or replaces `name`.
    #[must_use]
    pub fn with(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.0.insert(name.into(), value.into());
        self
    }
}

impl fmt::Debug for MapEnv {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Values may be secrets: only print names.
        let mut names: Vec<&String> = self.0.keys().collect();
        names.sort();
        f.debug_tuple("MapEnv").field(&names).finish()
    }
}

impl<K: Into<String>, V: Into<String>> FromIterator<(K, V)> for MapEnv {
    fn from_iter<I: IntoIterator<Item = (K, V)>>(iter: I) -> Self {
        Self(
            iter.into_iter()
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        )
    }
}

impl EnvSource for MapEnv {
    fn var(&self, name: &str) -> Option<String> {
        self.0.get(name).cloned()
    }
}

/// Looks up `name` in `explicit` first, then in `env`; empty values count as unset
/// (pi `resolveEnvConfigValue`).
pub(crate) fn lookup(
    name: &str,
    explicit: Option<&indexmap::IndexMap<String, String>>,
    env: &dyn EnvSource,
) -> Option<String> {
    explicit
        .and_then(|map| map.get(name))
        .filter(|value| !value.is_empty())
        .cloned()
        .or_else(|| env.var(name).filter(|value| !value.is_empty()))
}
