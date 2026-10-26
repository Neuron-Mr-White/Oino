#!/usr/bin/env bash
# Build Oino Mobile (gpui-app §6). `cargo xtask mobile …` delegates here.
#
#   apps/oino-mobile/scripts/build.sh android [--release] [--no-wasm] [--no-run] [--demo]
#                                             [--serial <adb serial>]
#   apps/oino-mobile/scripts/build.sh ios     [--release] [--no-wasm] [--no-run]
#                                             [--device | --simulator]
#
# android (Linux or macOS host + USB device):
#   1. `cargo xtask wasm` (bundle input; skip with --no-wasm when extensions/dist is current)
#   2. cargo ndk -t arm64-v8a -P 26 → android/app/src/main/jniLibs/arm64-v8a/liboino_mobile.so
#   3. Gradle assembleDebug / assembleRelease → apps/oino-mobile/dist/oino-mobile-<v>-<p>.apk
#   4. unless --no-run: adb install + launch on every attached device (or --serial /
#      $ANDROID_SERIAL). --demo also creates the demo-backend marker (debug builds only).
#   Needs: Android SDK ($ANDROID_HOME, default ~/Android/Sdk), an NDK (r26+; newest under
#   $ANDROID_HOME/ndk unless $ANDROID_NDK_HOME is set), JDK 17, `cargo install cargo-ndk`,
#   `rustup target add aarch64-linux-android`.
#
# ios (macOS host only — run remotely with scripts/ios-remote.sh):
#   1. `cargo xtask wasm` (as above)
#   2. cargo build --lib -p oino-mobile --target aarch64-apple-ios[-sim]
#   3. xcodegen + xcodebuild (ios/project.yml) → OinoMobile .app
#   4. unless --no-run: install + launch with devicectl (device) or simctl (simulator)
#   Needs: Xcode 16+, `brew install xcodegen`, `rustup target add aarch64-apple-ios
#   aarch64-apple-ios-sim`, OINO_IOS_TEAM=<Apple team id> for device builds.
#
# Memory: set JOBS (default 2); GPUI + wasmtime are heavy to compile.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
mobile="$(cd "$here/.." && pwd)"
apps="$(cd "$mobile/.." && pwd)"
root="$(cd "$apps/.." && pwd)"
dist="$mobile/dist"

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31merror:\033[0m %s\n' "$*" >&2; exit 1; }

[[ $# -ge 1 ]] || { sed -n '2,27p' "$0"; exit 2; }
platform="$1"; shift
case "$platform" in
  android|ios) ;;
  -h|--help) sed -n '2,27p' "$0"; exit 0 ;;
  *) die "unknown platform: $platform (android | ios)" ;;
esac

profile=debug
cargo_profile=()
wasm=1
run=1
demo=0
serial="${ANDROID_SERIAL:-}"
ios_target=device
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile=release; cargo_profile=(--release) ;;
    --no-wasm) wasm=0 ;;
    --no-run) run=0 ;;
    --demo) demo=1 ;;
    --serial) serial="$2"; shift ;;
    --device) ios_target=device ;;
    --simulator) ios_target=simulator ;;
    -h|--help) sed -n '2,27p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
  shift
done

jobs="${JOBS:-2}"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$apps/Cargo.toml" | head -1)"
mkdir -p "$dist"

if [[ $wasm == 1 ]]; then
  log "building extension components (cargo xtask wasm)"
  (cd "$root" && cargo xtask wasm)
fi

# ── Android ──────────────────────────────────────────────────────────────────

