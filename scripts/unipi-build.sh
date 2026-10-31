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

if [ ! -f extensions/dist/checksums.json ]; then
  echo "==> extensions/dist missing: cargo xtask wasm"
  cargo xtask wasm
fi
sh scripts/release/stage-dist.sh apps/unipi/target/dist-stage

version=$(sed -n 's/.*"version": *"\([^"]*\)".*/\1/p' apps/unipi/src-tauri/tauri.conf.json | head -1)
stage="$root/apps/unipi/target/dist-stage/extensions"
# The dev identity merge (tauri.dev.conf.json) plus the staged extension
# resources — one --config, same shape as the release workflow's merge.
cat > apps/unipi/target/tauri-local.conf.json <<EOF
{
  "productName": "UniPi Dev",
  "identifier": "dev.unipi.app.dev",
  "bundle": { "resources": { "$stage": "extensions/" } }
}
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
        -output src-tauri/binaries/unipi-host-universal-apple-darwin
      codesign --force -s - --identifier dev.unipi.host \
        src-tauri/binaries/unipi-host-universal-apple-darwin)
    args=(--config "$conf" --target universal-apple-darwin --bundles dmg)
    out=apps/unipi/src-tauri/target/universal-apple-darwin/release/bundle
    ;;
  windows)
    echo "==> unipi-host sidecar (native MSVC)"
    (cd apps/unipi && cargo build --release -p unipi-host)
    mkdir -p apps/unipi/src-tauri/binaries
    cp apps/unipi/target/release/unipi-host.exe \
       apps/unipi/src-tauri/binaries/unipi-host-x86_64-pc-windows-msvc.exe
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

echo "==> bundles (UniPi Dev $version, identifier dev.unipi.app.dev):"
find "$out" -maxdepth 2 -type f \( -name '*.deb' -o -name '*.AppImage' -o -name '*.dmg' -o -name '*-setup.exe' \) \
  | sed 's/^/    /'
