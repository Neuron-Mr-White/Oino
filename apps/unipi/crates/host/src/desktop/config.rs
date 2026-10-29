//! The `desktop` block of `<host dir>/config.json`, the per-request gate,
//! display detection and the session log.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::json;
use unipi_proto::{AppErrorCode, AppErrorDto, DisplayKind};

/// Shown when the feature is off.
pub const ENABLE_HINT: &str = "On the PC run `unipi-host desktop enable` (or `--view-only`); no restart needed.";

/// Which capture backend to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum BackendPref {
    /// GNOME Wayland → mutter, other Wayland → portal, X11 → x11 (macOS /
    /// Windows → their native capture).
    #[default]
    Auto,
    /// GNOME's own `org.gnome.Mutter.{ScreenCast,RemoteDesktop}` (no dialog).
    Mutter,
    Portal,
    X11,
    /// Synthetic frames — check the phone side without sharing the screen.
    TestCard,
}

impl BackendPref {
    pub fn parse(value: &str) -> Option<BackendPref> {
        match value {
            "auto" => Some(BackendPref::Auto),
            "mutter" | "gnome" => Some(BackendPref::Mutter),
            "portal" => Some(BackendPref::Portal),
            "x11" => Some(BackendPref::X11),
            "test-card" | "test_card" => Some(BackendPref::TestCard),
            _ => None,
        }
    }
    pub fn as_str(self) -> &'static str {
        match self {
            BackendPref::Auto => "auto",
            BackendPref::Mutter => "mutter",
            BackendPref::Portal => "portal",
            BackendPref::X11 => "x11",
            BackendPref::TestCard => "test-card",
        }
    }
}

/// `config.json` → `desktop`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DesktopConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Ignore all input from phones (watch only).
    #[serde(default)]
    pub view_only: bool,
    /// Paired device ids / id prefixes (≥ 8 chars) / names; empty = all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
    #[serde(default)]
    pub backend: BackendPref,
}

#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    desktop: DesktopConfig,
}

impl DesktopConfig {
    /// Reads `config.json`'s `desktop` block; missing file/block = off.
    pub fn load(path: &Path) -> anyhow::Result<DesktopConfig> {
        Ok(crate::data_dir::read_json::<ConfigFile>(path)?.unwrap_or_default().desktop)
    }

    /// Writes the block back, keeping every other key of `config.json`.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut value: serde_json::Value = crate::data_dir::read_json(path)?.unwrap_or_else(|| json!({}));
        if !value.is_object() {
            value = json!({});
        }
        value["desktop"] = serde_json::to_value(self)?;
        crate::data_dir::write_json(path, &value)
    }

    pub fn device_allowed(&self, device_id: &str, device_name: Option<&str>) -> bool {
        self.devices.is_empty()
            || device_id == crate::protocol::LOCAL_DEVICE_ID
            || self.devices.iter().any(|entry| {
                entry == device_id
                    || (entry.len() >= 8 && device_id.starts_with(entry.as_str()))
                    || Some(entry.as_str()) == device_name
            })
    }

    /// The gate every `desktop_open` goes through.
    pub fn check(&self, device_id: &str, device_name: Option<&str>) -> Result<(), AppErrorDto> {
        if !self.enabled {
            return Err(disabled("Desktop streaming is off on this PC"));
        }
        if !self.device_allowed(device_id, device_name) {
            return Err(disabled("This device is not allowed to stream this PC's desktop"));
        }
        Ok(())
    }
}

pub fn disabled(message: &str) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_DESKTOP_DISABLED, message, false).with_hint(ENABLE_HINT)
}

/// `E_DESKTOP_UNAVAILABLE` with `detail.reason`.
pub fn unavailable(reason: &str, message: impl Into<String>) -> AppErrorDto {
    let retryable = matches!(reason, "capture_failed" | "busy");
    let mut error = AppErrorDto::new(AppErrorCode::E_DESKTOP_UNAVAILABLE, message, retryable)
        .with_detail(json!({ "reason": reason }));
    if let Some(hint) = hint_for(reason) {
        error = error.with_hint(hint);
    }
    error
}

