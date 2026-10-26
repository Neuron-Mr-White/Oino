//! Oino Mobile: the GPUI phone app for Android and iOS (gpui-app.md §4).
//!
//! The app embeds `oino-core` in-process with every bundled provider and unipi component —
//! the same [`embedded::EmbeddedHost`] the desktop shell uses (shared by source until it
//! moves into a shared crate) — and renders the touch layout from `oino_ui::mobile` on the
//! vendored `gpui-mobile` platform (`apps/vendor/gpui-mobile`, gpui-pre =0.3.9).
//!
//! # Entry points
//!
//! | Platform | Entry | Host project |
//! |---|---|---|
//! | Android | `android_main` (called by `android-activity` from `NativeActivity`) | `android/` (Gradle, `GpuiActivity` ⊂ `GpuiInputActivity`) |
//! | iOS | `oino_ios_register_app` then `gpui_ios_run_demo` (from `main.m`) | `ios/` (`XcodeGen`) |
//!
//! Both build a [`app::Boot`] (agent dir in the app sandbox, tokio runtime, host) and hand
//! [`app::Boot::launch`] to GPUI.
//!
//! # API map
//!
//! | Module | Contents |
//! |---|---|
//! | [`app`] | runtime + host construction, window launch |
//! | `embedded` | `oino-core` host (bundled + installed components), shared with desktop |
//! | `demo` | scripted backend (`<agent>/mobile-demo` marker file), shared with desktop |
//! | `prefs` | `<agent>/mobile.json` |
//! | `logging` | `tracing` → `<agent>/logs/mobile.log` (+ logcat on Android) |
//! | `platform` | soft keyboard / safe-area hooks for `oino_ui::mobile` |
//!
//! # Unsafe
//!
//! `unsafe_code` is denied crate-wide; the only exceptions are the `#[unsafe(no_mangle)]`
//! entry points the platform loaders look up by name (`android_main`,
//! `oino_ios_register_app`). Each carries its own `allow` with the reason.

#![deny(unsafe_code)]
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

pub mod app;
#[path = "../../oino-desktop/src/demo.rs"]
#[allow(
    dead_code,
    unreachable_pub,
    reason = "shared with oino-desktop by source; not every item is used here"
)]
mod demo;
#[path = "../../oino-desktop/src/embedded.rs"]
#[allow(unreachable_pub, reason = "shared with oino-desktop by source")]
mod embedded;
mod logging;
mod platform;
mod prefs;

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "ios")]
mod ios;
