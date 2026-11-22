# fix-bench handoff — 2026-11-14

## Outcome

Harness fixes implemented; baseline re-recording remains blocked. No acceptance
claim and no fabricated `baseline-v2` record. All changes are in perf/fix-bench.

## Findings

- QA-01: faux tool responses now use a unique atomic per-request counter in bash
  arguments. L1 checks each of thirty loops for completed rounds >= max(1, secs/10),
  second-half requests, open stream, and no settled event. The ten streaming
  chats must also remain open/unsettled. Failures are persisted as failed scenarios.
- Every v3 host metric exports full-precision raw samples; pooled metrics recompute
  quantiles from observations. Percentile gates with n<50 are INSUFFICIENT. Scalar
  gates retain worst observation. Legacy summary-only comparisons are ignored.
- `--runs N` repeats the non-soak suite, pools samples. `--wait-quiet` waits up to
  ten minutes; records include loadavg/CPU count at both ends. Noisy records fail gate.
- T9 wire-frame rate is now explicitly proxy/ungated. T4 validates success ack
  frames on the wire. T13 counts exact custom interrupted markers/crashed events;
  host restart/reopen speed is no longer presented as the T13 recovery budget.
- README inventories every T-budget's proxy/coverage gaps. Full UI/worker/soak
  certification is intentionally not claimed.

## Evidence

- `cargo test -p oino-bench`: 11 passed, zero failed (0.34 s test execution).
  Original QA suite: six tests. Added HTTP test verifies eight consecutive distinct
  tool argument payloads (beyond the four-result guard failure seen in QA), plus
  100 unique generated arguments. No host process needed for this test.
- Raw pooling test: 49 x 1 ms -> INSUFFICIENT; add one 100 ms sample -> n=50,
  p95=1 ms, max=100 ms; JSON retains all fifty observations. Scalar 0+1 failure
  pools to gated value 1. Legacy baseline rejection and quiet threshold tested.
- `cargo check -p xtask`: passed. `git diff --check`: passed.
- Logs in worktree: bench-verification.log, xtask-verification.log,
  target-base-build.log (ignored build logs, not committed).
- Base worktree created at `/tmp/bench-base`, detached at 0999864b. Base host
  release build initiated with CARGO_TARGET_DIR pointing at this fix worktree;
  compiler output confirms source paths under /tmp/bench-base.

## Risks / open work

1. No live L1/all run and no baseline-v2 JSON/markdown recorded. User permits
   isolated host launches ONLY through ~/.cache/perf-fixture/run-safe.sh. Existing
   benchmark Host::spawn starts custom scenario hosts directly. The safe runner
   only starts/probes a copied real-chat fixture and accepts no custom scenario
   settings, ports, or faux scripts; using it alone cannot execute this suite.
   README documents the mismatch. No prohibited launch or owner signal was made.
2. Base release build FAILED: background task td9fc740a006f timed out after
   60 seconds. Its requested ZVEC_LIB_DIR glob also matched no directory, so
   that environment variable was empty. No base host binary is verified. The
   benchmark release build also timed out after 60 seconds (task td9ffb39c007d),
   during dependency compilation. Tests and xtask check passed before that timeout;
   the later final test run passed all eleven tests.
3. Load moved from 5.50 to 25.81 on 32 CPUs during work (>16 threshold). No
   quiet-machine timing comparison can be claimed. --wait-quiet must be used for
   the eventual baseline run after release compilation finishes.
4. L1 remains subagent-like ordinary chats, not actual child agents. T12 is still
   an explicitly early sample; T1/T2/T3/T4/T6 paint, T9 webview, T10 hidden app,
   worker isolation and T14 are not measured by this host harness.

## Recommended next step

Provide a safe-runner-compatible custom-host launch interface or explicitly
approve the harness's existing isolated Host launcher. Then run the fixed harness
against the 0999864b release host with all --runs 5 --wait-quiet, identifying the
base host revision separately from the harness revision, and commit genuine
bench/records/baseline-v2-2026-11-14.{json,md}. Run integrated all/L1 afterward.
