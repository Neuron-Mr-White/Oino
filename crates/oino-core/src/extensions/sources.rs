//! Extension source discovery (dist.md §5, core.md P-DISCOVERY/P-DOCS): the ordered
//! package list every embedding shares — (1) the distribution share dir, (2) the user agent
//! dir (`~/.oino/extensions`), (3) the trust-gated project dir (`<cwd>/.oino/extensions`),
//! (4) `-e/--extension` explicit packages — with same-id replacement-in-place (a later
//! source shadows an earlier package of the same id at the earlier position), `-ne`
//! filtering (providers stay) and `--exclude-extension` drops (WP-X2).
//!
//! The successor of the `oino-bundle` embedded table (P-BUNDLE): nothing is compiled into a
//! binary; installers own the share dir, CLI-only installs unpack the extension tarball into
//! `~/.oino/extensions`. The host links core and reuses this module unchanged.
//!
//! std-only by design (P-WORKSPACE): discovery returns [`LoadedPackage`] descriptors and
//! never touches wasmtime; the caller (the CLI loader, the host) instantiates them.
//!
//! The docs root resolution (P-DOCS) lives here too:
//! `OINO_DOCS_ROOT` > `<share root>/docs` > legacy `~/.oino/bundled/oino.docs@*` > absent
//! (an absent root omits the system prompt's docs section instead of pointing at a missing
//! path; `ServicesOptions::docs_root` uses an empty path for "absent").

use std::path::{Path, PathBuf};

use oino_extension_api::LoadedPackage;
use oino_extension_api::manifest::{ExtensionKind, MANIFEST_FILE, Manifest, ManifestError};

/// Overrides the distribution share dir (points at the `extensions/` directory holding the
/// `<id>/` packages); set to an empty value to disable the share source (dev/tests).
pub const DIST_EXTENSIONS_ENV: &str = "OINO_DIST_EXTENSIONS";

/// Overrides the docs root (`OINO_DOCS_ROOT`).
pub const DOCS_ROOT_ENV: &str = "OINO_DOCS_ROOT";

/// Severity of a [`SourceDiagnostic`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SourceSeverity {
    /// Startup continues (the source is skipped, the package dropped).
    Warning,
    /// Fatal at startup (pi `-e` semantics: an unloadable explicit package fails the start).
    Error,
}

/// One discovery diagnostic; the embedder maps these to its own UI (the CLI: startup lines).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceDiagnostic {
    /// Severity.
    pub severity: SourceSeverity,
    /// Text.
    pub message: String,
}

impl SourceDiagnostic {
    /// A warning.
    #[must_use]
    pub fn warning(message: impl Into<String>) -> Self {
        Self {
            severity: SourceSeverity::Warning,
            message: message.into(),
        }
    }

    /// An error.
    #[must_use]
    pub fn error(message: impl Into<String>) -> Self {
        Self {
            severity: SourceSeverity::Error,
            message: message.into(),
        }
    }
}

/// A resolved package plus its provenance.
#[derive(Clone, Debug)]
pub struct SourcePackage {
    /// The package.
    pub package: LoadedPackage,
    /// Found under the project `<cwd>/.oino/extensions` (skipped when the project is
    /// untrusted).
    pub project: bool,
}

/// Discovery inputs.
#[derive(Clone, Debug)]
pub struct SourcesContext {
    /// Agent dir (`~/.oino`).
    pub agent_dir: PathBuf,
    /// Project cwd.
    pub cwd: PathBuf,
    /// `-ne`: drop `kind: "extension"` packages of the scanned sources (providers stay;
    /// explicit `-e` packages are never filtered — same as `-e` before the split).
    pub no_extensions: bool,
    /// `--exclude-extension` ids, dropped after composition (replacement-in-place included),
    /// before instantiation; providers are excluded the same way (WP-X2).
    pub excluded: Vec<String>,
}

