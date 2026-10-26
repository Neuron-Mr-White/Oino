#!/usr/bin/env bash
# Release build on the current host (quality §6 `release` job). `cargo xtask remote-build`
# runs this locally (`local`) or on a build host after syncing the tree; it can also be run by
# hand from the repository root.
#
#   scripts/release/build-host.sh [--universal] [--no-cli] [--no-desktop] [--formats <list>]
#                                 [--no-smoke] [--no-shot] [--android [--serial <s>]]
#
# Steps:
#   1. `cargo xtask checksums`: extensions/dist must be present and current (it is not built
#      here: build hosts have no wasm toolchain; `remote-build` ships the local dist).
#   2. `cargo build --release -p oino -p oino-kanboard` (macOS --universal: aarch64 + x86_64,
#      lipo'd; the thin x86_64 binaries are kept as well).
#   3. `apps/oino-desktop/scripts/build.sh --release --no-wasm` (tar.gz/AppImage/deb on Linux,
#      .app/.dmg on macOS, NSIS on Windows).
#   4. --android: `apps/oino-mobile/scripts/build.sh android --release --no-wasm`.
#   5. Copies everything into target/release-artifacts/<target>/ with SHA256SUMS,
#      build-info.txt and smoke.txt (smoke tests: --version, a faux-provider print run with
#      the bundled components, desktop --demo launch + screenshot where a display exists).
#
# Environment: JOBS (cargo -j, default 2), OINO_SHOT_WAIT (seconds before the desktop
# screenshot, default 20).
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$root"
universal=0 cli=1 desktop=1 smoke=1 shot=1 android=0 formats="" serial=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --universal) universal=1 ;;
    --no-cli) cli=0 ;;
    --no-desktop) desktop=0 ;;
    --formats) formats="$2"; shift ;;
    --no-smoke) smoke=0 ;;
    --no-shot) shot=0 ;;
    --android) android=1 ;;
    --serial) serial="$2"; shift ;;
    -h|--help) sed -n '2,23p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

export JOBS="${JOBS:-2}"
[[ -d "$HOME/.cargo/bin" ]] && export PATH="$HOME/.cargo/bin:$PATH"
case "$(uname -s)" in
  Linux*) os=linux ;;
  Darwin*) os=macos ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) echo "unsupported host: $(uname -s)" >&2; exit 1 ;;
esac
exe=""; [[ $os == windows ]] && exe=".exe"
host="$(rustc -vV | sed -n 's/^host: //p')"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
target="$host"
[[ $os == macos && $universal == 1 ]] && target=universal-apple-darwin
art="$root/target/release-artifacts"
out="$art/$target"
smoke_log="$out/smoke.txt"

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }
# Bounded commands: timeout (coreutils) / gtimeout (Homebrew) when available.
tmo() { local t="$1"; shift
  if command -v timeout >/dev/null; then timeout "$t" "$@"
  elif command -v gtimeout >/dev/null; then gtimeout "$t" "$@"
  else "$@"; fi; }
sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
note() { printf '%s\n' "$*" | tee -a "$smoke_log"; }
# check <name> <command…>: runs a smoke step, records PASS/FAIL with the first output line.
check() {
  local name="$1"; shift
  local output status=0
  output="$("$@" 2>&1)" || status=$?
  if [[ $status == 0 ]]; then
    note "PASS $name: $(printf '%s' "$output" | head -1)"
  else
    note "FAIL $name (exit $status): $(printf '%s' "$output" | tail -3 | tr '\n' ' ')"
  fi
}

rm -rf "$out"
mkdir -p "$out"
: > "$smoke_log"

log "checking extensions/dist (cargo xtask checksums)"
cargo xtask checksums

cli_build() { # $1 = optional target triple
  local args=(build --release -j "$JOBS" -p oino -p oino-kanboard)
  [[ -n "${1:-}" ]] && args+=(--target "$1")
  log "cargo ${args[*]}"
  cargo "${args[@]}"
}

if [[ $cli == 1 ]]; then
  if [[ $target == universal-apple-darwin ]]; then
    rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
    cli_build aarch64-apple-darwin
    cli_build x86_64-apple-darwin
    for bin in oino oino-kanboard; do
      lipo -create -output "$out/$bin" "target/aarch64-apple-darwin/release/$bin" \
        "target/x86_64-apple-darwin/release/$bin"
    done
    mkdir -p "$art/x86_64-apple-darwin"
    cp target/x86_64-apple-darwin/release/{oino,oino-kanboard} "$art/x86_64-apple-darwin/"
  else
    cli_build ""
    cp "target/release/oino$exe" "target/release/oino-kanboard$exe" "$out/"
  fi
fi

