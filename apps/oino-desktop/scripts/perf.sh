#!/usr/bin/env bash
# Frame-time check (design.md §2.11): the demo backend with N streaming panes (default 4),
# frame times logged every 120 frames (OINO_FRAME_LOG=1).
#
#   apps/oino-desktop/scripts/perf.sh [binary] [seconds] [panes]
#
# Gate: the WHOLE-RUN p99 (exact histogram over every frame, `run_p99_ms` of the last log
# line) must be ≤ BUDGET_MS (default 10). Also printed: worst 120-frame window p99, frames
# over 8.3 ms (120 Hz), presented fps, and the latency marks (first frame, session switch,
# palette open, keystroke → paint) plus idle CPU (IDLE_SECS, default 5, after IDLE_SETTLE,
# default 65 s, on a second instance with the idle welcome scene).
#
# GPU: with DISPLAY set (and PERF_HEADLESS unset) the app runs on the real display / GPU, e.g.
#   DISPLAY=:0 XAUTHORITY=/run/user/1000/.mutter-Xwaylandauth.* scripts/perf.sh
# Headless (PERF_HEADLESS=1 or no DISPLAY): Xvfb + lavapipe software Vulkan, an upper bound
# capped at Xvfb's 60 Hz.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
apps="$(cd "$here/../.." && pwd)"
bin="${1:-$apps/target/release/oino-desktop}"
secs="${2:-20}"
panes="${3:-4}"
budget="${BUDGET_MS:-10}"
idle_secs="${IDLE_SECS:-5}"
agent="$(mktemp -d "${TMPDIR:-$HOME/.cache}/oino-perf.XXXX")"
log="$agent/logs/desktop.log"
xvfb=""
mode="gpu (${DISPLAY:-})"
if [ -z "${DISPLAY:-}" ] || [ "${PERF_HEADLESS:-0}" = 1 ]; then
  export DISPLAY=":$((80 + RANDOM % 9))"
  Xvfb "$DISPLAY" -screen 0 1400x900x24 -nolisten tcp >/dev/null 2>&1 &
  xvfb=$!
  export VK_ICD_FILENAMES="${VK_ICD_FILENAMES:-/usr/share/vulkan/icd.d/lvp_icd.json}"
  mode="headless lavapipe ($DISPLAY)"
  sleep 1
fi
app=""
trap 'kill $app ${xvfb} 2>/dev/null || true' EXIT
env -u WAYLAND_DISPLAY OINO_AGENT_DIR="$agent" OINO_FRAME_LOG=1 RUST_LOG=info,oino_frame=info \
  OINO_DESKTOP_DEMO_SCENE=stream OINO_DESKTOP_PANES="$panes" OINO_DESKTOP_THEME=dark \
  OINO_DESKTOP_OPEN="${PERF_OPEN:-type}" \
  "$bin" --demo --cwd "$HOME" >/dev/null 2>&1 &
app=$!
sleep "$secs"
kill "$app" 2>/dev/null || true
wait "$app" 2>/dev/null || true
grep 'frame times' "$log" | tail -n +3 > "$agent/frames.txt" || true
grep 'latency' "$log" > "$agent/latency.txt" || true
if [ ! -s "$agent/frames.txt" ]; then
  echo "no frame samples in $log" >&2
  exit 2
fi

# Idle CPU: a fresh instance on the welcome scene, sampled after it settles.
idle_agent="$(mktemp -d "${TMPDIR:-$HOME/.cache}/oino-idle.XXXX")"
env -u WAYLAND_DISPLAY OINO_AGENT_DIR="$idle_agent" OINO_DESKTOP_DEMO_SCENE=welcome \
  OINO_DESKTOP_THEME=dark "$bin" --demo --cwd "$HOME" >/dev/null 2>&1 &
app=$!
# The demo splash header drains a meter for 60 s; idle starts after it.
sleep "${IDLE_SETTLE:-65}"
tick() { awk '{print $14 + $15}' "$1"; }
t0=$(tick "/proc/$app/stat"); m0=$(tick "/proc/$app/task/$app/stat")
sleep "$idle_secs"
t1=$(tick "/proc/$app/stat"); m1=$(tick "/proc/$app/task/$app/stat")
hz=$(getconf CLK_TCK)
kill "$app" 2>/dev/null || true
wait "$app" 2>/dev/null || true
idle_cpu=$(python3 -c "print(f'{($m1-$m0)/$hz/$idle_secs*100:.2f} % UI thread, {($t1-$t0)/$hz/$idle_secs*100:.2f} % process (incl. renderer threads)')")

python3 - "$agent/frames.txt" "$agent/latency.txt" "$budget" "$secs" "$mode" "$idle_cpu" <<'PY'
import re, sys, statistics
rows = [dict(re.findall(r'(\w+)=([\d.]+)', l)) for l in open(sys.argv[1])]
p50 = [float(r['p50_ms']) for r in rows]; p99 = [float(r['p99_ms']) for r in rows]
mx = [float(r['max_ms']) for r in rows]
last = rows[-1]
frames = int(float(last['frames'])); dropped = int(float(last['dropped']))
run_p99 = float(last.get('run_p99_ms', max(p99)))
over8 = int(float(last.get('over_8ms', 0)))
secs = float(sys.argv[4])
print(f"mode: {sys.argv[5]}")
print(f"windows={len(rows)} frames={frames} (~{frames/secs:.0f} fps rendered) dropped(>16.7ms)={dropped} over_8.3ms={over8}")
print(f"p50 median={statistics.median(p50):.2f}ms  window p99 median={statistics.median(p99):.2f}ms  worst window p99={max(p99):.2f}ms  max={max(mx):.2f}ms")
print(f"RUN p99 (all frames)={run_p99:.2f}ms  budget={sys.argv[3]}ms")
lat = {}
for l in open(sys.argv[2]):
    m = re.search(r'label="?([\w ]+?)"? ms=([\d.]+)', l)
    if m:
        lat.setdefault(m.group(1), []).append(float(m.group(2)))
for k, v in sorted(lat.items()):
    print(f"latency {k}: n={len(v)} median={statistics.median(v):.1f}ms max={max(v):.1f}ms")
print(f"idle CPU (welcome, after settle): {sys.argv[6]}")
sys.exit(0 if run_p99 <= float(sys.argv[3]) else 1)
PY
