#!/usr/bin/env bash
# Keep Oino's cargo build caches inside a disk budget (app-convention: every build cache
# needs a size limit or cleanup policy).
#
# Usage: scripts/target-gc.sh [--budget-gb N] [--age-hours H] [--dry-run] [--force]
#   Budget defaults to $OINO_TARGET_BUDGET_GB (30). Covers target/, extensions/target/,
#   apps/unipi/target/, apps/unipi/src-tauri/target/, and standalone harness targets.
#   Steps, applied in order until total is under budget:
#     1. remove harness/target dirs, incremental caches, examples, and tmp scratch
#     2. remove linked test executables in */deps/* older than AGE (keeps .rlib/.rmeta warm)
#     3. remove all remaining linked test executables in */deps/* (except .so proc-macros)
#     4. remove stale deps/ and .fingerprint/ entries older than AGE*3
#     5. `cargo clean --profile dev` per workspace (last resort)
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
budget_gb="${OINO_TARGET_BUDGET_GB:-30}"
age_h="${OINO_TARGET_GC_AGE_HOURS:-1}"
dry=0
force=0
while [ $# -gt 0 ]; do
  case "$1" in
    --budget-gb) budget_gb="$2"; shift 2 ;;
    --age-hours) age_h="$2"; shift 2 ;;
    --dry-run) dry=1; shift ;;
    --force) force=1; shift ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

dirs=("$root/target" "$root/extensions/target" "$root/apps/unipi/target" "$root/apps/unipi/src-tauri/target")

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
stale_min=$((age_min * 3))

run "rm -rf '$root'/extensions/*/*/harness/target"
for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  [ -d "$d/tmp" ] && run "rm -rf '$d/tmp'"
  [ -d "$d/debug/examples" ] && run "rm -rf '$d/debug/examples'"
  while IFS= read -r inc; do
    [ -n "$inc" ] || continue
    run "rm -rf '$inc'"
  done <<<"$(find "$d" -type d -name incremental -prune 2>/dev/null)"
done
[ "$(total_gb)" -le "$budget_gb" ] && { echo "target-gc: $(total_gb) GB after incremental/tmp cleanup"; exit 0; }

for d in "${dirs[@]}"; do
  [ -d "$d/debug/deps" ] || continue
  run "find '$d/debug/deps' -maxdepth 1 -type f -perm /111 ! -name '*.so' -mmin +$age_min -delete"
done
[ "$(total_gb)" -le "$budget_gb" ] && { echo "target-gc: $(total_gb) GB after aged test-bin cleanup"; exit 0; }

for d in "${dirs[@]}"; do
  [ -d "$d/debug/deps" ] || continue
  run "find '$d/debug/deps' -maxdepth 1 -type f -perm /111 ! -name '*.so' -delete"
done
[ "$(total_gb)" -le "$budget_gb" ] && { echo "target-gc: $(total_gb) GB after all test-bin cleanup"; exit 0; }

for d in "${dirs[@]}"; do
  [ -d "$d" ] || continue
  run "find '$d' -path '*/deps/*' -type f -mmin +$stale_min -delete"
  run "find '$d' -path '*/.fingerprint/*' -mindepth 1 -maxdepth 3 -type d -mmin +$stale_min -exec rm -rf {} +"
done
[ "$(total_gb)" -le "$budget_gb" ] && { echo "target-gc: $(total_gb) GB after stale artifact cleanup"; exit 0; }

for ws in "$root" "$root/extensions" "$root/apps/unipi/src-tauri"; do
  [ -f "$ws/Cargo.toml" ] && run "(cd '$ws' && cargo clean --profile dev)"
done
echo "target-gc: $(total_gb) GB after cargo clean --profile dev"
