# App wave 3b — WP-A30 integration, performance gates, QA (A1–A29 + U9–U12)

Date: 2026-10-31 · branch `v2` · HEAD `5391f108` · reviewer: opus-qa (A30).
Spec: `docs/dev/v2/specs/app.md` §5 WP-A30 · rulings `docs/dev/v2/RULINGS.md` · audit
`docs/dev/v2/zcode-audit.md`. Z-67 and Z-105..Z-110 excluded (wave 3c terminal/browser).

**Verdict: NOT green.** Wave 3b has 3 P0s — the app does not boot at HEAD, `src-tauri`
does not compile, and WP-A5 never landed (its worker stalled; the work is parked in
`~/oino-orphan-wip/`) — plus 6 P1 gate failures. All other WPs pass their acceptance rows;
the A27 timeline replacement loses no A6–A8 feature; §3.6 desktop budgets pass.

## 1. Gate results

| Gate | Result | Evidence |
|---|---|---|
| `cargo fmt --all --check` (root) | **PASS** | EXIT=0 |
| `cargo clippy --workspace --all-targets -- -D warnings` (root) | **PASS** | finished clean, no warnings |
| `cargo test --workspace --no-fail-fast` (root, `CARGO_BUILD_JOBS=8`) | **FAIL** | 1516 passed / **8 failed** in 6 binaries (§7) |
| extensions workspace `cargo test` | **PASS** | 623 passed / 0 failed |
| `extensions/unipi/file-summary/harness/run.sh` (A26) | **PASS** | 10/10 |
| `extensions/unipi/memory/harness/run.sh` (A25/G-9) | **PASS** | 3/3 |
| apps/unipi `npm test` | **FAIL** | 245/246 files, **2525/2527 tests**; 2 fail in `src/ui/__tests__/registry.test.ts` (§7) |
| apps/unipi `npm test --workspaces` (protocol) | **PASS** | 16 files / 417 tests |
| apps/unipi `npm run typecheck` | **FAIL** | 7 errors in 3 files (§7 P0-1, P1-1) |
| `cargo check --manifest-path apps/unipi/src-tauri/Cargo.toml` | **FAIL** | 15 errors, `unipi-mobile` lib (§7 P0-2) |
| `npx vitest bench --run` | PASS (with notes) | turns 2 000-turn build p50 **4.02 ms** (≤25), tail update p50 **0.006 ms** (≤0.5), sidebar grouping p50 **0.07 ms** (≤4); jsdom mount 34 ms vs 16 ms browser budget (P2-4) |

## 2. Per-WP acceptance verdicts

