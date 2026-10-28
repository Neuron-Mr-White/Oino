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

/// The host's current UTC offset in minutes (east positive).
///
/// WASI clocks are UTC and the guest cannot read the host time zone, so this is how extensions
/// derive local time: `local_hour_ms = now_ms + offset * 60_000` (unipi-core R3).
#[must_use]
pub fn local_offset_minutes() -> i32 {
    wh::get().local_offset_minutes
}

/// Hour of the day (0–23) in the host's local time zone at UTC timestamp `now_ms`.
#[must_use]
pub fn local_hour(now_ms: i64) -> i64 {
    let offset_ms = i64::from(local_offset_minutes()) * 60_000;
    (now_ms.saturating_add(offset_ms))
        .div_euclid(3_600_000)
        .rem_euclid(24)
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
