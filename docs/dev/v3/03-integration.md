# v3 integration gate — Round 2 (OIN-15)

## Verdict: FAIL

Linux x86_64, branch `v2`, starting at `dc5d3305`. All twelve implementation packages and the targeted audit fixes are now committed, verified with `git log --oneline --grep='\[v3/'`. Round 1 is retained in `03-integration-round-1.md`. No installation, board writes, stash, reset, or clean was performed. Existing unrelated files and the pre-existing dirty distribution checksum manifest were not staged.

## Integration changes

- Handle new `CompactionState` in the Rust tree renderer and copy-summary path, restoring workspace compilation.
- Add/register all six native Superagent commands. Five use existing authenticated `one_shot`; restricted session creation uses existing `open_session`, channel, stream registry and session command flow. No unrestricted session fallback. Native framing/registration regression passes. Live GUI-to-authenticated-host operation is still unverified.
- Supply missing epoch field in host synthetic session fixture; update sidebar navigation assertion for the Superagent entry.
- Give detach regression a canonical Medium shell call and a bounded wait. The old test waited indefinitely because legacy missing risk normalizes to High and is blocked. The corrected detach case passes; another checkpoint case in the same target still fails.
- Add recorded-provider integration acceptance to `v3_prefix.rs`, exercising actual tool_search, subsequent deferred execution, forced host compaction and unattended denial in one session. Original declarations and immutable slots remain unchanged; loaded schemas append as suffixes.

## Full gates

All Cargo builds use `CARGO_BUILD_JOBS=8`. Root target grew from 8.8 to 19 GB, below the requested 30 GB trigger. No target GC was needed. Separate app/extension caches also consume disk. Available disk initially 636 GB and available RAM 38 GiB.

| Gate | Result | Evidence under `evidence/round-2/` |
|---|---|---|
| `cargo build --workspace` | PASS after compaction renderer fix | `root-build-2.txt` |
| `cargo clippy --workspace -- -D warnings` | FAIL: three client diagnostics | `clippy-final.txt` |
| `cargo test --workspace --no-fail-fast` | FAIL: 20 targets, completed without hang after test fix | `root-test-final.txt` |
| extensions workspace tests, `--no-fail-fast` | FAIL: provider `pi_fixtures` target, six API fixture cases | `extensions.txt` |
| `cargo xtask wasm` | PASS: 37 components; all 100 distribution checksum entries verified | `wasm.txt` |
| src-tauri `cargo check` | PASS | `tauri.txt` |
| host/kanboard/proto tests, `--no-fail-fast` | FAIL: six existing host targets; dispatcher 9 and kanboard v3 4 pass | `host-final.txt` |
| `npx tsc --noEmit` | PASS | `tsc.txt` |
| `npx vitest run --pool threads --maxWorkers 4` | FAIL collection of one external QA fixture; 351 files / 3339 tests pass | `vitest-final.txt` |
| core v3 acceptance after integration | PASS: 92 tests (12 autonomy, 4 codemode, 18 compaction, 6 replay, 48 prefix, 2 reminders, 2 search snapshot) | `v3-final.txt` |
| tools v3 acceptance in workspace run | PASS: 4 execution, 7 filesystem, 4 default-feature zvec | full raw root log |
| native Superagent framing/registration regression | PASS: 1 | `native-bridge.txt` |
| Release script / deb build | PASS, NOT installed | `release.txt` |

The requested plain workspace test was attempted first and hit compilation failure. The final no-fail-fast command runs the same workspace tests and inventories all failures. One earlier run was terminated at the hung detach test binary; the final run completed normally with failure exit 101.

### Remaining failures

