//! Engine resolution (browser-control.md §4.1): `CloakBrowser` cache, system Chrome-family
//! candidates (unipi `CANDIDATES` order), Playwright cache. No disk crawl.

use std::path::{Path, PathBuf};

use crate::config::{BrowserSettings, EngineChoice};

/// Engine kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineKind {
    /// Chrome-family system browser.
    Chrome,
    /// `CloakBrowser` (stealth).
    Cloak,
    /// Explicit path.
    Path,
}

impl EngineKind {
    /// Wire name.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Chrome => "chrome",
            Self::Cloak => "cloak",
            Self::Path => "path",
        }
    }
}

/// A resolved browser executable.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Engine {
    /// Kind.
    pub kind: EngineKind,
    /// Executable.
    pub executable: PathBuf,
    /// Product name shown to the user.
    pub product: String,
}

/// Why no engine resolved.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineMissing {
    /// No browser found.
    NoEngine,
    /// `engine = cloak` without an installed build.
    CloakNotInstalled,
}

/// Resolves the engine for `settings` with the process environment.
///
/// # Errors
///
/// No engine resolves.
pub fn resolve(settings: &BrowserSettings) -> Result<Engine, EngineMissing> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    resolve_with(
        settings,
        &env,
        std::env::var_os("PATH").as_ref(),
        dirs::home_dir().as_deref(),
        candidates(),
    )
}

fn resolve_with(
    settings: &BrowserSettings,
    env: &dyn Fn(&str) -> Option<PathBuf>,
    path_var: Option<&std::ffi::OsString>,
    home: Option<&Path>,
    candidates: &[&str],
) -> Result<Engine, EngineMissing> {
    let explicit = settings
        .executable_path
        .clone()
        .filter(|p| p.is_absolute() && is_executable(p));
    match settings.engine {
        EngineChoice::Path => explicit
            .map(|executable| Engine {
                kind: EngineKind::Path,
                product: product_of(&executable),
                executable,
            })
            .ok_or(EngineMissing::NoEngine),
        EngineChoice::Cloak => cloak(home).ok_or(EngineMissing::CloakNotInstalled),
        EngineChoice::Auto => cloak(home)
            .or_else(|| chrome(explicit, env, path_var, home, candidates))
            .ok_or(EngineMissing::NoEngine),
        EngineChoice::Chrome => {
            chrome(explicit, env, path_var, home, candidates).ok_or(EngineMissing::NoEngine)
        }
    }
}

fn chrome(
    explicit: Option<PathBuf>,
    env: &dyn Fn(&str) -> Option<PathBuf>,
    path_var: Option<&std::ffi::OsString>,
    home: Option<&Path>,
    candidates: &[&str],
) -> Option<Engine> {
    let from_env = ["CHROME_PATH", "CHROME_EXECUTABLE"]
        .iter()
        .filter_map(|k| env(k))
        .find(|p| is_executable(p));
    let found = explicit
        .or(from_env)
        .or_else(|| candidates.iter().find_map(|c| locate(c, path_var, home)))
        .or_else(|| playwright(home))?;
    Some(Engine {
        kind: EngineKind::Chrome,
        product: product_of(&found),
        executable: found,
    })
}

/// Installed `CloakBrowser` builds: Oino's cache, then the wrapper's `~/.cloakbrowser/`.
fn cloak(home: Option<&Path>) -> Option<Engine> {
    let home = home?;
    let roots = [
        home.join(".cache/oino/cloakbrowser"),
        home.join(".cloakbrowser"),
    ];
    let binary = if cfg!(target_os = "macos") {
        "Chromium.app/Contents/MacOS/Chromium"
    } else if cfg!(windows) {
        "chrome.exe"
    } else {
        "chrome"
    };
    for root in roots {
        let Ok(dir) = std::fs::read_dir(&root) else {
            continue;
        };
        let mut versions: Vec<PathBuf> = dir
            .filter_map(Result::ok)
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        versions.sort();
        if let Some(exe) = versions
            .iter()
            .rev()
            .map(|v| v.join(binary))
            .find(|p| is_executable(p))
        {
            return Some(Engine {
                kind: EngineKind::Cloak,
                executable: exe,
                product: "CloakBrowser".into(),
            });
        }
    }
    None
}

fn playwright(home: Option<&Path>) -> Option<PathBuf> {
    let root = if cfg!(target_os = "macos") {
        home?.join("Library/Caches/ms-playwright")
    } else if cfg!(windows) {
        home?.join("AppData/Local/ms-playwright")
    } else {
        home?.join(".cache/ms-playwright")
    };
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root)
        .ok()?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("chromium-"))
        })
        .collect();
    dirs.sort();
    let rel = if cfg!(target_os = "macos") {
        "chrome-mac/Chromium.app/Contents/MacOS/Chromium"
    } else if cfg!(windows) {
        "chrome-win/chrome.exe"
    } else {
        "chrome-linux64/chrome"
    };
    dirs.iter()
        .rev()
        .flat_map(|d| [d.join(rel), d.join("chrome-linux/chrome")])
        .find(|p| is_executable(p))
}

