//! Host-provided services (WIT `host-service`, v2 ext-abi §3): one thin wrapper over
//! [`wit::host_service`].
//!
//! A service `name` ("browser", "cron", …) is registered by the embedding host;
//! under a plain `oino` TUI no service exists and `available` returns
//! `false` (extensions degrade: tools report unavailable). `call` is permission-gated
//! per name (`permissions.hostServices`); `payload` and the reply are the service's own
//! JSON contract — user-facing failures travel inside the reply, the host-error is only
//! permission (`denied`), missing service (`not-found`) and transport problems.

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::host_service as whs;

/// Whether this host provides the service `name` (cheap, never traps).
#[must_use]
pub fn available(name: &str) -> bool {
    whs::available(name)
}

/// Names of the services this host provides (sorted).
#[must_use]
pub fn list() -> Vec<String> {
    whs::list()
}

/// Runs one operation of host service `name` with a JSON payload; returns the service's
/// reply JSON.
///
/// # Errors
/// `Error::Host` when the host denies the call (`hostServices:<name>`), provides no such
/// service (`not-found`) or the transport fails.
pub async fn call(name: &str, op: &str, payload: &Value) -> Result<Value> {
    let reply = whs::call(name.to_owned(), op.to_owned(), payload.to_string(), None).await?;
    Ok(serde_json::from_str(&reply)?)
}

/// Runs one operation and decodes the reply.
///
/// # Errors
/// As [`call`], plus a decode error when the reply does not match `T`.
pub async fn call_as<T: DeserializeOwned>(name: &str, op: &str, payload: &Value) -> Result<T> {
    let reply = whs::call(name.to_owned(), op.to_owned(), payload.to_string(), None).await?;
    Ok(serde_json::from_str(&reply)?)
}