impl SourcesContext {
    /// The scanned directories: agent global, then the trust-gated project dir.
    #[must_use]
    fn scan_dirs(&self) -> Vec<(PathBuf, bool)> {
        vec![
            (self.agent_dir.join("extensions"), false),
            (self.cwd.join(".oino").join("extensions"), true),
        ]
    }
}

/// Discovery output.
#[derive(Clone, Debug, Default)]
pub struct ResolvedSources {
    /// Composed packages in load order.
    pub packages: Vec<SourcePackage>,
    /// The share extensions dir that contributed, when any (env override or exe-relative).
    pub share_dir: Option<PathBuf>,
    /// Diagnostics, in discovery order.
    pub diagnostics: Vec<SourceDiagnostic>,
}

/// Per-OS exe-relative share-dir candidates (dist.md §5 layouts; first existing wins):
/// Linux `/usr/bin/oino` → `/usr/share/unipi-oino/extensions`; macOS
/// `UniPi.app/Contents/Resources/bin/oino` → `Contents/Resources/extensions`; Windows
/// `%LOCALAPPDATA%\UniPi\oino.exe` → `%LOCALAPPDATA%\UniPi\extensions`.
#[must_use]
pub fn share_candidates(exe: &Path) -> Vec<PathBuf> {
    let Some(bin) = exe.parent() else {
        return Vec::new();
    };
    let linux = || {
        bin.parent()
            .map(|p| p.join("share").join("unipi-oino").join("extensions"))
    };
    if cfg!(windows) {
        [Some(bin.join("extensions")), linux()]
            .into_iter()
            .flatten()
            .collect()
    } else if cfg!(target_os = "macos") {
        [
            Some(bin.join("../extensions")),
            linux(),
            Some(bin.join("extensions")),
        ]
        .into_iter()
        .flatten()
        .collect()
    } else {
        linux().into_iter().collect()
    }
}

/// The distribution share extensions dir: `OINO_DIST_EXTENSIONS` when set (empty = disabled;
/// a missing dir warns), else the first existing exe-relative candidate.
#[must_use]
pub fn dist_share_dir() -> Option<PathBuf> {
    let mut diagnostics = Vec::new();
    dist_share_dir_with(
        std::env::var_os(DIST_EXTENSIONS_ENV).as_deref(),
        &exe_path(),
        &mut diagnostics,
    )
}

fn exe_path() -> PathBuf {
    std::env::current_exe().unwrap_or_else(|_| PathBuf::from("oino"))
}

/// Pure core of [`dist_share_dir`].
///
/// # Errors
///
/// None; problems are reported through `diagnostics`.
#[must_use]
pub fn dist_share_dir_with(
    env: Option<&std::ffi::OsStr>,
    exe: &Path,
    diagnostics: &mut Vec<SourceDiagnostic>,
) -> Option<PathBuf> {
    if let Some(value) = env {
        if value.is_empty() {
            return None;
        }
        let dir = PathBuf::from(value);
        if dir.is_dir() {
            return Some(dir);
        }
        diagnostics.push(SourceDiagnostic::warning(format!(
            "{DIST_EXTENSIONS_ENV} points at a missing directory: {}",
            dir.display()
        )));
        return None;
    }
    share_candidates(exe).into_iter().find(|dir| dir.is_dir())
}

/// Loads the package rooted at `root` (`extension.json` + component path).
fn load_dir(root: &Path, bundled: bool) -> Result<LoadedPackage, ManifestError> {
    let manifest = Manifest::load_package(root)?;
    Ok(LoadedPackage {
        component: oino_extension_api::ComponentSource::Path(manifest.component_path(root)),
        manifest,
        root: Some(root.to_path_buf()),
        bundled,
    })
}

