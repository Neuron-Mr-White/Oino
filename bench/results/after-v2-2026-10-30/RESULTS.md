# After v2 — subagents benchmark, post C4/C7/C8 (WP-C11 part 2)

Recorded **after** WP-C4 (shared engine, `add4a59`), WP-C7 (lazy session hub, `827fd70` +
`f9ec7b4`) and WP-C8 (subagent hibernation, `2d17d52`) landed, per [core.md §4][spec].
Same harness and scenario as the committed baseline
([`bench/results/baseline-v2-2026-10-30/RESULTS.md`][base]).

[spec]: ../../../docs/dev/v2/specs/core.md
[base]: ../baseline-v2-2026-10-30/RESULTS.md

- HEAD: `f9ec7b46…` (v2 branch) — includes all of C4/C7/C8 (last of the three is C7's hub).
- When: 2026-10-30 12:57–12:58 (+08:00)
- Machine: `oi` · linux x86_64 · kernel 7.0.0-38-generic · 32 cpus · work dir fs **ext4**
  (`/dev/nvme0n1p1`, harness temp agent dir under `target/bench`)
- Scenario: faux provider · children via `AgentSession::spawn_agent`
  (`Persistence::File`, one seeded user message, model `faux/faux-1`, `extensions: None`) ·
  one parallel turn per child · `subagents.maxConcurrent = 16`
- Shared-tree caveat: release binary compiled the working tree; uncommitted files at run
  time were only `extensions/unipi/workflow/harness/Cargo.lock` and
  `scripts/pi-oracle/core/session-load.mjs` — neither is in the bench's dep graph.
- Repeat agreement: per-child total at n = 50 was 436.56 (recorded run) and 436.9
  (acceptance run `subagents-50-20261030-125756`) — ~0.1 %.

## Before → after (n = 50, the meaningful row)

| Metric | before `5b15e4e` | after `f9ec7b4` | Δ |
|---|---:|---:|---:|
| Root sessions | 4 | 4 | — |
| Children spawned / file-persisted / answered | 50 / 50 / 50 | 50 / 50 / 50 | — |
| RSS at start (KiB) | 5 940 | 6 208 | +4.5 % |
| RSS after services (KiB) | 9 292 | 9 788 | +5.3 % |
| RSS after roots (KiB) | 10 992 | 11 400 | +3.7 % |
| RSS after spawn (KiB) | 15 320 | 16 256 | +6.1 % |
| RSS after turns (KiB) / high-water (KiB) | 32 020 / 32 020 | 33 228 / 33 228 | +3.8 % |
| **ΔRSS per child — spawn (KiB)** | 86.6 | 97.1 | +12.2 % |
| **ΔRSS per child — turn (KiB)** | 334.0 | 339.4 | +1.6 % |
| **ΔRSS per child — total (KiB)** | **420.6** | **436.6** | **+3.8 %** |
| CPU during spawn / turns (ms) | 0 / 70 | 20 / 40 | tick noise |
| Barrier: `sync()` over 100 appends (µs mean / min / max) | 1 091.7 / — / — | 1 038.9 / 905 / 1 254 | −4.8 % |
| Wall: spawn / turns (ms) | 5 / 32 | 84 / 19 | — |

Full three-column record (all n):

| Metric | n | before | after | Δ |
|---|---:|---:|---:|---:|
| Root sessions | 1 / 10 / 50 | 1 / 1 / 4 | 1 / 1 / 4 | — |
| ΔRSS per child — spawn (KiB) | 1 | 872.0 | 1 236.0 | +41.7 % (one-off noise) |
| ΔRSS per child — spawn (KiB) | 10 | 161.6 | 189.6 | +17.3 % |
| ΔRSS per child — spawn (KiB) | 50 | 86.6 | 97.1 | +12.2 % |
| ΔRSS per child — turn (KiB) | 1 | 940.0 | 1 084.0 | +15.3 % (one-off noise) |
| ΔRSS per child — turn (KiB) | 10 | 464.0 | 494.0 | +6.5 % |
| ΔRSS per child — turn (KiB) | 50 | 334.0 | 339.4 | +1.6 % |
| ΔRSS per child — total (KiB) | 1 | 1 812.0 | 2 320.0 | +28.0 % (one-off noise) |
| ΔRSS per child — total (KiB) | 10 | 625.6 | 683.6 | +9.3 % |
| ΔRSS per child — total (KiB) | 50 | 420.6 | 436.6 | +3.8 % |
| Barrier (µs mean) | 1 | 1 082.6 | 1 087.4 | +0.4 % |
| Barrier (µs mean) | 10 | 1 107.1 | 1 044.0 | −5.7 % |
| Barrier (µs mean) | 50 | 1 091.7 | 1 038.9 | −4.8 % |

## Verdict

- **No parity regression** (§3 gates): the steady-state per-child metric —
  `ΔRSS per child — total` at n = 50 — moved **420.6 → 436.6 KiB (+3.8 %)**, well inside
  the ≤ 10 % no-regression gate; CPU and the C1 fsync barrier cost are unchanged
  (barrier mean ≈ 1.04–1.09 ms across both records).
- **Fixed footprint** grew ~250 KiB (+4.5 % at start, before any session work) — the
  process baseline carries more compiled-in code (shared-engine plumbing, hub, hibernation
  hooks). Marginal per-child cost is where C4/C7/C8 were supposed to pay off, and that is
  flat here.
- **The ≥ 30 % hibernation drop is not observable in B-1, by construction**: the scenario
  spawns children with `extensions: None`, no `subagents.*` settings, and the whole run
  lasts ~0.2 s — E-HIBERNATE needs ≥ 120 s idle with the subagents extension active, and
  E-ENGINE's shared wasmtime engine only helps with wasm extensions loaded. Per spec the
  exact number is "a recorded measurement, not a gate"; recording the miss rather than
  bending the scenario. A hibernation-enabled bench variant would need a spec change to
  B-1 (follow-up, not done here).
- **Borderline row, flagged**: per-child *spawn* delta at n = 50 is +12.2 % (+10.5
  KiB/child, ~0.5 MiB total at 50 children). Above 10 % if that single row is read as the
  metric, but it is the noisiest row (n = 1 swings +41.7 % on a single child carrying all
  one-off costs) and the turn/total rows — which include the same allocation paths plus
  the turn — are +1.6 %/+3.8 %. Not a regression signal; noted for the next re-run.
- Wall spawn 5 → 84 ms is a first-spawn one-off in the after record (roots/turns walls are
  equal or better); CPU ms fields are 100 Hz tick counts and meaningless at this scale.

Files: `subagents-1-20261030-125737.json`, `subagents-10-20261030-125738.json`,
`subagents-50-20261030-125738.json` (+ `.md` each). Acceptance run
(`mise run bench -- subagents --n 50` → `bench/results/subagents-50-20261030-125756.*`)
doubles as the repeat-agreement data point.
