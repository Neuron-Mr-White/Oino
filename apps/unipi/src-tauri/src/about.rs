//! Settings → About → Components: what this install actually ships.
//!
//! The installers bundle everything UniPi Oino needs — the app, the
//! `unipi-oino-host` sidecar, the `oino` engine CLI and the extensions under
//! `share/oino/extensions` — so there is nothing to detect or install on a
//! fresh PC. This only *reports* those pieces (versions, where they are) and
//! names a problem when the install is genuinely broken (the host or the
//! extensions are missing), with the fix. It never touches the network, never
//! reads or writes any per-user state, and never offers to install anything.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::Serialize;
use unipi_proto::AppErrorDto;

/// How long a `--version` call may take before the row says "unknown".
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);

/// The engine CLI's file name next to the host.
const OINO_NAME: &str = if cfg!(windows) { "oino.exe" } else { "oino" };

/// One bundled binary: where it is and what `--version` said.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Binary {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

/// The bundled extensions.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Extensions {
    pub count: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dir: Option<String>,
}

/// A broken install: what is wrong and how to fix it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Problem {
    /// `host` | `extensions`.
    pub component: String,
    pub message: String,
    pub fix: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Components {
    pub app: String,
    pub host: Binary,
    pub oino: Binary,
    pub extensions: Extensions,
    pub problems: Vec<Problem>,
}

/// `unipi-host 1.0.0` / `oino 1.0.2` / `1.0.2` → `1.0.2`.
pub fn parse_version(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .flat_map(str::split_whitespace)
        .map(|word| word.trim_start_matches('v'))
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()) && word.contains('.'))
        .map(str::to_string)
}

/// Where the bundled extensions may be, for a host binary at `host_bin`: the
/// same exe-relative layouts the host itself searches (`share/oino/extensions`
/// for the .deb / AppImage, `Resources/extensions` in a macOS bundle,
/// `lib/<product>/extensions` for a Tauri resource dir), after
/// `OINO_DIST_EXTENSIONS`.
pub fn extension_dirs(host_bin: Option<&Path>, env_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = env_dir.map(Path::to_path_buf).into_iter().collect();
    let Some(bin_dir) = host_bin.and_then(Path::parent) else {
        return dirs;
    };
    if let Some(prefix) = bin_dir.parent() {
        dirs.push(prefix.join("share").join("oino").join("extensions"));
        dirs.push(prefix.join("extensions"));
        dirs.push(prefix.join("Resources").join("extensions"));
        if let Ok(entries) = std::fs::read_dir(prefix.join("lib")) {
            let mut found: Vec<PathBuf> = entries
                .filter_map(Result::ok)
                .map(|e| e.path().join("extensions"))
                .collect();
            found.sort();
            dirs.extend(found);
        }
    }
    dirs.push(bin_dir.join("extensions"));
    dirs
}

/// The extensions in `dir`: subdirectories with an `extension.json`.
pub fn count_extensions(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().join("extension.json").is_file())
                .count()
        })
        .unwrap_or(0)
}

/// The first candidate that holds any extension, and how many.
pub fn find_extensions(candidates: &[PathBuf]) -> Extensions {
    candidates
        .iter()
        .map(|dir| (dir, count_extensions(dir)))
        .find(|(_, count)| *count > 0)
        .map(|(dir, count)| Extensions {
            count,
            dir: Some(dir.display().to_string()),
        })
        .unwrap_or_default()
}

/// The problems a report shows: a missing host or no extensions. A missing
/// `oino` CLI is not one — the app talks to the host, never to the CLI.
pub fn problems(host: &Binary, extensions: &Extensions) -> Vec<Problem> {
    const REINSTALL: &str = "Reinstall Unipi from the latest release.";
    let mut out = Vec::new();
    if host.path.is_none() {
        out.push(Problem {
            component: "host".into(),
            message: "The agent host (unipi-oino-host) is missing from this install.".into(),
            fix: REINSTALL.into(),
        });
    }
    if host.path.is_some() && extensions.count == 0 {
        out.push(Problem {
            component: "extensions".into(),
            message: "No bundled extensions were found next to the host.".into(),
            fix: REINSTALL.into(),
        });
    }
    out
}

