//! Remote desktop browser (UNI-215, forwarding phase 3).
//!
//! The host launches its **own** Chrome (Google Chrome first, then Chromium,
//! then other Chromium-based browsers — see [`CANDIDATES`]) with
//! `--remote-debugging-port=0` and a dedicated profile in
//! `<host dir>/browser-profile`. It never attaches to the user's running
//! browser: a separate `--user-data-dir` is a separate browser process, even
//! when the user's own Chrome is open (Chrome's process singleton is per
//! profile directory). The host is the CDP client: it streams JPEG screencast frames to
//! the phone and dispatches the phone's taps, drags, keys and text.
//!
//! Security: OFF by default. The owner enables it in `<host dir>/config.json`
//! (read on every request — no restart), or with `unipi-host browser enable`:
//!
//! ```json
//! { "browser": { "enabled": true, "headless": true, "idle_minutes": 10,
//!                "path": "/usr/bin/google-chrome", "devices": ["Pixel"] } }
//! ```
//!
//! - `headless` (default true): no window on the PC. `false` shows it.
//!   Changing it while the browser runs relaunches it in the new mode within
//!   a couple of seconds, reopening the same tabs (viewers get
//!   `closed{reason:"relaunch"}` and reconnect).
//! - `idle_minutes` (default 10): the browser quits after that long with no
//!   viewer. `0` = never.
//! - `path`: a specific browser binary (else auto-detect).
//! - `devices`: paired device ids / id prefixes (≥ 8 chars) / names;
//!   empty = all paired devices.
//!
//! The DevTools port listens on 127.0.0.1 only, but any local process of
//! any user on the PC can connect to it while the browser runs — it is the
//! dedicated profile, never the user's own.
//!
//! ## The profile directory
//!
//! The browser gets a `--user-data-dir` it can actually write:
//!
//! - every browser: `<host dir>/browser-profile`;
//! - a **snap** browser (`/snap/bin/chromium`, or a symlink to `/usr/bin/snap`):
//!   `~/snap/<snap name>/common/unipi-browser-profile`.
//!
//! Snap confinement (AppArmor) lets a snap write only inside its own
//! `~/snap/<name>/{current,common}` and its private `/tmp`, so the usual
//! profile makes snap Chromium die with
//! `Failed to create …/browser-profile/SingletonLock: Permission denied`
//! before DevTools ever comes up (UNI-287). See [`profile_for`].

pub mod agent;
pub mod agent_endpoint;
pub mod cdp;
pub mod cloak;
pub mod cookies;
pub mod display;
pub mod gate;
pub mod humanize;
pub mod profiles;
pub mod session;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::json;
use tokio::io::{AsyncBufReadExt, BufReader};
use unipi_proto::{AppErrorCode, AppErrorDto, BrowserAgent, BrowserInfo, BrowserProfileInfo, BrowserProfilesData, BrowserStatusData, BrowserTab};

use crate::data_dir::HostPaths;

/// Shown when the feature is off.
pub const ENABLE_HINT: &str = "On the PC run `unipi-host browser enable` (no restart needed).";
/// Shown when no browser was found (per OS).
pub const INSTALL_HINT: &str = if cfg!(target_os = "macos") {
    "Install Google Chrome on the PC (from google.com/chrome, or `brew install --cask google-chrome`), or point `unipi-host browser enable --path <binary>` at one."
} else if cfg!(windows) {
    "Install Google Chrome on the PC (from google.com/chrome, or `winget install Google.Chrome`), or point `unipi-host browser enable --path <binary>` at one."
} else {
    "Install Google Chrome on the PC (from google.com/chrome — the .deb or .rpm), or point `unipi-host browser enable --path <binary>` at one."
};

/// One browser we know how to find: a name on `PATH` or an absolute path
/// (`%VAR%` / `~` expanded), and its kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Candidate {
    pub bin: &'static str,
    pub kind: &'static str,
}

const fn c(bin: &'static str, kind: &'static str) -> Candidate {
    Candidate { bin, kind }
}

/// Tried in order, first found wins: Google Chrome, then Chromium, then the
/// other Chromium-based browsers last.
#[cfg(target_os = "macos")]
pub const CANDIDATES: &[Candidate] = &[
    c("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "chrome"),
    c("~/Applications/Google Chrome.app/Contents/MacOS/Google Chrome", "chrome"),
    c("/Applications/Chromium.app/Contents/MacOS/Chromium", "chromium"),
    c("~/Applications/Chromium.app/Contents/MacOS/Chromium", "chromium"),
    c("google-chrome", "chrome"),
    c("chromium", "chromium"),
    c("/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge", "edge"),
    c("/Applications/Brave Browser.app/Contents/MacOS/Brave Browser", "brave"),
];
#[cfg(windows)]
pub const CANDIDATES: &[Candidate] = &[
    c("%ProgramFiles%\\Google\\Chrome\\Application\\chrome.exe", "chrome"),
    c("%ProgramFiles(x86)%\\Google\\Chrome\\Application\\chrome.exe", "chrome"),
    c("%LocalAppData%\\Google\\Chrome\\Application\\chrome.exe", "chrome"),
    c("chrome.exe", "chrome"),
    c("%LocalAppData%\\Chromium\\Application\\chrome.exe", "chromium"),
    c("%ProgramFiles(x86)%\\Microsoft\\Edge\\Application\\msedge.exe", "edge"),
    c("%ProgramFiles%\\Microsoft\\Edge\\Application\\msedge.exe", "edge"),
    c("%ProgramFiles%\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", "brave"),
    c("%LocalAppData%\\BraveSoftware\\Brave-Browser\\Application\\brave.exe", "brave"),
];
#[cfg(not(any(target_os = "macos", windows)))]
pub const CANDIDATES: &[Candidate] = &[
    c("google-chrome", "chrome"),
    c("google-chrome-stable", "chrome"),
    c("/opt/google/chrome/chrome", "chrome"),
    c("chromium", "chromium"),
    c("chromium-browser", "chromium"),
    // A systemd user service's PATH usually lacks /snap/bin.
    c("/snap/bin/chromium", "chromium"),
    c("microsoft-edge", "edge"),
    c("microsoft-edge-stable", "edge"),
    c("brave-browser", "brave"),
    c("brave", "brave"),
];

/// The name shown to people for a browser kind. Chrome, Chromium and Edge by
/// name; everything else is "Chromium-based browser".
pub fn display_name(kind: &str) -> &'static str {
    match kind {
        "chrome" => "Google Chrome",
        "chromium" => "Chromium",
        "edge" => "Microsoft Edge",
        _ => "Chromium-based browser",
    }
}

/// The kind of a binary from its file name (configured paths).
pub fn kind_of(path: &Path) -> &'static str {
    let stem = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let stem = stem.trim_end_matches(".exe");
    if stem.contains("chrome") && !stem.contains("chromium") && path.to_string_lossy().to_ascii_lowercase().contains("chromium") {
        // %LocalAppData%\Chromium\Application\chrome.exe
        return "chromium";
    }
    if stem.contains("google-chrome") || stem == "chrome" || stem == "google chrome" {
        "chrome"
    } else if stem.contains("chromium") {
        "chromium"
    } else if stem.contains("edge") {
        "edge"
    } else if stem.contains("brave") {
        "brave"
    } else {
        "other"
    }
}

fn default_true() -> bool {
    true
}
fn default_idle() -> u32 {
    10
}
fn default_max_running() -> u32 {
    3
}
fn is_default_max_running(n: &u32) -> bool {
    *n == default_max_running()
}

/// Which browser the host runs (UNI-292).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    /// CloakBrowser (stealth Chromium, downloaded on first use; always
    /// headed). Falls back to Chrome when it can't be had or won't start.
    #[default]
    Cloak,
    /// The Chrome/Chromium detection of UNI-215/268.
    Chrome,
}

impl Engine {
    pub fn as_str(self) -> &'static str {
        match self {
            Engine::Cloak => "cloak",
            Engine::Chrome => "chrome",
        }
    }
    pub fn parse(text: &str) -> Option<Engine> {
        match text.trim().to_ascii_lowercase().as_str() {
            "cloak" | "cloakbrowser" => Some(Engine::Cloak),
            "chrome" | "chromium" => Some(Engine::Chrome),
            _ => None,
        }
    }
}

/// The `browser` block of `config.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BrowserConfig {
    #[serde(default)]
    pub enabled: bool,
    /// Chrome only: CloakBrowser always shows its window.
    #[serde(default = "default_true")]
    pub headless: bool,
    #[serde(default = "default_idle")]
    pub idle_minutes: u32,
    /// The Chrome-family binary (else auto-detect) — also the fallback's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub devices: Vec<String>,
    /// UNI-292: `cloak` (default) or `chrome`.
    #[serde(default)]
    pub engine: Engine,
    /// UNI-292: at most this many profiles' browsers at once (default 3).
    #[serde(default = "default_max_running", skip_serializing_if = "is_default_max_running")]
    pub max_running: u32,
    /// UNI-292: a CloakBrowser binary to use instead of the downloaded one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloak_path: Option<String>,
    /// UNI-292: a CloakBrowser version instead of the pinned free build.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cloak_version: Option<String>,
    /// UNI-293: `default` (the only on/off switch — always on) or `careful`.
    #[serde(default, skip_serializing_if = "humanize::Humanize::is_default")]
    pub humanize: humanize::Humanize,
    /// UNI-301: was `enabled: false` explicitly written to `config.json`
    /// (the owner's `unipi-host browser disable`, or hand-edited)? Never
    /// serialized — recomputed from the raw JSON on every [`Self::load`].
    /// Distinguishes that from a missing/default `enabled` (no key at all),
    /// which still allows the PC's own desktop app (see [`Self::is_enabled_for`]).
    #[serde(skip)]
    explicit_off: bool,
}

impl Default for BrowserConfig {
    fn default() -> Self {
        BrowserConfig {
            enabled: false,
            headless: true,
            idle_minutes: default_idle(),
            path: None,
            devices: Vec::new(),
            engine: Engine::default(),
            max_running: default_max_running(),
            cloak_path: None,
            cloak_version: None,
            humanize: humanize::Humanize::default(),
            explicit_off: false,
        }
    }
}

impl BrowserConfig {
    /// Reads `config.json`'s `browser` block; missing file/block = default (off,
    /// but the local app is still allowed — see [`Self::is_enabled_for`]).
    pub fn load(path: &Path) -> anyhow::Result<BrowserConfig> {
        let raw: Option<serde_json::Value> = crate::data_dir::read_json(path)?;
        let block = raw.as_ref().and_then(|v| v.get("browser")).cloned();
        let mut config: BrowserConfig = match &block {
            Some(v) => serde_json::from_value(v.clone())?,
            None => BrowserConfig::default(),
        };
        config.explicit_off = block.as_ref().and_then(|v| v.get("enabled")).and_then(|v| v.as_bool()) == Some(false);
        Ok(config)
    }

    /// Writes the block back, keeping every other key of `config.json`.
    pub fn save(&self, path: &Path) -> anyhow::Result<()> {
        let mut value: serde_json::Value =
            crate::data_dir::read_json(path)?.unwrap_or_else(|| json!({}));
        if !value.is_object() {
            value = json!({});
        }
        value["browser"] = serde_json::to_value(self)?;
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

    /// UNI-301: is the browser usable for `device_id` right now? The PC's own
    /// desktop app (`Peer::Local`, device id [`crate::protocol::LOCAL_DEVICE_ID`])
    /// works out of the box: absent/default `enabled` (`false`, never written
    /// by the user) still allows it. An explicit `enabled: false` — written by
    /// `unipi-host browser disable` or by the user — turns it off for
    /// everyone, local included. Phones (any other device id) still need
    /// `enabled: true` plus the device allow-list, as before (UNI-248).
    pub fn is_enabled_for(&self, device_id: &str) -> bool {
        if device_id == crate::protocol::LOCAL_DEVICE_ID {
            self.enabled || !self.explicit_off
        } else {
            self.enabled
        }
    }

    /// The gate every `browser_*` request goes through.
    pub fn check(&self, device_id: &str, device_name: Option<&str>) -> Result<(), AppErrorDto> {
        if !self.is_enabled_for(device_id) {
            return Err(disabled("The remote browser is off on this PC"));
        }
        if !self.device_allowed(device_id, device_name) {
            return Err(disabled("This phone is not allowed to use the remote browser on this PC"));
        }
        Ok(())
    }
}

fn disabled(message: &str) -> AppErrorDto {
    AppErrorDto::new(AppErrorCode::E_BROWSER_DISABLED, message, false).with_hint(ENABLE_HINT)
}

/// `E_BROWSER_UNAVAILABLE` with `detail.reason`.
pub fn unavailable(reason: &str, message: impl Into<String>) -> AppErrorDto {
    let mut error = AppErrorDto::new(AppErrorCode::E_BROWSER_UNAVAILABLE, message, reason != "not_found")
        .with_detail(json!({ "reason": reason }));
    if reason == "not_found" {
        error = error.with_hint(INSTALL_HINT);
    }
    error
}

// ------------------------------------------------------------- detection

#[cfg(unix)]
fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata().map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0).unwrap_or(false)
}

