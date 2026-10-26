#!/usr/bin/env bash
# Build (and optionally run) Oino Mobile for iOS on the macOS host over SSH (gpui-app §6).
#
#   apps/oino-mobile/scripts/ios-remote.sh [--host mac] [--dir oino-build]
#                                          [build.sh ios options…]
#
# Syncs the working tree (sources only: no target/, dist/, Gradle or Xcode build output) to
# <host>:<dir> with rsync, then runs `apps/oino-mobile/scripts/build.sh ios …` there and
# copies the resulting .app (and simulator screenshots) back into apps/oino-mobile/dist/.
# The host is the `mac` SSH alias (user desmondgiam; override with --host or $OINO_IOS_HOST).
# For device builds the iPhone must be plugged into the Mac; OINO_IOS_TEAM is forwarded when
# set locally (or set it in the remote ~/.zshenv). Examples:
#
#   apps/oino-mobile/scripts/ios-remote.sh --simulator --no-wasm --demo
#   OINO_IOS_TEAM=ABCDE12345 apps/oino-mobile/scripts/ios-remote.sh --device --no-wasm
#
# The remote needs Xcode 16+, xcodegen, rustup with aarch64-apple-ios(-sim), and — unless
# --no-wasm is passed and extensions/dist is synced — the wasm toolchain for `cargo xtask wasm`.
set -euo pipefail

here="$(cd "$(dirname "$0")" && pwd)"
root="$(cd "$here/../../.." && pwd)"
host="${OINO_IOS_HOST:-mac}"
dir=oino-build
args=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --host) host="$2"; shift ;;
    --dir) dir="$2"; shift ;;
    -h|--help) sed -n '2,20p' "$0"; exit 0 ;;
    *) args+=("$1") ;;
  esac
  shift
done

log() { printf '\033[1;35m==>\033[0m %s\n' "$*"; }

log "syncing $root → $host:$dir"
rsync -az --delete \
  --exclude '/target/' --exclude '/apps/target/' --exclude '/extensions/target/' \
  --exclude '/apps/oino-mobile/dist/' --exclude '/apps/oino-desktop/dist/' \
  --exclude '/apps/oino-mobile/android/' --exclude '/apps/oino-mobile/ios/build/' \
  --exclude '/apps/oino-mobile/ios/OinoMobile.xcodeproj/' \
  --exclude '.git/' \
  "$root/" "$host:$dir/"

log "building on $host: build.sh ios ${args[*]:-}"
# Login shell so Homebrew / rustup PATH entries apply. No tty: runs from scripts and agents.
status=0
ssh "$host" "cd '$dir' && OINO_IOS_TEAM='${OINO_IOS_TEAM:-}' JOBS='${JOBS:-}' OINO_SHOT_DELAY='${OINO_SHOT_DELAY:-}' OINO_DEMO_MARKER='${OINO_DEMO_MARKER:-}' exec \$SHELL -lc 'apps/oino-mobile/scripts/build.sh ios ${args[*]:-}'" || status=$?

log "fetching the .app"
mkdir -p "$root/apps/oino-mobile/dist"
rsync -az "$host:$dir/apps/oino-mobile/dist/" "$root/apps/oino-mobile/dist/" || true
exit "$status"
