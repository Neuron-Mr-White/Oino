//! Extension sources: the components bundled into this binary (D14, `oino-bundle`), installed
//! packages (`~/.oino/extensions`, trust-gated `<cwd>/.oino/extensions`), `-e/--extension`
//! package directories and `builtin:<id>` test components.
//!
//! Load order: bundled packages in bundle order (providers, then unipi `LOAD_ORDER`); an
//! installed or `-e` package with the id of an earlier one replaces it in place, others are
//! appended (global, project, then `-e`). `OINO_BUNDLED=0` skips the bundled packages.
//! `--exclude-extension <id>` drops composed packages by id before instantiation (WP-X2).
//!
//! Every `CliLoader` builds its own [`WasmRuntime`] (the cwd is per instance), but the
//! engine, epoch-ticker thread, compile caches and local-services hub are process-shared
//! (`oino_wasm_host::EngineRegistry`, E-ENGINE): the startup path, every ACP session and a
//! multi-cwd host share one engine and compile each distinct component once per process.

use std::path::{Path, PathBuf};
use std::sync::{Arc, PoisonError};

use futures::future::BoxFuture;
use oino_core::extensions::api::ExtensionInstance;
use oino_core::extensions::host::HostApi;
use oino_core::services::ExtensionLoader;
use oino_extension_api::LoadedPackage;
use oino_extension_api::manifest::ExtensionKind;
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};

use crate::cli::args::Diagnostic;

/// pi `EXTENSION_LOAD_FAILURE_HINT` (D13).
pub const EXTENSION_LOAD_FAILURE_HINT: &str = "Hint: Start without extensions using \"oino -ne\".";

/// Environment variable that disables the bundled components when set to `0`.
pub const BUNDLED_ENV: &str = "OINO_BUNDLED";

/// Components compiled into this binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Builtin {
    /// Scripted provider (feature `faux`).
    #[cfg(feature = "faux")]
    Faux,
    /// UI probe extension (feature `faux`).
    #[cfg(feature = "faux")]
    UiProbe,
}

impl Builtin {
    fn parse(id: &str) -> Option<Self> {
        match id {
            #[cfg(feature = "faux")]
            "faux" => Some(Self::Faux),
            #[cfg(feature = "faux")]
            "ui-probe" => Some(Self::UiProbe),
            _ => None,
        }
    }

    #[cfg_attr(
        not(feature = "faux"),
        allow(unused_variables, reason = "no built-ins without features")
    )]
    fn instantiate(self, host: &Arc<dyn HostApi>) -> Arc<dyn ExtensionInstance> {
        match self {
            #[cfg(feature = "faux")]
            Self::Faux => Arc::new(crate::faux::FauxComponent(
                crate::faux::FauxProvider::from_env().with_host(host),
            )),
            #[cfg(feature = "faux")]
            Self::UiProbe => Arc::new(crate::faux::UiProbe::new(host)),
        }
    }
}

/// Where packages are looked up.
#[derive(Clone, Debug)]
pub struct LoaderContext {
    /// Agent dir (`~/.oino`).
    pub agent_dir: PathBuf,
    /// Project cwd.
    pub cwd: PathBuf,
    /// `-ne`: skip bundled and installed `kind: "extension"` packages (providers stay).
    pub no_extensions: bool,
    /// `--exclude-extension`: package ids dropped after composition, before instantiation
    /// (WP-X2); `builtin:` test components are not subject.
    pub exclude_extensions: Vec<String>,
}

impl LoaderContext {
    /// Bundled components are enabled unless `OINO_BUNDLED=0`.
    fn bundled_enabled() -> bool {
        std::env::var(BUNDLED_ENV).map_or(true, |value| value.trim() != "0")
    }
}

/// Loader for bundled, installed and `-e` extensions.
pub struct CliLoader {
    builtins: Vec<Builtin>,
    wasm: Option<CoreLoader>,
    diagnostics: Vec<Diagnostic>,
}

impl std::fmt::Debug for CliLoader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CliLoader")
            .field("builtins", &self.builtins)
            .field("wasm", &self.wasm)
            .finish_non_exhaustive()
    }
}

/// Adds `package`, replacing an earlier package with the same id in place.
fn place(packages: &mut Vec<SourcedPackage>, package: SourcedPackage) {
    let id = &package.package.manifest.id;
    match packages.iter_mut().find(|p| &p.package.manifest.id == id) {
        Some(slot) => *slot = package,
        None => packages.push(package),
    }
}