| WP | Verdict | Evidence (tests run this pass unless cited) |
|---|---|---|
| A1 prune + HostClient seam + capture harness | **PASS** | `rg herdr\|tmux\|chat_list\|mux_` → comments/placeholders only; `lib/host/*`, `ConnectionState`, `src/dev/mock` present; mock served 31+ ops per capture tab. Note: v1 `NewChatSheet.tsx`/`SessionMenus.tsx` still in tree unreferenced (A5 deletions, P0-3) |
| A2 session data layer | PASS | stream/store/reducer/turns/hub/outbox/selectors + tests green; turns bench above; "Lost the connection · Retry" banner verified live |
| A3 shell + containers | PASS | `panes.test`/`layout.test`/`AppShell.test` green; shell captures render desktop + phone |
| A4 sessions sidebar | PASS | grouping bench 0.07 ms; `sidebar-many` live: 220 sessions, Pinned 3, paging; Z-8/9/10/11/12/14 verified |
| **A5 session actions, archive, draft** | **FAIL — never landed** | **no `[app-a5]` commits**; see §5 (Z-13/15/16 open, Z-19/20 partial) |
| A6 timeline core | PASS | 143 tests green across `src/components/timeline` + `src/lib/timeline`; budgets §4; `tl:first-paint` marks 58–116 ms |
| A7 turn content + actions | PASS | TurnActions/UserBubble/TimeLabel/Navigator tests green; Fork/Retry/Copy/Quote/Edit+undo-files verified in code + tests |
| A8 tool rows + renderers | PASS | bodies (run/edit/write+md/read-search-web-memory heads, todo, mcp, io fallback); `RendererCard.golden` snapshots; collapsed rows mount no body (test) |
| A9 dock, dialogs, crash containment | PASS | BottomDock/DialogPanel/ApprovalPanel/ErrorRow green; approval dialog verified live ("Hide Approval · bash npm test…") |
| A10 side tabs | PASS | Diff/Tree/Btw/Subagent tabs + tests green; "btw never sends to main session" tested |
| A11 queue/steer/stop | PASS | QueuePanel/sendMode/escapeStop green; held queue verified live ("Held · 3 [Resume][Clear]…") |
| A12 composer core | PASS | paste 5119/5120, history, drafts, slash/mention menus green |
| A13 attachments + toolbar + permission | PASS | attachment state machine, ellipse (D-5), D-6 store, compression, PermissionControl (G-6) green |
| A14 model/thinking/context/mode | PASS | 25 tests green this pass (suite total 115 per a14); ContextRing/ModeChip captures render |
| A15 git pane | PASS | GitSection green; git pane verified live ("main · origin/main ↑2 ↓1", Last-turn source) |
| A16 status sections | PASS | StatusSections green; LH/plan/running/capsule captures render |
| A17 session modal | PASS | SessionModal/Context/Running/Info/Limits green; modal verified live (BASH run + subagents) |
| A18 command center + shortcuts | PASS | CommandCenter/FindBar/Shortcuts tests green; Ctrl+K visible in captures |
| A19 notifications/unread/toasts | PASS | needsYou/toasts/deep-link tests green |
| A20 files tab | PASS | FilesTab/FileTree/FileSearch green |
| A21 editor + LSP | PASS | EditorLsp green; Rust `lsp_*` tests green in workspace run (`lsp_servers_lists…`, `lsp_open_streams…` ok) |
| A22 board | PASS | 89 tests green; board live: lanes + dashboard counters |
| **A23 cron** | **PASS w/ gate break** | cron tests + form preview green; **but `src/dev/mock/cron.ts:126` fails typecheck 5× TS2783 (P1-1)** and `unipi-proto` cron fixture test fails (P1-3) |
| **A24 settings** | **PASS w/ P0** | settings tests green, search/appearance/extensions verified live; **`UpdateCard.tsx` import breaks the app boot (P0-1)** |
| A25 harness reachability | PASS | `reachability.test` 27/27; capability pages + memory panel harness green |
| A26 file-summary extension | PASS | harness 10/10 (snapshot/undo/conflict/dedup/kill-resume) |
| A27 turn files footer | PASS | `TurnFilesFooter` tests + reserved body registration; live: "3 files +15 −04 · undone"; A6–A8 regression suite 143 green (§6) |
| A28 empty/motion/tooltips | PASS | a11y suite green; empty state verified live ("Good afternoon!" greeting + lore) |
| A29 phone shell | PASS | phone captures render (this pass: 18 flows at 412×915); perf misses recorded in `qa/app-perf.md` as follow-ups |
| U9 registry/NodeView | **PASS w/ 2 failures** | 12/14 in `registry.test.ts` (P1-2); NodeView/goldens green |
| U10 slots + dialogs | PASS | slot hosts/dialog bodies/status strip tests green |
| U11 settings forms | PASS | form engine tests green (schema/agent/list/advanced) |
| U12 reserved routing | PASS | `reserved.test` 19/19; native git tab + fallback verified live |

## 3. Z-item closure (exceptions only)

Marker sweep + tests verified every Z row of spec §9.1 **except**:

