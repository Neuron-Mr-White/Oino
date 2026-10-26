#!/usr/bin/env sh
# Runs the unipi.core wasm-host harness tests. Reuses the root workspace's lock file and target
# dir so wasmtime is not rebuilt; `-j 2` because rustc is memory-hungry on small machines.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cp "$root/Cargo.lock" "$here/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}" cargo test -j "${JOBS:-2}" --manifest-path "$here/Cargo.toml" "$@"
