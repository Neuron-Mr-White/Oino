#!/usr/bin/env sh
# Oino installer (macOS / Linux / other Unix).
#
# Default: install the `oino` CLI/TUI from the latest GitHub release into
# $OINO_PREFIX (default ~/.local): the binary goes to `<prefix>/bin`, the
# extension packages to `<prefix>/share/oino/extensions` (the loader's first
# exe-relative share dir, dist.md §5).
#
#   --app        install the UniPi desktop app instead:
#                  Linux: AppImage into ~/.local/bin + .desktop + icon
#                         (--deb: the .deb via apt, needs sudo)
#                  macOS: .dmg → /Applications/UniPi.app, quarantine cleared
#   --help       usage
#
# Every download is verified against the release's SHA256SUMS. Without a
# matching release binary the CLI falls back to a source build.
#
# Environment:
#   OINO_INSTALL_BASE_URL  asset base (default: the GitHub release permalink);
#                          may be http(s)://… or file:///… (tests: point it at
#                          a local dir laid out as <base>/SHA256SUMS + assets)
#   OINO_PREFIX            install prefix (default ~/.local)
#   OINO_REF               release tag to install (default: latest)
#   OINO_FROM_SOURCE       1 = skip the release download
#   OINO_DIR               source checkout used by the fallback
#   OINO_DRY_RUN           1 = print the plan, touch nothing
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

PREFIX=${OINO_PREFIX:-"$HOME/.local"}
BIN_DIR="$PREFIX/bin"
REPO_SLUG=${OINO_REPO_SLUG:-"Neuron-Mr-White/Oino"}
REPO=${OINO_REPO:-"https://github.com/$REPO_SLUG.git"}
SRC_DIR=${OINO_DIR:-"$HOME/.cache/oino/source"}
DRY_RUN=${OINO_DRY_RUN:-0}

app=0 deb=0
for arg in "$@"; do
  case "$arg" in
    --app) app=1 ;;
    --deb) deb=1 ;;
    -h|--help) sed -n '3,26p' "$0"; exit 0 ;;
    *) err "unknown argument: $arg (want --app / --deb)" ;;
  esac
done

# ── release layout (release.yml) ────────────────────────────────────────────
# Assets live next to SHA256SUMS; `latest` is a GitHub permalink. A custom
# OINO_INSTALL_BASE_URL replaces everything before the asset name (curl's
# file:// makes a plain directory work).

base_url() {
  if [ -n "${OINO_INSTALL_BASE_URL:-}" ]; then
    printf '%s' "$OINO_INSTALL_BASE_URL"
  elif [ -n "${OINO_REF:-}" ]; then
    printf 'https://github.com/%s/releases/download/%s' "$REPO_SLUG" "$OINO_REF"
  else
    printf 'https://github.com/%s/releases/latest/download' "$REPO_SLUG"
  fi
}

download() { # <url> <out>
  if have curl; then
    run curl -fsSL "$1" -o "$2"
  elif have wget; then
    run wget -qO "$2" "$1"
  else
    return 1
  fi
}

os=$(uname -s 2>/dev/null || printf unknown)
arch=$(uname -m 2>/dev/null || printf unknown)
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
esac
case "$os" in
  # Releases ship static musl on Linux (runs on glibc and musl alike).
  Linux) target="$arch-unknown-linux-musl" ;;
  Darwin) target="$arch-apple-darwin" ;;
  *) target="" ;;
esac

# ── checksums ───────────────────────────────────────────────────────────────

sums=$(mktemp "${TMPDIR:-/tmp}/oino-sums.XXXXXX")
trap 'rm -f "$sums"' EXIT INT TERM

fetch_sums() {
  download "$(base_url)/SHA256SUMS" "$sums" || return 1
  [ -s "$sums" ] || return 1
}

# find_asset <egrep-pattern> → the asset file name (from SHA256SUMS)
find_asset() {
  grep -E "[[:space:]]$1" "$sums" 2>/dev/null | head -1 | awk '{ print $2 }'
}

