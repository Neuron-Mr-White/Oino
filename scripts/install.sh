#!/usr/bin/env sh
set -eu

# Oino source installer. It is intentionally dependency-light:
# - uses an existing source checkout when run from one
# - otherwise downloads/clones Oino when git + curl/wget are available
# - installs Rust with rustup when cargo is missing and a downloader is available
#
# Environment:
#   OINO_REPO      git URL to clone when not in a checkout
#   OINO_REF       git ref to checkout after clone
#   OINO_PREFIX    install prefix; default: ~/.local
#   OINO_DIR       source checkout dir; default: ~/.cache/oino/source
#   OINO_DRY_RUN   set to 1 to print actions without running build/install

say() { printf '%s\n' "$*"; }
err() { printf 'error: %s\n' "$*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }
run() {
  say "+ $*"
  if [ "${OINO_DRY_RUN:-0}" != "1" ]; then
    "$@"
  fi
}

OS=$(uname -s 2>/dev/null || printf unknown)
case "$OS" in
  Linux|Darwin|FreeBSD|OpenBSD|NetBSD|MINGW*|MSYS*|CYGWIN*) ;;
  *) say "warning: untested OS '$OS'; continuing with POSIX shell path" ;;
esac

PREFIX=${OINO_PREFIX:-"$HOME/.local"}
BIN_DIR="$PREFIX/bin"
REPO=${OINO_REPO:-"https://github.com/Neuron-Mr-White/Oino.git"}
SRC_DIR=${OINO_DIR:-"$HOME/.cache/oino/source"}

if [ -f "Cargo.toml" ] && grep -q 'oino-app' Cargo.toml 2>/dev/null; then
  SRC=$(pwd)
else
  SRC=$SRC_DIR
  if [ ! -d "$SRC/.git" ]; then
    have git || err "git is required to clone Oino. Install git, or run this script from an Oino source checkout."
    run mkdir -p "$(dirname "$SRC")"
    run git clone "$REPO" "$SRC"
  fi
  if [ -n "${OINO_REF:-}" ]; then
    run git -C "$SRC" fetch --all --tags
    run git -C "$SRC" checkout "$OINO_REF"
  fi
fi

if ! have cargo; then
  if have rustup; then
    say "rustup exists; using it"
  else
    if have curl; then
      say "Installing Rust with rustup..."
      if [ "${OINO_DRY_RUN:-0}" = "1" ]; then
        say "+ curl https://sh.rustup.rs | sh -s -- -y --profile minimal"
      else
        curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
      fi
    elif have wget; then
      say "Installing Rust with rustup..."
      if [ "${OINO_DRY_RUN:-0}" = "1" ]; then
        say "+ wget https://sh.rustup.rs -O- | sh -s -- -y --profile minimal"
      else
        wget -qO- https://sh.rustup.rs | sh -s -- -y --profile minimal
      fi
    else
      err "cargo is missing and neither curl nor wget is available to install rustup. Install Rust from https://rustup.rs/ and rerun."
    fi
  fi
  # shellcheck disable=SC1091
  [ "${OINO_DRY_RUN:-0}" = "1" ] || . "$HOME/.cargo/env"
fi

run cargo build --manifest-path "$SRC/Cargo.toml" -p oino-app --bin oino --release
run mkdir -p "$BIN_DIR"
run cp "$SRC/target/release/oino" "$BIN_DIR/oino"

if [ -x "$SRC/scripts/install-all-builtins.sh" ]; then
  if have bash; then
    run bash "$SRC/scripts/install-all-builtins.sh"
  else
    say "bash not found; skipping optional built-in extension install"
    say "Install bash and run: bash $SRC/scripts/install-all-builtins.sh"
  fi
fi

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "Add this to your shell profile if needed: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

say "Oino installed: $BIN_DIR/oino"
say "Start with: oino"
