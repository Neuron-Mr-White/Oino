//! Host background tasks (C12; permission `process.spawn`).

use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::tasks as wt;
use crate::error::parse;

/// Starts a task from a `TaskSpec` JSON → task id.
pub fn start(spec: &Value) -> Result<String> {
    Ok(wt::start(&spec.to_string())?)
}
/// Adopts a running bash tool call.
pub fn adopt_bash(tool_call_id: &str, reason: &str) -> Result<String> {
    Ok(wt::adopt_bash(tool_call_id, reason)?)
}
/// Tasks of this session.
pub fn list() -> Result<Vec<Value>> {
    parse(&wt::list())
}
/// One task.
pub fn get(id: &str) -> Result<Option<Value>> {
    wt::get(id).map(|t| parse(&t)).transpose()
}
/// Output text.
pub fn logs(id: &str, max_bytes: u32, tail: bool) -> Result<String> {
    Ok(wt::logs(id, max_bytes, tail)?)
}
/// Kills a task.
pub fn kill(id: &str) -> Result<()> {
    Ok(wt::kill(id)?)
}
pub use wt::clear_finished;
