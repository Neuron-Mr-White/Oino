# Desktop design review — `design-desktop` @ `a83dd76`

Reviewer: QA (design-review), 2026-10-27. Spec: [design.md](../design.md) **including Amendment A1**
(commit `3ab29fd`, which landed *before* `a83dd76`). Inputs: `design/ref/*`, `design/shots/desktop/01…16`,
extra shots taken with the `--demo` backend, code in `apps/oino-ui` + `apps/oino-desktop`, the
app-convention skill. No code was changed.

## Verdict: **FAIL**

The build and test gates pass. The package has a lot of ZCode-style layout, the docked interaction
cards and the component library. It still fails acceptance:

1. **Amendment A1 was not implemented at all.** It is binding and overrides §2 colour values:
   - The palette is still Zai neutral grey with a black/white primary. It should be the unipi
     warm OLED palette with the shell-red primary.
   - The embedded browser (A1.4) is missing.
   - The PTY terminal (A1.5) is missing.
   - The Unicrab mascot window is missing.
   - `DESIGN-STATUS.md` still records #42 and #43 as "⛔ diverge", but A1.6 makes them **Match**.
2. **Keyboard behaviour is mostly missing.** Only the composer, palette, docked cards and dialogs
   handle keys. Menus, sidebar, timeline, side tabs, diff viewer, status panel, board and settings
   do not have the key tables from §4.
3. **The perf budget is not demonstrated.**
   - The only evidence is from lavapipe.
   - The worst-window p99 is 14.3 ms against a 10 ms budget.
   - The ≥ 100 fps target and several §2.11 rows were never measured.
   - A real GPU (RTX 4070 Ti) is available on the dev machine.

## Gates (run by the reviewer)

| Gate | Command (`apps/`, `TMPDIR=$PWD/target/tmp`) | Result |
|---|---|---|
| fmt | `cargo fmt --all -- --check` | PASS (exit 0) |
| clippy | `cargo clippy --workspace --all-targets -- -D warnings -W clippy::pedantic` | PASS (exit 0) |
| tests | `cargo test --workspace` | PASS: oino-ui 65, oino-desktop 4, oino-mobile 4 |
| view-model tests (no GPUI) | `cargo test -p oino-ui --no-default-features` | PASS: 61 |
| build | `cargo build -p oino-desktop`; `cargo build --release -p oino-desktop` | PASS |
| guard `no_literal_styles` | part of the tests | PASS. Exception: `src/mobile/` (`design/tests.rs:47-50`) |
| contrast | `design::tests` | PASS **but relaxed**: `fg_subtle` is checked at ≥ 3:1, not the spec's 4.5:1 (`design/tests.rs:17-22`) |
| perf, 4 panes | `PERF_HEADLESS=1 scripts/perf.sh target/release/oino-desktop 25 4` | `frames=1320 dropped=0`, p50 6.03, **p99 median 9.70, p99 worst 14.32**, max 15.14 ms. The script passes only because it compares the *median of 120-frame-window p99s* |
| perf, 1 pane | `… 20 1` | p50 2.14, p99 median 3.05, worst 3.42, max 5.91 ms |

## Acceptance criteria (§10.1)

| Criterion | Result | Evidence |
|---|---|---|
| Palette exactly per §2.1 **as amended by A1.2** (unipi warm OLED, shell primary, wordmark semantics, derived light) | **FAIL** | `oino-ui/src/design/tokens.rs:101` LIGHT and `:214` DARK use Zai values: `window 0x2B2B2B`, `frame 0x161616`, `primary 0xFFFFFF/0x000000` (`:135`, `:248`), `brand 0xE0301A/0xFF5A3C` (`:138`, `:251`). There is no `#0A0908`, `#F03018`, `#F5EFE6`, `#28BEF0` etc. `DESIGN-STATUS.md` #1 says "Zai values". Every shot shows a grey UI with a white primary |
| Every §8 Match/Adapt row visible in a screenshot or covered by a view test | **FAIL** | #42 browser and #43 terminal are missing. #6 motion, #23–25, #31, #33, #40, #52, #54 are partial. Rows #17 query rail, #18 find bar, #30 completion menu, #33 queue, #47 toasts, D7 bottom frame, D11 editor-replace, D13 onboarding, ErrorBanner, DockDialog, and the Files/Tree/Agent tabs have **no screenshot and no view test** |
| G1–G23 closed | Mostly PASS | G3/G4/G6–G23 are addressed. G5 header compresses at narrow widths (chips drop). G20 Unicrab exists, but A1's mascot window does not |
| §2.11 p99 ≤ 10 ms with 4 streaming panes on the Linux dev machine | **FAIL / unproven** | See Gates. Software rendering only. Worst window is 14.3 ms. The ≥ 100 fps target is untested: Xvfb runs at 60 Hz, giving 1320 frames / 25 s ≈ 53 fps. Input-to-paint, session switch, cold open, palette at 10k items, first frame and idle CPU were not measured |
| Guard test green | PASS | see Gates |
| gpui-app.md §5 → pointer (R6) | Open | requested from integration (not owned) |
| Screenshots of spec shots 01–09 | PASS | 01–09 plus 10–16 in `docs/dev/v1/design/shots/desktop/` |

