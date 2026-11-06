# wp-autonomy / OIN-11 handoff

## Implemented

- `oino-settings::autonomy`: ordered Low/Medium/High (Medium default), shared conservative recursive bash floor and PowerShell floor, validated version-1 permission envelope and deny → legacy deny → ask → allow → default resolution.
- Core `session::permission`: host-owned gate after legacy normalization; canonical file destinations and parsed patch source/move destinations, effective risk max(declaration, floor), exact refusal `needs Autonomy High`. Write/edit denies and asks also apply to patches.
- Calls with no approval transport fail closed immediately. Merely having UI, declared tools, child profiles or manifest process grants is never consent. Legacy calls get High through existing normalization; public canonical policy calls reject missing/malformed risk.
- Direct, nested/codemode, legacy aliases, background shell launch, user-shell execution and WASM process exec/spawn traverse policy. Extension input rewrites are checked again. WASM argv process execution conservatively declares High.
- Child ceiling is bounded by the parent on creation and dynamically on each check. Foreground child requests go to the parent's approval channel. Background children are unattended; foreground → background emits a restrictive host signal. Unattended state and session ceilings append compatibility entries rather than modifying historical records.
- Single-use UUID approvals, wrong-session/stale/duplicate rejection, cancellation/disconnect handling, deny recheck after approval, exact command grants atomically appended under the project settings lock. Exact grants are interactive/project-only and do not raise autonomy. A configured shell command prefix disables exact-grant reuse/persistence conservatively.
- Workflow no longer performs permission approval, profile-based substitution or jev judging. Old transcript/state decoding is retained. Plan remains independently enforced, with PowerShell and patches conservatively refused while planning.

## Frontend / dispatcher integration API

Native authenticated frontends use `SessionBackend::set_autonomy(Level)`, `set_approval_ui(bool)` and `approval_response(ApprovalResponse)`.

RPC:

- Explicit capability: `_oino/hello` with `ui.approvals:true`. Generic `has_ui` / protocol-v2 capability does not opt in.
- Host event: `approval_request {requestId,sessionId,tool,arguments,riskLevel,ceiling,reason}`. Non-shell requests have null riskLevel.
- Client: `approval_response {requestId,decision:"allow_once"|"always"|"deny"}`. Session binding comes from the backend receiving the record. Optional RPC `id` receives a success/error reply.
- Frontend ceiling update: `set_autonomy {level:"low"|"medium"|"high"}`.
- Disconnect opts out and drops all pending requests. TUI and other frontends remain unattended until they actually implement approval UI and opt in.

ACP maps these events directly to `session/request_permission`, with allow_once / always / deny options. Cancelled/unknown replies deny.

Host dispatcher/cron must call `AgentSession::set_unattended()` before starting work. Set worker ticket ceilings through `set_autonomy`; cron leaves the global default. These host-owned integration files remain assigned to sa-backend. Core tests use fake cron callers per spec.

## Verification

All commands use `CARGO_BUILD_JOBS=8`.

- `cargo test -p oino-core --test v3_autonomy --test v3_codemode --test v3_prefix --test v3_reminders --test v3_legacy_replay`: 71 passed (12 autonomy, 4 codemode, 47 prefix, 2 reminders, 6 replay).
- `cargo test -p oino-acp --test acp`: 16 passed, including real autonomy request_permission transport regression.
- `cargo test -p oino-rpc --test autonomy --test ui_wire`: 3 passed, including real JSONL capability negotiation, correlation and disconnect.
- `cargo test -p oino-tools --lib --test v3_fs --test v3_exec`: 57 passed.
- `cargo test -p oino-settings permission_grant_tests`: 1 passed, verifies independent settings managers do not lose concurrent grants.
- `cargo test --manifest-path extensions/Cargo.toml -p unipi-workflow -p unipi-subagents`: 43 passed.
- Strict clippy clean: oino-settings, oino-tools, oino-core, oino-extension-api, oino-wasm-host; extension unipi-workflow and unipi-subagents.
- Strict client/RPC/ACP clippy is blocked by existing `oino-client/src/hub.rs:620` unused-async diagnostics and `in_process.rs:85` needless-pass-by-value. Recheck with only those lint categories excluded passes. No unrelated baseline fixes made.
- `cargo xtask wasm` completed for all 37 components; final scoped `cargo xtask wasm -p unipi-workflow -p unipi-subagents` completed. Only workflow/subagents artifacts and their checksum entries belong to this change.
- `git diff --check`: passed.

Local verification logs: `/tmp/wp-autonomy-final-core.log`, `/tmp/wp-autonomy-final-acp.log`, `/tmp/wp-autonomy-rpc.log`, `/tmp/wp-autonomy-tools.log`, `/tmp/wp-autonomy-settings.log`, `/tmp/wp-autonomy-ext-tests.log`, `/tmp/wp-autonomy-clippy.log`, `/tmp/wp-autonomy-ext-clippy.log`, `/tmp/wp-autonomy-wire-clippy.log`, `/tmp/wp-autonomy-wire-clippy-filtered.log`, `/tmp/wp-autonomy-wasm-final.log`.

## Ownership exceptions

Narrow integration edits in oino-client backend interfaces, core agents/services/events/user-shell/nested execution, tools patch parser/path exports, extension API and WASM process imports. These are needed to prevent alternate execution paths bypassing the host gate. No host cron/dispatcher or frontend rendering files edited.

## Limitations and conservative choices

- No actual Windows host was available. The Windows-only acceptance case exists; platform-independent PowerShell classifier cases execute on Linux too. Actual Windows execution remains an open release gate.
- Unknown shell syntax, heredocs, eval/xargs/find-exec and most PowerShell invocation forms classify High, rather than attempting permissive interpretation. Canonical exact grants retain quoting/adjacency and normalize only inter-token whitespace. Unparseable commands cannot be persisted.
- Native extension process imports conservatively use High for the whole argv invocation. This may require more approvals for internal extension CLI calls; it never uses manifest availability as approval.
- Legacy workflow settings remain readable but do not elevate the new Medium default. Legacy denies in global/project extension settings are considered case-insensitively so migration does not weaken old matching semantics.
- This is permission enforcement, not an OS sandbox. A permitted shell command can execute project scripts or configured build hooks. Policy floors cover supported syntax conservatively; a fresh adversarial bypass audit is still required.
- Client/RPC/ACP strict clippy baseline and actual Windows gates prevent claiming fully clean package closure under the worker-rules definition.
