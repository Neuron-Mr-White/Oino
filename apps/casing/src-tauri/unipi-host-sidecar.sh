#!/usr/bin/env bash
# UNI-236 (plan A4, docs/m8/SELF-INSTALL-AND-EMBEDDED-HOST.md): build the host
# as a Tauri **sidecar**, ready for `bundle.externalBin`. The app resolves it
# through `host_supervisor::sidecar` and prefers it over any `unipi-host` on
# PATH, so this has to run before `cargo tauri build`.
#
#   apps/mobile/src-tauri/unipi-host-sidecar.sh [app-target-triple…]
#
# Tauri looks for `apps/mobile/src-tauri/binaries/unipi-host-<app target
# triple>` (the triple the *desktop app* is built for), so each static musl
# host is copied under the gnu triple of the same architecture. There is no
# musl-gcc on these PCs: cargo-zigbuild is the only linker that works.
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$(cd "$HERE/../../.." && pwd)"
cd "$ROOT"

# app triple <- the host target we build it from
LINUX=(
  "x86_64-unknown-linux-gnu:x86_64-unknown-linux-musl"
  "aarch64-unknown-linux-gnu:aarch64-unknown-linux-musl"
)
# The desktop installer targets the MSVC triple; the host is cross-built with
# the gnu one (cargo-zigbuild) and copied under the msvc name.
WINDOWS=(
  "x86_64-pc-windows-msvc:x86_64-pc-windows-gnu"
)

build() {
  local app=$1 target=$2
  echo "==> unipi-host sidecar: $target -> src-tauri/binaries/unipi-host-$app"
  cargo zigbuild -p unipi-host --release --target "$target"
  local src="target/$target/release/unipi-host" ext=""
  [ "$target" = "x86_64-pc-windows-gnu" ] && ext=".exe"
  [ -s "$src$ext" ] || { echo "   no $src$ext" >&2; return 1; }
  mkdir -p "$HERE/binaries"
  # Tauri looks for binaries/unipi-host-<triple>.exe on Windows targets.
  cp -f "$src$ext" "$HERE/binaries/unipi-host-$app$ext"
  file "$HERE/binaries/unipi-host-$app$ext" | sed 's/^/    /'
}

if [ $# -gt 0 ]; then
  # An explicit app triple: pick the pair that provides it.
  found=0
  for pair in "${LINUX[@]}" "${WINDOWS[@]}"; do
    [ "${pair%%:*}" = "$1" ] && { build "${pair%%:*}" "${pair##*:}"; found=1; }
  done
  [ "$found" = 1 ] || { echo "no sidecar recipe for $1 (have: ${LINUX[*]} ${WINDOWS[*]})" >&2; exit 1; }
else
  for pair in "${LINUX[@]}"; do build "${pair%%:*}" "${pair##*:}"; done
fi
echo "==> sidecars in apps/mobile/src-tauri/binaries:"
ls -1 "$HERE/binaries" | sed 's/^/    /'