/// Discovers the packages of one directory (children with an `extension.json`, sorted for a
/// stable order, first occurrence of an id wins). Skips missing directories silently: an
/// empty agent/project dir is normal, not a diagnostic.
fn discover_dir(
    dir: &Path,
    bundled: bool,
    diagnostics: &mut Vec<SourceDiagnostic>,
) -> Vec<LoadedPackage> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.join(MANIFEST_FILE).is_file())
        .collect();
    roots.sort();
    let mut packages: Vec<LoadedPackage> = Vec::new();
    for root in roots {
        match load_dir(&root, bundled) {
            Ok(package) => {
                if packages
                    .iter()
                    .any(|p| p.manifest.id == package.manifest.id)
                {
                    continue;
                }
                packages.push(package);
            }
            Err(error) => diagnostics.push(SourceDiagnostic::warning(format!(
                "Skipped extension package {}: {error}",
                root.display()
            ))),
        }
    }
    packages
}

/// Adds `package`, replacing an earlier package with the same id in place (a later source
/// shadows an earlier one at the earlier position).
fn place(packages: &mut Vec<SourcePackage>, package: SourcePackage) {
    let id = package.package.manifest.id.clone();
    match packages.iter_mut().find(|p| p.package.manifest.id == id) {
        Some(slot) => *slot = package,
        None => packages.push(package),
    }
}

/// Drops every composed package whose id is excluded (one warning per dropped id, `-e` paths
/// included) and warns when an excluded id matched nothing (helps safe mode name the right
/// suspect). Never fatal.
fn apply_exclusions(
    packages: &mut Vec<SourcePackage>,
    excluded: &[String],
    diagnostics: &mut Vec<SourceDiagnostic>,
) {
    if excluded.is_empty() {
        return;
    }
    let mut dropped = Vec::new();
    packages.retain(|p| {
        let id = p.package.manifest.id.as_str();
        let hit = excluded.iter().any(|x| x == id);
        if hit {
            dropped.push(id.to_owned());
        }
        !hit
    });
    for id in &dropped {
        diagnostics.push(SourceDiagnostic::warning(format!(
            "Excluded extension \"{id}\" not loaded"
        )));
    }
    for id in excluded {
        if !dropped.contains(id) {
            diagnostics.push(SourceDiagnostic::warning(format!(
                "--exclude-extension \"{id}\" matched no extension package"
            )));
        }
    }
}

/// Resolves an `-e` source to a package (a directory, or a file inside one).
fn explicit_package(
    source: &str,
    cwd: &Path,
    diagnostics: &mut Vec<SourceDiagnostic>,
) -> Option<LoadedPackage> {
    let home = oino_settings::paths::home_dir();
    let path = oino_settings::paths::resolve_path(source, cwd, home.as_deref(), false);
    let root = if path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or(path)
    } else {
        path
    };
    if !root.join(MANIFEST_FILE).is_file() {
        diagnostics.push(SourceDiagnostic::error(format!(
            "Failed to load extension \"{source}\": no {MANIFEST_FILE} in {}",
            root.display()
        )));
        return None;
    }
    match load_dir(&root, false) {
        Ok(package) => Some(package),
        Err(error) => {
            diagnostics.push(SourceDiagnostic::error(format!(
                "Failed to load extension \"{source}\": {error}"
            )));
            None
        }
    }
}

/// Resolves the ordered package list (P-DISCOVERY). `-e` packages replace earlier same-id
/// packages in place and win over every scanned source; `builtin:<id>` test components are
/// the caller's (the CLI's) business, not discovery's.
#[must_use]
pub fn resolve(explicit: &[String], context: &SourcesContext) -> ResolvedSources {
    resolve_with(
        explicit,
        context,
        std::env::var_os(DIST_EXTENSIONS_ENV).as_deref(),
    )
}

