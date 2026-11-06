# v3 integration gate (OIN-15)

## Verdict: FAIL / do not approve v3

Run on Linux x86_64, branch `v2`, 2026-11-06 UTC, starting at `7d30a3f8`.
Read OIN-15 using the prescribed CLI. No board writes, installs, stash, reset, or clean operations were performed. Existing unrelated untracked files were untouched.

The premise that all twelve implementation packages are committed is false at this revision. Git history and targeted artifact checks agree with the upstream handoffs: only wp-cache, wp-tools-fs, wp-ext-reminders and wp-loop-guards landed. The eight remaining packages are blocked/unimplemented. Missing whole features were not implemented as speculative integration fixes.

## Changes

- Handle `EntryKind::SystemSnapshot` in `oino-view` tree display, fixing the workspace compile error. Regression test verifies empty display/copy text.
- Replace the obsolete Anthropic first-party inline-tool assertion with request-level declaration and no-inline-beta assertions, as required by decision C.
- Rebuilt all 37 WASM components. Only tracked `extensions/dist/checksums.json` changed; tracked component bytes were unchanged. All 100 checksum entries were independently verified against disk. Toolchain metadata now records rustc 1.99.0 rather than the upstream build's 1.99.1.
- Updated changelog with the failed integration gate. No release version bump or publication.

## A–L coverage

Statuses distinguish scoped evidence from full acceptance. PASS (scoped) does not imply the integration gate passed.

| Decision | Implemented where / upstream commit | Test / evidence | Status |
|---|---|---|---|
| A frozen Oino system prompt | `oino-resources/src/system_prompt.rs`, core session prompt/tools; `9c5b1f3e` | `v3_prefix::prompt_excludes_factory_content` and persisted-prefix tests | PASS (scoped); legacy format-v2 append-only gap remains |
| B file tools and execution | `oino-tools/src/{read,write,grep,legacy,apply_patch}.rs`, edit; `8b46ab8a`, `f36aff30` | `v3_fs`: 5 Linux tests pass; real model read/grep/edit/bash succeeds | PARTIAL: new shell schema, task_output/stop, shared registry and Windows jobs absent; ACP mutation regression |
| C append-only context | core session prompt/tools/host, session entry, AI transcript, provider serializers; `9c5b1f3e`, `f85ba69a` | `v3_prefix`: 45 tests; `v3_reminders`: 2; Anthropic tool changes: 3 | PARTIAL: byte comparisons pass in test serializers; live wire prefix recording unsuccessful; discovery API absent |
| D skill reminders | `extensions/unipi/skill-registry`, reminder-migrated extensions; `322cfc01` | extension workspace skill tests and core reminder tests pass | PARTIAL: workflow/watchdog Decision judging removal awaits missing execution/autonomy work |
| E autonomy | required `crates/oino-core/src/session/permission.rs` absent | required `v3_autonomy.rs` and `AutonomyControl.test.tsx` absent | FAIL: no v3 gate, approval protocol or UI acceptance |
| F compaction | legacy `session/compaction_flow.rs` remains | required `v3_compaction.rs` absent; prefix test manually appends legacy compaction | FAIL: no v3 state/epoch/delta/trigger acceptance |
| G loop guards | `oino-agent/src/agent_loop.rs`; `bf1a3d2d` | `v3_guards`: 4 pass; old pi event golden fails on new reminder events | PASS (scoped), full suite fails |
| H codemode/deferred discovery | declaration persistence exists in core; codemode registration absent | required `v3_codemode.rs` absent; prefix loading test uses direct host registration, not tool_search | FAIL: no end-to-end tool_search/codemode |
| I semantic grep | exact in-process grep exists; required `oino-tools/src/zvec/` absent | `v3_fs` confirms default schema omits semantic mode; `v3_zvec.rs` absent | FAIL: consent/settings/engine/Windows acceptance absent |
| J memory/dream | existing dream UI/backend plus memory usage metadata `322cfc01` | extension workspace `usage::tests::memory_usage_and_prune`, schedule tests pass | PARTIAL: no live dream execution; Rust-only MemPalace metadata limitation remains |
| K sheet select | existing `8abc78b3`, `SheetSelect.test.tsx` | included in frontend's 346 passing test files | PASS (existing shared regression); named downstream Subagents acceptance not established |
| L Superagent | required host `superagent.rs` and frontend component directory absent | required dispatcher/kanboard/SuperagentPage v3 acceptance files absent | FAIL: dispatcher/protocol/worker policy/page absent |

