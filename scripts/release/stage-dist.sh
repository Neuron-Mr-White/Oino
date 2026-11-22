#!/usr/bin/env bash
# Stage the release copy of extensions/dist (dist.md §3): every package except
# the fixtures (`oino.fixture.*` — dev/test helpers) and checksums.json (a
# build-time check, not a runtime file). Used by the release workflow and the
# local unipi:build task; the staged tree is what the oino archives unpack into
# `<prefix>/share/oino/extensions` and the desktop bundles ship as a resource.
#
#   scripts/release/stage-dist.sh <destination-dir>     # writes <dst>/extensions
set -euo pipefail

root="$(cd "$(dirname "$0")/../.." && pwd)"
src="$root/extensions/dist"
dst=${1:?usage: stage-dist.sh <destination-dir>}

[ -f "$src/checksums.json" ] || {
  echo "stage-dist: $src is missing — run \`cargo xtask wasm\` first" >&2
  exit 1
}
rm -rf "$dst/extensions"
mkdir -p "$dst/extensions"
for pkg in "$src"/*/; do
  name=$(basename "$pkg")
  case "$name" in
    oino.fixture.*) continue ;; # dev/test fixtures never ship (dist.md §3)
  esac
  cp -R "$pkg" "$dst/extensions/$name"
done
# The per-file sha map verifies a *build* of dist; releases carry SHA256SUMS
# of the archives instead, so it is not staged.
echo "staged $(ls -1 "$dst/extensions" | wc -l) extension packages into $dst/extensions"
