#!/usr/bin/env sh
set -eu

# End-to-end smoke test for the Unix installer using this source checkout.
# It builds Oino, installs into temporary OINO_PREFIX/OINO_HOME, verifies the
# binary starts, and verifies built-in extension packages were enabled.

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
TMP=${TMPDIR:-/tmp}/oino-install-smoke-$$
PREFIX="$TMP/prefix"
HOME_DIR="$TMP/home"
LOG="$TMP/install.log"

cleanup() {
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

mkdir -p "$TMP"
cd "$ROOT"

OINO_PREFIX="$PREFIX" OINO_HOME="$HOME_DIR" sh scripts/install.sh >"$LOG" 2>&1

test -x "$PREFIX/bin/oino"
"$PREFIX/bin/oino" --help >/dev/null

test -f "$HOME_DIR/.oino/settings.json"
for package_id in \
  oino.9router \
  oino.footer_status \
  oino.ralph_loop \
  oino.mode_sandbox \
  oino.notify \
  oino.craft_skill \
  oino.vcc \
  oino.ask_user
  do
    test -f "$HOME_DIR/.oino/extension-packages/$package_id/oino.package.json"
    grep -q "\"$package_id\"" "$HOME_DIR/.oino/settings.json"
  done

printf 'install smoke passed: %s\n' "$PREFIX/bin/oino"