## unipi surface mapping (§7): row results

PASS means a native rendering exists and was seen in a shot or code. PARTIAL and FAIL rows name the gap.

| Package / surface | Result | Note |
|---|---|---|
| core hint line (DockStrip, ‹ ›) | PASS | shots 01–04 |
| core hints browser / tray (D10, D11) | PARTIAL | code path exists (`Placement EditorReplace`), but there is no shot |
| core header / crab hero | PASS | 01 (crab drawn ≈ 64 px, not 128) |
| footer glance frame / stats strip | PASS | 01–03 context header and meta row |
| footer process line, suppressed when StatusPanel expanded | PARTIAL | not shown |
| info-screen dashboard (D10) | PARTIAL | 09. Tabs are a horizontal Segmented, not the left vertical list at ≥ 760 |
| workflow approval / plan review / mode chip | PASS | 03, 10 |
| workflow saved rules (Permissions page) | PARTIAL | no shot |
| ask-user form | PASS | 04 |
| ask-user `osNotify` / OS notification | **FAIL** | `views/shell.rs:489-491` turns OsNotify into an in-window toast. `:383-391` also shows only a toast when the window is unfocused, which the user cannot see |
| long-horizon goal in StatusPanel | PARTIAL | text only. No pause/resume, elapsed time or tokens, and the state is not on the chevron line (`components/status_panel.rs:170`) |
| memory / dream ProvenanceCard | PASS | 08 |
| subagents AgentRow → Agent tab | PARTIAL | needs R5 (accepted) |
| fusion sidekick / wake widget | PARTIAL | no shot |
| btw side chat (D11) | PARTIAL | no shot |
| background-tasks StatusPanel Run + D7 Tasks | PARTIAL | Run rows have no logs/kill/open actions. No D7 shot |
| notify toast / OS notification | **FAIL** | same as ask-user: no OS notifications |
| kanboard board (D9) | PARTIAL | 07. The toolbar has no Filter/Display/search/New task or project picker. Task detail renders inline, not as an overlay. There is no Board nav ready-count badge |
| mcp / skill-registry / updater overlays | PARTIAL | the generic D10 path exists. Update badge detection is a string heuristic (`views/shell.rs:484`) |
| command-enchantment completion colours | PARTIAL | no shot of the CompletionMenu |
| compactor SummaryCard | PARTIAL | code only |
| app-bridge side tab | PARTIAL | code only |
| input-shortcuts | PASS | palette Actions |
| watchdog ErrorBanner | PARTIAL | `timeline/cards.rs:22`, no shot |
| web-api ToolRow verbs | PASS | code |
| settings hub | PASS | 13 |
| Unicrab aggregate state | PARTIAL | the glyph is in the sidebar top row, not the footer, and there is no mascot window. Window title "● Oino" works (`views/shell.rs:433`) |
| A1: links / `openUrl` → side-pane Browser; localhost preview chip | **FAIL** | `views/shell.rs` `UiEffect::OpenUrl` → `cx.open_url` (external browser) |

## ZCode parity checklist (§8): items that don't meet the target

Failing (required Match after A1):
- #42 Embedded browser: missing.
- #43 Interactive terminal: missing.

Partial:
- #6 Motion: nothing animates; `with_animation` appears 0 times in `oino-ui/src`.
- #11 Sidebar footer: crab not in the footer; wrong `Ctrl+K` hint next to the model label.
- #23–25: depend on requested core work, but Undo and Rewind are still shown as live buttons.
- #28 Model menu: sections instead of submenus.
- #33 Queue: partial.
- #40 Status panel: no actions.
- #45 Platform modifier: literal `⌘Enter` in `components/interaction/dock_dialog.rs:275`.
- #52 Performance: unproven.
- #54 Window chrome: server-side decorations on Linux and Windows.

The other rows (layout, frames, timeline rows, user bubble, docked cards, palette scopes, settings
page, toasts, split panes) match closely. Shots 01–03 put side by side with
`ref/zcode-welcome-dark.png` and `ref/zcode-workspace-dark.png` show the same density, composer
shell, chips and one-line tool rows.

---

## Defects (prioritised)

### P0: blocks acceptance

