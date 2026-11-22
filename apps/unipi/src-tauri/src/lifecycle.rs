//! v2 app lifetime (host.md §5.4–§5.6, replacing unipi-app's "close always hides"
//! from UNI-237): ask-on-close remembered, the real Quit confirm, "Start at login"
//! and starting hidden in the tray.
//!
//! - **Closing the window** follows the remembered `closeAction` (app setting in
//!   this app's own config dir): `ask` (the default) shows a dialog once —
//!   **Minimize to tray** (default) or **Quit**, with "Remember my choice";
//!   `tray` hides the window silently; `quit` quits. Changeable in Settings →
//!   General (§5.4).
//! - **Quitting for real** (tray → Quit, the app menu, Ctrl+Q, or Quit in that
//!   dialog) asks first when the host is app-managed **and** at least one phone
//!   is paired: "phones can't reach this PC until you open UniPi again" (§5.5).
//!   Quit stops an app-managed host (it exits with its parent).
//! - **Start at login** (`tauri-plugin-autostart`, per-user login item, no admin
//!   or service) launches the app with `--hidden`, which starts it in the tray
//!   without a window (§5.6); the app then adopts/spawns the host.
//! - The app settings live in the app config dir `dev.unipi.oino` (§5.9/§9.9),
//!   never the host's data dir.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use unipi_proto::{AppErrorCode, AppErrorDto};

/// Where this app keeps its own settings (not the host's data dir).
pub fn config_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        match std::env::var_os("XDG_CONFIG_HOME").filter(|v| !v.is_empty()) {
            Some(dir) => PathBuf::from(dir).join("dev.unipi.oino"),
            None => home().join(".config").join("dev.unipi.oino"),
        }
    }
    #[cfg(target_os = "macos")]
    {
        home()
            .join("Library")
            .join("Application Support")
            .join("dev.unipi.oino")
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        home()
            .join("AppData")
            .join("Roaming")
            .join("dev.unipi.oino")
    }
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn read_setting(file: &str, key: &str) -> Option<serde_json::Value> {
    let raw = std::fs::read_to_string(config_dir().join(file)).ok()?;
    serde_json::from_str::<serde_json::Value>(&raw)
        .ok()?
        .get(key)
        .cloned()
}

fn write_setting(file: &str, key: &str, value: serde_json::Value) -> std::io::Result<()> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(
        dir.join(file),
        serde_json::json!({ key: value }).to_string(),
    )
}

// ------------------------------------------------------------ close action

/// What a window close does (`closeAction`, host.md §5.4).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CloseAction {
    /// Ask once: Minimize to tray (default) or Quit; "Remember my choice".
    #[default]
    Ask,
    /// Hide to the tray; the app and an app-managed host keep running.
    Tray,
    /// Quit the app (and with it an app-managed host).
    Quit,
}

/// The app setting's file and key (§9.9 names the key `closeAction`).
pub const CLOSE_ACTION_FILE: &str = "close-action.json";
pub const CLOSE_ACTION_KEY: &str = "closeAction";

/// The remembered answer (default `ask`: the app has never been told).
pub fn close_action() -> CloseAction {
    read_setting(CLOSE_ACTION_FILE, CLOSE_ACTION_KEY)
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

/// Remembers the answer (Settings → General shows and changes it).
pub fn set_close_action(action: CloseAction) -> std::io::Result<()> {
    let value = serde_json::to_value(action).map_err(std::io::Error::other)?;
    write_setting(CLOSE_ACTION_FILE, CLOSE_ACTION_KEY, value)
}

/// What `CloseRequested` does for the remembered setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// Show the "Minimize to tray / Quit" dialog (the default, once).
    Ask,
    /// Hide the window and keep everything running.
    Hide,
    /// Quit the app (the app-managed host exits with it).
    Quit,
}

/// §5.4's mapping: the setting decides; `ask` means a dialog, not a default act.
pub fn close_decision(action: CloseAction) -> CloseDecision {
    match action {
        CloseAction::Ask => CloseDecision::Ask,
        CloseAction::Tray => CloseDecision::Hide,
        CloseAction::Quit => CloseDecision::Quit,
    }
}

