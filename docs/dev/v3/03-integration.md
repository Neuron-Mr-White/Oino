# v3 integration gate — Round 3 (OIN-15)

## Verdict: FAIL — one host browser test remains unstable

Branch `v2`, Linux x86_64, starting at `3e5bb5c5`. Verified all Round 3 commits using `git log --oneline 8d3205e3..HEAD`. Previous reports retained in `03-integration-round-{1,2}.md`. No installation, board writes, stash/reset/clean or supervisor restart was performed.

Only integration fix: `crates/oino-wasm-host/tests/probes.rs::services_bus_and_tasks_between_instances` explicitly authorizes the fixture's harmless platform shell (`sh` / `cmd`). After the task-service security fix, manifest grants alone correctly return `needs Autonomy High`. No production policy changed and no permission assertion was weakened. Probe target: 6 passed. The final full workspace run passes.

## Full gates

All Cargo commands used `CARGO_BUILD_JOBS=8`. Root target started at 26 GB and finished at 27 GB, below the 30 GB GC trigger. Initial free disk 599 GB and available RAM 37 GiB. Separate app/extension caches remain substantial. Counts below sum Rust test-result summaries, including doctests; ignored tests are not counted as passes.

| Gate / command | Result | Counts |
|---|---|---|
| `cargo build --workspace` | PASS | exit 0 |
| `cargo clippy --workspace -- -D warnings` | PASS | exit 0, no diagnostics |
| `cargo test --workspace --no-fail-fast` | PASS after fixture correction | **1002 passed, 0 failed, 1 ignored**, 126 result groups |
| `cargo test --manifest-path extensions/Cargo.toml --workspace --no-fail-fast` | PASS | **672 passed, 0 failed, 6 ignored**, 72 result groups |
| host/kanboard/proto `cargo test --manifest-path apps/unipi/Cargo.toml -p unipi-host -p unipi-kanboard -p unipi-proto --no-fail-fast` | FAIL | **819 passed, 1 failed, 8 ignored**, 81 result groups |
| src-tauri `cargo check` | PASS | exit 0 |
| `cd apps/unipi && npx tsc --noEmit` | PASS | exit 0 |
| `cd apps/unipi && npx vitest run --pool threads --maxWorkers 4` | PASS with skips | **3341 passed, 4 skipped**, 351 files passed / 1 skipped |
| `cargo xtask wasm` | PASS | 37 components, all 100 checksum entries verified |
| Core v3 prefix/replay/autonomy/compaction/codemode/reminders/search tests | PASS | **92 passed** |
| Real background-extension harness | PASS | **5 passed**, including task-service denial before side effects |
| Native Superagent framing/registration | PASS | **1 passed** |
| Release build script | PASS | deb and AppImage; 30 bundled extensions |

Evidence: `evidence/round-3/{counts.json,build.txt,clippy-final.txt,root-final.txt,extensions.txt,host-final.txt,tauri.txt,vitest.txt,release.txt}`. Full logs and exit files: `/tmp/oino-v3-integrate-r3/`.

### Remaining failing target

**P1 integration gate blocker:** `unipi-host --test browser::remote_browser_streams_frames_and_takes_input` fails in both complete host runs. First failure: `apps/unipi/crates/unipi-host/tests/browser.rs:116`, timeout waiting for a frame with Chromium 156 at `/snap/bin/chromium`. Standalone rerun passed (1 test, 10.76 seconds). Final complete run failed at `browser.rs:132`, frame acknowledgement returned `Stopped(0)`. This is intermittent browser/stream behavior, not a confirmed root cause. No timeout inflation, skip or production transport change was made. The full host gate remains FAIL despite the isolated pass.

Four frontend tests skip only because the external Android QA feed is absent. Other ignored tests retain their repository configuration. Actual Windows gates are not covered by Linux passes.

## A–L coverage

PASS means scoped Linux implementation/acceptance evidence; it does not supersede the overall failed host gate.

| Decision | Implemented where / commit evidence | Test | Status |
|---|---|---|---|
| A frozen prompt | resource prompt, core prompt/tools; `9c5b1f3e`, `465a6475`; Round 3 oracles | prefix 48, actual v2 replay, real request recording | PASS scoped |
| B tools/execution | tools filesystem and atomic capabilities `eaa92525`; shared execution `68ae3e5b`; task authorization `e57559fd` | workspace tools, ACP, 5 real WASM background cases; real-model fireAndForget/output | PASS Linux scoped; Windows open |
| C append-only context | session snapshots, provider serialization, per-entry image policy; reopen/continuation fix `4905f105` | recorded prefix, reload/image tests, combined discovery/compaction/deny | PASS scoped |
| D skills | registry/reminder migration `322cfc01` | extension skill tests, reminders/reload/compaction retention, full workspace | PASS scoped |
| E autonomy | host policy `2d3835a8`, option floors `5f06cde5`, task authorization `e57559fd`, UI `e73039cd` | 12 autonomy, shell option regressions, process denial, task-service no-side-effect repro | PASS scoped; real Windows/live GUI disconnect open |
| F compaction | state/frontier/adaptive delta `c53b60e3` | 18 compaction + forced compact in captured combined test | PASS scoped; live VCC roundtrip/catalog limitations open |
| G loop guards | agent loop `bf1a3d2d`, Round 3 reviewed steering golden | guards + full agent/workspace tests | PASS scoped |
| H discovery/codemode | QuickJS and tool_search `54b8025b`, child declaration isolation `5e285596` | 4 codemode + actual deferred load/execute in recorded flow | PASS scoped; documented pi parity limitations retained |
| I semantic grep | optional CPU engine `20114e99`, capability controls/size decision `37479f38` | default-feature acceptance, 5 SearchSettings cases, capability tests | RELEASE OFF by size decision; controls hidden when unsupported; no installed-screen verification |
| J memory/dream | existing dream UI/backend, metadata usage `322cfc01` | full extension tests incl schedule/prune/migration, host memory tests | PASS unit scope; no live scheduled dream |
| K sheet select | existing `8abc78b3` | frontend regression suite passes | PASS scoped |
| L Superagent | dispatcher `1fe3aa36`, UI `dc5d3305`, native bridge `8d3205e3` | dispatcher/kanboard v3 pass, frontend suite, native framing test | PASS scoped; live native authenticated GUI and Windows tray unverified |

