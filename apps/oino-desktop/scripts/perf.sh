#!/usr/bin/env bash
# Frame-time check (design.md §2.11): the demo backend with N streaming panes (default 4),
# frame times logged every 120 frames (OINO_FRAME_LOG=1). Fails when the median p99 of the
# logged windows exceeds BUDGET_MS (default 10).
#
#   apps/oino-desktop/scripts/perf.sh [binary] [seconds] [panes]
#
# Headless runs use Xvfb + lavapipe (software Vulkan): numbers there include CPU rasterising
# and are an upper bound; run on a real GPU session (no Xvfb, DISPLAY set) for the budget.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
apps="$(cd "$here/../.." && pwd)"
bin="${1:-$apps/target/release/oino-desktop}"
secs="${2:-20}"
panes="${3:-4}"
budget="${BUDGET_MS:-10}"
agent="$(mktemp -d "${TMPDIR:-$HOME/.cache}/oino-perf.XXXX")"
log="$agent/logs/desktop.log"
xvfb=""
if [ -z "${DISPLAY:-}" ] || [ "${PERF_HEADLESS:-0}" = 1 ]; then
  export DISPLAY=":$((80 + RANDOM % 9))"
  Xvfb "$DISPLAY" -screen 0 1400x900x24 -nolisten tcp >/dev/null 2>&1 &
  xvfb=$!
  export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
  sleep 1
fi
trap 'kill $app ${xvfb} 2>/dev/null || true' EXIT
env -u WAYLAND_DISPLAY OINO_AGENT_DIR="$agent" OINO_FRAME_LOG=1 RUST_LOG=info,oino_frame=info \
  OINO_DESKTOP_DEMO_SCENE=stream OINO_DESKTOP_PANES="$panes" OINO_DESKTOP_THEME=dark \
  "$bin" --demo --cwd "$HOME" >/dev/null 2>&1 &
app=$!
sleep "$secs"
kill "$app" 2>/dev/null || true
wait "$app" 2>/dev/null || true
grep 'frame times' "$log" | tail -n +3 > "$agent/frames.txt" || true
if [ ! -s "$agent/frames.txt" ]; then
  echo "no frame samples in $log" >&2
  exit 2
fi
python3 - "$agent/frames.txt" "$budget" <<'PY'
import re, sys, statistics
rows = [dict(re.findall(r'(\w+)=([\d.]+)', l)) for l in open(sys.argv[1])]
p50 = [float(r['p50_ms']) for r in rows]; p99 = [float(r['p99_ms']) for r in rows]
mx = [float(r['max_ms']) for r in rows]
frames = int(float(rows[-1]['frames'])); dropped = int(float(rows[-1]['dropped']))
print(f"windows={len(rows)} frames={frames} dropped(>16.7ms)={dropped}")
print(f"p50 median={statistics.median(p50):.2f}ms  p99 median={statistics.median(p99):.2f}ms  p99 worst={max(p99):.2f}ms  max={max(mx):.2f}ms")
sys.exit(0 if statistics.median(p99) <= float(sys.argv[2]) else 1)
PY
