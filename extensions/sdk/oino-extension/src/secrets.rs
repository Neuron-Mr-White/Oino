//! Secrets in the extension's own namespace (permission `secrets`).

use crate::Result;
use crate::bindings::oino::extension::secrets as ws;

/// Reads a secret.
pub fn get(name: &str) -> Result<Option<String>> {
    Ok(ws::get(name)?)
}
/// Stores a secret.
pub fn set(name: &str, value: &str) -> Result<()> {
    Ok(ws::set(name, value)?)
}
/// Deletes a secret.
pub fn delete(name: &str) -> Result<()> {
    Ok(ws::delete(name)?)
}
