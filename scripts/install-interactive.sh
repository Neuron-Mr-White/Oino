#!/usr/bin/env sh
set -eu

# Interactive Oino installer. It delegates core install to scripts/install.sh,
# then optionally installs selected checked-in additional extension packages.

say() { printf '%s\n' "$*"; }
have() { command -v "$1" >/dev/null 2>&1; }
download() {
  url=$1
  out=$2
  if have curl; then curl -fsSL "$url" -o "$out"; elif have wget; then wget -qO "$out" "$url"; else return 1; fi
}
ask() {
  prompt=$1
  default=${2:-}
  if [ -n "$default" ]; then
    printf '%s [%s]: ' "$prompt" "$default"
  else
    printf '%s: ' "$prompt"
  fi
  read -r answer || answer=""
  if [ -z "$answer" ]; then answer=$default; fi
  printf '%s' "$answer"
}
yesno() {
  prompt=$1
  default=${2:-y}
  answer=$(ask "$prompt" "$default")
  case "$answer" in
    y|Y|yes|YES|Yes) return 0 ;;
    *) return 1 ;;
  esac
}

SCRIPT_DIR=$(CDPATH= cd -- "$(dirname -- "$0")" 2>/dev/null && pwd || pwd)
ROOT=$(CDPATH= cd -- "$SCRIPT_DIR/.." 2>/dev/null && pwd || pwd)
if [ -f "$ROOT/scripts/install.sh" ]; then
  cd "$ROOT"
  INSTALL_SH="scripts/install.sh"
  INSTALL_ALL_BUILTINS="scripts/install-all-builtins.sh"
else
  TMP=${TMPDIR:-/tmp}/oino-interactive-$$
  mkdir -p "$TMP"
  INSTALL_SH="$TMP/install.sh"
  download "https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/scripts/install.sh" "$INSTALL_SH" || {
    say "Could not download scripts/install.sh; run from a source checkout instead."
    exit 1
  }
  INSTALL_ALL_BUILTINS=""
fi

say "Oino interactive installer"
say ""

if yesno "Install/update Oino core binary?" y; then
  :
else
  OINO_SKIP_CORE=1
  export OINO_SKIP_CORE
fi

builtins="9router footer-status ralph-loop mode-sandbox notify craft-skill vcc ask-user"
selected_builtins=""
if yesno "Install built-in extensions?" y; then
  for name in $builtins; do
    default=n
    case "$name" in
      9router|footer-status|vcc|ask-user) default=y ;;
    esac
    if yesno "  include builtin:$name?" "$default"; then
      if [ -z "$selected_builtins" ]; then selected_builtins=$name; else selected_builtins=$selected_builtins,$name; fi
    fi
  done
else
  selected_builtins=none
fi

additional_selected=""
if [ -d extensions/additional ]; then
  for package in extensions/additional/*; do
    [ -f "$package/oino.package.json" ] || continue
    name=$(basename "$package")
    if yesno "Install additional extension $name?" n; then
      additional_selected="$additional_selected $package"
    fi
  done
fi

if [ "${OINO_SKIP_CORE:-0}" = "1" ]; then
  say "Skipping Oino core install"
  if [ "$selected_builtins" != "none" ] && [ -n "$selected_builtins" ]; then
    if [ -n "$INSTALL_ALL_BUILTINS" ]; then
      OINO_BUILTINS=$selected_builtins bash "$INSTALL_ALL_BUILTINS"
    else
      say "Built-in-only install without core requires a source checkout; skipping built-ins."
    fi
  fi
else
  OINO_EXTENSIONS=$selected_builtins sh "$INSTALL_SH"
fi

if [ -n "$additional_selected" ]; then
  OINO_ADDITIONAL_SELECTION=$additional_selected python3 - <<'PY'
import json, os, shutil
from pathlib import Path
home = Path(os.environ.get('OINO_HOME') or Path.home())
target = home / '.oino' / 'extension-packages'
settings_path = home / '.oino' / 'settings.json'
settings = {}
if settings_path.exists():
    settings = json.loads(settings_path.read_text())
settings.setdefault('extensions', {}).setdefault('packages', {})
target.mkdir(parents=True, exist_ok=True)
count = 0
for raw in os.environ['OINO_ADDITIONAL_SELECTION'].split():
    src = Path(raw)
    manifest = json.loads((src / 'oino.package.json').read_text())
    package_id = manifest['id']
    dst = target / package_id
    if dst.exists():
        shutil.rmtree(dst)
    shutil.copytree(src, dst)
    (dst / '.oino-install-source.json').write_text(json.dumps({'source': str(src)}, indent=2, sort_keys=True) + '\n')
    settings['extensions']['packages'][package_id] = 'enabled'
    count += 1
settings_path.parent.mkdir(parents=True, exist_ok=True)
settings_path.write_text(json.dumps(settings, indent=2, sort_keys=True) + '\n')
print(f'Installed and enabled {count} additional extension packages in {target}')
PY
fi

say "Interactive install complete. Start with: oino"
