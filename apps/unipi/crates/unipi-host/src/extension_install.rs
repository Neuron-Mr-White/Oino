//! Settings › Extensions › "Install extension…" (`host_cmd install_extension` /
//! `remove_extension`): installs a package that is already **on this host** — a folder with
//! `extension.json` + its component (or its only subfolder holding them), or a `.zip` of
//! such a folder (the phone uploads it through the blob channel first, so the zip is
//! detected by its magic bytes, never by its name) — into `<agent dir>/extensions/<id>/`.
//!
//! The package is validated exactly like core's discovery does (`Manifest::load_package`:
//! schema, `apiVersion`, semver, the component and resources exist, declared `checksums`
//! match) before anything is copied. Same-id rules follow `oino_core::extensions::sources`:
//! a user package shadows a bundled (share dir) package of the same id in place, so a
//! bundled id is **not** refused — the reply carries a warning instead (and says when the
//! user copy is not newer). An existing user package of the same id is replaced only with
//! `replace: true` (the first call answers `{installed: false, confirm: …}` so the app can
//! ask). The copy is staged next to the target and swapped in with renames, like
//! `oino install`, and a `.source.json` record is written for folder sources so
//! `oino update` can re-install from it.
//!
//! The host cannot load a new package into its running services (core has no extension
//! reload path; the hub caches one services set per project), so every success answers
//! `needsRestart: true` and the app offers "Restart agent host".

use std::io::Read;
use std::path::{Path, PathBuf};

use oino_extension_api::manifest::{MANIFEST_FILE, Manifest};
use serde::Serialize;

/// The record `oino update` re-installs from (same name and shape as the CLI's).
pub const SOURCE_FILE: &str = ".source.json";

/// Largest total size a zip may expand to (a package is a wasm component plus resources).
const MAX_UNPACKED: u64 = 1024 * 1024 * 1024;
/// Most entries a zip may hold.
const MAX_ENTRIES: usize = 20_000;

/// Failure of an install or remove.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// The source is not a usable package (missing, not a folder/zip, bad manifest,
    /// checksum mismatch …).
    #[error("{0}")]
    Invalid(String),
    /// Remove: nothing user-installed under that id.
    #[error("{0}")]
    NotInstalled(String),
    /// File system failure.
    #[error("{context}: {source}")]
    Io {
        /// What was being done.
        context: String,
        /// Cause.
        source: std::io::Error,
    },
}

fn io(context: impl Into<String>) -> impl FnOnce(std::io::Error) -> InstallError {
    let context = context.into();
    move |source| InstallError::Io { context, source }
}

/// A same-id user package already installed: the app asks before replacing it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Conflict {
    /// Extension id.
    pub id: String,
    /// Display name of the new package.
    pub name: String,
    /// Version of the new package.
    pub version: String,
    /// Version of the installed one (when its manifest still reads).
    pub installed_version: Option<String>,
}

/// The `install_extension` reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstallOutcome {
    /// `false` when a confirmation is needed first (see [`InstallOutcome::confirm`]).
    pub installed: bool,
    /// Set when a same-id user package exists and `replace` was not given.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Conflict>,
    /// Extension id.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Version.
    pub version: String,
    /// Where it was installed (`<agent dir>/extensions/<id>`).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    /// A previous user package of the same id was replaced (its version, when known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub replaced: Option<String>,
    /// Non-fatal notes (shadowing a bundled package, an older version …).
    pub warnings: Vec<String>,
    /// The settings namespaces the manifest declares statically (`ui.settings`), so the app
    /// knows which page to open once the package is live.
    pub settings_namespaces: Vec<String>,
    /// The running host keeps its loaded extensions until it restarts.
    pub needs_restart: bool,
}

/// The `remove_extension` reply.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveOutcome {
    /// Extension id.
    pub id: String,
    /// The removed folder.
    pub path: String,
    /// A bundled package of the same id takes over again after the restart.
    pub bundled_remains: bool,
    /// The running host keeps the package loaded until it restarts.
    pub needs_restart: bool,
}

