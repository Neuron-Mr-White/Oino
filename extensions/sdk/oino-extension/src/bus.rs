//! Event bus (pi `events`): deliveries arrive as `oino_bus` events.

use serde::Serialize;
use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::bus as wb;

/// Publishes `data` on `topic` (`sticky` keeps the last value for `get`).
pub fn publish(topic: &str, data: &impl Serialize, sticky: bool) -> Result<()> {
    wb::publish(topic, &serde_json::to_string(data)?, sticky);
    Ok(())
}
/// Subscribes to an exact topic or `prefix:*`.
pub fn subscribe(pattern: &str) {
    wb::subscribe(pattern);
}
/// Unsubscribes.
pub fn unsubscribe(pattern: &str) {
    wb::unsubscribe(pattern);
}
/// Last sticky value.
#[must_use]
pub fn get(topic: &str) -> Option<Value> {
    wb::get(topic).and_then(|t| serde_json::from_str(&t).ok())
}
