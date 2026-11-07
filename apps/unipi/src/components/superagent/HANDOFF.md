# sa-ui r5 — lead model redesign (v3/r5-saui)

Superseded sections below describe the r4 worker pane; the code now follows DESIGN.md.

## What changed
- `api.ts`: lead-shaped `superagent.status` (leads, needsYou, review, projects) normalized in
  one place (`normalizeStatus`); old `workers[]` still parses and renders (legacy leads).
  New calls: `setMode`, `configureProject`, `start(taskIds, presetId)`, `relay{delivery,priority}`,
  `presets.{list,save,remove,setDefault}`. Cap schema is 1–3.
- `state.ts` owns polling (5s), ticket titles, presets and every mutation; created in the page so
  the phone badge works while the sheet is closed. Pane files: `SuperagentPane`, `NeedsYou`,
  `Leads`, `Projects` (+ Start sheet), `Review`, `Presets` (picker + manager), `parts`, `format`.
- `lib/host/tauri.ts`: `superagent.presets.list` → `superagent_presets_list` (all dots, not only the first).
- Mock scenario `?scenario=superagent-leads` (2 leads, 3 approvals, 2 questions, 3 review, 5 projects).
- Tests: `SuperagentPage.test.tsx` (lead model, mutations, legacy), `SuperagentPhone.test.tsx` (badge, sheet tabs).

## Open items
- Native bridge: `src-tauri/src/chat.rs` only wraps status/configure/start(taskId)/stop/relay(taskId)/
  approval. It has no `superagent_set_mode`, `superagent_configure_project`, `superagent_presets_*`
  and its start/relay commands cannot send `taskIds`, `presetId`, `delivery`, `priority`. The new
  controls work in capture/mock and any host client that forwards ops, but not in the Tauri app
  until those commands exist.
- Protocol: no op to accept/reject a review ticket (Review only opens the ticket); relay to an
  unclaimed blocked ticket cannot name the ticket, so replies are sent to the lead with `Re <id>:`;
  `stop` takes one ticket id (the first claimed) and it is unspecified whether it stops the whole lead.
- Reply delivery defaults to `steer` for blocked answers (owner may prefer `queue`).

# sa-ui / OIN-14

## Outcome
Frontend implemented against sa-backend 1fe3aa36 and autonomy UI e73039cd.
Native runtime integration is incomplete because src-tauri exposes none of the
Superagent commands. User required edits to stay inside apps/unipi/src (screenshots
excepted), so no native source was changed. Do not close OIN-14 until this seam is
implemented and verified against an authenticated live host.

## Features
- Sidebar + palette route /superagent; phone Chats header entry and full-height
  workers sheet. Existing shared conversation, buttons, switch, sheets and approvals.
- Workspace-free restricted session stream, remembered separately per computer.
  No autonomy, Plan or filesystem plug controls, including history reopening via
  the persisted oino:superagent marker.
- Host status polling, watcher switch, validated cap, worker task/project/autonomy,
  step/tool/elapsed/cost, exact-id Open, confirmed Stop and explicit Relay.
- Needs you lists blocked/review tickets with wrapped questions and ticket autonomy.
  Opens the exact project via board store. Reviews use existing ticket UI and
  runtime approvals use the existing dock, without a duplicate approval surface.
- Task detail displays effective ticket autonomy (legacy default Medium).
- turns.ts distinguishes explicit authors/provenance from actual user steering.
  Custom relay author is preserved by the existing reducer and shown by event cards.
  Legacy transcript records are never rewritten.

## Protocol / required native follow-up
HostClient uses dot-named host operations. The Tauri transport maps these to
superagent_status, superagent_configure, superagent_start, superagent_stop,
superagent_relay; its new stream kind superagent maps to superagent_session.

Add and register native commands wrapping existing chat::one_shot for
Request::SuperagentStatus/Configure/Start/Stop/Relay. Add the streaming wrapper via
chat::open_session with Request::SuperagentSession and the existing onEvent channel,
stream registry and session_cmd flow. Preserve authenticated host request framing.
No filesystem/shell/session_new fallback is acceptable. Current native execution
will show a visible command-not-found error until this bridge ships. Frontend
capture/test clients implement the same HostClient seam but cannot prove native
registration or host authentication.

## Ownership exceptions
Narrow frontend integrations outside shell/superagent: main route; Chats entry;
phone navigation; HostClient/Tauri op mapping; board Task/Run types and task detail
chip; Conversation/Composer restricted controls; HarnessRow/eventKinds author
labels; capture host/scenario. Ordered turns.ts author edit follows upstream.
No concurrent compaction/Search settings paths, native files or Rust changed.

## Verification
- `cd apps/unipi && mise exec -- npx vitest run src/components/superagent/SuperagentPage.test.tsx src/lib/session/turns.test.ts src/lib/host/tauri.test.ts src/components/composer/AutonomyControl.test.tsx src/lib/phoneNav.test.ts src/components/shell/AppShell.test.tsx src/lib/timeline/eventKinds.test.ts`: 114 passed / 7 files. Required worker_and_needs_you, watcher_and_relay, author_not_steer included.
- `mise exec -- npx tsc --noEmit`: passed.
- `mise exec -- npx vite build`: passed (existing chunk warnings).
- `git diff --check`: passed on frontend changes.
- Impeccable detector: [] on feature and Rail.
- Local logs: /tmp/sa-ui-final-tests.log, /tmp/sa-ui-tsc.log, /tmp/sa-ui-build.log.
- Headless 1440x960 desktop and 390x844 phone, no horizontal overflow (document
  scrollWidth equals viewport), no workspace/autonomy chip. Actual browser relay
  emitted superagent.relay with projectId/taskId/text and displayed delivery notice.
- Screenshots: docs/dev/v3/shots/superagent/{before-desktop,before-phone,
  after-desktop,after-phone,after-phone-workers,after-phone-needs-you,
  after-phone-relay,after-phone-relayed}.png. Before recaptured against unmodified
  HEAD frontend via a temporary read-only Vite transform, after splash dismissal.
  Baseline server 1451 stopped. Existing capture server 1450 reused and left running.
  Image recognition attempted: provider 402. Images inspected directly.

## Risks
Native bridge/live-provider and Windows tray checks remain unverified. Status is
polled every five seconds, with unknown telemetry explicitly unreported. Needs-you
metadata comes from board show; failed metadata reads omit autonomy instead of
inventing a value. Watcher controls use host config, not shared Settings.