if [[ $desktop == 1 ]]; then
  args=(--release --no-wasm)
  [[ $target == universal-apple-darwin ]] && args+=(--universal)
  [[ -n $formats ]] && args+=(--formats "$formats")
  rm -rf apps/oino-desktop/dist
  log "apps/oino-desktop/scripts/build.sh ${args[*]}"
  desktop_status=0
  apps/oino-desktop/scripts/build.sh "${args[@]}" || desktop_status=$?
  ddist=apps/oino-desktop/dist
  rm -f "$ddist"/rw.*.dmg # create-dmg's scratch image, left behind when it fails
  if [[ $os == macos && -d $ddist/Oino.app ]] && ! ls "$ddist"/*.dmg >/dev/null 2>&1; then
    # cargo-packager's create-dmg drives Finder over AppleScript, which times out without a
    # GUI session (SSH): build a plain compressed image with hdiutil instead.
    log "create-dmg failed; building the .dmg with hdiutil"
    dmg_arch=x64; [[ $target == universal-apple-darwin ]] && dmg_arch=universal
    dmg_stage="$(mktemp -d "${TMPDIR:-/tmp}/oino-dmg.XXXX")"
    cp -R "$ddist/Oino.app" "$dmg_stage/"
    ln -s /Applications "$dmg_stage/Applications"
    hdiutil create -volname Oino -srcfolder "$dmg_stage" -ov -format UDZO \
      "$ddist/Oino_${version}_${dmg_arch}.dmg" && desktop_status=0
    rm -rf "$dmg_stage"
  fi
  [[ $desktop_status == 0 ]] || { log "desktop build failed ($desktop_status)"; exit "$desktop_status"; }
  rm -f "$ddist"/rw.*.dmg
  for f in apps/oino-desktop/dist/*; do
    [[ $(basename "$f") == SHA256SUMS ]] && continue
    cp -R "$f" "$out/"
  done
  cp "apps/target/release/oino-desktop$exe" "$out/"
fi

if [[ $android == 1 ]]; then
  log "apps/oino-mobile/scripts/build.sh android --release --no-wasm"
  mobile_args=(android --release --no-wasm)
  if [[ -n $serial ]]; then mobile_args+=(--serial "$serial"); else mobile_args+=(--no-run); fi
  apps/oino-mobile/scripts/build.sh "${mobile_args[@]}"
  mkdir -p "$art/aarch64-linux-android"
  cp "apps/oino-mobile/dist/oino-mobile-$version-release.apk" "$art/aarch64-linux-android/"
fi

# ── smoke tests ──────────────────────────────────────────────────────────────

faux_run() { # $1 = oino binary
  local sandbox; sandbox="$(mktemp -d "${TMPDIR:-/tmp}/oino-smoke.XXXX")"
  mkdir -p "$sandbox/home" "$sandbox/agent" "$sandbox/work"
  printf '{"faux":{"type":"api_key","key":"k"}}' > "$sandbox/agent/auth.json"
  local faux="$root/extensions/dist/oino.fixture.provider-faux"
  [[ $os == windows ]] && faux="$(cygpath -w "$faux")"
  local reply status=0
  reply="$(cd "$sandbox/work" && HOME="$sandbox/home" USERPROFILE="$sandbox/home" \
    OINO_AGENT_DIR="$sandbox/agent" tmo 300 "$1" -e "$faux" --model faux/faux-1 -p hi 2>&1)" \
    || status=$?
  local bundled; bundled="$(ls "$sandbox/agent/bundled" 2>/dev/null | wc -l | tr -d ' ')"
  rm -rf "$sandbox"
  [[ $status == 0 && $reply == "Hello from faux." ]] || { echo "$reply"; return 1; }
  echo "\"$reply\" ($bundled bundled packages unpacked)"
}

desktop_shot() { # $1 = oino-desktop binary, $2 = png
  local agent; agent="$(mktemp -d "${TMPDIR:-/tmp}/oino-shot.XXXX")"
  local wait="${OINO_SHOT_WAIT:-20}"
  case $os in
    linux)
      command -v Xvfb >/dev/null || { echo "no Xvfb"; return 1; }
      local display=":$((90 + RANDOM % 9))"
      Xvfb "$display" -screen 0 1400x900x24 -nolisten tcp >/dev/null 2>&1 &
      local xvfb=$!
      sleep 1
      env -u WAYLAND_DISPLAY DISPLAY="$display" OINO_AGENT_DIR="$agent" OINO_REDUCE_MOTION=1 \
        VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}" \
        OINO_DESKTOP_SIZE=1400x900 "$1" --demo >/dev/null 2>&1 &
      local app=$!
      sleep "$wait"
      local alive=0; kill -0 "$app" 2>/dev/null && alive=1
      python3 -c "from PIL import ImageGrab; ImageGrab.grab(xdisplay='$display').save('$2')"
      kill "$app" "$xvfb" 2>/dev/null || true
      wait "$app" "$xvfb" 2>/dev/null || true
      [[ $alive == 1 ]] || { echo "app exited before the screenshot"; return 1; }
      ;;
    macos)
      OINO_AGENT_DIR="$agent" OINO_REDUCE_MOTION=1 "$1" --demo >/dev/null 2>&1 &
      local app=$!
      sleep "$wait"
      local alive=0; kill -0 "$app" 2>/dev/null && alive=1
      screencapture -x "$2" || true
      kill "$app" 2>/dev/null || true
      wait "$app" 2>/dev/null || true
      [[ $alive == 1 ]] || { echo "app exited before the screenshot"; return 1; }
      ;;
    windows)
      # SSH sessions have no desktop: launch in the console session through a scheduled task.
      powershell.exe -NoProfile -ExecutionPolicy Bypass -File \
        "$(cygpath -w "$root/scripts/release/desktop-shot.ps1")" -Exe "$(cygpath -w "$1")" \
        -Png "$(cygpath -w "$2")" -Wait "$wait" | tr -d '\r'
      [[ -f $2 ]] || return 1
      ;;
  esac
  rm -rf "$agent"
  echo "screenshot $(basename "$2") ($(wc -c < "$2" | tr -d ' ') bytes)"
}

if [[ $smoke == 1 ]]; then
  log "smoke tests"
  if [[ -f "$out/oino$exe" ]]; then
    check "oino --version" "$out/oino$exe" --version
    check "oino-kanboard --version" "$out/oino-kanboard$exe" --version
    check "oino faux print run" faux_run "$out/oino$exe"
    if [[ $target == universal-apple-darwin ]]; then
      check "lipo oino" lipo -archs "$out/oino"
      check "x86_64 oino --version" "$art/x86_64-apple-darwin/oino" --version
    fi
  fi
  if [[ $desktop == 1 ]]; then
    check "oino-desktop --version" "$out/oino-desktop$exe" --version
    [[ $target == universal-apple-darwin ]] && check "lipo oino-desktop" lipo -archs "$out/oino-desktop"
    app_bin="$out/oino-desktop$exe"
    if [[ $os == macos ]]; then
      bundle="$(ls -d "$out"/*.app 2>/dev/null | head -1)"
      [[ -n $bundle ]] && app_bin="$bundle/Contents/MacOS/oino-desktop"
      [[ -n $bundle ]] && check ".app bundle oino-desktop --version" "$app_bin" --version
    fi
    [[ $shot == 1 ]] && check "desktop --demo launch + screenshot" desktop_shot "$app_bin" \
      "$out/desktop-demo.png"
  fi
fi

android_shot() { # $1 = serial, $2 = png
  local adb="${ANDROID_HOME:-$HOME/Android/Sdk}/platform-tools/adb"
  command -v adb >/dev/null && adb="$(command -v adb)"
  sleep "${OINO_SHOT_WAIT:-20}"
  local pid; pid="$("$adb" -s "$1" shell pidof dev.oino.mobile | tr -d '\r')"
  "$adb" -s "$1" exec-out screencap -p > "$2"
  [[ -n $pid ]] || { echo "dev.oino.mobile not running"; return 1; }
  echo "running (pid $pid), screenshot $(basename "$2") ($(wc -c < "$2" | tr -d ' ') bytes)"
}

if [[ $smoke == 1 && $android == 1 ]]; then
  apk_dir="$art/aarch64-linux-android"
  check "apk zip integrity" unzip -tq "$apk_dir/oino-mobile-$version-release.apk"
  [[ -n $serial ]] && check "android install + launch on $serial" android_shot "$serial" \
    "$apk_dir/android-launch.png"
fi

# ── metadata ─────────────────────────────────────────────────────────────────

dirs=("$out")
[[ $target == universal-apple-darwin && $cli == 1 ]] && dirs+=("$art/x86_64-apple-darwin")
[[ $android == 1 ]] && dirs+=("$art/aarch64-linux-android")
for dir in "${dirs[@]}"; do
  {
    echo "target: $(basename "$dir")"
    echo "commit: $(git rev-parse HEAD 2>/dev/null || echo unknown)"
    echo "version: $version"
    echo "rustc: $(rustc -V)"
    echo "built: $(date -u +%Y-%m-%dT%H:%M:%SZ) on $(uname -sm)"
  } > "$dir/build-info.txt"
  (cd "$dir" && for f in *; do
    [[ -f $f && $f != SHA256SUMS ]] && sum "$f"
  done > SHA256SUMS) || true
done
log "artifacts in $out"
ls -la "$out"
if grep -q '^FAIL' "$smoke_log"; then
  log "smoke failures:"; grep '^FAIL' "$smoke_log"
fi
