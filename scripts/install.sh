#!/usr/bin/env sh
set -eu

# Oino hybrid installer:
# - prefers tagged GitHub release binaries when available
# - falls back to source checkout + cargo build for unsupported targets
# - optionally installs built-in extension packages
#
# Environment:
#   OINO_REPO       git URL to clone when source fallback is used
#   OINO_REPO_SLUG  GitHub owner/repo for release assets; default Neuron-Mr-White/Oino
#   OINO_REF        release tag/git ref; default latest release for binary, default branch for source
#   OINO_PREFIX     install prefix; default ~/.local
#   OINO_DIR        source checkout dir; default ~/.cache/oino/source
#   OINO_EXTENSIONS none|all|comma-separated built-in aliases; default all
#   OINO_DRY_RUN    set to 1 to print actions without making changes where practical
#   OINO_FROM_SOURCE set to 1 to skip release binary lookup

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
  url=$1
  out=$2
  if have curl; then
    run curl -fsSL "$url" -o "$out"
  elif have wget; then
    run wget -qO "$out" "$url"
  else
    return 1
  fi
}

PREFIX=${OINO_PREFIX:-"$HOME/.local"}
BIN_DIR="$PREFIX/bin"
REPO=${OINO_REPO:-"https://github.com/Neuron-Mr-White/Oino.git"}
REPO_SLUG=${OINO_REPO_SLUG:-"Neuron-Mr-White/Oino"}
SRC_DIR=${OINO_DIR:-"$HOME/.cache/oino/source"}
EXTENSIONS=${OINO_EXTENSIONS:-all}
EXT_BUNDLE_URL=""
EXT_BUNDLE_SHA=""

os=$(uname -s 2>/dev/null || printf unknown)
arch=$(uname -m 2>/dev/null || printf unknown)
case "$arch" in
  x86_64|amd64) arch=x86_64 ;;
  arm64|aarch64) arch=aarch64 ;;
  armv7*|armv7l) arch=armv7 ;;
esac
case "$os" in
  Linux) target="$arch-unknown-linux-gnu" ;;
  Darwin) target="$arch-apple-darwin" ;;
  MINGW*|MSYS*|CYGWIN*) target="$arch-pc-windows-msvc" ;;
  *) target="$arch-$os" ;;
esac

manifest_url() {
  if [ -n "${OINO_REF:-}" ]; then
    printf 'https://github.com/%s/releases/download/%s/release-manifest.json\n' "$REPO_SLUG" "$OINO_REF"
  else
    printf 'https://github.com/%s/releases/latest/download/release-manifest.json\n' "$REPO_SLUG"
  fi
}

try_binary_install() {
  [ "${OINO_DRY_RUN:-0}" != "1" ] || return 1
  [ "${OINO_FROM_SOURCE:-0}" != "1" ] || return 1
  have python3 || return 1
  tmp=${TMPDIR:-/tmp}/oino-install-$$
  run mkdir -p "$tmp"
  manifest="$tmp/release-manifest.json"
  url=$(manifest_url)
  say "Checking Oino release manifest: $url"
  download "$url" "$manifest" || return 1
  info="$tmp/artifact.txt"
  python3 - "$manifest" "$target" > "$info" <<'PY'
import json, sys
manifest = json.load(open(sys.argv[1]))
target = sys.argv[2]
for artifact in manifest.get('artifacts', []):
    if artifact.get('target') == target and artifact.get('kind', 'binary') == 'binary':
        print(artifact.get('url', ''))
        print(artifact.get('sha256', ''))
        bundle = manifest.get('extensions') or {}
        print(manifest.get('tag', ''))
        print(bundle.get('url', ''))
        print(bundle.get('sha256', ''))
        raise SystemExit(0)
raise SystemExit(1)
PY
  artifact_url=$(sed -n '1p' "$info")
  artifact_sha=$(sed -n '2p' "$info")
  artifact_tag=$(sed -n '3p' "$info")
  EXT_BUNDLE_URL=$(sed -n '4p' "$info")
  EXT_BUNDLE_SHA=$(sed -n '5p' "$info")
  [ -n "$artifact_url" ] || return 1
  bin_tmp="$tmp/oino"
  say "Installing Oino $artifact_tag binary for $target"
  download "$artifact_url" "$bin_tmp" || return 1
  if [ -n "$artifact_sha" ] && [ "${OINO_DRY_RUN:-0}" != "1" ]; then
    python3 - "$bin_tmp" "$artifact_sha" <<'PY'
import hashlib, sys
actual = hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest()
expected = sys.argv[2].strip().lower()
if actual.lower() != expected:
    raise SystemExit(f'sha256 mismatch: expected {expected}, got {actual}')
PY
  fi
  run mkdir -p "$BIN_DIR"
  run chmod +x "$bin_tmp"
  run cp "$bin_tmp" "$BIN_DIR/oino"
  return 0
}

