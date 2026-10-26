//! iOS entry point: `main.m` calls [`oino_ios_register_app`], then `gpui_ios_run_demo`
//! (gpui-mobile) starts GPUI inside UIKit's run loop.

use gpui_kit::{App, Application};

use crate::app::Boot;

/// `log` → `stderr` (visible in `devicectl --console` / Xcode); `tracing` also goes to
/// `<agent>/logs/mobile.log`.
struct StderrLog;

impl log::Log for StderrLog {
    fn enabled(&self, _: &log::Metadata<'_>) -> bool {
        true
    }

    #[allow(clippy::print_stderr, reason = "the platform console is the log sink")]
    fn log(&self, record: &log::Record<'_>) {
        eprintln!(
            "[{}] {}: {}",
            record.level(),
            record.target(),
            record.args()
        );
    }

    fn flush(&self) {}
}

static LOGGER: StderrLog = StderrLog;

/// Registers the Oino app with the GPUI iOS platform; call before `gpui_ios_run_demo`.
#[allow(
    unsafe_code,
    reason = "called from Objective-C (`main.m`) by its unmangled symbol name"
)]
#[unsafe(no_mangle)]
pub extern "C" fn oino_ios_register_app() {
    if log::set_logger(&LOGGER).is_ok() {
        log::set_max_level(log::LevelFilter::Info);
    }
    // `Documents/oino` is the app-private `~/.oino` (backed up with the app).
    let agent_dir = match gpui_mobile::packages::path_provider::documents_directory() {
        Ok(dir) => dir.join("oino"),
        Err(error) => {
            log::error!("oino_ios_register_app: no documents directory: {error}");
            return;
        }
    };
    let boot = match Boot::new(agent_dir) {
        Ok(boot) => boot,
        Err(error) => {
            log::error!("oino_ios_register_app: boot failed: {error:#}");
            return;
        }
    };
    gpui_mobile::ios::ffi::set_app_configure(Box::new(|application: Application| {
        application.with_assets(gpui_kit::assets::Assets)
    }));
    gpui_mobile::ios::ffi::set_app_callback(Box::new(move |cx: &mut App| boot.launch(cx)));
}
