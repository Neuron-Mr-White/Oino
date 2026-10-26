//! `-e/--extension` sources: `builtin:<id>` components compiled into the binary and
//! component files (loaded by the WASM host once it lands, deviations/modes.md).

use std::sync::Arc;

use futures::future::BoxFuture;
use oino_core::extensions::api::ExtensionInstance;
use oino_core::extensions::host::HostApi;
use oino_core::services::ExtensionLoader;

use crate::cli::args::Diagnostic;

/// pi `EXTENSION_LOAD_FAILURE_HINT` (D13).
pub const EXTENSION_LOAD_FAILURE_HINT: &str = "Hint: Start without extensions using \"oino -ne\".";

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
                crate::faux::FauxProvider::from_env(),
            )),
            #[cfg(feature = "faux")]
            Self::UiProbe => Arc::new(crate::faux::UiProbe::new(host)),
        }
    }
}

/// Loader for the `-e` list.
#[derive(Debug)]
pub struct CliLoader {
    builtins: Vec<Builtin>,
    diagnostics: Vec<Diagnostic>,
}

impl CliLoader {
    /// Resolves `-e` sources; unknown ones become `Failed to load extension` errors.
    #[must_use]
    pub fn new(sources: Vec<String>) -> Self {
        let mut builtins = Vec::new();
        let mut diagnostics = Vec::new();
        for source in sources {
            let error = match source.strip_prefix("builtin:") {
                Some(id) => match Builtin::parse(id) {
                    Some(builtin) => {
                        builtins.push(builtin);
                        continue;
                    }
                    None => format!("unknown built-in extension \"{id}\""),
                },
                None => "component files need the WASM extension host, which this build does not include yet".to_owned(),
            };
            diagnostics.push(Diagnostic::error(format!(
                "Failed to load extension \"{source}\": {error}"
            )));
        }
        Self {
            builtins,
            diagnostics,
        }
    }

    /// Load failures found while resolving sources.
    #[must_use]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

impl ExtensionLoader for CliLoader {
    fn load(
        &self,
        host: Arc<dyn HostApi>,
        _project_trusted: bool,
    ) -> BoxFuture<'_, Vec<Arc<dyn ExtensionInstance>>> {
        let instances = self
            .builtins
            .iter()
            .map(|builtin| builtin.instantiate(&host))
            .collect();
        Box::pin(async move { instances })
    }
}
