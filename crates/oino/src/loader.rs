//! Extension sources (`-e/--extension`, installed packages and the distribution share dir,
//! all resolved by [`oino_core::extensions::sources`] — dist.md §5) plus the `builtin:<id>`
//! test components (feature `faux`) and the [`WasmRuntime`] construction.
//!
//! Load order: share-dir packages, then `~/.oino/extensions`, then trust-gated
//! `<cwd>/.oino/extensions`, then `-e`; a package with the id of an earlier one replaces it
//! in place, so a user or `-e` copy shadows the distribution. `--exclude-extension <id>`
//! drops composed packages by id before instantiation (WP-X2). `OINO_DIST_EXTENSIONS`
//! overrides the share dir (empty: disabled).
//!
//! Every `CliLoader` builds its own [`WasmRuntime`] (the cwd is per instance), but the
//! engine, epoch-ticker thread, compile caches and local-services hub are process-shared
//! (`oino_wasm_host::EngineRegistry`, E-ENGINE): the startup path, every ACP session and a
//! multi-cwd host share one engine and compile each distinct component once per process.

use std::sync::{Arc, PoisonError};

use futures::future::BoxFuture;
use oino_core::extensions::api::ExtensionInstance;
use oino_core::extensions::host::HostApi;
use oino_core::extensions::sources::{SourcesContext, resolve};
use oino_core::services::ExtensionLoader;
use oino_wasm_host::core_bridge::{CoreLoader, SourcedPackage};
use oino_wasm_host::{HostConfig, WasmRuntime, WasmRuntimeOptions};

use crate::cli::args::Diagnostic;

/// The extension load failure hint (D13).
pub const EXTENSION_LOAD_FAILURE_HINT: &str = "Hint: Start without extensions using \"oino -ne\".";

/// Components instantiated by the binary itself (feature `faux`).
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
    pub agent_dir: std::path::PathBuf,
    /// Project cwd.
    pub cwd: std::path::PathBuf,
    /// `-ne`: skip `kind: "extension"` packages of the scanned sources (providers stay).
    pub no_extensions: bool,
    /// `--exclude-extension`: package ids dropped after composition, before instantiation
    /// (WP-X2); `builtin:` test components are not subject.
    pub exclude_extensions: Vec<String>,
}

impl LoaderContext {
    fn sources_context(&self) -> SourcesContext {
        SourcesContext {
            agent_dir: self.agent_dir.clone(),
            cwd: self.cwd.clone(),
            dist_dirs: Vec::new(),
            no_extensions: self.no_extensions,
            excluded: self.exclude_extensions.clone(),
        }
    }
}

/// Loader for share-dir, installed and `-e` extensions (`builtin:` test components).
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

fn diagnostic(d: oino_core::extensions::sources::SourceDiagnostic) -> Diagnostic {
    use oino_core::extensions::sources::SourceSeverity;
    match d.severity {
        SourceSeverity::Warning => Diagnostic::warning(d.message),
        SourceSeverity::Error => Diagnostic::error(d.message),
    }
}

/// An unrecognised `builtin:` id keeps the `-e` error shape (and names the id).
fn unknown_builtin(source: &str, id: &str) -> Diagnostic {
    Diagnostic::error(format!(
        "Failed to load extension \"{source}\": unknown built-in extension \"{id}\""
    ))
}

impl CliLoader {
    /// Resolves the package set; unknown `-e` sources become `Failed to load extension` errors.
    #[must_use]
    pub fn new(sources: &[String], context: &LoaderContext) -> Self {
        let mut builtins = Vec::new();
        let mut explicit = Vec::new();
        let mut diagnostics = Vec::new();
        for source in sources {
            if let Some(id) = source.strip_prefix("builtin:") {
                match Builtin::parse(id) {
                    Some(builtin) => builtins.push(builtin),
                    None => diagnostics.push(unknown_builtin(source, id)),
                }
            } else {
                explicit.push(source.clone());
            }
        }
        let resolved = resolve(&explicit, &context.sources_context());
        diagnostics.extend(resolved.diagnostics.into_iter().map(diagnostic));
        let packages = resolved
            .packages
            .into_iter()
            .map(|p| SourcedPackage {
                package: p.package,
                project: p.project,
            })
            .collect::<Vec<_>>();
        let wasm = if packages.is_empty() {
            None
        } else {
            let config = HostConfig::new(context.agent_dir.clone(), context.cwd.clone());
            // Resolves the process-shared engine (E-ENGINE): repeated loaders (ACP sessions,
            // a multi-cwd host) reuse one engine, one ticker thread and the compile caches.
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
