//! [`Boot`]: everything the app needs before GPUI starts, and the window launch.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use gpui_kit::{App, AppContext as _, Global, WindowOptions};
use oino_ui::host::SessionHost;
use oino_ui::mobile::MobileShell;
use oino_ui::rt::Tokio;
use oino_ui::views;

use crate::{demo, embedded, logging, platform};

/// Marker file that selects the scripted demo backend (no providers or API keys needed).
pub const DEMO_MARKER: &str = "mobile-demo";

/// Keeps the tokio runtime `oino-core` runs on alive for the app's lifetime.
struct Runtime(#[allow(dead_code, reason = "held for its Drop")] tokio::runtime::Runtime);

impl Global for Runtime {}

/// Agent dir, runtime and host, built before the GPUI application.
pub struct Boot {
    runtime: tokio::runtime::Runtime,
    host: Arc<dyn SessionHost>,
}

impl std::fmt::Debug for Boot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Boot")
            .field("host", &self.host.info())
            .finish_non_exhaustive()
    }
}

impl Boot {
    /// Prepares `agent_dir` (the app-private `~/.oino`): logging, a 4-worker tokio runtime and
    /// the session host. New sessions start in `<agent_dir>/workspace`.
    ///
    /// # Errors
    ///
    /// Creating the directories or the runtime failed.
    pub fn new(agent_dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&agent_dir).context("creating the agent dir")?;
        logging::init(&agent_dir);
        let cwd = agent_dir.join("workspace");
        std::fs::create_dir_all(&cwd).context("creating the workspace dir")?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("oino-core")
            .enable_all()
            .build()
            .context("starting the tokio runtime")?;
        let host: Arc<dyn SessionHost> = if agent_dir.join(DEMO_MARKER).exists() {
            tracing::info!("demo backend");
            Arc::new(demo::DemoHost::new(agent_dir, cwd))
        } else {
            // Bundled resources are extracted inside the runtime (blocking I/O off the UI).
            let _guard = runtime.enter();
            Arc::new(embedded::EmbeddedHost::new(agent_dir, cwd))
        };
        tracing::info!(info = ?host.info(), "oino mobile booted");
        Ok(Self { runtime, host })
    }

    /// Installs globals and opens the full-screen window (call inside the GPUI app callback).
    pub fn launch(self, cx: &mut App) {
        gpui_kit::init(cx);
        Tokio::init(cx, self.runtime.handle().clone());
        cx.set_global(Runtime(self.runtime));
        views::init(cx);
        platform::hooks().init(cx);
        let host = self.host;
        let options = WindowOptions {
            window_bounds: None,
            app_id: Some("dev.oino.mobile".into()),
            ..WindowOptions::default()
        };
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| MobileShell::new(host, window, cx))
        });
        if let Err(error) = opened {
            tracing::error!(%error, "opening the window");
        }
        cx.activate(true);
    }
}
