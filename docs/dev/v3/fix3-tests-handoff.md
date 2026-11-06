# Round 3 assigned test targets

Outcome: all assigned targets pass on branch v2. External Android QA fixture skips when absent. No permission, data-safety, append-only, or prefix-cache assertions were removed.

## Triage

(a) = intended v3 expectation change; (b) = regression or stale non-v3 contract.

| Target | Cause | Class | Fix |
|---|---|---|---|
| tools behaviour | Registry renamed/expanded; old 2000-line shell cutoff | (a) B/H | Assert glob/codemode/tool_search and 32 KiB head+tail with complete private log |
| tools golden_tools | pi schemas/results predate v3 | (a) B | Reviewed v3 metadata/output goldens; retain all mutation checks, ambiguity/overlap failures, BOM/CRLF bytes; direct harness applies legacy result caps |
| wasm bin_resolve | Manifest grants no longer imply process consent; resolved executable is authorized | (a) E | Explicitly authorize each harmless test executable including resolved paths |
| wasm echo | Process probes lacked host consent | (a) E | Authorize sh/cat in both positive and missing-manifest tests, preserving capability-denial checks |
| host instance | Read-only status probe briefly holds host lock, racing startup | (b) | Bounded lock-acquisition retry, preserving exclusive ownership and exit 75 |
| host session_stream | Spawned tap subscribed after immediate rename | (b) | Subscribe synchronously before spawning event pump |
| host sessions_hub | Reopened prefix/message count differed | (b) | Concurrent core fix 4905f105 restores prefix on reopen; existing fork-isolation assertion passes unchanged |
| host snapshot_bound | Fixture expected oversize error despite existing bounded reconciliation | (b) | Assert successful bounded frame, preserved entry IDs/truncation and refetch; media byte checks retained |
| host unipi_agents | Branch copy inherited parent declarations despite memory-only policy | (b), plus (a) C inspection shape | New child drops copied prefix/declaration metadata and relinks entries/reindexes compaction anchors; inspect toolsAdded and require nonempty memory-only declarations excluding memory_delete |
| host unipi_settings | Removed permission command expected success | (a) E | Require rejection and retain no-project-mode assertions |
| provider pi_fixtures six APIs | Mutable system sections and inline additional_tools oracle | (a) A/C/H | Preserve original pi fixture and add explicit v3RequestBody for 14 changed cases; all URL/header/event/output checks retained |
| zz-qa-update-android | External /tmp feed absent at collection | (b) | existsSync guard and describe.skipIf; malformed present fixtures still fail |

## Commits

- c3c0d4de: tools/WASM expectation updates (B/E/H)
- 6dd8831f: explicit provider oracle overrides (A/C/H)
- 5e285596: branch-copy declaration isolation (C/E)
- dc946409: host races, bounded snapshot and retired consent command (E), external fixture skip

## Verification

Raw logs: `/tmp/oino-v3-fix3-tests/`.

- `cargo test -p oino-tools`: 77 passed (`tools-final.log`).
- `cargo test -p oino-wasm-host --test bin_resolve --test echo`: 9 passed (`wasm-final.log`).
- `cargo test --manifest-path apps/unipi/Cargo.toml -p unipi-host --test instance --test session_stream --test sessions_hub --test snapshot_bound --test unipi_agents --test unipi_settings --no-fail-fast`: 31 passed (`host-final.log`). Final strengthened memory_delete assertion rerun separately: pass (`memory-final.log`).
- `cargo test --manifest-path extensions/Cargo.toml -p oino-provider-kit --test pi_fixtures`: 8 passed (`provider-final.log`).
- `cargo test -p oino-core --test agents --test v3_compaction`: 29 passed (`core.log`).
- `cd apps/unipi && npx vitest run --pool threads --maxWorkers 4`: 351 files / 3341 tests passed; external fixture's 4 tests skipped (`vitest-final.log`).
- `cd apps/unipi && npx tsc --noEmit`: pass (`tsc.log`).
- `cargo clippy -p oino-tools -p oino-wasm-host -p oino-core -- -D warnings`: pass (`clippy-lib.log`).
- Provider kit strict library clippy: pass (`clippy-provider.log`).
- Host strict clippy blocked by pre-existing `unipi-proto/src/session.rs:591` large_enum_variant (`clippy-host.log`). Extra root `--tests` lint attempt reports existing core test lint failures (`clippy-root.log`). No suppressions added.
- `git diff --check`: pass. Root target 26 GiB, below 30 GiB cleanup trigger.

## Ownership exceptions and open items

`crates/oino-core/src/agents/child.rs` is outside this assignment's ordinary area, but not a concurrently edited file; required to fix real child policy leakage. Did not edit taskauth-owned hub.rs/tasks.rs/imports/runtime.rs or settings autonomy.rs. Other workers' distribution changes and unrelated untracked files remain untouched.

No full workspace build/test rerun or release approval is claimed. Host strict lint remains blocked as above. Original QA NO-SHIP findings remain owned by their designated parallel workers; these test passes alone do not approve shipping.
