#!/usr/bin/env bash
# Round-4 integration gate run (alpha.4). Logs to target/gate-round4.log
set -u
cd "$(dirname "$0")/../.."   # repo root
ROOT=$PWD
export TMPDIR=$ROOT/target/tmp
mkdir -p "$TMPDIR"
LOG=$ROOT/target/gate-round4.log
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
# `cargo doc` no longer forwards trailing args (cargo 1.99): flags go through RUSTDOCFLAGS.
RUSTDOCFLAGS="-D warnings" run nice -n 10 cargo doc --workspace --no-deps
R4=$?
run nice -n 10 cargo xtask wit
R5=$?
run bash scripts/docs-check.sh
R6=$?
run bash scripts/install-smoke.sh
R7=$?

echo "=== EXTENSIONS workspace ===" | tee -a "$LOG"
cd "$ROOT/extensions"
run nice -n 10 cargo fmt --all --check
E1=$?
run nice -n 10 cargo clippy --workspace --all-targets --target wasm32-wasip2 -j 4 -- -D warnings -W clippy::pedantic
E2=$?
run nice -n 10 cargo test --workspace -j 4
E3=$?
run bash -c 'for h in unipi/*/harness/run.sh; do echo "--- $h"; nice -n 10 bash "$h" || exit 1; done'
E4=$?
cd "$ROOT"
run nice -n 10 cargo xtask checksums
E5=$?

echo "=== APPS workspace ===" | tee -a "$LOG"
cd "$ROOT/apps"
run nice -n 10 cargo fmt --all --check
A1=$?
run nice -n 10 cargo clippy --workspace --all-targets -j 4 -- -D warnings -W clippy::pedantic
A2=$?
run nice -n 10 cargo test --workspace -j 4
A3=$?
RUSTDOCFLAGS="-D warnings" run nice -n 10 cargo doc --workspace --no-deps
A4=$?

echo "=== summary root: fmt=$R1 clippy=$R2 test=$R3 doc=$R4 wit=$R5 docs=$R6 install=$R7 ; extensions: fmt=$E1 clippy=$E2 test=$E3 harness=$E4 checksums=$E5 ; apps: fmt=$A1 clippy=$A2 test=$A3 doc=$A4" | tee -a "$LOG"
