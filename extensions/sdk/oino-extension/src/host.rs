//! Host information and logging.

use crate::Result;
pub use crate::bindings::oino::extension::host_info::{Info, WorkspaceInfo};
pub use crate::bindings::oino::extension::log::LogLevel;
use crate::bindings::oino::extension::{host_info as wh, log as wl};

/// Platform, version, mode, frontends.
#[must_use]
pub fn info() -> Info {
    wh::get()
}
/// Workspace identity (D10).
#[must_use]
pub fn workspace() -> WorkspaceInfo {
    wh::workspace()
}
/// Env var (permission `env`).
pub fn env(name: &str) -> Result<Option<String>> {
    Ok(wh::env_get(name)?)
}
/// A frontend has focus.
#[must_use]
pub fn frontend_focused() -> bool {
    wh::frontend_focused()
}
/// Logs to the extension log.
pub fn log(level: LogLevel, message: &str) {
    wl::log(level, message);
}
/// Info log.
pub fn info_log(message: &str) {
    log(LogLevel::Info, message);
}
