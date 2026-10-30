# Wave 1 QA report — core, ABI, kanboard CLI, long-horizon, oino-ui

Date: 2026-10-30/31 · Branch: `v2` · HEAD verified: `917e92c` (`917e92cab794aa57355a406b8c672f7563c76512`) ·
Reviewer: wave1-qa. Every acceptance row re-run by QA on this machine (rustc/cargo 1.99.1,
Linux x86_64, 32 cpus, ext4; pi oracle = pi-coding-agent **1.4.0** at the mise 24.16.0 install).

Scope: `specs/core.md` WP-C1..C11, `specs/ext-abi.md` WP-X1 / X2 (via C6) / X3 / X7,
`specs/ui-protocol.md` WP-U1 / U2, the wave gates, and DIRECTION/RULINGS compliance.

## Verdict

**All 17 WPs PASS.** Full gates green. No P0/P1. Three P2 defects (extensions-workspace fmt
drift on wave-1 files, uncommitted oracle/fixture additions, undemonstrated hibernation bench
expectation) and four P3 observations — none block wave 2 starting, but P2-1/P2-2 should be
cleaned before the wave-2 tree forks from here.

| WP | Verdict | Key evidence (all re-run by QA) |
|---|---|---|
| C1 writer + barriers | PASS | `durability.rs` 3/3, `writer_durability.rs` 1/1 (kill -9 + reopen), `manager.rs` `appends_reuse_one_open_handle`, `sync_fails_closed_when_file_cannot_be_written`, `sync_retries_failed_first_flush`; goldens green |
| C2 dispatch + repair | PASS | `repair.rs` 6/6, `resume_repair.rs` 4/4, `session_flow.rs` 11/11; `node scripts/pi-oracle/core/session-load.mjs` → "all checks passed" (committed and working-tree versions) |
| C3 lease + takeover | PASS | `lease.rs` 6/6 (in-process fail-fast, kernel release on `kill -9` child), `lease_takeover.rs` 4/4, oino `lock.rs` 3/3 e2e (two processes; reload ≤10 s; `SessionLocked` text) |
| C4 shared engine | PASS | `shared_engine.rs` 2/2: one `Engine` + one ticker for two services, compile once, **second cold start asserted cheaper than the first** (test line ~215) |
| C5 crash protection | PASS | `trap.rs` 1/1 (session-A-only disable + both events), `crash_protection.rs` 2/2 (`SessionCrashed` + run_state idle + next prompt works + siblings survive) |
| C6 safe mode + exclude | PASS | `safe_mode.rs` unit + e2e `safe_mode_banner_names_suspects_and_resets_on_success`; `exclude_extension.rs` 5/5; manual runs below |
| C7 lazy hub | PASS | `hub.rs` 5/5: lazy list (0 loaders, no lock files), open → lease+repair+live, idle eviction (`session_shutdown{idle}` seen, reopen continues), running bash keeps live, crashed listed `crashed`, services LRU |
| C8 hibernation | PASS | `hibernation.rs` 6/6 + wasm-host `hibernate.rs` 1/1: hibernate within window, wake continues same child session, subscribers prevent hibernation, parent dispose closes shells, wake repairs damaged child file |
| C9 extension-state fixes | PASS | restore units in ext workspace (`local_map_restores…`, `pending_compaction_restores…`, `sidekick_pairing_restores…`); **kill-and-restart test in all 7 harnesses**, all green; `cargo xtask wasm` + checksums idempotent |
| C10 package split | PASS | byte-scan below + `binary_carries_no_bundled_components`; sources unit 11/11 (`share_env_override_and_order_shadowing`), `dist.rs` 4/4 (staged share dir, user shadow, docs-root chain, absent-omitted); `scripts/docs-check.sh` ok |
| C11 benchmarks | PASS | `bench/results/{baseline,after}-v2-2026-10-30/RESULTS.md` + n=1/10/50 JSON/MD committed; barrier µs recorded (1010–1121 µs mean); harness re-run by QA (n=1) works; parity gates green. See P2-3 for the hibernation expectation |
| X1 host-service | PASS | `wasm-tools component wit wit/oino-extension` ok; `host_service.rs` 4/4 (per-name gating, `"*"` bundled-only, discovery ungated, not-found, registry-before-inner); workspace clippy/test green; dist rebuilt (37 components) |
| X2 --exclude-extension | PASS | via C6; manual: staged dist + `--exclude-extension unipi.notify` → `Warning: Excluded extension "unipi.notify" not loaded`, exit 0, answer; no-match warning, exit 0; `-ne` unchanged; `--help` documents both |
| X3 kanboard CLI | PASS | dist `bin/unipi-kanboard` built by xtask; `project add --json` → exit 0 pretty JSON; `frobnicate --json` → exit 2, `{"ok":false,"error":"error: unrecognized subcommand 'frobnicate'","kind":"usage"}` on stderr only; `bin_resolve.rs` 4/4; harness 6/6; checksums diff clean after rebuild |
| X7 long-horizon retry | PASS | ext-workspace `graph::tests`: `classify_failure_table`, `transient_retries_with_backoff_then_exhausts`, `retry_after_honored_without_consuming_budget`, `single_huge_retry_after_is_not_honored` (1 h cap), `bind_rearm_schedule`; sticky `unipi:long-horizon:work-failed` + `os_notify` wired (lib.rs:74,1830) |
| U1 oino-ui types | PASS | `wire.rs` 54, `fixtures.rs` 4 (corpus + round-trip), `schema_parity.rs` 3 (schema ≡ Appendix A), `validate.rs` 11 (limits); **bench: validate() = 222 µs** on a 256 KiB tree (≤1 ms); `cargo check -p oino-ui --target wasm32-wasip2` clean |
| U2 diff/patch + vectors | PASS | `patch.rs` 20 incl. `apply_inverts_diff` (proptest **10 000 cases**) and fallback-to-replace; **207 layout vectors** (≥200, constraint × flex × spacing × margin); `gen_layout_vectors.rs` regen is byte-compare → no-op diff |

