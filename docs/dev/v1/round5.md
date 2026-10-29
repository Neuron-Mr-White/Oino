# Round 5 — spec: GUI-first, ZCode UX, harness-only parity

Status: decided (2026-10-30). Branch `v1-alpha`. Authoritative inputs:

- Owner direction + 27 issues: [round5-feedback.md](round5-feedback.md) (verbatim owner text, D1–D4).
- Scout reports: [round5/scout-oino.md](round5/scout-oino.md) (code map), [round5/scout-zed.md](round5/scout-zed.md)
  (Zed mechanisms + licenses), [round5/scout-gpui.md](round5/scout-gpui.md) (in-pin component inventory),
  [round5/scout-ref.md](round5/scout-ref.md) (ZCode + unipi UX metrics). Devin picker research (lead,
  2026-10-29): Devin Desktop = family-grouped picker with **explicit pinning** (changelog v1.9566.9),
  per-row token rates, per-family variant memory; Zed = star favourites + `alt-tab` cycling; Cursor =
  nested groups with implicit stickiness and **no** pin control (the cautionary case).
- Supersedes: the "unipi-first" amendments and every TUI-parity goal (see updated
  [CHARTER.md](CHARTER.md) Vision).

## 0. Direction, transcribed into engineering terms

| Dir | Meaning for this repo |
|---|---|
| D1 | Delete TUI-only / presentational extensions without argument (§3). No unipi-TUI chrome comes back. |
| D2 | UX/UI/motion follows **ZCode** ([scout-ref PART A](round5/scout-ref.md)) unless an issue explicitly pins unipi (board layout #8/#10, model-picker modal shape #1, markdown palette #17, logo #26). Every harness feature still bubbles up to the GUI. |
| D3 | Parity = **harness parity only**: pi 1.4.0 runtime behaviour + unipi features that change agent behaviour. The desktop is the product; no TUI client work (`crates/oino-tui` is frozen, deletion candidate for a later round). |
| D4 | Take components from Zed / awesome-gpui **only where the license allows**: Apache-2.0/MIT code (`gpui` family, `gpui-kit`/`gpui-component`/`gpui-base` 0.8.1 — already compiled into `apps/`, verified in [scout-gpui §0](round5/scout-gpui.md)) may be used and ported directly. Zed's app crates (`editor`, `project`, `picker`, `markdown`, `settings_ui`, …) are **GPL-3.0-or-later**: mechanism reference only — reimplement, never copy (keeps Oino MIT/Apache-2.0). Relicensing to GPL is *not* taken. |

Standing constraint from D4: every "learn from Zed" item below is a **reimplement-from-mechanism**
(scout-zed gives file:line for each mechanism). The only direct ports allowed are from
Apache-2.0/MIT sources, chiefly `gpui-component`/`gpui-base` at the current pin and
`longbridge/gpui-kit` examples.

## 1. Cross-cutting decisions

1. **Design tokens first.** Adopt ZCode's scale and motion as theme constants
   ([scout-ref A](round5/scout-ref.md)): text `xl 18 / lg 16 / base 14 / caption 13 / sm 12 / xs 10`,
   settings title 24/30; motion `fast 120ms / base 160ms / enter 200ms / ink 320ms`, ease
   `cubic-bezier(0.22,0.61,0.36,1)`; every animated block honours `reduce_motion`. These land in
   `design/type_scale.rs` + `design/tokens.rs` (owned by WP-shell) early; all packages consume.
2. **Contrast floor (#25).** Dark `fg_subtlest` `#6E655D` → a value passing AA 4.5:1 on `window`
   *and* `card` (target `#8A8078`-class, current `fg_subtle` passes at 4.9:1); `idle_task` (2.56:1)
   and `attention_fg`/`primary_fg` pairs raised likewise; light theme audited with the same rule.
   Token change is owned by WP-shell; each package re-checks the views it owns (see WP steps).
3. **Shared-file protocol.** Files have exactly one owning package (§7). Two exceptions:
   `apps/oino-ui/src/automation/*`, `scripts/gui-drive.py` and the scenario dir are **additive-only**
   for everyone (new driver steps/methods/probes, new `r5-*.json` scenarios); and `apps/oino-ui/src/host.rs`
   (owned by WP-settings) admits exactly one additive hunk from WP-composer (`favorite_models`).
   WP-extensions may delete *string-keyed references to removed extension ids* anywhere (bounded by
   the grep in its spec); owners must not revert those prunes.
4. **Verification.** Every package proves its issues with GUI-driver scenarios
   ([gui-driver.md](gui-driver.md)) plus the charter gates for the workspaces it touches. New
   scenarios are `docs/dev/v1/gui/scenarios/r5-<name>.json`; the consolidated `r5` suite manifest is
   assembled by `round5-integration` (packages do not edit one shared manifest in parallel).
5. **Owner screenshots are unavailable**; each `[image N]` item is resolved by inspecting the running
   app on `ti`/Xvfb with driver screenshots, per the feedback file's own instruction.

## 2. Issue decisions (#1–#27)

Each entry: **decision** (what to build / delete), **reference** (what to copy or port),
**acceptance** (driver-verifiable unless noted). The package that implements it is named; details in §7.

### Conversation & sessions

- **#4 — `+` duplicates sessions.** In `Workspace::open` (`views/shell.rs:678-733`), `OpenRequest::New`
  first looks for an existing pane whose session is empty (no user messages) with the same cwd and
  activates it instead of spawning a new backend session; only otherwise spawns. Applies to sidebar
  `+`, ⌘N (`views/mod.rs:120`) and the new-chat page. *Ref:* scout-oino §2.1. **Accept**
  (`r5-no-dup-sessions`): two rapid `+` presses → `state.panes == 1`; typing + Enter keeps one
  `session_id`; a second `+` *after* content exists opens a genuinely new session. *(WP-shell)*
- **#6 — new-session opens a ZCode-like new chat page.** New view `views/new_chat.rs`: greeting +
  Oino logo, a **project picker** (recent projects from `recent_cwds`, "No project" bucket,
  "Choose folder…" via the existing native prompt), each choice calls `open(New{cwd})` which now
  dedups (#4) — the "draft" is an empty session, reused not stacked. Sidebar keeps per-project
  "+" affordances (ZCode `PurposeSections` pattern, scout-ref A(#6)). The chat page's own
  empty state (greeting + logo, #26) is WP-chat's. *Ref:* scout-ref A(#6); ZCode draft state
  (`ConversationDraftEmptyState.tsx`). **Accept** (`r5-new-chat-page`): `+` routes to the page; picking
  a project creates a session whose tree shows the project; "No project" works; Esc returns. *(WP-shell)*
- **#15 — Esc twice stops, with hint.** First Esc while `busy && composer empty` **arms** stop and
  shows a transient toast "Press Esc again to stop" (~2.5 s, `components/toast.rs`); a second Esc
  while armed aborts (`PiCommand::Abort`); arming clears when the run ends or the composer gains
  text. Guards: Esc consumed by an open dialog/completion menu must neither arm nor stop (keep
  Zed's dialog-path guard discipline, scout-ref A(#15); the double-Esc hint itself is Oino design —
  no reference exists). **Accept** (`r5-esc-twice-stop`): stream scene → Esc → toast appears and run
  continues (`busy` still true) → Esc again → run stops (`busy` false, stopped row present); Esc with
  a dialog open only closes the dialog. *(WP-chat)*
- **#16 — virtualization: yes, keep and harden.** The transcript already uses GPUI `list()` +
  `ListState` with `FollowMode::Tail` (scout-oino #16). Decision: keep the architecture, adopt Zed's
  event→list-op discipline fully (this is the #21 fix); document it in `conversation.rs` module docs.
  **Accept:** `r5-bounce-watch` (below) plus a module-doc note; no code change beyond #21's. *(WP-chat)*
- **#21 — chat bounces.** Port Zed's no-bounce rules onto the existing list (all
  `session_pane.rs`/`conversation.rs`, scout-zed §1): (1) never full-list `remeasure()` while a turn
  streams — the checkpoint-load path `session_pane.rs:1202` becomes targeted `remeasure_items` or is
  deferred to turn end; (2) gate async-growing rows with `set_item_measurement_readiness` (Zed
  `conversation_view.rs:1595-1597`) or reserve stable provisional heights for markdown/thinking rows;
  (3) render the "generating" strip as a **spliced list item**, not a fixed footer (Zed
  `thread_view.rs:8408-8424`); (4) `restrict_scroll_to_axis` on horizontally-scrollable code blocks.
  **Accept** (`r5-bounce-watch`, `stream` scene): during streaming, poll `tree` ≤300 ms — once a new
    row appears it must remain continuously visible until a newer row appears (no disappear/reappear
  gaps), and the final row is bottom-most at turn end; screenshot strip captured for the owner. *(WP-chat)*
- **#17 — unipi markdown colors.** Copy unipi's `.md` palette (scout-ref B(#17), values verified in
  the archive) into `design/theme.rs` as a markdown palette mapped onto gpui-component
  `TextViewState::markdown` styling: strong `#FFF8EC` w680; inline code cream `#F0F0A8` on
  surface-3; links `#28BEF0` underlined; list markers `#F07818`; blockquote 2px line bar + muted;
  tables 13.5/19, head surface-2; code blocks radius-14 surface-1 + inset ring, 12.5/19 code, mono
  lang label; syntax tokens kw `#F03C32`, string `#5AD25A`, number `#F0C030`, comment dim italic,
  function `#28BEF0`, type `#F0A818`, property `#FAC828`; checkbox accent `#5AD25A`. Do **not** port
  Zed's `crates/markdown` (GPL, heavy — scout-zed §2). Copy-button/headings polish beyond TextView's
  surface is out of scope this round. **Accept:** unit test asserting the palette constants; scenario
  `r5-markdown-palette` (faux markdown answer) captures screenshots for the owner (driver cannot
  assert colors). *(WP-chat)*
- **#25 (conversation half) — contrast inside the conversation.** Apply the §1.2 token floor to
  thinking-block labels, meta rows, query rail, timeline captions. **Accept:** token unit test (AA
  ratios) + screenshots. *(WP-chat; token values owned by WP-shell)*
- **#26 — real logo.** Ship `unipi-logo.png` (512×512, from
  `/home/oi/Projects/Personal/archived/unipi/docs/assets/unipi-logo.png`) as a GPUI asset in
  `design/brand.rs`; render it in the conversation empty state and (WP-shell integration step) the
  sidebar brand row at ~26 px. The grey `CRAB_*` glyphs stay only where the mascot window needs them
  (`components/unicrab.rs`, `views/mascot.rs` untouched). *Ref:* scout-ref B(#26). **Accept**
  (`r5-logo-real`): empty session shows the logo element (id `brand-logo`) in tree + screenshot;
  sidebar row shows it after WP-shell's step. *(WP-chat)*
- **#27 — remove the chat-top refresh button.** Delete the Sessions-column refresh control
  (`components/sidebar.rs:679-682`) and the `refresh_sessions` plumbing (`views/shell.rs:314`);
  the session index already refreshes on session events. Implemented by **WP-shell** (owner of
  `sidebar.rs`/`shell.rs`) as a tagged integration step; acceptance belongs to WP-chat.
  **Accept** (`r5-refresh-gone`): no refresh element in `tree` over the sessions header; creating a
  session still makes it appear in the list without any manual refresh. *(WP-chat verifies; WP-shell edits)*

### Composer

- **#1 — modal model picker (search, full names, fusion, custom lead/sidekick, pins).** Replace the
  popover drill-down (`components/model_menu.rs`, deleted) with `components/model_picker.rs`: a real
  modal (gpui-component `Dialog` + `command`/`searchable_list` delegate, scout-gpui §2/#1) shaped like
  unipi's `ModelPickerSheet` (scout-ref B(#1)): "Now" row; **Single / Fusion** segmented pills;
  search field (always visible on desktop) filtering over `name` **and** `provider/model`; 54 px rows
  with name + mono `provider/id` sub; picked state (14 % accent bg + 3 px accent bar) with sticky
  footer Cancel/Apply. **Pins (Devin/Zed pattern):** star toggle per row; favourites render as the
  first section and persist in `desktop.json` `favorite_models` (the one allowed additive `host.rs`
  hunk). **Fusion tab:** Lead and Sidekick lists contain *every* model (fixes unipi's curated-only
  limitation) plus per-side effort selects; Apply drives the same pair-set path the `/fusion`
  dialog uses today (persistence via `control.set-default-model`, STATUS §1b R4). Opens from the
  composer model chip (click / Ctrl+M) and via a global action the header pill dispatches (shared
  touch point, §7 WP-chat). *Ref:* scout-ref B(#1), scout-zed §3 (mechanism), Devin research
  (pin + family grouping + variant memory → pin + full-name search this round). **Accept**
  (`r5-model-picker-modal`): modal opens (element `model-picker`); search "anth" filters to
  Anthropic rows with full `provider/id` sub; star a row → it appears in the Pinned section after
  close+reopen; Fusion tab picks any lead+sidekick, Apply → chip reads `Lead ◆ Sidekick`; Esc closes. *(WP-composer)*
- **#2 — mode switcher up front; no permission access; no "unipi" text.** Rework the composer inner
  footer (ZCode `V4ComposerModeControls`, scout-ref A(#2/#3)): left cluster = **mode switcher**
  (ghost trigger `h-7` with icon + label + chevron; upward menu, 256 px) listing *agent modes*:
  Regular, Plan, Goal, Graph, Ralph — each icon + name + one-line description, Ctrl+Shift+M cycles,
  rolling label swap on change. It drives the exact commands/topics the chips use today
  (`unipi:workflow:status`, `/plan`, `/goal`, `/graph`, `/ralph`) — nothing new harness-side. The
  permission chip (Ask/Plan/Auto/Full) is **removed** from the composer (approvals live in the
  conversation; level stays settable via `/permission`). The "unipi" text disappears with
  command-enchantment's deletion (WP-extensions) plus a label cleanup in `model/composer.rs`
  completion sections (no raw extension ids as section titles — use the command's own group label).
  *Ref:* scout-ref A(#2/#3), scout-oino #2. **Accept** (`r5-mode-switcher`): switcher visible with
  the five modes; picking Goal sets the goal mode surface (status/chip) without typing a command;
  Ctrl+Shift+M cycles; old permission `mode-chip` absent from tree; no "unipi" substring anywhere in
  the composer tree. *(WP-composer)*
- **#3 — remove the input's outer footer; context inside the input.** Delete the composer context
  header (`components/composer.rs:210-292` — project button, memory count) and every strip below the
  card except the single native glance row (#18). Inside the one rounded card (ZCode
  `ChatPromptEditor` shell: radius-16, 12 px padding, focus border brighten): text area, then inner
  footer = mode switcher (left) + right cluster **context ring** (14 px SVG ring gauge,
  `strokeDashoffset = C·(1−used)`, tones normal/healthy/urgent; hover panel with title, compact
  token label, multisegment bar, per-source rows — ZCode `context.tsx`/`contextUsage.tsx`, scout-ref
  A(#2/#3), *without* ZCode business logic: sources = our context breakdown topics) + model select +
  thought select + stop/send (stop = filled square, tooltip "Stop · Esc"). Placeholder mentions `@`
  and `/`. **Accept** (`r5-composer-context-ring`): no project/memory row below/above the card
  (absent in tree); `context-ring` element present with a value state; model + thought selects
  present; stop shows while busy. *(WP-composer)*
- **#18 — keep bottom-most footer, remove the second.** The "oino / 0 mem…" strip *is* the footer
  extension's frame (scout-oino §2.5/#18) — **deleted with the extension** (WP-extensions). Keep the
  native glance line (`components/meta_row.rs` fallback segments: ctx %, ↑↓, $cost) as the single
  row under the composer. No meta_row code change required this round. **Accept** (checked by
  `r5-composer-context-ring` and WP-extensions' `r5-tui-removal`): exactly one row under the
  composer; no "Oino" wordmark or "0 mem"-class segment in `state.chrome` frames or the tree;
  screenshots for the owner. *(WP-composer verifies; deletion by WP-extensions)*

### Shell / chrome

- **#5 + #13 — missing icons (Automation, cron).** `IconName::CalendarClock` is referenced
  (`components/sidebar.rs:1133`, `views/cron.rs`) but the SVG was never embedded
  (`design/icons.rs` `icon_assets!` list; gpui-kit's default set lacks it too). Add
  `calendar-clock.svg` (lucide `CalendarClock`, 16 px stroke usage) to the app's embedded assets;
  audit the whole `IconName` enum against the embed list in the same pass (any other blank icons get
  their SVG too). **Accept:** unit test that every `IconName` variant resolves to an embedded asset;
  `r5-icons-automations` screenshots the sidebar + Automations page for the owner (driver cannot
  assert pixels). *(WP-shell)*
- **#12 — central host switcher.** New `components/host_switcher.rs` mounted once at the top of the
  sidebar; full design in §5. Per-page host-like pickers cease to exist structurally: the composer's
  project header is deleted (#3), the board keeps only its *project* chip (a project picker, not a
  host picker), session groups stay. **Accept** (`r5-host-switcher`): with no hosts configured the
  switcher reads exactly "Current PC"; a seeded hosts file lists hosts in its menu; selection
  persists; screenshots. *(WP-shell)*
- **#25 (app half) — contrast across the app.** Token floor (§1.2) + audit of sidebar, header strip,
  board empty states, cron, onboarding, extensions page (the `fg_subtlest`-defaulted surfaces,
  scout-oino §2.4). Views owned by other packages (board, settings, conversation) get their pass in
  those packages' steps. **Accept:** AA-ratio unit test over the token table; `r5-contrast-tokens`
  screenshots (sidebar/settings/board) for the owner. *(WP-shell)*

### Kanboard

- **#7 — in-process, no daemon.** Full design in §4. `oino-kanboard serve`, `daemon.json`/lock,
  idle shutdown, token auth and the extension's detached spawn are deleted; the board becomes an
  in-process library service in the desktop and CLI hosts. **Accept** (`r5-board-inprocess`): opening
  the Board page never spawns an `oino-kanboard` process (assert via scenario env + `state`: no
  "Starting the kanboard daemon" surface); board works offline of any HTTP port; CLI `oino-kanboard`
  add/move still works against the same files and the GUI refreshes from it. *(WP-kanboard)*
- **#8 — draggable cards, unipi drag animation.** PC board drag per unipi `cardDrag.ts` +
  `board-desktop.css` (scout-ref B(#8)): 5 px pick-up threshold; ghost = lifted clone
  `rotate(1.5°) scale(1.03)` + large shadow (140 ms ease-out), source card dims to 0.35/scale .98;
  2 px drop-line with hollow cap; hovered valid lane gets 1.5 px accent ring + tint; **refused lanes
  collapse to a 36 px vertical strip** (180 ms, vertical title); edge autoscroll (~80 px zones) both
  axes; source lane pinned via scroll compensation; Esc cancels; final lanes (done/cancelled/archived)
  don't drag. Build on gpui-pre `on_drag`/`on_drag_move`/`on_drop` + `gpui_base::motion`
  (`Transition`, `Easing::cubic_bezier`) — all in-pin (scout-gpui §2/#8). **Requires a driver `drag`
  step** (mouse move with pressed button — currently unsupported, gui-driver §8), added additively by
  WP-kanboard. **Accept** (`r5-board-drag-pc`): drag a card lane→lane → task's lane changes
  (persisted after reopen); mid-drag over a refused lane its width < 60 px in tree bounds;
  screenshot strip mid-drag for the owner. *(WP-kanboard)*
- **#9 — modal formatting.** New-task/detail dialogs rebuilt on gpui-component `form/` (`Form` +
  `Field`, scout-gpui §2/#14) inside the restyled `modal_frame` (WP-kanboard owns `components/dialog.rs`):
  unipi metrics — 44 px action pills, 14 px labels, hairline separators, title 20/26 semibold,
  sticky footer with Cancel + primary (scout-ref B(#1) sheet metrics as the modal grammar).
  **Accept** (`r5-board-modal-form`): FAB → dialog renders `board-new-task-*-input` fields with
  labels/descriptions; screenshots for the owner. *(WP-kanboard)*
- **#10 — phone layout in the side panel, unipi PC board full-page.** Side panel board
  (`components/side_pane.rs`) renders the unipi **phone** layout: lane pill strip (40 px pills,
  status glyph + count, selected = lane-tinted ring) + one lane at a time with its cards. Full-page
  board (`views/board.rs` + `components/ous/kanban.rs`) follows unipi PC `board-desktop.css` as
  exactly as GPUI allows: 288 px lanes / 10 px gaps, 42 px sticky lane heads (glyph + name + count +
  ⓘ tooltip + ⋮), 8 px card gaps, 12 px-radius cards (title 14/20, mono id 11, excerpt 12.5/17,
  140 ms card transitions), inline "Add task" row (32 px), remove the current OUS toolbar pills
  (Filter/Display/search/Move "weird buttons") in favour of unipi's toolbar chips (view toggle,
  scope chip, search chip, Filter, Display, Clear + right-aligned New task). *Ref:* scout-ref B(#10)
  metrics. **Accept** (`r5-board-side-phone`): side-panel board shows lane pills + single lane
  (tree: pill elements, one lane container); `r5-board-drag-pc` screenshots evidence the PC layout;
  no OUS toolbar pill row in tree. *(WP-kanboard)*
- **#11 — overlay shows full text.** Image 3 is unavailable; apply the blanket rule to board
  overlays/menus: no `line_clamp`/`truncate` in the task detail overlay (title, description,
  activity) and board menus wrap instead of truncating (the unipi `AnchoredMenu` `truncate` class is
  the likely culprit, scout-ref B(#11)). The extension side keeps sending the full activity list
  (`extensions/unipi/kanboard/src/panel.rs` currently sends only the last line — change to all lines
  with newest first). **Accept** (`r5-board-fulltext`): a task with a >120-char title/detail opens in
  the overlay with the full text present in `tree` labels (no ellipsis char) and lane card keeps its
  2-line clamp. *(WP-kanboard)*
- **#23 — scroll leaks through overlays.** App-level containment rule, implemented by WP-kanboard in
  the shared overlay components it owns (`components/dialog.rs`, `menu.rs`, `palette.rs`): the
  topmost overlay consumes wheel events (`on_scroll_wheel` + `stop_propagation`, and `.occlude()`
  where a scrim exists) so the board under a search field/overlay never scrolls simultaneously
  (scout-oino §2.3). **Accept** (`r5-scroll-containment`): with the board search field focused,
  injecting `scroll` leaves the lanes' tree bounds unchanged and the field's own scroll consumed;
  if the synthetic wheel is still rejected by GPUI's hitbox (known D11 limit for `list`), prove it
  on Xvfb with before/after screenshots and record the follow-up. *(WP-kanboard)*

### Settings

- **#14 — ZCode settings layout.** Rebuild `views/settings_page.rs` (1,857 lines) on
  gpui-component's `setting/` framework (Apache-2.0, in-pin: `Settings`/`SettingPage`/`SettingGroup`
  + typed fields, scout-zed §4, scout-gpui §2/#14) with ZCode density (scout-ref A(#14)): 268 px
  labelled nav rail (icon rail when narrow), scrollable nav, hairline-divided rows in `rounded-xl`
  cards (`px-4 py-3`, label 14 medium, description same-size subtle, control right-aligned), 24/30
  section titles, **search field** filtering groups (Zed's `filter_table` idea). Layout constants
  live in `settings_page.rs` (not `design/tokens.rs`). **Accept** (`r5-settings-zcode`): nav rail
  width ≈268 in tree bounds; typing "theme" in the search field hides non-matching nav rows/groups;
  mascot toggle still flips (regression from `settings-switches`); screenshots. *(WP-settings)*
- **#19 — Project tab labels the project.** The scope segmented control becomes
  `Global | Project · <folder-name>` (folder name = active session cwd basename); Project scope is
  disabled with a "no open session" hint when no session backend exists (also the #22 no-op fix).
  *Ref:* Zed's `SettingsUiFile::Project` + worktree labels (mechanism, scout-zed §4). **Accept**
  (`r5-settings-zcode` step): with a session open the tab label contains the project folder name;
  without one, Project is disabled with the hint. *(WP-settings)*
- **#20 — no Extensions page in Settings.** Remove the "Manage extensions" nav row and
  `SettingsEvent::OpenExtensions` (the sidebar's Extensions route ⌘3 remains the single surface).
  Per-extension *settings* pages stay (they are settings, not management) and render under the new
  layout. **Accept:** no "Manage extensions" row in settings tree; sidebar Extensions nav still
  routes (`r5-settings-trim` covers both). *(WP-settings)*
- **#22 — every setting does what it says.** With WP-extensions landed first: (a) delete dead app
  settings `sidebar_compact`, `compact`, `hide_thinking` (desktop + its mobile writer) and
  `expand_tools` (key + the dead OR-read in `components/timeline/mod.rs:161` — one-line removal by
  WP-chat, listed in its steps); (b) footer/hints/command-enchantment/input-shortcuts namespaces are
  gone with their crates; (c) `native_wayland` stays, labelled "applies at next launch"; (d) fix the
  silent no-op write (toast "Needs an open session" instead of silent return,
  `settings_page.rs:433-435`); (e) scope shadowing: when a Project-scope value shadows the Global
  one, the row shows a small "set in project" badge (ZCode scope-badge pattern) so Global writes
  don't look dead (scout-oino §3.3 caveats 3–4). **Accept** (`r5-settings-trim`): none of the dead
  keys' rows exist; footer namespace page absent; the wayland row carries the restart note; a
  Project-scoped toggle shows the badge; no-session write toasts. *(WP-settings)*

### Editor

- **#24 — built-in files view / editor with simple LSP + formatting.** Scoped, feasible plan in §6:
  new Files page (tree + editor on the in-pin `gpui-base` editor engine, syntect highlighting),
  thin stdio JSON-RPC LSP client (`lsp-types` 0.97, already in `apps/Cargo.lock`) driving
  diagnostics (squiggles + Problems list), hover, and format-on-save (LSP `textDocument/formatting`
  with `rustfmt` fallback); rust-analyzer is the first/only configured server this round;
  completions are a stretch goal. Zed's editor is **not** taken (GPL + ~300 k LOC transitive, no
  seam — scout-zed §5). **Accept** (`r5-files-editor`, `r5-editor-lsp`): open a repo file, content
  in tree, edit + save persists (reopen shows the edit; toast "Saved"), syntax colors on
  screenshot; a file with a deliberate type error shows the diagnostic (element label contains the
  message); hover over a symbol shows a popover; format-on-save reflows a badly-indented file.
  *(WP-editor)*

## 3. Extension keep / delete list (D1/D3)

From scout-oino §4 (classification evidence) — **delete**:

| Extension / part | Action | One-line reason |
|---|---|---|
| `extensions/unipi/footer` | **delete crate** | TUI status-line chrome; its data already exists natively (glance row, header strip); the #18/#22 no-op source. |
| `extensions/unipi/command-enchantment` | **delete crate** | `/`-completion eye-candy; the "unipi"-look package tags D2 drops; core completions work without it. |
| `extensions/unipi/input-shortcuts` | **delete crate** | alt+h/m/p TUI editor intents; desktop binds chords natively; the mode switcher (#2) supersedes it. |
| `extensions/unipi/app-bridge` | **delete crate** | Presentational control-centre; its unique `service_call` mirroring is dead without `oino serve`; its tabs (BTW/dream/info) already have native surfaces. |
| `extensions/unipi/core` → `hints` module | **delete module** (+ `hints` namespace; `decision-model` too if only hints consumes it — verify by grep) | unipi TUI hint/start-screen chrome; the desktop has its own welcome (reworked by #26). |
| `extensions/unipi/core` → start-header card emission | **delete emission** | Same as above; keep the work tray (GUI-useful) and the `unipi-migrate` skill (harness). |

**Keep** (harness parity or GUI-surfaced function; D3):

workflow, ask-user, subagents, background-tasks, memory, web-api, mcp, compactor, long-horizon,
watchdog, dream, fusion, skill-registry, kanboard (reworked in-process, §4), cron, utility,
`oino.browser`, providers (all 8), btw (side-chat panel), info-screen (`/info` dashboard),
notify (OS notifications), updater (update check + About; its `/readme`,`/changelog` stay — cheap,
harness-neutral), core's work tray. `crates/oino-tui`: frozen (no parity work; deletion candidate
later — not this round, it must keep compiling).

**Settings deleted with them:** namespaces `footer.*`, `hints`/`decision-model` (core),
`command-enchantment`, `input-shortcuts`; app keys `sidebar_compact`, `compact`, `hide_thinking`,
`expand_tools` (WP-settings); kanboard daemon settings `openBrowser`, `requireAuth`, `keepToken`,
`open`/`stopDaemon`/`rotateToken` actions (WP-kanboard). Everything else in scout-oino §3 stays.

## 4. Kanboard in-process design (#7)

Facts (scout-oino §5): all board logic is already a plain Rust library (`crates/oino-kanboard`
lib: model, commands, transitions, file locks, atomic writes, `notify` file-watch); only the
transport is a daemon (axum `serve/`, SSE, token auth, idle shutdown, `daemon.lock`), spawned
detached by the extension, which then talks CLI-subprocess + SSE — the source of the refresh lag.

**Design:**

1. **Desktop host owns the service.** New `KanboardService` in `apps/oino-ui` (WP-kanboard) wrapping
   the library per board-home (`~/.oino/state/unipi.kanboard`, `$OINO_KANBOARD_HOME` honoured): open/list/mutate + a change stream from the lib's `notify`. The native Board views (page + side panel) call it directly — no HTTP, no SSE, no panel republish round-trip; refresh is a file-watch event (also catches CLI/agent writes). Lifetime = app lifetime.
2. **Extension tools go through a WIT host service.** New host interface (e.g. `oino:kanboard/service`:
   `command(board_home, json) -> json` + change subscription) implemented by the desktop host
   (delegating to `KanboardService`) and by the `oino` CLI host (oino-core's LocalServices,
   `crates/oino-core/src/services.rs`, delegating to the lib in-process). `extensions/unipi/kanboard` replaces its `process.exec` CLI calls and
   `follow_sse` with the host service; the daemon spawn (`lib.rs:605-655`), health poll and
   daemon settings/actions are deleted. Reminders (via cron), monitor nudges and the task tools stay.
3. **Delete the daemon surface.** `crates/oino-kanboard/src/serve/` (axum, SSE, auth, idle) and
   `src/daemon.rs` are removed; `main.rs` keeps the non-serve CLI subcommands (add/list/move/… for
   scripts and cron) — safe against the in-process service because both go through the lib's file
   locks, and the service's notify picks up CLI writes.
4. **Packaging** stops requiring the bundled binary for the GUI (it remains shipped for CLI users).

*Rationale for WIT-service over in-process-axum:* it removes the process *and* the HTTP hop, fixes
the known refresh-latency gap (STATUS §1b.10 gap 17), and keeps one code path for both hosts.
WIT is already a breaking-change surface (charter).

## 5. Central host switcher design (#12)

Today the desktop has **no** host picker at all — what exists are per-page *project* pickers, and
`oino serve`/remote attach does not exist yet (scout-oino §6; mobile's host list is gated on it).
So #12 is a green-field build plus structural de-duplication:

- **One switcher, sidebar top** (`components/host_switcher.rs`): a chip reading the active host —
  exactly **"Current PC"** when no hosts are configured — with a monitor icon and a chevron; menu
  lists configured hosts (name + `user@host` sub + connection dot) with the active one checked,
  "Current PC" always present, and "Add host…" opening a small form (name + SSH target). Modelled
  on ZCode's centralized identity (sidebar device header + Remote devices settings section,
  scout-ref A(#12)); pages never pick hosts.
- **Persistence:** `hosts.json` next to `desktop.json` (own store file keeps `host.rs` ownership
  clean, §1.3; same shape as mobile's list). Empty/missing file ⇒ "Current PC".
- **Honest remote scope:** selecting a remote host this round stores the selection and toasts
  "Remote agents need `oino serve` (coming later) — staying on Current PC"; the app keeps running
  locally. Real attach lands with `oino serve` (STATUS known issue 4), at which point the switcher
  is already the single mount point.
- **De-duplicated pickers:** composer context header deleted (#3); board keeps its *project* chip
  (project ≠ host); session groups unchanged.

## 6. Editor approach (#24) — scope and justification

- **Not Zed's editor.** `crates/editor` is 190,699 LOC / 72 deps and GPL-3.0-or-later, coupled to
  `project`/`workspace` with no widget seam (scout-zed §5) — both a license and a weight rejection.
  D4 is satisfied by the Apache-2.0 **same-lineage engine already compiled into `apps/`**:
  `gpui-base 0.8.1` `input/editor/` (Zed-lineage editor: auto-close, indent, in-editor search,
  `DiagnosticSet` squiggles, LSP provider traits `CompletionProvider`/`HoverProvider`/…,
  `apply_lsp_edits`) + `gpui-component` `highlighter` (syntect enabled today; tree-sitter later) and
  `tree` (scout-gpui §2/#24).
- **This round's scope:** a Files page (`views/files.rs`): left tree (lazy dirs) + editor pane;
  open/edit/save; syntect highlighting; **rust-analyzer over a hand-rolled stdio JSON-RPC client**
  (~300–500 LOC, `lsp-types 0.97` already in the lock — keeps deps minimal per charter):
  `didOpen`/`didChange`, `publishDiagnostics` → `DiagnosticSet` + a Problems list, hover popover,
  format-on-save (`textDocument/formatting`, `rustfmt` fallback); server discovery = `rust-analyzer`
  on PATH, gated by a settings toggle. Reference glue to port: `longbridge/gpui-kit`
  `examples/editor/src/main.rs` (Apache-2.0, 1,208 lines; `apply_lsp_edits` at :312 is the formatting
  path). **Stretch:** LSP completions through the engine's completion menu.
- **Deferred (say so, don't sneak it):** tree-sitter grammars (compile-time weight), the `dock`
  workbench shell, multi-server config, non-Rust servers, the side-pane Files tab upgrade (it keeps
  the read-only viewer; an "Open in editor" affordance is a later polish).
- **Justification:** one strong worker (swe-coder) can land tree + editor + diagnostics + hover +
  format in a round; completions and grammars are the natural round-6 slice once the LSP client is
  proven against rust-analyzer on `ti` (the scout's recommended spike becomes the first step).

## 7. Work packages

Seven packages, already scheduled as workers; names are fixed. Ownership is **disjoint**; the
shared touch points list names the owner for every shared file. All packages: run the charter gates
for the workspaces they touch; keep `docs/dev/v1/` in sync; commit with their `[wp-…]` prefix.

**Sequencing:** WP-extensions lands first (unblocks WP-settings; its prunes are the base everyone
rebases on). Then WP-shell, WP-chat, WP-composer, WP-kanboard in parallel; WP-settings after
WP-extensions; WP-editor after WP-shell. `qa-r5` (glm-review) + `qa-r5-fix` + `round5-integration`
(alpha.5) close the round; integration assembles `docs/dev/v1/gui/suites/r5.json`, updates STATUS.md
and prunes now-dead constants (e.g. `SETTINGS_*` in `design/tokens.rs`).

### Shared touch points (owner named)

| File / area | Owner | Others |
|---|---|---|
| `apps/oino-ui/src/design/tokens.rs`, `type_scale.rs`, `icons.rs` | WP-shell | consume only |
| `apps/oino-ui/src/design/theme.rs`, `syntax.rs`, `brand.rs` | WP-chat | consume only |
| `apps/oino-ui/src/host.rs` (`AppSettings`) | WP-settings | WP-composer: exactly one additive hunk (`favorite_models`) |
| `apps/oino-ui/src/views/mod.rs` (routes/actions) | WP-shell | WP-editor adds the Files route after WP-shell lands |
| `apps/oino-ui/src/automation/*`, `scripts/gui-drive.py`, `docs/dev/v1/gui/scenarios/`, `gui-driver.md` | additive-only for all | WP-kanboard adds the `drag` step + doc |
| `components/dialog.rs`, `menu.rs`, `palette.rs` | WP-kanboard | consume only (scroll-containment rule lands there) |
| String-keyed references to removed extension ids | WP-extensions may prune anywhere (bounded grep) | do not revert prunes |
| `apps/Cargo.toml`, `apps/oino-ui/Cargo.toml` | WP-editor (deps for #24) | — |

### WP-extensions (ds-coder) — D1/D3 removals

Mechanical, fully decided. **Issues:** D1/D3 (feeds #2 label source, #18 strip, #22 namespaces).

**Owns:** `extensions/unipi/{footer,command-enchantment,input-shortcuts,app-bridge}/` (delete),
`extensions/unipi/core/` (hints removal), `extensions/sdk/unipi-kit/src/suite.rs`,
`extensions/dist/` + checksums, `crates/oino-wasm-host/tests/unipi_settings.rs`,
`crates/oino-core/tests/gui_feeds.rs`, `crates/oino-bundle` (if it names packages),
`apps/oino-desktop/src/demo.rs`, `apps/oino-ui/src/model/status.rs`,
`apps/oino-ui/src/components/dock.rs`, docs listed below.

**Ordered steps:**
1. Inventory: `rg -n "unipi\.(footer|command-enchantment|input-shortcuts|app-bridge)" crates apps extensions scripts docs/dev/v1` — every hit is in scope.
2. Delete the four crates (dirs + any workspace/manifest references).
3. `extensions/unipi/core`: delete `src/hints/`, the `hints` namespace (+ `decision-model` iff
   grep shows only hints consumes it), the start-header/hint-strip emission and the harness tests
   covering hints; keep `tray.rs`, the migrate skill, load position.
4. `suite.rs`: remove the four `packages!` rows + core's dropped namespaces; move the ids to
   `NOT_PORTED` with one-line D1 reasons; fix the `load_order_is_umbrella_order_with_core_first`
   test.
5. Prune crates tests (`unipi_settings.rs` footer entries, `gui_feeds.rs` input-shortcuts block) and
   any `oino-bundle` package lists.
6. Apps string-keyed prunes: `demo.rs` (footer/app-bridge/input-shortcuts simulation blocks — the
   glance falls back to native segments), `model/status.rs` (`unipi:footer:*` mappings, footer/
   process filters), `components/dock.rs` (footer process line), plus the bounded hits in
   `model/composer.rs` (tag test) and `components/side_pane.rs:280` (app-bridge branch) — the only
   edits WP-extensions makes inside another package's files.
7. Rebuild artifacts: `cargo xtask wasm` (39 → 35 components) + `cargo xtask checksums`.
8. Docs & scenarios: `unipi-port.md` (mark the four + core-hints removed, D1 reason),
   `gui-driver.md` scenario tables (drop the four rows), delete
   `docs/dev/v1/gui/scenarios/unipi-{footer,command-enchantment,input-shortcuts,app-bridge}.json`,
   update `docs/dev/v1/gui/suites/unipi.json`; update `status-panel.json` if its Git-row assertion
   depended on `unipi:footer:git` (that row disappearing is an accepted D1 consequence — record it
   in the report).
9. Gates: extensions workspace (fmt/clippy/tests + wasm/checksums), root (touched crates), apps
   (demo/status/dock compile + tests).
10. New scenario `r5-tui-removal`: Extensions page lacks the four ids; `/` completions carry no
    package tags; `alt+h` is inert (no hint strip); `state.chrome` has no footer-brand segments.

**Acceptance:** `r5-tui-removal` PASS; `cargo xtask gui suite unipi` green with 24 scenarios (28 − 4);
all gates green; grep from step 1 returns only intentional doc-history hits.

### WP-chat (swe-coder) — conversation quality

**Issues:** #15 #16 #17 #21 #26 #27 (+ #25 in-conversation).

**Owns:** `apps/oino-ui/src/views/session_pane.rs`, `components/conversation.rs`,
`components/header.rs`, `components/timeline/**`, `components/keys.rs`, `components/query_rail.rs`,
`components/toast.rs`, `design/theme.rs`, `design/syntax.rs`, `design/brand.rs` (+ logo asset).

**Ordered steps:**
1. #21 list discipline: kill full-`remeasure()` during streams (`session_pane.rs:1202` path);
   add `set_item_measurement_readiness` gating (or reserved provisional heights) for async-growing
   rows; splice the generating strip as a list item; `restrict_scroll_to_axis` on code blocks.
2. #16: module doc in `conversation.rs` stating the virtualization contract (events → minimal list
   ops; follow-tail rules).
3. #15: two-stage Esc in `session_pane` (arm → toast via `toast.rs` → abort), with dialog/menu guards.
4. #17: markdown palette constants (§2 values) in `design/theme.rs` + `syntax.rs`, wired into
   `TextViewState::markdown` styling; unit test on constants.
5. #25-in-conv: re-map low-contrast roles in timeline/keys/query_rail to the new token floor;
   remove the dead `expand_tools` read (`timeline/mod.rs:161`, one line, per WP-settings #22).
6. #26: logo asset + empty-state render (`brand-logo` element).
7. #27: verify after WP-shell's removal step; write `r5-refresh-gone`.
8. Scenarios + screenshots; gates for `apps/`.

**Acceptance:** `r5-esc-twice-stop`, `r5-bounce-watch`, `r5-markdown-palette` (unit test +
screenshots), `r5-logo-real`, `r5-refresh-gone` — all PASS; regression scenarios
`new-session-prompt`, `approval-allow`, `queue-edit`, `zcode-row-actions` stay green.

### WP-composer (glm-lead) — composer & model picker

**Issues:** #1 #2 #3 #18.

**Owns:** `apps/oino-ui/src/components/composer.rs`, `components/model_menu.rs` (delete),
new `components/model_picker.rs` + `components/mode_switcher.rs`, `model/composer.rs`,
`components/meta_row.rs` (verify-only this round), the one additive `host.rs` hunk.

**Ordered steps:**
1. #3 shell: one rounded card (radius 16, 12 px pad, focus brighten), delete the context header row
   (`composer.rs:210-292`) and any strip below except the glance row; placeholder with `@`/`/`.
2. #3 context ring: 14 px ring gauge + hover panel (title, token label, multisegment bar, per-source
   rows from our context topics); tones normal/healthy/urgent.
3. #2 mode switcher: trigger + upward 256 px menu (Regular/Plan/Goal/Graph/Ralph with icons +
   descriptions), Ctrl+Shift+M cycle, rolling label; drives existing topics/commands; permission
   chip removed; completion section labels de-extensioned.
4. #1 model picker modal: `model_picker.rs` per §2 (search over name + `provider/model`, Now row,
   Single/Fusion pills, 54 px rows, picked state, sticky footer, star pins persisted in
   `favorite_models`, fusion = any lead/sidekick + efforts via the existing pair-set path);
   `model_menu.rs` deleted; composer chip (Ctrl+M) and the header-pill action open it.
5. #18: verify single glance row after WP-extensions (integration-time if parallel).
6. Scenarios; gates.

**Acceptance:** `r5-model-picker-modal`, `r5-mode-switcher`, `r5-composer-context-ring` PASS;
`zcode-model-menu` scenario is replaced by `r5-model-picker-modal` (delete the old one);
`new-session-prompt` stays green.

### WP-shell (glm-flash-worker) — shell, sessions, hosts

**Issues:** #4 #5 #6 #12 #13 + #25 (app half).

**Owns:** `apps/oino-ui/src/views/shell.rs`, `views/mod.rs`, `views/onboarding.rs`, `views/cron.rs`,
new `views/new_chat.rs` + `components/host_switcher.rs`, `components/sidebar.rs`,
`design/tokens.rs`, `design/type_scale.rs`, `design/icons.rs` (+ SVG assets).

**Ordered steps:**
1. Tokens first (§1.1–1.2): ZCode text/motion constants + contrast floor incl. `idle_task`,
   `attention_fg`, `primary_fg`; `IconName`-vs-assets unit test; add `calendar-clock.svg` (+ any
   other missing SVGs found) — #5/#13.
2. #4: `open()` dedup for `OpenRequest::New` (empty-session + same-cwd ⇒ activate).
3. #6: `views/new_chat.rs` (greeting, logo, project picker, "No project", "Choose folder…"),
   routed from sidebar `+`/⌘N.
4. #12: `components/host_switcher.rs` + `hosts.json` (§5), mounted at sidebar top.
5. Integration micro-edits for WP-chat: sidebar brand row renders `design/brand.rs` logo (#26);
   remove the sessions refresh button + `refresh_sessions` plumbing (#27).
6. #25 audit over owned views; scenarios; gates.

**Acceptance:** `r5-no-dup-sessions`, `r5-new-chat-page`, `r5-host-switcher`, `r5-icons-automations`
(screenshots), `r5-contrast-tokens` (unit test + screenshots) PASS; `settings-switches`,
`side-pane-tabs`, `zcode-palette-history` stay green.

### WP-settings (glm-lead; after WP-extensions) — settings rebuild

**Issues:** #14 #19 #20 #22.

**Owns:** `apps/oino-ui/src/views/settings_page.rs`, `components/settings.rs`, `host.rs`
(`AppSettings`), `mobile/settings.rs`.

**Ordered steps:**
1. Rebuild on gpui-component `setting/` with ZCode layout (§2 #14): rail/groups/rows/search; local
   layout constants.
2. #19 scope labels (`Global | Project · <name>`, disabled-with-hint when no session).
3. #20 remove "Manage extensions" row + `OpenExtensions` event.
4. #22: delete dead keys (`sidebar_compact`, `compact`, `hide_thinking`, `expand_tools`), restart
   note on `native_wayland`, no-session write toast, project-scope "set in project" badge.
5. Re-verify every remaining setting does something (scout-oino §3 table is the checklist);
   scenarios; gates.

**Acceptance:** `r5-settings-zcode`, `r5-settings-trim` PASS; `settings-switches` and the
`unipi-workflow`/`unipi-notify`/`unipi-mcp` settings-hub steps stay green.

### WP-kanboard (swe-coder) — in-process board

**Issues:** #7 #8 #9 #10 #11 #23.

**Owns:** `crates/oino-kanboard/**`, `extensions/unipi/kanboard/**`, `wit/` (new host
service) + `crates/oino-wasm-host/src/**` (host impl) + `crates/oino-core/src/services.rs`
(CLI-host service registration — different file from WP-extensions' `oino-core` test
prunes), `apps/oino-ui/src/views/board.rs`, `components/ous/**`, `components/side_pane.rs`,
`components/dialog.rs`, `components/menu.rs`, `components/palette.rs`, driver `drag` step.

**Ordered steps:**
1. Driver first: `drag` step (MouseDown → MouseMove-with-pressed → Up) in `automation/` +
   `gui-drive.py` + gui-driver.md §2/§8 update — everything else is verified through it.
2. #7 in-process service per §4: delete `serve/` + `daemon.rs`; `KanboardService` in `apps/oino-ui`;
   WIT host service + both host impls; extension switches to it (delete spawn/SSE/CLI-subprocess,
   delete daemon settings/actions).
3. #10 layouts: PC board per unipi metrics; side panel phone layout; toolbar chips replace OUS pills.
4. #8 drag UX per §2 (ghost, collapse-to-strip, drop-line, autoscroll, pin, Esc).
5. #9 modal rebuild on `form/`; #11 no-truncate rule + full activity list from the extension.
6. #23 scroll containment in `dialog.rs`/`menu.rs`/`palette.rs`.
7. Scenarios; gates (root + extensions + apps — WIT regen `cargo xtask wit`).

**Acceptance:** `r5-board-inprocess`, `r5-board-drag-pc`, `r5-board-side-phone`,
`r5-board-modal-form`, `r5-board-fulltext`, `r5-scroll-containment` PASS; `board-task`,
`unipi-kanboard` (updated for the native board) stay green.

### WP-editor (swe-coder; after WP-shell) — files view + editor + LSP

**Issues:** #24.

**Owns:** new `apps/oino-ui/src/views/files.rs`, new `apps/oino-ui/src/editor/**` (editor glue +
LSP client), `apps/Cargo.toml` + `apps/oino-ui/Cargo.toml` (deps), the additive Files route in
`views/mod.rs`.

**Ordered steps:**
1. Spike on `ti` (or local Xvfb): rust-analyzer over the `gpui-base` provider traits via the ported
   `examples/editor` glue — prove diagnostics before building the page (scout-gpui risk #1).
2. Files page: tree (lazy dirs) + editor pane, open/edit/save, syntect highlighting, dirty state.
3. LSP: stdio JSON-RPC client (`lsp-types`); didOpen/didChange; publishDiagnostics → `DiagnosticSet`
   + Problems list; hover popover; format-on-save (LSP → `rustfmt` fallback); settings toggle;
   completions as stretch.
4. Scenarios (`r5-files-editor`, `r5-editor-lsp` — rust-analyzer installed on the test host);
   gates.

**Acceptance:** both scenarios PASS (§2 #24 assertions); no regression in existing suites; the
side-pane read-only Files tab unchanged.

## 8. Open questions & follow-ups

- **Owner-visible checks needing screenshots** (driver can't assert pixels): #17 palette, #21
  bounce-strip, #5/#13 icons, #14 layout, #9 modal, #8 drag animation, #25 contrast — each package
  files screenshots under `docs/dev/v1/design/shots/gui/r5-*/` for the owner's pass.
- **Remote hosts** stay inert until `oino serve` exists (§5) — tracked as the follow-up to STATUS
  known issue 4, not a round-5 deliverable.
- **Accepted D1 losses:** footer-fed git-branch/status segments (native surfaces keep model/ctx/cost);
  hint strip/start card (replaced by the native welcome, #26).
- Round closure: `qa-r5` → `qa-r5-fix` → `round5-integration` (alpha.5): r5 suite manifest, STATUS
  refresh, `design/tokens.rs` dead-constant prune, this file's status line.
