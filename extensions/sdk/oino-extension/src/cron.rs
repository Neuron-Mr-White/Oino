//! Cron scheduler control (WIT `cron`, unipi-port §3.24): one thin wrapper over
//! [`wit::cron::call`].
//!
//! `op` is one of `list`, `show`, `add`, `edit`, `remove`, `enable`, `disable`, `run`,
//! `stop`, `runs`, `log`, `pause`, `resume`, `status`, `settings-get`, `settings-set`,
//! `test-notify`, `parse`; `payload` holds the request's fields. Needs manifest
//! permission `cron`. The host routes to the running scheduler (`oino cron serve`, the
//! installed user service, or the desktop's embedded one).

use serde::de::DeserializeOwned;
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::cron as wc;

/// Runs one scheduler op with a JSON payload; returns the op's reply JSON.
///
/// # Errors
/// `Error::Host` when the host denies the call or the scheduler reports an error.
pub async fn call(op: &str, payload: &Value) -> Result<Value> {
    let reply = wc::call(op.to_owned(), payload.to_string()).await?;
    Ok(serde_json::from_str(&reply)?)
}

/// Runs one scheduler op and decodes the reply.
///
/// # Errors
/// As [`call`], plus a decode error when the reply does not match `T`.
pub async fn call_as<T: DeserializeOwned>(op: &str, payload: &Value) -> Result<T> {
    let reply = wc::call(op.to_owned(), payload.to_string()).await?;
    Ok(serde_json::from_str(&reply)?)
}