# verify <file> <expected-sha256>
verify() {
  actual=""
  if have sha256sum; then
    actual=$(sha256sum "$1" | awk '{ print $1 }')
  elif have shasum; then
    actual=$(shasum -a 256 "$1" | awk '{ print $1 }')
  else
    say "warning: no sha256 tool found; skipping verification"
    return 0
  fi
  [ "$actual" = "$2" ] || err "sha256 mismatch for $(basename "$1") — aborting"
}

# fetch_asset <egrep-pattern> <out> → 0; downloads + verifies
fetch_asset() {
  name=$(find_asset "$1") || name=""
  [ -n "$name" ] || return 1
  download "$(base_url)/$name" "$2" || return 1
  sha=$(awk -v f="$name" '$2 == f { print $1; exit }' "$sums")
  [ -n "$sha" ] || return 1
  if [ "$DRY_RUN" != "1" ]; then
    verify "$2" "$sha"
  fi
  return 0
}

# ── CLI from the release ────────────────────────────────────────────────────

install_cli_release() {
  [ "${OINO_FROM_SOURCE:-0}" != "1" ] || return 1
  [ -n "$target" ] || return 1
  fetch_sums || return 1
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/oino-install.XXXXXX")
  trap 'rm -rf "$tmp" "$sums"' EXIT INT TERM
  fetch_asset "oino-.*-$target\\.tar\\.gz\$" "$tmp/oino.tar.gz" || return 1
  say "Installing oino into $PREFIX (bin + share/oino/extensions)"
  run mkdir -p "$BIN_DIR" "$PREFIX/share/oino"
  run tar -xzf "$tmp/oino.tar.gz" -C "$PREFIX"
  run chmod 755 "$BIN_DIR/oino"
  rm -rf "$tmp"
  return 0
}

# ── CLI from source (fallback, unchanged in spirit) ─────────────────────────

