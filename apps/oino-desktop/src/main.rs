//! Oino Desktop: the GPUI window shell over an in-process `oino-core` with every bundled
//! provider and unipi component (gpui-app.md §3).
//!
//! ```text
//! oino-desktop [--demo] [--cwd <dir>]
//! ```
//!
//! - `--demo` (or `OINO_DESKTOP_DEMO=1`): scripted backend, no providers needed.
//! - `--cwd <dir>`: directory for the first session (default: the process cwd, or `$HOME`
//!   when launched from a desktop entry at `/`).
//!
//! Threads: GPUI owns the main thread; `oino-core` runs on a 4-worker tokio runtime whose
//! handle lives in the `oino_ui::rt::Tokio` global. Logs go to `~/.oino/logs/desktop.log`
//! (`RUST_LOG` filters, default `info`).

#![forbid(unsafe_code)]
#![cfg_attr(
    test,
    allow(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::print_stderr,
        reason = "tests assert by unwrapping"
    )
)]

mod demo;
mod embedded;
mod prefs;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context as _;
use gpui_kit::{
    App, AppContext as _, Bounds, TitlebarOptions, WindowBounds, WindowOptions, px, size,
};
use oino_ui::host::SessionHost;
use oino_ui::rt::Tokio;
use oino_ui::views::{self, Quit, Workspace};

struct Args {
    demo: bool,
    cwd: Option<PathBuf>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        demo: std::env::var("OINO_DESKTOP_DEMO").is_ok_and(|v| v == "1"),
        cwd: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--demo" => args.demo = true,
            "--cwd" => args.cwd = Some(it.next().context("--cwd needs a directory")?.into()),
            #[allow(clippy::print_stdout, reason = "CLI output at the binary edge")]
            "--version" | "-V" => {
                println!("oino-desktop {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            #[allow(clippy::print_stdout, reason = "CLI output at the binary edge")]
            "--help" | "-h" => {
                println!("oino-desktop [--demo] [--cwd <dir>]");
                std::process::exit(0);
            }
            other => anyhow::bail!("unknown argument: {other}"),
        }
    }
    Ok(args)
}

fn init_logging(agent_dir: &std::path::Path) {
    use tracing_subscriber::EnvFilter;
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let dir = agent_dir.join("logs");
    let file = std::fs::create_dir_all(&dir)
        .and_then(|()| {
            std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(dir.join("desktop.log"))
        })
        .ok();
    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false);
    match file {
        Some(file) => builder.with_writer(Arc::new(file)).init(),
        None => builder.with_writer(std::io::stderr).init(),
    }
}

fn default_cwd(explicit: Option<PathBuf>) -> PathBuf {
    explicit
        .or_else(|| {
            std::env::current_dir()
                .ok()
                .filter(|d| d.parent().is_some())
        })
        .or_else(oino_settings::paths::home_dir)
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Default window size (design.md §5).
const DEFAULT_SIZE: (f32, f32) = (1400.0, 900.0);
/// Minimum window size.
const MIN_SIZE: (f32, f32) = (720.0, 480.0);
/// macOS traffic-light origin inside the sidebar top row.
const TRAFFIC_LIGHTS: (f32, f32) = (16.0, 16.0);

/// Window size: `OINO_DESKTOP_SIZE=WxH` (screenshots) or the default.
fn window_size() -> (f32, f32) {
    std::env::var("OINO_DESKTOP_SIZE")
        .ok()
        .and_then(|v| {
            let (w, h) = v.split_once('x')?;
            Some((w.parse().ok()?, h.parse().ok()?))
        })
        .unwrap_or(DEFAULT_SIZE)
}

fn main() -> anyhow::Result<()> {
    let args = parse_args()?;
    let agent_dir = oino_settings::paths::agent_dir().context("locating the agent dir")?;
    init_logging(&agent_dir);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .thread_name("oino-core")
        .enable_all()
        .build()
        .context("starting the tokio runtime")?;
    let cwd = default_cwd(args.cwd);
    let host: Arc<dyn SessionHost> = if args.demo {
        Arc::new(demo::DemoHost::new(agent_dir, cwd))
    } else {
        // Bundled resources are extracted inside the runtime (blocking I/O off the UI path).
        let _guard = runtime.enter();
        Arc::new(embedded::EmbeddedHost::new(agent_dir, cwd))
    };
    let handle = runtime.handle().clone();
    gpui_kit::application()
        .with_assets(oino_ui::design::icons::OinoAssets)
        .run(move |cx: &mut App| {
            gpui_kit::init(cx);
            oino_ui::design::theme::register_fonts(cx);
            Tokio::init(cx, handle);
            views::init(cx);
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
            let (w, h) = window_size();
            let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
            let options = WindowOptions {
                window_bounds: Some(WindowBounds::Windowed(bounds)),
                window_min_size: Some(size(px(MIN_SIZE.0), px(MIN_SIZE.1))),
                titlebar: Some(TitlebarOptions {
                    title: Some("Oino".into()),
                    // macOS: content under a transparent title bar, traffic lights in the
                    // sidebar's 78 px safe area (design.md §3.1).
                    appears_transparent: cfg!(target_os = "macos"),
                    traffic_light_position: cfg!(target_os = "macos")
                        .then(|| gpui_kit::point(px(TRAFFIC_LIGHTS.0), px(TRAFFIC_LIGHTS.1))),
                }),
                app_id: Some("dev.oino.desktop".into()),
                ..WindowOptions::default()
            };
            let host = Arc::clone(&host);
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| Workspace::new(host, window, cx))
            });
            if let Err(error) = opened {
                tracing::error!(%error, "opening the window");
                cx.quit();
            }
            cx.activate(true);
        });
    drop(runtime);
    Ok(())
}
