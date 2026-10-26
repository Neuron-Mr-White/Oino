# Oino Mobile

The GPUI phone app for Android and iOS (gpui-app.md §4). It runs `oino-core` in-process with
every bundled provider and unipi component — the same embedded host as Oino Desktop — and
renders the touch layout from `oino_ui::mobile` on the vendored
[gpui-mobile](../vendor/gpui-mobile/VENDOR.md) platform.

```text
apps/oino-mobile/
  src/            lib (cdylib for Android, staticlib for iOS): boot, entry points, platform hooks
  android/        Gradle host (NativeActivity: dev.gpui.mobile.GpuiActivity ⊂ GpuiInputActivity)
  ios/            XcodeGen host (main.m, full-screen GPUI window + CADisplayLink)
  scripts/        build.sh (android | ios), ios-remote.sh (build on the macOS host over SSH)
apps/oino-ui/src/mobile/   MobileShell: sessions → conversation → sheets, settings
```

## Layout

- **Sessions**: sessions grouped by workspace with live dots (running / waiting on you /
  unread), a filter field and a full-width "New session" button.
- **Conversation**: the desktop `SessionPane` (virtualised transcript, streaming markdown,
  tool rows with diffs/tails, composer with `/` completion and queue, dialogs, approvals,
  OUS panels) under a 52 px app bar — back, title, model · thinking (tap → model sheet),
  stop while running, ⋯ actions (new session, compact, `/login`, settings, `sidePane`
  panels).
- **Sheets**: the model + thinking picker and the action list are bottom sheets; diffs, tool
  output and `sidePane` panels open as full-screen sheets.
- **Settings**: theme, thinking blocks, tool rows, loaded/failed extensions, version, backend,
  agent dir.
- Android back (= `escape`) pops sheet → side sheet → screen. Safe-area and keyboard insets
  pad the window; the soft keyboard follows the composer's focus.

## Data

| | Android | iOS |
|---|---|---|
| Agent dir (`~/.oino`) | `Context.getFilesDir()/oino` | `Documents/oino` |
| New sessions start in | `<agent>/workspace` | `<agent>/workspace` |
| Preferences | `<agent>/mobile.json` | same |
| Logs | `<agent>/logs/mobile.log` + logcat tag `oino` | `<agent>/logs/mobile.log` + stderr |

Providers need credentials: open a session, ⋯ → "Log in to a provider" (or send `/login`).
A scripted demo backend (no providers or keys) is selected by the marker file
`<agent>/mobile-demo` (`build.sh android --demo` creates it on debug builds).

## Build

Bundle input first (`cargo xtask wasm` → `extensions/dist`; the scripts run it unless
`--no-wasm`). Set `JOBS` (default 2) on machines with little memory.

### Android (local, USB device)

Requirements: Android SDK (`ANDROID_HOME`, default `~/Android/Sdk`) with an NDK r26+
(newest under `$ANDROID_HOME/ndk`, or `ANDROID_NDK_HOME`), JDK 17, `cargo install cargo-ndk`,
`rustup target add aarch64-linux-android`.

```sh
apps/oino-mobile/scripts/build.sh android              # debug APK, install + launch on every attached device
apps/oino-mobile/scripts/build.sh android --release --serial <adb serial>
apps/oino-mobile/scripts/build.sh android --no-wasm --demo   # demo backend
adb logcat -s oino gpui-mobile                         # logs
```

The APK lands in `apps/oino-mobile/dist/oino-mobile-<version>-<profile>.apk` (+ `.sha256`).
By hand: `cd apps && cargo ndk -t arm64-v8a -P 26 -o oino-mobile/android/app/src/main/jniLibs
build -p oino-mobile`, then `cd oino-mobile/android && ./gradlew assembleDebug`.

### iOS (macOS host `desmondgiam`, USB device)

Requirements on the Mac: Xcode 16+, `brew install xcodegen`, `rustup target add
aarch64-apple-ios aarch64-apple-ios-sim`, and `OINO_IOS_TEAM=<Apple team id>` for device
builds.

```sh
# from this (Linux) machine: rsync the tree to desmondgiam:~/oino-build and build there
OINO_IOS_TEAM=<team> apps/oino-mobile/scripts/ios-remote.sh --device
apps/oino-mobile/scripts/ios-remote.sh --simulator --no-run
# or directly on the Mac
OINO_IOS_TEAM=<team> apps/oino-mobile/scripts/build.sh ios --device
```

iOS forbids JIT, so wasmtime's Cranelift cannot run the components on a device; the
app starts and core runs, but extension components (including providers) fail to load until
`oino-wasm-host` can target Pulley (see `docs/dev/v1/deviations/gpui-mobile.md`).

## Checks

```sh
cd apps
cargo clippy -j 2 -p oino-mobile -p oino-ui --features oino-ui/mobile --all-targets -- -D warnings -W clippy::pedantic
cargo test -j 2 -p oino-mobile -p oino-ui --features oino-ui/mobile
cargo ndk -t arm64-v8a -P 26 clippy -j 2 -p oino-mobile -- -D warnings -W clippy::pedantic
```
