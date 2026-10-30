//! Finding the kanboard daemon and its binary.
//!
//! - `daemon.json` lives in the kanboard home: `UNIPI_KANBOARD_HOME`, else
//!   `~/.unipi/kanboard` (same rule as `crates/kanboard/src/store.rs`).
//! - The binary: `UNIPI_KANBOARD_BIN` (authoritative when set, like the pi
//!   extension's `resolveBinary`), then `unipi-kanboard` on `PATH`, then the
//!   pi-installed platform package
//!   `~/.pi/agent/npm/node_modules/@pi-unipi/kanboard-<platform>/bin/unipi-kanboard`.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use serde::Deserialize;

/// `daemon.json` as written by `unipi-kanboard serve` (`DaemonInfo`).
/// Unknown fields (e.g. `startedAt`) are ignored.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DaemonInfo {
    pub pid: u32,
    pub port: u16,
    #[serde(default)]
    pub version: Option<String>,
    /// Bind address; entries from older daemons omit it (loopback).
    #[serde(default = "default_host")]
    pub host: String,
    /// Access token; present for non-loopback binds or `--require-auth`.
    #[serde(default)]
    pub token: Option<String>,
}

fn default_host() -> String {
    "127.0.0.1".into()
}

impl DaemonInfo {
    /// Parses `daemon.json` text; `None` for anything unusable.
    pub fn parse(text: &str) -> Option<DaemonInfo> {
        let info: DaemonInfo = serde_json::from_str(text).ok()?;
        (info.port != 0).then_some(info)
    }

    /// Reads `<home>/daemon.json`.
    pub fn read(home: &Path) -> Option<DaemonInfo> {
        DaemonInfo::parse(&std::fs::read_to_string(home.join("daemon.json")).ok()?)
    }

    /// The `host:port` the host dials. Wildcard binds are reached on
    /// loopback; a specific bind address (e.g. a tailnet IP) is dialed as
    /// recorded.
    pub fn authority(&self) -> String {
        let host = self
            .host
            .trim()
            .trim_start_matches('[')
            .trim_end_matches(']');
        let host = match host {
            "" | "0.0.0.0" | "localhost" => "127.0.0.1",
            "::" | "::1" => "[::1]",
            other => other,
        };
        if host.contains(':') && !host.starts_with('[') {
            format!("[{host}]:{}", self.port)
        } else {
            format!("{host}:{}", self.port)
        }
    }

    /// The bearer token to send, if the daemon requires one.
    pub fn bearer(&self) -> Option<&str> {
        self.token.as_deref().filter(|token| !token.is_empty())
    }
}

/// The npm platform key (`process.platform-process.arch`) for this build,
/// e.g. `linux-x64`, `darwin-arm64`.
pub fn platform_key() -> String {
    let os = match std::env::consts::OS {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x64",
        "aarch64" => "arm64",
        "x86" => "ia32",
        other => other,
    };
    format!("{os}-{arch}")
}

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "unipi-kanboard.exe"
    } else {
        "unipi-kanboard"
    }
}

/// Inputs for the binary lookup, injectable for tests.
#[derive(Debug, Clone, Default)]
pub struct BinaryLookup {
    /// `UNIPI_KANBOARD_BIN`.
    pub override_bin: Option<PathBuf>,
    /// `PATH`.
    pub path_env: Option<OsString>,
    /// `$HOME` (for `~/.pi/agent/npm/...`).
    pub home: Option<PathBuf>,
    /// npm platform key, e.g. `linux-x64`.
    pub platform: String,
}

impl BinaryLookup {
    pub fn from_env() -> Self {
        BinaryLookup {
            override_bin: std::env::var_os("UNIPI_KANBOARD_BIN")
                .filter(|value| !value.to_string_lossy().trim().is_empty())
                .map(PathBuf::from),
            path_env: std::env::var_os("PATH"),
            home: home_dir(),
            platform: platform_key(),
        }
    }

    /// The pi-installed platform package binary path (may not exist).
    pub fn pi_package_path(&self) -> Option<PathBuf> {
        Some(
            self.home
                .as_ref()?
                .join(".pi/agent/npm/node_modules/@pi-unipi")
                .join(format!("kanboard-{}", self.platform))
                .join("bin")
                .join(exe_name()),
        )
    }

    /// First hit wins; `None` = not installed.
    pub fn find(&self) -> Option<PathBuf> {
        if let Some(explicit) = &self.override_bin {
            return is_runnable(explicit).then(|| explicit.clone());
        }
        if let Some(path_env) = &self.path_env {
            for dir in std::env::split_paths(path_env) {
                if dir.as_os_str().is_empty() {
                    continue;
                }
                let candidate = dir.join(exe_name());
                if is_runnable(&candidate) {
                    return Some(candidate);
                }
            }
        }
        self.pi_package_path().filter(|path| is_runnable(path))
    }
}

pub fn home_dir() -> Option<PathBuf> {
    crate::fs::home_dir_opt()
}