pub fn hint_for(reason: &str) -> Option<&'static str> {
    Some(match reason {
        "not_compiled" => "This unipi-host was built without desktop streaming. Install the release build (curl -fsSL https://unipi.nrn.one/install.sh | sh) or build with the default features.",
        "no_display" => "Log in to a desktop session on the PC (unipi-host must run inside it, e.g. as a systemd user service started by your login).",
        "denied" => "Someone pressed Cancel in the PC's share dialog. Connect again and choose Share on the PC.",
        "no_gstreamer" => "Install the PipeWire GStreamer plugin on the PC: sudo apt install gstreamer1.0-pipewire gstreamer1.0-plugins-base",
        "no_portal" => "The desktop portal is not available. On GNOME/KDE install xdg-desktop-portal-gnome / -kde.",
        _ => return None,
    })
}

/// What graphical session the host process can reach.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Display {
    pub kind: DisplayKind,
    /// `$DISPLAY` for the X11 backend (also set on Wayland when Xwayland runs).
    pub x11: Option<String>,
    /// The session is GNOME (Mutter's D-Bus screen-cast API is there).
    pub gnome: bool,
}

/// Pure detection from an env lookup + socket probe (tests pass fakes).
pub fn detect_display(env: impl Fn(&str) -> Option<String>, exists: impl Fn(&Path) -> bool) -> Display {
    let nonempty = |key: &str| env(key).filter(|v| !v.trim().is_empty());
    let runtime = nonempty("XDG_RUNTIME_DIR");
    let session = nonempty("XDG_SESSION_TYPE");
    let mut wayland = nonempty("WAYLAND_DISPLAY").is_some() || session.as_deref() == Some("wayland");
    if !wayland && session.as_deref() != Some("x11") {
        // A systemd user service may lack the session env: probe the socket.
        if let Some(dir) = &runtime {
            wayland = exists(&PathBuf::from(dir).join("wayland-0"));
        }
    }
    let mut x11 = nonempty("DISPLAY");
    if x11.is_none() && exists(Path::new("/tmp/.X11-unix/X0")) {
        x11 = Some(":0".to_string());
    }
    let kind = if wayland {
        DisplayKind::Wayland
    } else if x11.is_some() {
        DisplayKind::X11
    } else {
        DisplayKind::None
    };
    // "ubuntu:GNOME", "GNOME", "GNOME-Classic:GNOME", "Pop:GNOME"…
    let gnome = [nonempty("XDG_CURRENT_DESKTOP"), nonempty("XDG_SESSION_DESKTOP"), nonempty("DESKTOP_SESSION")]
        .iter()
        .flatten()
        .any(|v| v.split(':').any(|part| part.eq_ignore_ascii_case("gnome") || part.to_ascii_lowercase().starts_with("gnome-")));
    Display { kind, x11, gnome }
}

pub fn detect_here() -> Display {
    // The console session of a logged-in user is always graphical.
    #[cfg(target_os = "macos")]
    {
        Display { kind: DisplayKind::Macos, x11: None, gnome: false }
    }
    #[cfg(windows)]
    {
        Display { kind: DisplayKind::Windows, x11: None, gnome: false }
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        detect_display(|k| std::env::var(k).ok(), |p| p.exists())
    }
}

/// Capture backends compiled into this binary.
pub fn compiled() -> Vec<String> {
    let mut list = Vec::new();
    if cfg!(all(feature = "desktop-mutter", target_os = "linux")) {
        list.push("mutter".to_string());
    }
    if cfg!(all(feature = "desktop-portal", target_os = "linux")) {
        list.push("portal".to_string());
    }
    if cfg!(all(feature = "desktop-x11", target_os = "linux")) {
        list.push("x11".to_string());
    }
    if cfg!(target_os = "macos") {
        list.push("macos".to_string());
    }
    if cfg!(windows) {
        list.push("windows".to_string());
    }
    list
}

