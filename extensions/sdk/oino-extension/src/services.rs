//! Inter-extension service calls (permission `services.call`).

use std::time::Duration;

use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::services as ws;
use crate::error::parse;

/// Calls service `name` on `target` (or whichever extension serves it).
pub async fn call(
    target: Option<&str>,
    name: &str,
    payload: &Value,
    timeout: Option<Duration>,
) -> Result<Value> {
    let timeout = timeout.map(|t| u32::try_from(t.as_millis()).unwrap_or(u32::MAX));
    parse(
        &ws::call(
            target.map(ToOwned::to_owned),
            name.to_owned(),
            payload.to_string(),
            timeout,
        )
        .await?,
    )
}
/// Registered services `(extension, name)`.
#[must_use]
pub fn list() -> Vec<(String, String)> {
    ws::list()
}