android() {
  export ANDROID_HOME="${ANDROID_HOME:-${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}}"
  [[ -d $ANDROID_HOME ]] || die "Android SDK not found (set ANDROID_HOME)"
  if [[ -z "${ANDROID_NDK_HOME:-}" ]]; then
    ANDROID_NDK_HOME="$(ls -d "$ANDROID_HOME"/ndk/* 2>/dev/null | sort -V | tail -1)"
    export ANDROID_NDK_HOME
  fi
  [[ -d "${ANDROID_NDK_HOME:-}" ]] || die "Android NDK not found (set ANDROID_NDK_HOME)"
  # cargo-ndk warns when the NDK variables disagree.
  export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME" NDK_HOME="$ANDROID_NDK_HOME"
  command -v cargo-ndk >/dev/null || die "cargo-ndk missing: cargo install cargo-ndk"
  rustup target add aarch64-linux-android >/dev/null 2>&1 || true
  local adb="$ANDROID_HOME/platform-tools/adb"
  command -v adb >/dev/null && adb="$(command -v adb)"

  log "building liboino_mobile.so (arm64-v8a, $profile, NDK $(basename "$ANDROID_NDK_HOME"))"
  (cd "$apps" && cargo ndk -t arm64-v8a -P 26 -o "$mobile/android/app/src/main/jniLibs" \
    build -j "$jobs" -p oino-mobile "${cargo_profile[@]}")
  local so="$mobile/android/app/src/main/jniLibs/arm64-v8a/liboino_mobile.so"
  [[ -f $so ]] || die "missing $so"
  log "native library: $(du -h "$so" | cut -f1)"

  local task=assembleDebug
  [[ $profile == release ]] && task=assembleRelease
  log "gradle $task"
  (cd "$mobile/android" && ./gradlew --quiet "$task")
  local apk="$mobile/android/app/build/outputs/apk/$profile/app-$profile.apk"
  [[ -f $apk ]] || die "missing $apk"
  local out="$dist/oino-mobile-$version-$profile.apk"
  cp "$apk" "$out"
  (cd "$dist" && sha256sum "$(basename "$out")" > "$(basename "$out").sha256")
  log "APK: $out ($(du -h "$out" | cut -f1))"

  [[ $run == 1 ]] || return 0
  local devices
  if [[ -n $serial ]]; then
    devices="$serial"
  else
    devices="$("$adb" devices | awk 'NR > 1 && $2 == "device" { print $1 }')"
  fi
  [[ -n $devices ]] || { log "no device attached; APK left at $out"; return 0; }
  for device in $devices; do
    log "installing on $device"
    "$adb" -s "$device" install -r "$out"
    if [[ $demo == 1 ]]; then
      "$adb" -s "$device" shell run-as dev.oino.mobile sh -c \
        'mkdir -p files/oino && touch files/oino/mobile-demo' \
        || log "could not create the demo marker (release build?)"
    fi
    "$adb" -s "$device" shell am start -n dev.oino.mobile/dev.gpui.mobile.GpuiActivity >/dev/null
    log "launched on $device — logs: adb -s $device logcat -s oino gpui-mobile RustStdoutStderr"
  done
}

# ── iOS ──────────────────────────────────────────────────────────────────────

ios() {
  [[ "$(uname -s)" == Darwin ]] || die "iOS builds need a macOS host (see scripts/ios-remote.sh)"
  command -v xcodegen >/dev/null || die "xcodegen missing: brew install xcodegen"
  local target sdk
  if [[ $ios_target == simulator ]]; then
    target=aarch64-apple-ios-sim; sdk=iphonesimulator
  else
    target=aarch64-apple-ios; sdk=iphoneos
    [[ -n "${OINO_IOS_TEAM:-}" ]] || die "set OINO_IOS_TEAM to your Apple development team id"
  fi
  rustup target add "$target" >/dev/null 2>&1 || true

  log "building liboino_mobile.a ($target, $profile)"
  (cd "$apps" && cargo build -j "$jobs" --lib -p oino-mobile --target "$target" "${cargo_profile[@]}")
  [[ -f "$apps/target/$target/$profile/liboino_mobile.a" ]] || die "missing static library"

  local config=Debug
  [[ $profile == release ]] && config=Release
  log "xcodegen + xcodebuild ($config, $sdk)"
  (cd "$mobile/ios" && OINO_IOS_TEAM="${OINO_IOS_TEAM:-}" xcodegen generate --spec project.yml --quiet)
  local derived="$mobile/ios/build"
  local destination="generic/platform=iOS"
  [[ $ios_target == simulator ]] && destination="generic/platform=iOS Simulator"
  (cd "$mobile/ios" && xcodebuild -project OinoMobile.xcodeproj -scheme OinoMobile \
    -configuration "$config" -sdk "$sdk" -destination "$destination" \
    -derivedDataPath "$derived" -allowProvisioningUpdates build | tail -20)
  local app
  app="$(find "$derived/Build/Products/$config-$sdk" -maxdepth 1 -name 'Oino.app' -type d | head -1)"
  [[ -n $app ]] || die "Oino.app not found under $derived"
  rm -rf "$dist/Oino-$version-$sdk.app"
  cp -R "$app" "$dist/Oino-$version-$sdk.app"
  log "app: $dist/Oino-$version-$sdk.app"

  [[ $run == 1 ]] || return 0
  if [[ $ios_target == simulator ]]; then
    local sim
    sim="$(xcrun simctl list devices booted | grep -Eo '[0-9A-F-]{36}' | head -1)"
    if [[ -z $sim ]]; then
      sim="$(xcrun simctl list devices available | grep iPhone | grep -Eo '[0-9A-F-]{36}' | head -1)"
      xcrun simctl boot "$sim"
    fi
    xcrun simctl install "$sim" "$app"
    xcrun simctl launch "$sim" dev.oino.mobile
  else
    local device
    device="$(xcrun devicectl list devices 2>/dev/null | awk '/connected/ { print $3; exit }')"
    [[ -n $device ]] || { log "no iOS device connected; app left at $app"; return 0; }
    xcrun devicectl device install app --device "$device" "$app"
    xcrun devicectl device process launch --device "$device" dev.oino.mobile
  fi
}

"$platform"