The feature-OFF release is now intentional under the supervisor's >40 MB size branch, not an accidental semantic packaging omission. `wp-zvec-release-size.md` records 85,055,680 bytes extra host/native runtime size and unsupported musl ONNX distribution. Search hides toggle/consent/index controls when the host reports unsupported. No semantic runtime libraries or model weights are shipped. Actual installed Search screen was not checked because installation is prohibited.

## End-to-end rerun

### Real model: PASS with explicitly authorized background session

Used OpenRouter `openai/gpt-4.1-mini` through a loopback recording proxy. HOME, agent, project and staged distribution are isolated under `/tmp/oino-v3-integrate-r3/recorded-background`. Only provider and background packages are staged. API key stays in environment/forwarded authorization header, not captures.

Successful calls: read, grep, edit, foreground bash, bash `echo background-ok` with `fireAndForget:true`, then `task_output` with returned task id and `block:true`. All tool results non-error, CLI exit 0, file changed from `alpha\nbeta\n` to `gamma\nbeta\n`.

Five main-model request bodies share the same **11,479-byte canonical immutable-prefix vector**, SHA-256 **`093b33bb36a46128d6f2394af44a58f05fc2437b90245e004d211351e3a25707`**. Compared ordered declarations plus first two messages; JSON values are canonicalized by the recorder, so this does not claim raw HTTP whitespace equality. `recorded-background-summary.json` is durable evidence. A separate four-tool real-model run also passes (`recorded-real-summary.json`).

Initial attempt using explicit non-bundled extension paths and default Medium failed closed (`process.allow:/bin/bash`, then `needs Autonomy High`). Corrected setup uses the release-style bundled distribution and an explicit High ceiling in the isolated agent settings, because WASM process imports conservatively classify High. No production ceiling was raised and the failure was not concealed as a passing run. Recorder scripts are committed alongside summaries for reproduction.

### Recorded provider: PASS

`v3_prefix::integration_discovery_compaction_autonomy_prefix` performs actual `tool_search` loading `mcp_integration_probe`, executes the loaded tool, forces host compaction, then attempts unattended High `sudo true`. It receives `needs Autonomy High` before execution. Both production serializers preserve immutable slot text and original declaration bytes, permitting suffix-only schemas and excluding movable cache hints. Twelve payload captures: `integration-wire.json`. Summarizer uses its own prompt and is excluded from main-session comparisons.

### Format-v2 and bypass regressions: PASS

`v3_legacy_replay::actual_format_v2_preserves_source_bytes_on_open_append_and_reopen` passes: actual version-2 header/source bytes survive open, v3 append and reopen unchanged as a prefix, with exactly one snapshot. All 6 replay tests pass. No owner sessions were opened.

The real background harness executes `task_service_policy_denial_precedes_all_side_effects`: the audit command is refused with no marker, output directory or registry entry. All 5 harness tests pass, including output/stop, completion delivery and wake routing.

### Unverified e2e scope

Discovery/forced compaction/autonomy use a recorded fake provider, not the working real model. No single live-model session spans every scenario. VCC state/trigger acceptance passes, but no live VCC extension roundtrip was performed. Format-v2 preservation is an independent replay test, not the same captured network session. Independent sessions have intentionally different initial context, so no identical-prefix claim is made between sessions.

## Release deb — built, NOT installed

Exact command: `CARGO_BUILD_JOBS=8 UNIPI_RELEASE_IDENTITY=1 bash scripts/unipi-build.sh`.

- Path: `apps/unipi/src-tauri/target/release/bundle/deb/UniPi Oino_1.0.1_amd64.deb`
- SHA-256: **`1fa3c12e99154b1d7d8869b31b62c97769b85015a1b06ce377b711793ff20fe5`**
- Size: **86,931,114 bytes (82.90 MiB)**.
- 30 bundled extensions, feature-OFF lean release identity. Version retained at 1.0.1. AppImage also generated.
- No install, publication or supervisor restart. The only subsequent source change is a test fixture, so runtime binary matches final runtime source.

## Distribution provenance and open gates

The working tree already contained modified workflow WASM/extension manifest and checksum manifest on entry. Saved binary diff: `/tmp/oino-v3-integrate-r3/preexisting-dist.patch`. Rebuilding all components did not change that dirty set; every one of 100 working-tree checksum entries matches disk. These pre-existing artifacts are intentionally **not staged or committed by this integration task**. The deb contains this verified working-tree distribution, so it is not reproducible from the integration commit alone until the distribution owner reconciles/commits those files. This is an explicit shipment/provenance gate.

Next: investigate the browser frame/ack instability and rerun the full host gate. Coordinate dirty workflow/checksum ownership. Complete real Windows/MSVC/PowerShell/process-tree/tray acceptance, live native GUI transport and remaining live-model/VCC checks. Do not treat passing Linux workspace tests or the built deb as full release approval. Supervisor owns installation.
