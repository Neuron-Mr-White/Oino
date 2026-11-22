#!/usr/bin/env bash
# Perf bench entry point (`mise run perf -- [options]`). Runs ON A BENCH VM, never on a
# workstation that runs the owner's Unipi (it refuses when one is running).
#
#   scripts/perf/run.sh --deb unipi.deb [--out DIR] [--scenarios a,b,c,d] [--sizes 10,50,200]
#                       [--soak-hours 8] [--owner-file chat.jsonl] [--profile perf|heaptrack]
#                       [--host-bin unipi-oino-host]   # e.g. a glibc build for heaptrack
#
# From a workstation use scripts/perf/remote.sh (creates an Agent37 VM, builds HEAD there,
# runs this, fetches the results, deletes the VM).
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
DEB="" OUT="" SC="a,b,d" HOSTBIN="" PASS=()
while [ $# -gt 0 ]; do
  case "$1" in
    --deb) DEB=$2; shift 2 ;;
    --out) OUT=$2; shift 2 ;;
    --scenarios) SC=$2; shift 2 ;;
    --host-bin) HOSTBIN=$2; shift 2 ;;
    *) PASS+=("$1"); shift ;;
  esac
done
if pgrep -x unipi-oino >/dev/null && [ "${PERF_FORCE:-0}" != 1 ]; then
  echo "perf: a Unipi app is running on this machine; benches run on a bench VM (scripts/perf/remote.sh)." >&2
  exit 75
fi
[ -n "$DEB" ] || { echo "perf: --deb <unipi .deb> is required" >&2; exit 64; }
OUT=${OUT:-$PWD/perf-out/$(date +%Y%m%d-%H%M%S)}
mkdir -p "$OUT"
TREE="$OUT/deb"
[ -x "$TREE/usr/bin/unipi-oino-host" ] || dpkg-deb -x "$DEB" "$TREE"
HOSTBIN=${HOSTBIN:-$TREE/usr/bin/unipi-oino-host}
# a non-deb host binary still needs the bundled extensions next to it (../share/oino/extensions)
if [ "$HOSTBIN" != "$TREE/usr/bin/unipi-oino-host" ]; then
  d="$OUT/hostbin"; mkdir -p "$d/bin" "$d/share"
  cp -f "$HOSTBIN" "$d/bin/unipi-oino-host"; cp -f "$TREE/usr/bin/oino" "$d/bin/"
  ln -sfn "$TREE/usr/share/oino" "$d/share/oino"; HOSTBIN="$d/bin/unipi-oino-host"
fi
echo "perf: host $HOSTBIN -> $OUT"
core=$(echo "$SC" | tr ',' '\n' | grep -E '^[abc]$' | paste -sd, || true)
[ -z "$core" ] || python3 "$HERE/bench.py" --host "$HOSTBIN" --out "$OUT" --scenarios "$core" "${PASS[@]}"
if echo "$SC" | grep -q d; then
  python3 "$HERE/app.py" --deb "$DEB" --out "$OUT"
fi
# profiles: perf.data -> flamegraph SVG + top self-time symbols; heaptrack -> text summary
FG=${FLAMEGRAPH_DIR:-/root/FlameGraph}
for d in "$OUT"/perf-*.data; do
  [ -s "$d" ] || continue
  perf report -i "$d" --stdio --no-children --sort symbol 2>/dev/null | grep -v '^#' | grep -v '^$' | head -60 > "${d%.data}.top.txt" || true
  [ -x "$FG/flamegraph.pl" ] && perf script -i "$d" 2>/dev/null | "$FG/stackcollapse-perf.pl" | "$FG/flamegraph.pl" > "${d%.data}.svg" || true
done
for h in "$OUT"/heaptrack-*.zst "$OUT"/heaptrack-*.gz; do
  [ -s "$h" ] && heaptrack_print -f "$h" --print-peaks 1 --print-leaks 1 --print-allocators 1 -n 25 > "${h%%.*}.txt" 2>/dev/null || true
done
python3 "$HERE/report.py" "$OUT" > "$OUT/summary.md" && echo "perf: $OUT/summary.md"
