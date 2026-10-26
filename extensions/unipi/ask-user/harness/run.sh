#!/usr/bin/env sh
# Runs the unipi-ux wasm-host harness tests. Reuses the root lock file and target dir so wasmtime
# is not rebuilt. UNIPI_UX_WORKSPACE may point at another extensions workspace root (one that
# lists only buildable members) when sibling work in progress breaks `extensions/`.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cp "$root/Cargo.lock" "$here/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}" cargo test -j "${JOBS:-2}" --manifest-path "$here/Cargo.toml" "$@"
