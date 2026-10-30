# WP-H10 — host integration + performance record (spec-host §7.4/§7.5/§12)

Measured on this machine while implementing `[host-h10]`, branch `v2`, branch base
`886316a` plus the `[host-h10]` working tree, rustc 1.99.1, Linux x86_64, 32 CPUs, ext4. Host binary: the **debug** build
cargo produces for tests (`target/debug/unipi-host`), so absolute memory numbers are
bloated by debuginfo — the per-session deltas and latencies are what the gates read.

Reproduce (the provider package comes from `extensions/dist`):

```sh
cargo xtask wasm                 # once (CI: the wasm job's `dist` artifact)
cargo test -p unipi-host --test integration -- --ignored --nocapture
```

The test is `crates/unipi-host/tests/integration.rs::headless_host_client_e2e_and_performance_gates`.
It spawns the real binary:

```text
unipi-host serve --data-dir <tmp> --relay https://127.0.0.1:9 --relay-only \
  --no-extensions --mode manual
```

with `OINO_AGENT_DIR`, `OINO_DIST_EXTENSIONS` and `HOME` sandboxed under one temp dir,
then talks frames + proto over `app.sock` (the named pipe on Windows). CI wiring:
`.github/workflows/ci.yml` jobs `v2-host-integration` (Linux, blocking) and
`v2-host-integration-windows` (Windows, `continue-on-error`: the spec's "Windows
secondary"); both consume the wasm job's `dist` artifact.

## Scenario (all green, two consecutive runs)

| Step | What it proves | Result (run 2) |
|---|---|---|
| startup | host binds iroh (relay-only), app.sock, services, cron | ready in **104 ms** (210 ms in one run on a loaded machine) |
| local admin | `host_info` reports our pid/mode/proto; `devices_list`; `pair_begin` + `pair_status{cancel}` | pid/mode/proto match; pairing cancelled cleanly |
| `sessions_list` | fresh host | `[]` |
| `session_new` + faux model | snapshot `reset:true`, `state`/`entries`, `set_model` correlated `ok{id}` | 100k-token reply |
| 100k-token stream | 400,000 chars, 100 SSE content chunks (4k chars each, 2 ms apart) | **27 frames / 28 events (15 delta events in 14 frames)**, 357.6 ms delta window, 375.3 ms total, `seq` 1..=115 strictly increasing |
| drop + resume | close the stream, `session_open{since_seq:115}` | `reset:false`, replay 0, `seq:115`, next `get_state` `ok` |
| terminal | `term_open` (host PTY) → kind-2 `echo` → kind-1 output | `H10-OK` on a `/bin/bash` PTY |
| cron | `cron_add` + `cron_run` against `OINO_CRON_BIN` (faux `oino` printing a `turn_end`) | run `ok`, summary `H10-OK cron`, **103.9 ms** add→ok |
| push | `push_list` with `push.enabled` in `config.json` | `enabled:true`, `host_name` matches, empty list |
| 50 sessions | 49 header-only session files beside the streamed one → next `sessions_list` | **3.0 ms** (53 cards), limit 150 ms |
| RSS | baseline after the first run (engine/provider warm), then 10 idle `session_new` runtimes | **398 → 399 MB** (+1 MB total, **0.05 MB/session**) |

The run before measured 391 → 391 MB (0.04 MB/session), 14 delta frames in 364.9 ms,
50 sessions in 4.6 ms, cron 103.9 ms — same shape, no flake.

## Gates (spec-host §7.5 / WP-H10 acceptance)

| Gate | Limit | Measured | Verdict |
|---|---|---|---|
| `Events` frames while streaming | ≤ 1 frame per 16 ms per stream (delta-bearing frames) | 14 delta frames / 357.6 ms = one per **25.5 ms** (≤ 16 ms window + start/end flushes; the test's bound is `ceil(ms/16)+4` = 27) | PASS |
| coalescing | deltas merge (fewer events than chunks) | 15 delta events for 100 chunks, merged text = 400,000 chars | PASS |
| `seq` | strictly increasing, 1-based | 1..=115, no gaps or reordering on the wire | PASS |
| resume | `since_seq` continues without a reset | `reset:false`, replay 0, same session | PASS |
| 50-session list | < 150 ms cold | **3.0 ms** | PASS |
| 10 idle hub sessions | "within spec-core §3.2 budget" | +1 MB total (**0.05 MB/session**; one shared wasm engine) | PASS |

**Budget note.** spec-core §3.2 (E-LAZY/E-ENGINE) names no byte figure for idle sessions —
only that sessions stay records until needed and providers/extensions share one engine. The
test therefore asserts the meaningful shape: ten extra *live, idle* sessions add less than
32 MB in total (24× headroom over the ~1 MB measured, covering debug-build variance), and
prints the per-session number above. The 32 MB figure is a test guard, not a spec number.

## Defaults / deviations taken

- **Faux provider.** `unipi-host` has no built-in `faux` feature (that lives in the `oino`
  binary), so "session_new with a faux provider env" is implemented as the deterministic
  provider pattern already used by `scripts/pi-oracle/providers-v1/host-e2e.rs`: the test
  stages `extensions/dist/oino.openai-compat`, writes a `models.json` provider `faux`
  (`api: openai-completions`, model `faux-1`) pointed at a loopback SSE server it runs, and
  streams 400k characters in 100 chunks. The host loads it as a *provider* package under
  `--no-extensions`, so no unipi extension is exercised (and none can interfere).
- **100k tokens** is 400,000 characters at the usual ~4 chars/token; the wire sees 100
  real provider deltas, which is what the 16 ms coalescing gate measures.
- **Resume after the run** has an empty replay: 115 was the last numbered event, so there
  is nothing after it; the point is that the reply is `reset:false` (not a snapshot) and the
  stream stays usable. Mid-run drops are covered by WP-H4's `session_stream` test.
- **Windows** runs the same test; the terminal and cron sections are `#[cfg(unix)]` (POSIX
  shells / a shell-script faux binary) and RSS is reported as unsupported outside Linux.

## Follow-ups

- Run the Windows job once on a real runner when wave 2 CI runs; the pipe path is exercised
  but the terminal/cron legs are skipped there by design. The v2 pipe name is fixed
  (`\\.\pipe\unipi-oino-app`, §6.1), so the machine must not already run another
  `unipi-host` when this test starts (CI runners are clean).
- The app-side e2e (`specs/app.md` WP-A30: `apps/unipi/e2e/*.mjs`, "H10 pattern") reuses this
  scenario's sandbox (`OINO_AGENT_DIR` + staged provider + loopback SSE) once the Tauri app
  builds; H10 itself has no app-crate test because that crate is mid-rewrite in this tree.
- If debug-build RSS noise ever makes the 32 MB guard uncomfortable, run the same test
  against a release host binary (the test already takes the binary from Cargo's env, so a
  release test profile is a CI knob, not a code change).