/// X-EXCLUDE/WP-X2: drops every composed package whose id is excluded (one diagnostic per
/// dropped id, `-e` paths included) and warns when an excluded id matched nothing (helps
/// safe mode name the right suspect). Never fatal.
fn apply_exclusions(
    packages: &mut Vec<SourcedPackage>,
    excluded: &[String],
    diagnostics: &mut Vec<Diagnostic>,
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
        diagnostics.push(Diagnostic::warning(format!(
            "Excluded extension \"{id}\" not loaded"
        )));
    }
    for id in excluded {
        if !dropped.contains(id) {
            diagnostics.push(Diagnostic::warning(format!(
                "--exclude-extension \"{id}\" matched no extension package"
            )));
        }
    }
}

/// The bundled packages (resources extracted under `agent_dir/bundled`).
fn bundled(agent_dir: &Path, diagnostics: &mut Vec<Diagnostic>) -> Vec<LoadedPackage> {
    let mut out = Vec::new();
    for ext in oino_bundle::BUNDLED {
        let root = match ext.extract(agent_dir) {
            Ok(root) => Some(root),
            Err(error) => {
                diagnostics.push(Diagnostic::warning(format!(
                    "Bundled extension \"{}\": {error}",
                    ext.id
                )));
                None
            }
        };
        let package = ext
            .component()
            .map_err(|e| e.to_string())
            .and_then(|bytes| {
                oino_wasm_host::packages::from_bytes(ext.manifest, Arc::from(bytes), root)
                    .map_err(|e| e.to_string())
            });
        match package {
            Ok(package) => out.push(package),
            Err(error) => diagnostics.push(Diagnostic::warning(format!(
                "Failed to load extension \"{}\": {error}",
                ext.id
            ))),
        }
    }
    out
}

/// Resolves an `-e` source to a package directory.
fn explicit_package(source: &str, cwd: &Path) -> Result<LoadedPackage, String> {
    let home = oino_settings::paths::home_dir();
    let path = oino_settings::paths::resolve_path(source, cwd, home.as_deref(), false);
    let root = if path.is_file() {
        path.parent().map(Path::to_path_buf).unwrap_or(path)
    } else {
        path
    };
    if !root
        .join(oino_extension_api::manifest::MANIFEST_FILE)
        .is_file()
    {
        return Err(format!(
            "no {} in {}",
            oino_extension_api::manifest::MANIFEST_FILE,
            root.display()
        ));
    }
    oino_wasm_host::packages::load_dir(&root, false).map_err(|e| e.to_string())
}

impl CliLoader {
    /// Resolves the package set; unknown `-e` sources become `Failed to load extension` errors.
    #[must_use]
    pub fn new(sources: Vec<String>, context: &LoaderContext) -> Self {
        let mut builtins = Vec::new();
        let mut diagnostics = Vec::new();
        let mut packages: Vec<SourcedPackage> = Vec::new();
        let keep = |p: &LoadedPackage| {
            !context.no_extensions || p.manifest.kind == ExtensionKind::Provider
        };
        if LoaderContext::bundled_enabled() {
            for package in bundled(&context.agent_dir, &mut diagnostics) {
                if keep(&package) {
                    packages.push(SourcedPackage {
                        package,
                        project: false,
                    });
                }
            }
        }
        let discovered = [
            (context.agent_dir.join("extensions"), false),
            (context.cwd.join(".oino").join("extensions"), true),
        ];
        for (dir, project) in discovered {
            let (found, problems) = oino_wasm_host::packages::discover(&[dir], false);
            for problem in problems {
                diagnostics.push(Diagnostic::warning(format!(
                    "Skipped extension package {}: {}",
                    problem.path.display(),
                    problem.error
                )));
            }
            for package in found.into_iter().filter(|p| keep(p)) {
                place(&mut packages, SourcedPackage { package, project });
            }
        }
        for source in sources {
            if let Some(id) = source.strip_prefix("builtin:") {
                match Builtin::parse(id) {
                    Some(builtin) => builtins.push(builtin),
                    None => diagnostics.push(Diagnostic::error(format!(
                        "Failed to load extension \"{source}\": unknown built-in extension \"{id}\""
                    ))),
                }
                continue;
            }
            match explicit_package(&source, &context.cwd) {
                Ok(package) => place(
                    &mut packages,
                    SourcedPackage {
                        package,
                        project: false,
                    },
                ),
                Err(error) => diagnostics.push(Diagnostic::error(format!(
                    "Failed to load extension \"{source}\": {error}"
                ))),
            }
        }
        // X-EXCLUDE: by id after composition (`-e` replacement-in-place happened above),
        // before instantiation; providers are excluded the same way.
        apply_exclusions(&mut packages, &context.exclude_extensions, &mut diagnostics);
        let wasm = if packages.is_empty() {
            None
        } else {
            let config = HostConfig::new(context.agent_dir.clone(), context.cwd.clone());
            // Resolves the process-shared engine (E-ENGINE): repeated loaders (ACP sessions,
            // a multi-cwd host) reuse one engine, one ticker thread and the compile caches.
            match WasmRuntime::new(WasmRuntimeOptions::new(config)) {
                Ok(runtime) => {
                    crate::browser::install(&runtime, &context.agent_dir);
                    Some(CoreLoader::new(Arc::new(runtime), packages))
                }
                Err(error) => {
                    diagnostics.push(Diagnostic::error(format!(
                        "Failed to start the extension host: {error}"
                    )));
                    None
                }
            }
        };
        Self {
            builtins,
            wasm,
            diagnostics,
        }
    }

