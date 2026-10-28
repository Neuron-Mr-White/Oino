#!/usr/bin/env bash
# Build and package Oino Desktop (gpui-app §6). `cargo xtask desktop …` delegates here.
#
#   apps/oino-desktop/scripts/build.sh [--release] [--universal] [--no-wasm] [--no-package]
#                                      [--formats <list>]
#
# Steps:
#   1. `cargo xtask wasm`  — builds every provider / unipi component into extensions/dist
#      (bundle input for oino-bundle; skip with --no-wasm when dist is current).
#   2. cargo build of oino-desktop (and the `oino` CLI, installed alongside).
#   3. Packaging:
#        Linux    tar.gz (always) + AppImage/deb via cargo-packager when installed
#        macOS    .app + .dmg via cargo-packager (--universal: lipo aarch64 + x86_64)
#        Windows  NSIS installer via cargo-packager (run from Git Bash / MSYS on the host)
#      Artifacts land in apps/oino-desktop/dist/.
#
# Requirements: Rust 1.99 (rust-toolchain.toml), wasm32-wasip2 + wasm-tools for step 1,
# `cargo install cargo-packager --locked` for installers. Linux system packages:
#   clang libfontconfig-dev libwayland-dev libxkbcommon-x11-dev libx11-xcb-dev libssl-dev
#   libzstd-dev libasound2-dev libvulkan1
# Memory: set JOBS (default 2) on small machines; GPUI + wasmtime are heavy to compile.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
apps="$(cd "$here/../.." && pwd)"
root="$(cd "$apps/.." && pwd)"
dist="$apps/oino-desktop/dist"

profile=debug
cargo_profile=()
universal=0
wasm=1
package=1
formats=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --release) profile=release; cargo_profile=(--release) ;;
    --universal) universal=1 ;;
    --no-wasm) wasm=0 ;;
    --no-package) package=0 ;;
    --formats) formats="$2"; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
  shift
done

jobs="${JOBS:-2}"
case "$(uname -s)" in
  Linux*) os=linux ;;
  Darwin*) os=macos ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) echo "unsupported host: $(uname -s)" >&2; exit 1 ;;
esac
exe=""; [[ $os == windows ]] && exe=".exe"
version="$(sed -n 's/^version = "\(.*\)"/\1/p' "$apps/Cargo.toml" | head -1)"

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }

if [[ $wasm == 1 ]]; then
  log "building extension components (cargo xtask wasm)"
  (cd "$root" && cargo xtask wasm)
fi

build_target() { # $1 = optional target triple
  local target_args=()
  [[ -n "${1:-}" ]] && target_args=(--target "$1")
  log "building oino-desktop ${1:-host} ($profile)"
  # bash 3.2 (macOS /bin/bash) + `set -u` aborts on an empty array expansion, so both
  # arrays use the ${arr[@]+"${arr[@]}"} idiom (same as apps/oino-mobile/scripts/build.sh).
  (cd "$apps" && cargo build -j "$jobs" -p oino-desktop \
    ${cargo_profile[@]+"${cargo_profile[@]}"} ${target_args[@]+"${target_args[@]}"})
  log "building oino CLI ${1:-host} ($profile)"
  (cd "$root" && cargo build -j "$jobs" -p oino \
    ${cargo_profile[@]+"${cargo_profile[@]}"} ${target_args[@]+"${target_args[@]}"})
  # Companion binaries the WASM host finds next to the running executable (GA-41):
  # `oino-kanboard` is the kanboard daemon the Board page starts; cron is an `oino`
  # subcommand (`oino cron serve`), which ships with the CLI above.
  log "building oino-kanboard ${1:-host} ($profile)"
  (cd "$root" && cargo build -j "$jobs" -p oino-kanboard \
    ${cargo_profile[@]+"${cargo_profile[@]}"} ${target_args[@]+"${target_args[@]}"})
}

