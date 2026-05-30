#!/usr/bin/env bash
set -euo pipefail

# Install every checked-in optional Oino built-in extension package globally.
# Usage from a source checkout:
#   bash scripts/install-all-builtins.sh
# Optional:
#   OINO_HOME=/custom/home bash scripts/install-all-builtins.sh

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OINO_HOME="${OINO_HOME:-$HOME}"
PACKAGES_DIR="$ROOT/crates/oino-extension-builtins/packages"
TARGET_DIR="$OINO_HOME/.oino/extension-packages"
SETTINGS_PATH="$OINO_HOME/.oino/settings.json"

python3 - "$PACKAGES_DIR" "$TARGET_DIR" "$SETTINGS_PATH" <<'PY'
import json
import shutil
import sys
from pathlib import Path

packages_dir = Path(sys.argv[1])
target_dir = Path(sys.argv[2])
settings_path = Path(sys.argv[3])

package_ids = [
    "oino.router",
    "oino.footer_status",
    "oino.ralph_loop",
    "oino.mode_sandbox",
    "oino.notify",
    "oino.craft_skill",
    "oino.vcc",
    "oino.ask_user",
]

target_dir.mkdir(parents=True, exist_ok=True)
for package_id in package_ids:
    package_file = next(
        (path for path in packages_dir.glob("*/oino.package.json")
         if json.loads(path.read_text()).get("id") == package_id),
        None,
    )
    if package_file is None:
        raise SystemExit(f"missing built-in package {package_id}")
    destination = target_dir / package_id
    if destination.exists():
        shutil.rmtree(destination)
    shutil.copytree(package_file.parent, destination)
    source_record = {"source": f"builtin:{package_file.parent.name}"}
    (destination / ".oino-install-source.json").write_text(
        json.dumps(source_record, indent=2, sort_keys=True) + "\n"
    )

settings = {}
if settings_path.exists():
    try:
        settings = json.loads(settings_path.read_text())
    except json.JSONDecodeError as exc:
        raise SystemExit(f"settings file is not valid JSON: {settings_path}: {exc}")
settings.setdefault("extensions", {}).setdefault("packages", {})
for package_id in package_ids:
    settings["extensions"]["packages"][package_id] = "enabled"
settings_path.parent.mkdir(parents=True, exist_ok=True)
settings_path.write_text(json.dumps(settings, indent=2, sort_keys=True) + "\n")
print(f"Installed and enabled {len(package_ids)} Oino built-in packages in {target_dir}")
PY
