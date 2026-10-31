#!/usr/bin/env sh
# scripts/install-test.sh — run scripts/install.sh against a fake local release
# (OINO_INSTALL_BASE_URL, no network): the CLI archive is laid out exactly like
# the release workflow builds it, checksums are verified, and a tampered
# archive must fail. Runs in a throwaway HOME; installs nothing real.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
TMP=$(mktemp -d "${TMPDIR:-/tmp}/oino-install-test.XXXXXX")
trap 'rm -rf "$TMP"' EXIT INT TERM
REL="$TMP/rel"                 # the fake release dir (served via file://)
HOME_DIR="$TMP/home"
VERSION=9.9.9
TARGET=$(uname -m)
case "$TARGET" in
  x86_64|amd64) TARGET=x86_64-unknown-linux-musl ;;
  arm64|aarch64) TARGET=aarch64-unknown-linux-musl ;;
  *) echo "unsupported test host arch: $TARGET" >&2; exit 1 ;;
esac

fail() { printf 'FAIL %s\n' "$*" >&2; exit 1; }
ok() { printf 'ok   %s\n' "$*"; }

# ── fake release: CLI archive (bin/oino + share/oino/extensions) ────────────
mkdir -p "$REL" "$TMP/stage/bin" "$TMP/stage/share/oino/extensions/oino.example"
cat > "$TMP/stage/bin/oino" <<EOF
#!/bin/sh
echo "oino $VERSION (fake)"
EOF
chmod 755 "$TMP/stage/bin/oino"
printf '{"id":"oino.example","name":"Example","version":"1.0.0"}\n' \
  > "$TMP/stage/share/oino/extensions/oino.example/extension.json"
tar -czf "$REL/oino-$VERSION-$TARGET.tar.gz" -C "$TMP/stage" bin share

# ── fake AppImage (a stub: the icon extraction must degrade gracefully) ─────
printf '#!/bin/sh\necho fake appimage\n' > "$REL/UniPi_${VERSION}_amd64.AppImage"
chmod 755 "$REL/UniPi_${VERSION}_amd64.AppImage"

(cd "$REL" && sha256sum oino-*.tar.gz UniPi_*.AppImage > SHA256SUMS)

run_install() {
  HOME="$HOME_DIR" OINO_PREFIX="$HOME_DIR/.local" \
    OINO_INSTALL_BASE_URL="file://$REL" sh "$ROOT/scripts/install.sh" "$@"
}

# ── 1. the happy path installs the CLI + the share tree ─────────────────────
mkdir -p "$HOME_DIR"
out=$(run_install 2>&1) || { printf '%s\n' "$out" >&2; fail "install.sh exited non-zero"; }
[ -x "$HOME_DIR/.local/bin/oino" ] || fail "no ~/.local/bin/oino"
[ "$("$HOME_DIR/.local/bin/oino")" = "oino $VERSION (fake)" ] || fail "wrong oino stub output"
[ -f "$HOME_DIR/.local/share/oino/extensions/oino.example/extension.json" ] \
  || fail "no share/oino/extensions/oino.example"
case "$out" in *"Installed oino into"*) ;; *) fail "unexpected install output" ;; esac
ok "CLI + share tree installed, checksum verified"

# ── 2. --app installs the AppImage + a menu entry ──────────────────────────
out=$(run_install --app 2>&1) || { printf '%s\n' "$out" >&2; fail "install.sh --app exited non-zero"; }
[ -x "$HOME_DIR/.local/bin/UniPi.AppImage" ] || fail "no ~/.local/bin/UniPi.AppImage"
[ -f "$HOME_DIR/.local/share/applications/unipi.desktop" ] || fail "no unipi.desktop"
case "$out" in *"Installed UniPi"*) ;; *) fail "unexpected --app output" ;; esac
ok "--app AppImage install + desktop entry"

# ── 3. a tampered archive must fail the checksum ────────────────────────────
printf 'tampered' > "$REL/oino-$VERSION-$TARGET.tar.gz"  # sums stay stale
rm -rf "$HOME_DIR/.local"
if out=$(run_install 2>&1); then
  printf '%s\n' "$out" >&2
  fail "tampered archive was accepted"
fi
case "$out" in *"sha256 mismatch"*) ;; *) fail "wrong failure message: $out" ;; esac
ok "tampered archive rejected"

printf 'install-test passed (fake release at %s)\n' "$REL"