if [[ $os == macos && $universal == 1 ]]; then
  for t in aarch64-apple-darwin x86_64-apple-darwin; do build_target "$t"; done
  out="$apps/target/$profile"
  mkdir -p "$out"
  lipo -create -output "$out/oino-desktop" \
    "$apps/target/aarch64-apple-darwin/$profile/oino-desktop" \
    "$apps/target/x86_64-apple-darwin/$profile/oino-desktop"
  mkdir -p "$root/target/$profile"
  lipo -create -output "$root/target/$profile/oino" \
    "$root/target/aarch64-apple-darwin/$profile/oino" \
    "$root/target/x86_64-apple-darwin/$profile/oino"
  lipo -create -output "$root/target/$profile/oino-kanboard" \
    "$root/target/aarch64-apple-darwin/$profile/oino-kanboard" \
    "$root/target/x86_64-apple-darwin/$profile/oino-kanboard"
else
  build_target ""
fi

app_bin="$apps/target/$profile/oino-desktop$exe"
cli_bin="$root/target/$profile/oino$exe"
kanboard_bin="$root/target/$profile/oino-kanboard$exe"
# GA-41: the WASM host resolves a bare program name against the directory of the running
# executable, so the kanboard daemon ships next to oino-desktop (and in the tarball's bin/).
[[ -f "$cli_bin" ]] && cp "$cli_bin" "$apps/target/$profile/"
[[ -f "$kanboard_bin" ]] && cp "$kanboard_bin" "$apps/target/$profile/"
[[ $package == 0 ]] && { log "built $app_bin"; exit 0; }

mkdir -p "$dist"
arch="$(uname -m)"
if [[ $os == linux ]]; then
  stage="$(mktemp -d "$apps/target/stage.XXXX")"
  name="oino-desktop-$version-linux-$arch"
  mkdir -p "$stage/$name/share/applications" "$stage/$name/share/icons/hicolor/256x256/apps" "$stage/$name/bin"
  cp "$app_bin" "$stage/$name/bin/"
  [[ -f "$cli_bin" ]] && cp "$cli_bin" "$stage/$name/bin/"
  [[ -f "$kanboard_bin" ]] && cp "$kanboard_bin" "$stage/$name/bin/"
  cp "$apps/oino-desktop/assets/oino-desktop.desktop" "$stage/$name/share/applications/"
  cp "$apps/oino-desktop/assets/oino-256.png" "$stage/$name/share/icons/hicolor/256x256/apps/oino-desktop.png"
  tar -C "$stage" -czf "$dist/$name.tar.gz" "$name"
  rm -rf "$stage"
  log "wrote $dist/$name.tar.gz"
  formats="${formats:-appimage,deb}"
elif [[ $os == macos ]]; then
  formats="${formats:-app,dmg}"
else
  formats="${formats:-nsis}"
fi

if command -v cargo-packager >/dev/null 2>&1; then
  log "packaging ($formats) with cargo-packager"
  (cd "$apps/oino-desktop" && cargo packager \
    ${cargo_profile[@]+"${cargo_profile[@]}"} --formats "$formats" --out-dir "$dist")
else
  log "cargo-packager not installed: skipping $formats (cargo install cargo-packager --locked)"
fi

# GA-41: the companions go beside the app binary inside the bundle, where the WASM host
# resolves them (`Contents/MacOS` on macOS). The AppImage/deb layout is cargo-packager's.
if [[ $os == macos && -d "$dist/Oino.app/Contents/MacOS" ]]; then
  for b in oino oino-kanboard; do
    [[ -f "$root/target/$profile/$b" ]] && cp "$root/target/$profile/$b" "$dist/Oino.app/Contents/MacOS/"
  done
fi

sum() { if command -v sha256sum >/dev/null; then sha256sum "$@"; else shasum -a 256 "$@"; fi; }
(cd "$dist" && for f in *; do [[ -f "$f" && "$f" != SHA256SUMS ]] && sum "$f"; done > SHA256SUMS) || true
log "artifacts in $dist"