## Full gates (all re-run on HEAD `917e92c`)

| Gate | Command | Result |
|---|---|---|
| Format (root) | `CARGO_BUILD_JOBS=8 cargo fmt --all --check` | **PASS** (exit 0) |
| Format (extensions ws) | `cargo fmt --manifest-path extensions/Cargo.toml --all --check` | **FAIL — 2 files** (P2-1) |
| Lints | `CARGO_BUILD_JOBS=8 cargo clippy --workspace --all-targets -- -D warnings` | **PASS** (exit 0, zero diagnostics) |
| Tests (root ws) | `CARGO_BUILD_JOBS=8 cargo test --workspace` | **PASS** — exit 0, **939 passed / 0 failed** |
| Tests (extensions ws) | `cargo test --manifest-path extensions/Cargo.toml --workspace` | **PASS** — exit 0, **592 passed / 0 failed** (68 binaries) |
| Wasm dist | `cargo xtask wasm` + `git diff --exit-code extensions/dist/checksums.json` | **PASS** — 37 components + `unipi-kanboard` bin; checksums idempotent (exit 0) |
| Harnesses | `sh extensions/unipi/{subagents,workflow,utility,core,ask-user,notify,kanboard}/harness/run.sh` | **PASS** — 6+7+10+3+3+5+6 = 40 tests, 0 failures (each incl. a `kill_and_restart_*` step) |
| pi oracle | `node scripts/pi-oracle/core/session-load.mjs` | **PASS** — "all checks passed" (7 checks committed, 9 with the uncommitted C3 addition) |
| pi oracle (generators) | `tools/{run,meta,units}.mjs`, `agent/validation.mjs` regenerated and diffed | **IN SYNC** with pi 1.4.0 output (byte-equal); `session-events`/`compaction` generators embed random ids/tempdirs — parity enforced by the Rust goldens (green), not byte-regen |
| Docs | `sh scripts/docs-check.sh` | **PASS** |
| WIT | `wasm-tools component wit wit/oino-extension` | **PASS** |

Note: all cargo runs had ambient `UNIPI_KANBOARD_*` unset (this QA session itself exports
`UNIPI_KANBOARD_MAX_SESSIONS=5`, which breaks `eight_threads_race…` — the hazard WP-X3
documented in `crates/unipi-kanboard/tests/common/mod.rs`; with the vars unset everything passes).

## Manual probes (all by QA on this machine)

### 1. Durability: kill -9 mid tool call, resume, pi 1.4.0 loads the repair

Script: `/tmp/qa/durability-probe.sh` (faux provider, print mode, sandboxed HOME/agent dir).
Sequence and results:

1. `oino -e builtin:faux --model faux/faux-1 --session-dir <dir> -p "run the sleep"` with a
   scripted `bash` tool call `sleep 30`. The `oino.tool-dispatch` custom record was observed
   **on disk 2 polls (~0.4 s) after session creation** — i.e. before the tool body finished.
2. `kill -9` while the body ran. Pre-resume file: 7 lines, last entry = the dispatch record;
   no toolResult.