#[cfg(not(unix))]
fn is_executable(path: &Path) -> bool {
    path.is_file()
}

/// Expands `~/` and `%VAR%` with `env`; `None` when a variable is unset.
pub fn expand(bin: &str, env: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let mut out = if let Some(rest) = bin.strip_prefix("~/") {
        format!("{}/{rest}", env("HOME")?)
    } else {
        bin.to_string()
    };
    while let Some(start) = out.find('%') {
        let end = start + 1 + out[start + 1..].find('%')?;
        let value = env(&out[start + 1..end]).filter(|v| !v.is_empty())?;
        out.replace_range(start..=end, &value);
    }
    Some(out)
}

fn info(kind: &str, path: &Path) -> BrowserInfo {
    BrowserInfo { name: display_name(kind).to_string(), path: path.display().to_string(), version: None }
}

/// Finds a browser: `configured` (a path or a name on `PATH`) or the first of
/// `candidates` (names on `path_var`, absolute paths as they are).
pub fn detect_in(
    configured: Option<&str>,
    path_var: Option<&std::ffi::OsStr>,
    candidates: &[Candidate],
    env: &dyn Fn(&str) -> Option<String>,
) -> Option<BrowserInfo> {
    let dirs: Vec<PathBuf> = path_var.map(|p| std::env::split_paths(p).collect()).unwrap_or_default();
    let find = |name: &str| -> Option<PathBuf> {
        let name = expand(name, env)?;
        let path = PathBuf::from(&name);
        if path.is_absolute() || name.contains('/') {
            return is_executable(&path).then_some(path);
        }
        dirs.iter().map(|dir| dir.join(&name)).find(|p| is_executable(p))
    };
    if let Some(configured) = configured.filter(|c| !c.trim().is_empty()) {
        let path = find(configured.trim())?;
        return Some(info(kind_of(&path), &path));
    }
    candidates.iter().find_map(|cand| find(cand.bin).map(|path| info(cand.kind, &path)))
}

/// [`detect_in`] with this OS's [`CANDIDATES`] and the process env.
pub fn detect(configured: Option<&str>, path_var: Option<&std::ffi::OsStr>) -> Option<BrowserInfo> {
    detect_in(configured, path_var, CANDIDATES, &|k| {
        if k == "HOME" {
            return crate::fs::home_dir_opt().map(|home| home.display().to_string());
        }
        std::env::var(k).ok()
    })
}

/// Detects with the process `PATH`.
pub fn detect_here(config: &BrowserConfig) -> Option<BrowserInfo> {
    detect(config.path.as_deref(), std::env::var_os("PATH").as_deref())
}

/// Can a browser window show on this PC (a graphical session is reachable)?
pub fn can_show_window() -> bool {
    crate::desktop::config::detect_here().kind != unipi_proto::DisplayKind::None
}

/// Headless as configured, but always headless without a graphical session
/// (a headed launch would just fail there).
pub fn effective_headless(configured: bool, can_show: bool) -> bool {
    configured || !can_show
}

/// How often a running browser re-reads config.json (window toggle, idle).
pub const WATCH_EVERY: Duration = Duration::from_secs(2);

// --------------------------------------------------------------- profile

/// The profile directory name inside a snap's own writable `common/`.
pub const SNAP_PROFILE_NAME: &str = "unipi-browser-profile";

/// Directories snap applications live in (`/snap/bin/chromium`,
/// `/snap/chromium/3554/…`, `/var/lib/snapd/snap/…` on some distros).
const SNAP_DIRS: &[&str] = &["/snap/", "/var/lib/snapd/snap/"];

/// The snap package `path` belongs to, when it is one.
///
/// `/snap/bin/chromium` and `/snap/chromium/current/…` say it in the path;
/// every `/snap/bin/*` entry (and any other symlink to `/usr/bin/snap`) is a
/// snap launcher, so the symlink's own name is the package.
pub fn snap_name_of(path: &Path) -> Option<String> {
    let text = path.to_string_lossy();
    if let Some(rest) = SNAP_DIRS.iter().find_map(|dir| text.strip_prefix(dir)) {
        let first = rest.split('/').next().filter(|part| !part.is_empty())?;
        let name = if first == "bin" {
            // `/snap/bin/chromium` — the package is the entry's file name.
            match text.rsplit('/').next().filter(|n| !n.is_empty()) {
                Some(n) if n != "bin" => n,
                _ => return None,
            }
        } else if first.chars().all(|c| c.is_ascii_digit()) {
            return None; // /snap/<revision>/… without a package name
        } else {
            first
        };
        return Some(name.trim_end_matches(".exe").to_string());
    }
    // Any other symlink to snap's own launcher (`/usr/bin/snap`) is a snap
    // entry too — the package is the link's name.
    if std::fs::canonicalize(path).is_ok_and(|target| target == Path::new("/usr/bin/snap")) {
        let name = path.file_name()?.to_str()?;
        return (!name.is_empty()).then(|| name.trim_end_matches(".exe").to_string());
    }
    None
}

/// The profile directory to launch `bin` with: `<host dir>/browser-profile`
/// for everything except a snap, which only writes inside
/// `~/snap/<snap name>/common`.
pub fn profile_for(bin: &Path, default: &Path, home: &Path) -> PathBuf {
    match snap_name_of(bin) {
        Some(name) => home.join("snap").join(name).join("common").join(SNAP_PROFILE_NAME),
        None => default.to_path_buf(),
    }
}

/// [`profile_for`] with this process's `$HOME` (no snap directory without it).
pub fn profile_for_bin(bin: &Path, default: &Path) -> PathBuf {
    match crate::fs::home_dir() {
        Ok(home) => profile_for(bin, default, &home),
        Err(_) => default.to_path_buf(),
    }
}

/// The command-line flags for our dedicated browser; `urls` open as tabs
/// (a relaunch reopens the tabs it had), none = one blank tab.
pub fn launch_args(profile: &Path, headless: bool, urls: &[String]) -> Vec<String> {
    let mut args = vec![
        "--remote-debugging-port=0".to_string(),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--password-store=basic".into(),
        "--use-mock-keychain".into(),
        "--disable-features=Translate,MediaRouter".into(),
        "--hide-crash-restore-bubble".into(),
        "--disable-session-crashed-bubble".into(),
        "--window-size=1280,900".into(),
    ];
    if headless {
        args.push("--headless=new".into());
        args.push("--hide-scrollbars".into());
        args.push("--mute-audio".into());
    }
    let urls: Vec<&String> = urls.iter().filter(|u| reopenable(u)).take(MAX_REOPEN).collect();
    if urls.is_empty() {
        args.push("about:blank".into());
    } else {
        // Positional URLs after `--` so a page URL can never be read as a flag.
        args.push("--".into());
        args.extend(urls.into_iter().cloned());
    }
    args
}

/// At most this many tabs are reopened after a relaunch.
pub const MAX_REOPEN: usize = 20;

/// A tab URL worth reopening after a relaunch.
pub fn reopenable(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    !url.is_empty()
        && url.len() <= unipi_proto::browser::BROWSER_MAX_URL
        && !url.chars().any(|c| c.is_control())
        && ["http://", "https://", "file://", "about:blank", "data:"].iter().any(|p| lower.starts_with(p))
}

/// Parses Chrome's `DevTools listening on ws://…` stderr line.
pub fn parse_devtools_line(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("DevTools listening on ")?;
    rest.starts_with("ws://").then(|| rest.to_string())
}

/// The profile's writability / a screen to open a window on, as Chrome words it.
const PROFILE_TROUBLE: &[&str] =
    &["permission denied", "failed to create data directory", "cannot read and write to its data directory", "unable to create", "creating directory", "read-only file system"];
const NO_DISPLAY: &[&str] = &[
    "cannot open display",
    "failed to open x display",
    "unable to open x display",
    "no display name",
    "$display is not set",
    "display is not set",
    "missing x server",
    "cannot establish connection to x server",
];

/// A plain-language explanation of why a browser died before DevTools, read
/// off its last stderr lines — `profile` is the `--user-data-dir` we gave it.
/// `None` when nothing in the tail tells us anything.
pub fn failure_explanation(tail: &[String], bin: &Path, profile: &Path) -> Option<String> {
    let lower: Vec<String> = tail.iter().map(|l| l.to_ascii_lowercase()).collect();
    let says = |needles: &[&str]| lower.iter().any(|l| needles.iter().any(|n| l.contains(n)));
    let quote = |needles: &[&str]| {
        tail.iter()
            .zip(&lower)
            .find(|(_, l)| needles.iter().any(|n| l.contains(n)))
            .map(|(line, _)| line.trim().to_string())
            .filter(|line| !line.is_empty())
    };
    if says(PROFILE_TROUBLE) {
        let quoted = quote(PROFILE_TROUBLE).map(|l| format!(" — {l}")).unwrap_or_default();
        let mut message =
            format!("it could not write its profile directory {}{quoted}", profile.display());
        if let Some(name) = snap_name_of(bin) {
            message.push_str(&format!(
                " — a snap browser can only write inside ~/snap/{name}/common"
            ));
        }
        return Some(message);
    }
    if says(NO_DISPLAY) {
        let quoted = quote(NO_DISPLAY).map(|l| format!(" — {l}")).unwrap_or_default();
        return Some(format!("it could not open a window (no screen session on this PC){quoted}"));
    }
    None
}

// --------------------------------------------------------------- running

/// The pids of processes launched with `--user-data-dir=<profile>` (Linux
/// `/proc`; empty elsewhere), never this process.
pub fn profile_processes(profile: &Path) -> Vec<u32> {
    let flag = format!("--user-data-dir={}", profile.display());
    let me = std::process::id();
    let Ok(entries) = std::fs::read_dir("/proc") else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|entry| {
            let pid = entry.file_name().to_str().and_then(|n| n.parse::<u32>().ok())?;
            let cmdline = std::fs::read(entry.path().join("cmdline")).ok()?;
            (pid != me && cmdline_has_arg(&cmdline, &flag)).then_some(pid)
        })
        .collect()
}

/// Kills every process launched with `--user-data-dir=<profile>` — only our
/// dedicated profile, never the user's browser. Used before a launch (a
/// leftover from a crashed host would otherwise hold the profile lock and
/// swallow the new instance) and at host exit. Returns how many were killed.
pub fn kill_profile_processes(profile: &Path) -> usize {
    profile_processes(profile)
        .into_iter()
        .filter(|pid| std::process::Command::new("kill").args(["-KILL", &pid.to_string()]).status().is_ok_and(|s| s.success()))
        .count()
}

/// Does a `/proc/<pid>/cmdline` carry exactly `arg`? Chrome (e.g. snap
/// Chromium) may rewrite its title into ONE space-joined string, so a
/// single-entry cmdline is split on spaces too.
pub fn cmdline_has_arg(cmdline: &[u8], arg: &str) -> bool {
    let parts: Vec<&[u8]> = cmdline.split(|b| *b == 0).filter(|p| !p.is_empty()).collect();
    if parts.contains(&arg.as_bytes()) {
        return true;
    }
    parts.len() == 1 && String::from_utf8_lossy(parts[0]).split(' ').any(|p| p == arg)
}

/// The DevTools port of `ws://127.0.0.1:<port>/devtools/browser/<id>`.
pub fn ws_port(ws_url: &str) -> Option<u16> {
    let rest = ws_url.strip_prefix("ws://")?;
    let authority = rest.split('/').next()?;
    authority.rsplit(':').next()?.parse().ok()
}

/// Established TCP connections to local `port` in a `/proc/net/tcp` table.
pub fn established_to(table: &str, port: u16) -> usize {
    let want = format!(":{port:04X}");
    table
        .lines()
        .skip(1)
        .filter(|line| {
            let cols: Vec<&str> = line.split_whitespace().collect();
            cols.len() > 3 && cols[1].ends_with(&want) && cols[3] == "01"
        })
        .count()
}

/// CDP websocket clients connected to `port` (all of them, incl. the host's
/// own). Linux only; 0 elsewhere.
pub fn cdp_connections(port: u16) -> usize {
    ["/proc/net/tcp", "/proc/net/tcp6"]
        .iter()
        .filter_map(|p| std::fs::read_to_string(p).ok())
        .map(|t| established_to(&t, port))
        .sum()
}

