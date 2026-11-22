#!/usr/bin/env sh
# Runs the unipi-integrations wasm-host harness tests (notify, web-api, mcp, updater).
# Reuses the root workspace's lock file and target dir so wasmtime is not rebuilt.
# Includes the §2.1.8 kill-and-restart step: the kill_and_restart_* tests SIGKILL the
# host (drop the wasm runtime) mid-turn and assert extension state restored or safely reset.
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cp "$root/Cargo.lock" "$here/Cargo.lock"
CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$root/target}" cargo test -j "${JOBS:-2}" --manifest-path "$here/Cargo.toml" "$@"
