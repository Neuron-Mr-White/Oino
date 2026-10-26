# Vendored gpui-mobile

GPUI `Platform` implementation for iOS (Metal) and Android (Vulkan/GLES) used by
`apps/oino-mobile` (gpui-app.md §2, §4).

- Upstream: <https://github.com/longbridge/gpui-mobile> at `c24a130` (2026-10-14), the
  Longbridge packaging of <https://github.com/itsbalamurali/gpui-mobile> (whose `main`,
  `5fd54f4`, merges the same changes but still pins gpui-pre =0.3.6). This fork already pins
  `gpui-pre =0.3.9` / `gpui-pre-wgpu =0.3.9`, the same GPUI as `gpui-kit =0.8.0`.
- Package name `gpui-pre-mobile`, lib name `gpui_mobile`.
- License: tri-licensed `GPL-3.0-or-later OR AGPL-3.0-or-later OR Apache-2.0`; **Oino uses it
  under Apache-2.0** (`LICENSE-APACHE`). The GPL/AGPL texts are kept because the upstream
  package metadata references them.

## Contents

`Cargo.toml`, `src/`, `README.md`, license files. Not vendored: `example/` (its Android
Java glue lives, trimmed, in `apps/oino-mobile/android/app/src/main/java/dev/gpui/mobile/`),
`tests/`, `screenshots/`, `vendor/gpui-pre-wgpu` (only needed by `offscreen`).

## Oino patches

1. `Cargo.toml`: dropped the `offscreen` feature, its `image` dependency, the
   `[patch.crates-io]` of `gpui-pre-wgpu`, dev-dependencies and profiles (the apps workspace
   owns those); `crate-type = ["rlib"]` (the app crate produces the cdylib/staticlib);
   `unexpected_cfgs` allowed for the remaining `cfg(feature = "offscreen")` sites.
2. `src/offscreen.rs` removed.
3. `Cargo.toml` ends with an empty `[workspace]` and the crate has a `rustfmt.toml` with
   `disable_all_formatting = true`: it stays out of the root workspace and `cargo fmt --all`
   leaves the upstream sources untouched.
4. `src/ios/ffi.rs`: `set_app_configure` — a hook applied to the `Application` before
   `run_embedded`, so the iOS host can install the gpui-kit asset source (icons). Android
   already has `Application::with_assets` on the `android_main` path.
5. `src/android/jni.rs` + `src/android/window.rs`: `Java_dev_gpui_mobile_GpuiInputActivity_nativeInsets`
   receives system-bar/cutout and IME insets from the activity's
   `OnApplyWindowInsetsListener` (`apps/oino-mobile/android/.../GpuiInputActivity.java`);
   `safe_area_insets()` is the max of those and the content-rect insets, and the IME inset
   feeds `keyboard_height()`. Edge-to-edge windows (enforced at targetSdk 35+) otherwise
   report zero insets and the keyboard height was never set on Android.

Default features are off in `apps/Cargo.toml`; Oino enables `clipboard`, `notifications`,
`file_selector`, `share`, `path_provider`, `shared_preferences`, `deeplink`.

## Updating

Re-copy `src/` + `Cargo.toml` from a newer upstream commit whose `gpui-pre` pin matches
`apps/Cargo.toml`, re-apply the patches above, and check `cargo tree -d -i gpui-pre` is empty.