async fn version_of(bin: &Path) -> Option<String> {
    let out = tokio::time::timeout(
        VERSION_TIMEOUT,
        tokio::process::Command::new(bin)
            .arg("--version")
            .stdin(std::process::Stdio::null())
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

async fn binary(path: Option<PathBuf>) -> Binary {
    match path {
        Some(path) => Binary {
            version: version_of(&path).await,
            path: Some(path.display().to_string()),
        },
        None => Binary::default(),
    }
}

/// `about_components`: the app, host, engine and extension versions.
#[tauri::command]
pub async fn about_components(app: tauri::AppHandle) -> Result<Components, AppErrorDto> {
    let host_bin = crate::host_supervisor::host_bin(Some(&app));
    let oino_bin = host_bin
        .as_deref()
        .and_then(Path::parent)
        .map(|dir| dir.join(OINO_NAME))
        .filter(|p| p.is_file());
    let env_dir = std::env::var_os("OINO_DIST_EXTENSIONS")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let extensions = find_extensions(&extension_dirs(host_bin.as_deref(), env_dir.as_deref()));
    let (host, oino) = tokio::join!(binary(host_bin), binary(oino_bin));
    let problems = problems(&host, &extensions);
    Ok(Components {
        app: env!("CARGO_PKG_VERSION").to_string(),
        host,
        oino,
        extensions,
        problems,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_come_out_of_any_banner() {
        assert_eq!(
            parse_version("unipi-host 1.0.0\n").as_deref(),
            Some("1.0.0")
        );
        assert_eq!(parse_version("1.0.2").as_deref(), Some("1.0.2"));
        assert_eq!(
            parse_version("oino v1.2.3-alpha.1").as_deref(),
            Some("1.2.3-alpha.1")
        );
        assert_eq!(parse_version("no version here"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn the_deb_layout_counts_its_bundled_extensions() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join("usr/bin");
        let share = root.path().join("usr/share/oino/extensions");
        std::fs::create_dir_all(&bin).unwrap();
        for name in ["unipi.core", "unipi.kanboard", "oino.anthropic"] {
            std::fs::create_dir_all(share.join(name)).unwrap();
            std::fs::write(share.join(name).join("extension.json"), "{}").unwrap();
        }
        // A stray dir without a manifest is not an extension.
        std::fs::create_dir_all(share.join("stray")).unwrap();
        let host = bin.join("unipi-oino-host");
        let found = find_extensions(&extension_dirs(Some(&host), None));
        assert_eq!(found.count, 3);
        assert_eq!(
            found.dir.as_deref(),
            Some(share.display().to_string().as_str())
        );
    }

    #[test]
    fn the_env_override_wins() {
        let root = tempfile::tempdir().unwrap();
        let env = root.path().join("dist");
        std::fs::create_dir_all(env.join("unipi.core")).unwrap();
        std::fs::write(env.join("unipi.core/extension.json"), "{}").unwrap();
        let dirs = extension_dirs(None, Some(&env));
        assert_eq!(find_extensions(&dirs).count, 1);
    }

    #[test]
    fn only_a_missing_host_or_no_extensions_is_a_problem() {
        let host = Binary {
            version: Some("1.0.0".into()),
            path: Some("/usr/bin/unipi-oino-host".into()),
        };
        let some = Extensions {
            count: 31,
            dir: Some("/usr/share/oino/extensions".into()),
        };
        assert!(problems(&host, &some).is_empty());
        let none = Extensions::default();
        assert_eq!(problems(&host, &none)[0].component, "extensions");
        let missing = problems(&Binary::default(), &none);
        assert_eq!(missing.len(), 1, "a missing host says only that");
        assert_eq!(missing[0].component, "host");
        assert!(missing[0].fix.contains("Reinstall"));
        assert!(!missing[0].fix.to_lowercase().contains("npm"));
    }
}
