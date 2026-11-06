# wp-autonomy-ui / OIN-12

## Outcome
Implemented against wp-autonomy `2d3835a8`. Autonomy chip cycles Low → Medium → High → Low with exact tooltips; Plan remains separate. Host approvals render once/always/deny in the existing dock. TUI reuses its existing approval dialog, sends typed correlated responses concurrently, maps Escape to deny and opts out on teardown.

## Integration
- `apps/unipi/src/lib/session/turns.ts` owns branch-local render-cutoff selection, including legacy anchors. The divider is before the inclusive kept turn; old turns remain available. Ready for sa-ui's ordered author/isSteer edit.
- Settings › Autonomy saves global `autonomy.defaultLevel`. No speculative permission-rule editor was added: the binding spec defines the storage envelope, not a rule-editing UX.
- Settings hooks use `import.meta.glob` for downstream-owned `CompactionSettings.tsx` and `SearchSettings.tsx`. Each must export a **default Solid component**. Sections appear on desktop/phone only when the file exists. Existing dream settings are unchanged.
- Foreground child approval events carry the child's sessionId on the parent's event channel. The parent channel is authoritative for responses, as demonstrated by upstream `foreground_child_approval_routes_to_parent_and_background_revokes_it`; clients must not reject the child's id.

## Ownership exceptions
Narrow frontend transport/reducer/store/draft/dock/timeline integrations, capture fixture/config, plus `crates/oino-client/src/{protocol,in_process}.rs` and its existing GUI integration test. The app host parses typed RpcCommand records and could not consume the upstream JSONL-only command fast paths. Typed variants bridge set_autonomy/approval_response, and get_state exposes the effective ceiling. No files under `apps/unipi/crates` or kanboard edited or staged.

## Verification
- Scoped Vitest: **91 passed in 9 files**, including all three named acceptance cases, child routing, duplicate decisions, settings errors, legacy Plan behavior, turns/store/reducer and Sheet select regression.
- `cd apps/unipi && npx tsc --noEmit`: passed.
- `cd apps/unipi && npx vite build`: passed (existing chunk/dynamic-import warnings).
- `cargo test -p oino-tui --test autonomy --test ui_keys`: **13 passed**.
- `cargo test -p oino-client --test in_process_gui typed_autonomy_and_approval_bridge`: **1 passed**, real native command parser, ceiling, pending deny and duplicate rejection.
- Client protocol unit cases: **3 passed**.
- `cargo clippy -p oino-tui --no-deps -- -D warnings`: passed.
- Client local clippy passes excluding the three pre-existing unused-async/needless-pass-by-value lint categories documented upstream. Strict dependency-inclusive clippy was retried, but stopped in concurrently edited `oino-session/src/manager.rs:729` (collapsible_if); earlier attempt stopped in concurrent core prompt code. Neither file was changed here.
- Impeccable detector over changed feature UI: `[]`.
- `git diff --check` on owned paths passed. Resource checks: 41 GiB RAM available at start, target stayed 27 GiB (below 30 GiB cleanup threshold).

Logs: `/tmp/autonomy-{vitest-final,tsc-final,build,rust-final,bridge-final,client,clippy-local,clippy-retry,client-clippy-filtered}.log`.

## Screenshots
Committed under `docs/dev/v3/shots/autonomy/`: before-desktop/phone, after-desktop/phone, after-phone-high, after-cutoff-desktop, settings-desktop. Baselines captured with the HEAD PermissionControl supplied through a temporary dev-only Vite loader (removed afterward), after splash settlement. After captures use the explicit `scenario=autonomy` fixture. Browser checked 1280px desktop and 390×844 `shell=phone`: no horizontal page overflow, 40px approval buttons and chip, all decisions visible. Browser interaction verified ceiling command and correlated deny payload. Images inspected directly; `image_recognize` was attempted but provider returned billing error 402.

## Risks / open questions
- Actual Windows execution unavailable.
- App dock opts out on normal unmount and clears approvals on stream loss. Abrupt app/network disappearance still relies on the upstream 300-second fail-closed timeout: the existing app host subscription Drop does not revoke approval capability immediately. This is in concurrently owned `apps/unipi/crates/unipi-host/src/sessions/stream.rs`, left untouched. Host owner should revoke approvals when its last approval-capable subscriber disconnects, and test abrupt disconnect. No command is authorized by disconnect.
- Multiple frontends answering the same request are rejected by core single-use correlation. There is no host approval-dismiss event for another frontend's decision; stale local cards can return a visible error until run end/reset.
- No deployment requested or performed. Capture server on port 1450 remains running for inspection; temporary baseline server was stopped.