source_install() {
  if [ -f "Cargo.toml" ] && [ -d "crates/oino-core" ]; then
    SRC=$(pwd)
  else
    SRC=$SRC_DIR
    if [ ! -d "$SRC/.git" ]; then
      have git || err "git is required for the source build. Install git, or run from an Oino checkout."
      run mkdir -p "$(dirname "$SRC")"
      run git clone "$REPO" "$SRC"
    fi
    run git -C "$SRC" fetch --all --tags
    if [ -n "${OINO_REF:-}" ]; then
      run git -C "$SRC" checkout "$OINO_REF"
    fi
  fi
  if ! have cargo; then
    have curl || err "cargo is missing. Install Rust from https://rustup.rs/ and rerun."
    say "Installing Rust with rustup..."
    if [ "$DRY_RUN" = "1" ]; then
      say "+ curl https://sh.rustup.rs | sh -s -- -y --profile minimal"
    else
      curl --proto '=https' --tlsv1.2 -fsSL https://sh.rustup.rs | sh -s -- -y --profile minimal
      . "$HOME/.cargo/env"
    fi
  fi
  # rust-toolchain.toml pins the toolchain and the wasm32-wasip2 target.
  (cd "$SRC" && run cargo xtask wasm)
  run cargo build --manifest-path "$SRC/Cargo.toml" --release -p oino-cli
  run mkdir -p "$BIN_DIR" "$PREFIX/share/oino/extensions"
  for name in oino; do
    run cp "$SRC/target/release/$name" "$BIN_DIR/$name"
  done
  # The share tree ships with releases; a source build stages the same packages
  # (fixtures excluded, dist.md §3).
  if [ "$DRY_RUN" != "1" ] && [ -f "$SRC/extensions/dist/checksums.json" ]; then
    for pkg in "$SRC"/extensions/dist/*/; do
      name=$(basename "$pkg")
      case "$name" in oino.fixture.*) continue ;; esac
      rm -rf "$PREFIX/share/oino/extensions/$name"
      cp -R "$pkg" "$PREFIX/share/oino/extensions/$name"
    done
  fi
}

# ── PATH ────────────────────────────────────────────────────────────────────

path_hint() {
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) say "Add $BIN_DIR to PATH, e.g.: export PATH=\"$BIN_DIR:\$PATH\"" ;;
  esac
}

# Marked block appended to ~/.profile (only when the file already exists —
# otherwise printing the hint above is safer than inventing shell config).
add_to_path() {
  [ "$DRY_RUN" = "1" ] && return 0
  profile="$HOME/.profile"
  [ -f "$profile" ] || return 0
  case ":$(cat "$profile" 2>/dev/null):" in
    *":$BIN_DIR:"*) return 0 ;;
  esac
  if printf '\n# added by the oino installer\nexport PATH="%s:$PATH"\n' "$BIN_DIR" >> "$profile"; then
    say "Added $BIN_DIR to PATH in ~/.profile (start a new shell, or: export PATH=\"$BIN_DIR:\$PATH\")"
  fi
}

# ── the desktop app (--app) ─────────────────────────────────────────────────

install_app() {
  [ "$DRY_RUN" = "1" ] && err "OINO_DRY_RUN does not support --app; run it for real"
  fetch_sums || err "could not fetch SHA256SUMS from $(base_url)"
  tmp=$(mktemp -d "${TMPDIR:-/tmp}/oino-app.XXXXXX")
  trap 'rm -rf "$tmp" "$sums"' EXIT INT TERM
  case "$os" in
    Linux) install_app_linux "$tmp" ;;
    Darwin) install_app_macos "$tmp" ;;
    *) err "no UniPi desktop installer for $os (see apps/unipi/BUILDING.md)" ;;
  esac
  rm -rf "$tmp"
}

install_app_linux() {
  tmp=$1
  if [ "$deb" = "1" ]; then
    have apt-get || err "--deb needs apt-get; use the AppImage (default) instead"
    fetch_asset "UniPi_.*_amd64\\.deb\$" "$tmp/unipi.deb" || err "no .deb in the release (SHA256SUMS)"
    if [ "$(id -u)" = 0 ]; then
      run apt-get install -y "$tmp/unipi.deb"
    else
      have sudo || err "--deb needs sudo (or run as root)"
      run sudo apt-get install -y "$tmp/unipi.deb"
    fi
    say "Installed UniPi (.deb). Run: unipi"
    return 0
  fi
  fetch_asset "UniPi_.*_amd64\\.AppImage\$" "$tmp/UniPi.AppImage" || err "no AppImage in the release (SHA256SUMS)"
  run mkdir -p "$BIN_DIR"
  run cp "$tmp/UniPi.AppImage" "$BIN_DIR/UniPi.AppImage"
  run chmod 755 "$BIN_DIR/UniPi.AppImage"
  # Icon + .desktop: pull the hicolor icons out of the AppImage itself.
  if (cd "$tmp" && "$BIN_DIR/UniPi.AppImage" --appimage-extract 'usr/share/icons/*' >/dev/null 2>&1) \
      && [ -d "$tmp/squashfs-root/usr/share/icons" ]; then
    sizes=$(find "$tmp/squashfs-root/usr/share/icons" -name '*.png' -exec dirname {} \; \
      | while read -r d; do basename "$(dirname "$d")"; done | sort -u)
    # hicolor has no @2 size dirs; prefer the largest plain WxH
    size=$(printf '%s\n' "$sizes" | grep -E '^[0-9]+x[0-9]+$' | sort -n | tail -1)
    [ -n "$size" ] || size=$(printf '%s\n' "$sizes" | sort -n | tail -1)
    icon=$(find "$tmp/squashfs-root/usr/share/icons" -path "*/$size/*" -name '*.png' | head -1)
    if [ -n "$icon" ]; then
      run mkdir -p "$HOME/.local/share/icons/hicolor/$size/apps"
      run cp "$icon" "$HOME/.local/share/icons/hicolor/$size/apps/unipi.png"
      icon_name=unipi.png
    fi
  fi
  # AppImages mount through FUSE (libfuse2 + /dev/fuse). Machines without
  # it (containers, some VMs) still run fine extracted; the menu entry then
  # pays a few seconds of extraction per launch instead of failing outright.
  fuse_ok=1
  [ -e /dev/fuse ] || fuse_ok=0
  if [ "$fuse_ok" = 1 ] && have ldconfig \
      && ! ldconfig -p 2>/dev/null | grep -q 'libfuse\.so\.2'; then
    fuse_ok=0
  fi
  exec_args=""
  if [ "$fuse_ok" = 0 ]; then
    exec_args=" --appimage-extract-and-run"
    say "No usable FUSE (/dev/fuse + libfuse2): the menu entry runs the AppImage extracted (--appimage-extract-and-run)."
  fi
  run mkdir -p "$HOME/.local/share/applications"
  desktop=$HOME/.local/share/applications/unipi.desktop
  {
    printf '[Desktop Entry]\n'
    printf 'Type=Application\n'
    printf 'Name=UniPi\n'
    printf 'Comment=The UniPi agent workspace (oino)\n'
    printf 'Exec=%s%s\n' "$BIN_DIR/UniPi.AppImage" "$exec_args"
    printf 'Icon=%s\n' "${icon_name:-unipi}"
    printf 'Terminal=false\n'
    printf 'Categories=Development;\n'
  } > "$desktop" 2>/dev/null || desktop=""
  if [ "$desktop" ] && [ "$DRY_RUN" != "1" ]; then
    say "Wrote $desktop"
  fi
  have update-desktop-database && run update-desktop-database "$HOME/.local/share/applications"
  say "Installed UniPi: $BIN_DIR/UniPi.AppImage (+ menu entry)"
  return 0
}

install_app_macos() {
  tmp=$1
  fetch_asset "UniPi_.*_universal\\.dmg\$" "$tmp/UniPi.dmg" || err "no .dmg in the release (SHA256SUMS)"
  run hdiutil attach -nobrowse -readonly -mountpoint "$tmp/mnt" "$tmp/UniPi.dmg"
  [ -d "$tmp/mnt/UniPi.app" ] || { hdiutil detach "$tmp/mnt" >/dev/null; err "no UniPi.app inside the dmg"; }
  if [ -d /Applications/UniPi.app ]; then
    run rm -rf /Applications/UniPi.app
  fi
  run cp -R "$tmp/mnt/UniPi.app" /Applications/UniPi.app
  run hdiutil detach "$tmp/mnt" >/dev/null
  # The dmg is unsigned (ad-hoc): Gatekeeper would flag the downloaded
  # quarantine xattr on first launch. We remove it here — that is a trust
  # decision: you are choosing to run this build. See apps/unipi/BUILDING.md.
  run xattr -dr com.apple.quarantine /Applications/UniPi.app
  say "Installed /Applications/UniPi.app (quarantine cleared — see the note above)."
  say "First launch: if macOS still asks, right-click UniPi.app → Open (the build is ad-hoc signed, not notarized)."
  return 0
}

# ── main ────────────────────────────────────────────────────────────────────

if [ "$app" = "1" ]; then
  install_app
  exit 0
fi

if [ "$DRY_RUN" = "1" ]; then
  say "dry run: would install oino (${target:-source build}) into $PREFIX"
  say "  release: $(base_url) (bin/oino + share/oino/extensions)"
  say "  PATH: $BIN_DIR"
  say "Done. Run: oino"
  exit 0
fi

if install_cli_release; then
  say "Installed oino into $PREFIX"
else
  say "No release binary for ${target:-this platform} (or download failed); building from source."
  source_install
  say "Built and installed oino from source into $PREFIX"
fi

add_to_path
path_hint
say "Done. Run: oino"
