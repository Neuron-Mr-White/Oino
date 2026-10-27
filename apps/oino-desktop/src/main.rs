//! Oino Desktop: the GPUI window shell over an in-process `oino-core` with every bundled
//! provider and unipi component (gpui-app.md §3).
//!
//! ```text
//! oino-desktop [--demo] [--cwd <dir>] [--soak <seconds>]
//! ```
//!
//! - `--demo` (or `OINO_DESKTOP_DEMO=1`): scripted backend, no providers needed.
//! - `--cwd <dir>`: directory for the first session (default: the process cwd, or `$HOME`
//!   when launched from a desktop entry at `/`).
//! - `--soak <seconds>` (or `OINO_DESKTOP_SOAK`): run the soak driver, then quit
//!   (`oino_ui::views::soak`, `scripts/soak.sh`).
//!
//! Stability (docs/dev/v1/stability.md): panics write `~/.oino/logs/desktop-crash-*.log`
//! (UI-thread panics abort after logging); a watchdog logs `desktop-stall-*.log` when the UI
//! thread is silent for 2 s; on quit every session is shut down and the runtime gets 3 s.
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
    soak: Option<u64>,
}

fn parse_args() -> anyhow::Result<Args> {
    let mut args = Args {
        demo: std::env::var("OINO_DESKTOP_DEMO").is_ok_and(|v| v == "1"),
        cwd: None,
        soak: None,
    };
    let mut it = std::env::args().skip(1);
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--demo" => args.demo = true,
            "--cwd" => args.cwd = Some(it.next().context("--cwd needs a directory")?.into()),
            "--soak" => {
                let secs = it.next().context("--soak needs a duration in seconds")?;
                args.soak = Some(secs.parse().context("--soak needs a number of seconds")?);
            }
            #[allow(clippy::print_stdout, reason = "CLI output at the binary edge")]
            "--version" | "-V" => {
                println!("oino-desktop {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            #[allow(clippy::print_stdout, reason = "CLI output at the binary edge")]
            "--help" | "-h" => {
                println!("oino-desktop [--demo] [--cwd <dir>] [--soak <seconds>]");
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

/// The GPUI platform and whether the embedded browser can run on it (design.md A1.4).
///
/// Linux: `WebKitGTK` embeds only into X11 windows, so Oino runs on X11 / `XWayland` unless the
/// user chose "Native Wayland" (or no X server is reachable).
fn platform(settings: &oino_ui::host::AppSettings) -> (gpui_kit::Application, bool) {
    #[cfg(target_os = "linux")]
    {
        let has_x = std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty());
        if !settings.native_wayland && has_x {
            return (
                gpui_kit::platform::linux(gpui_kit::WindowingModes::X11),
                true,
            );
        }
        (gpui_kit::application(), false)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = settings;
        (gpui_kit::application(), true)
    }
}

/// Posts a native notification off the UI thread (`D-Bus` / `NSUserNotification` / `WinRT`).
fn os_notify(title: String, body: String) {
    let spawned = std::thread::Builder::new()
        .name("oino-notify".into())
        .spawn(move || {
            let result = notify_rust::Notification::new()
                .appname("Oino")
                .summary(&title)
                .body(&body)
                .icon("dev.oino.desktop")
                .show();
            if let Err(error) = result {
                tracing::warn!(%error, "posting an OS notification");
            }
        });
    if let Err(error) = spawned {
        tracing::warn!(%error, "starting the notification thread");
    }
}

fn main() -> anyhow::Result<()> {
    let process_start = std::time::Instant::now();
    let args = parse_args()?;
    let agent_dir = oino_settings::paths::agent_dir().context("locating the agent dir")?;
    init_logging(&agent_dir);
    let log_dir = agent_dir.join("logs");
    oino_ui::stability::install_panic_hook(&log_dir, "oino-desktop");
    let watchdog = oino_ui::stability::Watchdog::start(log_dir, oino_ui::stability::STALL);

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
    let (app, browser) = platform(&host.app_settings());
    // Session shutdown started at quit; awaited (bounded) after the event loop returns.
    let quitting: Arc<std::sync::Mutex<Option<tokio::task::JoinHandle<()>>>> = Arc::default();
    let app_host = Arc::clone(&host);
    let app_quitting = Arc::clone(&quitting);
    let beat = watchdog.clone();
    let soak = args.soak;
    app.with_assets(oino_ui::design::icons::OinoAssets)
        .run(move |cx: &mut App| {
            let host = app_host;
            heartbeat(beat, cx);
            if let Some(secs) = soak {
                // Read by `Workspace::new` (`oino_ui::views::soak`).
                cx.set_global(views::soak::Requested(secs));
            }
            {
                let host = Arc::clone(&host);
                let quitting = app_quitting;
                let handle = handle.clone();
                cx.on_app_quit(move |_| {
                    let join = handle.spawn(host.shutdown());
                    if let Ok(mut slot) = quitting.lock() {
                        *slot = Some(join);
                    }
                    async {}
                })
                .detach();
            }
            gpui_kit::init(cx);
            oino_ui::design::theme::register_fonts(cx);
            Tokio::init(cx, handle);
            cx.default_global::<oino_ui::design::perf::FrameProbe>()
                .process_start = Some(process_start);
            oino_ui::platform::OsNotifier::install(cx, Arc::new(os_notify));
            cx.set_global(oino_ui::platform::BrowserSupport(browser));
            views::init(cx);
            cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
            let options = window_options(cx);
            let host = Arc::clone(&host);
            let opened = gpui_kit::open_window(options, cx, move |window, cx| {
                cx.new(|cx| Workspace::new(host, window, cx))
            });
            match opened {
                // Closing the main window quits even while the mascot window is open (the
                // process used to linger with only the mascot).
                Ok(main) => {
                    let main = main.0.window_id();
                    cx.on_window_closed(move |cx, id| {
                        if id == main {
                            cx.quit();
                        }
                    })
                    .detach();
                }
                Err(error) => {
                    tracing::error!(%error, "opening the window");
                    cx.quit();
                }
            }
            cx.activate(true);
        });
    watchdog.stop();
    let pending = quitting.lock().ok().and_then(|mut slot| slot.take());
    runtime.block_on(async {
        let shutdown = async {
            match pending {
                Some(join) => drop(join.await),
                None => host.shutdown().await,
            }
        };
        if tokio::time::timeout(QUIT_GRACE, shutdown).await.is_err() {
            tracing::warn!("sessions did not shut down within the quit grace period");
        }
    });
    // A blocking task stuck in I/O must not hang the exit.
    runtime.shutdown_timeout(QUIT_GRACE);
    Ok(())
}

/// The main window: default size, client-side chrome on Linux / Windows (design.md §3.1).
fn window_options(cx: &mut App) -> WindowOptions {
    let (w, h) = window_size();
    let bounds = Bounds::centered(None, size(px(w), px(h)), cx);
    WindowOptions {
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
        // Linux / Windows draw their own chrome (sidebar top row + headers drag, window
        // controls in the right-most header); macOS keeps the traffic lights.
        window_decorations: (!cfg!(target_os = "macos"))
            .then_some(gpui_kit::WindowDecorations::Client),
        window_background: if cfg!(target_os = "linux") {
            gpui_kit::WindowBackgroundAppearance::Transparent
        } else {
            gpui_kit::WindowBackgroundAppearance::Opaque
        },
        app_owns_titlebar_drag: true,
        ..WindowOptions::default()
    }
}

/// Time sessions get to shut down at quit, and the runtime after that.
const QUIT_GRACE: std::time::Duration = std::time::Duration::from_secs(3);

/// Beats the watchdog from the UI thread every [`oino_ui::stability::BEAT`].
fn heartbeat(dog: oino_ui::stability::Watchdog, cx: &mut App) {
    cx.spawn(async move |cx| {
        loop {
            dog.beat();
            cx.background_executor()
                .timer(oino_ui::stability::BEAT)
                .await;
        }
    })
    .detach();
}
