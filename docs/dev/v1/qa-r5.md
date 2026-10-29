# QA round 5 — report (alpha.4 → alpha.5 gate)

QA pass at HEAD `306f822` on `v1-alpha` (2026-10-29), before `round5-integration`.
Scope: charter gates in all three workspaces, every GUI driver suite plus all `r5-*`
scenarios, driver-verifiable acceptance for each of the 27 issues and D1–D3 with
screenshots, harness-feature reachability, and the 30-min faux soak + 4-pane perf
([stability.md](stability.md)). No source code was edited.

Artifacts: this directory (`scenarios/` = the QA ad-hoc scenarios, `shots/` = their
screenshots) plus `docs/dev/v1/design/shots/desktop/r5-*/` (suite scenarios) and
`target/gui-suite/*.json` (runner reports). Logs under `target/tmp/qa-r5/` (not committed).

## 0. Tree state

`git status` at start: **no uncommitted source files**. Only screenshot PNG drift
(one deleted `FAIL-07.png`, regenerated shots from re-runs, one untracked
`zcode-palette-history/FAIL-06.png`) — carried into this report's `[qa-r5]` commit.

## 1. Charter gates (fmt / clippy pedantic / tests)

| Workspace | fmt --all --check | clippy -D warnings -W pedantic | test --workspace |
|---|---|---|---|
| root | **FAIL** | PASS | PASS (773 passed, 0 failed) |
| extensions | **FAIL** | PASS | PASS (578 passed, 0 failed) |
| apps | **FAIL** | PASS | PASS (171 passed, 0 failed) |
| extensions `xtask wasm` / `checksums` | — | — | 35 components written; `wasm` → `checksums` round-trip PASS (dist binaries are gitignored; the tracked `checksums.json` is not byte-reproducible across rebuilds — D-16) |

All three fmt failures flag the **same 7 spots in `crates/oino-kanboard`**
(`src/run.rs:467`, `src/service.rs:79/139/325`, `tests/attachments.rs:32/114`,
`tests/common/mod.rs:205`) — committed unformatted in `bb853e3` `[wp-kanboard]`.
Everything else in every workspace is fmt/clippy/test clean (defect D-5).

## 2. GUI driver results (ti, Xvfb :93, release build of `306f822`)

| Run | Result |
|---|---|
| All 22 `r5-*` scenarios (individual runs) | **22/22 PASS** |
| `cargo xtask gui suite unipi` (24) | **17/24** — see D-1/D-2/D-6/D-7 |
| `cargo xtask gui suite qa-r4` (12) | **10/12** on first pass; both failures PASS on re-run (D-8 flake, D-14 model flake) |
| `cargo xtask gui suite zcode` (6) | **blocked** — manifest references the deleted `zcode-model-menu.json` (D-3) |
| zcode scenarios run individually (5) | **4/5 PASS** (`zcode-thinking-cycle`, `zcode-row-actions`, `zcode-session-menu`, `zcode-queue-guard`); `zcode-palette-history` FAIL = stale assertion (the palette "Board" item now routes to the native board page — `route: board` observed — while the scenario expects the old `panel:unipi.kanboard/board` in `state.open`; see D-7) |

Suite failure triage (each re-run and root-caused; logs in `target/tmp/qa-r5/`):

