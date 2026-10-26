//! Scoped key-value state (`global`, `workspace`, `session`).

use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Result;
use crate::bindings::oino::extension::kv as wk;
pub use crate::bindings::oino::extension::kv::Scope;

/// Raw bytes.
#[must_use]
pub fn get(scope: Scope, key: &str) -> Option<Vec<u8>> {
    wk::get(scope, key)
}
/// Stores bytes.
pub fn set(scope: Scope, key: &str, value: &[u8]) -> Result<()> {
    Ok(wk::set(scope, key, value)?)
}
/// Deletes a key.
pub fn delete(scope: Scope, key: &str) {
    wk::delete(scope, key);
}
/// Keys with `prefix`.
#[must_use]
pub fn keys(scope: Scope, prefix: &str) -> Vec<String> {
    wk::keys(scope, prefix)
}
/// Typed JSON read.
pub fn get_json<T: DeserializeOwned>(scope: Scope, key: &str) -> Result<Option<T>> {
    get(scope, key)
        .map(|b| serde_json::from_slice(&b).map_err(Into::into))
        .transpose()
}
/// Typed JSON write.
pub fn set_json(scope: Scope, key: &str, value: &impl Serialize) -> Result<()> {
    set(scope, key, &serde_json::to_vec(value)?)
}