/// Picks the backend for a display, or the reason none fits.
pub fn choose_backend(pref: BackendPref, display: &Display, compiled: &[String]) -> Result<&'static str, (&'static str, String)> {
    let has = |name: &str| compiled.iter().any(|c| c == name);
    if pref == BackendPref::TestCard {
        return Ok("test_card");
    }
    if compiled.is_empty() {
        return Err(("not_compiled", "Desktop streaming is not compiled into this unipi-host build".into()));
    }
    let want = match pref {
        BackendPref::Mutter => "mutter",
        BackendPref::Portal => "portal",
        BackendPref::X11 => "x11",
        BackendPref::TestCard => unreachable!("handled above"),
        BackendPref::Auto => match display.kind {
            // GNOME: Mutter's own API (no share dialog, works for the
            // logged-in user's processes); the portal when not compiled in.
            DisplayKind::Wayland if display.gnome && has("mutter") => "mutter",
            DisplayKind::Wayland => "portal",
            DisplayKind::X11 => "x11",
            DisplayKind::Macos => "macos",
            DisplayKind::Windows => "windows",
            DisplayKind::None => {
                return Err(("no_display", "No graphical session on this PC — it looks headless or unipi-host runs outside your login".into()))
            }
        },
    };
    if !has(want) {
        return Err(("not_compiled", format!("The {want} capture backend is not compiled into this unipi-host build")));
    }
    if want == "x11" && display.x11.is_none() {
        return Err(("no_display", "No X11 display ($DISPLAY) reachable from unipi-host".into()));
    }
    if (want == "portal" || want == "mutter") && display.kind == DisplayKind::None {
        return Err(("no_display", "No graphical session on this PC — it looks headless or unipi-host runs outside your login".into()));
    }
    Ok(want)
}

/// Where the portal restore token lives (`persist_mode=2`).
pub fn restore_token_path(root: &Path, view_only: bool) -> PathBuf {
    root.join(if view_only { "desktop-token-view" } else { "desktop-token-control" })
}