3. Resume (`-c -p continue`): **exit 0**, answered. Post-resume file adds, in order:
   - toolResult message: `isError: true`, text "Tool execution was interrupted before
     completion; the outcome is unknown. The effect may or may not have happened. Do not
     retry blindly…" (spec-exact D-REPAIR text), `toolCallId: "t1"`;
   - one `oino.resume-repair` custom record: `{"closedToolCalls":["t1"],"tornTail":false}`;
   - the new user + assistant messages. Exactly **1** dispatch and **1** resume-repair record
     (idempotent, no duplicates).
4. **pi 1.4.0** (`SessionManager.open` + `buildSessionContext` via the oracle lib): loads the
   file — 10 entries, context 5892 chars, **includes** the interrupted toolResult (balanced
   transcript — a next provider request converts) and **excludes** the `oino.*` custom records.

**PASS** end-to-end (D-WRITER/D-FLUSH/D-DISPATCH/D-REPAIR + pi parity in one probe).

### 2. `--exclude-extension` and safe mode (manual, built binaries)

- `--exclude-extension unipi.kanboard,oino.anthropic -p hi` (clean agent dir): exit 0, answer,
  two `Warning: --exclude-extension "…" matched no extension package` diagnostics.
- Staged `OINO_DIST_EXTENSIONS=extensions/dist` + `--exclude-extension unipi.notify`:
  exit 0, answer, `Warning: Excluded extension "unipi.notify" not loaded` (real package dropped).
- Tripped safe mode (`agent/state/oino.safe-mode.json` with 2 failures): exit 0, answer
  (**providers on**), `Warning: Safe mode: extensions are off after 2 failed starts; suspects:
  unipi.notify (init failed: boom)`; state file after success:
  `{"v":1,"consecutiveFailures":0,"lastFailures":[]}` (reset).
- `--help` documents `--exclude-extension` next to `--no-extensions` (verified).
- `-ne`: exit 0, providers stay (answer produced).

### 3. The `oino` binary embeds no extensions (P-BUNDLE)

- release binary (80.6 MB): `OINO_BUNDLED` → 0 occurrences, `oino-bundle` → 0,
  `unipi.kanboard/extension.json` → 0; the single `unipi.kanboard` string is inside the
  `--exclude-extension` help example. `crates/oino-bundle/` absent. Mirrors the committed test
  `crates/oino/tests/dist.rs::binary_carries_no_bundled_components` (passed).

### 4. `unipi-kanboard` CLI `--json` contract (dist `bin/unipi-kanboard`)

- `--actor agent project add --root . --name Demo --json` → exit 0, one pretty JSON value on
  stdout (project object).
- `frobnicate --json` → exit 2, stdout empty, one stderr JSON line:
  `{"error":"error: unrecognized subcommand 'frobnicate'","kind":"usage","ok":false}`.
- `status --json` → `{ok, home, binary, projects…}`; home default `~/.oino/state/unipi.kanboard`.

## DIRECTION / RULINGS compliance

- **§1 no compiled-in extensions / bundle moved** — C10 ✓ (probe 3).
- **§3.1 durability 2–6** — barriers fail-closed ✓ (C1 tests + chmod-0444 scenario), dispatch
  before body ✓ (probe 1), repair-on-resume with spec text ✓, single-writer + Devin-style
  takeover ✓ (C3 e2e), extension state from custom entries + kv ✓ (C9 + harness restarts).
- **§3.2** — shared engine ✓ (C4), lazy session records ✓ (C7), hibernating subagents ✓ (C8),
  benchmarks before/after ✓ (C11; hibernation variant open — P2-3).
- **§3.3** — per-session trap disable + `ExtensionDisabled` ✓, `SessionCrashed` isolation ✓,
  safe mode after 2 failed starts with suspects banner ✓, `--exclude-extension` ✓.
- **§3.4.1/3.4.2 (wave-1 share)** — `kanboard.wit` deleted, CLI via `process.exec`, no daemon ✓
  (X3); generic `host-service` WIT additive ✓ (X1); `cron.wit`/`browser.wit` removal is X4/X5
  (wave 2), correctly untouched; package still `@1.0.0` (bump is X6) ✓.
- **RULINGS §"core.md §7"** — providers on in safe mode ✓ (manual), one trap per session with
  the process breaker as backstop ✓ (`trap.rs` + runner), hibernate default 120 s ✓, hub idle
  900 s ✓ (defaults asserted in hub tests).
- **RULINGS §"ext-abi §11"** — 1 h cumulative Retry-After cap ✓
  (`single_huge_retry_after_is_not_honored`); `unipi.browser` rename is X5 (wave 2) — n/a.
- **Session format v3 only, parity tests green** — oracle + goldens + 939/592 test passes ✓;
  sidecar files (`.lock`, `.lock.json`, `.takeover`) proven inert to pi's reader (oracle).