| # | Screen | Expected | Actual | File hints |
|---|---|---|---|---|
| P0-1 | All (01–16), dark and light | A1.2: Oino Dark = unipi warm OLED (`bg #0A0908`, `surface-1/2/3 #141211/#1C1917/#26211F`, `line #2E2825`, `text #F5EFE6`, `text-muted #A39A90`, `text-dim #6E655D`). Primary = shell `#F03018` (hover `shell-hi #F01818`), text on primary = face `#181818`. Semantics = wordmark cyan/green/yellow/red. Destructive = `deep #C00018`. Ember glow behind the welcome header. Oino Light derived from the same hues at ≥ 4.5:1 | Zai neutral palette. Primary button and send/stop are white-on-black (03 "Allow once", 04 "Next"). Semantics are Zai green/orange/red. No ember glow | `apps/oino-ui/src/design/tokens.rs:101-330`, `design/theme.rs` (gpui-kit derivation), `design/tones.rs`, `apps/oino-desktop/DESIGN-STATUS.md` #1 |
| P0-2 | 03, 04, 10, sidebar rows, header capsule | A1.2: the `waiting_surface` treatment becomes unipi **needs-you** (shell colour, pulsing, crab-eyes icon + word). Agent state colour, icon and word follow unipi §1.3 everywhere (working = `claw`, done = wm-green + dot, error = wm-red, offline = text-dim) | "Waiting for you" is a green Zai badge; running is Zai orange | `design/tokens.rs` (`waiting_surface`, `waiting_fg`), `components/badge.rs` StatusCapsule/Dot, `components/sidebar.rs`, `components/header.rs` |
| P0-3 | Side pane | A1.4: **Browser** tab via `gpui-webview` (gpui-kit 0.8.1): URL bar, back/forward/reload, open externally, devtools in debug. `openUrl` and transcript links open there. `localhost:*` preview chip from bash output. Linux X11/XWayland plus a "Native Wayland" setting | Not built. gpui-kit is still `=0.8.0` (`apps/Cargo.toml:20`). `SideTab` has no Browser (`views/session_pane.rs:82-100`). `OpenUrl` → external (`views/shell.rs` UiEffect::OpenUrl). Status file marks #42 ⛔ | `apps/Cargo.toml`, `views/session_pane.rs` SideTab, `components/side_pane.rs`, `views/shell.rs` |
| P0-4 | Bottom frame | A1.5: **Terminal** tab with real PTYs (`alacritty_terminal` + `portable-pty`): tabs, split, copy/paste, links, 10k scrollback, token theme | Not built. `BottomTab` = Output / Tasks / Panel only (`views/session_pane.rs:177-184`). There is no `alacritty`/`pty` dependency. Status file marks #43 ⛔ | `views/session_pane.rs` BottomTab, new `components/terminal/` |
| P0-5 | Perf (§2.11, #52) | p99 ≤ 10 ms with 4 streaming panes and ≥ 100 fps on the Linux dev machine, with the other §2.11 rows measured | Only lavapipe was measured. Worst-window p99 is 14.32 ms. The script gates on the median of window p99s, which hides the tail. Frames ≈ 53 fps (60 Hz Xvfb), so ≥ 100/120 fps is unproven. Not measured: typing latency, session switch (≤ 50 ms), cold open (≤ 150 ms), palette 10k (≤ 16/4 ms), first frame (≤ 150 ms), idle CPU. A real GPU (RTX 4070 Ti) is available | `apps/oino-desktop/scripts/perf.sh:40-48`, `design/perf.rs` |

### P1: must fix before sign-off

| # | Screen | Expected | Actual | File hints |
|---|---|---|---|---|
| P1-1 | All controls | A1.2 shapes: buttons, chips and toolbar capsules pill-shaped. Cards and dialogs/menus 16, inputs 12. Focus = 2 px **shell** outline, offset 2. Uppercase muted section labels 12/500/.1em | Buttons and chips use `R_LG` 8 (`components/button.rs:117`). Cards use `R_XL` 12. Focus uses the `input_border_focus` grey step | `design/tokens.rs:357-367` radii, `components/button.rs`, `components/controls.rs`, `design/theme.rs` focus |
| P1-2 | Desktop | A1.2: opt-in **mascot window** (transparent, always-on-top, crab + bubble, native GPUI). Unicrab glyph in the sidebar footer (§4.7) | No mascot window (`grep -ri mascot` finds nothing). Glyph moved to the sidebar top row | `apps/oino-desktop/src/main.rs`, `components/unicrab.rs`, `components/sidebar.rs` footer |
| P1-3 | Menus, sidebar, timeline, side pane, diff, status panel, board, settings, queue | §4 key tables:<br>• Menus: ↑/↓ wrap, →/← submenu, type-ahead, Esc returns focus.<br>• Sidebar: ↑/↓, ←/→, Enter, ⌘Enter, F2, Delete, `/`.<br>• Timeline: focus cursor, Enter, `c`, `o`, Home/End, ⌘↑/⌘↓.<br>• Side tabs: ⌘1–9, Ctrl+Tab, ⌘W.<br>• DiffViewer: n/p, ]/[, u.<br>• StatusPanel: ↑/↓, Enter, Delete.<br>• Kanban: h/j/k/l, Enter, m, c, `/`.<br>• Settings: ⌘F, ↑/↓.<br>• Queue: ↑/↓, Enter, ⌘Enter, Delete, Alt+↑/↓.<br>• Tooltips also show on keyboard focus | Key handling exists only in `components/composer.rs:93`, `components/palette.rs:151`, `components/interaction/*`, `components/dialog.rs:58`. The keymap is global only (`views/mod.rs:80-97`). No handlers in `menu.rs`, `sidebar.rs`, `timeline/*`, `side_pane.rs`, `status_panel.rs`, `ous/kanban.rs`, `views/settings_page.rs`. Principle §1.7 "keyboard first" is not met | the files listed |
| P1-4 | 03 (approval appears while typing) | The composer stays visible and typing is allowed, with the draft kept (§4.12). A keystroke meant for the composer must never resolve an approval | `InteractionView::new` moves focus to the card (`components/interaction/mod.rs:146`). The next Enter approves the highlighted option (`interaction/approval.rs:69`, default `allow_once`) and digits pick options. A user pressing Enter to steer at that moment approves a shell command | `components/interaction/mod.rs:135-147`, `views/session_pane.rs` `sync_interaction` |
| P1-5 | Unfocused window | `osNotify` → OS notification when unfocused. Approvals and questions notify via the OS ("Oino needs approval: …", `notifyOnAsk`) | Only an in-window toast, which an unfocused user never sees | `views/shell.rs:383-391`, `:489-491` |
| P1-6 | 08, 11, 16 (TurnFooter), user bubble hover | Undo and Rewind are offered only when they work (core checkpoints pending) | Enabled "Undo" and "Rewind files to here" buttons that only toast "needs core file checkpoints" | `components/timeline/cards.rs:308-318`, `components/timeline/user.rs:98-108` |
| P1-7 | 10, 15, 16-narrow, 720 px | The status panel / mini capsule never covers timeline content (§3.2: the timeline shifts when it would overlap) | The mini capsule overlaps the user bubble text in 10 ("…commas and i[◎0/3 ⎇0]") and body text in every split pane in 15. At 720 px it covers the code block | `components/status_panel.rs`, `components/conversation.rs` (offset/inset logic) |
| P1-8 | Window chrome (#54, §3.1) | Client-side decorations on Linux/Windows: gpui-kit `TitleBar` merged into the sidebar top row, window controls at the right, header as drag region, Linux 16 px shell radius | Server-side decorations on Linux/Windows (the status file says so) | `apps/oino-desktop/src/main.rs`, `views/shell.rs` |
| P1-9 | All text (`fg_subtle`) | `fg` and `fg_subtle` on frame/card/input/menu ≥ 4.5:1 | The test was relaxed to 3:1 for `fg_subtle`, which is used for tool verbs, descriptions, inactive session titles and the meta row. Fixed automatically by P0-1 (unipi `text-muted` on `bg` ≈ 7:1) | `design/tests.rs:17-22` |
| P1-10 | Sidebar session row context menu (§4.7) | Rename, Pin/Unpin, Fork, Open in new pane, Copy session id, Reveal file, Delete (with confirm). Group header ⋯ (Reveal, Copy path, Remove). ⇅ sort button. Marquee title | Only Pin, Open in new pane, Copy session id. No group ⋯ menu and no sort button (01: only "–" collapse). The `⌘↵` label is hard-coded | `components/sidebar.rs:262-300` |
| P1-11 | 14 Extensions (D12) | Columns: icon, name, id mono, version, kind, state with error tooltip, permissions summary. Row actions: Enable/Disable, Settings, Readme. Header: Install from file…, Open extensions folder | Only id, version, kind, state and a Settings link. "11 loaded" counts the Failed `unipi.mcp`. Kind badges stretch to the column width | `views/extensions_page.rs:146` and the row builder |
| P1-12 | 07 Board (D9) | Header "Board · <project ▾>". Toolbar: Filter, Display, search, `+ New task`. Agent chip (`bot` + elapsed) on claimed cards. Enter opens the detail as an overlay | Only the extension's keys (Refresh, Add to Backlog, Show in chat). No project picker. "claimed by this session" in danger red. Detail is inline under the lanes | `views/board.rs`, `components/ous/kanban.rs` |
| P1-13 | Acceptance evidence | Every Match/Adapt row visible in a shot or covered by a view test. D13 reachable in the demo | No shot for: find bar, query-rail tooltip, completion menu, queue, toasts, bottom frame, editor-replace (tray/btw), DockDialog, ErrorBanner, Files/Tree/Agent tabs, empty side-pane launcher, onboarding. `OINO_DESKTOP_ONBOARDING=1 … --demo` still opens D1, because the demo settings are already `onboarded` (`views/shell.rs:145`) | `apps/oino-desktop/scripts/screenshot.sh`, `src/demo.rs` |
| P1-14 | Settings → Models & Providers | app-convention: logout (an important action) needs confirmation | "Log out" fires immediately | `views/settings_page.rs:641-642`, `:247-251` |

### P2: polish and consistency

| # | Screen | Expected | Actual | File hints |
|---|---|---|---|---|
| P2-1 | Menus, dialogs, palette, toasts, status panel | §2.10: enter fade + 4 px slide or 0.98→1 scale at 180 ms, exit 0.7×. Running shimmer on the run-state/thinking label. Dot pulse 1.2 s. app-convention Motion section | No animations at all (`with_animation` = 0 in `oino-ui/src`) | `design/` motion tokens, `components/menu.rs`, `dialog.rs`, `toast.rs`, `timeline/run_state` |
| P2-2 | 03, 10 | Number keys as **prefix** `Kbd` ("1 Allow once") | Suffix ("Allow once 1") | `components/interaction/approval.rs:137`, `plan.rs` |
| P2-3 | 02, 03 (running) | Placeholder "Steer Oino…" while running | Always "Ask Oino anything, …" | `views/session_pane.rs:304,323` |
| P2-4 | 02/08 ThinkingBlock | "Thought for 12s" | "Thought" | `components/timeline/thinking.rs` |
| P2-5 | 02 vs 03 | Diff meta always `+8 −3` | Streaming preview shows `-3 +8` | `model/tools.rs` streaming preview |
| P2-6 | 02 status panel | Plan step text keeps inline code | Backticks removed and the spacing left behind: "Patch the , handling so a closing ] after a c…" | `model/status.rs` plan parsing |
| P2-7 | 05 palette | The active row fills the full row width. `Kbd` right-aligned. Session rows show a state dot. History chips. EXTENSIONS and APP sections in All | The highlight hugs the content width. `Kbd` sits right after the label. Sessions use a dashed-square icon. No history row. No Extensions/App sections | `components/palette.rs` |
| P2-8 | 12 model menu | Other providers as submenus. Centre-aligned to the trigger. "Pinned" group. Type-ahead filter. Separate capability badges | Flat sections. Left-shifted relative to the trigger. "Vision · 400k" is one mono tag | `components/model_menu.rs` |
| P2-9 | Sidebar footer (all) | settings · update badge · model label · crab | Shows `Ctrl+K` next to the model label (the model shortcut is ⌘M). No update badge in the shots | `components/sidebar.rs:533` |
| P2-10 | Sidebar groups | `folder` icon tinted by a stable hash of the cwd (`categorical`) | All groups are the same colour | `components/sidebar.rs` WorkspaceGroup |
| P2-11 | 01 welcome | Suggestions: "Resume …" (most recent unfinished), "Board: N ready", then prompt templates. Hint strip below the composer (D1). Hero Unicrab 128 | No Resume card. Board card says "Open the kanboard". Hint strip is above the composer. Crab ≈ 64 px | `views/welcome` / `components/conversation.rs` empty state |
| P2-12 | 06 Appearance | Interface size 12–18 (Select) and code font 10–20. Sidebar density row. Nav has an "Extensions" entry | 6-way segmented with no 17. Code font 11–16. No sidebar density. No Extensions nav entry | `views/settings_page.rs` |
| P2-13 | 13 hub | Secret field has a reveal eye toggle | Masked field + "Stored" badge only | `components/settings.rs` |
| P2-14 | 09 info overlay | Left vertical tabs at ≥ 760. Dialog `large` 760×min(680, 85vh) | Horizontal Segmented. Height shrinks to content | `components/ous/mod.rs` tabs in overlay, `views/session_pane.rs` overlay |
| P2-15 | 11 Review | Old/new line numbers. Per-file actions (Open, Copy path, Revert). "All turns" select. Unified/split toggle (≥ 900) | None visible. The hunk header is clipped | `components/side_pane.rs` DiffViewer |
| P2-16 | 15 split panes | Usable panes. Header controls on-screen. Text ellipsised | At 1400 px four ≈ 260 px panes. The 4th pane header's `›` toggle is clipped at the right edge. Placeholder and meta row are clipped without ellipsis ("comma", "json-lite"). Consider auto-collapsing the sidebar or a minimum pane width | `views/shell.rs` layout, `components/meta_row.rs`, `components/composer.rs` |
| P2-17 | Composer | Context ring tooltip "Compact now". Compaction should be confirmed or undoable (app-convention: important actions) | One click compacts immediately. The ring is hidden until usage exists (01) | `components/composer.rs:376-393` |
| P2-18 | 03 | Send is disabled while a card is pending. Stop should stay available | Stop is disabled too (`.disabled(blocked)`); only Esc/⌘. abort | `components/composer.rs:394-405` |
| P2-19 | DockDialog editor | Platform modifier label (#45) | Literal "⌘Enter to submit" on Linux | `components/interaction/dock_dialog.rs:275` |
| P2-20 | Update badge | Driven by `unipi.updater` state | Heuristic: notify text containing "update" and "available" | `views/shell.rs:484` |
| P2-21 | Async surfaces | app-convention: skeleton loaders for async UI (session list, extensions, settings schema, auth providers) | No skeletons; content pops in | `components/sidebar.rs`, `views/extensions_page.rs`, `views/settings_page.rs` |
| P2-22 | Light theme coverage | Light screenshots for the key screens | Only 08 and 13 are light. Reviewer shots (welcome and approval in light) look consistent, but they are not committed | `scripts/screenshot.sh` |
| P2-23 | 04 question card | Footer: Back / Skip / Next (§4.13) | An extra "Cancel esc" ghost button. OK, but not in the spec; document it as Adapt | `components/interaction/question.rs` |

## Notes

- Positives worth keeping: the docked cards (03/04/10) closely match ZCode `ChatBottomDock`. The
  one-line tool rows and AgentRow match `zcode-workspace-dark.png`. The composer shell and context
  header match `zcode-welcome-dark.png`. The token guard test works, the ticker stops when idle,
  and panes are cached.
- Reviewer screenshots (not committed) were taken with the same Xvfb + lavapipe recipe as
  `screenshot.sh`. Light welcome, light approval, 720×600, and the palette with the query "kan"
  were consistent with the findings above.

---

## Resolution (`design-desktop-fix`, 2026-10-27)

Fix-up on `v1-alpha` after this review. A1 is implemented. Gates (`apps/`,
`TMPDIR=$PWD/target/tmp`): `cargo fmt --all -- --check` ✔, `cargo clippy --workspace
--all-targets -- -D warnings -W clippy::pedantic` ✔, `cargo test --workspace` ✔ (oino-ui 72
incl. browser/terminal/menu/keys/board tests, oino-desktop 4, oino-mobile 4),
`cargo test -p oino-ui --no-default-features` ✔ (62), debug + release builds ✔. Screenshots
01–16 refreshed and 17–30 added in `shots/desktop/` (`scripts/screenshot.sh`).
Toolkit bump: gpui-kit `=0.8.1` / gpui-pre `=0.3.10` (needed by gpui-webview 0.8.1); the
`secondary-` keystroke prefix is gone in 0.3.10, so bindings use `cmd-` on macOS and `ctrl-`
elsewhere (`views::primary_mod`, `components::secondary`).

Status: **Fixed**, **Partial** (what is left is named), **Open**, **Adapt** (accepted deviation).

### P0

| # | Status | What changed |
|---|---|---|
| P0-1 | Fixed | `design/tokens.rs`: Oino Dark = unipi warm OLED (`bg #0A0908` frames on `surface-1 #141211` chrome, `surface-2/3`, `line #2E2825`, `text #F5EFE6`, `text-muted #A39A90`, `text-dim #6E655D`); primary `shell #F03018` (hover `shell-hi`), text on primary `face #181818`; new tokens `focus`, `working` (claw), `amber`, `gold`, `cream` (caret), `deep` (destructive fills), `glow`, `info*` (wm-cyan); semantics = wordmark green/yellow/red/cyan. Oino Light derived from the same hues on warm off-white. Ember glow behind the welcome crab. Contrast test covers the new pairs (`face` on `shell` is 4.35:1 by A1's own values; the test floors it at 4.3 and says why) |
| P0-2 | Fixed | `Capsule::Waiting` = **Needs you** (shell surface, crab-eyes `!` glyph, pulsing); `Capsule::Running` = **Working** in claw; sidebar needs-you dot pulses; `CrabState` colours: working claw, done wm-green, idle muted, offline/sleep dim; kanban and status-panel "running" use claw |
| P0-3 | Fixed | `components/browser.rs` over `gpui-webview` 0.8.1: side-pane **Browser** tab (URL bar with search fallback, back / forward / reload, devtools in debug, open externally), one per pane; `openUrl`, transcript links, terminal ⌘/Ctrl-click and a meta-row `localhost:*` preview chip (from bash / shell-tool output) open there; native view hidden while an overlay, other route or another pane covers it. Linux starts on X11/XWayland (`gpui_kit::platform::linux(WindowingModes::X11)`), Settings → Appearance → "Native Wayland" opts out (browser then falls back to the system browser). Shot 18 |
| P0-4 | Fixed | `components/terminal.rs`: `alacritty_terminal` 0.26 emulation over its platform PTY, painted with GPUI text runs per style span; bottom-frame **Terminal** tabs (`+`, split into two shells, close), keys incl. app-cursor mode / Ctrl / Alt, copy & paste (⌘C/⌘V, Ctrl+Shift+C/V elsewhere, bracketed paste), mouse selection (word / line on double / triple click), ⌘/Ctrl-click links → Browser, 10 000-line scrollback (wheel, Shift+PgUp/PgDn), ANSI + 256 + true colour from tokens, `cream` cursor. Shot 17 |
| P0-5 | Partial | Measured on the RTX 4070 Ti (GNOME XWayland): 96–101 fps, p50 3.6–3.9 ms, window-p99 median 7.5–8.4 ms, **whole-run p99 9.3–11.6 ms** over five 25 s 4-pane runs on a quiet desktop, 13.3–14.9 ms in three later runs while the shared machine had load average 7–10 (p50 rose from 3.6 to 5.7 ms with it); before the fixes it was 16.0 ms / 72 fps; first frame 280 ms, keystroke → paint median 3–5 ms, session switch 6–12 ms, palette 10k filter 0.98 ms, idle UI-thread CPU 1–3 %. `perf.sh` now gates on the exact whole-run p99 (histogram) and prints all of these. Fixes: ticker notifies only the views that asked (scoped, round-robin) instead of `refresh_windows`; background split panes stream at ≈ 30 fps with a per-session phase; idle crab static. **Left:** the 10 ms run-p99 gate is not reliably met (passes 2 of 8 runs; frames where all four panes re-lay out together cost 6–12 ms); needs a dedicated, unloaded measurement and probably cheaper per-pane layout |

### P1

| # | Status | What changed |
|---|---|---|
| P1-1 | Fixed | Radii tokens `R_CARD`/`R_DIALOG` 16, `R_INPUT` 12, `R_SHEET` 24, `R_BUTTON` pill; buttons, segmented controls, chips pill-shaped; cards, docked cards, menus, dialogs 16; option rows/inputs 12; gpui-kit radius derived. Keyboard focus = 2 px shell ring (`focus_visible`) on every button (now Tab stops); input focus border = shell. Section labels uppercase 12/500 muted. (GPUI has no outline offset: the ring is a 2 px border) |
| P1-2 | Fixed | `views/mascot.rs`: opt-in transparent, frameless, always-on-top (`WindowKind::PopUp`) mascot window at the primary display's bottom-right with the animated aggregate Unicrab and a 6 s bubble for needs-you / finished; click → Oino to front, drag moves; Settings → Appearance → "Mascot window" + crab size 72/96/128. Sidebar footer carries the crab glyph again |
| P1-3 | Partial | Keyboard tables added: **menus** (`KeyMenu`: focus on open, ↑/↓ wrap skipping disabled, Home/End, Enter/Space, → submenu, type-ahead, Esc/← close and return focus); **sidebar** (↑/↓, Home/End, ←/→ collapse/expand/up to group, Enter, ⌘Enter new pane, F2 rename, Delete confirm, `/` search, Esc back to composer, focus ring); **timeline** (⌘↑ from composer, ↑/↓ k/j, Home/End, PgUp/PgDn, ⌘↑/⌘↓ prompts, Enter/Space expand, `c` copy, `o` open in side pane, Esc/`i` back; shell-colour row rail); **side tabs** (⌘1–9, Ctrl+Tab / Ctrl+Shift+Tab, ⌘W); **Review/Diff** (n/p, ]/[ with scroll-to-file); **board** (h/j/k/l, Enter, m, c, `/`, Esc); **settings** (⌘F, ↑/↓ pages, Esc). **Left:** StatusPanel and Queue keys, DiffViewer `u`, tooltips on keyboard focus |
| P1-4 | Fixed | A docked approval / plan card no longer takes focus when the composer has it (`InteractionView::new(.., typing, ..)`); ↑ in an empty composer or Shift+Tab moves to the card; resolving keys are ignored for the first 400 ms; Stop stays enabled while a card is pending |
| P1-5 | Fixed | `oino_ui::platform::OsNotifier` (desktop: `notify-rust`, off-thread): `osNotify` while the window is inactive and approvals/questions while unfocused ("Oino needs approval: …") post OS notifications; in-window toast otherwise |
| P1-6 | Fixed | TurnFooter Undo and the user-bubble "Rewind files" button are removed until core file checkpoints exist |
| P1-7 | Fixed | The mini status capsule is a right-aligned strip **above** the timeline (no longer floating over rows); the expanded panel keeps shifting the reading column. Shots 10, 15, 16, 30 |
| P1-8 | Fixed | Client-side decorations on Linux/Windows (`components/window_chrome.rs`): sidebar top row and pane headers are drag regions (double-click zoom, right-click window menu), min/max/close at the right end of the right-most header (or the side tab bar when the side pane is open), resize edges, Linux 16 px radius while floating, no invisible client inset. Verified on GNOME XWayland (`_MOTIF_WM_HINTS` decorations 0). Xvfb shots have no compositor, so GPUI falls back to server decorations there |
| P1-9 | Fixed | `fg_subtle` ≥ 4.5:1 restored in `design/tests.rs` (dark 6.3–7.2:1, light 6.9–7.2:1) |
| P1-10 | Fixed | Session menu: Rename (F2), Pin/Unpin, Fork (clone), Open in new pane, Copy session id, Reveal file, Delete… (confirm dialog, closes its pane, deletes the file). Group `⋯`: Reveal folder, Copy path, Remove from sidebar. `⇅` sort (recent ↔ name). `⌘↵` label goes through `platform_keys`. Marquee titles not done (truncate). Shots 27, 28 |
| P1-11 | Fixed | Extensions page: name + mono id, version, kind badge (no stretch), state (Loaded / Failed with error tooltip / Disabled), bundled, permission summary from the manifest; row Enable/Disable (`disabledExtensions`, applies to new sessions; the desktop host filters packages), Settings, Readme (dialog); header Install from file… (copies a package directory into `~/.oino/extensions` after validating it) and Open extensions folder; "N loaded · N failed · N disabled". Shot 14 |
| P1-12 | Fixed | Board: "Board · <project ▾>" picker over the sidebar workspaces, toolbar Filter (all / blocked / high priority / claimed), Display (Backlog, Done/Cancelled), search, Move (m), extension keys, `+ New task` (c); claimed cards show a `bot` chip in the working colour (not danger red); the selected task opens as an overlay dialog; keyboard cursor; sidebar Board nav shows the ready count. Elapsed time on the agent chip needs task timing from the extension (not in the panel spec). Shot 07 |
| P1-13 | Partial | Shots added: 17 terminal, 18 browser, 19/20 light, 21 find bar, 22 completion menu, 23 toasts, 24 bottom frame, 25 Files tab, 26 side launcher, 27 session menu, 28 confirm dialog, 29 onboarding (`OINO_DESKTOP_ONBOARDING=1` now forces D13 even with onboarded demo settings), 30 720 px. **Left:** query-rail tooltip, editor-replace (tray / btw), DockDialog, ErrorBanner and the Agent tab still have only code / view-model coverage |
| P1-14 | Fixed | "Log out" asks first (inline "Sign out?" + Cancel / destructive Log out) |

### P2

| # | Status | Note |
|---|---|---|
| P2-1 | Partial | Menus, dialogs and toasts enter with fade + 4 px rise (180 ms ease-out, `design::motion`, off with reduce motion); needs-you pulse. Exit animations and the run-label shimmer are not done |
| P2-2 | Fixed | Number keys are prefix `Kbd` (`Btn::kbd_lead`) on approval and plan options |
| P2-3 | Fixed | "Steer Oino… (Enter steers, Alt+Enter queues a follow-up)" while running |
| P2-4 | Open | "Thought for Ns" needs thinking timing in the transcript model |
| P2-5 | Fixed | Streaming preview is `+n −m` like the final diff meta |
| P2-6 | Fixed | Plan steps keep inline code (`` `,` ``) instead of dropping the backticks |
| P2-7 | Partial | Rows fill the width (highlight full-row, `Kbd` right-aligned), sessions use a state-dot icon. History chips and Extensions/App sections not added |
| P2-8 | Open | Model menu submenus / pinned group / type-ahead filter (type-ahead now comes from `KeyMenu`) |
| P2-9 | Fixed | Footer: settings · update · model label · `⌘M` · crab. Update detection still heuristic (P2-20) |
| P2-10 | Fixed | Group folders tinted by a stable cwd hash (visible in every shot) |
| P2-11 | Partial | Hero Unicrab at 128 with ember glow. Resume card, "Board: N ready" title wording kept; hint strip still above the composer |
| P2-12 | Fixed | Interface size 12–18 and code font 10–20 steppers, Sidebar density (Comfortable / Compact), Extensions entry in the settings nav, plus Unicrab / window rows (mascot, crab size, Native Wayland) |
| P2-13 | Open | Secret reveal toggle |
| P2-14 | Open | Info overlay vertical tabs / fixed large height |
| P2-15 | Partial | n/p file stepping with a focus ring; no line numbers, per-file actions or split view |
| P2-16 | Partial | Mini status no longer clips content; 4 × 260 px panes still clip header / meta row text |
| P2-17 | Open | Compact-now confirmation |
| P2-18 | Fixed | Stop stays available while a card is pending |
| P2-19 | Fixed | DockDialog label uses `platform_keys("⌘Enter")` |
| P2-20 | Open | Update badge still from notify text |
| P2-21 | Open | Skeleton loaders |
| P2-22 | Fixed | Light shots 08, 13, 19, 20 |
| P2-23 | Adapt | QuestionCard keeps the "Cancel esc" ghost button (recorded as Adapt) |