| Z | Status | Evidence |
|---|---|---|
| Z-13 archive | **OPEN** | no `session_archive`/archived list anywhere in `apps/unipi/src`; host `sessions/hub.rs` has no restore/delete/trash ops wired to UI; parked `tracked.patch` carries the unwired hub functions |
| Z-15 rename | **OPEN** | `registerChatAction('rename')` has no caller (only a doc comment, `ConversationHeader.tsx:12`); `session_rename` exists only in dead v1 `lib/chat/store.ts:1142`; meta `titleLock` plumbing exists (A4) |
| Z-16 session row context menu | **OPEN** | no right-click/long-press menu in `SessionRow.tsx`; header ⋯ carries only chat-scoped actions (copy-id/path/log, reveal, info + A25's export/duplicate) |
| Z-19 new chat draft | **PARTIAL** | draft route `#/new/:host?cwd=` + A28 empty state exist; no `DraftView`/`HostPicker`/`FolderPicker`, no per-project `+`, no Cmd/Ctrl+N |
| Z-20 export/delete | **PARTIAL** | Export via A25 `BuiltinActions` (`/export` → `_oino/export_jsonl`); **Delete with confirm absent**; v1 sheets still in tree |
| Z-67, Z-105..Z-110 | excluded (wave 3c) | per instruction |

Everything else (Z-1..Z-12, Z-14, Z-17, Z-18, Z-21..Z-66, Z-68..Z-64 wave rows, Z-69..Z-104,
Z-111..Z-116) has both a code marker and a green test or live capture — see `a30-*` shots and
the per-WP rows above.

## 4. G-1..G-21 reachability

All 21 gaps closed: `lib/reachability.test.ts` **27/27 pass** (walks the audit table as data,
imports each evidence module, asserts route↔table drift empty); G-8..G-11/G-15 additionally
verified by the A25 pages + builtins routing tests; G-9 memory panel harness 3/3; G-21
RendererCard goldens; G-14 UpdateCard exists **but is the P0-1 boot breaker** (surface closed,
integration broken).

## 5. Orphaned WIP (`~/oino-orphan-wip/`) — is anything still open because it never landed?

**Yes — the whole WP-A5 cluster.**
- `tracked.patch` = A5's `hub.ts` archive functions (`loadArchived`/`archive`/`restore`/
  `deleteArchived`/`emptyTrash`, `session_removed` cleanup) + 3 drive-by `data-tip` edits +
  the two **TEMP-A8-CAPTURE re-exports** (`LOCAL_HOST` from `factory.ts`, `syncMascotWindow`
  from `mascot.ts`). None of it landed.
- `untracked.tgz` = `lib/session/actions.ts` (row menu actions, rename/fork/archive wiring),
  `dev/mock/host.fs.test.ts`, `probe.test.tsx`. None landed; `actions.ts` does not exist at HEAD.
- Consequence 1: **Z-13/Z-15/Z-16 open** (§3) because this work never landed.
- Consequence 2 (worse): the parked **TEMP re-exports were load-bearing** — committed
  consumers (`UpdateCard.tsx:26`, `UnicrabSection.tsx:11`) import them, so HEAD does not
  typecheck and the app does not boot (P0-1). The re-exports themselves are the wrong fix;
  the imports should point at `lib/host/client.ts` / `lib/mascotNative.ts`.

## 6. A27 timeline suite vs A6–A8 acceptance (replacement QA)

`ChatView.tsx`, `Transcript.tsx`, `Cards.tsx` deleted (spec §2 rows satisfied); the
`components/timeline/` suite + `lib/timeline/` carry every A6–A8 acceptance row:
- A6: `heightCache`/`bottomLock`/`scrollMemory` tests; `Timeline.test` (live-tail migration,
  fold forced open, jump button gating); virtualizer (`overscan 4`, estimate from cache);
  §3.6 budgets measured (§ below); compaction `/compact` echo hidden; harness/cron rows.
- A7: `TurnActions` (Copy glyph→check 1.2 s Z-93, Quote, Fork→`session_fork`, Retry on last,
  Edit + "Also undo file changes" via `undoFrom`), `TimeLabel` 4 formats, navigator jump
  detaches, selection quote, bubble fold 120 px.
- A8: tool-row anatomy hover chevron, Execute/Explored/Changed groups, all body kinds incl.
  `.md` write preview, thinking row, image chips, `_oino/render` + `renderCache`
  (keyed `(target,key,entryId,rev,width)`, 2 s timeout), golden DOM snapshots, native
  model-change/compaction/harness cards kept.
- Regression count: **143/143 tests green** in the timeline suites; live capture shows turns,
  folds ("Worked for 15s ran 1 command, edited 1"), live tail, footers. No lost feature found.
  Minor: only 2 `a6-`-prefixed desktop shots from A6 itself (mid-stream/unread-dot states are
  covered by the a29/a30 sets instead) — P2-2.

## 7. Defects (prioritised)