/// [`resolve`] with the `OINO_DIST_EXTENSIONS` override passed in (tests, embedders).
#[must_use]
pub fn resolve_with(
    explicit: &[String],
    context: &SourcesContext,
    dist_env: Option<&std::ffi::OsStr>,
) -> ResolvedSources {
    let mut out = ResolvedSources::default();
    let share = dist_share_dir_with(dist_env, &exe_path(), &mut out.diagnostics);
    out.share_dir.clone_from(&share);
    let keep =
        |p: &LoadedPackage| !context.no_extensions || p.manifest.kind == ExtensionKind::Provider;
    // (1) distribution share dir — installer-owned, so its packages carry bundle trust.
    for package in share
        .as_deref()
        .map(|dir| discover_dir(dir, true, &mut out.diagnostics))
        .unwrap_or_default()
    {
        if keep(&package) {
            place(
                &mut out.packages,
                SourcePackage {
                    package,
                    project: false,
                },
            );
        }
    }
    // (2) agent dir, (3) trust-gated project dir.
    for (dir, project) in context.scan_dirs() {
        for package in discover_dir(&dir, false, &mut out.diagnostics) {
            if keep(&package) {
                place(&mut out.packages, SourcePackage { package, project });
            }
        }
    }
    // (4) `-e` wins over everything.
    for source in explicit {
        if let Some(package) = explicit_package(source, &context.cwd, &mut out.diagnostics) {
            place(
                &mut out.packages,
                SourcePackage {
                    package,
                    project: false,
                },
            );
        }
    }
    apply_exclusions(&mut out.packages, &context.excluded, &mut out.diagnostics);
    warn_missing_binaries(&mut out.packages, &mut out.diagnostics);
    out
}

/// Load-time binary check (ext-abi §5.2): a package declaring `binaries` whose
/// `bin/<name>` is absent still loads — its tools report "binary unavailable" —
/// but discovery says so (a wrong-platform dist is the usual cause).
fn warn_missing_binaries(
    packages: &mut [SourcePackage],
    diagnostics: &mut Vec<SourceDiagnostic>,
) {
    for p in packages {
        let Some(root) = p.package.root.clone() else {
            continue;
        };
        for name in p.package.manifest.missing_binaries(&root) {
            diagnostics.push(SourceDiagnostic::warning(format!(
                "Extension {}: binary \"{name}\" missing from bin/ — its tools may report unavailable",
                p.package.manifest.id
            )));
        }
    }
}

/// The docs root of the system prompt's docs section (P-DOCS):
/// `OINO_DOCS_ROOT` > `<share root>/docs` (the sibling `docs/` of the share `extensions/`
/// dir, when it carries the docs tree) > legacy `~/.oino/bundled/oino.docs@*` > absent.
#[must_use]
pub fn docs_root(agent_dir: &Path) -> Option<PathBuf> {
    docs_root_with(
        agent_dir,
        dist_share_dir().as_deref(),
        std::env::var_os(DOCS_ROOT_ENV).as_deref(),
    )
}

/// Pure core of [`docs_root`] (embedders pass their resolved share dir and env override).
#[must_use]
pub fn docs_root_with(
    agent_dir: &Path,
    share: Option<&Path>,
    env: Option<&std::ffi::OsStr>,
) -> Option<PathBuf> {
    if let Some(value) = env.filter(|value| !value.is_empty()) {
        return Some(PathBuf::from(value));
    }
    if let Some(share) = share
        && let Some(root) = share.parent().map(|p| p.join("docs"))
        && root.join("README.md").is_file()
    {
        return Some(root);
    }
    legacy_docs_root(agent_dir)
}