Paths abbreviated to crate-local source above. Binding requirements remain `00-owner-decisions.md` and `01-spec.md`.

## Full checks

All Cargo commands used `CARGO_BUILD_JOBS=8`. Root target started at 15 GB and remained below 30 GB (23 GB at final check), so the requested root-target GC condition was not reached. Separate app/extension caches also exist; aggregate caches exceed 30 GB and need a coordinated later GC when builds are idle. No cache cleanup was run while another workspace was compiling.

| Command | Result |
|---|---|
| `cargo build --workspace` | Initially failed exhaustive snapshot match; passes after fix |
| `cargo clippy --workspace -- -D warnings` | FAIL: 7 session-manager warnings; no blanket suppression added |
| `cargo test --workspace` | Compiles after fix; first runtime failure ACP filesystem |
| `cargo test --workspace --no-fail-fast` | FAIL: 17 targets (listed below) |
| `cargo test --manifest-path extensions/Cargo.toml --workspace --no-fail-fast` | FAIL: one target, provider pi fixtures, six cases; obsolete first-party inline test fixed |
| `cargo xtask wasm` | PASS: 37 components |
| `cargo check --manifest-path apps/unipi/src-tauri/Cargo.toml` | PASS |
| `cargo test --manifest-path apps/unipi/Cargo.toml -p unipi-host -p unipi-kanboard -p unipi-proto --no-fail-fast` | FAIL: four host targets; kanboard/proto pass |
| `cd apps/unipi && npx tsc --noEmit` | PASS |
| `cd apps/unipi && npx vitest run --pool threads --maxWorkers 4` | 346 files / 3322 tests pass; one additional suite cannot load missing temporary feed |
| core `v3_prefix`, `v3_legacy_replay`, `v3_reminders` | 45 + 4 + 2 pass |
| tools `v3_fs`; agent `v3_guards`; `cargo test -p oino-view` | 5 + 4 + 44 pass |
| provider `anthropic_tool_changes` after assertion update | 3 pass |
| `git diff --check` | PASS |

### Unfixed failure inventory

Root targets: ACP `acp`; agent `agent_loop_golden`; CLI `contract`, `dist`, `rpc`; client `hub`, `in_process_gui`; core `cwd_change`, `gui_feeds`, `mode_tools_next_turn`, `runtime_flow`, `session_flow`, `session_golden`; resources `golden`, `loader`; tools `behaviour`, `golden_tools`.

Several assertions intentionally encode v2 behavior (pi prompt sections, default tool set, reminder-free event order, trailing newline in short reads). Others expose actual integration defects, so expectations must not be blindly regenerated:

- **P1 ACP writes bypass the client filesystem:** `crates/oino-tools/src/edit/storage.rs:167–179` treats `env.is_local()` as authorization for direct OS atomic replacement. ACP's environment reports local while routing writes through `fs/write_text_file`. New write/edit no longer call that environment method. `crates/oino-acp/tests/acp.rs:205` observes zero client writes instead of one. Fix requires a capability-aware atomic mutation boundary, preserving ACP client routing and local atomic guarantees.
- **P1 actual format-v2 open rewrites history:** `crates/oino-session/src/manager/mod.rs:502–506` still atomically serializes migrated records. Passing fixture tests cover v2-era entries in format-v3 JSONL, not the real older file format.
- **P1 missing autonomy/discovery/execution/compaction packages:** cannot safely certify unattended behavior or the requested background flows.
- **P2 strict lint:** session manager list/offsets contain too-many-lines, collapsible-if and items-after-statements diagnostics. These pre-existing style issues were left unchanged.
- **P2 frontend QA fixture:** `apps/unipi/src/lib/zz-qa-update-android.test.tsx:13` reads `/tmp/qaupd/local/feeds/app/android/latest.json`. Missing external fixture prevents collection; no fabricated feed was supplied to claim a full pass.
- **P2 host tests:** `instance::a_host_terminated_while_starting_is_not_a_failed_start` repeatedly times out waiting for a lock; `sessions_hub::fork_keeps_the_original_wired_to_its_own_runtime` expects 4 messages, receives 3; `snapshot_bound::a_huge_session_opens_with_a_bounded_snapshot` expects an error, receives a bounded success; `unipi_agents::memory_side_save_runs_after_a_writing_task` does not find expected context. Root cause of each remains unverified.
- **P2 provider pi fixtures:** six API cases expect mid-conversation system mutation, changed inline declarations or unescaped source bytes; review individually against v3 before updating expected wire fixtures.

