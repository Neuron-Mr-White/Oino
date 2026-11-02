#!/usr/bin/env sh
# Oino installer (macOS / Linux / other Unix).
#
# Default: install the `oino` CLI/TUI from the latest GitHub release into
# $OINO_PREFIX (default ~/.local): the binary goes to `<prefix>/bin`, the
# extension packages to `<prefix>/share/oino/extensions` (the loader's first
# exe-relative share dir, dist.md §5).
#
#   --app        install the Unipi desktop app instead:
#                  Linux: AppImage into ~/.local/bin + .desktop + icon
#                         (--deb: the .deb via apt, needs sudo)
#                  macOS: .dmg → /Applications/UniPi.app, quarantine cleared
#   --host       install the agent host without the desktop app (GUI-less
#                machines, sandboxes): `unipi-oino-host` + `oino` into
#                <prefix>/bin and the bundled extensions into
#                <prefix>/share/oino/extensions. Safe to rerun (upgrades in
#                place, also while the host runs).
#                Also installs a user service that restarts the host when it
#                fails (Linux: systemd --user unit; macOS: a LaunchAgent):
#                  --no-service  skip it    --no-start  write it, don't start
#                  --uninstall   remove the service + host (data is kept)
#   --prefix DIR install prefix (same as OINO_PREFIX; default ~/.local)
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

app=0 deb=0 host=0 service=1 start=1 uninstall=0
while [ $# -gt 0 ]; do
  case "$1" in
    --app) app=1 ;;
    --deb) deb=1 ;;
    --host) host=1 ;;
    --no-service) service=0 ;;
    --no-start) start=0 ;;
    --uninstall) uninstall=1 ;;
    --prefix)
      [ $# -ge 2 ] && [ -n "$2" ] || err "--prefix needs a directory"
      PREFIX=$2; shift ;;
    --prefix=*) PREFIX=${1#--prefix=}; [ -n "$PREFIX" ] || err "--prefix needs a directory" ;;
    -h|--help) sed -n '2,36p' "$0"; exit 0 ;;
    *) err "unknown argument: $1 (want --app / --deb / --host / --prefix DIR)" ;;
  esac
  shift
