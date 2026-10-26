//! Package manager subcommands (modes.md §1.2, runtime §15): `oino install <source>
//! [--project] [--yes]`, `remove|uninstall <id> [--project]`, `update <id>… | --extensions`,
//! `list`, `config`.
//!
//! Sources: a local package directory, a `.tar.zst` / `.tar.gz` / `.tgz` archive file, an
//! `https://` URL of such an archive, or `git+https://…[#ref]` (cloned with the system
//! `git`). Archives are unpacked with the system `tar`. A package is a directory with
//! `extension.json` (at the source root or in its only subdirectory); it is validated
//! (manifest, `apiVersion`, `checksums`) before it is copied to `<agent>/extensions/<id>/`
//! (or `.oino/extensions/<id>/` with `--project`) next to a `.source.json` record that
//! `update` re-installs from.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The record `update` re-installs from.
pub const SOURCE_FILE: &str = ".source.json";

/// Failure of a package command (printed as `Error: …`).
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PackageError {
    /// Bad command line.
    #[error("{0}")]
    Usage(String),
    /// The source could not be fetched or unpacked.
    #[error("{0}")]
    Fetch(String),
    /// The package is invalid.
    #[error("invalid package: {0}")]
    Invalid(String),
    /// The user declined, or `--yes` is missing without a terminal.
    #[error("{0}")]
    Declined(String),
    /// File system failure.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// Cause.
        source: std::io::Error,
    },
}

fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> PackageError {
    let context = context.into();
    move |source| PackageError::Io { context, source }
}

/// `.source.json`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRecord {
    /// Source as given (absolute for local paths).
    pub source: String,
    /// Installed version.
    pub version: String,
    /// Install time (RFC 3339, UTC).
    pub installed_at: String,
}

/// Where a command operates.
#[derive(Clone, Debug)]
pub struct PackageContext {
    /// Agent dir (`~/.oino`).
    pub agent_dir: PathBuf,
    /// Working directory (project scope, relative sources).
    pub cwd: PathBuf,
}

impl PackageContext {
    fn root(&self, project: bool) -> PathBuf {
        if project {
            self.cwd.join(".oino").join("extensions")
        } else {
            self.agent_dir.join("extensions")
        }
    }
}

/// Parsed flags of one subcommand.
#[derive(Debug, Default)]
struct Flags {
    project: bool,
    yes: bool,
    extensions: bool,
    positional: Vec<String>,
}

fn parse(args: &[String]) -> Result<Flags, PackageError> {
    let mut flags = Flags::default();
    for arg in args {
        match arg.as_str() {
            "--project" | "-l" | "--local" => flags.project = true,
            "--yes" | "-y" => flags.yes = true,
            "--extensions" => flags.extensions = true,
            other if other.starts_with('-') => {
                return Err(PackageError::Usage(format!("Unknown option: {other}")));
            }
            other => flags.positional.push(other.to_owned()),
        }
    }
    Ok(flags)
}

/// A fetched source: the package directory (and a temp dir to drop afterwards).
struct Fetched {
    root: PathBuf,
    _temp: Option<tempfile::TempDir>,
}

fn temp_dir(context: &PackageContext) -> Result<tempfile::TempDir, PackageError> {
    let base = context.agent_dir.join("cache").join("packages");
    std::fs::create_dir_all(&base).map_err(io(format!("creating {}", base.display())))?;
    tempfile::tempdir_in(&base).map_err(io("creating a temporary directory"))
}

