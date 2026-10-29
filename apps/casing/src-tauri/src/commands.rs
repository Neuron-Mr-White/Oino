//! Tauri commands exposed to the SolidJS UI.

use serde::Serialize;
use unipi_proto::{AppErrorDto, ProtoError, PROTO_VERSION};

/// Information about the running app core, returned to the UI on
/// startup so it can show version/platform and check protocol
/// compatibility.
#[derive(Debug, Clone, Serialize)]
pub struct AppInfo {
    /// The app core's own version (`CARGO_PKG_VERSION`).
    pub version: String,
    /// The OS platform string from `tauri-plugin-os` (for example
    /// `"android"`, `"linux"`, `"macos"`, `"windows"`).
    pub platform: String,
    /// The wire protocol version this build implements. See
    /// `unipi-app/DESIGN.md` §5.4.
    pub proto: String,
}

/// Returns the app core's version, platform and protocol version.
///
/// Errors map onto the shared `AppErrorDto` envelope (`unipi-app/DESIGN.md`
/// §5.5) so the UI never has to special-case this command.
#[tauri::command]
pub fn app_info() -> Result<AppInfo, AppErrorDto> {
    let platform = tauri_plugin_os::platform();
    if platform.is_empty() {
        return Err(
            ProtoError::Internal("empty platform string from OS plugin".to_string()).to_app_error(),
        );
    }

    Ok(AppInfo {
        version: env!("CARGO_PKG_VERSION").to_string(),
        platform: platform.to_string(),
        proto: PROTO_VERSION.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_reports_current_proto_version() {
        let info = app_info().expect("app_info should succeed in tests");
        assert_eq!(info.proto, PROTO_VERSION);
        assert!(!info.version.is_empty());
        assert!(!info.platform.is_empty());
    }
}