done
[ "$app" = 1 ] && [ "$host" = 1 ] && err "--app and --host are separate installs; run them one at a time"
case "$PREFIX" in
  /*) ;;
  *) PREFIX="$(pwd)/$PREFIX" ;; # the service unit needs absolute paths
esac
BIN_DIR="$PREFIX/bin"

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

# Sets SRC to an Oino checkout (this dir, else a clone in OINO_DIR) and makes
# sure cargo exists.
prepare_source() {
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
}

source_install() {
  prepare_source
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
    *) err "no Unipi desktop installer for $os (see apps/unipi/BUILDING.md)" ;;
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
    say "Installed Unipi (.deb). Run: unipi-mobile (or launch Unipi from the menu)"
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
    printf 'Name=Unipi\n'
    printf 'Comment=The Unipi agent workspace (oino)\n'
    printf 'Exec=%s%s\n' "$BIN_DIR/UniPi.AppImage" "$exec_args"
    printf 'Icon=%s\n' "${icon_name:-unipi}"
    printf 'Terminal=false\n'
    printf 'Categories=Development;\n'
  } > "$desktop" 2>/dev/null || desktop=""
  if [ "$desktop" ] && [ "$DRY_RUN" != "1" ]; then
    say "Wrote $desktop"
  fi
  have update-desktop-database && run update-desktop-database "$HOME/.local/share/applications"
  say "Installed Unipi: $BIN_DIR/UniPi.AppImage (+ menu entry)"
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

# ── the host without the GUI (--host) ───────────────────────────────────────
# Release asset: unipi-oino-host-<version>-<target>.tar.gz =
#   bin/unipi-oino-host  bin/oino  share/oino/extensions/<pkg>/
# (scripts/release/pack-host.sh). Both binaries find the extensions through
# their exe-relative <prefix>/share/oino/extensions candidate (dist.md §5).

HOST_BIN=unipi-oino-host
EXT_DIR="$PREFIX/share/oino/extensions"

# place_host_tree <staged root> — move a staged bin/ + share/ tree into the
# prefix. Binaries are renamed over the old ones (a running host keeps its
# inode, no "text file busy"); the extension tree is swapped as a whole so a
# package dropped by the new release does not linger.
place_host_tree() {
  stage=$1
  [ -x "$stage/bin/$HOST_BIN" ] || err "archive layout changed: no bin/$HOST_BIN"
  [ -x "$stage/bin/oino" ] || err "archive layout changed: no bin/oino"
  mkdir -p "$BIN_DIR" "$PREFIX/share/oino"
  for name in "$HOST_BIN" oino; do
    cp "$stage/bin/$name" "$BIN_DIR/.$name.new"
    chmod 755 "$BIN_DIR/.$name.new"
    mv -f "$BIN_DIR/.$name.new" "$BIN_DIR/$name"
  done
  if [ -d "$stage/share/oino/extensions" ]; then
    rm -rf "$EXT_DIR.new" "$EXT_DIR.old"
    cp -R "$stage/share/oino/extensions" "$EXT_DIR.new"
    if [ -d "$EXT_DIR" ]; then mv "$EXT_DIR" "$EXT_DIR.old"; fi
    mv "$EXT_DIR.new" "$EXT_DIR"
    rm -rf "$EXT_DIR.old"
  fi
  say "Placed $BIN_DIR/$HOST_BIN, $BIN_DIR/oino and $(ls -1 "$EXT_DIR" 2>/dev/null | wc -l | tr -d ' ') extension packages in $EXT_DIR"
}

install_host_release() {
  [ "${OINO_FROM_SOURCE:-0}" != "1" ] || return 1
  [ -n "$target" ] || return 1
  fetch_sums || return 1
  htmp=$(mktemp -d "${TMPDIR:-/tmp}/oino-host.XXXXXX")
  trap 'rm -rf "$htmp" "$sums"' EXIT INT TERM
  fetch_asset "$HOST_BIN-.*-$target\\.tar\\.gz\$" "$htmp/host.tar.gz" || return 1
  mkdir -p "$htmp/x"
  tar -xzf "$htmp/host.tar.gz" -C "$htmp/x"
  place_host_tree "$htmp/x"
  rm -rf "$htmp"
  return 0
}

# The fallback: build both binaries from a checkout (heavy: needs Rust and a
# few GB of RAM), stage them like the release archive, then place them.
source_host_install() {
  prepare_source
  (cd "$SRC" && run cargo xtask wasm)
  run cargo build --manifest-path "$SRC/Cargo.toml" --release -p oino-cli
  # unipi-host lives in the apps/unipi cargo workspace.
  run cargo build --manifest-path "$SRC/apps/unipi/Cargo.toml" --release -p unipi-host
  htmp=$(mktemp -d "${TMPDIR:-/tmp}/oino-host-src.XXXXXX")
  trap 'rm -rf "$htmp" "$sums"' EXIT INT TERM
  mkdir -p "$htmp/bin" "$htmp/share/oino"
  cp "$SRC/target/release/oino" "$htmp/bin/oino"
  cp "$SRC/apps/unipi/target/release/unipi-host" "$htmp/bin/$HOST_BIN"
  bash "$SRC/scripts/release/stage-dist.sh" "$htmp/share/oino"
  place_host_tree "$htmp"
  rm -rf "$htmp"
}

install_host() {
  if [ "$DRY_RUN" = "1" ]; then
    say "dry run: would install $HOST_BIN + oino (${target:-source build}) into $PREFIX"
    say "  release: $(base_url) ($HOST_BIN-<version>-${target:-?}.tar.gz)"
    say "  bin: $BIN_DIR  extensions: $EXT_DIR"
    return 0
  fi
  if install_host_release; then
    say "Installed the host into $PREFIX"
  else
    say "No host release for ${target:-this platform} (or download failed); building from source."
    source_host_install
    say "Built and installed the host from source into $PREFIX"
  fi
}

# ── the host as a user service (--host) ─────────────────────────────────────
# Linux: a systemd --user unit, macOS: a LaunchAgent. Both restart the host when
# it fails (crash, kill, self-update exit 76) and leave it stopped when it exits
# 75 (another host — the desktop app's — already owns the data dir; the app
# adopts a running host, so start the service first on a PC that has both).
# `serve --mode service` shows up as "Run by the background service" in the app.

SERVICE_NAME=unipi-oino-host
LAUNCHD_LABEL=dev.unipi.oino.host
UNIT_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/systemd/user"
UNIT_FILE="$UNIT_DIR/$SERVICE_NAME.service"
PLIST_FILE="$HOME/Library/LaunchAgents/$LAUNCHD_LABEL.plist"

# The host's agents run tools (bash, git, language servers…) with the
# service's PATH: take this shell's PATH, prefix the install's bin dir.
service_path() {
  case ":$PATH:" in
    *":$BIN_DIR:"*) printf '%s' "$PATH" ;;
    *) printf '%s:%s' "$BIN_DIR" "$PATH" ;;
  esac
}

# sd_quote <text> — one systemd word: double-quoted, \ and " escaped, % doubled
# (specifier syntax), $ doubled (environment expansion in ExecStart).
sd_quote() {
  printf '"%s"' "$(printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/%/%%/g' -e 's/\$/$$/g')"
}

# sd_env <NAME> <value> — an Environment= line (no $ expansion there).
sd_env() {
  printf 'Environment=%s\n' "$(printf '%s=%s' "$1" "$2" \
    | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g' -e 's/%/%%/g' -e 's/^/"/' -e 's/$/"/')"
}

systemd_unit() {
  printf '[Unit]\n'
  printf 'Description=UniPi Oino host (agent sessions for the Unipi desktop and phone apps)\n'
  printf 'Documentation=https://github.com/%s#host\n' "$REPO_SLUG"
  # A host that keeps dying fast stops being restarted after 5 tries in 2 min
  # (journalctl --user -u %s shows why); `systemctl --user restart` resets it.
  printf 'StartLimitIntervalSec=120\nStartLimitBurst=5\n\n'
  printf '[Service]\n'
  printf 'Type=simple\n'
  printf 'ExecStart=%s serve --mode service\n' "$(sd_quote "$BIN_DIR/$HOST_BIN")"
  printf 'Restart=on-failure\n'
  printf 'RestartSec=3\n'
  # 75 = another host already owns the data dir: restarting cannot help.
  printf 'RestartPreventExitStatus=75\n'
  # SIGTERM: the host closes its sessions and removes its sockets.
  printf 'TimeoutStopSec=30\n'
  sd_env PATH "$(service_path)"
  for var in OINO_HOST_DIR OINO_AGENT_DIR RUST_LOG; do
    eval "val=\${$var:-}"
    [ -n "$val" ] && sd_env "$var" "$val"
  done
  printf '\n[Install]\nWantedBy=default.target\n'
}

# xml_escape <text>
xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

launchd_plist() {
  log="$HOME/Library/Logs/$SERVICE_NAME.log"
  # /bin/sh wrapper: launchd has no "don't restart on exit 75", so 75 is turned
  # into a clean exit; the log is rotated at start past 5 MB (bounded, like
  # the Windows host.log).
  wrapper='log=$1; bin=$2; if [ -n "$(find "$log" -size +5120k 2>/dev/null)" ]; then mv -f "$log" "$log.1"; fi; "$bin" serve --mode service >>"$log" 2>&1; c=$?; [ "$c" -eq 75 ] && exit 0; exit "$c"'
  printf '<?xml version="1.0" encoding="UTF-8"?>\n'
  printf '<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">\n'
  printf '<plist version="1.0">\n<dict>\n'
  printf '  <key>Label</key><string>%s</string>\n' "$LAUNCHD_LABEL"
  printf '  <key>ProgramArguments</key>\n  <array>\n'
  for a in /bin/sh -c "$wrapper" "$SERVICE_NAME" "$log" "$BIN_DIR/$HOST_BIN"; do
    printf '    <string>%s</string>\n' "$(xml_escape "$a")"
  done
  printf '  </array>\n'
  printf '  <key>RunAtLoad</key><true/>\n'
  printf '  <key>KeepAlive</key>\n  <dict><key>SuccessfulExit</key><false/></dict>\n'
  printf '  <key>ThrottleInterval</key><integer>10</integer>\n'
  printf '  <key>ProcessType</key><string>Background</string>\n'
  printf '  <key>EnvironmentVariables</key>\n  <dict>\n'
  printf '    <key>PATH</key><string>%s</string>\n' "$(xml_escape "$(service_path)")"
  for var in OINO_HOST_DIR OINO_AGENT_DIR RUST_LOG; do
    eval "val=\${$var:-}"
    [ -n "$val" ] && printf '    <key>%s</key><string>%s</string>\n' "$var" "$(xml_escape "$val")"
  done
  printf '  </dict>\n</dict>\n</plist>\n'
}

# Is a systemd user manager reachable from this shell?
have_user_systemd() {
  have systemctl && systemctl --user show-environment >/dev/null 2>&1
}

linger_hint() {
  have loginctl || return 0
  user=${USER:-$(id -un)}
  if [ "$(loginctl show-user "$user" -p Linger --value 2>/dev/null)" != "yes" ]; then
    say ""
    say "The host stops when you log out. To keep it running (and start it at boot"
    say "on a machine nobody logs into), run once:"
    say "  loginctl enable-linger $user"
  fi
}

install_service_linux() {
  mkdir -p "$UNIT_DIR"
  systemd_unit > "$UNIT_FILE.tmp" && mv -f "$UNIT_FILE.tmp" "$UNIT_FILE"
  say "Wrote $UNIT_FILE"
  if [ "$start" != 1 ]; then
    say "Not started (--no-start). Start it: systemctl --user daemon-reload && systemctl --user enable --now $SERVICE_NAME"
    return 0
  fi
  if ! have_user_systemd; then
    say "No systemd user session here (container / no login session): the unit is written but not started."
    say "Start the host by hand: $BIN_DIR/$HOST_BIN serve"
    return 0
  fi
  run systemctl --user daemon-reload
  run systemctl --user enable "$SERVICE_NAME.service"
  # restart = start, or pick up the new binary on a rerun
  run systemctl --user restart "$SERVICE_NAME.service"
  sleep 2
  if systemctl --user is-active --quiet "$SERVICE_NAME.service"; then
    say "The host runs as the user service $SERVICE_NAME (systemctl --user status $SERVICE_NAME)."
  else
    say "warning: $SERVICE_NAME did not stay up. Why: journalctl --user -u $SERVICE_NAME -n 30"
    say "  (exit 75 = a host already runs on this data dir, e.g. the desktop app's)"
  fi
  linger_hint
}

install_service_macos() {
  mkdir -p "$(dirname "$PLIST_FILE")" "$HOME/Library/Logs"
  launchd_plist > "$PLIST_FILE.tmp" && mv -f "$PLIST_FILE.tmp" "$PLIST_FILE"
  say "Wrote $PLIST_FILE"
  if [ "$start" != 1 ]; then
    say "Not started (--no-start). Start it: launchctl bootstrap gui/$(id -u) $PLIST_FILE"
    return 0
  fi
  domain="gui/$(id -u)"
  launchctl bootout "$domain/$LAUNCHD_LABEL" >/dev/null 2>&1 || true
  run launchctl bootstrap "$domain" "$PLIST_FILE"
  say "The host runs as the LaunchAgent $LAUNCHD_LABEL (log: ~/Library/Logs/$SERVICE_NAME.log)."
}

install_service() {
  if [ "$service" != 1 ]; then
    say "No service (--no-service). Start the host by hand: $BIN_DIR/$HOST_BIN serve"
    return 0
  fi
  if [ "$DRY_RUN" = "1" ]; then
    case "$os" in
      Linux) say "  service: $UNIT_FILE (systemd --user, Restart=on-failure)" ;;
      Darwin) say "  service: $PLIST_FILE (launchd, KeepAlive on failure)" ;;
    esac
    return 0
  fi
  case "$os" in
    Linux) install_service_linux ;;
    Darwin) install_service_macos ;;
    *) say "No user-service support for $os here; start the host by hand: $BIN_DIR/$HOST_BIN serve" ;;
  esac
}

# --host --uninstall: stop + remove the service and the host binary. The oino
# CLI, the extensions and the host's data (~/.oino/host: pairings, key) stay.
uninstall_host() {
  case "$os" in
    Linux)
      if [ -f "$UNIT_FILE" ]; then
        if have_user_systemd; then
          run systemctl --user disable --now "$SERVICE_NAME.service" || true
        fi
        run rm -f "$UNIT_FILE"
        have_user_systemd && run systemctl --user daemon-reload
      fi
      ;;
    Darwin)
      if [ -f "$PLIST_FILE" ]; then
        launchctl bootout "gui/$(id -u)/$LAUNCHD_LABEL" >/dev/null 2>&1 || true
        run rm -f "$PLIST_FILE"
      fi
      ;;
  esac
  run rm -f "$BIN_DIR/$HOST_BIN"
  say "Removed the host service and $BIN_DIR/$HOST_BIN."
  say "Kept: $BIN_DIR/oino, $EXT_DIR and the host data (~/.oino/host — pairings, key)."
  say "Remove those too: rm -rf \"$BIN_DIR/oino\" \"$EXT_DIR\" ~/.oino/host"
}

# ── main ────────────────────────────────────────────────────────────────────

if [ "$host" = "1" ]; then
  if [ "$uninstall" = "1" ]; then
    uninstall_host
    exit 0
  fi
  install_host
  install_service
  add_to_path
  path_hint
  say "Done. Pair the desktop or phone app: $BIN_DIR/$HOST_BIN pair"
  exit 0
fi
[ "$uninstall" = "1" ] && err "--uninstall goes with --host"

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
