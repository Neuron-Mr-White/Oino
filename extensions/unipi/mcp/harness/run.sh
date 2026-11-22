#!/usr/bin/env sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../../../.." && pwd)
cd "$root"
export PATH="$HOME/.cargo/bin:$HOME/.local/bin:$HOME/.local/share/mise/shims:$PATH"
export CARGO_BUILD_JOBS=4
export CARGO_TARGET_DIR="$root/target"
nice -n 10 cargo test --release --manifest-path extensions/Cargo.toml -p unipi-mcp -- --nocapture
nice -n 10 cargo build --release --target wasm32-wasip2 --manifest-path extensions/Cargo.toml -p unipi-mcp
nice -n 10 cargo test --release --manifest-path "$here/Cargo.toml" "$@"