fn candidates() -> &'static [&'static str] {
    if cfg!(target_os = "macos") {
        &[
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "~/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Google Chrome Beta.app/Contents/MacOS/Google Chrome Beta",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
            "~/Applications/Chromium.app/Contents/MacOS/Chromium",
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Brave Browser.app/Contents/MacOS/Brave Browser",
        ]
    } else if cfg!(windows) {
        &[
            "%ProgramFiles%\\Google\\Chrome\\Application\\chrome.exe",
            "%ProgramFiles(x86)%\\Google\\Chrome\\Application\\chrome.exe",
            "%LocalAppData%\\Google\\Chrome\\Application\\chrome.exe",
            "%LocalAppData%\\Chromium\\Application\\chrome.exe",
            "%ProgramFiles(x86)%\\Microsoft\\Edge\\Application\\msedge.exe",
            "%ProgramFiles%\\Microsoft\\Edge\\Application\\msedge.exe",
            "%ProgramFiles%\\BraveSoftware\\Brave-Browser\\Application\\brave.exe",
        ]
    } else {
        &[
            "google-chrome",
            "google-chrome-stable",
            "google-chrome-beta",
            "google-chrome-unstable",
            "chromium",
            "chromium-browser",
            "/snap/bin/chromium",
            "/var/lib/flatpak/exports/bin/org.chromium.Chromium",
            "microsoft-edge",
            "microsoft-edge-stable",
            "brave-browser",
            "brave",
        ]
    }
}

fn locate(
    candidate: &str,
    path_var: Option<&std::ffi::OsString>,
    home: Option<&Path>,
) -> Option<PathBuf> {
    if let Some(rest) = candidate.strip_prefix("~/") {
        return Some(home?.join(rest)).filter(|p| is_executable(p));
    }
    if candidate.contains('%') {
        let expanded = expand_windows(candidate)?;
        return Some(expanded).filter(|p| is_executable(p));
    }
    let p = Path::new(candidate);
    if p.is_absolute() {
        return Some(p.to_path_buf()).filter(|p| is_executable(p));
    }
    std::env::split_paths(path_var?)
        .map(|d| d.join(candidate))
        .find(|p| is_executable(p))
}

fn expand_windows(s: &str) -> Option<PathBuf> {
    let mut out = String::new();
    let mut parts = s.split('%');
    out.push_str(parts.next()?);
    let mut var = true;
    for part in parts {
        if var {
            out.push_str(&std::env::var(part).ok()?);
        } else {
            out.push_str(part);
        }
        var = !var;
    }
    Some(PathBuf::from(out))
}

fn product_of(path: &Path) -> String {
    let s = path.to_string_lossy().to_lowercase();
    let product = if s.contains("cloak") {
        "CloakBrowser"
    } else if s.contains("edge") {
        "Microsoft Edge"
    } else if s.contains("brave") {
        "Brave"
    } else if s.contains("google") || s.contains("chrome.app") || s.ends_with("chrome.exe") {
        "Google Chrome"
    } else {
        "Chromium"
    };
    product.into()
}

fn is_executable(path: &Path) -> bool {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        path.metadata()
            .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }
    #[cfg(not(unix))]
    {
        path.is_file()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, reason = "tests")]
mod tests {
    use super::*;

    fn exe(dir: &Path, rel: &str) -> PathBuf {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        p
    }

    const CANDS: &[&str] = &["google-chrome", "chromium"];

    #[cfg(target_os = "linux")]
    #[test]
    fn resolution_order() {
        let home = tempfile::tempdir().unwrap();
        let bin = tempfile::tempdir().unwrap();
        let none = |_: &str| None;
        let path = Some(bin.path().as_os_str().to_owned());
        let s = BrowserSettings::default();
        assert_eq!(
            resolve_with(&s, &none, path.as_ref(), Some(home.path()), CANDS),
            Err(EngineMissing::NoEngine)
        );
        let pw = exe(
            home.path(),
            ".cache/ms-playwright/chromium-1248/chrome-linux64/chrome",
        );
        let e = resolve_with(&s, &none, path.as_ref(), Some(home.path()), CANDS).unwrap();
        assert_eq!((e.kind, e.executable.clone()), (EngineKind::Chrome, pw));
        let sys = exe(bin.path(), "chromium");
        let e = resolve_with(&s, &none, path.as_ref(), Some(home.path()), CANDS).unwrap();
        assert_eq!(e.executable, sys);
        assert_eq!(e.product, "Chromium");
        let cloak = exe(home.path(), ".cloakbrowser/chromium-146.0/chrome");
        let e = resolve_with(&s, &none, path.as_ref(), Some(home.path()), CANDS).unwrap();
        assert_eq!((e.kind, e.executable), (EngineKind::Cloak, cloak));
        let chrome_only = BrowserSettings {
            engine: EngineChoice::Chrome,
            ..BrowserSettings::default()
        };
        let e = resolve_with(&chrome_only, &none, path.as_ref(), Some(home.path()), CANDS).unwrap();
        assert_eq!(e.executable, sys);
        let empty = tempfile::tempdir().unwrap();
        let cloak_only = BrowserSettings {
            engine: EngineChoice::Cloak,
            ..BrowserSettings::default()
        };
        assert_eq!(
            resolve_with(&cloak_only, &none, path.as_ref(), Some(empty.path()), CANDS),
            Err(EngineMissing::CloakNotInstalled)
        );
    }
}
