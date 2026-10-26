#!/usr/bin/env bash
# Headless screenshots of the --demo script (Xvfb + lavapipe software Vulkan + PIL ImageGrab).
#
#   apps/oino-desktop/scripts/screenshot.sh [out-dir] [binary]
#
# Captures the window at fixed times: streaming + tool preview, approval dialog, final
# state with the side-pane panel. Needs: Xvfb, python3-pil (XCB), mesa Vulkan (lvp).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
apps="$(cd "$here/../.." && pwd)"
out="${1:-$apps/oino-desktop/screenshots}"
bin="${2:-$apps/target/debug/oino-desktop}"
mkdir -p "$out"
out="$(cd "$out" && pwd)"
display=":$((90 + RANDOM % 9))"
Xvfb "$display" -screen 0 1400x900x24 -nolisten tcp >/dev/null 2>&1 &
xvfb=$!
trap 'kill $app $xvfb 2>/dev/null || true' EXIT
sleep 1
agent="$(mktemp -d "${TMPDIR:-$HOME/.cache}/oino-shot.XXXX")"
env -u WAYLAND_DISPLAY DISPLAY="$display" OINO_AGENT_DIR="$agent" \
  VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}" \
  OINO_DESKTOP_DEMO_PROMPT="Make the JSON parser accept trailing commas and run the tests" \
  OINO_DESKTOP_DEMO_APPROVE_MS="${APPROVE_MS:-4000}" \
  "$bin" --demo --cwd "$HOME/Projects/json-lite" &
app=$!
shot() { python3 -c "from PIL import ImageGrab; ImageGrab.grab(xdisplay='$display').save('$out/$1.png')"; echo "$out/$1.png"; }
sleep "${T1:-4}";  shot 01-streaming
sleep "${T2:-3}";  shot 02-approval
sleep "${T3:-9}";  shot 03-done