pub fn read_token(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

pub fn write_token(path: &Path, token: &str) {
    if let Err(error) = crate::data_dir::write_atomic(path, token.as_bytes()) {
        tracing::warn!(%error, "could not store the desktop restore token");
    }
}

/// One line per desktop connection in `<host dir>/desktop.log`.
pub fn log_session(root: &Path, line: &str) {
    use std::io::Write;
    let path = root.join("desktop.log");
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    if let Ok(mut file) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(file, "{stamp} {line}");
    }
    tracing::info!(op = "desktop", "{line}");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn env(map: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = map.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
        move |k| map.get(k).cloned()
    }

    #[test]
    fn detects_sessions() {
        let none = |_: &Path| false;
        let d = detect_display(env(&[("WAYLAND_DISPLAY", "wayland-0"), ("DISPLAY", ":0")]), none);
        assert_eq!(d, Display { kind: DisplayKind::Wayland, x11: Some(":0".into()), gnome: false });
        let d = detect_display(env(&[("DISPLAY", ":1"), ("XDG_SESSION_TYPE", "x11")]), none);
        assert_eq!(d.kind, DisplayKind::X11);
        let d = detect_display(env(&[]), none);
        assert_eq!(d.kind, DisplayKind::None);
        // systemd user service without session env: probe sockets.
        let d = detect_display(env(&[("XDG_RUNTIME_DIR", "/run/user/1000")]), |p| p.ends_with("wayland-0"));
        assert_eq!(d.kind, DisplayKind::Wayland);
        let d = detect_display(env(&[]), |p| p == Path::new("/tmp/.X11-unix/X0"));
        assert_eq!(d, Display { kind: DisplayKind::X11, x11: Some(":0".into()), gnome: false });
        // GNOME detection (systemd user env carries XDG_CURRENT_DESKTOP).
        for desktop in ["ubuntu:GNOME", "GNOME", "GNOME-Classic:GNOME", "pop:GNOME"] {
            assert!(detect_display(env(&[("WAYLAND_DISPLAY", "wayland-0"), ("XDG_CURRENT_DESKTOP", desktop)]), none).gnome, "{desktop}");
        }
        assert!(detect_display(env(&[("DESKTOP_SESSION", "gnome-xorg"), ("DISPLAY", ":0")]), none).gnome);
        for desktop in ["KDE", "sway", "XFCE", "Budgie"] {
            assert!(!detect_display(env(&[("WAYLAND_DISPLAY", "wayland-0"), ("XDG_CURRENT_DESKTOP", desktop)]), none).gnome, "{desktop}");
        }
    }

    #[test]
    fn chooses_backends() {
        let all = vec!["portal".to_string(), "x11".to_string()];
        let wl = Display { kind: DisplayKind::Wayland, x11: Some(":0".into()), gnome: false };
        let x = Display { kind: DisplayKind::X11, x11: Some(":0".into()), gnome: false };
        let headless = Display { kind: DisplayKind::None, x11: None, gnome: false };
        let gnome = Display { kind: DisplayKind::Wayland, x11: Some(":0".into()), gnome: true };
        let with_mutter = vec!["mutter".to_string(), "portal".to_string(), "x11".to_string()];
        assert_eq!(choose_backend(BackendPref::Auto, &gnome, &with_mutter), Ok("mutter"), "GNOME Wayland prefers Mutter");
        assert_eq!(choose_backend(BackendPref::Auto, &gnome, &all), Ok("portal"), "no mutter compiled → portal");
        assert_eq!(choose_backend(BackendPref::Auto, &wl, &with_mutter), Ok("portal"), "KDE/wlroots → portal");
        assert_eq!(choose_backend(BackendPref::Portal, &gnome, &with_mutter), Ok("portal"), "explicit portal wins");
        assert_eq!(choose_backend(BackendPref::Mutter, &wl, &with_mutter), Ok("mutter"));
        assert_eq!(choose_backend(BackendPref::Mutter, &gnome, &all).unwrap_err().0, "not_compiled");
        assert_eq!(choose_backend(BackendPref::Mutter, &headless, &with_mutter).unwrap_err().0, "no_display");
        let mac = Display { kind: DisplayKind::Macos, x11: None, gnome: false };
        let win = Display { kind: DisplayKind::Windows, x11: None, gnome: false };
        assert_eq!(choose_backend(BackendPref::Auto, &mac, &["macos".into()]), Ok("macos"));
        assert_eq!(choose_backend(BackendPref::Auto, &win, &["windows".into()]), Ok("windows"));
        assert_eq!(choose_backend(BackendPref::Auto, &mac, &["x11".into()]).unwrap_err().0, "not_compiled");
        assert_eq!(choose_backend(BackendPref::X11, &mac, &["macos".into()]).unwrap_err().0, "not_compiled");
        assert_eq!(choose_backend(BackendPref::Auto, &wl, &all), Ok("portal"));
        assert_eq!(choose_backend(BackendPref::Auto, &x, &all), Ok("x11"));
        assert_eq!(choose_backend(BackendPref::X11, &wl, &all), Ok("x11"));
        assert_eq!(choose_backend(BackendPref::Auto, &headless, &all).unwrap_err().0, "no_display");
        assert_eq!(choose_backend(BackendPref::Auto, &wl, &[]).unwrap_err().0, "not_compiled");
        assert_eq!(choose_backend(BackendPref::Auto, &wl, &["x11".into()]).unwrap_err().0, "not_compiled");
        assert_eq!(choose_backend(BackendPref::X11, &headless, &all).unwrap_err().0, "no_display");
        assert_eq!(choose_backend(BackendPref::TestCard, &headless, &[]), Ok("test_card"));
    }

    #[test]
    fn config_round_trip_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"forwarding":{"enabled":true,"ports":"any"}}"#).unwrap();
        assert_eq!(DesktopConfig::load(&path).unwrap(), DesktopConfig::default());
        let config = DesktopConfig { enabled: true, view_only: true, devices: vec!["Pixel".into()], backend: BackendPref::X11 };
        config.save(&path).unwrap();
        assert_eq!(DesktopConfig::load(&path).unwrap(), config);
        let raw: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(raw["forwarding"]["enabled"], true);
    }

    #[test]
    fn gate() {
        let off = DesktopConfig::default();
        assert_eq!(off.check("abc", None).unwrap_err().code, AppErrorCode::E_DESKTOP_DISABLED);
        let on = DesktopConfig { enabled: true, devices: vec!["Pixel".into(), "deadbeef12".into()], ..Default::default() };
        assert!(on.check("x", Some("Pixel")).is_ok());
        assert!(on.check("deadbeef1234", None).is_ok());
        assert!(on.check("cafe", Some("Redmi")).is_err());
    }

    #[test]
    fn unavailable_has_reason_and_hint() {
        let e = unavailable("denied", "Cancelled");
        assert_eq!(e.detail.unwrap()["reason"], "denied");
        assert!(e.hint.is_some());
        assert_eq!(BackendPref::parse("mutter"), Some(BackendPref::Mutter));
        assert_eq!(BackendPref::Mutter.as_str(), "mutter");
    }
}