/// `UNIPI_KANBOARD_HOME`, else `~/.unipi/kanboard`.
pub fn kanboard_home() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("UNIPI_KANBOARD_HOME") {
        if !home.to_string_lossy().trim().is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    Some(home_dir()?.join(".unipi").join("kanboard"))
}

fn is_runnable(path: &Path) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    if !metadata.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        metadata.permissions().mode() & 0o111 != 0
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_daemon_json_layouts() {
        let full = r#"{
  "pid": 1431480,
  "port": 45895,
  "version": "3.0.0-alpha.30",
  "startedAt": "2026-10-07T07:03:29Z",
  "host": "127.0.0.1"
}"#;
        let info = DaemonInfo::parse(full).unwrap();
        assert_eq!(info.pid, 1431480);
        assert_eq!(info.port, 45895);
        assert_eq!(info.version.as_deref(), Some("3.0.0-alpha.30"));
        assert_eq!(info.bearer(), None);
        assert_eq!(info.authority(), "127.0.0.1:45895");

        // Older daemons: no host field → loopback.
        let old = DaemonInfo::parse(r#"{"pid":1,"port":7788,"version":"1"}"#).unwrap();
        assert_eq!(old.authority(), "127.0.0.1:7788");

        let remote =
            DaemonInfo::parse(r#"{"pid":1,"port":9,"version":"1","host":"0.0.0.0","token":"abc"}"#)
                .unwrap();
        assert_eq!(remote.authority(), "127.0.0.1:9");
        assert_eq!(remote.bearer(), Some("abc"));
        let tailnet =
            DaemonInfo::parse(r#"{"pid":1,"port":9,"host":"100.64.1.2","token":""}"#).unwrap();
        assert_eq!(tailnet.authority(), "100.64.1.2:9");
        assert_eq!(tailnet.bearer(), None);
        let v6 = DaemonInfo::parse(r#"{"pid":1,"port":9,"host":"::"}"#).unwrap();
        assert_eq!(v6.authority(), "[::1]:9");
        let v6_specific = DaemonInfo::parse(r#"{"pid":1,"port":9,"host":"fd00::1"}"#).unwrap();
        assert_eq!(v6_specific.authority(), "[fd00::1]:9");

        assert!(DaemonInfo::parse("").is_none());
        assert!(DaemonInfo::parse("{").is_none());
        assert!(DaemonInfo::parse(r#"{"pid":1}"#).is_none());
        assert!(DaemonInfo::parse(r#"{"pid":1,"port":0}"#).is_none());
    }

    #[test]
    fn reads_daemon_json_from_home() {
        let dir = tempfile::tempdir().unwrap();
        assert!(DaemonInfo::read(dir.path()).is_none());
        std::fs::write(dir.path().join("daemon.json"), r#"{"pid":2,"port":3}"#).unwrap();
        assert_eq!(DaemonInfo::read(dir.path()).unwrap().port, 3);
    }

    fn fake_bin(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "#!/bin/sh\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    #[test]
    fn binary_lookup_order() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let home = root.join("home");
        let path_a = root.join("a");
        let path_b = root.join("b");
        std::fs::create_dir_all(&path_a).unwrap();
        let explicit = root.join("explicit/unipi-kanboard");
        let on_path = path_b.join(exe_name());
        let packaged = home
            .join(".pi/agent/npm/node_modules/@pi-unipi/kanboard-linux-x64/bin")
            .join(exe_name());
        let mut lookup = BinaryLookup {
            override_bin: None,
            path_env: Some(std::env::join_paths([&path_a, &path_b]).unwrap()),
            home: Some(home.clone()),
            platform: "linux-x64".into(),
        };
        assert_eq!(lookup.pi_package_path().unwrap(), packaged);
        assert_eq!(lookup.find(), None, "nothing installed");

        fake_bin(&packaged);
        assert_eq!(lookup.find(), Some(packaged.clone()), "pi package last");

        fake_bin(&on_path);
        assert_eq!(
            lookup.find(),
            Some(on_path.clone()),
            "PATH beats pi package"
        );

        lookup.override_bin = Some(explicit.clone());
        assert_eq!(lookup.find(), None, "a missing override is authoritative");
        fake_bin(&explicit);
        assert_eq!(lookup.find(), Some(explicit), "override beats PATH");

        // Platform key selects the package directory.
        let other = BinaryLookup {
            override_bin: None,
            path_env: None,
            home: Some(home),
            platform: "darwin-arm64".into(),
        };
        assert_eq!(other.find(), None);
    }

    #[cfg(unix)]
    #[test]
    fn non_executable_files_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join(exe_name());
        std::fs::write(&bin, "x").unwrap();
        let lookup = BinaryLookup {
            path_env: Some(dir.path().as_os_str().to_owned()),
            ..BinaryLookup::default()
        };
        assert_eq!(lookup.find(), None);
    }

    #[test]
    fn platform_key_matches_npm_names() {
        let key = platform_key();
        assert!(
            [
                "linux-x64",
                "linux-arm64",
                "darwin-arm64",
                "darwin-x64",
                "win32-x64"
            ]
            .contains(&key.as_str())
                || !key.is_empty()
        );
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        assert_eq!(key, "linux-x64");
    }
}
