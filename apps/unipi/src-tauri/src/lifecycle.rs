//! UNI-237 (plan A6) app lifecycle: close-to-tray, the real Quit confirm and
//! "Start at login" (user decision 2: closing the window ALWAYS keeps the app
//! — and an app-managed host — running in the tray).
//!
//! - **Closing the window** hides it to the tray. Nothing is asked: the host
//!   keeps running either way, and phones keep working.
//! - **Quitting for real** (tray → Quit UniPi, the app menu, Ctrl+Q) asks
//!   first when the host is app-managed **and** at least one phone is paired:
//!   "Phones won't reach this PC until you open UniPi again" — Quit / Keep in
//!   tray.
//! - The tray's status line says "Phones can connect" or "Phones can't reach
//!   this PC".
//! - "Start at login" (`tauri-plugin-autostart`) starts the app hidden in the
//!   tray; the choice is stored under the app's own config dir and survives
//!   restarts.

use std::path::PathBuf;

use unipi_proto::{AppErrorCode, AppErrorDto};

/// Where this app keeps its own settings (not the host's data dir).
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir).join("dev.unipi.app"),
            None => home().join(".config").join("dev.unipi.app"),
        }
    }
    #[cfg(target_os = "macos")]
    {
        home().join("Library").join("Application Support").join("dev.unipi.app")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        home().join("AppData").join("Roaming").join("dev.unipi.app")
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

/// The "Start at login" flag (default off — the app has never asked).
pub const AUTOSTART_FILE: &str = "autostart.json";

pub fn autostart_enabled() -> bool {
    std::fs::read_to_string(config_dir().join(AUTOSTART_FILE))
        .ok()
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| v.get("enabled").and_then(|e| e.as_bool()))
        .unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join(AUTOSTART_FILE), serde_json::json!({ "enabled": enabled }).to_string())
}

// ------------------------------------------------------------ pure decisions

/// The tray's line and the tooltip (asked first by the pure helpers, so the
/// UI, the tray menus and the tests share one wording).
pub fn phones_line(host_running: bool, paired: usize) -> String {
    match (host_running, paired) {
        (true, 0) => "No phone paired yet".to_string(),
        (true, 1) => "Phones can connect".to_string(),
        (true, n) => format!("Phones can connect ({n})"),
        (false, 0) => "Phones can't reach this PC".to_string(),
        (false, _) => "Phones can't reach this PC — UniPi is closed".to_string(),
    }
}

/// Quitting for real: ask only when the host is **our** child and phones are
/// paired (a service host keeps running without the app, so there is nothing
/// to warn about).
pub fn quit_needs_confirm(host_run_by_app: bool, paired: usize) -> bool {
    host_run_by_app && paired > 0
}

/// The confirm's copy.
pub fn quit_message(paired: usize) -> String {
    match paired {
        1 => "One phone is paired with this PC. Quitting UniPi stops the host it runs, so that phone can't reach this PC until you open UniPi again.".to_string(),
        n => format!("{n} phones are paired with this PC. Quitting UniPi stops the host it runs, so they can't reach this PC until you open UniPi again."),
    }
}

/// The tray tooltip line that follows the host.
pub fn tray_tooltip(base: &str, host_running: bool, paired: usize) -> String {
    format!("{base} — {}", phones_line(host_running, paired))
}

/// What the app does on `CloseRequested` for the main window. There is only
/// one answer (decision 2), spelled out so the window event and the tests
/// share it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseAction {
    /// Hide to the tray; the app and an app-managed host keep running.
    ToTray,
}

/// `CloseRequested` → hide to the tray, always. Nothing to ask.
pub fn close_action(_host_running: bool) -> CloseAction {
    CloseAction::ToTray
}

// ---------------------------------------------------------------- autostart

/// `autostart_enabled` / `autostart_set` for the UI.
#[tauri::command]
pub fn autostart_state() -> bool {
    autostart_enabled()
}

/// `autostart_set {enabled}`: remember it AND apply it (the plugin writes the
/// Linux autostart entry / the macOS login item / the Windows Run key).
#[tauri::command]
pub fn autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<bool, AppErrorDto> {
    set_autostart(enabled).map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, format!("can't save the setting: {e}"), false))?;
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        let result = if enabled { manager.enable() } else { manager.disable() };
        result.map_err(|e| AppErrorDto::new(AppErrorCode::E_INTERNAL, format!("can't change the login item: {e}"), false))?;
    }
    Ok(enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tray_line_says_whether_phones_can_connect() {
        assert_eq!(phones_line(true, 0), "No phone paired yet");
        assert_eq!(phones_line(true, 1), "Phones can connect");
        assert_eq!(phones_line(true, 3), "Phones can connect (3)");
        assert_eq!(phones_line(false, 0), "Phones can't reach this PC");
        assert!(phones_line(false, 2).contains("UniPi is closed"));
        assert_eq!(tray_tooltip("All quiet", true, 1), "All quiet — Phones can connect");
    }

    #[test]
    fn closing_the_window_always_keeps_the_app() {
        // Decision 2: never a question, host or not.
        assert_eq!(close_action(true), CloseAction::ToTray);
        assert_eq!(close_action(true), CloseAction::ToTray);
        assert_eq!(close_action(true), CloseAction::ToTray);
    }

    #[test]
    fn quit_asks_only_for_an_app_managed_host_with_phones() {
        assert!(quit_needs_confirm(true, 1));
        assert!(quit_needs_confirm(true, 4));
        // A service host keeps running: nothing to warn about.
        assert!(!quit_needs_confirm(false, 4));
        assert!(!quit_needs_confirm(true, 0));
        assert!(quit_message(1).contains("until you open UniPi again"));
        assert!(quit_message(3).contains("3 phones"));
    }

    #[test]
    fn autostart_round_trips_in_a_config_dir() {
        // The flag lives in the app's own config dir, never the host's.
        let dir = config_dir();
        assert!(dir.file_name().map(|n| n.to_string_lossy().contains("unipi")).unwrap_or(false), "{dir:?}");
        // The default is off; writing and reading back the file works.
        assert!(!autostart_enabled() || std::fs::remove_file(dir.join(AUTOSTART_FILE)).is_ok());
        set_autostart(true).unwrap();
        assert!(autostart_enabled());
        set_autostart(false).unwrap();
        assert!(!autostart_enabled());
    }
}

// --------------------------------------------------------------- quitting

/// The tray / menu / Ctrl+Q "Quit": ask first when it matters (an app-managed
/// host and paired phones), then exit. The UI answers with `quit_now`.
pub fn request_quit<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let run_by_app = super::host_supervisor::app_managed_blocking();
    let paired = super::local_host::paired_devices();
    if !quit_needs_confirm(run_by_app, paired) {
        app.exit(0);
        return;
    }
    use tauri::Emitter;
    let _ = app.emit_to(
        "main",
        crate::desktop::QUIT_CONFIRM_EVENT,
        serde_json::json!({ "message": quit_message(paired), "paired": paired }),
    );
}

/// `quit_now`: the UI's "Quit" from that sheet.
#[tauri::command]
pub fn quit_now(app: tauri::AppHandle) {
    app.exit(0);
}
