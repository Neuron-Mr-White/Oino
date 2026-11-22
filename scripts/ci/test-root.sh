#!/usr/bin/env bash
# Workspace tests unify oino-zvec's feature into CLI tests. The zvec sys crate
# emits a link search path, not an executable rpath: include its acquired SDK
# for test executables AND the CLI subprocesses they spawn.
set -euo pipefail
root=$(cd "$(dirname "$0")/../.." && pwd)
cd "$root"
target=${CARGO_TARGET_DIR:-"$root/target"}
# CLI sandboxes deliberately env_clear(), so LD_LIBRARY_PATH alone cannot work.
# Test-only origin rpaths survive a cleared environment; no release flags change.
export RUSTFLAGS="${RUSTFLAGS:-} -Clink-arg=-Wl,-rpath,\$ORIGIN -Clink-arg=-Wl,-rpath,\$ORIGIN/deps"
cargo test --locked --workspace --no-run "$@"
paths="${ZVEC_LIB_DIR:-}:${OINO_ORT_DIR:-}"
while IFS= read -r lib; do
  paths="$paths:$(dirname "$lib")"
  cp "$lib" "$target/debug/deps/libzvec_c_api.so"
done < <(find "$target/debug/build" -type f -name 'libzvec_c_api.so' -print)
export LD_LIBRARY_PATH="$paths:$target/debug:$target/debug/deps${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
cargo test --locked --workspace "$@"
