#!/usr/bin/env bash
# Soak test for Oino Desktop (docs/dev/v1/stability.md).
#
#   apps/oino-desktop/scripts/soak.sh [--minutes N] [--backend demo|faux] [--bin PATH]
#                                     [--display :N] [--max-panes N] [--out DIR]
#
# Starts ONE app instance with `--soak` (in-app driver: sessions, long streams, side tabs,
# terminals, theme switches, palette, settings, board, splits, reopen) on its own Xvfb
# (default) or an existing display, samples RSS / threads / fds every 5 s from /proc, and fails
# when the app crashes, stalls, exceeds 4 GB RSS (aborted at once), or grows beyond:
#   RSS slope over the second half      > SOAK_MAX_SLOPE_MB_MIN (default 8 MB/min)
#   threads (end - first-quarter max)   > SOAK_MAX_THREAD_GROWTH (default 16)
#   fds     (end - first-quarter max)   > SOAK_MAX_FD_GROWTH (default 64)
#   run max frame time                  > SOAK_MAX_FRAME_MS (default 0 = report only; Xvfb
#                                         renders in software, so frame times are only judged on
#                                         a real display)
# Backends: `demo` (scripted, no providers) or `faux` (in-process oino-core with every bundled
# extension and the provider-faux fixture: real sessions, WASM, tools; no network cost).
# Only PIDs started here are ever killed.
set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
apps="$(cd "$here/../.." && pwd)"
root="$(cd "$apps/.." && pwd)"

minutes=30
backend=demo
bin="$apps/target/release/oino-desktop"
display=""
max_panes=""
out=""
while [[ $# -gt 0 ]]; do
  case "$1" in
    --minutes) minutes="$2"; shift 2 ;;
    --backend) backend="$2"; shift 2 ;;
    --bin) bin="$2"; shift 2 ;;
    --display) display="$2"; shift 2 ;;
    --max-panes) max_panes="$2"; shift 2 ;;
    --out) out="$2"; shift 2 ;;
    -h|--help) sed -n '2,22p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

max_slope="${SOAK_MAX_SLOPE_MB_MIN:-8}"
max_threads="${SOAK_MAX_THREAD_GROWTH:-16}"
max_fds="${SOAK_MAX_FD_GROWTH:-64}"
max_frame="${SOAK_MAX_FRAME_MS:-0}"
rss_cap_kb=$((4 * 1024 * 1024))
secs=$((minutes * 60))

[[ -x "$bin" ]] || { echo "no binary at $bin (cargo build --release -p oino-desktop)" >&2; exit 2; }
out="${out:-$(mktemp -d "${TMPDIR:-/tmp}/oino-soak.XXXXXX")}"
mkdir -p "$out"
agent="$out/agent"
proj="$out/project"
mkdir -p "$agent/logs" "$proj"
printf 'soak project\n' > "$proj/README.md"
echo '{"onboarded":true}' > "$agent/desktop.json"

args=(--soak "$secs" --cwd "$proj")
case "$backend" in
  demo) args+=(--demo) ;;
  faux)
    fixture="$root/extensions/dist/oino.fixture.provider-faux"
    [[ -f "$fixture/component.wasm" ]] || { echo "missing $fixture (cargo xtask wasm)" >&2; exit 2; }
    mkdir -p "$agent/extensions"
    cp -r "$fixture" "$agent/extensions/provider-faux"
    echo '{"defaultProvider":"faux","defaultModel":"faux-1"}' > "$agent/settings.json"
    echo '{"faux":{"type":"api_key","key":"k"}}' > "$agent/auth.json"
    ;;
  *) echo "unknown backend: $backend" >&2; exit 2 ;;
esac

pids_to_kill=()
cleanup() {
  for p in ${pids_to_kill[@]+"${pids_to_kill[@]}"}; do
    kill "$p" 2>/dev/null || true
  done
}
trap cleanup EXIT

if [[ -z "$display" ]]; then
  for n in $(seq 120 160); do
    [[ -e "/tmp/.X11-unix/X$n" || -e "/tmp/.X$n-lock" ]] && continue
    display=":$n"; break
  done
  Xvfb "$display" -screen 0 1600x1000x24 -nolisten tcp >"$out/xvfb.log" 2>&1 &
  pids_to_kill+=("$!")
  xvfb_started=1
  sleep 1
fi

env_vars=(
  DISPLAY="$display"
  OINO_AGENT_DIR="$agent"
  OINO_SESSION_DIR="$agent/sessions"
  RUST_BACKTRACE=1
  RUST_LOG="${RUST_LOG:-info}"
  OINO_DESKTOP_DEMO_APPROVE_MS=300
  OINO_DESKTOP_DEMO_STREAM_REPEAT="${OINO_DESKTOP_DEMO_STREAM_REPEAT:-30}"
)
[[ -n "$max_panes" ]] && env_vars+=(OINO_DESKTOP_MAX_PANES="$max_panes")
# On our own Xvfb, render with lavapipe (software Vulkan): keeps the GPU driver out of long
# unattended runs.
if [[ -n "${xvfb_started:-}" && -f /usr/share/vulkan/icd.d/lvp_icd.json ]]; then
  env_vars+=(VK_ICD_FILENAMES=/usr/share/vulkan/icd.d/lvp_icd.json)
