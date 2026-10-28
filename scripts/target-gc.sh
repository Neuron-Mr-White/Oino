#!/usr/bin/env bash
# Keep Oino's cargo build caches inside a disk budget (app-convention: every build cache
# needs a size limit or cleanup policy).
#
# Usage: scripts/target-gc.sh [--budget-gb N] [--age-hours H] [--dry-run]
#   Budget defaults to $OINO_TARGET_BUDGET_GB (80). Covers target/, apps/target/ and
#   extensions/target/. Steps, applied in order until the total is under budget:
#     1. remove target*/tmp scratch entries older than AGE (default 6 h)
#     2. remove incremental caches older than AGE
#     3. remove deps/ and .fingerprint/ entries not touched for AGE*4 (stale hashed
#        artifacts; cargo rebuilds anything it still needs)
#     4. `cargo clean --profile dev` per workspace (last resort)
# Safe to run while nothing is compiling; refuses to run while a cargo process is active
# unless --force is given.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
budget_gb="${OINO_TARGET_BUDGET_GB:-80}"
age_h="${OINO_TARGET_GC_AGE_HOURS:-6}"
dry=0
force=0
while [ $# -gt 0 ]; do
  case "$1" in
    --budget-gb) budget_gb="$2"; shift 2 ;;
    --age-hours) age_h="$2"; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --force) force=1; shift ;;
    -h|--help) sed -n '2,15p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

dirs=("$root/target" "$root/apps/target" "$root/extensions/target")

total_gb() {
  local sum=0 d
  for d in "${dirs[@]}"; do
    [ -d "$d" ] || continue
    sum=$((sum + $(du -s --block-size=1G "$d" | cut -f1)))
  done
  echo "$sum"
}

if [ "$force" -eq 0 ] && pgrep -x cargo >/dev/null 2>&1; then
  echo "target-gc: a cargo process is running; retry later or pass --force" >&2
  exit 3
fi

run() {
  if [ "$dry" -eq 1 ]; then echo "would: $*"; else eval "$@"; fi
}

before="$(total_gb)"
echo "target-gc: ${before} GB used, budget ${budget_gb} GB"
[ "$before" -le "$budget_gb" ] && { echo "target-gc: within budget"; exit 0; }

age_min=$((age_h * 60))
stale_min=$((age_min * 4))

for d in "${dirs[@]}"; do
  [ -d "$d/tmp" ] && run "find '$d/tmp' -mindepth 1 -maxdepth 1 -mmin +$age_min -exec rm -rf {} +"
done
[ "$(total_gb)" -le "$budget_gb" ] && { echo "target-gc: $(total_gb) GB after tmp cleanup"; exit 0; }

for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  run "find '$d' -type d -name incremental -prune -exec find {} -mindepth 1 -maxdepth 1 -mmin +$age_min -exec rm -rf '{}' + \\;"
done
[ "$(total_gb)" -le "$budget_gb" ] && { echo "target-gc: $(total_gb) GB after incremental cleanup"; exit 0; }

for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  run "find '$d' -path '*/deps/*' -type f -mmin +$stale_min -delete"
  run "find '$d' -path '*/.fingerprint/*' -mindepth 1 -maxdepth 3 -type d -mmin +$stale_min -exec rm -rf {} +"
done
[ "$(total_gb)" -le "$budget_gb" ] && { echo "target-gc: $(total_gb) GB after stale artifact cleanup"; exit 0; }

for ws in "$root" "$root/apps" "$root/extensions"; do
  run "(cd '$ws' && cargo clean --profile dev)"
done
echo "target-gc: $(total_gb) GB after cargo clean --profile dev"