/// `<agent dir>/extensions`.
#[must_use]
pub fn user_root(agent_dir: &Path) -> PathBuf {
    agent_dir.join("extensions")
}

/// The bundled (share dir) package of `id`, when one ships with the app.
fn bundled_manifest(id: &str, dist_dirs: &[PathBuf]) -> Option<Manifest> {
    dist_dirs
        .iter()
        .map(|dir| dir.join(id))
        .find(|dir| dir.join(MANIFEST_FILE).is_file())
        .and_then(|dir| read_manifest(&dir))
}

/// The manifest at `dir` (parse only — an installed package is not re-verified here).
fn read_manifest(dir: &Path) -> Option<Manifest> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_FILE)).ok()?;
    serde_json::from_str(&text).ok()
}

/// The user-dir folder of package `id`: `<agent dir>/extensions/<id>`, else a hand-copied
/// folder of another name whose manifest declares `id` (discovery keys by manifest id).
#[must_use]
pub fn user_package_dir(id: &str, agent_dir: &Path) -> Option<PathBuf> {
    if !valid_id(id) {
        return None;
    }
    let root = user_root(agent_dir);
    let direct = root.join(id);
    if direct.join(MANIFEST_FILE).is_file() {
        return Some(direct);
    }
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(&root)
        .ok()?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.join(MANIFEST_FILE).is_file()
                && !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
        })
        .collect();
    dirs.sort();
    dirs.into_iter()
        .find(|dir| read_manifest(dir).is_some_and(|m| m.id.as_str() == id))
}

/// Is `id` installed in the user dir (`<agent dir>/extensions`)?
#[must_use]
pub fn is_user_installed(id: &str, agent_dir: &Path) -> bool {
    user_package_dir(id, agent_dir).is_some()
}

fn valid_id(id: &str) -> bool {
    oino_extension_api::types::ExtensionId::new(id.to_owned()).is_ok()
}

/// A scratch folder inside the user root (same file system as the target, so the final
/// move is a rename), removed on drop.
struct Scratch(PathBuf);

impl Scratch {
    fn new(root: &Path, label: &str) -> Result<Self, InstallError> {
        std::fs::create_dir_all(root).map_err(io(format!("creating {}", root.display())))?;
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_nanos());
        let dir = root.join(format!(".{label}-{}-{nanos}", std::process::id()));
        std::fs::create_dir_all(&dir).map_err(io(format!("creating {}", dir.display())))?;
        Ok(Self(dir))
    }
}

impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Is `path` a zip archive (local file header magic)?
fn is_zip(path: &Path) -> bool {
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .is_ok()
        && magic == *b"PK\x03\x04"
}

/// Unpacks `archive` into `dest`, refusing entries that escape it, symlinks, and archives
/// that expand past [`MAX_UNPACKED`] / [`MAX_ENTRIES`].
fn unzip(archive: &Path, dest: &Path) -> Result<(), InstallError> {
    let file = std::fs::File::open(archive).map_err(io(format!("opening {}", archive.display())))?;
    let mut zip = zip::ZipArchive::new(file)
        .map_err(|e| InstallError::Invalid(format!("not a readable .zip: {e}")))?;
    if zip.len() > MAX_ENTRIES {
        return Err(InstallError::Invalid(format!(
            "the .zip holds {} entries (at most {MAX_ENTRIES})",
            zip.len()
        )));
    }
    let mut total: u64 = 0;
    for index in 0..zip.len() {
        let mut entry = zip
            .by_index(index)
            .map_err(|e| InstallError::Invalid(format!("damaged .zip: {e}")))?;
        let Some(relative) = entry.enclosed_name() else {
            return Err(InstallError::Invalid(format!(
                "the .zip entry `{}` points outside the package",
                entry.name()
            )));
        };
        if entry.is_symlink() {
            continue;
        }
        let target = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target).map_err(io(format!("creating {}", target.display())))?;
            continue;
        }
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED {
            return Err(InstallError::Invalid(
                "the .zip expands past 1 GiB — that is not an extension package".to_owned(),
            ));
        }
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(io(format!("creating {}", parent.display())))?;
        }
        let mut out =
            std::fs::File::create(&target).map_err(io(format!("writing {}", target.display())))?;
        // `take` guards against a lying size header.
        let mut limited = (&mut entry).take(MAX_UNPACKED);
        std::io::copy(&mut limited, &mut out).map_err(io(format!("unpacking {}", target.display())))?;
    }
    Ok(())
}

