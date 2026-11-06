# sa-backend / OIN-13 handoff

## Implemented

- `unipi-host/src/superagent.rs`: machine-wide kernel advisory lock (`File::try_lock`, held for owner lifetime), shared watcher/manual async transaction, global slot reservations, one worker per project, durable `Run.session` before launch, exact-id create/attach. Watcher defaults off, cap defaults 1. Lowering cap does not stop workers.
- Restart reconciles local dispatcher claims before accepting dispatch. Missing runtime becomes blocked with exactly `Worker was interrupted; move to todo to retry`; never automatically requeues. Worker exit without transition blocks with its last assistant text. Kanboard reaper and `release --to todo` cannot requeue dispatcher claims. Same-session `start` is idempotent, including dispatcher ownership.
- Real workers use the public sticky unattended API and ticket autonomy (legacy/new unspecified = Medium). A native `kanboard` tool supplies the exact raw reserved session identity, avoiding the old extension's `oino-<id>` session tag and shell approval requirements for board operations.
- Explicit relay validates the current dispatcher claim, saves a system-author comment, sends a custom follow-up to precisely that runtime, and reports `delivered:false` plus a stable delivery reason on partial failure. Custom messages carry author and provenance, so backend relays are not synthesized as user steering messages.
- `superagent_session.rs`: no cwd; no workspace snapshot, context files, ordinary extensions, filesystem, shell, subagent or codemode tools. Only cross-project kanboard, sidebar/zoxide project resolver, start/stop/relay and ask_user. Provider components remain available. Core policy also rejects nonallowlisted tools and cwd changes. Reload uses the same restricted factory. Normal frozen v3 system-prompt/prefix machinery is retained.
- Removed command registration and handlers for `/kanboard-do` and `/kanboard-autowork`; historical renderers and decoding retained. Updated skill/help/manifest. Rebuilt scoped WASM and native board binary.
- Cron print/JSON execution calls `set_unattended()` before executing prompts, keeping the global default ceiling.

## Host protocol for sa-ui

Normal authenticated host framing is unchanged. Operations:

- `superagent.session {}` opens a restricted Superagent session stream (same stream protocol as session_new).
- `superagent.status {}` → `{enabled,concurrencyCap,workers:[{projectId,taskId,sessionId,autonomy,step,tool,startedAt,cost}],needsYou:[{projectId,taskId,status,question}]}`. Unknown live telemetry is null, never fabricated.
- `superagent.configure {enabled,concurrencyCap}` → effective config, persisted under `config.json.superagent`.
- `superagent.start {projectId,taskId}` → `{sessionId}`.
- `superagent.stop {projectId,taskId}` → `{stopped}`.
- `superagent.relay {projectId,taskId,text}` → `{commentId,sessionId,delivered,error?}`. Comment persistence can succeed while delivery fails.

Refusals use the existing `E_BOARD_REJECTED` envelope with a stable dispatcher reason in `detail.code` and an actionable message. The Superagent session exposes no autonomy UI field; sa-ui must hide the chip for this session kind. No frontend/TUI files changed in this package. Existing positional isSteer author rendering remains sa-ui integration work; backend relay messages are custom-role with `details.author=superagent` and `details.oinoProvenance`.

Kanboard create/edit now accept `--autonomy low|medium|high`; show/list/board task JSON includes the effective default Medium. Stored task autonomy remains optional for legacy compatibility. `Run.owner=dispatcher` is the durable ownership marker, preserving all existing Run fields.

## Verification

All commands used `CARGO_BUILD_JOBS=8`.

- `cargo test --manifest-path apps/unipi/Cargo.toml -p unipi-host --test v3_dispatcher`: 9 passed, including required race/project-cap/crash names, real subprocess death/OS-lock release, cap lowering, last-message blocking, restricted reload, and real faux-provider worker exact-id/unattended/idempotence/author relay.
- `cargo test --manifest-path apps/unipi/Cargo.toml -p unipi-kanboard --test v3_kanboard`: 4 passed (required reaper/idempotence/default/relay names plus autonomy CLI and wrong-session terminal transitions).
- Full kanboard suite passed (all integration binaries); extension unit suite 69 passed.
- Standalone real WASM harness: 6 passed using `CARGO_TARGET_DIR=<repo>/target cargo test --manifest-path extensions/unipi/kanboard/harness/Cargo.toml -- --test-threads=1`.
- Core `v3_autonomy` + `v3_prefix`: 12 + 47 passed before the parallel compaction edits.
- Cookie import regression tests: 10 passed after aligning rusqlite to 0.37 for the parallel zvec dependency's sqlite links requirement.
- Strict clippy passed kanboard, extension kanboard, core/session/API/WASM before concurrent compaction edits. Host/proto clippy passes with only `clippy::large_enum_variant` excluded (pre-existing proto session HubEvent). CLI strict clippy remains blocked by existing client unused-async/pass-by-value diagnostics.
- Scoped `cargo xtask wasm -p unipi-kanboard` completed; only this package's artifacts and checksum keys included.
- Additional existing host sessions_hub suite: 6 passed, `fork_keeps_the_original_wired_to_its_own_runtime` failed message-count equality (3 vs 4) during concurrent compaction edits. Not repaired outside this scope.

Local logs: `/tmp/sa-dispatch-tests.log`, `/tmp/sa-kan-v3.log`, `/tmp/sa-kan-tests.log`, `/tmp/sa-ext-tests.log`, `/tmp/sa-harness.log`, `/tmp/sa-wasm.log`, `/tmp/sa-clippy.log`, `/tmp/sa-clippy-filtered.log`, `/tmp/sa-core-only-clippy.log`, `/tmp/sa-cookie-tests.log`, `/tmp/sa-hub-tests.log`.

## Ownership exceptions

Narrow integration in host main/session hub/stream and proto request enum; core prefix/no-cwd/policy guards; session manager `materialize()` for durable restricted draft identity; CLI print unattended entry point; WASM Composite process-check forwarding and explicit test-recorder process authorization (default stays deny). Host rusqlite aligned to 0.37 because optional zg-engine introduced a conflicting sqlite links version. Cookie regressions pass. Shared manager file staged only the materialize hunk, preserving parallel compaction changes. Concurrent host Cargo feature additions, root/app lockfile updates and other workers' source changes left unstaged.

## Open gates / conservative choices

- Linux verification only. `windows_tray_dispatcher` is cfg(windows); a real Windows tray run remains required before closure.
- No provider-network end-to-end run or actual UI screenshots. Real runtime integration uses a faux provider; restricted tool/cwd/reload assertions execute natively.
- Reviewer should re-run host tests/clippy after parallel compaction/zvec packages settle. Current shared core clippy failures include their in-progress compaction changes, not fixed here.
- Superagent config mutates through `superagent.configure`; generic host settings writes are persisted but do not hot-reload the dispatcher's in-memory config.
- Live runtimes are attached by exact session id/path. Completed workers retire after transition/exit; opening their saved chat later uses normal session history.