- **P1 release blockers:** strict clippy, workspace/extension/host test suites and complete end-to-end proof remain red or incomplete. Do not approve v3 from a successfully built bundle alone.
- **P1 semantic packaging:** default release script builds without optional `zvec`; this deb is exact-search-only. Feature-enabled CPU engine runtime staging/attribution and clean-machine load checks are still required to ship decision I. See `wp-zvec-handoff.md`.
- **P2 clippy:** `crates/oino-client/src/hub.rs:620` emits unused_async and unused_async_trait_impl; `in_process.rs:85` emits needless_pass_by_value. No suppressions or API refactor were added.
- **P2 root failing targets:** agent `agent_loop_golden`; CLI `contract`, `dist`, `frontend`, `rpc`; client `hub`, `in_process_gui`; core `cwd_change`, `durability`, `gui_feeds`, `mode_tools_next_turn`, `runtime_flow`, `session_flow`, `session_golden`; resources `golden`, `loader`; tools `behaviour`, `golden_tools`; WASM host `bin_resolve`, `echo`.
- **P2 host failing targets:** `instance`, `session_stream`, `sessions_hub`, `snapshot_bound`, `unipi_agents`, `unipi_settings`. Examples include fork message counts, renamed-card event, snapshot behavior, permission-gated memory process work and settings migration. Root causes were not all established.
- **P2 legacy assertions:** several pi/provider fixtures still expect mutable system sections, old tool schemas, old event counts and unrestricted process imports. WASM `echo` now reports `needs Autonomy High`; do not restore manifest-as-consent behavior merely to pass those fixtures. Review and adapt expectations individually, preserving security assertions.
- **P2 frontend external fixture:** `src/lib/zz-qa-update-android.test.tsx:13` reads absent `/tmp/qaupd/local/feeds/app/android/latest.json`; untouched. All collectable tests pass after navigation assertion fix.

## A–L coverage table

PASS here means scoped Linux acceptance, not full release closure.

| Decision | Implemented where / commit | Test | Status |
|---|---|---|---|
| A frozen prompt | resource system prompt, core prompt/tools, `9c5b1f3e`; legacy fix `465a6475` | 48 prefix cases incl forbidden content, resume and captured serializers | PASS scoped |
| B tools/execution | tools FS `8b46ab8a`, atomic capabilities `eaa92525`, shared background `68ae3e5b` | 7 FS + 4 execution; ACP target passes; real model background smoke | PASS Linux scoped; Windows open |
| C append-only context | core/session/provider kit `9c5b1f3e`, reminder bindings `f85ba69a`, image policies `465a6475` | real wire hash and combined prefix test; image-policy/reload tests | PASS scoped; complete byte-for-byte HTTP stream proof caveat below |
| D skills | skill registry and reminder extensions `322cfc01` | reminder/retention and extension unit tests | PASS scoped; broad suite still red |
| E autonomy | settings policy/core permission `2d3835a8`, UI `e73039cd` | 12 autonomy + combined denial test; AutonomyControl in passing frontend suite | PASS scoped; Windows/live disconnect gaps remain |
| F compaction | session state/frontier, core compaction `c53b60e3` | 18 tests + forced compact in combined prefix test | PASS scoped; live-model/VCC roundtrip and model-catalog limitations open |
| G loop guards | agent loop `bf1a3d2d` | v3 guards pass in workspace; old event golden fails | PASS scoped |
| H codemode/deferred | tools JS/core declaration path `54b8025b` | 4 codemode + combined actual tool_search load/execute | PASS scoped; documented pi parity deviations remain |
| I semantic grep | embedded optional CPU engine/settings `20114e99` | default-feature v3_zvec 4 + snapshot 2; feature-enabled prior worker evidence | PARTIAL: release lacks feature/runtime staging, no new feature-enabled test here |
| J memory/dream | existing dream + usage metadata `322cfc01` | usage/prune/schedule extension tests pass | PASS unit scope; no live scheduled dream launched |
| K sheet select | `8abc78b3`, shared regression tests | full collectable frontend suite | PASS scoped; actual Windows/UI QA not repeated |
| L Superagent | dispatcher `1fe3aa36`, UI `dc5d3305`, native bridge in this integration | dispatcher 9 + board 4 + frontend tests + native bridge framing | PARTIAL: native bridge compiled/tested, live GUI/authenticated-host/Windows verification open |

## End-to-end and prefix evidence

### Real-model CLI run: PASS

Used OpenRouter `openai/gpt-4.1-mini` through a local HTTP recording proxy which forwards with the environment API key. Isolated HOME, agent and project directories under `/tmp/oino-v3-integrate-r2/recorded-background`; no owner session data opened. Headers/keys are not captured. Only explicitly selected provider/background packages were loaded, with distribution discovery disabled.

