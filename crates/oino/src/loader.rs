//! Extension sources: the components bundled into this binary (D14, `oino-bundle`), installed
//! packages (`~/.oino/extensions`, trust-gated `<cwd>/.oino/extensions`), `-e/--extension`
//! package directories and `builtin:<id>` test components.
//!
//! Load order: bundled packages in bundle order (providers, then unipi `LOAD_ORDER`); an
//! installed or `-e` package with the id of an earlier one replaces it in place, others are
//! appended (global, project, then `-e`). `OINO_BUNDLED=0` skips the bundled packages.

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
        let wasm = if packages.is_empty() {
            None
        } else {
            let config = HostConfig::new(context.agent_dir.clone(), context.cwd.clone());
            match WasmRuntime::new(WasmRuntimeOptions::new(config)) {
                Ok(runtime) => Some(CoreLoader::new(Arc::new(runtime), packages)),
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
        Box::pin(async move {
            let mut instances: Vec<Arc<dyn ExtensionInstance>> = self
                .builtins
                .iter()
                .map(|builtin| builtin.instantiate(&host))
                .collect();
            if let Some(wasm) = &self.wasm {
                instances.extend(wasm.load(host, project_trusted).await);
            }
            instances
        })
    }
}
