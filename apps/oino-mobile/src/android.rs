//! Android entry point: `NativeActivity` (via `GpuiActivity` ⊂ `GpuiInputActivity`) loads
//! `liboino_mobile.so`; `android-activity` calls [`android_main`] on its native thread.

use android_activity::AndroidApp;
use gpui_kit::{App, Application};
use gpui_mobile::android::jni;

use crate::app::Boot;

/// The app entry point; returns when the activity is destroyed.
#[allow(
    unsafe_code,
    clippy::no_mangle_with_rust_abi,
    clippy::needless_pass_by_value,
    reason = "android-activity declares `extern \"Rust\" fn android_main(app: AndroidApp)` and \
              links it by its unmangled name"
)]
#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("oino"),
    );
    jni::install_panic_hook();
    // `Context.getFilesDir()/oino` is the app-private `~/.oino`.
    let Some(agent_dir) = app.internal_data_path().map(|dir| dir.join("oino")) else {
        log::error!("android_main: no internal data path");
        return;
    };
    let boot = match Boot::new(agent_dir) {
        Ok(boot) => boot,
        Err(error) => {
            log::error!("android_main: boot failed: {error:#}");
            return;
        }
    };
    let _platform = jni::init_platform(&app);
    let Some(shared) = jni::shared_platform() else {
        log::error!("android_main: no shared platform");
        return;
    };
    // `Platform::run` drives the Android event loop and runs the callback once the native
    // window exists; it returns when the activity finishes.
    Application::with_platform(shared.into_rc())
        .with_assets(gpui_kit::assets::Assets)
        .run(move |cx: &mut App| boot.launch(cx));
    log::info!("android_main: event loop exited");
}
