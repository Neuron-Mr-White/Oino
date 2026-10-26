#!/usr/bin/env sh
# Oino installer (macOS / Linux / other Unix).
#
# Installs `oino` and `oino-kanboard` into $OINO_PREFIX/bin (default ~/.local/bin). Every
# provider and unipi extension is bundled inside the `oino` binary; nothing else is needed.
#
# 1. Prefer a tagged release binary (release-manifest.json, sha256-verified).
# 2. Fall back to a source build: `cargo xtask wasm` (extension components), then
#    `cargo build --release -p oino -p oino-kanboard`.
#
# Environment:
#   OINO_PREFIX       install prefix (default ~/.local)
#   OINO_REF          git tag / branch to install (default: latest release, source: v1-alpha)
#   OINO_FROM_SOURCE  1 = skip the release download
#   OINO_DIR          source checkout used by the fallback (default ~/.cache/oino/source)
#   OINO_DRY_RUN      1 = print the commands only
set -eu

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
run() {
  say "+ $*"
  if [ "${OINO_DRY_RUN:-0}" != "1" ]; then
    "$@"
  fi
}
download() {
  if have curl; then
    run curl -fsSL "$1" -o "$2"
  elif have wget; then
    run wget -qO "$2" "$1"
  else
    return 1
  fi
}

PREFIX=${OINO_PREFIX:-"$HOME/.local"}
BIN_DIR="$PREFIX/bin"
REPO=${OINO_REPO:-"https://github.com/Neuron-Mr-White/Oino.git"}
REPO_SLUG=${OINO_REPO_SLUG:-"Neuron-Mr-White/Oino"}
SRC_DIR=${OINO_DIR:-"$HOME/.cache/oino/source"}
BINARIES="oino oino-kanboard"

os=$(uname -s 2>/dev/null || printf unknown)
arch=$(uname -m 2>/dev/null || printf unknown)
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
esac
case "$os" in
  Linux) target="$arch-unknown-linux-gnu" ;;
  Darwin) target="$arch-apple-darwin" ;;
  *) target="$arch-$os" ;;
esac

manifest_url() {
  if [ -n "${OINO_REF:-}" ]; then
    printf 'https://github.com/%s/releases/download/%s/release-manifest.json\n' "$REPO_SLUG" "$OINO_REF"
  else
    printf 'https://github.com/%s/releases/latest/download/release-manifest.json\n' "$REPO_SLUG"
  fi
}

# Prints "<url> <sha256>" for binary $1 of $target from manifest $2.
artifact() {
  python3 - "$2" "$target" "$1" <<'PY'
import json, sys
manifest, target, name = json.load(open(sys.argv[1])), sys.argv[2], sys.argv[3]
for a in manifest.get('artifacts', []):
    if a.get('target') == target and a.get('name', 'oino') == name:
        print(a.get('url', ''), a.get('sha256', ''))
        raise SystemExit(0)
raise SystemExit(1)
PY
}

verify() {
  python3 - "$1" "$2" <<'PY'
import hashlib, sys
actual = hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest()
if sys.argv[2] and actual != sys.argv[2].strip().lower():
    raise SystemExit(f'sha256 mismatch for {sys.argv[1]}')
PY
}

try_binary_install() {
  [ "${OINO_DRY_RUN:-0}" != "1" ] || return 1
  [ "${OINO_FROM_SOURCE:-0}" != "1" ] || return 1
  have python3 || return 1
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/oino-install.XXXXXX")
  manifest="$tmp/release-manifest.json"
  say "Checking Oino release manifest: $(manifest_url)"
  download "$(manifest_url)" "$manifest" || return 1
  for name in $BINARIES; do
    info=$(artifact "$name" "$manifest") || return 1
    url=${info% *}
    sha=${info#* }
    download "$url" "$tmp/$name" || return 1
    verify "$tmp/$name" "$sha" || return 1
  done
  run mkdir -p "$BIN_DIR"
  for name in $BINARIES; do
    run chmod +x "$tmp/$name"
    run cp "$tmp/$name" "$BIN_DIR/$name"
  done
  rm -rf "$tmp"
  return 0
}

source_install() {
  if [ -f "Cargo.toml" ] && [ -d "crates/oino-bundle" ]; then
    SRC=$(pwd)
  else
    SRC=$SRC_DIR
    if [ ! -d "$SRC/.git" ]; then
      have git || err "git is required for the source build. Install git, or run from an Oino checkout."
      run mkdir -p "$(dirname "$SRC")"
      run git clone "$REPO" "$SRC"
    fi
    run git -C "$SRC" fetch --all --tags
    run git -C "$SRC" checkout "${OINO_REF:-v1-alpha}"
  fi
  if ! have cargo; then
    have curl || err "cargo is missing. Install Rust from https://rustup.rs/ and rerun."
    say "Installing Rust with rustup..."
    if [ "${OINO_DRY_RUN:-0}" = "1" ]; then
      say "+ curl https://sh.rustup.rs | sh -s -- -y --profile minimal"
    else
      curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
      . "$HOME/.cargo/env"
    fi
  fi
  # rust-toolchain.toml pins the toolchain and the wasm32-wasip2 target.
  (cd "$SRC" && run cargo xtask wasm)
  run cargo build --manifest-path "$SRC/Cargo.toml" --release -p oino -p oino-kanboard
  run mkdir -p "$BIN_DIR"
  for name in $BINARIES; do
    run cp "$SRC/target/release/$name" "$BIN_DIR/$name"
  done
}

if try_binary_install; then
  say "Installed release binaries into $BIN_DIR"
else
  say "No release binary for $target (or download failed); building from source."
  source_install
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "Add $BIN_DIR to PATH, e.g.: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac
say "Done. Run: oino"
