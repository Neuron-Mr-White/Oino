#!/usr/bin/env sh
# Runs the unipi.watchdog wasm-host harness tests (WP-A17 session caps): builds the
# component (release, wasm32-wasip2) into extensions/target, then drives it in the
# real wasm host. EXT_WORKSPACE may point at another extensions workspace root when
# sibling work in progress breaks `extensions/`.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cp "$root/Cargo.lock" "$here/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}" cargo test -j "${JOBS:-1}" --manifest-path "$here/Cargo.toml" "$@"
