//! [`Boot`]: everything the app needs before GPUI starts, and the window launch.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use gpui_kit::{App, AppContext as _, Global, WindowOptions};
use oino_ui::host::SessionHost;
use oino_ui::mobile::{Launch, MobileShell};
use oino_ui::rt::Tokio;
use oino_ui::views;

use crate::{demo, embedded, logging, platform};

/// Marker file that selects the scripted demo backend (no providers or API keys needed).
///
/// Its lines are `key=value` pairs: `scene=` / `theme=` configure the demo backend (the
/// desktop `OINO_DESKTOP_DEMO_SCENE` / `OINO_DESKTOP_THEME`), the rest is a [`Launch`]
/// (`open=new`, `sheet=model`, `drawer=1`, …) used by `scripts/screenshots.sh`.
pub const DEMO_MARKER: &str = "mobile-demo";

/// Demo backend variables read from the marker (`key` → environment variable).
const DEMO_ENV: [(&str, &str); 5] = [
    ("approve", "OINO_DESKTOP_DEMO_APPROVE_MS"),
    ("scene", "OINO_DESKTOP_DEMO_SCENE"),
    ("theme", "OINO_DESKTOP_THEME"),
    ("pace", "OINO_DESKTOP_DEMO_PACE"),
    ("hour", "OINO_LOCAL_HOUR"),
];

/// Exports the demo variables of the marker before any other thread of ours exists.
#[allow(
    unsafe_code,
    reason = "std::env::set_var is unsafe in edition 2024; called during boot before the tokio \
              runtime and GPUI start, and only the demo backend reads these variables"
)]
fn export_demo_env(marker: &str) {
    for line in marker.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if let Some((_, var)) = DEMO_ENV.iter().find(|(k, _)| *k == key.trim()) {
            // SAFETY: see the function's allow reason (no concurrent environment access).
            unsafe { std::env::set_var(var, value.trim()) };
        }
    }
}

/// Keeps the tokio runtime `oino-core` runs on alive for the app's lifetime.
struct Runtime(#[allow(dead_code, reason = "held for its Drop")] tokio::runtime::Runtime);

impl Global for Runtime {}

/// Agent dir, runtime and host, built before the GPUI application.
pub struct Boot {
    runtime: tokio::runtime::Runtime,
    host: Arc<dyn SessionHost>,
    launch: Launch,
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
        let marker = std::fs::read_to_string(agent_dir.join(DEMO_MARKER)).ok();
        if let Some(m) = &marker {
            export_demo_env(m);
        }
        logging::init(&agent_dir);
        platform::set_agent_dir(agent_dir.clone());
        let cwd = agent_dir.join("workspace");
        std::fs::create_dir_all(&cwd).context("creating the workspace dir")?;
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .thread_name("oino-core")
            .enable_all()
            .build()
            .context("starting the tokio runtime")?;
        let launch = marker.as_deref().map(Launch::parse).unwrap_or_default();
        let host: Arc<dyn SessionHost> = if marker.is_some() {
            tracing::info!("demo backend");
            Arc::new(demo::DemoHost::new(agent_dir, cwd))
        } else {
            // Bundled resources are extracted inside the runtime (blocking I/O off the UI).
            let _guard = runtime.enter();
            Arc::new(embedded::EmbeddedHost::with_browser(agent_dir, cwd, None))
        };
        tracing::info!(info = ?host.info(), "oino mobile booted");
        Ok(Self {
            runtime,
            host,
            launch,
        })
    }

    /// Installs globals and opens the full-screen window (call inside the GPUI app callback).
    pub fn launch(self, cx: &mut App) {
        gpui_kit::init(cx);
        Tokio::init(cx, self.runtime.handle().clone());
        cx.set_global(Runtime(self.runtime));
        views::init(cx);
        platform::hooks().init(cx);
        let (host, launch) = (self.host, self.launch);
        let options = WindowOptions {
            window_bounds: None,
            app_id: Some("dev.oino.mobile".into()),
            ..WindowOptions::default()
        };
        let opened = gpui_kit::open_window(options, cx, move |window, cx| {
            cx.new(|cx| MobileShell::new(host, launch, window, cx))
        });
        if let Err(error) = opened {
            tracing::error!(%error, "opening the window");
        }
        cx.activate(true);
    }
}