### P0
1. **App does not boot at HEAD** (browser + typecheck). `components/UpdateCard.tsx:26`
   imports `LOCAL_HOST` from `lib/host/factory` (export lives in `lib/host/client.ts:30`);
   `screens/settings/UnicrabSection.tsx:11` imports `syncMascotWindow` from `lib/mascot`
   (lives in `lib/mascotNative.ts:42`). Repro: `npm run typecheck`; or open the capture page —
   splash forever, console `SyntaxError: … does not provide an export named 'LOCAL_HOST'`.
   Expected: app boots; actual: dead. Root cause: committed consumers of the parked
   TEMP-A8-CAPTURE re-exports (`~/oino-orphan-wip/tracked.patch`). Fix: point the two imports
   at the real modules (or land A5 which owns the file).
2. **`src-tauri` does not compile**: `cargo check --manifest-path apps/unipi/src-tauri/Cargo.toml`
   → 15×E0425/E0433/E0599 in `src/hosts_commands.rs` (SessionSummary, MuxKind, ListData,
   TreeData, MuxAction, ShortcutsData, AttachTarget, Lines, PreviewData; Request::MuxAction/
   List/Tree/Preview/Shortcuts) — the mux/herdr commands were never pruned when the proto
   deleted those types (D-14 follow-through missing). Blocks every Tauri/desktop build.
3. **WP-A5 never landed** (§5): Z-13/Z-15/Z-16 open, Z-19/Z-20 partial; dead v1
   `NewChatSheet.tsx`/`SessionMenus.tsx` still in tree. All A30 archive/restore flow steps
   impossible. Fix: land A5 from the parked WIP + finish the unwired pieces.

### P1
4. `npm run typecheck` also red via `src/dev/mock/cron.ts:126` — 5× TS2783 duplicate keys
   (`created/updated/enabled/time_limit_s/tz`), A23's mock fixture.
5. U9 `src/ui/__tests__/registry.test.ts` 2 failures: (a) expects `unipi.kanboard/board` but
   commit `98048a42 [sdk-neutral]` renamed the shared fixture to `acme.board/board` without
   updating the TS test; (b) "slot selectors keep their array" — selector array identity
   broken by U10's registry change (regression, both fail standalone).
6. `unipi-proto --test cron_fixtures::shared_cron_fixtures_agree` — "unknown schema CronRun":
   the shared H8 fixture set gained `CronRun` rows; the test's schema match was not extended.
7. `unipi-kanboard` session-cap regression, 4 deterministic failures:
   `board.rs:632 eight_threads_race_but_the_two_session_cap_holds` (5 running vs cap 2),
   `sessions.rs:66 the_session_cap_is_per_project`, `start_finish.rs:148
   a_session_may_start_several_tasks…` and `start_resumes_a_blocked_task…` (unwrap_err on Ok —
   a 3rd session starts). Likely the `[wp-x3]` CLI transport rework (`run.rs:328` reads
   `max_sessions` from the exec env) not feeding the cap on the test path.
8. Known pre-existing failure re-checked — **still failing**: `unipi-host
   sessions::card::tests::state_and_card_wire_shape` (`card.rs:247`). Root cause confirmed:
   linking oino-core enables `serde_json/preserve_order`, so `Value` equality becomes
   key-insertion-order-sensitive; the test compares a serialized struct against a `json!`
   literal written in a different key order (prints identical, compares unequal). Standalone
   repro: `cargo test -p unipi-host --lib state_and_card_wire_shape`. Fix: compare against
   `serde_json::from_value` round-trip or canonicalise keys.
9. ⋯ menu **Rename and Fork rows are dead** (no `registerChatAction` callers); rename is
   unreachable app-wide (subset of P0-3 but a distinct shipped surface — `CHAT_ACTIONS`
   lists rows that never render).

### P2
10. `unipi-host --test instance` races (`racing_serves_leave_exactly_one`,
    `exit_with_parent_follows_the_parent`) fail under the parallel workspace run, pass
    standalone — load-flaky, not regressions.
11. No 2 000-turn scenario exists → §3.6 "2 000-turn open ≤ 120 ms" and "DOM ≤ 3 000 with
    2 000 turns" remain unmeasured end-to-end (data layer covered: bench 4 ms/0.006 ms;
    80-turn session: first-paint 93–116 ms, 760 DOM nodes).
