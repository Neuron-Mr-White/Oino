#!/usr/bin/env sh
# Runs the unipi.kanboard wasm-host harness: builds unipi-kanboard (apps/unipi workspace)
# and the component, then drives the component in the real wasm host against the real binary.
# KANBOARD_EXT_WORKSPACE may point at another extensions workspace root (one listing only
# buildable members) when sibling work in progress breaks `extensions/`.
# Includes the §2.1.8 kill-and-restart step: the kill_and_restart_* tests SIGKILL the
# host (drop the wasm runtime) mid-turn and assert extension state restored or safely reset.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cargo build -j "${JOBS:-2}" --manifest-path "$root/apps/unipi/Cargo.toml" -p unipi-kanboard
cp "$root/Cargo.lock" "$here/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}" cargo test -j "${JOBS:-2}" --manifest-path "$here/Cargo.toml" "$@" -- --test-threads=1