fn run(program: &str, args: &[&str]) -> Result<(), PackageError> {
    let output = std::process::Command::new(program)
        .args(args)
        .output()
        .map_err(|e| PackageError::Fetch(format!("running {program}: {e}")))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(PackageError::Fetch(format!(
            "{program} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }
}

fn is_archive(name: &str) -> bool {
    [".tar.zst", ".tar.gz", ".tgz", ".tar"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

fn unpack(archive: &Path, into: &Path) -> Result<(), PackageError> {
    let archive = archive.to_string_lossy();
    let into = into.to_string_lossy();
    if archive.ends_with(".tar.zst") {
        run("tar", &["--zstd", "-xf", &archive, "-C", &into])
    } else {
        run("tar", &["-xf", &archive, "-C", &into])
    }
}

async fn download(url: &str, to: &Path) -> Result<(), PackageError> {
    let response = reqwest::get(url)
        .await
        .and_then(reqwest::Response::error_for_status)
        .map_err(|e| PackageError::Fetch(format!("downloading {url}: {e}")))?;
    let bytes = response
        .bytes()
        .await
        .map_err(|e| PackageError::Fetch(format!("downloading {url}: {e}")))?;
    std::fs::write(to, &bytes).map_err(io(format!("writing {}", to.display())))
}

/// The directory holding `extension.json`: `dir` itself or its only subdirectory.
fn package_root(dir: &Path) -> Result<PathBuf, PackageError> {
    if dir.join("extension.json").is_file() {
        return Ok(dir.to_path_buf());
    }
    let children: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(io(format!("reading {}", dir.display())))?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir() && p.join("extension.json").is_file())
        .collect();
    match children.as_slice() {
        [only] => Ok(only.clone()),
        [] => Err(PackageError::Invalid(format!(
            "no extension.json in {}",
            dir.display()
        ))),
        _ => Err(PackageError::Invalid(format!(
            "{} holds several packages; install one directory",
            dir.display()
        ))),
    }
}

/// Normalises a source (absolute local paths) for `.source.json`.
fn canonical_source(source: &str, cwd: &Path) -> String {
    if source.starts_with("https://") || source.starts_with("git+") {
        return source.to_owned();
    }
    let path = Path::new(source);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    path.display().to_string()
}

async fn fetch(source: &str, context: &PackageContext) -> Result<Fetched, PackageError> {
    if let Some(git) = source.strip_prefix("git+") {
        let (url, reference) = git
            .split_once('#')
            .map_or((git, None), |(u, r)| (u, Some(r)));
        let temp = temp_dir(context)?;
        let target = temp.path().join("repo");
        let target_text = target.to_string_lossy().into_owned();
        let mut args = vec!["clone", "--depth", "1"];
        if let Some(reference) = reference {
            args.extend(["--branch", reference]);
        }
        args.extend([url, target_text.as_str()]);
        run("git", &args)?;
        return Ok(Fetched {
            root: package_root(&target)?,
            _temp: Some(temp),
        });
    }
    if source.starts_with("https://") || source.starts_with("http://") {
        if !is_archive(source.split(['?', '#']).next().unwrap_or(source)) {
            return Err(PackageError::Usage(format!(
                "URL sources must point to a .tar.zst, .tar.gz or .tgz archive: {source}"
            )));
        }
        let temp = temp_dir(context)?;
        let name = source
            .split(['?', '#'])
            .next()
            .and_then(|s| s.rsplit('/').next())
            .unwrap_or("package.tar.gz");
        let archive = temp.path().join(name);
        download(source, &archive).await?;
        let unpacked = temp.path().join("unpacked");
        std::fs::create_dir_all(&unpacked).map_err(io("creating the unpack directory"))?;
        unpack(&archive, &unpacked)?;
        return Ok(Fetched {
            root: package_root(&unpacked)?,
            _temp: Some(temp),
        });
    }
    let path = PathBuf::from(canonical_source(source, &context.cwd));
    if path.is_dir() {
        return Ok(Fetched {
            root: package_root(&path)?,
            _temp: None,
        });
    }
    if path.is_file() && is_archive(&path.to_string_lossy()) {
        let temp = temp_dir(context)?;
        unpack(&path, temp.path())?;
        return Ok(Fetched {
            root: package_root(temp.path())?,
            _temp: Some(temp),
        });
    }
    Err(PackageError::Fetch(format!("Source not found: {source}")))
}

/// Non-default permission entries of a manifest, one `key: value` line each.
fn permission_lines(manifest: &oino_extension_api::manifest::Manifest) -> Vec<String> {
    let Ok(Value::Object(permissions)) = serde_json::to_value(&manifest.permissions) else {
        return Vec::new();
    };
    permissions
        .into_iter()
        .filter(|(_, v)| match v {
            Value::Bool(b) => *b,
            Value::Null => false,
            Value::Array(a) => !a.is_empty(),
            Value::Object(o) => o.values().any(|v| {
                !matches!(v, Value::Bool(false) | Value::Null) && v != &json!([]) && v != &json!({})
            }),
            Value::String(s) => !s.is_empty() && s != "none",
            Value::Number(_) => true,
        })
        .map(|(k, v)| format!("  {k}: {v}"))
        .collect()
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), PackageError> {
    std::fs::create_dir_all(to).map_err(io(format!("creating {}", to.display())))?;
    for entry in std::fs::read_dir(from).map_err(io(format!("reading {}", from.display())))? {
        let entry = entry.map_err(io(format!("reading {}", from.display())))?;
        let name = entry.file_name();
        if name == ".git" {
            continue;
        }
        let target = to.join(&name);
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            copy_dir(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .map_err(io(format!("copying {}", entry.path().display())))?;
        }
    }
    Ok(())
}

fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant), UTC.
    let z = i64::try_from(days).unwrap_or(0) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

fn confirm(question: &str) -> bool {
    let mut stderr = std::io::stderr().lock();
    let _ = write!(stderr, "{question} [y/N] ");
    let _ = stderr.flush();
    let mut line = String::new();
    std::io::stdin().lock().read_line(&mut line).is_ok()
        && matches!(line.trim().to_lowercase().as_str(), "y" | "yes")
}

/// Installs `source`; returns the installed id and version.
///
/// # Errors
///
/// Fetch, validation, confirmation and copy failures.
pub async fn install(
    source: &str,
    project: bool,
    yes: bool,
    context: &PackageContext,
    out: &mut dyn Write,
) -> Result<(String, String), PackageError> {
    let fetched = fetch(source, context).await?;
    let package = oino_wasm_host::packages::load_dir(&fetched.root, false)
        .map_err(|e| PackageError::Invalid(e.to_string()))?;
    let manifest = &package.manifest;
    let id = manifest.id.to_string();
    let lines = permission_lines(manifest);
    let _ = writeln!(out, "{} {} ({id})", manifest.name, manifest.version);
    if lines.is_empty() {
        let _ = writeln!(out, "Permissions: none");
    } else {
        let _ = writeln!(out, "Permissions:");
        for line in &lines {
            let _ = writeln!(out, "{line}");
        }
    }
    if !yes {
        if !std::io::stdin().is_terminal() {
            return Err(PackageError::Declined(
                "Pass --yes to install without a terminal".to_owned(),
            ));
        }
        if !confirm("Install?") {
            return Err(PackageError::Declined("Installation cancelled".to_owned()));
        }
    }
    let root = context.root(project);
    std::fs::create_dir_all(&root).map_err(io(format!("creating {}", root.display())))?;
    let target = root.join(&id);
    let staging = root.join(format!(".{id}.installing"));
    let old = root.join(format!(".{id}.old"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&old);
    copy_dir(&fetched.root, &staging)?;
    let record = SourceRecord {
        source: canonical_source(source, &context.cwd),
        version: manifest.version.clone(),
        installed_at: now_rfc3339(),
    };
    let text = serde_json::to_string_pretty(&record).unwrap_or_default();
    std::fs::write(staging.join(SOURCE_FILE), format!("{text}\n"))
        .map_err(io("writing the source record"))?;
    if target.exists() {
        std::fs::rename(&target, &old).map_err(io(format!("replacing {}", target.display())))?;
    }
    std::fs::rename(&staging, &target).map_err(io(format!("installing {}", target.display())))?;
    let _ = std::fs::remove_dir_all(&old);
    let _ = writeln!(
        out,
        "Installed {id} {} into {}",
        manifest.version,
        target.display()
    );
    Ok((id, manifest.version.clone()))
}

/// Removes an installed package.
///
/// # Errors
///
/// Unknown or bundled ids, I/O failures.
pub fn remove(id: &str, project: bool, context: &PackageContext) -> Result<PathBuf, PackageError> {
    let target = context.root(project).join(id);
    if !target.join("extension.json").is_file() {
        if oino_bundle::get(id).is_some() {
            return Err(PackageError::Usage(format!(
                "{id} is bundled and cannot be removed; disable it with \"disabledExtensions\" in settings.json"
            )));
        }
        return Err(PackageError::Usage(format!("Not installed: {id}")));
    }
    std::fs::remove_dir_all(&target).map_err(io(format!("removing {}", target.display())))?;
    Ok(target)
}

/// One installed or bundled package for `list`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Listed {
    /// Id.
    pub id: String,
    /// Version.
    pub version: String,
    /// `global`, `project` or `bundled`.
    pub scope: &'static str,
    /// Recorded source.
    pub source: Option<String>,
}

fn read_record(dir: &Path) -> Option<SourceRecord> {
    serde_json::from_str(&std::fs::read_to_string(dir.join(SOURCE_FILE)).ok()?).ok()
}

/// Installed packages (global, then project) and the bundled set.
#[must_use]
pub fn list(context: &PackageContext) -> Vec<Listed> {
    let mut out = Vec::new();
    for (project, scope) in [(false, "global"), (true, "project")] {
        let (packages, _) = oino_wasm_host::packages::discover(&[context.root(project)], false);
        out.extend(packages.into_iter().map(|p| Listed {
            id: p.manifest.id.to_string(),
            version: p.manifest.version.clone(),
            scope,
            source: p.root.as_deref().and_then(read_record).map(|r| r.source),
        }));
    }
    out.extend(oino_bundle::BUNDLED.iter().map(|b| Listed {
        id: b.id.to_owned(),
        version: b.version.to_owned(),
        scope: "bundled",
        source: None,
    }));
    out
}

/// Re-installs `ids` (every package with a source record when empty) from their records.
///
/// # Errors
///
/// The first failure; earlier packages stay updated.
pub async fn update(
    ids: &[String],
    project: bool,
    context: &PackageContext,
    out: &mut dyn Write,
) -> Result<usize, PackageError> {
    let root = context.root(project);
    let (packages, _) = oino_wasm_host::packages::discover(std::slice::from_ref(&root), false);
    let mut updated = 0;
    for package in packages {
        let id = package.manifest.id.to_string();
        if !ids.is_empty() && !ids.contains(&id) {
            continue;
        }
        let Some(record) = package.root.as_deref().and_then(read_record) else {
            let _ = writeln!(out, "Skipping {id}: no {SOURCE_FILE} (installed by hand)");
            continue;
        };
        let (_, version) = install(&record.source, project, true, context, out).await?;
        if version != record.version {
            let _ = writeln!(out, "Updated {id}: {} → {version}", record.version);
        }
        updated += 1;
    }
    for id in ids {
        if !root.join(id).join("extension.json").is_file() {
            return Err(PackageError::Usage(format!("Not installed: {id}")));
        }
    }
    Ok(updated)
}

/// Runs `oino <command> <args…>` for the package commands; returns the exit code.
pub async fn run_command(command: &str, args: &[String], context: &PackageContext) -> i32 {
    let mut stdout = std::io::stdout().lock();
    if args.iter().any(|a| a == "--help" || a == "-h") {
        let usage = match command {
            "install" => {
                "oino install <source> [--project|-l] [--yes|-y]\n  <source>: package dir, .tar.zst/.tar.gz archive, https:// archive URL, git+https://…[#ref]"
            }
            "remove" | "uninstall" => "oino remove <id> [--project|-l]",
            "update" => {
                "oino update [--check]            update Oino itself\noino update <id>… | --extensions  re-install packages from .source.json"
            }
            "list" => "oino list",
            _ => "oino config",
        };
        let _ = writeln!(stdout, "Usage: {usage}");
        return 0;
    }
    let result = match command {
        "install" => match parse(args) {
            Ok(flags) => match flags.positional.as_slice() {
                [source] => install(source, flags.project, flags.yes, context, &mut stdout)
                    .await
                    .map(drop),
                _ => Err(PackageError::Usage(
                    "Usage: oino install <source> [--project] [--yes]".to_owned(),
                )),
            },
            Err(error) => Err(error),
        },
        "remove" | "uninstall" => match parse(args) {
            Ok(flags) => match flags.positional.as_slice() {
                [id] => remove(id, flags.project, context).map(|path| {
                    let _ = writeln!(stdout, "Removed {id} ({})", path.display());
                }),
                _ => Err(PackageError::Usage(format!(
                    "Usage: oino {command} <id> [--project]"
                ))),
            },
            Err(error) => Err(error),
        },
        "update" => match parse(args) {
            Ok(flags) => update(&flags.positional, flags.project, context, &mut stdout)
                .await
                .map(|n| {
                    let _ = writeln!(stdout, "{n} package(s) updated");
                }),
            Err(error) => Err(error),
        },
        "list" => {
            for item in list(context) {
                let source = item.source.map(|s| format!("  {s}")).unwrap_or_default();
                let _ = writeln!(
                    stdout,
                    "{:<32} {:<14} {:<8}{source}",
                    item.id, item.version, item.scope
                );
            }
            Ok(())
        }
        "config" => {
            let _ = writeln!(
                stdout,
                "{}",
                context.agent_dir.join("settings.json").display()
            );
            let project = context.cwd.join(".oino").join("settings.json");
            if project.is_file() {
                let _ = writeln!(stdout, "{}", project.display());
            }
            Ok(())
        }
        other => Err(PackageError::Usage(format!("Unknown command: {other}"))),
    };
    match result {
        Ok(()) => 0,
        Err(error) => {
            crate::eprintln_line(&format!("Error: {error}"));
            1
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_and_sources() {
        let date = now_rfc3339();
        assert_eq!(date.len(), 20, "{date}");
        assert!(date.starts_with("20"));
        assert_eq!(
            canonical_source("pkg", Path::new("/w")),
            Path::new("/w/pkg").display().to_string()
        );
        assert_eq!(
            canonical_source("git+https://x/y#v1", Path::new("/w")),
            "git+https://x/y#v1"
        );
        assert!(is_archive("a.tar.zst") && is_archive("a.tgz") && !is_archive("a.zip"));
    }

    #[test]
    fn flags_parse() {
        let flags = parse(&["x".into(), "-y".into(), "--project".into()]).unwrap();
        assert!(flags.yes && flags.project);
        assert_eq!(flags.positional, ["x"]);
        assert!(parse(&["--bogus".into()]).is_err());
    }
}
