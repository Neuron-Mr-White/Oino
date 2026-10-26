#!/usr/bin/env sh
# End-to-end smoke test of scripts/install.sh from this source checkout: builds and installs
# into a temporary prefix, then runs one print-mode prompt against the bundled extensions with
# the faux provider fixture (no network, no API key).
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TMP=$(mktemp -d "${TMPDIR:-$ROOT/target}/oino-install-smoke.XXXXXX")
PREFIX="$TMP/prefix"
AGENT="$TMP/agent"
trap 'rm -rf "$TMP"' EXIT INT TERM

cd "$ROOT"
OINO_FROM_SOURCE=1 OINO_PREFIX="$PREFIX" sh scripts/install.sh >"$TMP/install.log" 2>&1 || {
  cat "$TMP/install.log" >&2
  exit 1
}

test -x "$PREFIX/bin/oino"
test -x "$PREFIX/bin/oino-kanboard"
"$PREFIX/bin/oino" --version
"$PREFIX/bin/oino-kanboard" --help >/dev/null

mkdir -p "$AGENT" "$TMP/work"
printf '{"faux":{"type":"api_key","key":"smoke"}}' >"$AGENT/auth.json"
reply=$(cd "$TMP/work" && OINO_AGENT_DIR="$AGENT" "$PREFIX/bin/oino" \
  -e "$ROOT/extensions/dist/oino.fixture.provider-faux" --model faux/faux-1 -p "hello")
test "$reply" = "Hello from faux."
ls "$AGENT/bundled" | grep -q '^unipi.core@'
printf 'install smoke passed: %s\n' "$PREFIX/bin/oino"