## End-to-end results

A real model key worked. CLI release binary, explicitly loaded `oino.openai-compat`, OpenRouter `openai/gpt-4.1-mini`, no context/skills/templates, isolated HOME/agent/cwd under `/tmp/oino-v3-integrate/real`. An initial attempt without explicit runtime key returned 401; retry with the environment key passed. No credentials were written to the report or captured event files.

The successful run performed all four requested tools: read, grep, edit and bash. Every tool result was non-error, process exit was zero, and notes changed from `alpha\nbeta\n` to `gamma\nbeta\n`. Evidence: `real-model-key.jsonl`, `real-model-key.stderr`, `real-model-key.exit` under the evidence directory below.

| Scenario | Result |
|---|---|
| Real model read/grep/edit/bash | PASS |
| fireAndForget + task_output/task_stop | BLOCKED: required interfaces not implemented |
| tool_search loads deferred tool | BLOCKED: tool not registered; existing prefix test uses direct host registration |
| Forced v3 compaction, vcc and llm | BLOCKED: v3 implementation/acceptance absent; legacy test setup is not this check |
| Old v2 session replay | Fixture replay PASS (4 tests); actual format-v2 append-only FAIL by inspected migration path. Owner sessions were never opened or modified |
| Unattended autonomy block | BLOCKED: v3 policy absent; no risky command was run to manufacture evidence |
| Cached prefix bytes identical across all scenarios | NOT VERIFIED: scenarios missing. Existing production-serializer tests pass with movable cache-control hints excluded. Attempt to record real requests through a loopback proxy failed with connection error before any request arrived; zero-body capture is not treated as evidence |
| Skill reminders / loop guards / dream schedule | Scoped acceptance/unit tests pass; no live dream job launched |

## Release bundle (built, NOT installed)

Followed `scripts/unipi-build.sh` with `UNIPI_RELEASE_IDENTITY=1 CARGO_BUILD_JOBS=8`. First build failed because the musl Rust target was absent. `rustup target add x86_64-unknown-linux-musl` succeeded, then the same script completed, including zigbuild sidecar, staged dist, frontend and Tauri bundling. No installer or supervisor restart ran.

Deb: `apps/unipi/src-tauri/target/release/bundle/deb/UniPi Oino_1.0.1_amd64.deb`

- Package `uni-pi-oino`, version `1.0.1`, architecture `amd64`, 80.44 MiB.
- SHA-256: `abf7f2a8f4bfcdac0b86fe4e7866cd4e60d6469bd77bd51fbb1070a98fa4b5ba`.
- Script verified 30 bundled extensions. AppImage was also produced.
- This is an integration artifact from an incomplete v3 tree, not release approval. Existing version was deliberately preserved.

## Evidence and continuation

Raw logs, exit files, isolated sessions, real-model events and the failed recording experiment remain at `/tmp/oino-v3-integrate/`. This directory is local and temporary; the verdict, failure names, checks, bundle hash and coverage table above are durable in git. Preserve the raw directory before machine cleanup if detailed traces are needed.

Next: resolve and finish the eight blocked implementation packages in dependency order, fix the ACP atomic-operation boundary and actual format-v2 append-only migration, then reconcile v2 golden assertions with independently reviewed v3 expectations. Rerun every failing target and all end-to-end scenarios with request-body recording. Actual Windows/MSVC file replacement, PowerShell, process-tree and tray dispatcher gates remain mandatory and unrun. Supervisor owns any installation decision. Board remains unchanged.
