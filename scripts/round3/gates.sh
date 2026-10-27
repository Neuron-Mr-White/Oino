#!/usr/bin/env bash
# Round-3 integration gate run. Logs to target/gate-round3.log
set -u
cd "$(dirname "$0")/../.."   # repo root
ROOT=$PWD
export TMPDIR=$ROOT/target/tmp
mkdir -p "$TMPDIR"
LOG=$ROOT/target/gate-round3.log
: > "$LOG"

run() {
  echo "== [$(date -u +%H:%M:%S)] $*" | tee -a "$LOG"
  "$@" >> "$LOG" 2>&1
  local rc=$?
  echo "== [$(date -u +%H:%M:%S)] exit $rc: $*" | tee -a "$LOG"
  return $rc
}

echo "=== ROOT workspace ===" | tee -a "$LOG"
run nice -n 10 cargo fmt --all --check
R1=$?
run nice -n 10 cargo clippy --workspace --all-targets -j 4 -- -D warnings -W clippy::pedantic
R2=$?
run nice -n 10 cargo test --workspace -j 4
R3=$?
run nice -n 10 cargo doc --workspace --no-deps -- -D warnings
R4=$?

echo "=== EXTENSIONS workspace ===" | tee -a "$LOG"
cd "$ROOT/extensions"
run nice -n 10 cargo fmt --all --check
E1=$?
run nice -n 10 cargo clippy --workspace --all-targets --target wasm32-wasip2 -j 4 -- -D warnings -W clippy::pedantic
E2=$?
run nice -n 10 cargo test --workspace -j 4
E3=$?

echo "=== APPS workspace ===" | tee -a "$LOG"
cd "$ROOT/apps"
run nice -n 10 cargo fmt --all --check
A1=$?
run nice -n 10 cargo clippy --workspace --all-targets -j 4 -- -D warnings -W clippy::pedantic
A2=$?
run nice -n 10 cargo test --workspace -j 4
A3=$?

echo "=== summary root: fmt=$R1 clippy=$R2 test=$R3 doc=$R4 ; extensions: fmt=$E1 clippy=$E2 test=$E3 ; apps: fmt=$A1 clippy=$A2 test=$A3" | tee -a "$LOG"