/// The package root inside `dir`: `dir` itself when it holds `extension.json`, else its
/// only subfolder holding one (a zip of the folder, or a release folder around it).
fn package_root(dir: &Path) -> Result<PathBuf, InstallError> {
    if dir.join(MANIFEST_FILE).is_file() {
        return Ok(dir.to_path_buf());
    }
    let children: Vec<PathBuf> = std::fs::read_dir(dir)
        .map_err(io(format!("reading {}", dir.display())))?
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_dir()
                && !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.') || n == "__MACOSX")
        })
        .collect();
    match children.as_slice() {
        [only] if only.join(MANIFEST_FILE).is_file() => Ok(only.clone()),
        _ => Err(InstallError::Invalid(format!(
            "no {MANIFEST_FILE} in {} — pick the extension's folder (extension.json + component.wasm) or a .zip of it",
            dir.display()
        ))),
    }
}

/// Copies a package tree (regular files and folders; symlinks, `.git` and an old
/// `.source.json` are skipped).
fn copy_dir(from: &Path, to: &Path) -> Result<(), InstallError> {
    std::fs::create_dir_all(to).map_err(io(format!("creating {}", to.display())))?;
    for entry in std::fs::read_dir(from).map_err(io(format!("reading {}", from.display())))? {
        let entry = entry.map_err(io(format!("reading {}", from.display())))?;
        let name = entry.file_name();
        if name == ".git" || name == SOURCE_FILE {
            continue;
        }
        let kind = entry
            .file_type()
            .map_err(io(format!("reading {}", entry.path().display())))?;
        let target = to.join(&name);
        if kind.is_dir() {
            copy_dir(&entry.path(), &target)?;
        } else if kind.is_file() {
            std::fs::copy(entry.path(), &target)
                .map_err(io(format!("copying {}", entry.path().display())))?;
        }
    }
    Ok(())
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true)
}

/// Settings namespaces `ui.settings` declares.
fn static_namespaces(manifest: &Manifest) -> Vec<String> {
    manifest
        .ui
        .settings
        .iter()
        .filter_map(|ns| ns.get("namespace").and_then(serde_json::Value::as_str))
        .map(str::to_owned)
        .collect()
}