/// The newest extracted `oino.docs@<version>-<hash8>` of retired `oino-bundle` runs.
fn legacy_docs_root(agent_dir: &Path) -> Option<PathBuf> {
    let bundled = agent_dir.join("bundled");
    let Ok(entries) = std::fs::read_dir(&bundled) else {
        return None;
    };
    let mut roots: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("oino.docs@"))
                && p.join("README.md").is_file()
        })
        .collect();
    roots.sort();
    roots.pop()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A minimal package at `dir` (`kind: "extension"` unless overridden).
    fn package(dir: &Path, id: &str, kind: &str) {
        package_versioned(dir, id, kind, "1.0.0");
    }

    fn package_versioned(dir: &Path, id: &str, kind: &str, version: &str) {
        std::fs::create_dir_all(dir).unwrap();
        // A provider must provide at least one wire API (§7.2).
        let provides = if kind == "provider" {
            ", \"provides\": {\"apis\": [\"openai-completions\"]}"
        } else {
            ""
        };
        let json = format!(
            r#"{{"apiVersion":"{}","id":"{id}","name":"{id}","version":"{version}","kind":"{kind}"{provides}}}"#,
            oino_extension_api::manifest::API_VERSION
        );
        std::fs::write(dir.join(MANIFEST_FILE), json).unwrap();
        std::fs::write(dir.join("component.wasm"), b"\0asm").unwrap();
    }

    fn ctx(dir: &Path) -> SourcesContext {
        SourcesContext {
            agent_dir: dir.join("agent"),
            cwd: dir.join("cwd"),
            no_extensions: false,
            excluded: Vec::new(),
        }
    }

    fn ids(out: &ResolvedSources) -> Vec<String> {
        out.packages
            .iter()
            .map(|p| p.package.manifest.id.to_string())
            .collect()
    }

    fn messages(out: &ResolvedSources) -> Vec<String> {
        out.diagnostics.iter().map(|d| d.message.clone()).collect()
    }

    #[test]
    fn share_candidates_match_the_installer_layouts() {
        let candidates = |exe: &str| {
            share_candidates(Path::new(exe))
                .iter()
                .map(|c| c.display().to_string())
                .collect::<Vec<_>>()
        };
        if cfg!(windows) {
            assert_eq!(
                candidates(r"C:\Users\u\AppData\Local\UniPi\oino.exe"),
                vec![
                    r"C:\Users\u\AppData\Local\UniPi\extensions".to_owned(),
                    r"C:\Users\u\AppData\Local\extensions".to_owned(),
                ]
            );
        } else if cfg!(target_os = "macos") {
            assert_eq!(
                candidates("/Applications/UniPi.app/Contents/Resources/bin/oino"),
                vec![
                    "/Applications/UniPi.app/Contents/Resources/bin/../extensions".to_owned(),
                    "/Applications/UniPi.app/Contents/Resources/share/unipi-oino/extensions"
                        .to_owned(),
                    "/Applications/UniPi.app/Contents/Resources/bin/extensions".to_owned(),
                ]
            );
        } else {
            assert_eq!(
                candidates("/usr/bin/oino"),
                vec!["/usr/share/unipi-oino/extensions".to_owned()]
            );
        }
    }

    #[test]
    fn share_env_override_empty_disables_missing_silenced() {
        let mut diagnostics = Vec::new();
        assert!(
            dist_share_dir_with(
                Some(std::ffi::OsStr::new("")),
                Path::new("/x"),
                &mut diagnostics
            )
            .is_none()
        );
        assert_eq!(diagnostics.len(), 0);
        let missing = Path::new("/does-not-exist-oino");
        assert!(
            dist_share_dir_with(Some(missing.as_os_str()), Path::new("/x"), &mut diagnostics)
                .is_none()
        );
        assert_eq!(
            diagnostics,
            vec![SourceDiagnostic::warning(format!(
                "{DIST_EXTENSIONS_ENV} points at a missing directory: /does-not-exist-oino"
            ))]
        );
    }

    #[test]
    fn share_env_override_and_order_shadowing() {
        let dir = tempfile::tempdir().unwrap();
        let mut context = ctx(dir.path());
        context.cwd = dir.path().join("cwd-other");
        package(
            &dir.path().join("agent/extensions/unipi.core"),
            "unipi.core",
            "extension",
        );
        package(
            &dir.path().join("cwd-other/.oino/extensions/unipi.proj"),
            "unipi.proj",
            "extension",
        );
        // Share dir: core shadowed by a newer agent copy in place, notify appended.
        package_versioned(
            &dir.path().join("dist/extensions/unipi.core"),
            "unipi.core",
            "extension",
            "0.9.0",
        );
        package(
            &dir.path().join("dist/extensions/unipi.notify"),
            "unipi.notify",
            "extension",
        );
        let share = dir.path().join("dist/extensions");
        let out = resolve_with(&[], &context, Some(share.as_os_str()));
        assert_eq!(out.share_dir, Some(share));
        assert_eq!(
            ids(&out),
            vec![
                "unipi.core".to_owned(),
                "unipi.notify".to_owned(),
                "unipi.proj".to_owned(),
            ]
        );
        // The agent copy (1.0.0) replaced the share copy (0.9.0) in place.
        assert_eq!(out.packages[0].package.manifest.version, "1.0.0");
        assert!(!out.packages[0].package.bundled);
        assert!(out.packages[1].package.bundled);
        assert!(out.packages[2].project);
    }

    #[test]
    fn explicit_e_wins_over_every_source() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        package(
            &dir.path().join("agent/extensions/unipi.core"),
            "unipi.core",
            "extension",
        );
        package_versioned(
            &dir.path().join("e/core"),
            "unipi.core",
            "extension",
            "9.9.9",
        );
        let out = resolve(&[dir.path().join("e/core").display().to_string()], &context);
        assert_eq!(ids(&out), vec!["unipi.core".to_owned()]);
        assert_eq!(out.packages[0].package.manifest.version, "9.9.9");
        assert!(!out.packages[0].package.bundled);
    }

    #[test]
    fn explicit_source_inside_a_file_takes_its_parent() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        let root = dir.path().join("pkg");
        package(&root, "x.y", "extension");
        let manifest = root.join(MANIFEST_FILE).display().to_string();
        let out = resolve(&[manifest], &context);
        assert_eq!(ids(&out), vec!["x.y".to_owned()]);
    }

    #[test]
    fn bad_explicit_source_is_an_error_with_the_pi_message() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        let out = resolve(&[dir.path().join("nope").display().to_string()], &context);
        assert!(out.packages.is_empty());
        assert_eq!(
            messages(&out),
            vec![format!(
                "Failed to load extension \"{}\": no {MANIFEST_FILE} in {}",
                dir.path().join("nope").display(),
                dir.path().join("nope").display()
            )]
        );
        assert_eq!(out.diagnostics[0].severity, SourceSeverity::Error);
    }

    #[test]
    fn invalid_scanned_package_is_skipped_with_a_warning() {
        let dir = tempfile::tempdir().unwrap();
        let context = ctx(dir.path());
        let bad = dir.path().join("agent/extensions/bad");
        std::fs::create_dir_all(&bad).unwrap();
        std::fs::write(bad.join(MANIFEST_FILE), "{}").unwrap();
        package(
            &dir.path().join("agent/extensions/good"),
            "good.one",
            "extension",
        );
        let out = resolve(&[], &context);
        assert_eq!(ids(&out), vec!["good.one".to_owned()]);
        assert_eq!(out.diagnostics.len(), 1);
        assert_eq!(out.diagnostics[0].severity, SourceSeverity::Warning);
        assert!(
            out.diagnostics[0]
                .message
                .starts_with(&format!("Skipped extension package {}:", bad.display()))
        );
    }

    #[test]
    fn no_extensions_keeps_providers_of_scanned_sources() {
        let dir = tempfile::tempdir().unwrap();
        let mut context = ctx(dir.path());
        context.no_extensions = true;
        package(
            &dir.path().join("agent/extensions/oino.anthropic"),
            "oino.anthropic",
            "provider",
        );
        package(
            &dir.path().join("agent/extensions/unipi.core"),
            "unipi.core",
            "extension",
        );
        let out = resolve(&[], &context);
        assert_eq!(ids(&out), vec!["oino.anthropic".to_owned()]);
    }

    #[test]
    fn exclusions_drop_by_id_with_diagnostics() {
        let dir = tempfile::tempdir().unwrap();
        let mut context = ctx(dir.path());
        package(
            &dir.path().join("agent/extensions/oino.anthropic"),
            "oino.anthropic",
            "provider",
        );
        package(
            &dir.path().join("agent/extensions/unipi.notify"),
            "unipi.notify",
            "extension",
        );
        context.excluded = vec![
            "unipi.notify".to_owned(),
            "oino.anthropic".to_owned(),
            "does.not-exist".to_owned(),
        ];
        let out = resolve(&[], &context);
        assert!(out.packages.is_empty());
        assert_eq!(
            messages(&out),
            vec![
                "Excluded extension \"oino.anthropic\" not loaded".to_owned(),
                "Excluded extension \"unipi.notify\" not loaded".to_owned(),
                "--exclude-extension \"does.not-exist\" matched no extension package".to_owned(),
            ]
        );
        assert!(
            out.diagnostics
                .iter()
                .all(|d| d.severity == SourceSeverity::Warning)
        );
    }

    #[test]
    fn missing_declared_binaries_warn_but_load() {
        let dir = tempfile::tempdir().unwrap();
        let pkg = dir.path().join("agent/extensions/unipi.kanboard");
        package(&pkg, "unipi.kanboard", "extension");
        let manifest = std::fs::read_to_string(pkg.join(MANIFEST_FILE)).unwrap();
        std::fs::write(
            pkg.join(MANIFEST_FILE),
            manifest.replace("\"kind\"", "\"binaries\":[\"unipi-kanboard\"],\"kind\""),
        )
        .unwrap();
        // Missing: one warning, the package still loads.
        let out = resolve(&[], &ctx(dir.path()));
        assert_eq!(ids(&out).len(), 1);
        assert_eq!(
            messages(&out),
            vec![
                "Extension unipi.kanboard: binary \"unipi-kanboard\" missing from bin/ — its tools may report unavailable"
                    .to_owned(),
            ]
        );
        assert!(out
            .diagnostics
            .iter()
            .all(|d| d.severity == SourceSeverity::Warning));
        // Present: silent.
        std::fs::create_dir_all(pkg.join("bin")).unwrap();
        std::fs::write(pkg.join("bin/unipi-kanboard"), b"#").unwrap();
        let out = resolve(&[], &ctx(dir.path()));
        assert_eq!(ids(&out).len(), 1);
        assert_eq!(messages(&out).len(), 0);
    }

    #[test]
    fn docs_root_chain_env_share_legacy_absent() {
        let dir = tempfile::tempdir().unwrap();
        let agent = dir.path().join("agent");
        std::fs::create_dir_all(&agent).unwrap();
        // Absent.
        assert_eq!(docs_root_with(&agent, None, None), None);
        // Legacy extraction (newest wins).
        let old = agent.join("bundled/oino.docs@1.0.0-aaaaaaaa");
        let new = agent.join("bundled/oino.docs@2.0.0-bbbbbbbb");
        std::fs::create_dir_all(&old).unwrap();
        std::fs::create_dir_all(&new).unwrap();
        std::fs::write(old.join("README.md"), b"old").unwrap();
        std::fs::write(new.join("README.md"), b"new").unwrap();
        assert_eq!(
            docs_root_with(&agent, None, None).as_deref(),
            Some(new.as_path())
        );
        // Share docs (a share dir without docs falls back to legacy).
        let share = dir.path().join("dist/extensions");
        std::fs::create_dir_all(&share).unwrap();
        assert_eq!(
            docs_root_with(&agent, Some(&share), None).as_deref(),
            Some(new.as_path())
        );
        let docs = dir.path().join("dist/docs");
        std::fs::create_dir_all(&docs).unwrap();
        std::fs::write(docs.join("README.md"), b"docs").unwrap();
        assert_eq!(
            docs_root_with(&agent, Some(&share), None),
            Some(docs.clone())
        );
        // Env wins.
        let env = dir.path().join("env-docs");
        assert_eq!(
            docs_root_with(&agent, Some(&share), Some(env.as_os_str())),
            Some(env)
        );
        // Empty env = unset.
        assert_eq!(
            docs_root_with(&agent, Some(&share), Some(std::ffi::OsStr::new(""))),
            Some(docs)
        );
    }
}
