#!/usr/bin/env sh
# Runs the unipi.utility wasm-host harness tests. Reuses the root workspace's lock file and
# target dir so wasmtime is not rebuilt.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cp "$root/Cargo.lock" "$here/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}" cargo test -j "${JOBS:-2}" --manifest-path "$here/Cargo.toml" "$@"