/// Installs the package at `source` (a path on this host) into `<agent dir>/extensions`.
///
/// # Errors
///
/// [`InstallError::Invalid`] for anything that is not a valid package (the manifest
/// loader's text, checksum mismatches included); [`InstallError::Io`] for copy failures.
pub fn install(
    source: &Path,
    agent_dir: &Path,
    dist_dirs: &[PathBuf],
    replace: bool,
) -> Result<InstallOutcome, InstallError> {
    let root_dir = user_root(agent_dir);
    // A picked `extension.json` means its folder.
    let source = if source.file_name().is_some_and(|n| n == MANIFEST_FILE) {
        source.parent().unwrap_or(source).to_path_buf()
    } else {
        source.to_path_buf()
    };
    if !source.exists() {
        return Err(InstallError::Invalid(format!(
            "{} does not exist on this PC",
            source.display()
        )));
    }
    // A zip unpacks into scratch space next to the target (kept until the copy is done).
    let mut _unpacked: Option<Scratch> = None;
    let from_dir = source.is_dir();
    let unpack_root = if from_dir {
        source.clone()
    } else if is_zip(&source) {
        let scratch = Scratch::new(&root_dir, "unpack")?;
        unzip(&source, &scratch.0)?;
        let dir = scratch.0.clone();
        _unpacked = Some(scratch);
        dir
    } else {
        return Err(InstallError::Invalid(format!(
            "{} is neither a folder nor a .zip",
            source.display()
        )));
    };
    let root = package_root(&unpack_root)?;
    // The manifest loader core's discovery uses: schema, apiVersion, semver, files, checksums.
    let manifest = Manifest::load_package(&root)
        .map_err(|e| InstallError::Invalid(format!("invalid extension package: {e}")))?;
    let id = manifest.id.as_str().to_owned();
    // Replace a hand-copied folder of another name in place (one package per id).
    let target = user_package_dir(&id, agent_dir).unwrap_or_else(|| root_dir.join(&id));
    let installed = target.join(MANIFEST_FILE).is_file();
    let installed_version = installed
        .then(|| read_manifest(&target).map(|m| m.version))
        .flatten();
    let mut outcome = InstallOutcome {
        installed: false,
        confirm: None,
        id: id.clone(),
        name: manifest.name.clone(),
        version: manifest.version.clone(),
        path: None,
        replaced: None,
        warnings: Vec::new(),
        settings_namespaces: static_namespaces(&manifest),
        needs_restart: false,
    };
    if installed && !replace {
        outcome.confirm = Some(Conflict {
            id,
            name: manifest.name.clone(),
            version: manifest.version.clone(),
            installed_version,
        });
        return Ok(outcome);
    }
    // sources.rs: a user package shadows the bundled one of the same id — allowed, said.
    if let Some(bundled) = bundled_manifest(&id, dist_dirs) {
        let newer = match (
            semver::Version::parse(&manifest.version),
            semver::Version::parse(&bundled.version),
        ) {
            (Ok(new), Ok(old)) => new > old,
            _ => false,
        };
        outcome.warnings.push(if newer {
            format!(
                "{} ships with the app (v{}); this v{} replaces it on this PC until you remove it",
                manifest.name, bundled.version, manifest.version
            )
        } else {
            format!(
                "{} ships with the app as v{} — this v{} is not newer, but it replaces the bundled one on this PC until you remove it",
                manifest.name, bundled.version, manifest.version
            )
        });
    }
    if let Some(missing) = Some(manifest.missing_binaries(&root)).filter(|m| !m.is_empty()) {
        outcome.warnings.push(format!(
            "missing native helper(s) for this platform: {} — the tools that need them stay unavailable",
            missing.join(", ")
        ));
    }
    // Stage, then swap with renames (an interrupted install never leaves half a package).
    let staging = root_dir.join(format!(".{id}.installing"));
    let old = root_dir.join(format!(".{id}.old"));
    let _ = std::fs::remove_dir_all(&staging);
    let _ = std::fs::remove_dir_all(&old);
    copy_dir(&root, &staging)?;
    // The copy must still verify (a file changed or vanished mid-copy).
    if let Err(error) = Manifest::load_package(&staging) {
        let _ = std::fs::remove_dir_all(&staging);
        return Err(InstallError::Invalid(format!("invalid extension package: {error}")));
    }
    if from_dir {
        let record = serde_json::json!({
            "source": dunce::canonicalize(&source).unwrap_or(source.clone()).to_string_lossy(),
            "version": manifest.version,
            "installedAt": now_rfc3339(),
        });
        let text = serde_json::to_string_pretty(&record).unwrap_or_default();
        std::fs::write(staging.join(SOURCE_FILE), format!("{text}\n"))
            .map_err(io("writing the source record"))?;
    }
    if installed {
        std::fs::rename(&target, &old).map_err(io(format!("replacing {}", target.display())))?;
    } else if target.exists() {
        // A leftover folder without a manifest: not a package, safe to clear.
        std::fs::remove_dir_all(&target).map_err(io(format!("clearing {}", target.display())))?;
    }
    if let Err(error) = std::fs::rename(&staging, &target) {
        if installed {
            let _ = std::fs::rename(&old, &target);
        }
        let _ = std::fs::remove_dir_all(&staging);
        return Err(io(format!("installing {}", target.display()))(error));
    }
    let _ = std::fs::remove_dir_all(&old);
    outcome.installed = true;
    outcome.path = Some(target.to_string_lossy().into_owned());
    outcome.replaced = installed.then(|| installed_version.unwrap_or_default());
    outcome.needs_restart = true;
    tracing::info!(id = %outcome.id, version = %outcome.version, "installed extension package");
    Ok(outcome)
}

