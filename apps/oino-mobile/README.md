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

## Layout (design.md §6)

The shell composes the shared design system at `Density::Touch` (rows 56, tool rows 32,
targets 44, UI text 15, editable text 16) — see [DESIGN-STATUS.md](DESIGN-STATUS.md) for the
screen-by-screen status and screenshots in `docs/dev/v1/design/shots/mobile/`.

- **M1 Sessions**: app bar with the aggregate Unicrab glyph, search, settings; "Sessions"
  title with Display; Needs-you card; workspace sections; rows swipe left (Pin, Delete) and
  long-press (Open, Rename, Fork, Copy id, Pin, Delete); pull to refresh; New session pill.
- **M2 Conversation**: app bar (back, title + "workspace · model", status dot, side drawer,
  actions), status capsule row (→ Status sheet), the shared `SessionPane` (timeline,
  compressed composer, meta row top-2).
- **M3 Interactions**: approvals, questions and plan reviews dock above the composer with
  stacked 44 pt buttons; small dialogs, plan reviews and docks taller than half the screen
  become bottom sheets (pending ones are not dismissible).
- **M4 Right drawer** (Review, Plan, Agent, `sidePane` panels), **M5 pages** (diff, output,
  overlay panels), **M6 sheets** (model + thinking, actions, status, rename, confirm, row
  menus, display, setting editor), **M7 search**, **M8 settings** (stacked pages, extension
  settings hub), **M9 board** (lane pager), **M10 hosts / pairing**.
- Android back (= `escape`) and the iOS edge swipe pop menu → sheet → drawer → page.
  Safe-area and keyboard insets pad the window; the soft keyboard follows text focus.
- Links open in the in-app browser (Android Custom Tabs via `OinoBrowser.java`, iOS
  `SFSafariViewController`); `osNotify` and pending interactions post local notifications
  while backgrounded; the iOS badge counts sessions that need you.

## Data

| | Android | iOS |
|---|---|---|
| Agent dir (`~/.oino`) | `Context.getFilesDir()/oino` | `Documents/oino` |
| New sessions start in | `<agent>/workspace` | `<agent>/workspace` |
| Preferences | `<agent>/mobile.json` | same |
| Logs | `<agent>/logs/mobile.log` + logcat tag `oino` | `<agent>/logs/mobile.log` + stderr |

Providers need credentials: open a session, ⋯ → "Log in to a provider" (or send `/login`).
A scripted demo backend (no providers or keys) is selected by the marker file
`<agent>/mobile-demo` (`build.sh android --demo` creates it on debug builds). Its
`key=value` lines pick the demo scene / theme and where the shell starts (`open=new`,
`sheet=model`, `drawer=1`, …); `scripts/screenshots.sh` uses them to capture every screen.

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

### iOS (macOS host `mac`, user desmondgiam)

Requirements on the Mac: Xcode 16+, `brew install xcodegen`, `rustup target add
aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios` (the simulator target follows the
Mac's CPU), and `OINO_IOS_TEAM=<Apple team id>` for device builds. Simulator builds are
unsigned.

```sh
# from this (Linux) machine: rsync the tree to mac:~/oino-build and build there
apps/oino-mobile/scripts/ios-remote.sh --simulator --no-wasm --demo --screenshot apps/oino-mobile/dist/ios-sim.png
OINO_IOS_TEAM=<team> apps/oino-mobile/scripts/ios-remote.sh --device
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