- Spec-format note: ext-abi §4 acceptance text says `--mode print`; the real flag is `-p`
  (`--mode` accepts text/json/rpc/acp). C6's tests/e2e use `-p`; documentation-level mismatch
  only (P3-5).

## Defects

### P2 (fix before wave 2 forks the tree)

1. **Extensions workspace fmt drift on wave-1 files.**
   Repro: `cargo fmt --manifest-path extensions/Cargo.toml --all --check` → exit 1,
   2 diffs: `extensions/fixtures/echo/src/lib.rs:196` (the X1 host-service probe arm) and
   `extensions/fixtures/hibernate-probe/src/lib.rs:13` (C8's shutdown handler).
   Expected: clean (wave rule "each wave ends green: build, tests, lints").
   Actual: two unformatted hunks committed in `9ce0eae`/`dfd08bb` and `2d17d52`.
   Root cause: the root-workspace fmt gate doesn't cover the extensions workspace, and no CI
   gate runs `cargo fmt` there. Fix: `cargo fmt --manifest-path extensions/Cargo.toml` + add
   the gate (xtask or CI).
2. **Uncommitted wave-1 artifacts left in the tree.**
   Repro: `git status` → `M scripts/pi-oracle/core/session-load.mjs` (WP-C3's additive
   lease-sidecar oracle checks — good content, QA ran both versions, both pass),
   `M extensions/unipi/workflow/harness/Cargo.lock`, untracked `node_modules/`,
   `package.json`, `package-lock.json` (typescript dev-deps installed for the oracle at repo
   root — should be gitignored or vendored deliberately), `docs/dev/v1/round6/`,
   `extensions/unipi/kanboard/harness/target/`.
   Expected: committed or cleaned; the C3 sidecar check in particular belongs in the repo (it
   is the only D-LOCK-vs-pi regression check). Likely owners: wp-c3 (oracle), wp-c9 (harness
   lock), whoever installed the node deps (add to .gitignore).
3. **"≥30 % RSS drop with hibernation on" (core.md §4) is undemonstrated.**
   Repro: `grep -rn hibernate crates/oino-bench xtask/src/bench.rs` → no hits; the recorded
   after-runs never enable hibernation (default 120 s idle; the bench finishes sooner), so the
   E-HIBERNATE benefit column is absent — after vs before is per-child total **+3.8 %**
   (honestly flagged in `bench/results/after-v2-2026-10-30/RESULTS.md`).
   Expected: a bench variant (`--hibernate-after-sec 0/1`) demonstrating idle-children RSS
   drop; the WP-C11 acceptance row itself ("numbers committed + table + barrier + no parity
   regression") is met, but the §4 efficiency expectation for C8 has no evidence.
   Likely file: `crates/oino-bench/src/main.rs` / `xtask/src/bench.rs` (knob), scenario in
   `crates/oino-bench`.

### P3 (observations, owners known)

4. **Per-child spawn ΔRSS at n=50 is +12.2 % vs baseline** — breaches the §4 ">10 % worse"
   guard on that single row; dominated by noise (n=1 swings ±40 %), turn/total rows are
   +1.6 %/+3.8 %. Already flagged in RESULTS.md; watch at the next re-run.
5. **Spec text `--mode print`** (ext-abi §4/§X2 acceptance) is not a real flag — `-p` is.
   Doc-level; C6 implemented the real thing.
6. **Stale `oino-kanboard`/`OINO_KANBOARD_*` names** in `scripts/install.sh:9,43,136`,
   `scripts/install.ps1:8,21,80`, plus `docs/install.md`, `docs/environment-variables.md`
   and `scripts/{podman-test.sh,release/build-host.sh}` — installers would fail to build the
   renamed crate. Owned by WP-D (dist.md §6), wave 3.
7. **`UNIPI_KANBOARD_*` ambient-env test hazard** — an oino/UniPi session exports these and
   breaks `unipi-kanboard` in-process tests (`eight_threads_race…`); documented in
   `crates/unipi-kanboard/tests/common/mod.rs`, hit by QA itself. Consider env-cleaning the
   test harness instead of relying on the runner.

## Reproduction artifacts

QA scripts/logs (ephemeral): `/tmp/qa/` — `gates.sh`, `harnesses.sh`, `durability-probe.sh`,
`pi-load-durability.mjs`, per-gate logs (`fmt.log`, `clippy.log`, `test.log`, `ext-test.log`,
`harness-*.log`, `wasm.log`). The durability probe session lives at
`/tmp/qa/durability/sessions/--home-oi-Projects-Personal-llm-oino--/`.