/// The event the UI shows the close dialog for: `{message, remembered}`.
pub const CLOSE_ASK_EVENT: &str = "unipi-close-ask";

/// The close dialog's copy ("Minimize to tray" is the default button).
pub const CLOSE_ASK_MESSAGE: &str =
    "Close Unipi, or keep it running in the tray so phones and cron keep working?";

// ---------------------------------------------------------------- autostart

/// The "Start at login" flag (default off — the app has never been asked).
pub const AUTOSTART_FILE: &str = "autostart.json";
pub const AUTOSTART_KEY: &str = "autostart";

pub fn autostart_enabled() -> bool {
    read_setting(AUTOSTART_FILE, AUTOSTART_KEY)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

pub fn set_autostart(enabled: bool) -> std::io::Result<()> {
    write_setting(
        AUTOSTART_FILE,
        AUTOSTART_KEY,
        serde_json::Value::Bool(enabled),
    )
}

/// `--hidden` (written into the login item by `tauri-plugin-autostart`): start
/// in the tray with no window; the app then adopts/spawns the host (§5.6).
pub const HIDDEN_ARG: &str = "--hidden";

/// Does this launch start hidden? (`--hidden` anywhere in argv.)
pub fn start_hidden(args: impl IntoIterator<Item = String>) -> bool {
    args.into_iter().any(|arg| arg == HIDDEN_ARG)
}

// ------------------------------------------------------------- quit confirm

/// Quitting for real: ask only when the host is **our** child and phones are
/// paired (an adopted host keeps running without the app, so there is nothing
/// to warn about).
pub fn quit_needs_confirm(host_run_by_app: bool, paired: usize) -> bool {
    host_run_by_app && paired > 0
}

/// The confirm's copy (§5.5 keeps unipi-app's warning).
pub fn quit_message(paired: usize) -> String {
    match paired {
        1 => "One phone is paired with this PC. Quitting Unipi stops the host it runs, so that phone can't reach this PC until you open Unipi again.".to_string(),
        n => format!("{n} phones are paired with this PC. Quitting Unipi stops the host it runs, so they can't reach this PC until you open Unipi again."),
    }
}

// ---------------------------------------------------------------- commands

/// `close_action_state`: `ask` | `tray` | `quit` (Settings → General).
#[tauri::command]
pub fn close_action_state() -> CloseAction {
    close_action()
}

/// `close_action_set {action}`: remember the answer and apply it to this app.
#[tauri::command]
pub fn close_action_set(action: CloseAction) -> Result<CloseAction, AppErrorDto> {
    set_close_action(action).map_err(|e| {
        AppErrorDto::new(
            AppErrorCode::E_INTERNAL,
            format!("can't save the setting: {e}"),
            false,
        )
    })?;
    Ok(action)
}

/// `close_answer {action, remember}`: the close dialog's answer. `tray` hides to
/// the tray; `quit` goes through the real Quit (with the phone warning when it
/// applies); `remember` stores the answer as `closeAction`.
#[tauri::command]
pub fn close_answer(
    app: tauri::AppHandle,
    action: CloseAction,
    remember: bool,
) -> Result<(), AppErrorDto> {
    if action == CloseAction::Ask {
        return Err(AppErrorDto::new(
            AppErrorCode::E_SCHEMA,
            "the close dialog answers `tray` or `quit`",
            false,
        ));
    }
    if remember {
        set_close_action(action).map_err(|e| {
            AppErrorDto::new(
                AppErrorCode::E_INTERNAL,
                format!("can't save the setting: {e}"),
                false,
            )
        })?;
    }
    #[cfg(desktop)]
    {
        match action {
            CloseAction::Tray => crate::desktop::hide_main(&app),
            // Quit stops an app-managed host; the warning (phones paired) is the
            // same one the tray's Quit shows.
            _ => request_quit(&app),
        }
        Ok(())
    }
    #[cfg(not(desktop))]
    {
        let _ = (app, action, remember);
        Err(AppErrorDto::new(
            AppErrorCode::E_UNSUPPORTED,
            "The close dialog is desktop only",
            false,
        ))
    }
}

/// `autostart_state` for the UI.
#[tauri::command]
pub fn autostart_state() -> bool {
    autostart_enabled()
}

/// `autostart_set {enabled}`: remember it AND apply it (the plugin writes the
/// Linux XDG autostart entry / the macOS login item / the Windows Run key; it
/// passes `--hidden`, so a login start lands in the tray).
#[tauri::command]
pub fn autostart_set(app: tauri::AppHandle, enabled: bool) -> Result<bool, AppErrorDto> {
    set_autostart(enabled).map_err(|e| {
        AppErrorDto::new(
            AppErrorCode::E_INTERNAL,
            format!("can't save the setting: {e}"),
            false,
        )
    })?;
    #[cfg(desktop)]
    {
        use tauri_plugin_autostart::ManagerExt;
        let manager = app.autolaunch();
        let result = if enabled {
            manager.enable()
        } else {
            manager.disable()
        };
        result.map_err(|e| {
            AppErrorDto::new(
                AppErrorCode::E_INTERNAL,
                format!("can't change the login item: {e}"),
                false,
            )
        })?;
    }
    Ok(enabled)
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
    // The sheet lives on the main window: it must be visible to be answered.
    #[cfg(desktop)]
    {
        use tauri::Manager;
        if let Some(window) = app.get_webview_window("main") {
            let _ = window.show();
            let _ = window.unminimize();
            let _ = window.set_focus();
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closing_the_window_follows_the_remembered_answer() {
        // The default is `ask`: one dialog, never a silent quit or hide.
        assert_eq!(CloseAction::default(), CloseAction::Ask);
        assert_eq!(close_decision(CloseAction::Ask), CloseDecision::Ask);
        assert_eq!(close_decision(CloseAction::Tray), CloseDecision::Hide);
        assert_eq!(close_decision(CloseAction::Quit), CloseDecision::Quit);
        assert!(CLOSE_ASK_MESSAGE.contains("tray"));
        // The setting's wire names are the spec's (`ask | tray | quit`).
        for (action, name) in [
            (CloseAction::Ask, "ask"),
            (CloseAction::Tray, "tray"),
            (CloseAction::Quit, "quit"),
        ] {
            let json = serde_json::to_string(&action).unwrap();
            assert_eq!(json, format!("\"{name}\""));
            assert_eq!(serde_json::from_str::<CloseAction>(&json).unwrap(), action);
        }
    }

    #[test]
    fn start_at_login_starts_hidden_in_the_tray() {
        assert!(!start_hidden(["unipi".to_string()]));
        assert!(start_hidden(["unipi".to_string(), HIDDEN_ARG.to_string()]));
        // The autostart plugin's own args include it.
        assert!(start_hidden(vec![
            "/usr/bin/unipi".to_string(),
            HIDDEN_ARG.to_string()
        ]));
    }

    #[test]
    fn quit_asks_only_for_an_app_managed_host_with_phones() {
        assert!(quit_needs_confirm(true, 1));
        assert!(quit_needs_confirm(true, 4));
        // An adopted host keeps running: nothing to warn about.
        assert!(!quit_needs_confirm(false, 4));
        assert!(!quit_needs_confirm(true, 0));
        assert!(quit_message(1).contains("until you open Unipi again"));
        assert!(quit_message(3).contains("3 phones"));
    }

    #[test]
    fn app_settings_round_trip_in_the_v2_config_dir() {
        // The app's own dir, the v2 identifier, never the host's data dir.
        let dir = config_dir();
        assert!(dir.ends_with("dev.unipi.oino"), "{dir:?}");
        // The close action round-trips (default ask when absent).
        let before = close_action();
        set_close_action(CloseAction::Quit).unwrap();
        assert_eq!(close_action(), CloseAction::Quit);
        set_close_action(CloseAction::Tray).unwrap();
        assert_eq!(close_action(), CloseAction::Tray);
        set_close_action(before).unwrap();
        // Autostart is a boolean too.
        let before = autostart_enabled();
        set_autostart(!before).unwrap();
        assert_eq!(autostart_enabled(), !before);
        set_autostart(before).unwrap();
    }
}