source_install() {
  if [ -f "Cargo.toml" ] && grep -q 'oino-app' Cargo.toml 2>/dev/null; then
    SRC=$(pwd)
  else
    SRC=$SRC_DIR
    if [ ! -d "$SRC/.git" ]; then
      have git || err "git is required for source fallback. Install git, or run from an Oino source checkout."
      run mkdir -p "$(dirname "$SRC")"
      run git clone "$REPO" "$SRC"
    fi
    if [ -n "${OINO_REF:-}" ]; then
      run git -C "$SRC" fetch --all --tags
      run git -C "$SRC" checkout "$OINO_REF"
    fi
  fi

  if ! have cargo; then
    if ! have rustup; then
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
        err "cargo is missing and neither curl nor wget is available. Install Rust from https://rustup.rs/ and rerun."
      fi
    fi
    [ "${OINO_DRY_RUN:-0}" = "1" ] || . "$HOME/.cargo/env"
  fi

  run cargo build --manifest-path "$SRC/Cargo.toml" -p oino-app --bin oino --release
  run mkdir -p "$BIN_DIR"
  run cp "$SRC/target/release/oino" "$BIN_DIR/oino"
}

install_extensions_from_dir() {
  packages_dir=$1
  target_home=${OINO_HOME:-$HOME}
  python3 - "$packages_dir" "$target_home/.oino/extension-packages" "$target_home/.oino/settings.json" "${OINO_BUILTINS:-}" <<'PY'
import json, shutil, sys
from pathlib import Path
packages_dir = Path(sys.argv[1])
target_dir = Path(sys.argv[2])
settings_path = Path(sys.argv[3])
requested = [item.strip() for item in sys.argv[4].split(',') if item.strip()]
aliases = {
    '9router': 'oino.9router',
    'footer-status': 'oino.footer_status',
    'ralph-loop': 'oino.ralph_loop',
    'mode-sandbox': 'oino.mode_sandbox',
    'notify': 'oino.notify',
    'craft-skill': 'oino.craft_skill',
    'vcc': 'oino.vcc',
    'ask-user': 'oino.ask_user',
}
manifest_by_id = {}
for manifest_path in packages_dir.glob('*/oino.package.json'):
    manifest = json.loads(manifest_path.read_text())
    manifest_by_id[manifest['id']] = manifest_path
package_ids = list(manifest_by_id)
if requested:
    package_ids = []
    for item in requested:
        package_id = aliases.get(item, item)
        if package_id not in manifest_by_id:
            raise SystemExit(f"unknown built-in package {item}; available: {', '.join(sorted(aliases))}")
        package_ids.append(package_id)
target_dir.mkdir(parents=True, exist_ok=True)
for package_id in package_ids:
    src = manifest_by_id[package_id].parent
    dst = target_dir / package_id
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)
    (dst / '.oino-install-source.json').write_text(json.dumps({'source': f"builtin:{src.name}"}, indent=2, sort_keys=True) + '\n')
settings = {}
if settings_path.exists():
    settings = json.loads(settings_path.read_text())
settings.setdefault('extensions', {}).setdefault('packages', {})
for package_id in package_ids:
    settings['extensions']['packages'][package_id] = 'enabled'
settings_path.parent.mkdir(parents=True, exist_ok=True)
settings_path.write_text(json.dumps(settings, indent=2, sort_keys=True) + '\n')
print(f"Installed and enabled {len(package_ids)} Oino built-in packages in {target_dir}")
PY
}

install_extensions_from_bundle() {
  [ -n "$EXT_BUNDLE_URL" ] || return 1
  have python3 || return 1
  have tar || return 1
  tmp=${TMPDIR:-/tmp}/oino-extensions-$$
  run mkdir -p "$tmp"
  bundle="$tmp/extensions.tar.gz"
  say "Installing built-in extensions from release bundle"
  download "$EXT_BUNDLE_URL" "$bundle" || return 1
  if [ -n "$EXT_BUNDLE_SHA" ] && [ "${OINO_DRY_RUN:-0}" != "1" ]; then
    python3 - "$bundle" "$EXT_BUNDLE_SHA" <<'PY'
import hashlib, sys
actual = hashlib.sha256(open(sys.argv[1], 'rb').read()).hexdigest()
expected = sys.argv[2].strip().lower()
if actual.lower() != expected:
    raise SystemExit(f'sha256 mismatch: expected {expected}, got {actual}')
PY
  fi
  run tar -xzf "$bundle" -C "$tmp"
  install_extensions_from_dir "$tmp/extensions/built-in"
}

install_extensions() {
  case "$EXTENSIONS" in
    none|false|0) say "Skipping built-in extension install"; return 0 ;;
    all|true|1) unset OINO_BUILTINS ;;
    *) OINO_BUILTINS=$EXTENSIONS; export OINO_BUILTINS ;;
  esac
  if [ -x "${SRC:-$(pwd)}/scripts/install-all-builtins.sh" ]; then
    if have bash; then
      run bash "${SRC:-$(pwd)}/scripts/install-all-builtins.sh"
    else
      say "bash not found; skipping optional built-in extension install"
    fi
  elif install_extensions_from_bundle; then
    :
  else
    say "Built-in installer script or release extension bundle not available; skipping extension install"
  fi
}

if ! try_binary_install; then
  say "No matching release binary for $target; falling back to source build."
  source_install
fi
install_extensions

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "Add this to your shell profile if needed: export PATH=\"$BIN_DIR:\$PATH\"" ;;
esac

say "Oino installed: $BIN_DIR/oino"
say "Start with: oino"
