#!/usr/bin/env bash
# mise unipi:dev — run the Unipi desktop app from source with the DEV identity.
#
# A dev/test build must never share the installed app's identity or data
# (apps/unipi/BUILDING.md): the tauri.dev.conf.json merge renames the build to
#   identifier  dev.unipi.app.dev   (the installed app is dev.unipi.app)
#   productName UniPi Dev           (own app-data dir; the visible name stays Unipi)
# and OINO_HOST_DIR points the app-managed host at a scratch dir, so the real
# ~/.oino/host is never locked or written by a dev run.
#
# Steps: build the debug unipi-host sidecar for this machine + stage it under
# src-tauri/binaries (tauri's build.rs requires it), build extensions/dist if
# missing, then `cargo tauri dev`.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$root"

exe=""
triple=$(rustc -vV | sed -n 's/^host: //p')
[[ "$triple" == *windows* ]] && exe=".exe"

if [ ! -f extensions/dist/checksums.json ]; then
  echo "==> extensions/dist missing: cargo xtask wasm"
  cargo xtask wasm
fi

echo "==> debug unipi-host sidecar ($triple)"
(cd apps/unipi && cargo build -p unipi-host)
mkdir -p apps/unipi/src-tauri/binaries
cp "apps/unipi/target/debug/unipi-host$exe" \
   "apps/unipi/src-tauri/binaries/unipi-oino-host-$triple$exe"

export OINO_HOST_DIR="${OINO_HOST_DIR:-$root/apps/unipi/target/unipi-dev/host}"
mkdir -p "$OINO_HOST_DIR"
echo "==> dev host data dir: $OINO_HOST_DIR (the real ~/.oino/host is untouched)"
echo "==> cargo tauri dev (identifier dev.unipi.app.dev, productName 'UniPi Dev', shown as Unipi)"
cd apps/unipi
exec cargo tauri dev --config tauri.dev.conf.json
