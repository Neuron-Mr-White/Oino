#!/usr/bin/env sh
# Runs the unipi.kanboard wasm-host harness: builds oino-kanboard (root workspace) and the
# component, then drives the component in the real wasm host against the real binary.
# KANBOARD_EXT_WORKSPACE may point at another extensions workspace root (one listing only
# buildable members) when sibling work in progress breaks `extensions/`.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cargo build -j "${JOBS:-2}" --manifest-path "$root/Cargo.toml" -p oino-kanboard
cp "$root/Cargo.lock" "$here/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}" cargo test -j "${JOBS:-2}" --manifest-path "$here/Cargo.toml" "$@" -- --test-threads=1
