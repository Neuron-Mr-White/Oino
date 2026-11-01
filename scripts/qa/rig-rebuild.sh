#!/usr/bin/env bash
# Rebuild the app from the working tree into the rig's installed tree, then
# restart the app in the rig. Frontend + Tauri shell always; the host sidecar
# too with --host. ~2-4 min incremental.
#   scripts/qa/rig-rebuild.sh [--host]
set -euo pipefail
root="$(cd "$(dirname "$0")/../.." && pwd)"
QA_HOME=${QA_HOME:-$HOME/.cache/oino-qa}
export CARGO_BUILD_JOBS=${CARGO_BUILD_JOBS:-8}
[ -d "$QA_HOME/app/usr/bin" ] || { echo "run scripts/qa/rig.sh up first" >&2; exit 1; }
if [ "${1:-}" = --host ]; then
  (cd "$root/apps/unipi" && cargo build --release -p unipi-host 2>&1 | tail -3)
  install -m 755 "$root/apps/unipi/target/release/unipi-host" "$QA_HOME/app/usr/bin/unipi-oino-host"
fi
# vite build + the release shell with the custom protocol (what `cargo tauri build`
# does, without its file watcher, which hits the inotify limit on busy machines).
(cd "$root/apps/unipi" && npx vite build 2>&1 | grep -E "error|built in" | tail -3)
(cd "$root/apps/unipi/src-tauri" && cargo build --release --features tauri/custom-protocol 2>&1 | grep -E "^error|Finished" | tail -5)
install -m 755 "$root/apps/unipi/src-tauri/target/release/unipi-mobile" "$QA_HOME/app/usr/bin/unipi-oino"  # cargo names it after the crate; the bundler renames it to mainBinaryName
node "$root/scripts/qa/rig.mjs" start
echo "rig restarted with the working tree build"