fi
env "${env_vars[@]}" "$bin" "${args[@]}" >"$out/app.out" 2>&1 &
app=$!
pids_to_kill+=("$app")
echo "soak: pid $app, backend $backend, $minutes min, display $display, out $out"

samples="$out/samples.tsv"
printf 't_s\trss_kb\tthreads\tfds\n' > "$samples"
start=$(date +%s)
aborted=""
while kill -0 "$app" 2>/dev/null; do
  now=$(( $(date +%s) - start ))
  rss=$(awk '/^VmRSS/ {print $2}' "/proc/$app/status" 2>/dev/null || echo 0)
  thr=$(awk '/^Threads/ {print $2}' "/proc/$app/status" 2>/dev/null || echo 0)
  fds=$(find "/proc/$app/fd" -mindepth 1 -maxdepth 1 2>/dev/null | wc -l || true)
  [[ -n "$rss" ]] && printf '%s\t%s\t%s\t%s\n' "$now" "$rss" "$thr" "$fds" >> "$samples"
  if [[ "${rss:-0}" -gt "$rss_cap_kb" ]]; then
    aborted="RSS ${rss} kB over the 4 GB cap"
    kill "$app" 2>/dev/null || true
    break
  fi
  if [[ "$now" -gt $((secs + 120)) ]]; then
    aborted="did not quit after the soak"
    kill "$app" 2>/dev/null || true
    break
  fi
  sleep 5
done
status=0
wait "$app" || status=$?

log="$agent/logs/desktop.log"
crashes=$(find "$agent/logs" -maxdepth 1 -name 'desktop-crash-*.log' | wc -l)
stalls=$(find "$agent/logs" -maxdepth 1 -name 'desktop-stall-*.log' | wc -l)
last_soak=$( (grep 'oino_soak' "$log" 2>/dev/null || true) | (grep ' soak elapsed' || true) | tail -1)
run_max=$(sed -n 's/.*run_max_ms=\([0-9.]*\).*/\1/p' <<<"$last_soak")
run_p99=$(sed -n 's/.*run_p99_ms=\([0-9.]*\).*/\1/p' <<<"$last_soak")
steps=$(sed -n 's/.*steps=\([0-9]*\).*/\1/p' <<<"$last_soak")

read -r slope rss_start rss_end rss_max thr_q thr_end fds_q fds_end n < <(awk -F'\t' '
  NR > 1 { t[++n] = $1; r[n] = $2; th[n] = $3; f[n] = $4 }
  END {
    if (n < 4) { print "0 0 0 0 0 0 0 0 " n; exit }
    h = int(n / 2) + 1; m = 0; sx = sy = sxx = sxy = 0
    for (i = h; i <= n; i++) { x = t[i] / 60; y = r[i] / 1024; m++; sx += x; sy += y; sxx += x * x; sxy += x * y }
    d = m * sxx - sx * sx; s = d == 0 ? 0 : (m * sxy - sx * sy) / d
    q = int(n / 4); if (q < 1) q = 1; tq = fq = 0
    for (i = 1; i <= q; i++) { if (th[i] > tq) tq = th[i]; if (f[i] > fq) fq = f[i] }
    mx = 0; for (i = 1; i <= n; i++) if (r[i] > mx) mx = r[i]
    printf "%.2f %d %d %d %d %d %d %d %d\n", s, r[1] / 1024, r[n] / 1024, mx / 1024, tq, th[n], fq, f[n], n
  }' "$samples")

fail=()
[[ -n "$aborted" ]] && fail+=("$aborted")
[[ "$status" -ne 0 ]] && fail+=("exit status $status")
[[ "$crashes" -gt 0 ]] && fail+=("$crashes crash log(s)")
[[ "$stalls" -gt 0 ]] && fail+=("$stalls UI stall(s)")
[[ -z "$last_soak" ]] && fail+=("no soak report in $log")
awk -v s="$slope" -v m="$max_slope" 'BEGIN { exit !(s > m) }' && fail+=("RSS slope $slope MB/min > $max_slope")
[[ $((thr_end - thr_q)) -gt "$max_threads" ]] && fail+=("threads grew $thr_q -> $thr_end")
[[ $((fds_end - fds_q)) -gt "$max_fds" ]] && fail+=("fds grew $fds_q -> $fds_end")
if [[ "$max_frame" != 0 && -n "$run_max" ]]; then
  awk -v a="$run_max" -v b="$max_frame" 'BEGIN { exit !(a > b) }' && fail+=("max frame ${run_max} ms > $max_frame")
fi

summary="$out/summary.txt"
{
  echo "backend=$backend minutes=$minutes samples=$n steps=${steps:-?}"
  echo "rss_mb start=$rss_start end=$rss_end max=$rss_max slope_2nd_half_mb_per_min=$slope"
  echo "threads first_quarter_max=$thr_q end=$thr_end"
  echo "fds first_quarter_max=$fds_q end=$fds_end"
  echo "frame_ms run_max=${run_max:-?} run_p99=${run_p99:-?}"
  echo "crash_logs=$crashes stall_logs=$stalls exit=$status"
  if [[ ${#fail[@]} -eq 0 ]]; then echo "RESULT: PASS"; else printf 'RESULT: FAIL: %s\n' "${fail[@]}"; fi
} | tee "$summary"
[[ ${#fail[@]} -eq 0 ]]
