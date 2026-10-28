//! Session control (pi `ctx.*`, `setModel`, tools, commands, command-context operations).

use serde_json::Value;

use crate::Result;
use crate::bindings::oino::extension::control as wc;
use crate::error::parse;

pub use wc::{
    abort, active_tools, has_pending_messages, is_idle, is_project_trusted, running_bash, shutdown,
    system_prompt, thinking_level, tool_progress,
};

/// Current model (`Model` JSON).
#[must_use]
pub fn model() -> Option<Value> {
    wc::model().and_then(|t| serde_json::from_str(&t).ok())
}
/// pi `setModel`; `false` when the model has no credentials (permission `sessionControl`).
pub fn set_model(provider: &str, id: &str) -> Result<bool> {
    Ok(wc::set_model(provider, id)?)
}

/// Persists the model a fresh session starts with (core settings `defaultProvider`,
/// `defaultModel`, `modelThinkingLevels`); the running session is unchanged. `false` when the
/// model is unknown or has no credentials.
///
/// # Errors
///
/// Host errors (unknown thinking level, settings write failure).
pub fn set_default_model(provider: &str, id: &str, thinking: Option<&str>) -> Result<bool> {
    Ok(wc::set_default_model(provider, id, thinking)?)
}
/// pi `setThinkingLevel`; returns the clamped level.
pub fn set_thinking_level(level: &str) -> Result<String> {
    Ok(wc::set_thinking_level(level)?)
}
/// Available models.
pub fn models() -> Result<Vec<Value>> {
    parse(&wc::models_list())
}
/// pi `getContextUsage`.
pub fn context_usage() -> Result<Value> {
    parse(&wc::context_usage())
}
/// Session stats.
pub fn session_stats() -> Result<Value> {
    parse(&wc::session_stats())
}
/// Run state.
pub fn run_state() -> Result<Value> {
    parse(&wc::run_state())
}
/// pi `getSettings`.
pub fn settings() -> Result<Value> {
    parse(&wc::get_settings())
}
/// pi `setActiveTools`.
pub fn set_active_tools(names: &[String]) -> Result<()> {
    Ok(wc::set_active_tools(names)?)
}
/// pi `getAllTools`.
pub fn all_tools() -> Result<Vec<Value>> {
    parse(&wc::all_tools())
}
/// pi `getCommands`.
pub fn commands() -> Result<Vec<Value>> {
    parse(&wc::commands_list())
}
/// pi `ctx.compact`.
pub async fn compact(custom_instructions: Option<&str>) -> Result<Value> {
    parse(&wc::compact(custom_instructions.map(ToOwned::to_owned)).await?)
}
/// Runs any command (cross-extension) with a command context.
pub async fn invoke_command(name: &str, args: &str) -> Result<Option<Value>> {
    wc::command_invoke(name.to_owned(), args.to_owned())
        .await?
        .map(|t| parse(&t))
        .transpose()
}
/// pi `ctx.executeTool` (tool-call context only); returns `AgentToolCallOutcome`.
pub async fn execute_tool(name: &str, args: &Value) -> Result<Value> {
    parse(&wc::execute_tool(name.to_owned(), args.to_string(), None).await?)
}
/// Detaches a running built-in bash call into a background task.
pub fn bash_detach(tool_call_id: &str, reason: &str) -> Result<String> {
    Ok(wc::bash_detach(tool_call_id, reason)?)
}
/// Command context: wait until the agent is idle.
pub async fn wait_for_idle() -> Result<()> {
    Ok(wc::wait_for_idle().await?)
}
/// Command context: new session; `true` when cancelled.
pub async fn new_session(parent: Option<&str>) -> Result<bool> {
    Ok(wc::new_session(parent.map(ToOwned::to_owned)).await?)
}
/// Command context: fork at/before an entry.
pub async fn fork(entry_id: &str, at: bool) -> Result<Value> {
    parse(&wc::fork(entry_id.to_owned(), at).await?)
}
/// Command context: switch session; `true` when cancelled.
pub async fn switch_session(path: &str) -> Result<bool> {
    Ok(wc::switch_session(path.to_owned()).await?)
}
/// Command context: navigate the tree.
pub async fn navigate_tree(target_id: &str, options: &Value) -> Result<Value> {
    parse(&wc::navigate_tree(target_id.to_owned(), options.to_string()).await?)
}
/// Command context: reload resources and extensions.
pub async fn reload() -> Result<()> {
    Ok(wc::reload().await?)
}