/// Removes the user-installed package `id` (`<agent dir>/extensions/<id>`). Bundled
/// packages are the installer's and cannot be removed (turn them off instead).
///
/// # Errors
///
/// [`InstallError::NotInstalled`] for unknown and bundled-only ids, [`InstallError::Io`].
pub fn remove(id: &str, agent_dir: &Path, dist_dirs: &[PathBuf]) -> Result<RemoveOutcome, InstallError> {
    if !valid_id(id) {
        return Err(InstallError::Invalid(format!("invalid extension id `{id}`")));
    }
    let bundled = bundled_manifest(id, dist_dirs).is_some();
    let Some(target) = user_package_dir(id, agent_dir) else {
        return Err(InstallError::NotInstalled(if bundled {
            format!("{id} ships with the app and cannot be removed — turn it off instead")
        } else {
            format!("{id} is not installed in {}", user_root(agent_dir).display())
        }));
    };
    std::fs::remove_dir_all(&target).map_err(io(format!("removing {}", target.display())))?;
    tracing::info!(%id, "removed extension package");
    Ok(RemoveOutcome {
        id: id.to_owned(),
        path: target.to_string_lossy().into_owned(),
        bundled_remains: bundled,
        needs_restart: true,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, reason = "test assertions")]
mod tests {
    use super::*;
    use oino_extension_api::manifest::sha256_tag;
    use std::io::Write;

    fn manifest(id: &str, version: &str, extra: &str) -> String {
        format!(
            r#"{{"apiVersion":"oino.extension/2","id":"{id}","name":"Acme {id}","version":"{version}"{extra}}}"#
        )
    }

    /// A valid package folder at `dir` (component + matching checksum).
    fn package(dir: &Path, id: &str, version: &str, wasm: &[u8]) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("component.wasm"), wasm).unwrap();
        let tag = sha256_tag(wasm);
        let settings = r#","ui":{"settings":[{"namespace":"acme","sections":[]}]}"#;
        std::fs::write(
            dir.join(MANIFEST_FILE),
            manifest(id, version, &format!(r#","checksums":{{"component.wasm":"{tag}"}}{settings}"#)),
        )
        .unwrap();
    }

    fn zip_dir(dir: &Path, out: &Path, prefix: &str) {
        let file = std::fs::File::create(out).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            zip.start_file(name, options).unwrap();
            zip.write_all(&std::fs::read(entry.path()).unwrap()).unwrap();
        }
        zip.finish().unwrap();
    }

    #[test]
    fn installs_a_valid_folder_and_writes_the_source_record() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src/acme-notify");
        package(&src, "acme.notify", "1.0.0", b"wasm");
        let agent = tmp.path().join("agent");
        let out = install(&src, &agent, &[], false).unwrap();
        assert!(out.installed && out.needs_restart);
        assert_eq!(out.id, "acme.notify");
        assert_eq!(out.settings_namespaces, vec!["acme".to_owned()]);
        assert!(out.warnings.is_empty());
        let target = agent.join("extensions/acme.notify");
        assert!(target.join("component.wasm").is_file());
        Manifest::load_package(&target).unwrap();
        assert!(target.join(SOURCE_FILE).is_file());
        // No scratch/staging folders left behind.
        let leftovers: Vec<_> = std::fs::read_dir(agent.join("extensions"))
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with('.'))
            .collect();
        assert!(leftovers.is_empty(), "{leftovers:?}");
    }

    #[test]
    fn installs_from_a_picked_manifest_and_a_wrapping_folder() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let wrapped = tmp.path().join("release");
        package(&wrapped.join("acme-notify"), "acme.notify", "1.0.0", b"wasm");
        assert!(install(&wrapped, &agent, &[], false).unwrap().installed);
        let other = tmp.path().join("other");
        package(&other, "acme.other", "1.0.0", b"w2");
        let out = install(&other.join(MANIFEST_FILE), &agent, &[], false).unwrap();
        assert_eq!(out.id, "acme.other");
    }

    #[test]
    fn installs_a_zip_detected_by_magic_not_by_name() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("pkg");
        package(&src, "acme.zipped", "2.0.0", b"wasm-z");
        // A blob upload has no extension: `<blobs>/<sha256>`.
        let blob = tmp.path().join("0123abcd");
        zip_dir(&src, &blob, "acme-zipped/");
        let agent = tmp.path().join("agent");
        let out = install(&blob, &agent, &[], false).unwrap();
        assert!(out.installed);
        assert!(agent.join("extensions/acme.zipped/component.wasm").is_file());
        // A zip source records no `.source.json` (the upload is not a stable source).
        assert!(!agent.join("extensions/acme.zipped").join(SOURCE_FILE).exists());
    }

    #[test]
    fn refuses_an_invalid_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("bad");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("component.wasm"), b"wasm").unwrap();
        std::fs::write(
            src.join(MANIFEST_FILE),
            r#"{"apiVersion":"oino.extension/1","id":"acme.bad","name":"Bad","version":"1.0.0"}"#,
        )
        .unwrap();
        let agent = tmp.path().join("agent");
        let error = install(&src, &agent, &[], false).unwrap_err();
        assert!(matches!(error, InstallError::Invalid(_)), "{error}");
        assert!(!agent.join("extensions/acme.bad").exists());
        // Not a folder, not a zip.
        let text = tmp.path().join("notes.txt");
        std::fs::write(&text, "hello").unwrap();
        assert!(matches!(install(&text, &agent, &[], false), Err(InstallError::Invalid(_))));
        // A folder without any manifest.
        let empty = tmp.path().join("empty");
        std::fs::create_dir_all(&empty).unwrap();
        assert!(matches!(install(&empty, &agent, &[], false), Err(InstallError::Invalid(_))));
        // Missing component.
        let nocomp = tmp.path().join("nocomp");
        std::fs::create_dir_all(&nocomp).unwrap();
        std::fs::write(nocomp.join(MANIFEST_FILE), manifest("acme.nocomp", "1.0.0", "")).unwrap();
        assert!(matches!(install(&nocomp, &agent, &[], false), Err(InstallError::Invalid(_))));
    }

    #[test]
    fn refuses_a_checksum_mismatch() {
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("pkg");
        package(&src, "acme.sum", "1.0.0", b"wasm");
        std::fs::write(src.join("component.wasm"), b"tampered").unwrap();
        let agent = tmp.path().join("agent");
        let error = install(&src, &agent, &[], false).unwrap_err();
        assert!(error.to_string().contains("checksum"), "{error}");
        assert!(!agent.join("extensions/acme.sum").exists());
    }

    #[test]
    fn replacing_asks_first_then_swaps_in_place() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        let v1 = tmp.path().join("v1");
        package(&v1, "acme.notify", "1.0.0", b"one");
        install(&v1, &agent, &[], false).unwrap();
        let v2 = tmp.path().join("v2");
        package(&v2, "acme.notify", "1.1.0", b"two");
        // Without `replace`: nothing changes, the app gets a confirmation to show.
        let ask = install(&v2, &agent, &[], false).unwrap();
        assert!(!ask.installed && !ask.needs_restart);
        let conflict = ask.confirm.unwrap();
        assert_eq!(conflict.installed_version.as_deref(), Some("1.0.0"));
        assert_eq!(conflict.version, "1.1.0");
        let target = agent.join("extensions/acme.notify");
        assert_eq!(std::fs::read(target.join("component.wasm")).unwrap(), b"one");
        // With `replace`: swapped.
        let done = install(&v2, &agent, &[], true).unwrap();
        assert!(done.installed);
        assert_eq!(done.replaced.as_deref(), Some("1.0.0"));
        assert_eq!(std::fs::read(target.join("component.wasm")).unwrap(), b"two");
        assert!(!agent.join("extensions/.acme.notify.old").exists());
    }

    #[test]
    fn a_bundled_id_is_shadowed_with_a_warning_not_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let dist = tmp.path().join("dist");
        package(&dist.join("acme.core"), "acme.core", "2.0.0", b"bundled");
        let agent = tmp.path().join("agent");
        let older = tmp.path().join("older");
        package(&older, "acme.core", "1.0.0", b"older");
        let out = install(&older, &agent, &[dist.clone()], false).unwrap();
        assert!(out.installed);
        assert!(out.warnings[0].contains("not newer"), "{:?}", out.warnings);
        let newer = tmp.path().join("newer");
        package(&newer, "acme.core", "2.1.0", b"newer");
        let out = install(&newer, &agent, &[dist.clone()], true).unwrap();
        assert!(!out.warnings[0].contains("not newer"), "{:?}", out.warnings);
        // Remove brings the bundled one back; the bundled one itself cannot be removed.
        let removed = remove("acme.core", &agent, &[dist.clone()]).unwrap();
        assert!(removed.bundled_remains && removed.needs_restart);
        assert!(!agent.join("extensions/acme.core").exists());
        assert!(dist.join("acme.core").is_dir());
        let error = remove("acme.core", &agent, &[dist]).unwrap_err();
        assert!(error.to_string().contains("ships with the app"), "{error}");
    }

    #[test]
    fn a_hand_copied_folder_is_found_by_manifest_id() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        package(&agent.join("extensions/my-copy"), "acme.copied", "1.0.0", b"one");
        assert!(is_user_installed("acme.copied", &agent));
        let v2 = tmp.path().join("v2");
        package(&v2, "acme.copied", "2.0.0", b"two");
        assert!(install(&v2, &agent, &[], false).unwrap().confirm.is_some());
        install(&v2, &agent, &[], true).unwrap();
        // Replaced in place, not duplicated under a second folder.
        assert_eq!(std::fs::read(agent.join("extensions/my-copy/component.wasm")).unwrap(), b"two");
        assert!(!agent.join("extensions/acme.copied").exists());
        remove("acme.copied", &agent, &[]).unwrap();
        assert!(!agent.join("extensions/my-copy").exists());
    }

    #[test]
    fn remove_refuses_unknown_and_unsafe_ids() {
        let tmp = tempfile::tempdir().unwrap();
        let agent = tmp.path().join("agent");
        assert!(matches!(remove("acme.none", &agent, &[]), Err(InstallError::NotInstalled(_))));
        assert!(matches!(remove("../etc", &agent, &[]), Err(InstallError::Invalid(_))));
        assert!(matches!(remove("", &agent, &[]), Err(InstallError::Invalid(_))));
    }

    #[test]
    fn a_zip_escaping_its_folder_is_refused() {
        let tmp = tempfile::tempdir().unwrap();
        let archive = tmp.path().join("evil.zip");
        let file = std::fs::File::create(&archive).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("../evil.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(b"x").unwrap();
        zip.finish().unwrap();
        let agent = tmp.path().join("agent");
        assert!(matches!(install(&archive, &agent, &[], false), Err(InstallError::Invalid(_))));
        assert!(!agent.join("evil.txt").exists());
    }
}
