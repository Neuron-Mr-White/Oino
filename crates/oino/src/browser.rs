//! The process-wide in-app browser service for the `oino` binary (browser-control.md §6.4,
//! §14): one `BrowserService` per process, installed into every extension host.
//!
//! `OINO_BROWSER=off|on|ephemeral|headless|window` overrides `browser.enabled` /
//! `browser.display` for the process (`ephemeral` = temporary profile, deleted at exit).

use std::path::Path;
use std::sync::{Arc, OnceLock};

use oino_browser::config::DisplayChoice;
use oino_browser::{BrowserOptions, BrowserService};
use oino_extension_api::browser::BrowserHost;
use oino_wasm_host::WasmRuntime;

static SERVICE: OnceLock<Option<BrowserService>> = OnceLock::new();

/// The process's browser service (created on first use; `None` when off).
#[must_use]
pub fn service(agent_dir: &Path) -> Option<BrowserService> {
    SERVICE
        .get_or_init(|| {
            let mut options = BrowserOptions::for_agent_dir(agent_dir);
            match std::env::var("OINO_BROWSER").as_deref() {
                Ok("off") => options.settings.enabled = false,
                Ok("ephemeral") => {
                    options.ephemeral = true;
                    options.settings.display = DisplayChoice::Headless;
                }
                Ok("headless") => options.settings.display = DisplayChoice::Headless,
                Ok("window") => options.settings.display = DisplayChoice::Window,
                _ => {}
            }
            options
                .settings
                .enabled
                .then(|| BrowserService::new(options))
        })
        .clone()
}

/// Installs the browser service into `runtime`'s local services.
pub fn install(runtime: &WasmRuntime, agent_dir: &Path) {
    if let (Some(hub), Some(svc)) = (runtime.local_services(), service(agent_dir)) {
        hub.set_browser(Some(Arc::new(svc) as Arc<dyn BrowserHost>));
    }
}