/// Where a browser's window goes (UNI-299).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Headed, on the host's private X display: only the app (screencast)
    /// shows it. The default for CloakBrowser.
    Hidden,
    /// Headed on the PC's own desktop ("Show window on this PC").
    Window,
    /// No window at all.
    Headless,
}

impl Placement {
    pub fn as_str(self) -> &'static str {
        match self {
            Placement::Hidden => "hidden",
            Placement::Window => "window",
            Placement::Headless => "headless",
        }
    }
}

/// What CloakBrowser should do for this config: a shown window when the
/// user asked for one (`headless: false`) and there is a screen, else the
/// hidden display (never headless by choice: sites spot it).
pub fn wanted_placement(config: &BrowserConfig, can_show: bool) -> Placement {
    match config.engine {
        Engine::Cloak if !config.headless && can_show => Placement::Window,
        Engine::Cloak => Placement::Hidden,
        Engine::Chrome if effective_headless(config.headless, can_show) => Placement::Headless,
        Engine::Chrome => Placement::Window,
    }
}

/// The note shown when CloakBrowser had to run headless.
/// Linux: Xvfb would give a hidden display (UNI-299).
#[cfg(target_os = "linux")]
pub const HEADLESS_NOTE: &str = "Less stealthy: no hidden display on this PC (install Xvfb)";
/// Windows/macOS have no private X server: headless is how the browser stays
/// out of sight there; "Show window" puts it on the desktop instead.
#[cfg(not(target_os = "linux"))]
pub const HEADLESS_NOTE: &str = "Less stealthy: runs headless on this system (use Show window for a real window)";

/// Everything a launch needs.
struct LaunchSpec {
    info: BrowserInfo,
    args: Vec<String>,
    profile: PathBuf,
    headless: bool,
    engine: Engine,
    stealth: bool,
    stealth_reason: Option<String>,
    placement: Placement,
    /// `DISPLAY` to run on (hidden placement); `WAYLAND_DISPLAY` is removed then.
    display: Option<String>,
}

/// One running dedicated browser (one profile).
pub struct Running {
    pub cdp: cdp::Cdp,
    pub info: BrowserInfo,
    pub headless: bool,
    pub ws_url: String,
    /// The engine actually running (`Chrome` for a fallback).
    pub engine: Engine,
    /// CloakBrowser's fingerprint patches are active.
    pub stealth: bool,
    /// Why not (the fallback's cause / "engine chrome").
    pub stealth_reason: Option<String>,
    /// UNI-299: hidden display / own window / headless.
    pub placement: Placement,
    child: tokio::sync::Mutex<tokio::process::Child>,
    pid: Option<u32>,
    profile: Option<PathBuf>,
    /// UNI-296: the gated DevTools endpoint's port (agents connect here).
    gate_port: OnceLock<u16>,
}

impl Running {
    /// Still what `config` asks for (else the watcher relaunches it, tabs
    /// kept). A CloakBrowser that fell back to headless counts as hidden:
    /// relaunching would only fall back again.
    pub fn matches(&self, config: &BrowserConfig, can_show: bool) -> bool {
        match self.engine {
            Engine::Cloak => {
                let want = wanted_placement(config, can_show);
                self.placement == want || (want == Placement::Hidden && self.placement == Placement::Headless)
            }
            Engine::Chrome => self.headless == headless_for(config, can_show),
        }
    }

    /// The "less stealthy" note: CloakBrowser without any display.
    pub fn stealth_note(&self) -> Option<String> {
        (self.engine == Engine::Cloak && self.placement == Placement::Headless).then(|| HEADLESS_NOTE.to_string())
    }
}

impl Running {
    async fn launch(spec: LaunchSpec) -> Result<Running, AppErrorDto> {
        let LaunchSpec { info, args, profile, headless, engine, stealth, stealth_reason, placement, display: x_display } = spec;
        std::fs::create_dir_all(&profile)
            .map_err(|e| unavailable("launch_failed", format!("Can't create the browser profile: {e}")))?;
        if kill_profile_processes(&profile) > 0 {
            tracing::info!(op = "browser", "killed a leftover browser on our profile");
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
        let mut command = tokio::process::Command::new(&info.path);
        command
            .args(&args)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        if let Some(display) = &x_display {
            // UNI-299: only the private X display — never the user's desktop.
            command.env("DISPLAY", display).env_remove("WAYLAND_DISPLAY");
        }
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| unavailable("launch_failed", format!("Could not start {}: {e}", info.name)))?;
        let pid = child.id();
        let stderr = child.stderr.take().expect("piped stderr");
        let mut lines = BufReader::new(stderr).lines();
        let mut tail: Vec<String> = Vec::new();
        let found = tokio::time::timeout(Duration::from_secs(30), async {
            while let Ok(Some(line)) = lines.next_line().await {
                if let Some(url) = parse_devtools_line(&line) {
                    return Some(url);
                }
                tail.push(line);
                if tail.len() > 8 {
                    tail.remove(0);
                }
            }
            None
        })
        .await;
        let ws_url = match found {
            Ok(Some(url)) => url,
            other => {
                let _ = child.start_kill();
                kill_profile_processes(&profile);
                let why = if other.is_err() { "timed out" } else { "exited" };
                let mut message = format!("{} {why} before DevTools came up", info.name);
                if let Some(explanation) = failure_explanation(&tail, Path::new(&info.path), &profile) {
                    message.push_str(&format!(" — {explanation}"));
                }
                return Err(unavailable("launch_failed", message)
                    .with_detail(json!({ "reason": "launch_failed", "stderr": tail })));
            }
        };
        // Keep draining stderr so the browser never blocks on a full pipe.
        tokio::spawn(async move { while let Ok(Some(_)) = lines.next_line().await {} });
        let cdp = match cdp::Cdp::connect(&ws_url).await {
            Ok(cdp) => cdp,
            Err(e) => {
                let _ = child.start_kill();
                kill_profile_processes(&profile);
                return Err(unavailable("launch_failed", format!("Could not connect to {}: {e}", info.name)));
            }
        };
        let mut info = info;
        if let Ok(version) = cdp.call("Browser.getVersion", json!({}), None).await {
            info.version = version.get("product").and_then(|v| v.as_str()).map(str::to_string);
        }
        let _ = cdp.call("Target.setDiscoverTargets", json!({ "discover": true }), None).await;
        tracing::info!(op = "browser", browser = %info.name, engine = engine.as_str(), stealth, placement = placement.as_str(), x_display = x_display.as_deref().unwrap_or("-"), pid, profile = %profile.display(), "launched");
        Ok(Running {
            cdp,
            info,
            headless,
            ws_url,
            engine,
            stealth,
            stealth_reason,
            placement,
            child: tokio::sync::Mutex::new(child),
            pid,
            profile: Some(profile),
            gate_port: OnceLock::new(),
        })
    }

    /// After a relaunch: open `wanted` as tabs, then close everything else
    /// (the blank start tab, a restored session showing up late) — checked
    /// a few times as restores can land after the first look.
    async fn reopen_tabs(&self, wanted: &[String]) {
        let wanted: Vec<String> = wanted.iter().filter(|u| reopenable(u)).take(MAX_REOPEN).cloned().collect();
        for url in &wanted {
            let _ = self.cdp.call("Target.createTarget", json!({ "url": url }), None).await;
        }
        for _ in 0..4 {
            tokio::time::sleep(Duration::from_millis(350)).await;
            for id in extra_tabs(&self.tabs().await, &wanted) {
                let _ = self.cdp.call("Target.closeTarget", json!({ "targetId": id }), None).await;
            }
        }
    }

    pub fn alive(&self) -> bool {
        !self.cdp.is_closed()
    }

    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// The `--user-data-dir` it runs on.
    pub fn profile_dir(&self) -> Option<&Path> {
        self.profile.as_deref()
    }

    /// `http://127.0.0.1:<port>` (`/json/version`, `/json/list`).
    pub fn http_endpoint(&self) -> Option<String> {
        ws_port(&self.ws_url).map(|port| format!("http://127.0.0.1:{port}"))
    }

    /// Other CDP clients (an agent) connected now, besides the host itself.
    pub fn cdp_clients(&self) -> usize {
        ws_port(&self.ws_url).map(|p| cdp_connections(p).saturating_sub(1)).unwrap_or(0)
    }

    /// UNI-296: the gate's port (UNI-303's stable endpoint splices onto it).
    pub fn gate_port(&self) -> Option<u16> {
        self.gate_port.get().copied()
    }

    /// UNI-296: the gated endpoint agents should use (`http://127.0.0.1:<port>`).
    pub fn gate_http(&self) -> Option<String> {
        self.gate_port.get().map(|port| format!("http://127.0.0.1:{port}"))
    }

    /// UNI-296: the gated endpoint's browser websocket (same path as the raw one).
    pub fn gate_ws(&self) -> Option<String> {
        let port = self.gate_port.get()?;
        let path = self.ws_url.strip_prefix("ws://")?.split_once('/')?.1;
        Some(format!("ws://127.0.0.1:{port}/{path}"))
    }

    /// Starts the gated endpoint for `gate` (once; it ends with the browser).
    async fn start_gate(&self, gate: Arc<agent::AgentGate>, humanize: humanize::Humanize) {
        if self.gate_port.get().is_some() {
            return;
        }
        let Some(upstream) = ws_port(&self.ws_url) else { return };
        let cdp = self.cdp.clone();
        match gate::serve(gate, upstream, humanize, async move { cdp.closed().await }).await {
            Ok(port) => {
                let _ = self.gate_port.set(port);
                tracing::info!(op = "browser", port, "agent endpoint (take-over aware) ready");
            }
            Err(error) => tracing::warn!(op = "browser", %error, "could not open the agent endpoint"),
        }
    }

    /// Page targets (tabs), in CDP order.
    pub async fn tabs(&self) -> Vec<BrowserTab> {
        let Ok(result) = self.cdp.call("Target.getTargets", json!({}), None).await else { return Vec::new() };
        tabs_from_targets(&result)
    }

    async fn stop(&self) {
        let _ = tokio::time::timeout(Duration::from_secs(3), self.cdp.call("Browser.close", json!({}), None)).await;
        let mut child = self.child.lock().await;
        if tokio::time::timeout(Duration::from_secs(5), child.wait()).await.is_err() {
            if let Some(pid) = self.pid {
                // The whole process group (renderers, GPU, zygote).
                let _ = std::process::Command::new("kill").args(["-KILL", "--", &format!("-{pid}")]).status();
            }
            let _ = child.kill().await;
        }
        drop(child);
        // Wrapper scripts (brave-browser, snap chromium) leave the real
        // browser behind when only the wrapper dies.
        if let Some(profile) = self.profile.as_deref() {
            kill_profile_processes(profile);
        }
    }
}

/// Tabs to close so each wanted URL is open as often as asked, and nothing
/// else (restored session / new-tab pages) — keeping at least one tab.
pub fn extra_tabs(tabs: &[BrowserTab], wanted: &[String]) -> Vec<String> {
    let wanted: Vec<&String> = wanted.iter().filter(|u| reopenable(u)).take(MAX_REOPEN).collect();
    let mut left: HashMap<&str, usize> = HashMap::new();
    for url in &wanted {
        *left.entry(url.as_str()).or_default() += 1;
    }
    let mut close = Vec::new();
    for tab in tabs {
        match left.get_mut(tab.url.as_str()) {
            Some(n) if *n > 0 => *n -= 1,
            _ => close.push(tab.id.clone()),
        }
    }
    if close.len() == tabs.len() {
        close.pop();
    }
    close
}

