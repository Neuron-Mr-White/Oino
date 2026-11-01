#!/usr/bin/env bash
# mise unipi:build — a local RELEASE bundle of the UniPi desktop app for THIS
# OS, using the same steps as the release workflow (.github/workflows/
# release.yml): sidecar, staged extension dist as a bundle resource, version
# from tauri.conf.json.
#
# Like unipi:dev this keeps the DEV identity (dev.unipi.app.dev / 'UniPi Dev',
# apps/unipi/tauri.dev.conf.json): a locally built bundle can never clobber the
# installed dev.unipi.app app. CI release builds are the only dev.unipi.app
# builds.
#
# Bundles: Linux .deb + .AppImage (webkitgtk dev packages required),
# macOS universal .dmg (Xcode CLT + both darwin targets), Windows NSIS .exe.
# Output: apps/unipi/src-tauri/target/[<target>/]release/bundle/…
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

os=linux
case "$(uname -s)" in
  Linux*) os=linux ;;
  Darwin*) os=macos ;;
  MINGW*|MSYS*|CYGWIN*) os=windows ;;
  *) echo "unsupported host: $(uname -s)" >&2; exit 1 ;;
esac

# Rebuild the extension dist when it is missing or incomplete (a killed
# `cargo xtask wasm` leaves checksums.json with only some packages built).
dist_ok() {
  [ -f extensions/dist/checksums.json ] || return 1
  local n
  n=$(find extensions/dist -mindepth 2 -maxdepth 2 -name extension.json | wc -l)
  [ "$n" -ge 30 ]
}
if ! dist_ok; then
  echo "==> extensions/dist missing or incomplete: cargo xtask wasm"
  cargo xtask wasm
  dist_ok || { echo "extensions/dist still incomplete after cargo xtask wasm" >&2; exit 1; }
fi
sh scripts/release/stage-dist.sh apps/unipi/target/dist-stage

version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' apps/unipi/src-tauri/tauri.conf.json | head -1)
stage="$root/apps/unipi/target/dist-stage/extensions"
# The dev identity merge (tauri.dev.conf.json) plus the bundled extensions —
# one --config, same shape as the release workflow's merge. UNIPI_RELEASE_IDENTITY=1
# keeps the release identity (UniPi Oino / dev.unipi.oino): byte-for-byte the
# package a user installs. Linux puts the extensions in /usr/share/oino/extensions
# and the CLI in /usr/bin/oino (deb + AppImage `files`); macOS/Windows use resources.
if [ "${UNIPI_RELEASE_IDENTITY:-0}" = 1 ]; then identity=''; label="UniPi Oino $version, identifier dev.unipi.oino"
else identity='"productName": "UniPi Dev", "identifier": "dev.unipi.app.dev",'; label="UniPi Dev $version, identifier dev.unipi.app.dev"; fi
if [ "$os" = linux ]; then
  echo "==> oino CLI (release)"
  cargo build --release -p oino-cli --bin oino
  oino="$root/target/release/oino"
  files="{ \"/usr/share/oino/extensions\": \"$stage\", \"/usr/bin/oino\": \"$oino\" }"
  bundle="{ \"linux\": { \"deb\": { \"files\": $files }, \"appimage\": { \"files\": $files } } }"
else
  bundle="{ \"resources\": { \"$stage\": \"extensions/\" } }"
fi
cat > apps/unipi/target/tauri-local.conf.json <<EOF
{ $identity "bundle": $bundle }
EOF
conf=target/tauri-local.conf.json

case "$os" in
  linux)
    arch=$(uname -m); case "$arch" in x86_64|amd64) arch=x86_64;; aarch64|arm64) arch=aarch64;; esac
    echo "==> unipi-host sidecar (static musl via zigbuild)"
    apps/unipi/src-tauri/unipi-host-sidecar.sh "${arch}-unknown-linux-gnu"
    args=(--config "$conf" --bundles deb,appimage)
    out=apps/unipi/src-tauri/target/release/bundle
    ;;
  macos)
    echo "==> universal sidecar (lipo + ad-hoc codesign)"
    (cd apps/unipi &&
      rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null 2>&1 || true
      for t in aarch64-apple-darwin x86_64-apple-darwin; do
        cargo build --release -p unipi-host --target "$t"
      done
      mkdir -p src-tauri/binaries
      lipo -create \
        target/aarch64-apple-darwin/release/unipi-host \
        target/x86_64-apple-darwin/release/unipi-host \
        -output src-tauri/binaries/unipi-oino-host-universal-apple-darwin
      codesign --force -s - --identifier dev.unipi.host \
        src-tauri/binaries/unipi-oino-host-universal-apple-darwin)
    args=(--config "$conf" --target universal-apple-darwin --bundles dmg)
    out=apps/unipi/src-tauri/target/universal-apple-darwin/release/bundle
    ;;
  windows)
    echo "==> unipi-host sidecar (native MSVC)"
    (cd apps/unipi && cargo build --release -p unipi-host)
    mkdir -p apps/unipi/src-tauri/binaries
    cp apps/unipi/target/release/unipi-host.exe \
       apps/unipi/src-tauri/binaries/unipi-oino-host-x86_64-pc-windows-msvc.exe
    args=(--config "$conf" --bundles nsis)
    out=apps/unipi/src-tauri/target/release/bundle
    ;;
esac

if [ ! -d apps/unipi/node_modules ]; then
  echo "==> npm ci"
  (cd apps/unipi && npm ci)
fi

echo "==> cargo tauri build ${args[*]}"
(cd apps/unipi && cargo tauri build "${args[@]}")

echo "==> bundles ($label):"
find "$out" -maxdepth 2 -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.dmg' -o -name '*-setup.exe' \) \
  | sed 's/^/    /'

# Gate: a bundle without its extensions is not a release (it opens with no providers).
while IFS= read -r -d '' deb; do
  n=$(dpkg-deb -c "$deb" | grep -c 'extensions/[^/]*/extension.json$' || true)
  [ "$n" -ge 30 ] || { echo "ERROR: $deb bundles only $n extensions" >&2; exit 1; }
  echo "    $deb: $n extensions bundled"
done < <(find "$out" -maxdepth 2 -name '*.deb' -print0)