12. Sidebar jsdom mount 34 ms vs the 16 ms browser budget (`sidebar.bench.tsx` documents the
    jsdom interpretation; real-browser row unfilled).
13. `qa/app-perf.md` Desktop section still says "(filled by A30)" — this report holds the
    numbers; fold them in when P0-1 is fixed and the harness runs against HEAD again.
14. A29's Redmi perf misses (scroll p95 150 ms, input ≤86 ms, in-app open ~8 s) remain open
    follow-ups; the session-switch row's blocker (Solid `cleanNode` unmount crash) **did not
    reproduce at HEAD** — switch rendered in 33 ms with no boundary (see §8), so that row
    can be closed once re-measured on the device.

## 8. §3.6 performance (desktop, capture page, real Chromium 1280×820)

Measured on a private headless Chromium via CDP against the capture harness. Because of P0-1
the harness was served from a throwaway copy of `apps/unipi` in `/tmp` with **only** the two
parked re-exports applied (repo untouched); everything else is HEAD.

| Budget | Measured | Verdict |
|---|---|---|
| session open ≤ 120 ms first paint | `tl:first-paint` 93 / 110 / 116 ms (stream-basic / timeline-many / turn-files; 58 ms modal) | **pass** (80-turn scenarios; 2 000-turn row open, P2-11) |
| scroll 60 fps, no long task > 50 ms | p50 **17 ms**, p95 **17 ms**, max 18 ms, **0 long tasks** over a 2 s scripted scroll | **pass** |
| streaming input latency < 50 ms | 12 samples **13–14 ms** input→paint during a live tail | **pass** |
| DOM nodes ≤ 3 000 | **760** nodes, 80-turn scenario (6 turns + live tail mounted) | pass at 80 turns |
| switch sessions ≤ 60 ms | hash→new header **33 ms**, turns render, no error boundary (a29's `cleanNode` crash not reproduced) | **pass** |

Phone rows: unchanged from `qa/app-perf.md` (Redmi Note 7 misses stand as A30 follow-ups).

## 9. Capture scenarios (A30 flow list) — taken and inspected

Driver `docs/dev/v2/design/shots/app/a30-capture.mjs` (+ `a30-report.json`); 36 shots
`a30-<flow>-{desktop,phone}.png` at 1280×820 / 412×915. Flows (app.md §5 A30): new chat
(`empty-chat`), stream (`stream-basic`), steer (`stream-steer`), long session
(`timeline-many`), tool rows (`tools-mix`), file-summary undo (`turn-files-undo`),
queue/steer (`queue-held`), dialog (`dialog-approval`), takeover from `oino` TUI
(`taken-over` → "Taken over from the terminal. Read-only here — take it back… [Take back]"),
Board (`board-lanes`), Cron (`cron-list`), Settings (`settings-nav`), plus sidebar-many,
modal-running, git-last-turn, status-lh-graph, u12-native, cap-memory. Inspection was by DOM
probe per flow (element counts + text; screenshots are the pixel record) — this model cannot
view images. Archive/restore could not be captured: **P0-3**.

All shots render the app (≥40 KB, splash gone, expected DOM present). One caveat for
re-runs: `Target.createTarget(background: true)` throttles the virtualizer — bring the tab
to front before settling (the driver now does).

## 10. Re-verify list for the fixer ( quickest path to green )

1. Fix the two imports (P0-1) → `npm run typecheck` green except `cron.ts` (P1-4) → fix
   fixture keys; re-run `npm test` (registry P1-5 needs the `acme.*` rename + selector fix).
2. Delete/prune `hosts_commands.rs` mux commands (P0-2) → `cargo check` in src-tauri.
3. Land WP-A5 from `~/oino-orphan-wip/` + finish `SessionMenu`/`ArchivedList`/`RenameDialog`/
   draft pickers (P0-3, P1-9) → Z-13/15/16/19/20 close.
4. `cron_fixtures.rs`: add `CronRun` to the schema match (P1-6); kanboard cap (P1-7);
   `card.rs` test canonicalisation (P1-8).
5. Fold §8 into `qa/app-perf.md`; re-measure on HEAD and on the phone.