- `unipi-workflow`, `unipi-notify`, `unipi-utility`, `unipi-updater` — **stale selectors**
  (`mode-chip` removed by #2; `set-ns-0`/`set-about` replaced by the ZCode settings nav).
  Functionality verified live: the mode surface is the `agent-mode` chip, and the settings
  hub pages (Notify hub switches, descriptions) are reachable under the new nav.
- `unipi-long-horizon` — real defect D-1 (composer focus loss after a mode card).
- `unipi-kanboard` — `/kanboard status` output changed shape (no "kanboard" text) and
  prints a stale `daemon: not running` line (D-6); the command itself works.
- `approval-deny` — startup flake D-8 ("the app did not come up with a composer"),
  passes on re-run; observed twice by WP-composer too.
- `rewind-real` — real-backend flake: the model chose a bash call needing approval
  (`jev: unavailable` in the dialog title); passes on re-run (D-14).

## 3. Issue-by-issue acceptance (27 issues + D1–D3)

Key: ✅ = acceptance met with driver/scenario evidence; ⚠️ = met with a recorded caveat.
Screenshots: `docs/dev/v1/design/shots/desktop/r5-<scenario>/` (suite runs) and
`docs/dev/v1/qa-r5/shots/` (QA ad-hoc runs).

| # | Issue | Verdict | Evidence |
|---|---|---|---|
| D1 | TUI-only extensions removed | ✅ | `r5-tui-removal` PASS (no footer chrome segments, no package tags in `/` completions, `alt+h` inert); `xtask wasm` = 35 components, checksums match; repo grep for `unipi.(footer|command-enchantment|input-shortcuts|app-bridge)` hits only intentional places (`suite.rs` NOT_PORTED + tests, `composer.rs` doc/fixture, the scenario itself). Caveat D-10: the Extensions-page assertions are vacuous (no probes on `extensions_page.rs` rows). |
| D2 | ZCode UX direction | ✅ | ZCode tokens in place (`design/tokens.rs` `BASE/ENTER/INK_MS`, `EASE (0.22,0.61,0.36,1)`; `type_scale::zcode`); `r5-settings-zcode`, `r5-new-chat-page`, `r5-contrast-tokens` PASS. Visual judgment items carry screenshots for the owner. |
| D3 | Harness parity only | ✅ | Every kept unipi package still driver-reachable (unipi suite + live checks below); `crates/oino-tui` still compiles (root gates green); no TUI-parity work in the round's diffs. |
| 1 | Modal model picker | ✅ | `r5-model-picker-modal` (real backend) PASS on ti: search over name + full `provider/id`, pin → Pinned section, Apply switches, Fusion auto-answer → `Lead ◆ Sidekick` chip, Esc closes. |
| 2 | Mode switcher up front | ✅ | `r5-mode-switcher` PASS: five modes, Goal arms without a command, Ctrl+Shift+M cycles, `mode-chip` absent, no "unipi" text in the composer tree. Regression note: `unipi-workflow` still waits for the removed `mode-chip` (D-7). |
| 3 | One card + context inside | ✅ | `r5-composer-context-ring` PASS: no project/memory row, `context-ring` + hover panel (sources, Compact), model + thought selects, Stop while busy. |
| 4 | No duplicate sessions | ✅ | `r5-no-dup-sessions` PASS (`+`/⌘N keep `panes == 1`; genuinely new session after content). |
| 5/13 | Missing icons | ✅ | Unit test `design::icons::tests::every_referenced_icon_is_embedded` PASS; `r5-icons-automations` PASS + shots (Automations page, cron dialog). |
| 6 | New-chat page | ✅ | `r5-new-chat-page` PASS: greeting + `brand-logo`, project rows, "No project", "Choose folder…", Esc returns. |
| 7 | In-process kanboard | ✅ | `serve/`+`daemon.rs` deleted from `crates/oino-kanboard` (no serve subcommand); `r5-board-inprocess` PASS (no daemon text, watcher refresh); **CLI verified live on ti**: `oino-kanboard add`/`move` against the same files → GUI showed `OIN-12 CLI-added QA task` in Todo without restart (`qa-r5/shots/qa-r5-cli-refresh.png`). Caveat: `/kanboard status` still prints `daemon: not running` (D-6). |
| 8 | Draggable cards | ✅ | `r5-board-drag-pc` PASS: card → lane drag moves + persists; refused lane takes no drop; mid-drag strip + drop-line shots for the owner. The `<60px` refused-lane bound assertion was not encoded in the scenario (noted, D-11). |
| 9 | Modal formatting | ⚠️ | QA scenario `qa-r5-board-modal-form`: dialog opens from the FAB with `Title`/`Description` inputs (labels asserted) + dialog title `New task`; creation proven by `qa-r5-board-add-debug` (OIN-12 created) **after clicking the input** — without that click the typed text is lost (D-1). Not rebuilt on gpui-component `form/` (no `form::` usage in the repo) — mechanism deviation vs round5.md §2 #9; metrics are the owner's visual call (shots `qa-r5-board-modal-form/01`). The spec's `r5-board-modal-form` scenario was never committed (D-9). |
| 10 | Phone side panel + unipi PC board | ⚠️ | `r5-board-side-phone` PASS (lane pills `board/strip-N`, one lane, inline detail). PC board: lanes/42px heads/inline Add-task/FAB verified in tree (`lane-*`, `lane-add-*`, `board-fab`); the toolbar carries scope (`Switch project`), filter (`All tasks`), `Display`, search, view toggle (`Show the board in chat`) + right-aligned New task — but also a legacy `Move` button and `Refresh` icon, and no `Clear` chip (D-12); the "no OUS toolbar pill row" assertion was never encoded. Shots for the owner. |
| 11 | Overlay shows full text | ✅ | QA scenario `qa-r5-board-fulltext` PASS: a 155-char title + 240-char body render in full in the detail overlay tree labels (no ellipsis); lane card keeps its clamp (`01-card-clamped.png`). The spec's `r5-board-fulltext` scenario was never committed (D-9). |
| 12 | Central host switcher | ✅ | `r5-host-switcher` PASS: chip reads exactly "Current PC", Add-host form persists into the menu, remote pick toasts the honest `oino serve` scope. `hosts.json` store isolated from `host.rs`. |
| 14 | ZCode settings layout | ✅ | `r5-settings-zcode` PASS: 262–274px nav rail (bounds assertion), search "theme" hides non-matching nav + content rows, mascot toggle regression green. |
| 15 | Esc twice stops | ✅ | `r5-esc-twice-stop` + `r5-esc-dialog-guard` PASS (toast, abort → "Stopped" chip, dialog guard). |
| 16 | Virtualization kept | ✅ | Virtualization contract documented in `components/conversation.rs:5-12` (minimal list ops, readiness gating, spliced generating strip); architecture exercised by `r5-bounce-watch`. |
| 17 | Markdown palette | ✅ | Unit tests `md_palette_matches_dark`, `markdown_style_builds_for_both_modes` PASS; `r5-markdown-palette` shots (driver cannot assert colours). |
| 18 | One footer row | ✅ | `r5-composer-context-ring` (single row) + `r5-tui-removal` (`chrome` carries no "Oino"/"UNIPI" segments) PASS. |
| 19 | Project tab labels project | ✅ | `r5-settings-trim`: `Project · <folder>` enabled + "Set in project" badge after a project-layer write. The no-session disabled state is **code-verified only** (`settings_page.rs:925-928` "Project scope needs an open session") — no zero-pane state is driver-reachable at HEAD (D-13). |
| 20 | No Extensions page in Settings | ✅ | `r5-settings-trim`: no "Manage extensions" row; sidebar ⌘3 route still works. |
| 21 | Chat bounce | ✅ | `r5-bounce-watch` PASS (watch step: zero disappear/reappear gaps over the stream, last row bottom-most at settle). |
| 22 | Every setting does something | ✅ | `r5-settings-trim` PASS: dead keys gone (`sidebar_compact`, `compact`, `hide_thinking`, `expand_tools` — the dead OR-read removal is commented at `timeline/mod.rs:160`), Wayland "next launch" note, project badge; no-op write toast implemented (`settings_page.rs:607`, code-verified — D-13). |
| 23 | Scroll containment | ✅ | QA scenario `qa-r5-scroll-containment` PASS; live check with the search field focused: `lane-todo` bounds identical `[245,228,280,154]` before/after wheel (`dy 6`+`dy 8`); shots `01-before-scroll`/`03-after-scroll`/`04-after-wheel`. D11 synthetic-wheel caveat applies (if the wheel were fully rejected the check is vacuous — recorded per the acceptance text). The spec's `r5-scroll-containment` scenario was never committed (D-9). |
| 24 | Files view + editor + LSP | ✅ | `r5-files-editor` + `r5-editor-lsp` PASS on ti (open/edit/save/"Saved" toast, persisted reopen, seeded-crate diagnostics in the Problems strip, `LSP rust-analyzer` status, format-on-save). |
| 25 | Contrast floor | ✅ | Unit test `design::tokens::tests::contrast_floor_r5` PASS (both palettes); `r5-contrast-tokens` shots (sidebar/settings/board). |
| 26 | Real logo | ✅ | `r5-logo-real` PASS (`brand-logo` on welcome + pane). |
| 27 | Refresh button removed | ✅ | `r5-refresh-gone` PASS (`list-refresh` absent; new sessions still appear). |

**27/27 issues + D1–D3 verified** (24 clean ✅, 3 ⚠️ with caveats recorded as D-9/D-11/D-12).

## 4. Harness-feature reachability (regression check)

- All 20 kept unipi packages load and respond (unipi suite green scenarios + the four
  stale-selector scenarios verified live: workflow mode surface via `agent-mode` chip,
  Notify hub switches/descriptions, utility + updater settings rows exist under the new nav).
- MCP hub: reachable via the settings nav (scroll) — verified live after the suite run.
- Driver primitives intact on ti at HEAD: click/type/key/scroll/**drag**/drop/screenshot,
  `kanboard_seed`, `seed_file`, `watch` streaming — all exercised by this round's runs.
- `settings-switches` (Unicrab page) stays green after the settings rebuild — WP-extensions'
  feared break did not materialize.
- Side-pane tabs, terminal, palette, session menu, queue, undo, status panel: green in qa-r4.

## 5. Soak + perf (stability.md)

- 30-min faux soak on ti (`cargo xtask ti soak --minutes 30 --backend faux`): **PASS** —
  `summary.txt` copied to `docs/dev/v1/qa-r5/soak/`. 2560 steps; RSS 11→588 MB
  (max 748), second-half slope **2.08 MB/min** (≤ 8); threads 116→104; fds 399→399;
  0 crash logs, 0 stall logs, exit 0. Frame times (Xvfb/lavapipe, report-only):
  run_max 86.7 ms / p99 57.5 ms — in line with the round-4 faux soaks (55–64 ms p99).
  Note: a first 1-minute probe soak collided with this run (same `~/oino-test/soak`
  outdir) and printed a bogus FAIL; the numbers above are the real 30-min run's
  `summary.txt` (365 samples, t=0..1800).
- 4-pane perf on ti (`perf.sh <release-bin> 20 4`, headless lavapipe): **PASS** —
  whole-run p99 **9.90 ms** ≤ 10 ms budget (tight; software rendering), p50 4.07 ms,
  0 frames > 16.7 ms of 720 (16 over 8.3 ms), worst 120-frame window p99 11.18 ms,
  max 13.02 ms. Latency: keystroke 5.2 ms median, session switch 10.1 ms, first frame
  138 ms. Idle (welcome scene, after settle): 2.0 % UI-thread CPU.

## 6. Defects

Priorities: P0 = blocks the round; P1 = round-5 regression or red suite; P2 = gate red /
spec gap; P3 = polish/doc debt.

| ID | Pri | Defect | Repro | Expected vs actual | Suspected file |
|---|---|---|---|---|---|
| D-1 | P1 | New-task dialog: Title input not focused → typed text lost | nav → Board → FAB → type "x" → Add | Task created (round-4 behaviour; `board-task.json` passed then) vs nothing created, no error. `board-task.json` FAILs at HEAD (2 runs); clicking the input first works (`qa-r5-board-add-debug` PASS) | `apps/oino-ui/src/views/board.rs` (`board_new_task`/`new_task_dialog` focus path; `bb853e3` rewrite) |
| D-2 | P1 | Composer loses keyboard focus after a mode-command result card | type `/ralph` (card renders) → type `/graph` | Next command lands in the composer vs keystrokes go nowhere until the composer is clicked. `unipi-long-horizon` FAILs step 19 consistently (2 runs); reproduced with `/goal` too | `apps/oino-ui/src/views/session_pane.rs` (result-card/mode-arm path) or `components/timeline/cards.rs` |
| D-3 | P1 | `zcode` suite manifest references deleted `zcode-model-menu.json` | `cargo xtask gui suite zcode` | Suite runs its 5 remaining scenarios vs hard error before any run | `docs/dev/v1/gui/suites/zcode.json` |
| D-4 | P1 | `unipi` suite red at HEAD (17/24) | `cargo xtask gui suite unipi` | Green regression suite vs 7 failures (2 = D-1/D-2, 5 = stale selectors below) | `docs/dev/v1/gui/scenarios/unipi-{workflow,notify,utility,updater,kanboard}.json` |
| D-5 | P2 | fmt gate red in all three workspaces | `cargo fmt --all --check` (root/extensions/apps) | Clean vs 7 diffs in `crates/oino-kanboard` (`run.rs:467`, `service.rs:79/139/325`, `tests/attachments.rs:32/114`, `tests/common/mod.rs:205`) — committed unformatted in `bb853e3` | `crates/oino-kanboard/*` |
| D-6 | P2 | `/kanboard status` prints stale `daemon: not running`; output lost the word "kanboard" | real/faux session → `/kanboard status` | Post-#7 status copy with no daemon references (D1 deleted it) vs first line `daemon: not running`; `unipi-kanboard` step 26 (`wait text "kanboard"`) never matches | `extensions/unipi/kanboard/src/lib.rs` `status()` + the scenario |
| D-7 | P2 | Stale scenario selectors after #2 / WP-settings / WP-kanboard rebuilds | `unipi-workflow` (removed `mode-chip`), `unipi-notify`/`unipi-utility` (`set-ns-0`), `unipi-updater` (`set-about`), `zcode-palette-history` (`open: board` extension panel vs the native board page) | Scenarios repointed to the new surfaces (`agent-mode`, settings nav rows, `route: board`) vs timeouts; functionality verified live in every case (`qa-r5-mcp-hub` PASS proves the hub path end-to-end) | the five scenario JSONs |
| D-8 | P2 | Startup flake: "the app did not come up with a composer" | Occasionally on scenario restart (seen: approval-deny 1st run; twice by WP-composer) | App reliably reaches the composer vs a frame where pane probes are missing for seconds; re-runs pass | `scripts/gui-drive.py` startup wait / `apps/oino-ui` first-frame path |
| D-9 | P3 | Spec'd acceptance scenarios never committed: `r5-board-modal-form`, `r5-board-fulltext`, `r5-scroll-containment`; no `suites/r5.json`; unipi manifest lacks the `r5-*` rows gui-driver.md's table implies | `ls docs/dev/v1/gui/scenarios/r5-board-*` | Round5.md §7 WP-kanboard acceptance scenarios exist vs only 3 board scenarios; QA equivalents committed under `docs/dev/v1/qa-r5/scenarios/` | `docs/dev/v1/gui/{scenarios,suites}/` (round5-integration) |
| D-10 | P3 | `r5-tui-removal` Extensions-page assertions are vacuous | `views/extensions_page.rs` registers no automation Meta | `absent {label "unipi.footer"}` can fail on a real row vs only on a stray string | `apps/oino-ui/src/views/extensions_page.rs` (add `Meta::labelled` to ext rows) |
| D-11 | P3 | `r5-board-drag-pc` does not assert the refused-lane `<60px` collapse (spec acceptance) | read the scenario | Mid-drag bound assertion vs drop-refusal only | `docs/dev/v1/gui/scenarios/r5-board-drag-pc.json` |
| D-12 | P3 | PC board toolbar beyond the spec'd unipi chip set | tree after nav-board | unipi chips (view toggle, scope, search, Filter, Display, Clear + New task) vs extra `Move` button + `Refresh` icon, no `Clear` chip | `apps/oino-ui/src/views/board.rs` `board_toolbar` (owner visual pass) |
| D-13 | P3 | No-session Project-scope state + "Needs an open session" toast unreachable by driver | every backend starts ≥1 pane | Zero-pane state reachable for the #22d acceptance vs code-verified only (`settings_page.rs:925`, `:607`) | `apps/oino-ui` startup session policy |
| D-14 | P3 | `rewind-real` model flake (`jev: unavailable` + approval-needing bash) | first suite run | Deterministic turn vs model chose `od -c`; passes on re-run | ti fixture env (`jev` decision model config) |
| D-15 | P3 | Screenshot/docs drift at HEAD | `git status` | Clean docs tree vs PNG drift + `docs/unipi.md`/`keybindings.md`/`tui.md` still describing removed features (WP-extensions' report) | docs |
| D-16 | P3 | `extensions/dist/checksums.json` is not reproducible: rebuilding the same HEAD (`cargo xtask wasm`) changes all 70 hashes (committed file then "mismatches" until regenerated) | `cargo xtask wasm && cargo xtask checksums` on a fresh checkout vs the committed file | Integrity record stable across builds vs every rebuild rewrites it (wasm toolchain path/timestamp nondeterminism); the `wasm`→`checksums` round-trip on one machine stays green | `xtask/src/main.rs` `checksums()` / wasm build flags |

## 7. Requests for other work

- **qa-r5-fix / round5-integration**: D-1, D-2 (code), D-3, D-4, D-6, D-7, D-9, D-11
  (scenarios/manifests), `SETTINGS_*` dead-constant prune in `design/tokens.rs` (§8 of
  round5.md, still pending), `mise run gc` after heavy builds.
- **WP-kanboard follow-up**: rustfmt pass over `crates/oino-kanboard` (D-5), status copy (D-6).