    /// Problems found while resolving sources.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Component load failures from the last [`ExtensionLoader::load`] (warnings).
    #[must_use]
    pub fn load_errors(&self) -> Vec<Diagnostic> {
        self.wasm.as_ref().map_or_else(Vec::new, |wasm| {
            wasm.errors
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .map(|e| {
                    Diagnostic::warning(format!(
                        "Failed to load extension \"{}\": {}",
                        e.extension, e.message
                    ))
                })
                .collect()
        })
    }
}

impl ExtensionLoader for CliLoader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        self.load_enabled(host, project_trusted, &[])
    }

    fn load_enabled<'a>(
        &'a self,
        host: Arc<dyn HostApi>,
        project_trusted: bool,
        disabled: &'a [String],
    ) -> BoxFuture<'a, Vec<Arc<dyn ExtensionInstance>>> {
        Box::pin(async move {
            let mut instances: Vec<Arc<dyn ExtensionInstance>> = self
                .builtins
                .iter()
                .map(|builtin| builtin.instantiate(&host))
                .collect();
            if let Some(wasm) = &self.wasm {
                instances.extend(wasm.load_enabled(host, project_trusted, disabled).await);
            }
            instances
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(id: &str) -> SourcedPackage {
        let manifest = oino_extension_api::manifest::Manifest::parse(&format!(
            r#"{{"apiVersion":"{}","id":"{id}","name":"{id}","version":"1.0.0"}}"#,
            oino_extension_api::manifest::API_VERSION
        ))
        .unwrap();
        SourcedPackage {
            package: LoadedPackage {
                manifest,
                component: oino_extension_api::ComponentSource::Bytes(Arc::from(&b""[..])),
                root: None,
                bundled: false,
            },
            project: false,
        }
    }

    fn messages(diagnostics: &[Diagnostic]) -> Vec<String> {
        diagnostics.iter().map(|d| d.message.clone()).collect()
    }

    #[test]
    fn exclusion_drops_matching_ids_with_a_diagnostic() {
        let mut packages = vec![package("unipi.notify"), package("oino.anthropic")];
        let mut diagnostics = Vec::new();
        apply_exclusions(
            &mut packages,
            &["unipi.notify".to_owned(), "oino.anthropic".to_owned()],
            &mut diagnostics,
        );
        assert!(packages.is_empty());
        assert_eq!(
            messages(&diagnostics),
            vec![
                "Excluded extension \"unipi.notify\" not loaded".to_owned(),
                "Excluded extension \"oino.anthropic\" not loaded".to_owned(),
            ]
        );
    }

    #[test]
    fn exclusion_of_an_unmatched_id_only_warns() {
        let mut packages = vec![package("unipi.notify")];
        let mut diagnostics = Vec::new();
        apply_exclusions(
            &mut packages,
            &["does.not-exist".to_owned(), "unipi.notify".to_owned()],
            &mut diagnostics,
        );
        assert!(packages.is_empty());
        assert_eq!(
            messages(&diagnostics),
            vec![
                "Excluded extension \"unipi.notify\" not loaded".to_owned(),
                "--exclude-extension \"does.not-exist\" matched no extension package".to_owned(),
            ]
        );
        // A warning, never an error: startup continues.
        assert!(
            diagnostics
                .iter()
                .all(|d| d.severity == crate::cli::args::Severity::Warning)
        );
    }

    #[test]
    fn duplicate_exclusions_do_not_warn() {
        let mut packages = vec![package("unipi.notify")];
        let mut diagnostics = Vec::new();
        apply_exclusions(
            &mut packages,
            &["unipi.notify".to_owned(), "unipi.notify".to_owned()],
            &mut diagnostics,
        );
        assert!(packages.is_empty());
        assert_eq!(messages(&diagnostics).len(), 1);
    }

    #[test]
    fn no_exclusions_is_a_noop() {
        let mut packages = vec![package("unipi.notify")];
        let mut diagnostics = Vec::new();
        apply_exclusions(&mut packages, &[], &mut diagnostics);
        assert_eq!(packages.len(), 1);
        assert_eq!(diagnostics.len(), 0);
    }
}