The model successfully executed read, grep, edit, foreground bash, bash `echo background-ok` with `fireAndForget:true`, then `task_output` with the returned task id and `block:true`. Every tool result was non-error, process exit 0, file content became `gamma\nbeta\n`.

Six actual provider request bodies had an identical canonical immutable-prefix byte vector of **11,479 bytes**, SHA-256 **`e2b1d2c7261f0ac834166e44e44b80df25f37b353759f59a56da1547503b9ffb`**. Compared fields are ordered tools plus the first two serialized messages (system and initial context). JSON field serialization is canonicalized by the recorder; this is equality of serialized prefix values, not a claim about arbitrary whitespace in raw HTTP bodies. Summary: `recorded-background-summary.json`. A separate four-tool real run also passed with four matching request prefixes (`recorded-real-summary.json`). An initial background experiment incorrectly used `-ne`, excluding the extension; corrected setup passed and replaced that experiment's result.

### Recorded-provider combined flow: PASS

`crates/oino-core/tests/v3_prefix.rs::integration_discovery_compaction_autonomy_prefix` runs real host session machinery with a scripted provider. It calls tool_search to load deferred `mcp_integration_probe`, executes it, calls forced host compaction, then attempts unattended High `sudo true`. Policy rejects the call before execution with `needs Autonomy High`. Captured Anthropic and completions payloads (`integration-wire.json`, 12 payloads) preserve exact immutable slot text and original declaration bytes, allowing suffix declarations and excluding movable Anthropic cache-control hints. Summarizer requests correctly use a different prompt and are not compared to the main session prefix.

### Real format-v2 replay: PASS

`v3_legacy_replay::actual_format_v2_preserves_source_bytes_on_open_append_and_reopen` opens the actual format-v2 fixture, asserts original bytes identical after open, appends v3 snapshot/reminder, checks original prefix unchanged and reopens without a second snapshot or any rewrite. All six replay tests pass. This closes the Round 1 inspected migration defect. ACP test target also passes, closing Round 1's client-write routing regression.

### Explicitly not proved

Discovery/compaction/autonomy used a recorded fake provider rather than the working real provider. Their real host paths and serializers execute, but there is no single real-model session containing every requested scenario. VCC and LLM both have state/trigger tests; a full live VCC extension roundtrip was not run. Original format-v2 byte preservation is tested independently, not as part of the shared captured network session. Do not claim literal cached-prefix identity across independent sessions, whose initial context intentionally differs.

## Release artifact — do NOT install automatically

Built with `CARGO_BUILD_JOBS=8 UNIPI_RELEASE_IDENTITY=1 bash scripts/unipi-build.sh`, following repository release staging/sidecar/Tauri steps. The script produced deb and AppImage, verified 30 extensions, and did not install or restart the supervisor.

- Deb: `apps/unipi/src-tauri/target/release/bundle/deb/UniPi Oino_1.0.1_amd64.deb`
- SHA-256: **`67cb74a40cc5aa2e99f523e208d5b5634eb98807b0d830588cb9edd99bbbd077`**
- Size reported by bundler: 82.88 MiB. Existing version preserved; no publication/version bump.
- Includes native Superagent bridge and compaction renderer fix. Later edits affect regression tests/report only.
- Exact-only build: semantic CPU feature/runtime delivery remains an explicit packaging gap.

## Evidence retention and next step

Committed log tails, gate exit summary, real-run prefix summaries and combined serializer captures are in `evidence/round-2/`. Full logs, real request bodies/events and recorder scripts remain in `/tmp/oino-v3-integrate-r2/`; preserve before cleanup if needed. The pre-existing dirty checksum manifest was saved there before rebuilding, verified after rebuilding, and left unstaged.

Resolve the 20 root and six host failing targets and strict lint without weakening permission/data-safety checks. Complete live native bridge and all-real-model/VCC scenarios, feature-enabled semantic packaging, and actual Windows/MSVC/PowerShell/process-tree/tray gates. Supervisor owns any installation decision. Overall release gate remains FAIL.
