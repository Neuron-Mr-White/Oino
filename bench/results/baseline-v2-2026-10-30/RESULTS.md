# Baseline v2 — subagents benchmark (WP-C11 part 1)

Recorded **before** WP-C4 (shared engine), WP-C7 (lazy hub) and WP-C8 (subagent hibernation)
land, per [core.md §4][spec]. Re-run after each of those WPs with the same harness
(`cargo xtask bench subagents --n {1,10,50}` / `mise run bench -- subagents --n 50`) and
compare against this record.

[spec]: ../../../docs/dev/v2/specs/core.md

- HEAD: `5b15e4e3…` (v2 branch). Committed state includes WP-C1 (fsync barriers) and
  WP-C2 (dispatch record + repair); no C4/C7/C8 commits exist yet.
- When: 2026-10-30 10:54–10:55 (+08:00)
- Machine: `oi` · linux x86_64 · kernel 7.0.0-38-generic · 32 cpus
- Work dir fs: **ext4** (`/dev/nvme0n1p1`) — a tmpfs work dir would zero the fsync barrier,
  so the harness places its temp agent dir under `target/bench` by default
- Scenario: faux provider (in-process double, no unipi set) · children via
  `AgentSession::spawn_agent` with `Persistence::File`, one seeded user message,
  model `faux/faux-1`, `extensions: None` · one parallel turn per child ·
  `extensionSettings.subagents.maxConcurrent = 16` (the `HARD_MAX_CONCURRENT` cap), so
  n > 16 fans out over `ceil(n / 16)` root sessions with all children alive at once
- Provenance: shared-tree caveat — the release binary compiles the working tree; at run
  time other workers held *uncommitted* C5/C8 edits (`spawn_isolated` wrapper, hibernation
  scaffolding). Neither can affect this scenario (no extensions, no wasmtime in the dep
  graph, hibernation needs ≥120 s idle vs a ~1 s run). Repeat run agreement: per-child
  total at n = 50 was 420.6 and 423.1 KiB across two consecutive runs (~0.6 %).

| Metric | n = 1 | n = 10 | n = 50 |
|---|---:|---:|---:|
| Root sessions | 1 | 1 | 4 |
| Children spawned / file-persisted / answered | 1 / 1 / 1 | 10 / 10 / 10 | 50 / 50 / 50 |
| RSS at start (KiB) | 5 972 | 5 900 | 5 940 |
| RSS after services (KiB) | 9 216 | 9 496 | 9 292 |
| RSS after roots (KiB) | 11 008 | 11 100 | 10 992 |
| RSS after spawn (KiB) | 11 880 | 12 716 | 15 320 |
| RSS after turns (KiB) | 12 820 | 17 356 | 32 020 |
| RSS high-water (KiB) | 12 820 | 17 356 | 32 020 |
| **ΔRSS per child — spawn (KiB)** | 872.0 | 161.6 | 86.6 |
| **ΔRSS per child — turn (KiB)** | 940.0 | 464.0 | 334.0 |
| **ΔRSS per child — total (KiB)** | 1 812.0 | 625.6 | 420.6 |
| CPU during spawn (ms) | 0 | 10 | 0 |
| CPU during turns (ms) | 0 | 10 | 70 |
| Barrier: `SessionManager::sync()` over 100 appends (µs mean) | 1 082.56 | 1 107.07 | 1 091.69 |
| Wall: spawn / turns (ms) | 0 / 2 | 2 / 3 | 5 / 32 |

Files: `subagents-1-20261030-105454.json`, `subagents-10-20261030-105454.json`,
`subagents-50-20261030-105455.json` (+ `.md` each).

## Notes for the after-WP comparison

- **Per-child steady-state RSS** is `ΔRSS per child — total` (420.6 KiB at n = 50). The C8
  target (≥30 % drop with hibernation on) reads against this row.
- **CPU** is tick-granular (100 Hz) and the faux provider makes turns nearly free; the raw
  `duringTurnsTicks` fields in the JSONs are the more sensitive view.
- **Barrier** is now measured exactly as spec'd — mean µs of `SessionManager::sync()` after
  each of 100 real session appends (WP-C1 writer, ≈ 1.05–1.11 ms/fsync on this nvme/ext4).
  One fsync ≈ 1 ms is the per-request/per-tool cost context for C1's group-commit risk
  (core.md §6.1).
- n = 1's per-child rows carry one-off noise (a single child carries the whole delta);
  n = 50 is the meaningful row.
- Layout: `bench/results/baseline-v2-<date>/` holds committed record runs; top-level
  `bench/results/subagents-<n>-<ts>.{json,md}` are ad-hoc acceptance runs
  (`mise run bench -- subagents --n 50` writes there by default).