/// `Target.getTargets` result → tabs (`type == "page"` only).
pub fn tabs_from_targets(result: &serde_json::Value) -> Vec<BrowserTab> {
    result
        .get("targetInfos")
        .and_then(|v| v.as_array())
        .map(|list| {
            list.iter()
                .filter(|t| t.get("type").and_then(|v| v.as_str()) == Some("page"))
                .filter_map(|t| {
                    Some(BrowserTab {
                        id: t.get("targetId")?.as_str()?.to_string(),
                        url: t.get("url").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                        title: t.get("title").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------- engines

/// Headless for this config: CloakBrowser is never headless by choice (a
/// hidden display or its own window); Chrome as configured, but always
/// headless without a graphical session.
pub fn headless_for(config: &BrowserConfig, can_show: bool) -> bool {
    match config.engine {
        Engine::Cloak => false,
        Engine::Chrome => effective_headless(config.headless, can_show),
    }
}

/// The host's one private X display (UNI-299), shared by every hidden
/// browser of this process; stopped at host exit.
fn hidden_displays() -> &'static display::Displays {
    static DISPLAYS: OnceLock<display::Displays> = OnceLock::new();
    DISPLAYS.get_or_init(display::Displays::new)
}

/// CloakBrowser's flags for `placement`: the X11 backend on the hidden
/// display (so it can't fall back to the user's Wayland desktop), and
/// `--headless=new` when no display was possible.
pub fn cloak_args_for(profile: &Path, seed: u32, placement: Placement) -> Vec<String> {
    let mut args = cloak_launch_args(profile, seed);
    let at = args.len().saturating_sub(1); // before the trailing about:blank
    match placement {
        Placement::Hidden => args.insert(at, "--ozone-platform=x11".into()),
        Placement::Headless => args.insert(at, "--headless=new".into()),
        Placement::Window => {}
    }
    args
}

/// CloakBrowser's flags: ours (DevTools on 127.0.0.1, the profile, quiet
/// first run) plus the wrapper's stealth set with the profile's seed. No
/// window size and no disabled features: a stock-looking headed browser.
pub fn cloak_launch_args(profile: &Path, seed: u32) -> Vec<String> {
    let mut args = vec![
        "--remote-debugging-port=0".to_string(),
        "--remote-debugging-address=127.0.0.1".to_string(),
        format!("--user-data-dir={}", profile.display()),
        "--no-first-run".into(),
        "--no-default-browser-check".into(),
        "--password-store=basic".into(),
        "--use-mock-keychain".into(),
        "--hide-crash-restore-bubble".into(),
    ];
    args.extend(cloak::stealth_args(seed));
    args.push("about:blank".into());
    args
}

/// Shown when CloakBrowser would need a window and there is no screen.
pub const NO_DISPLAY_MESSAGE: &str = "CloakBrowser always opens a window, and this PC has no screen session (it runs headless or over SSH only)";
pub const NO_DISPLAY_HINT: &str = "Start it from a desktop session, or use headless Chrome instead: unipi-host browser enable --engine chrome";

/// The binary + profile + flags to start `profile_name` with, after trying
/// CloakBrowser (download included) when the engine is `cloak`.
async fn plan_launch(host_dir: &Path, profile_name: &str, config: &BrowserConfig, can_show: bool) -> Result<LaunchSpec, AppErrorDto> {
    let cloak_dir = profiles::dir(host_dir, profile_name);
    let reason = match config.engine {
        Engine::Chrome => "the engine is set to chrome".to_string(),
        Engine::Cloak => {
            // In its own task: a viewer giving up mid-download doesn't waste it.
            let (path, version) = (config.cloak_path.clone(), config.cloak_version.clone());
            let ensured = tokio::spawn(async move { cloak::ensure(path.as_deref(), version.as_deref()).await }).await;
            match ensured.map_err(anyhow::Error::from).and_then(|r| r) {
                Ok(binary) => match cloak::profile_seed(&cloak_dir) {
                    Ok(seed) => {
                        // UNI-299: hidden display by default; a real window only on request.
                        let (placement, display) = match wanted_placement(config, can_show) {
                            Placement::Window => (Placement::Window, None),
                            _ => match hidden_displays().ensure().await {
                                Ok(d) => (Placement::Hidden, Some(d.display_var())),
                                Err(error) => {
                                    tracing::warn!(op = "browser", %error, "no hidden display: CloakBrowser runs headless");
                                    (Placement::Headless, None)
                                }
                            },
                        };
                        tracing::info!(op = "browser", path = %binary.path.display(), source = ?binary.source, seed, placement = placement.as_str(), "using CloakBrowser");
                        return Ok(LaunchSpec {
                            info: BrowserInfo { name: cloak::DISPLAY_NAME.into(), path: binary.path.display().to_string(), version: binary.version },
                            args: cloak_args_for(&cloak_dir, seed, placement),
                            profile: cloak_dir,
                            headless: placement == Placement::Headless,
                            engine: Engine::Cloak,
                            stealth: true,
                            stealth_reason: None,
                            placement,
                            display,
                        });
                    }
                    Err(e) => format!("CloakBrowser's fingerprint seed could not be written: {e}"),
                },
                Err(e) => format!("CloakBrowser is unavailable: {e:#}"),
            }
        }
    };
    chrome_spec(host_dir, profile_name, config, can_show, reason).await
}

async fn chrome_spec(host_dir: &Path, profile_name: &str, config: &BrowserConfig, can_show: bool, reason: String) -> Result<LaunchSpec, AppErrorDto> {
    let info = detect_here(config).ok_or_else(|| unavailable("not_found", "Google Chrome was not found on this PC"))?;
    let home = crate::fs::home_dir().ok();
    let profile = profiles::chrome_dir(Path::new(&info.path), host_dir, profile_name, home.as_deref());
    if config.engine == Engine::Cloak {
        tracing::warn!(op = "browser", %reason, browser = %info.name, "falling back to Chrome: NOT stealthy");
    }
    // UNI-299: the fallback honours the same choice as CloakBrowser — only in
    // the app unless "Show window" is on. Hidden display first, headless when
    // there is none; never a surprise window on the user's desktop.
    // As CloakBrowser would be placed (a Chrome-engine config keeps its own rule).
    let want = match config.engine {
        Engine::Cloak => wanted_placement(config, can_show),
        Engine::Chrome => wanted_placement(config, can_show),
    };
    let (placement, display) = match want {
        Placement::Window => (Placement::Window, None),
        Placement::Headless => (Placement::Headless, None),
        Placement::Hidden => match hidden_displays().ensure().await {
            Ok(d) => (Placement::Hidden, Some(d.display_var())),
            Err(_) => (Placement::Headless, None),
        },
    };
    let headless = placement == Placement::Headless;
    let mut args = launch_args(&profile, headless, &[]);
    if placement == Placement::Hidden {
        // Flags go before the positional part (`about:blank`, or `--` + URLs).
        let at = args.iter().position(|a| a == "--" || a == "about:blank").unwrap_or(args.len());
        args.insert(at, "--ozone-platform=x11".into());
    }
    Ok(LaunchSpec { args, info, profile, headless, engine: Engine::Chrome, stealth: false, stealth_reason: Some(reason), placement, display })
}

// --------------------------------------------------------------- manager

/// The browser of one profile: launched on demand, shared by every viewer
/// and CDP client, shut down after `idle_minutes` with neither.
pub struct Instance {
    pub name: String,
    running: tokio::sync::Mutex<Option<Arc<Running>>>,
    viewers: AtomicUsize,
    /// Last viewer leaving / start / CDP client seen.
    last_active: std::sync::Mutex<Instant>,
    /// The tab the last viewer looked at.
    pub last_target: std::sync::Mutex<Option<String>>,
    /// Its URL across a relaunch (tab ids change).
    last_url: std::sync::Mutex<Option<String>>,
    /// Fires when the browser is shut down on purpose (idle / close).
    shutdown: tokio::sync::broadcast::Sender<String>,
    /// UNI-296: agents' activity + the user's take-over (outlives relaunches).
    pub gate: Arc<agent::AgentGate>,
}

impl Instance {
    pub fn new(name: &str) -> Arc<Instance> {
        Arc::new(Instance {
            name: name.to_string(),
            running: tokio::sync::Mutex::new(None),
            viewers: AtomicUsize::new(0),
            last_active: std::sync::Mutex::new(Instant::now()),
            last_target: std::sync::Mutex::new(None),
            last_url: std::sync::Mutex::new(None),
            shutdown: tokio::sync::broadcast::channel(4).0,
            gate: agent::AgentGate::new(),
        })
    }

    /// The running browser, if any (and still connected).
    pub async fn current(&self) -> Option<Arc<Running>> {
        let guard = self.running.lock().await;
        guard.as_ref().filter(|r| r.alive()).cloned()
    }

    /// Without waiting on a launch in progress.
    pub fn current_now(&self) -> Option<Arc<Running>> {
        self.running.try_lock().ok()?.as_ref().filter(|r| r.alive()).cloned()
    }

    fn touch(&self) {
        *self.last_active.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn last_active(&self) -> Instant {
        *self.last_active.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// UNI-296: what agents do with it now: gated agents plus any client
    /// on the raw DevTools port that isn't a gated one's upstream.
    pub fn agent(&self, running: Option<&Running>) -> BrowserAgent {
        let direct = running.map(|r| r.cdp_clients().saturating_sub(self.gate.clients())).unwrap_or(0);
        self.gate.snapshot(direct)
    }

    /// A viewer or an agent uses it.
    pub fn busy(&self, running: Option<&Running>) -> bool {
        self.viewers() > 0 || running.is_some_and(|r| r.cdp_clients() > 0)
    }

    /// The running browser, launching it when needed (and relaunching it,
    /// tabs kept, when the window setting changed).
    async fn ensure(&self, host_dir: &Path, config: &BrowserConfig) -> Result<Arc<Running>, AppErrorDto> {
        let can_show = can_show_window();
        let mut guard = self.running.lock().await;
        let mut urls = Vec::new();
        if let Some(running) = guard.as_ref() {
            if running.alive() && running.matches(config, can_show) {
                return Ok(running.clone());
            }
            if running.alive() {
                urls = self.remember_tabs(running).await;
                let _ = self.shutdown.send("relaunch".into());
            }
            running.stop().await;
            *guard = None;
        }
        let spec = plan_launch(host_dir, &self.name, config, can_show).await?;
        // Tabs are reopened over CDP, not on the command line: some builds
        // also restore the previous session on top of command-line URLs.
        let running = match Running::launch(spec).await {
            Ok(running) => running,
            Err(error) if config.engine == Engine::Cloak => {
                // CloakBrowser itself wouldn't start: plain Chrome, not stealthy.
                let reason = format!("CloakBrowser failed to start: {}", error.message.as_str());
                tracing::warn!(op = "browser", %reason, "falling back to Chrome");
                let fallback = chrome_spec(host_dir, &self.name, config, can_show, reason).await?;
                Running::launch(fallback).await?
            }
            Err(error) => return Err(error),
        };
        let running = Arc::new(running);
        running.start_gate(self.gate.clone(), config.humanize).await;
        if !urls.is_empty() {
            running.reopen_tabs(&urls).await;
        }
        self.restore_last_target(&running).await;
        self.touch();
        *guard = Some(running.clone());
        Ok(running)
    }

    /// The open tabs' URLs (for a relaunch), noting which one was last viewed.
    async fn remember_tabs(&self, running: &Running) -> Vec<String> {
        let tabs = running.tabs().await;
        let last = self.last_target.lock().unwrap_or_else(|e| e.into_inner()).clone();
        *self.last_url.lock().unwrap_or_else(|e| e.into_inner()) =
            last.and_then(|id| tabs.iter().find(|t| t.id == id)).map(|t| t.url.clone());
        tabs.into_iter().map(|t| t.url).filter(|u| reopenable(u)).collect()
    }

    /// After a relaunch: point `last_target` at the new tab with the URL the
    /// last viewer looked at (tab ids change with the process).
    async fn restore_last_target(&self, running: &Running) {
        let url = self.last_url.lock().unwrap_or_else(|e| e.into_inner()).take();
        let tabs = if url.is_some() { running.tabs().await } else { Vec::new() };
        let id = url.and_then(|url| tabs.into_iter().find(|t| t.url == url)).map(|t| t.id);
        *self.last_target.lock().unwrap_or_else(|e| e.into_inner()) = id;
    }

    /// Shuts the browser down (idle, `browser_close`, eviction, tests).
    pub async fn shutdown(&self, reason: &str) -> bool {
        let running = self.running.lock().await.take();
        let Some(running) = running else { return false };
        let _ = self.shutdown.send(reason.to_string());
        running.stop().await;
        tracing::info!(op = "browser", profile = %self.name, reason, "shut down");
        true
    }

    pub fn subscribe_shutdown(&self) -> tokio::sync::broadcast::Receiver<String> {
        self.shutdown.subscribe()
    }

    pub fn viewers(&self) -> usize {
        self.viewers.load(Ordering::SeqCst)
    }

    /// Counts a viewer until the guard drops.
    pub fn viewer(self: &Arc<Self>) -> ViewerGuard {
        self.viewers.fetch_add(1, Ordering::SeqCst);
        ViewerGuard(self.clone())
    }

    /// Should an idle browser be shut down now? (`busy` = a CDP client is on.)
    pub fn idle_expired(&self, idle_minutes: u32, now: Instant) -> bool {
        idle_minutes > 0
            && self.viewers() == 0
            && now.duration_since(self.last_active()) >= Duration::from_secs(idle_minutes as u64 * 60)
    }
}

/// Decrements the viewer count (and stamps the idle clock) on drop.
pub struct ViewerGuard(Arc<Instance>);
impl Drop for ViewerGuard {
    fn drop(&mut self) {
        self.0.viewers.fetch_sub(1, Ordering::SeqCst);
        self.0.touch();
    }
}

/// What `browser start` reports.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartedInfo {
    pub profile: String,
    pub engine: String,
    pub stealth: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_reason: Option<String>,
    pub ws_url: String,
    /// `http://127.0.0.1:<port>`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub http: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    pub browser: BrowserInfo,
    pub headless: bool,
    /// UNI-299: `hidden` | `window` | `headless`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<String>,
    /// UNI-299: "Less stealthy…" when CloakBrowser had to run headless.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth_note: Option<String>,
    /// The browser was already running.
    pub reused: bool,
    /// UNI-296: the take-over aware endpoint for agents (`http://127.0.0.1:<port>`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_http: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_ws: Option<String>,
}

/// One row of `browser profiles`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProfileInfo {
    pub name: String,
    pub created: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_used: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_session: Option<String>,
    #[serde(default)]
    pub cookie_domains: Vec<profiles::CookieDomain>,
    pub running: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
    /// Viewers + CDP clients (agents) right now.
    #[serde(default)]
    pub viewers: usize,
    #[serde(default)]
    pub cdp_clients: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub engine: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stealth: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ws_url: Option<String>,
    pub dir: String,
}

impl StartedInfo {
    fn of(name: &str, running: &Running, reused: bool) -> StartedInfo {
        StartedInfo {
            profile: name.to_string(),
            engine: running.engine.as_str().into(),
            stealth: running.stealth,
            stealth_reason: running.stealth_reason.clone(),
            ws_url: running.ws_url.clone(),
            http: running.http_endpoint(),
            pid: running.pid,
            browser: running.info.clone(),
            headless: running.headless,
            display: Some(running.placement.as_str().into()),
            stealth_note: running.stealth_note(),
            reused,
            agent_http: running.gate_http(),
            agent_ws: running.gate_ws(),
        }
    }
}

/// The browsers of one host data dir: one [`Instance`] per profile in use,
/// at most `browser.max_running` running at once.
pub struct Manager {
    host_dir: PathBuf,
    instances: std::sync::Mutex<HashMap<String, Arc<Instance>>>,
    /// Serialises the cap decision + launch.
    starting: tokio::sync::Mutex<()>,
}

impl Manager {
    pub fn new(host_dir: PathBuf) -> Arc<Manager> {
        if let Err(error) = profiles::migrate(&host_dir, crate::push::now_ms()) {
            tracing::warn!(op = "browser", %error, "could not migrate the old browser profile");
        }
        Arc::new(Manager { host_dir, instances: Default::default(), starting: Default::default() })
    }

    fn registry() -> &'static std::sync::Mutex<HashMap<PathBuf, Arc<Manager>>> {
        static MANAGERS: OnceLock<std::sync::Mutex<HashMap<PathBuf, Arc<Manager>>>> = OnceLock::new();
        MANAGERS.get_or_init(Default::default)
    }

    /// Shuts down every browser this process started (host exit).
    pub async fn shutdown_all() {
        let managers: Vec<Arc<Manager>> =
            Self::registry().lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect();
        for manager in managers {
            for instance in manager.instances() {
                instance.shutdown("host_exit").await;
            }
        }
        hidden_displays().stop().await;
    }

    /// The manager for this host data dir (one per process and dir).
    pub fn for_paths(paths: &HostPaths) -> Arc<Manager> {
        let mut map = Self::registry().lock().unwrap_or_else(|e| e.into_inner());
        map.entry(paths.root.clone())
            .or_insert_with(|| {
                let manager = Manager::new(paths.root.clone());
                manager.clone().spawn_watch(paths.config());
                manager
            })
            .clone()
    }

    pub fn host_dir(&self) -> &Path {
        &self.host_dir
    }

    fn instances(&self) -> Vec<Arc<Instance>> {
        self.instances.lock().unwrap_or_else(|e| e.into_inner()).values().cloned().collect()
    }

    /// The instance of `name` (created, not started).
    pub fn instance(&self, name: &str) -> Arc<Instance> {
        self.instances.lock().unwrap_or_else(|e| e.into_inner()).entry(name.to_string()).or_insert_with(|| Instance::new(name)).clone()
    }

    /// UNI-296: profile `name`'s agent gate — what a relay reports agent
    /// input to and checks take-over with (see [`agent`]).
    pub fn gate(&self, name: &str) -> Arc<agent::AgentGate> {
        self.instance(name).gate.clone()
    }

    /// UNI-296: `browser_profiles` data.
    pub async fn profiles_data(&self) -> BrowserProfilesData {
        let profiles = self
            .profiles()
            .await
            .into_iter()
            .map(|p| BrowserProfileInfo {
                name: p.name,
                created: p.created,
                last_used: p.last_used,
                running: p.running,
                engine: p.engine,
                stealth: p.stealth,
                cookie_domains: p.cookie_domains.into_iter().map(|c| c.domain).collect(),
            })
            .collect();
        BrowserProfilesData { profiles, default: profiles::DEFAULT.into() }
    }

    fn existing(&self, name: &str) -> Option<Arc<Instance>> {
        self.instances.lock().unwrap_or_else(|e| e.into_inner()).get(name).cloned()
    }

    /// The default profile's running browser (status / tests). Never waits
    /// for a start in progress (a first start may be downloading).
    pub async fn current(&self) -> Option<Arc<Running>> {
        self.existing(profiles::DEFAULT)?.current_now()
    }

    /// Starts (or reuses) the browser of profile `name`, making room under
    /// `max_running` by stopping the least-recently-used idle one.
    pub async fn start(&self, name: &str, config: &BrowserConfig, session: Option<&str>) -> Result<(Arc<Instance>, Arc<Running>), AppErrorDto> {
        profiles::check_name(name).map_err(|e| AppErrorDto::new(AppErrorCode::E_SCHEMA, e.to_string(), false))?;
        let now = crate::push::now_ms();
        profiles::ensure(&self.host_dir, name, now).map_err(|e| AppErrorDto::new(AppErrorCode::E_NOT_FOUND, e.to_string(), false))?;
        let instance = self.instance(name);
        let can_show = can_show_window();
        if let Some(running) = instance.current().await {
            if running.matches(config, can_show) {
                instance.touch();
                let _ = profiles::touch(&self.host_dir, name, now, session);
                return Ok((instance, running));
            }
        }
        let _starting = self.starting.lock().await;
        // Room under the cap (a running instance never counts against itself).
        let slots: Vec<(profiles::Slot, Arc<Instance>)> = self
            .instances()
            .into_iter()
            .filter(|i| i.name != name)
            .filter_map(|i| {
                let running = i.current_now()?;
                let slot = profiles::Slot {
                    name: i.name.clone(),
                    busy: i.busy(Some(&running)),
                    last_active: i.last_active().elapsed().as_millis().try_into().map(|ago: u64| u64::MAX - ago).unwrap_or(0),
                };
                Some((slot, i))
            })
            .collect();
        let only: Vec<profiles::Slot> = slots.iter().map(|(s, _)| s.clone()).collect();
        let victims = profiles::pick_eviction(&only, name, config.max_running as usize).map_err(|m| unavailable("too_many", m))?;
        for victim in victims {
            tracing::info!(op = "browser", victim, starting = name, max = config.max_running, "at the cap: stopping the least-recently-used idle browser");
            if let Some((_, i)) = slots.iter().find(|(s, _)| s.name == victim) {
                i.shutdown("evicted").await;
            }
        }
        let running = instance.ensure(&self.host_dir, config).await?;
        let _ = profiles::touch(&self.host_dir, name, now, session);
        Ok((instance, running))
    }

    /// Stops profile `name`'s browser. Returns whether one ran.
    pub async fn stop(&self, name: &str, reason: &str) -> bool {
        match self.existing(name) {
            Some(i) => i.shutdown(reason).await,
            None => false,
        }
    }

    /// Relaunches running browsers whose window mode no longer matches the
    /// config (CloakBrowser: hidden display <-> window; Chrome: headless or not).
    pub async fn apply(&self, config: &BrowserConfig) -> Result<bool, AppErrorDto> {
        let mut any = false;
        let can_show = can_show_window();
        for instance in self.instances() {
            let Some(running) = instance.current_now() else { continue };
            // A running browser was started by someone allowed to (local or a
            // device with `enabled: true`); only an explicit `enabled: false`
            // (off for everyone) skips the relaunch here (UNI-301).
            if config.explicit_off || running.matches(config, can_show) {
                continue;
            }
            tracing::info!(op = "browser", profile = %instance.name, headless = config.headless, "window setting changed: relaunching");
            instance.ensure(&self.host_dir, config).await?;
            any = true;
        }
        Ok(any)
    }

    /// Every profile on disk, with what runs now.
    pub async fn profiles(&self) -> Vec<ProfileInfo> {
        let mut out = local_profiles(&self.host_dir);
        for row in &mut out {
            let Some(instance) = self.existing(&row.name) else { continue };
            row.viewers = instance.viewers();
            if let Some(running) = instance.current_now() {
                row.running = true;
                row.pid = running.pid;
                row.cdp_clients = running.cdp_clients();
                row.engine = Some(running.engine.as_str().into());
                row.stealth = Some(running.stealth);
                row.ws_url = Some(running.ws_url.clone());
                if let Some(dir) = running.profile_dir() {
                    row.dir = dir.display().to_string();
                }
            }
        }
        out
    }

    /// While browsers run: re-reads config.json every couple of seconds
    /// (the window toggle applies right away), keeps agents' browsers
    /// alive, and shuts idle ones down.
    fn spawn_watch(self: Arc<Self>, config_path: PathBuf) {
        let Ok(handle) = tokio::runtime::Handle::try_current() else { return };
        let weak = Arc::downgrade(&self);
        handle.spawn(async move {
            loop {
                tokio::time::sleep(WATCH_EVERY).await;
                let Some(manager) = weak.upgrade() else { return };
                let Ok(config) = BrowserConfig::load(&config_path) else { continue };
                for instance in manager.instances() {
                    let Some(running) = instance.current_now() else { continue };
                    if running.cdp_clients() > 0 {
                        instance.touch();
                    }
                    if instance.idle_expired(config.idle_minutes, Instant::now()) {
                        instance.shutdown("idle").await;
                    }
                }
                if let Err(error) = manager.apply(&config).await {
                    tracing::warn!(op = "browser", message = %error.message.as_str(), "relaunch failed");
                }
            }
        });
    }

    /// `browser_status` data (the default profile), `enabled` as it is for
    /// `device_id` (UNI-301: on for the local app even when the config has
    /// no explicit setting).
    pub async fn status(&self, config: &BrowserConfig, device_id: &str) -> BrowserStatusData {
        let can_show = can_show_window();
        let running = self.current().await;
        let (browser, engine, stealth, reason) = match &running {
            Some(r) => (Some(r.info.clone()), r.engine, r.stealth, r.stealth_reason.clone()),
            None => would_use(config),
        };
        let tabs = match &running {
            Some(r) => r.tabs().await,
            None => Vec::new(),
        };
        BrowserStatusData {
            enabled: config.is_enabled_for(device_id),
            install_hint: browser.is_none().then(|| INSTALL_HINT.to_string()),
            browser,
            running: running.is_some(),
            headless: running.as_ref().map(|r| r.headless).unwrap_or(headless_for(config, can_show)),
            idle_minutes: config.idle_minutes,
            tabs,
            can_show_window: Some(can_show),
            engine: Some(engine.as_str().into()),
            stealth: Some(stealth),
            stealth_reason: reason,
            stealth_note: running.as_ref().and_then(|r| r.stealth_note()),
            profile: Some(profiles::DEFAULT.into()),
            agent: Some(match self.existing(profiles::DEFAULT) {
                Some(instance) => instance.agent(running.as_deref()),
                None => BrowserAgent::default(),
            }),
            display: Some(match &running {
                Some(r) => r.placement.as_str().into(),
                None => wanted_placement(config, can_show).as_str().into(),
            }),
        }
    }
}

/// The browser a start would use now, without downloading anything:
/// (info, engine, stealth, why not stealthy).
pub fn would_use(config: &BrowserConfig) -> (Option<BrowserInfo>, Engine, bool, Option<String>) {
    let chrome = |reason: String| (detect_here(config), Engine::Chrome, false, Some(reason));
    match config.engine {
        Engine::Chrome => chrome("the engine is set to chrome".into()),
        Engine::Cloak => match cloak::locate(config.cloak_path.as_deref(), config.cloak_version.as_deref()) {
            Ok(Some(found)) => (
                Some(BrowserInfo { name: cloak::DISPLAY_NAME.into(), path: found.path.display().to_string(), version: found.version }),
                Engine::Cloak,
                true,
                None,
            ),
            // Not downloaded yet: it will be on first start.
            Ok(None) => match (cloak::wanted_version(config.cloak_version.as_deref()), cloak::cache_root()) {
                (Ok((version, _)), Some(root)) => (
                    Some(BrowserInfo { name: cloak::DISPLAY_NAME.into(), path: cloak::binary_in(&root.join(&version)).display().to_string(), version: Some(version) }),
                    Engine::Cloak,
                    true,
                    None,
                ),
                (Err(e), _) => chrome(format!("CloakBrowser is unavailable: {e}")),
                (_, None) => chrome("CloakBrowser is unavailable: no cache directory".into()),
            },
            Err(e) => chrome(format!("CloakBrowser is unavailable: {e}")),
        },
    }
}

/// The profiles on disk; `running`/`pid` from the process table only.
pub fn local_profiles(host_dir: &Path) -> Vec<ProfileInfo> {
    let _ = profiles::migrate(host_dir, crate::push::now_ms());
    let home = crate::fs::home_dir().ok();
    profiles::list(host_dir)
        .into_iter()
        .filter_map(|name| {
            let meta = profiles::load(host_dir, &name).ok().flatten()?;
            let dir = profiles::dir(host_dir, &name);
            let pid = profile_dirs(host_dir, &name, home.as_deref()).iter().find_map(|d| profile_processes(d).into_iter().min());
            Some(ProfileInfo {
                name,
                created: meta.created,
                last_used: meta.last_used,
                last_session: meta.last_session,
                cookie_domains: meta.cookie_domains,
                running: pid.is_some(),
                pid,
                viewers: 0,
                cdp_clients: 0,
                engine: None,
                stealth: None,
                ws_url: None,
                dir: dir.display().to_string(),
            })
        })
        .collect()
}

/// Every `--user-data-dir` a profile may run on: ours, and the snap
/// twins of the Chrome fallback.
pub fn profile_dirs(host_dir: &Path, name: &str, home: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs = vec![profiles::dir(host_dir, name)];
    if let Some(home) = home {
        if let Ok(snaps) = std::fs::read_dir(home.join("snap")) {
            for snap in snaps.flatten() {
                if let Some(snap) = snap.file_name().to_str() {
                    let twin = profiles::snap_dir(home, snap, host_dir, name);
                    if twin.is_dir() {
                        dirs.push(twin);
                    }
                }
            }
        }
    }
    dirs
}

// ------------------------------------------------------- local control

/// `browser *` requests from the CLI over `control.sock` (UNI-292).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum BrowserControl {
    Start {
        #[serde(default)]
        profile: Option<String>,
        #[serde(default)]
        session: Option<String>,
    },
    Stop {
        #[serde(default)]
        profile: Option<String>,
    },
    Profiles,
    /// UNI-294: the cookies were already decrypted (in the CLI, which has
    /// the user's D-Bus session) — the serve only starts/reuses the
    /// profile's browser, sends them over CDP and records the domain.
    Cookies {
        profile: String,
        domain: String,
        /// Pre-decrypted; the serve never decrypts anything itself.
        cookies: Vec<CookieWire>,
    },
}

/// One decrypted cookie crossing `control.sock` (never logged).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CookieWire {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub same_site: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires: Option<i64>,
}

/// Runs one CLI request in the serve; `Ok` is the JSON to send back.
pub async fn control(paths: &HostPaths, request: BrowserControl) -> Result<serde_json::Value, AppErrorDto> {
    let internal = |e: anyhow::Error| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false);
    let manager = Manager::for_paths(paths);
    match request {
        BrowserControl::Start { profile, session } => {
            let config = BrowserConfig::load(&paths.config()).map_err(internal)?;
            // UNI-301: `browser start` over control.sock is local by
            // definition — allowed unless the owner explicitly disabled it.
            if config.explicit_off {
                return Err(disabled("The remote browser is off on this PC"));
            }
            let name = profile.unwrap_or_else(|| profiles::DEFAULT.into());
            let reused = match manager.existing(&name) {
                Some(i) => i.current().await.is_some(),
                None => false,
            };
            let (_, running) = manager.start(&name, &config, session.as_deref()).await?;
            tracing::info!(op = "browser_start", profile = %name, reused, "cli");
            serde_json::to_value(StartedInfo::of(&name, &running, reused)).map_err(|e| internal(e.into()))
        }
        BrowserControl::Stop { profile } => {
            let name = profile.unwrap_or_else(|| profiles::DEFAULT.into());
            let stopped = manager.stop(&name, "closed").await;
            Ok(json!({ "profile": name, "stopped": stopped }))
        }
        BrowserControl::Profiles => serde_json::to_value(manager.profiles().await).map_err(|e| internal(e.into())),
        BrowserControl::Cookies { profile, domain, cookies } => {
            let config = BrowserConfig::load(&paths.config()).map_err(internal)?;
            let (_, running) = manager.start(&profile, &config, None).await?;
            let values: Vec<serde_json::Value> = cookies
                .iter()
                .map(|c| {
                    let mut v = json!({
                        "name": c.name, "value": c.value, "domain": c.domain, "path": c.path,
                        "secure": c.secure, "httpOnly": c.http_only,
                    });
                    if let Some(s) = &c.same_site {
                        v["sameSite"] = s.as_str().into();
                    }
                    if let Some(e) = c.expires {
                        v["expires"] = e.into();
                    }
                    v
                })
                .collect();
            let count = values.len();
            running
                .cdp
                .call("Storage.setCookies", json!({ "cookies": values }), None)
                .await
                .map_err(|e| unavailable("launch_failed", format!("Storage.setCookies failed: {e}")))?;
            profiles::record_cookie_domain(&manager.host_dir, &profile, &domain, crate::push::now_ms()).map_err(internal)?;
            tracing::info!(op = "browser_cookies", profile = %profile, domain = %domain, imported = count, "cli");
            Ok(json!({ "profile": profile, "domain": domain, "imported": count }))
        }
    }
}

/// Handles `browser_status` / `browser_open` / `browser_close` /
/// `browser_window` / `browser_profiles` / `browser_takeover` after the hello.
pub async fn serve_request<R, W>(
    paths: &HostPaths,
    req: unipi_proto::Request,
    device_id: &str,
    device_name: Option<&str>,
    read: &mut R,
    write: &mut W,
) -> Result<(), AppErrorDto>
where
    R: tokio::io::AsyncRead + Unpin,
    W: tokio::io::AsyncWrite + Unpin,
{
    use unipi_proto::{Control, Request};
    let internal = |e: anyhow::Error| AppErrorDto::new(AppErrorCode::E_INTERNAL, e.to_string(), false);
    let config = BrowserConfig::load(&paths.config()).map_err(internal)?;
    let manager = Manager::for_paths(paths);
    let reply = |data: Option<serde_json::Value>| Control::Ok { data };
    let write_err = |e: anyhow::Error| AppErrorDto::new(AppErrorCode::E_TRANSPORT, e.to_string(), true);
    match req {
        Request::BrowserStatus => {
            tracing::info!(op = "browser_status", "request");
            // Status is readable while off so the phone can explain how to enable it.
            if config.is_enabled_for(device_id) && !config.device_allowed(device_id, device_name) {
                config.check(device_id, device_name)?;
            }
            let data = manager.status(&config, device_id).await;
            let value = serde_json::to_value(data).map_err(|e| internal(e.into()))?;
            crate::protocol::write_control(write, &reply(Some(value))).await.map_err(write_err)?;
        }
        Request::BrowserOpen { viewport, url, target, profile } => {
            let name = profile.map(|p| p.as_str().to_string()).unwrap_or_else(|| profiles::DEFAULT.into());
            tracing::info!(op = "browser_open", w = viewport.width, h = viewport.height, profile = %name, "request");
            config.check(device_id, device_name)?;
            // Counted as a viewer before the start, so the cap never evicts it.
            let instance = manager.instance(&name);
            let viewer = instance.viewer();
            let (instance, running) = manager.start(&name, &config, None).await?;
            session::serve(instance, running, viewer, read, write, viewport, url.map(|u| u.as_str().to_string()), target).await?;
        }
        Request::BrowserClose => {
            tracing::info!(op = "browser_close", "request");
            config.check(device_id, device_name)?;
            manager.stop(profiles::DEFAULT, "closed").await;
            crate::protocol::write_control(write, &reply(None)).await.map_err(write_err)?;
        }
        Request::BrowserProfiles => {
            tracing::info!(op = "browser_profiles", "request");
            config.check(device_id, device_name)?;
            let value = serde_json::to_value(manager.profiles_data().await).map_err(|e| internal(e.into()))?;
            crate::protocol::write_control(write, &reply(Some(value))).await.map_err(write_err)?;
        }
        Request::BrowserTakeover { on, profile } => {
            let name = profile.map(|p| p.as_str().to_string()).unwrap_or_else(|| profiles::DEFAULT.into());
            tracing::info!(op = "browser_takeover", on, profile = %name, "request");
            config.check(device_id, device_name)?;
            let instance = manager.instance(&name);
            instance.gate.set_takeover(on);
            if on {
                instance.gate.note_user_input();
            }
            let agent = instance.agent(instance.current_now().as_deref());
            let value = serde_json::to_value(agent).map_err(|e| internal(e.into()))?;
            crate::protocol::write_control(write, &reply(Some(value))).await.map_err(write_err)?;
        }
        Request::BrowserWindow { show } => {
            tracing::info!(op = "browser_window", show, "request");
            config.check(device_id, device_name)?;
            if show && !can_show_window() {
                return Err(unavailable(
                    "no_display",
                    "This PC has no screen session to show a window on (it runs headless or over SSH only)",
                ));
            }
            let mut next = config.clone();
            next.headless = !show;
            if next != config {
                next.save(&paths.config()).map_err(internal)?;
            }
            manager.apply(&next).await?;
            let data = manager.status(&next, device_id).await;
            let value = serde_json::to_value(data).map_err(|e| internal(e.into()))?;
            crate::protocol::write_control(write, &reply(Some(value))).await.map_err(write_err)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_off_headless_ten_minutes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let config = BrowserConfig::load(&path).unwrap();
        assert!(!config.enabled);
        assert!(config.headless);
        assert_eq!(config.engine, Engine::Cloak);
        assert_eq!(config.idle_minutes, 10);
        std::fs::write(&path, r#"{"forwarding":{"enabled":true,"ports":"any"},"browser":{"enabled":true}}"#).unwrap();
        let config = BrowserConfig::load(&path).unwrap();
        assert!(config.enabled && config.headless);
    }

    #[test]
    fn save_keeps_other_keys() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, r#"{"forwarding":{"enabled":true,"ports":[5173]},"other":1}"#).unwrap();
        BrowserConfig { enabled: true, headless: false, idle_minutes: 3, ..Default::default() }.save(&path).unwrap();
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(value["forwarding"]["ports"], json!([5173]));
        assert_eq!(value["other"], 1);
        assert_eq!(value["browser"]["headless"], false);
        assert_eq!(value["browser"]["idle_minutes"], 3);
        let forward = crate::forward::HostConfig::load(&path).unwrap();
        assert!(forward.forwarding.enabled);
    }

    #[test]
    fn gate_off_and_per_device() {
        let off = BrowserConfig::default();
        let error = off.check("abcdef0123456789", None).unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_BROWSER_DISABLED);
        assert!(error.hint.unwrap().contains("unipi-host browser enable"));
        let on = BrowserConfig { enabled: true, devices: vec!["Pixel".into(), "abcdef01".into()], ..Default::default() };
        assert!(on.check("zzz", Some("Pixel")).is_ok());
        assert!(on.check("abcdef0123", None).is_ok());
        assert!(on.check("zzz", Some("Redmi")).is_err());
        let all = BrowserConfig { enabled: true, ..Default::default() };
        assert!(all.check("x", None).is_ok());
    }

    /// UNI-301: absent/default `enabled` still allows the PC's own desktop
    /// app (`Peer::Local`); an iroh device needs `enabled: true`; an explicit
    /// `enabled: false` (user-written) refuses everyone, local included.
    #[test]
    fn local_is_allowed_by_default_but_an_explicit_off_refuses_everyone() {
        use crate::protocol::LOCAL_DEVICE_ID;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");

        // No config.json at all: local allowed, a phone refused.
        let config = BrowserConfig::load(&path).unwrap();
        assert!(config.check(LOCAL_DEVICE_ID, Some("This computer")).is_ok());
        let error = config.check("abcdef0123456789", Some("Pixel")).unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_BROWSER_DISABLED);

        // `browser.enabled` absent from an existing config.json: same.
        std::fs::write(&path, r#"{"forwarding":{"enabled":true,"ports":"any"}}"#).unwrap();
        let config = BrowserConfig::load(&path).unwrap();
        assert!(config.check(LOCAL_DEVICE_ID, None).is_ok());
        assert!(config.check("abcdef0123456789", Some("Pixel")).is_err());

        // An iroh device with `enabled: true` + its id/name allowed: on for it too.
        std::fs::write(&path, r#"{"browser":{"enabled":true,"devices":["Pixel"]}}"#).unwrap();
        let config = BrowserConfig::load(&path).unwrap();
        assert!(config.check(LOCAL_DEVICE_ID, None).is_ok());
        assert!(config.check("abcdef0123456789", Some("Pixel")).is_ok());

        // Explicit `enabled: false` (the owner's `browser disable`, or
        // hand-edited): off for everyone, local included.
        std::fs::write(&path, r#"{"browser":{"enabled":false}}"#).unwrap();
        let config = BrowserConfig::load(&path).unwrap();
        assert!(config.explicit_off);
        let error = config.check(LOCAL_DEVICE_ID, Some("This computer")).unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_BROWSER_DISABLED);
        assert!(config.check("abcdef0123456789", Some("Pixel")).is_err());

        // Writing the default config back out does not turn it back on for
        // local by itself (no key at all vs. an explicit `false` are
        // different only until the owner writes something — `save` always
        // serializes the field).
        let default = BrowserConfig::default();
        assert!(!default.explicit_off);
    }

    #[test]
    fn unavailable_carries_reason_and_install_hint() {
        let e = unavailable("not_found", "none");
        assert_eq!(e.code, AppErrorCode::E_BROWSER_UNAVAILABLE);
        assert_eq!(e.detail.unwrap()["reason"], "not_found");
        let hint = e.hint.unwrap();
        assert!(hint.starts_with("Install Google Chrome on the PC"), "{hint}");
        assert!(unavailable("crashed", "x").hint.is_none());
    }

    #[test]
    fn user_facing_text_never_names_other_browsers() {
        // UNI-268: Chrome is the browser we tell people about.
        for text in [INSTALL_HINT, ENABLE_HINT] {
            assert!(!text.to_ascii_lowercase().contains("brave"), "{text}");
        }
        for kind in ["chrome", "chromium", "edge", "brave", "other"] {
            assert!(!display_name(kind).to_ascii_lowercase().contains("brave"));
        }
        assert_eq!(display_name("chrome"), "Google Chrome");
        assert_eq!(display_name("chromium"), "Chromium");
        assert_eq!(display_name("edge"), "Microsoft Edge");
        assert_eq!(display_name("brave"), "Chromium-based browser");
    }

    #[test]
    fn candidates_put_chrome_first_and_other_chromium_browsers_last() {
        let kinds: Vec<&str> = CANDIDATES.iter().map(|c| c.kind).collect();
        assert_eq!(kinds[0], "chrome");
        let last_chrome = kinds.iter().rposition(|k| *k == "chrome").unwrap();
        let first_chromium = kinds.iter().position(|k| *k == "chromium").unwrap();
        let last_chromium = kinds.iter().rposition(|k| *k == "chromium").unwrap();
        let first_other = kinds.iter().position(|k| *k != "chrome" && *k != "chromium").unwrap();
        assert!(last_chrome < first_chromium || cfg!(target_os = "macos"), "{kinds:?}");
        assert!(last_chromium < first_other, "{kinds:?}");
        assert!(kinds.contains(&"brave"), "still detected, just last");
    }

    fn exe(dir: &Path, name: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        if let Some(parent) = p.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[test]
    fn detects_chrome_first_and_honours_configured_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = std::ffi::OsString::from(dir.path().join("bin"));
        let no_env = |_: &str| None;
        // Linux order, with the absolute snap path pointed into the temp dir.
        let snap = dir.path().join("snap/chromium").display().to_string();
        let snap: &'static str = Box::leak(snap.into_boxed_str());
        let cands = [
            c("google-chrome", "chrome"),
            c("google-chrome-stable", "chrome"),
            c("chromium", "chromium"),
            c(snap, "chromium"),
            c("microsoft-edge", "edge"),
            c("brave-browser", "brave"),
        ];
        let detect = |configured: Option<&str>| detect_in(configured, Some(&path), &cands, &no_env);
        assert!(detect(None).is_none());
        exe(dir.path(), "bin/brave-browser");
        assert_eq!(detect(None).unwrap().name, "Chromium-based browser");
        exe(dir.path(), "bin/microsoft-edge");
        assert_eq!(detect(None).unwrap().name, "Microsoft Edge");
        exe(dir.path(), "snap/chromium");
        let found = detect(None).unwrap();
        assert_eq!((found.name.as_str(), found.path.as_str()), ("Chromium", snap));
        // Not executable: ignored.
        std::fs::write(dir.path().join("bin/google-chrome-stable"), "").unwrap();
        assert_eq!(detect(None).unwrap().name, "Chromium");
        exe(dir.path(), "bin/google-chrome-stable");
        assert_eq!(detect(None).unwrap().name, "Google Chrome");
        // Configured by name or path wins over the order.
        assert_eq!(detect(Some("microsoft-edge")).unwrap().name, "Microsoft Edge");
        assert_eq!(detect(Some("brave-browser")).unwrap().name, "Chromium-based browser");
        let custom = exe(dir.path(), "my-browser");
        assert_eq!(detect(Some(custom.to_str().unwrap())).unwrap().name, "Chromium-based browser");
        assert!(detect(Some("/nope/chrome")).is_none());
    }

    #[test]
    fn expands_home_and_windows_vars() {
        let env = |k: &str| match k {
            "HOME" => Some("/home/me".to_string()),
            "LocalAppData" => Some("C:\\Users\\me\\AppData\\Local".to_string()),
            _ => None,
        };
        assert_eq!(expand("~/Applications/X.app", &env).unwrap(), "/home/me/Applications/X.app");
        assert_eq!(
            expand("%LocalAppData%\\Google\\Chrome\\Application\\chrome.exe", &env).unwrap(),
            "C:\\Users\\me\\AppData\\Local\\Google\\Chrome\\Application\\chrome.exe"
        );
        assert!(expand("%ProgramFiles(x86)%\\x.exe", &env).is_none(), "unset var: skip");
        assert_eq!(expand("google-chrome", &env).unwrap(), "google-chrome");
    }

    #[test]
    fn kind_from_a_configured_binary() {
        assert_eq!(kind_of(Path::new("/usr/bin/google-chrome-stable")), "chrome");
        assert_eq!(kind_of(Path::new("/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")), "chrome");
        assert_eq!(kind_of(Path::new("C:/Program Files/Google/Chrome/Application/chrome.exe")), "chrome");
        assert_eq!(kind_of(Path::new("C:/Users/me/AppData/Local/Chromium/Application/chrome.exe")), "chromium");
        assert_eq!(kind_of(Path::new("/snap/bin/chromium")), "chromium");
        assert_eq!(kind_of(Path::new("/usr/bin/microsoft-edge")), "edge");
        assert_eq!(kind_of(Path::new("/usr/bin/brave-browser")), "brave");
        assert_eq!(kind_of(Path::new("/opt/x/thing")), "other");
    }

    #[test]
    fn launch_args_use_own_profile_and_ephemeral_port() {
        let args = launch_args(Path::new("/tmp/h/browser-profile"), true, &[]);
        assert!(args.contains(&"--remote-debugging-port=0".to_string()));
        assert!(args.contains(&"--user-data-dir=/tmp/h/browser-profile".to_string()));
        assert!(args.contains(&"--headless=new".to_string()));
        assert_eq!(args.last().unwrap(), "about:blank");
        let headed = launch_args(Path::new("/p"), false, &[]);
        assert!(!headed.iter().any(|a| a.starts_with("--headless")));
    }

    #[test]
    fn relaunch_reopens_the_tabs_after_a_double_dash() {
        let urls = vec![
            "https://a.example/x".to_string(),
            "chrome://settings".to_string(),
            "--evil-flag".to_string(),
            "about:blank".to_string(),
            "http://127.0.0.1:5173/".to_string(),
        ];
        let args = launch_args(Path::new("/p"), false, &urls);
        let dash = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(&args[dash + 1..], ["https://a.example/x", "about:blank", "http://127.0.0.1:5173/"]);
        let many: Vec<String> = (0..50).map(|i| format!("https://e/{i}")).collect();
        assert_eq!(launch_args(Path::new("/p"), true, &many).iter().skip_while(|a| *a != "--").count(), MAX_REOPEN + 1);
    }

    #[test]
    fn profile_flag_matches_rewritten_cmdlines() {
        let flag = "--user-data-dir=/h/browser-profile";
        assert!(cmdline_has_arg(b"/opt/chrome\0--user-data-dir=/h/browser-profile\0about:blank\0", flag));
        // Snap Chromium's main process: one space-joined title.
        assert!(cmdline_has_arg(b"/snap/chromium/1/chrome --x --user-data-dir=/h/browser-profile -- http://a/\0", flag));
        assert!(!cmdline_has_arg(b"/opt/chrome\0--user-data-dir=/h/browser-profile-2\0", flag));
        assert!(!cmdline_has_arg(b"/snap/chromium/1/chrome --user-data-dir=/h/browser-profile2\0", flag));
        // A multi-arg cmdline is never space-split (a shell line mentioning the flag).
        assert!(!cmdline_has_arg(b"bash\0-c\0echo --user-data-dir=/h/browser-profile\0", flag));
    }

    #[test]
    fn extra_tabs_after_a_relaunch() {
        let tab = |id: &str, url: &str| BrowserTab { id: id.into(), url: url.into(), title: String::new() };
        let wanted = vec!["https://a/".to_string(), "https://b/".to_string()];
        // A restored session doubled them, plus a new-tab page.
        let tabs = [tab("1", "https://a/"), tab("2", "https://b/"), tab("3", "https://a/"), tab("4", "https://b/"), tab("5", "chrome://newtab/")];
        assert_eq!(extra_tabs(&tabs, &wanted), ["3", "4", "5"]);
        // Two tabs on the same URL were wanted: both kept.
        let same = vec!["https://a/".to_string(), "https://a/".to_string()];
        assert!(extra_tabs(&tabs[..1].iter().chain(&tabs[2..3]).cloned().collect::<Vec<_>>(), &same).is_empty());
        // Nothing matches (pages still loading): keep one.
        assert_eq!(extra_tabs(&[tab("9", "about:blank"), tab("8", "x")], &wanted), ["9"]);
    }

    #[test]
    fn no_screen_means_headless() {
        assert!(effective_headless(true, true));
        assert!(!effective_headless(false, true));
        assert!(effective_headless(false, false));
    }

    #[test]
    fn devtools_line() {
        assert_eq!(
            parse_devtools_line("DevTools listening on ws://127.0.0.1:41234/devtools/browser/abc\n").as_deref(),
            Some("ws://127.0.0.1:41234/devtools/browser/abc")
        );
        assert!(parse_devtools_line("[123:ERROR] whatever").is_none());
    }

    #[test]
    fn snap_browsers_get_a_profile_they_can_write() {
        let home = Path::new("/home/me");
        let default = Path::new("/data/browser-profile");
        let snap_profile = home.join("snap/chromium/common/unipi-browser-profile");
        assert_eq!(profile_for(Path::new("/snap/bin/chromium"), default, home), snap_profile);
        assert_eq!(profile_for(Path::new("/snap/chromium/3554/usr/lib/chromium-browser/chrom"), default, home), snap_profile);
        assert_eq!(
            profile_for(Path::new("/var/lib/snapd/snap/bin/brave-browser"), default, home),
            home.join("snap/brave-browser/common/unipi-browser-profile")
        );
        // Everything not a snap keeps the host's own profile.
        for bin in ["/usr/bin/google-chrome", "/opt/brave.com/brave/brave-browser", "/snap/bin", "/snap/bin/", "/usr/bin/chromium.exe"] {
            assert_eq!(profile_for(Path::new(bin), default, home), default, "{bin}");
        }
    }

    #[test]
    fn snap_name_of_follows_launcher_symlinks() {
        assert_eq!(snap_name_of(Path::new("/snap/bin/chromium")).as_deref(), Some("chromium"));
        assert_eq!(snap_name_of(Path::new("/snap/bin/brave-browser")).as_deref(), Some("brave-browser"));
        assert_eq!(snap_name_of(Path::new("/snap/microsoft-edge/current/microsoft-edge")).as_deref(), Some("microsoft-edge"));
        assert!(snap_name_of(Path::new("/usr/bin/chromium")).is_none());
        assert!(snap_name_of(Path::new("/usr/local/bin/x")).is_none());
        assert!(snap_name_of(Path::new("/snap/3554/usr/lib/chrom")).is_none(), "revision without a package");
        // A real /snap/bin entry: a symlink to snap's own launcher.
        let dir = tempfile::tempdir().unwrap();
        let link = dir.path().join("chromium");
        #[cfg(unix)]
        std::os::unix::fs::symlink("/usr/bin/snap", &link).unwrap();
        #[cfg(unix)]
        assert_eq!(snap_name_of(&link).as_deref(), Some("chromium"));
    }

    #[test]
    fn launch_failure_explains_itself() {
        let snap = Path::new("/snap/bin/chromium");
        let chrome = Path::new("/usr/bin/google-chrome");
        let profile = Path::new("/home/me/.unipi/app-host/browser-profile");
        // Snap confinement: the profile can't be created/written.
        let tail = vec![
            "[1:1:1013/111741.407449:ERROR:chrome/browser/ui/views/message_box_dialog.cc:200] Unable to show message box:".to_string(),
            "Failed To Create Data Directory - Chromium cannot read and write to its data directory: /home/me/.cache/x/browser-profile".to_string(),
        ];
        let why = failure_explanation(&tail, snap, profile).unwrap();
        assert!(why.contains("/home/me/.unipi/app-host/browser-profile"), "{why}");
        assert!(why.contains("~/snap/chromium/common"), "{why}");
        let why =
            failure_explanation(&["Failed to create /x/browser-profile/SingletonLock: Permission denied".to_string()], chrome, profile).unwrap();
        assert!(why.contains("Permission denied"), "{why}");
        // A missing screen session.
        let why = failure_explanation(&["ERROR: Failed to open X display".to_string()], chrome, profile).unwrap();
        assert!(why.contains("no screen session"), "{why}");
        // Nothing recognisable: the plain message stands.
        assert!(failure_explanation(&["[1:1:ERROR] whatever".to_string()], chrome, profile).is_none());
        assert!(failure_explanation(&[], chrome, profile).is_none());
    }

    #[test]
    fn tabs_keep_pages_only() {
        let result = json!({ "targetInfos": [
            { "targetId": "A", "type": "page", "url": "https://a/", "title": "A" },
            { "targetId": "B", "type": "service_worker", "url": "https://a/sw.js", "title": "" },
            { "targetId": "C", "type": "page", "url": "about:blank", "title": "" },
        ]});
        let tabs = tabs_from_targets(&result);
        assert_eq!(tabs.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(), ["A", "C"]);
    }

    #[tokio::test]
    async fn viewer_count_and_idle_clock() {
        let instance = Instance::new("default");
        let start = Instant::now();
        {
            let _a = instance.viewer();
            let _b = instance.viewer();
            assert_eq!(instance.viewers(), 2);
            assert!(!instance.idle_expired(1, start + Duration::from_secs(3600)));
        }
        assert_eq!(instance.viewers(), 0);
        let left = Instant::now();
        assert!(!instance.idle_expired(1, left + Duration::from_secs(30)));
        assert!(instance.idle_expired(1, left + Duration::from_secs(61)));
        assert!(!instance.idle_expired(0, left + Duration::from_secs(1 << 20)), "0 = never");
    }

    #[test]
    fn engine_defaults_to_cloak_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        let config = BrowserConfig::load(&path).unwrap();
        assert_eq!((config.engine, config.max_running), (Engine::Cloak, 3));
        std::fs::write(&path, r#"{"browser":{"enabled":true,"engine":"chrome","max_running":5,"cloak_version":"146.0.7680.177.5"}}"#).unwrap();
        let config = BrowserConfig::load(&path).unwrap();
        assert_eq!((config.engine, config.max_running), (Engine::Chrome, 5));
        config.save(&path).unwrap();
        let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(raw["browser"]["engine"], "chrome");
        assert_eq!(raw["browser"]["max_running"], 5);
        // A bad engine is a config error, not a silent default.
        std::fs::write(&path, r#"{"browser":{"engine":"firefox"}}"#).unwrap();
        assert!(BrowserConfig::load(&path).is_err());
        assert_eq!(Engine::parse("CloakBrowser"), Some(Engine::Cloak));
        assert_eq!(Engine::parse("chromium"), Some(Engine::Chrome));
    }

    #[test]
    fn cloak_is_always_headed() {
        let cloak = BrowserConfig { engine: Engine::Cloak, headless: true, ..Default::default() };
        assert!(!headless_for(&cloak, true));
        assert!(!headless_for(&cloak, false), "never silently headless: the launch refuses instead");
        let chrome = BrowserConfig { engine: Engine::Chrome, headless: true, ..Default::default() };
        assert!(headless_for(&chrome, true));
        let chrome = BrowserConfig { headless: false, ..chrome };
        assert!(!headless_for(&chrome, true) && headless_for(&chrome, false));
    }

    #[test]
    fn cloak_args_carry_the_seed_and_no_headless() {
        let args = cloak_launch_args(Path::new("/h/browser-profiles/default"), 12345);
        for want in ["--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1", "--user-data-dir=/h/browser-profiles/default", "--fingerprint=12345", "--no-sandbox"] {
            assert!(args.iter().any(|a| a == want), "{want} in {args:?}");
        }
        assert!(!args.iter().any(|a| a.starts_with("--headless") || a.starts_with("--window-size") || a.starts_with("--disable-features")), "{args:?}");
        assert!(!args.iter().any(|a| a == "--enable-automation"));
        assert_eq!(args.last().unwrap(), "about:blank");
    }

    #[test]
    fn cloak_placement_is_hidden_unless_a_window_is_asked_for() {
        // UNI-299: a screen no longer matters for CloakBrowser — it goes to
        // the hidden display unless "Show window" (headless:false) is on and
        // there is a screen to show it on.
        let cloak = BrowserConfig { engine: Engine::Cloak, headless: true, ..Default::default() };
        assert_eq!(wanted_placement(&cloak, true), Placement::Hidden);
        assert_eq!(wanted_placement(&cloak, false), Placement::Hidden);
        let shown = BrowserConfig { headless: false, ..cloak.clone() };
        assert_eq!(wanted_placement(&shown, true), Placement::Window);
        assert_eq!(wanted_placement(&shown, false), Placement::Hidden, "no screen: hidden, never a failed window");
        let chrome = BrowserConfig { engine: Engine::Chrome, headless: true, ..Default::default() };
        assert_eq!(wanted_placement(&chrome, true), Placement::Headless);
        assert_eq!(wanted_placement(&BrowserConfig { headless: false, ..chrome }, true), Placement::Window);
    }

    #[test]
    fn cloak_args_per_placement() {
        let p = Path::new("/h/browser-profiles/default");
        let hidden = cloak_args_for(p, 7, Placement::Hidden);
        assert!(hidden.contains(&"--ozone-platform=x11".to_string()), "{hidden:?}");
        assert!(!hidden.iter().any(|a| a.starts_with("--headless")));
        assert_eq!(hidden.last().map(String::as_str), Some("about:blank"));
        let window = cloak_args_for(p, 7, Placement::Window);
        assert!(!window.iter().any(|a| a.starts_with("--ozone-platform") || a.starts_with("--headless")));
        let headless = cloak_args_for(p, 7, Placement::Headless);
        assert!(headless.contains(&"--headless=new".to_string()));
        assert_eq!(headless.last().map(String::as_str), Some("about:blank"));
    }

    #[tokio::test]
    async fn a_bogus_cloak_binary_falls_back_to_chrome_unstealthy() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let chrome = exe(&bin, "google-chrome");
        // A cloak_path that isn't executable: no download, straight to Chrome.
        let config = BrowserConfig {
            enabled: true,
            engine: Engine::Cloak,
            cloak_path: Some("/nonexistent/cloak/chrome".into()),
            path: Some(chrome.display().to_string()),
            ..Default::default()
        };
        let spec = plan_launch(dir.path(), "work", &config, true).await.unwrap();
        assert_eq!((spec.engine, spec.stealth, spec.headless), (Engine::Chrome, false, false));
        assert!(spec.stealth_reason.unwrap().contains("CloakBrowser is unavailable"));
        assert_eq!(spec.profile, dir.path().join("browser-profiles/work"));
        // A configured, executable CloakBrowser: stealth, seed written into the profile.
        let cloak = exe(&bin, "cloak-chrome");
        let config = BrowserConfig { cloak_path: Some(cloak.display().to_string()), ..config };
        let spec = plan_launch(dir.path(), "work", &config, true).await.unwrap();
        assert_eq!((spec.engine, spec.stealth, spec.info.name.as_str()), (Engine::Cloak, true, "CloakBrowser"));
        let seed = std::fs::read_to_string(dir.path().join("browser-profiles/work").join(cloak::SEED_FILE)).unwrap();
        assert!(spec.args.contains(&format!("--fingerprint={}", seed.trim())));
        // would_use says the same without launching.
        let (info, engine, stealth, _) = would_use(&config);
        assert_eq!((info.unwrap().path, engine, stealth), (cloak.display().to_string(), Engine::Cloak, true));
    }

    #[test]
    fn cdp_ports_and_client_counting() {
        assert_eq!(ws_port("ws://127.0.0.1:41234/devtools/browser/abc"), Some(41234));
        assert_eq!(ws_port("http://x"), None);
        // /proc/net/tcp: one listener, the host's own connection (both ends), one agent.
        let table = "  sl  local_address rem_address   st tx_queue rx_queue tr tm->when retrnsmt   uid  timeout inode\n\
   0: 0100007F:A132 00000000:0000 0A 00000000:00000000 00:00000000 00000000  1000        0 1\n\
   1: 0100007F:A132 0100007F:C350 01 00000000:00000000 00:00000000 00000000  1000        0 2\n\
   2: 0100007F:C350 0100007F:A132 01 00000000:00000000 00:00000000 00000000  1000        0 3\n\
   3: 0100007F:A132 0100007F:C351 01 00000000:00000000 00:00000000 00000000  1000        0 4\n";
        assert_eq!(established_to(table, 0xA132), 2);
        assert_eq!(established_to(table, 0xC350), 1);
    }

    /// UNI-296: `browser_takeover` / `browser_profiles` over the app protocol
    /// (no browser needs to run): off → refused; on → the gate flips, and
    /// `browser_status` reports it.
    #[tokio::test]
    async fn takeover_and_profiles_ops() {
        use unipi_proto::{Control, Request};
        let dir = tempfile::tempdir().unwrap();
        let paths = HostPaths::from_dir(dir.path());
        async fn run(paths: &HostPaths, req: Request) -> Result<Option<serde_json::Value>, AppErrorDto> {
            let (mut read, _) = tokio::io::duplex(64);
            let mut out = Vec::new();
            serve_request(paths, req, "dev-1", Some("Pixel"), &mut read, &mut out).await?;
            let (_, bytes) = crate::protocol::read_frame(&mut out.as_slice()).await.unwrap();
            match serde_json::from_slice::<Control>(&bytes).unwrap() {
                Control::Ok { data } => Ok(data),
                other => panic!("{other:?}"),
            }
        }
        // Off: refused like every browser op.
        let error = run(&paths, Request::BrowserTakeover { on: true, profile: None }).await.unwrap_err();
        assert_eq!(error.code, AppErrorCode::E_BROWSER_DISABLED);
        BrowserConfig { enabled: true, engine: Engine::Chrome, ..Default::default() }.save(&paths.config()).unwrap();
        profiles::ensure(dir.path(), "default", 1).unwrap();
        profiles::create(dir.path(), "shop", 2).unwrap();

        let data = run(&paths, Request::BrowserTakeover { on: true, profile: None }).await.unwrap().unwrap();
        let agent: BrowserAgent = serde_json::from_value(data).unwrap();
        assert_eq!(agent, BrowserAgent { connected: false, driving: false, taken_over: true });
        let manager = Manager::for_paths(&paths);
        assert_eq!(manager.gate("default").check("Input.dispatchMouseEvent"), Err(agent::TAKEN_OVER_MESSAGE));
        assert_eq!(manager.gate("shop").check("Input.dispatchMouseEvent"), Ok(()), "per profile");
        let status = run(&paths, Request::BrowserStatus).await.unwrap().unwrap();
        assert_eq!(status["agent"]["taken_over"], true);
        let data = run(&paths, Request::BrowserTakeover { on: false, profile: None }).await.unwrap().unwrap();
        assert_eq!(data["taken_over"], false);
        assert_eq!(manager.gate("default").check("Input.dispatchMouseEvent"), Ok(()));

        let data = run(&paths, Request::BrowserProfiles).await.unwrap().unwrap();
        let list: BrowserProfilesData = serde_json::from_value(data).unwrap();
        assert_eq!(list.default, "default");
        let names: Vec<&str> = list.profiles.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, ["default", "shop"]);
        assert!(list.profiles.iter().all(|p| !p.running));
    }
}
