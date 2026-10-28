# Oino Desktop + Mobile — design system and parity spec

Status: **binding** (work id `design-spec`, 2026-10-26). Owners of the implementation:
`design-desktop` (`apps/oino-ui` shared components + `apps/oino-desktop`) and `design-mobile`
(`apps/oino-mobile` + `apps/oino-ui/src/mobile/`). This document **supersedes**
[gpui-app.md](gpui-app.md) §5 (UX specification) wherever the two differ; gpui-app.md stays
binding for architecture, threads, bundling, build matrix and acceptance. Protocol facts come
from [ui-protocol.md](ui-protocol.md); unipi surfaces from [unipi-port.md](unipi-port.md).

Two parity targets, one rule each:

1. **ZCode drives the look and the navigation.** Reference: `github.com/zai-org/ZCode` @
   `3c5b2276` (v3.16.2, Apache-2.0), cloned to `/tmp/zcode-src`; read `DESIGN.md`,
   `packages/ui/src/styles.css` (Zai Light / Zai Dark), `docs/ui/*.md`, `docs/design/*.md`,
   `packages/ui/src/v4/*`. UX and token values are borrowed; **no code is copied**.
2. **unipi drives feature coverage.** Every unipi 3.0.0 UI surface (unipi-port §3) has a
   first-class native rendering listed in §7. Semantics of unipi-specific surfaces (Unicrab
   states, board lanes, needs-you) follow `/home/oi/Projects/Personal/archived/unipi/unipi-app/DESIGN.md`
   §1.3, §1.5, §3.9 — but rendered in the ZCode visual language, not in the unipi app palette.

### Amendment A1 (owner direction, 2026-10-27) — overrides anything below that conflicts

> A1.3 (ZCode owns layout and details) is withdrawn for look and layout by **Amendment A2**
> (unipi look and feel, no multiplexer concepts). Read A2 first.

1. **Priority: unipi first, ZCode second.** When the two disagree, unipi wins.
2. **unipi owns theming and features.** The palette, brand and state language come from
   `/home/oi/Projects/Personal/archived/unipi/unipi-app/DESIGN.md` §1.1–§1.6, not from Zai:
   - Oino Dark (default) = unipi warm OLED surfaces (`bg #0A0908`, `surface-1/2/3`, `line`,
     `text`, `text-muted`, `text-dim`), primary = `shell #F03018` (hover `shell-hi`), secondary
     accent `claw`, badges `amber`, highlights `gold`, caret/focus glow `cream`, destructive
     `deep`; semantic info/success/warning/error = wordmark `wm-cyan/green/yellow/red`; ember
     glow behind the welcome header. Text on primary = `face #181818`.
   - Oino Light: derived from the same hues (warm off-white surfaces, same shell primary and
     wordmark semantics, contrast ≥ 4.5:1). unipi had no light theme; this is ours.
   - Agent state colour + icon + word table (unipi §1.3: needs-you/working/done/idle/error/
     offline) is used everywhere a session or agent state shows (sidebar rows, header, status
     panel, toasts, tabs). The `waiting_surface` treatment of §2 becomes unipi needs-you.
   - Shapes from unipi §1.4/§1.6: cards 16, dialogs/menus 16, sheets 24 (top), inputs 12,
     buttons/chips/toolbar capsules pill-shaped; uppercase muted section labels (12/500/.1em);
     focus = 2 px shell outline offset 2. Inter + JetBrains Mono and Lucide 1.5 stay.
   - The animated Unicrab rig (unipi §1.5.1: six states, part tracks, blink, reduced motion)
     is the needs-you indicator in the desktop header and the mobile floating crab, plus the
     opt-in desktop **mascot window** (transparent, always-on-top, bubble) — all native GPUI.
   - Every existing unipi feature surface (§7 mapping) must be present and themed this way.
3. **ZCode owns UX, layout and the tiny details**: frames, navigation, sidebar, header,
   timeline rows, composer, docked interaction cards, status panel, side pane, palette, model
   menu, keyboard behaviour, micro-interactions, density, empty/loading states. §2 Zai colour
   values are replaced by A1.2; all non-colour ZCode rules in §2–§6 stay binding.
4. **Embedded browser is in scope (was Diverge).** Use `gpui-webview` (longbridge/gpui-kit
   `crates/webview`, Wry-based; WKWebView on macOS, WebView2 on Windows, WebKitGTK on Linux).
   Bump the apps workspace to gpui-kit 0.8.1 for it. Linux: gpui-webview hosts WebKitGTK in an
   X11 child window, so Oino Desktop on Linux runs on X11 / XWayland when the browser feature
   is on (default on; a setting "Native Wayland (disables embedded browser)" switches back).
   Enable the `gpui-fast` composition feature where supported so overlays draw above the page.
   Placement: side-pane **Browser** tab (URL bar, back/forward/reload, open externally, devtools
   in debug); `openUrl` from extensions and links in the transcript open there; local dev
   server previews (`localhost:*`) are detected from bash output and offered as a chip.
   Mobile: in-app browser via the OS (Android Custom Tabs, iOS SFSafariViewController).
5. **Interactive terminal is in scope (was Diverge).** Bottom frame **Terminal** tab with real
   PTYs (tabs, split, copy/paste, links, 10k scrollback, theme from tokens), built like Zed:
   `alacritty_terminal` for emulation + `portable-pty`/the platform PTY, rendered with GPUI text
   runs. Desktop only in v1; mobile shows Output/Tasks logs.
6. Checklist §8 rows 41–43 are therefore **Match**; the only Diverge left is row 50.

Reference images (small PNGs, 800 px wide) live in [design/ref/](design/ref/):

| File | What |
|---|---|
| `zcode-welcome-dark.png` | ZCode new-task screen (greeting, composer with project/branch header, mode chip, model + thinking, suggested tasks) — from zcode.z.ai docs |
| `zcode-model-menu-dark.png` | ZCode model menu (provider group label + badge, check mark, provider submenus, "Manage models") |
| `zcode-workspace-dark.png` | ZCode workspace (sidebar nav, grouped sessions, header chips, one-line tool rows, floating Progress status panel) — zcode.z.ai social card |
| `oino-current-approval.png` | Oino Desktop today (`--demo`): modal approval over the transcript |
| `oino-current-done.png` | Oino Desktop today: finished run, info-screen panel in the side pane |

### Amendment A2 (owner direction, 2026-10-27) — unipi look and feel; overrides A1 and §1–§10 where they conflict

Owner direction (binding):

1. **The desktop app looks and feels close to UNIPI first.** ZCode parity comes later, and only
   where it doesn't change the unipi look. A1.3 ("ZCode owns UX, layout and the tiny details")
   is **withdrawn for look and layout**. ZCode keeps only behaviour that has no unipi
   counterpart: query rail, find bar, inline diff, model menu structure and turn file
   footer. These are restyled with the A2 tokens.
2. **Oino Desktop is a fully native, single-machine app.** It has **no herdr, tmux, workspace,
   pane or multiplexer concepts**. A session is one Oino agent conversation running in this
   process. Pairing a phone or another PC is the only remote idea, and it is **later** (§A2.18).
3. Sources (in order): `archived/unipi/unipi-app/DESIGN.md` §1.1–§1.6 and §3.4, §3.6, §3.9,
   §5.8; the real UI code `unipi-app/apps/mobile/src/{ui.css,style.css,components/shell/shell.css,
   screens/chat.css,components/mascot.css,components/*,screens/*,mascotWindow.tsx,lore.ts}`;
   milestone captures `unipi-app/docs/m5…m9`; TUI shots `archived/unipi/docs/assets/screenshots/`.
   CSS numbers below are measured from that code. Where unipi has no rule, A1 and §2–§6 still
   apply.

#### A2.0 Reference shots — [design/ref/unipi/](design/ref/unipi/)

The `live-*` shots were captured on 2026-10-27 from the real unipi web UI. It ran under
`vite` (port 1488) with the repo's own mocked Tauri runtime (`docs/m7/capture-shell.py`, plus
playwright and chromium in headless mode). The other shots are copies of unipi's milestone
captures. All files are ≤ 1000 px wide and use a 128-colour PNG palette.

| File | Shows |
|---|---|
| `live-desktop-sessions-needs-you.png` | Desktop shell: 224 px sidebar (brand, search, nav), page header (title 30/36, tools, header crab), amber **needs-you strip**, section labels, cards, grouped rows |
| `live-desktop-chat-list.png` | list \| detail: grouped session rows (icon tile, state word, cwd), compact pane placeholder |
| `live-desktop-chat-open.png` | **Chat view**: 56 px detail header with session strip, harness cards (Memory), user bubble, folded "Ran commands · 2 steps" line, prose, code block, turn end, "Changes this turn", **composer** |
| `live-desktop-palette.png` | Command palette (620 wide, 16 radius, section labels, 40 px rows, shell-tinted selection, kbd chips, footer legend) |
| `live-desktop-settings.png` | Settings: section nav column, section label with icon, grouped card rows with icon tile + switch |
| `live-phone-sessions.png`, `live-phone-chat.png` | Phone: header without lead row, bottom nav, phone chat with the strip band and composer |
| `m7-desktop-board.png`, `m7-desktop-board-task.png` | Board: project title picker, toolbar pills, lane pill strip, lane headers with about text, cards, dashed empty lane, Add task, FAB |
| `m7-desktop-dialog-sheet.png`, `m7-desktop-anchored-menu.png` | Sheet → centred dialog (560, 16 radius, sticky footer), anchored menu at a card |
| `m7-desktop-browser-pane.png` | Browser side pane right of the chat |
| `m8-desktop-header-crab*.png`, `m8-desktop-needs-you-drawer.png`, `m8-desktop-settings-unicrab.png` | Header crab (idle / needs-you badge), needs-you drawer, Unicrab settings |
| `m8-mascot-window-bubble.png`, `m8-unicrab-states.png`, `m8-phone-floating-crab-bubble.png` | Mascot window with bubble, the six crab states, phone floating crab |
| `m6-phone-dialog-card.png` | Interaction card that takes over the composer slot (eyebrow, countdown, numbered options, own-answer row, Send) |
| `m6-phone-chat-welcome-empty.png`, `m6-phone-header-strip.png`, `m6-phone-control-centre.png`, `m6-phone-queue-strip.png` | Empty session, header + strip, control centre (model/fusion), queue strip |
| `m5-desktop-mode-chip.png`, `m5-desktop-mode-popover.png` | Working-mode chip (tinted, live count) and its anchored popover |
| `tui-glance-footer.png`, `tui-info-usage.png`, `tui-unicrab-start.png` | TUI references: glance frame + stats strip, info dashboard, start header |

The Oino side of the comparison is in `docs/dev/v1/gui-audit/04-welcome.png` and
`09-run-tools.png`.

#### A2.1 Key look decisions (what changes vs today)

| # | Decision | Replaces |
|---|---|---|
| L1 | **Flush panes, no floating frames.** Window bg `bg #0A0908`; sidebar, list column, chat and side pane are flush columns separated by 1 px `line_soft` rules. No 4 px gaps, no frame radius, no frame borders | §3.1 independent frames, §2.6 frame `R_XL` |
| L2 | **Unipi desktop shell**: a 224 px nav sidebar (brand crab + "Oino", search field, nav items). Sessions is a **list \| detail** screen (session list column 400, then the chat). There is no session tree in the sidebar | §4.7 sidebar groups/view switcher, §3.1 layout |
| L3 | **One page header** for Sessions, Board, Extensions and Settings: a 44 px lead/tools row, then title 30/36/600, then a 13 px meta line. **One detail header** (56 px) on top of every detail view (chat, task, review, browser, file) | §4.8 WorkspaceHeader |
| L4 | Pills everywhere: buttons, chips, toolbar capsules, send. Cards use radius 16, dialogs/menus/palette 16, phone sheets 24 (top corners), inputs 12, rows 10 | §2.6 nesting rule, §4.1 sizes |
| L5 | **Chat = deepseek-harness look** (unipi §3.4): a right user bubble (radius 20, `surface-3`), assistant prose with no bubble (15/24), one **folded line per turn** for thinking and tools, harness cards collapsed to one line, turn end "Done in 1s · $0.001", and a "Changes this turn" card | §4.9 timeline rows (row list kept, look replaced) |
| L6 | **Composer = one surface-2 shell, radius 26.** It holds the placeholder "Message Oino, / commands, @ files". The toolbar capsule is: attach circle 34 · mode chip · model chip (with context ring) · send 38 shell circle. Interaction cards **take the composer slot** (they replace the composer and keep the draft) | §4.11, §4.12–4.15 "dock above" |
| L7 | **Needs-you everywhere = Unicrab.** These all show it: the header crab in every page header's tools, the amber needs-you strip on Sessions, state words in rows, the window title "● Oino", the opt-in mascot window, and the floating crab on the phone (re-added; §6.11 "dropped" is overridden) | §4.28 glyph-only, §6.11 |
| L8 | **Empty states = crab or icon tile + one line.** A lore line appears only where there is nothing to do. The pixel Unicrab is drawn only in mascot moments (empty states, splash, header crab, mascot). The big Unicrab start header is **not** pinned above sessions (GA-5) | §4.28, §5.1 hero |
| L9 | Section labels: 12/500, uppercase, `text-dim`, written in sentence case in the source (it is uppercased in code, never typed in caps) | §4.4 group label, palette sections |
| L10 | No ZCode floating status panel by default. Long-horizon/plan/run state shows as the **working-mode chip** in the chat header and a "Progress" sheet. The floating StatusPanel stays as an opt-in (Settings → Appearance → Status panel: Off (default) / Mini / Expanded) | §4.19 default Auto |
| L11 | Focus: 2 px `shell` outline offset 2 on keyboard focus; inputs get a 1.5 px inset `shell` ring that follows the radius; a dialog with no autofocus field focuses itself (no ring on open) | §2.6 calm border-step focus |
| L12 | Wording: unipi's. **PC** is the one noun for a machine. "click / right-click". Never "pane", "workspace", "tab" (for a session), "attach", "multiplexer" | — |

#### A2.2 Palette (Oino Dark = unipi; replaces the A1.2 field values that differ)

The CSS custom properties (`style.css @theme`, `ui.css`) map to `design::tokens::Palette`
fields. Values are `0xRRGGBBAA`. **Bold** marks a change from today's `tokens.rs` DARK.

| unipi CSS | Value | `Palette` field(s) |
|---|---|---|
| `--color-bg` | `#0A0908` | **`window` = `0x0A0908FF`**, `frame`, `header`, `tab_active` |
| shell desktop bg glow `radial-gradient(circle at 0 0, shell 5 %, transparent 520px)` | — | `glow = 0xF030180D`. GPUI has no radial gradient: draw a 520 × 520 top-left quad with `linear_gradient(135°, glow → transparent)`. Never fill with a solid tint |
| `--color-surface-1` `#141211` | | `card`, `tab`, palette and dialog bg (**new `sheet = 0x141211FF`**), tool body, harness card hover |
| sidebar `rgb(20 18 17 / 72%)` | | **`sidebar = 0x141211B8`** (over `window`) |
| `--color-surface-2` `#1C1917` | | `input` (composer, fields), `popover`, `menu`, **`hover = 0x1C1917FF`** (row hover is opaque surface-2, not 5 % white), **`chip = 0x1C1917FF`** (new) |
| `--color-surface-3` `#26211F` | | `secondary` (secondary button), `tag` (inline code bg, kbd), **`bubble = 0x26211FFF`** (new), `menu_hover`, dialog option rows |
| `--color-line` `#2E2825` | | `border`, `card_border`, `input_border`, `popover_border` |
| `rgb(46 40 37 / 70%)` | | **`line_soft = 0x2E2825B3`** (new): sidebar right rule, detail header bottom rule, pane splits |
| `rgb(46 40 37 / 60%)` | | **`row_divider = 0x2E282599`** (new): inset rules between rows in grouped cards (inset left 58 / 14) |
| `--color-text` `#F5EFE6` / `--md-strong-color #FFF8EC` | | `fg`; **`fg_strong = 0xFFF8ECFF`** (new; markdown bold at weight 680→`SEMIBOLD`) |
| `--color-text-muted` `#A39A90` | | `fg_subtle`, `thinking` |
| `--color-text-dim` `#6E655D` | | `fg_subtlest` |
| `--color-shell` / `shell-hi` | `#F03018` / `#F01818` | `primary`, `primary_hover`, `brand`, `focus`, `input_border_focus` |
| `--color-face` `#181818` | | `primary_fg`, `fg_inverse` |
| `--color-claw` `#F07818` | | `working`; interaction card accent (eyebrow, selected option ring); markdown list markers (**new `list_marker`**); `$` prompt in bash |
| `--color-amber` `#F0A818` | | `amber`: badges, counters, **needs-you strip** text/border, queue kind tag |
| `--color-gold` `#F0C030` | | `gold`: selection marks, "Show all" links, goal mode tint; `selection` = gold at 25 % |
| `--color-cream` `#F0F0A8` | | `cream`: caret, inline code text (`--md-code-fg`), mascot eyes |
| `--color-deep` `#C00018` | | `deep`: destructive buttons with icon + label |
| `wm-cyan` `#28BEF0` | | `info`, links (**`link`** new), file nodes, transport/"web" badges, graph mode tint, In Review lane |
| `wm-green` `#5AD25A` | | `success`, done/online dots, context ring fill, ralph tint, `diff_added` |
| `wm-yellow` `#FAC828` | | `warning` |
| `wm-red` `#F03C32` | | `danger` (error **colour**, never a primary button), `diff_removed` |
| selected nav/list row `rgb(240 48 24 / 13%)` + inset `/22%` | | **`selected = 0xF0301821`**, **`selected_border = 0xF0301838`** (new) |
| palette active row `/14%` + inset `/25%` | | `selected` / **`selected_border_strong = 0xF0301840`** |
| active chip `/14%` + inset `/38%` | | **`chip_active = 0xF0301824`, `chip_active_border = 0xF0301861`** (new) |
| needs-you strip (amber 8 % bg, amber 60 % border) | | **`attention_surface = 0xF0A81814`, `attention_border = 0xF0A81899`, `attention_fg = amber`** (new); replaces `waiting_surface`/`waiting_fg` for strips and capsules |
| danger button `rgb(240 60 50 / 16%)` | | `danger_surface = 0xF03C3229` |
| scrim (sheet 50 %, palette 45 %) | | **`scrim = 0x00000073`** |
| scrollbar thumb `rgb(110 101 93 / 45%)`, hover 75 % | | **`scrollbar_thumb = 0x6E655D73`, `scrollbar_thumb_hover = 0x6E655DBF`** (gpui-kit `scrollbar_thumb*`) |
| diff bg `rgb(90 210 90 / 10%)`, `rgb(240 60 50 / 11%)`; fg `#b5e8b5` / `#f3b3ad` | | `diff_added_bg = 0x5AD25A1A`, `diff_removed_bg = 0xF03C321C`, **`diff_added_fg = 0xB5E8B5FF`, `diff_removed_fg = 0xF3B3ADFF`** (new) |
| error text in rows `#f6a39d` | | `danger_surface_fg = 0xF6A39DFF` |

State colours (unipi §1.3; every place a session or agent state shows uses **colour + icon + word**):
needs you = `shell` + pulsing dot + crab "!" eyes · working = `claw` + spinner · done (unseen)
= `success` + check · idle = `fg_subtle` + circle · error = `danger` + x · offline =
`fg_subtlest` + cloud-off. A row's state word is drawn in that colour (13 px).

Mode tints (chips, popovers, list badges): working modes goal `gold`, ralph `success`, swarm
violet `0xC678DDFF`, graph `info`. Permission modes: Ask before changes = `fg_subtle`, Plan =
`gold`, Auto = `claw`, Full access = `danger`. A tinted chip uses the tint at 13 % for the
bg, 45 % for the 1 px border, and full tint for the text and icon.

Oino Light stays derived (A1.2). Each new field gets a light value built the same way
(surfaces `#FAF7F2 / #F2EDE6 / #E9E2D9`, line `#DDD4C9`, text `#2A2420`, same shell primary).
Light is a second priority: it must not block A2 work.

#### A2.3 Typography (CSS px → `TextRole`; `ui_font_size` default 14)

GPUI has no letter-spacing. Section labels therefore use uppercase + weight 500 + `fg_subtlest`
and leave tracking out. Do not fake tracking by inserting spaces.

| unipi use (CSS) | size/line/weight | `TextRole` (offset from `ui`) |
|---|---|---|
| page title `.large-title` | 30/36/600 | `Display` (ui+16 / ui+22) — unchanged |
| dialog / sheet title, empty-state title | 20/26/600 | **new `Title`** (ui+6 / ui+12, 600) |
| markdown h1 | 18/23/650→600 | `Xl` (ui+4 / ui+9) |
| detail header title, markdown h2–h4, compact empty title | 16/21–23/600 | `Lg` (ui+2 / ui+7, 600 in headers) |
| chat prose, user bubble, palette input | 15/24 (bubble 15/22) | **new `Prose`** (ui+1 / ui+10) |
| body, nav item, rows, buttons-sm, palette row | 14/20 (buttons 15/600) | `Base` (ui / ui+6) |
| tool/fold line, settings description | 13.5/20, 13/18 | `Caption` (ui−1 / ui+4) |
| meta, detail sub-line, session strip, section label, tool subject (mono), code in tool | 12/16 | `Sm` (ui−2 / ui+2) |
| turn end, kbd, counters, mono stats | 11–11.5/15–16 | `Xs` → **ui−3 / ui+1** (was ui−4) |

Fonts: Inter for UI and JetBrains Mono for mono stay. Mono sizes are tool subject 12, tool
output 11.5/17, and code blocks 13/20. `code_font_size` default **12** (desktop) applies to
tool bodies and diffs, and code blocks are `code_font_size + 1`. Weights: 400 body, 500
labels/rows/nav, 600 titles and buttons. 650 (brand word "Oino" 17, markdown headings)
renders as `SEMIBOLD`.

#### A2.4 Spacing, sizes, radii, elevation → `tokens.rs`

| unipi | Value | Rust token |
|---|---|---|
| `--page-gutter` | 20 desktop / 16 phone | **`size::PAGE_GUTTER = 20.0`**, `touch::GUTTER = 16.0` (was 12) |
| `--page-head-bar` | 44 | **`size::PAGE_HEAD_BAR = 44.0`** (replaces `HEADER` 44 semantics) |
| page head padding | 10 top / 16 bottom desktop (phone 6/14) | `PAGE_HEAD_PAD = (10.0, 16.0)` |
| `--detail-head-h` | 56; padding 0 12 0 8; back 40; gap 4 | **`size::DETAIL_HEAD = 56.0`**, `ICON_BUTTON_LG = 40.0` |
| sidebar | 224, padding 14 10 12, gap 6 | **`size::SIDEBAR = 224.0`** (fixed; ⌘B hides it) |
| brand row | 40 high, crab 26, word 17/650 | `BRAND_ROW = 40.0`, `CRAB_BRAND = 26.0` |
| sidebar search | 36 high, radius 10, padding 0 8 0 10, margin-bottom 10 | `SEARCH = 36.0`, `R_ROW = 10.0` |
| nav item | 38 high, padding 0 10, gap 11, radius 10, 2 px apart | **`NAV_ITEM = 38.0`** (was 32) |
| list column (`DEFAULT_LIST_WIDTH`) | 400, min 280, max 560, splitter 8 hit / 2 line | **`LIST = 400.0`, `LIST_MIN = 280.0`, `LIST_MAX = 560.0`** |
| list row (grouped card) | min 64 (desktop 56 when narrowed), padding 8 14, icon tile 36 radius 10 | **`ROW = 64.0`, `ROW_COMPACT = 56.0`, `TILE_ROW = 36.0`** |
| `--chip-h` | 38 (segmented options 32) | **`CHIP = 38.0`, `SEGMENT_OPTION = 32.0`** |
| `--btn-h` / `.btn-sm` | 44 / 34; padding 0 20 / 0 14 | **`BUTTON = 44.0`, `BUTTON_SM = 34.0`** (replace CONTROL 28 as default) |
| icon button (ghost) | 40 round in headers, 32 in rows/bars | `ICON_BUTTON_LG = 40.0`, `ICON_BUTTON = 32.0` |
| kbd | min 20 × 20, padding 0 5, radius 5, 11/500 | `KBD = 20.0`, `R_KBD = 5.0` |
| chat column | max 940, padding 28 (prose ≤ 76ch ≈ 680) | **`READING = 940.0`, `READING_PAD = 28.0`, `PROSE_MAX = 680.0`** |
| chat column gap | 10 between rows; 14 top / 18 bottom | `S2_5 = 10.0` (allowed spacing value) |
| user bubble | max min(76ch, 80 %), padding 9 15, radius 20 | `BUBBLE = 0.8` (fraction), **`R_BUBBLE = 20.0`** |
| composer shell | radius 26, padding 8 8 8 0, row padding-left 10; bottom area padding 0 24 14, max 884 | **`R_COMPOSER = 26.0`, `COMPOSER_MAX = 884.0`** |
| send / attach | 38 circle / 34 circle; send chevron 22×38 | **`SEND = 38.0`, `ATTACH = 34.0`** |
| mode chip | 30 high, padding 0 10 0 8, gap 5 | `MODE_CHIP = 30.0` |
| context ring | 18, stroke 2.5 | `RING = 18.0`, `RING_STROKE = 2.5` |
| fold / tool row | min 32, padding 3 4, gap 7, radius 10, icon 18 | **`TOOL_ROW = 32.0`** (was 24) |
| fold body | margin 2 0 6 12, padding-left 10, 1 px left rule, max 420 | `FOLD_INDENT = 12.0`, `DETAIL_MAX = 420.0` |
| tool body | radius 12, output max 240, padding 8 12 | `R_TOOL = 12.0`, `TOOL_OUT_MAX = 240.0` |
| harness card | radius 14, padding 8 12, body max 320 | `R_HARNESS = 14.0` |
| interaction card | radius 24, padding 12 14, max min(64 %, 560); option row min 46 radius 14; number tile 22 radius 7 | **`R_DIALOG_CARD = 24.0`, `OPTION_ROW = 46.0`, `R_OPTION = 14.0`** |
| queue strip | radius 16 16 0 0, max 152, rows 30 icon buttons | `QUEUE_MAX = 152.0` |
| jump to latest | 38 circle, surface-3 | `JUMP = 38.0` |
| dialog (desktop sheet) | 560 × max min(80 %, 760), radius 16, padding-bottom 20, title 20/26 padded 0 20 10 | **`DIALOG_MD = 560.0`, `DIALOG_MAX_H = 760.0`**, `DIALOG_LG = 760.0` (shortcuts, info) |
| palette | 620 wide, top 14 %, max-h 64 %, input 52 (16 px), rows 40 radius 10, list padding 6, footer 12 px | **`PALETTE = 620.0`, `PALETTE_TOP = 0.14`, `PALETTE_INPUT = 52.0`, `PALETTE_ROW = 40.0`** |
| menu (anchored) | radius 16, rows 40 radius 10 | `MENU_ROW = 40.0` |
| settings | nav 200 wide (rows 38), content max 760, group radius 16, row min 56, icon tile 28 radius 8 | **`SETTINGS_NAV = 200.0`, `SETTINGS_CONTENT = 760.0`, `SETTING_ROW = 56.0`** |
| empty state | top 64; crab 88 / icon tile 56 radius 16; gap 10; body max 340; compact crab 56 / tile 44 radius 14 / button 38 | `EMPTY_TOP = 64.0`, `CRAB = (16, 56, 88, 128)`, `EMPTY_TILE = 56.0` |
| pane placeholder | tile 52 radius 16, title 16/600 muted, hint 13/19 dim, max 320 | `PLACEHOLDER_TILE = 52.0` |
| board | lane 280 gap 12; card padding 12 radius 16; lane pill 38; FAB 52 radius 16 | `LANE = 280.0`, `FAB = 52.0` |
| header crab | 28 + badge 16 (amber, `Xs` 600) | `CRAB_HEADER = 28.0` |
| mascot window | crab 72 / 96 / 128 (default 96), bubble max 260 radius 16 | `MASCOT = (72, 96, 128)` |
| phone floating crab | 56, half-transparent after 3 s | `touch::CRAB_FLOAT = 56.0` |
| phone app bar / bottom nav | header 44 lead + title row; bottom nav 8 12 padding, 12 px labels | `touch::APP_BAR = 52.0` stays, `touch::BOTTOM_NAV = 60.0` |

**Radii** (`radius` module): `R_ROW = 10` (nav, palette rows, tool rows, search, small
tiles), `R_INPUT = 12`, `R_OPTION/R_HARNESS = 14`, `R_CARD = R_FLOAT = 16` (cards, grouped
lists, dialogs, menus, palette, toasts, settings groups, tiles 52–56), `R_BUBBLE = 20`,
`R_DIALOG_CARD = 24`, `R_SHEET = 24` (phone, top only), `R_COMPOSER = 26`, `R_FULL` (buttons,
chips, capsules, send, avatars, badges, dots). **Delete the ZCode nesting rule and
`R_SM/MD/LG/XL` from view code.** They remain only for `R_KBD = 5`, inline code 6, the number
tile 7 and the settings icon tile 8.

**Spacing**: 4 px grid plus unipi's 6/10/14 half steps (`S1_5 = 6`, `S2_5 = 10`,
`S3_5 = 14`). The no-literal guard test (§9.3) stays.

**Elevation** (`E*` tuples, dark): composer `0 -8 30 #00000066` + inset line; dialog
`0 24 80 #00000099` + inset line; palette `0 30 90 #000000A6` + inset line; menu/popover
`0 12 32 #00000080`; jump/FAB `0 6 20 #00000080`; toast and mascot bubble `0 8 26 #00000080`.
Cards and rows use no shadow. They are separated by `surface-1` on `bg` plus an inset 1 px
`line`.

#### A2.5 Motion, scrollbars, focus, hover

- **Motion** (unipi §1.4: 150–200 ms ease-out; the Unicrab is the only looping animation):
  - Colour and hover transitions run at `FAST = 120 ms` for nav and rows and `NORMAL = 180 ms`
    for buttons.
  - Dialogs fade in with a scale from 0.97 to 1 over **160 ms**.
  - The palette enters over **140 ms**, scaling from 0.98 with `cubic-bezier(.2,.8,.2,1)`.
  - Detail views slide in 16 px with a fade over 180 ms (`pane-in`).
  - Fold chevrons rotate 90° over 160 ms.
  - The dot pulse runs at 1.2 s.
  - The live fold line shimmers in a 1.6 s sweep.
  - Phone sheets use a spring with drag.
  - Reduce motion holds every animation on frame 0, and the crab neither blinks nor hops.
- **Scrollbars**:
  - 10 px track, transparent. The thumb is a pill inset 2 px, so it shows 6 px, in
    `scrollbar_thumb` (hover `_hover`).
  - The scrollbar is always visible while content scrolls, with no overlay fade.
  - Terminal, lane strip and chip rows hide their bar.
  - Horizontal chip rows fade at the right edge (28 px gradient mask, drawn as an overlay
    quad).
- **Focus**: as in L11. The focus ring is 2 px `focus`, outset 2, and follows the
  element's radius.
- **Hover** (desktop only): any row or button gets `hover` (surface-2). Secondary buttons
  and chips go to surface-3. Primary goes to `shell_hi`. Ghost and danger buttons go to
  surface-2 with `fg`. Board cards go to surface-2 plus an inset 1 px border at 55 % dim.
  There is **no press-scale on desktop**.

#### A2.6 Page header pattern (`components/page_header.rs`, new)

Every top-level screen (Sessions, Board, Extensions, Settings) uses one `PageHeader`:

```text
x = sidebar + 20 ──────────────────────────────────────────────────────────────┐
│ [lead: back / project chip]                       [tools: ghost 40 …] [crab] │ 44 px (always on desktop)
│ Title (Display 30/36/600, ellipsis)  [title slot: count badge / picker ▾]    │ min 36, margin-top 4
│ meta line 13/18 fg_subtle  (e.g. "4 sessions · 1 working")                   │ min 18
└───────────────────────────────────────────────── padding 10 20 16 ─────────┘
```

The title always sits at the same x and y on every screen. **Tools end with the header crab**
(A2.13). There is no PC chip, because Oino runs on one machine. Board uses the lead slot for
the project avatar tile and makes the title the project picker (▾).

#### A2.7 Detail header (`components/header.rs` → `DetailHeader`)

```text
┌ 56 px, padding 0 12 0 8, 1px bottom line_soft ──────────────────────────────────────────────┐
│ [‹ 40] Session title (Lg 16/600)        [session strip pill: Single · model · ctx 1% · $0.00] [mode chip] [⋮ 40] [🌐 40] [▭ 40] │
│        ● project · branch  (Sm 12, fg_subtlest; dot = state colour)                          │
└──────────────────────────────────────────────────────────────────────────────────────────────┘
```

- **Back** closes the detail (Esc does the same). With the list column visible it stays as
  the keyboard-free close.
- **Title**: click to rename. A click on the sub-line or the title opens the **switch sheet**
  (A2.8).
- **Session strip**: the footer `setFooter` summary, compressed. It is a 32 px ghost pill
  (12 px `fg_subtlest`) with a max width of 46 %. Clicking it opens the control centre
  sheet: model and thinking, context, cost, and the full stats strip. This replaces the
  meta-row stats overflow (GA-26).
- **Working-mode chip**: shown only while a goal, ralph, swarm or graph is active or parked.
  It shows `Goal 45 %` / `Ralph 5/12` / `Graph 3/7` with a tint and a pulse while running.
  Click opens an anchored popover with what the mode does, live state with a bar, Show
  progress, and Switch mode.
- **⋮ menu**: Rename, Fork, Tree, Compact now, Export HTML, Copy session id, Reveal file,
  Delete…
- **Right icons**: 🌐 browser side pane (Ctrl+Shift+B), ▭ terminal/bottom, and review
  (`git-compare`) when the turn changed files.
- The same header is used, with its own actions, for the task page, review, file viewer and
  browser.

#### A2.8 Sidebar, list column and switch sheet

- **Sidebar** (224 px, `sidebar` fill, right rule `line_soft`):
  - **Brand row**: Unicrab 26 (static glyph in the aggregate state colour) + "Oino" 17/650.
    An update badge pill (cyan 12 % tint, "Update") sits at the right when the updater has
    a release.
  - **Search field**: ⌕ "Search" `Kbd Ctrl K`. It opens the palette.
  - **Nav**, each with an icon (16) and a label:
    - Sessions (`message-square`, Ctrl+1)
    - Board (`kanban`, Ctrl+2; hidden without kanboard)
    - Extensions (`blocks`, Ctrl+3)
    - Settings (`settings`, Ctrl+,)
  - The shortcut hint `Kbd` on each nav item fades in only on hover or when the sidebar has
    focus.
  - Bottom: nothing but optional build info. The model label and crab move to the
    header/composer.
- **List column** (Sessions screen, 400 px, resizable 280–560, double-click resets, width
  remembered):
  - Page header "Sessions", meta "N sessions · 2 need you".
  - Tools: refresh, `+` new session (Ctrl+N), then the crab.
  - The **needs-you strip** (A2.9) comes next.
  - Then one **section per project** (cwd): a section label with the project basename, a
    right aside with the count, and `+` to start a new session in this project.
  - Then a grouped card (`card`, radius 16, inset line) of **session rows**:
    - 64 px: a 36 icon tile with the Oino mark and a 6 px state dot at its bottom-right.
    - Title 15/500.
    - Sub-line: the state word in the state colour, then a relative time or cwd in
      `fg_subtlest`.
    - A trailing chevron.
    - Rows have inset dividers.
    - The open session row gets `selected` + a 3 px `shell` left bar.
    - Right-click opens an anchored menu: Open, Rename, Pin, Fork, Copy id, Reveal file,
      Delete….
  - Section order is attention-first: needs-you > working > done-unseen > recent.
- **Folding**:
  - The list column folds away when list + chat + side pane do not fit (< 1180 px with the
    side pane, < 900 px without).
  - Ctrl+B also folds the list column (then the sidebar).
  - With the list folded, the detail header's title opens the switch sheet.
- **Switch sheet** (unipi §3.6, Ctrl+O, or a click on the detail header title):
  - A 560 dialog with a filter field and the same rows, attention-first, grouped by project.
  - Each row shows its state chip and, as optional sub-text, model · ctx.
  - Enter opens; Esc closes.
- **Not used**: workspace/session trees and nested tabs or panes.
- **No-session placeholder** (detail empty): the compact pane placeholder: `message-square`
  tile 52, "No session open", hint "Pick a session on the left. It keeps running while you
  look at the others."

#### A2.9 Sessions home and the needs-you strip

- **Needs-you strip**:
  - Full list width, min 56, radius 16.
  - `attention_surface` fill, 1 px `attention_border`, padding 0 16.
  - Contents: the Unicrab 28 in the needs-you pose, then "2 sessions need you"
    (15/500 `amber`). With one item it shows the title instead.
  - Click opens the first waiting session. The ▾ opens the needs-you drawer, an anchored
    popover listing each waiting item (title, what it waits for: "Approve: bash cargo test",
    "3 questions", "Plan review") with inline Open.
  - The strip is hidden when nothing waits.
- **Done-unseen**: rows carry a `success` dot and the word "Done" until they are opened.
- **Empty list**:
  - The EmptyState shows the crab 88, "No sessions yet", "Start one and Oino gets to work in
    this folder." and the primary button "New session".
  - No lore here, because there is something to do.

#### A2.10 Chat view (detail)

Column: max 940, padding 28; prose ≤ 680; rows gap 10.

| Row | Look |
|---|---|
| **User bubble** | Right-aligned. `bubble` (surface-3), radius 20, padding 9 15, `Prose` 15/22, max 80 %, selectable. Starting with a `/command` makes the command a chip at the start. Hover action bar (top-right, −14 px): a pill `surface-2` with 28 px round buttons (Copy, Edit & resend, Fork, Rewind files) |
| **Assistant prose** | No bubble. 15/24 `fg`; paragraphs 10 apart; lists indent 22 with `list_marker` (claw) bullets; h1 18, h2–h4 16, 600; blockquote 3 px `line` rail + muted; links `link`; strong `fg_strong`. Inline code: `tag` bg, `cream` text, mono 12, radius 6. Code block: surface-1, radius 12, inset line; head row surface-2 with language `Sm` dim at left and "Copy" at right |
| **Fold line** (thinking + tools of one turn) | One line, min 32. Icon 18 `fg_subtlest`, title 13.5/500 `fg_subtle` ("Ran commands", "Read files, edited files"), `·` detail `fg_subtlest` ("2 steps", "1m 3s"), chevron at the far right. Live: the latest action ("Reading files · src/x.ts") with the 1.6 s shimmer. Open: body indent 12 + 10 with a 1 px `line` left rule, max 420, holding one-line tool rows |
| **Tool row** | Min 32, radius 10. Icon 18 · name 13.5/500 `fg` · 3 px dot separator · subject mono 12 `fg_subtlest` (error first line in `danger_surface_fg`, sans 13) · stat mono 11.5 (`+7` green, `−3` red, `3.4s`). Opens a tool body: surface-1, radius 12, inset line. Bash body: `$` in claw + command mono 12, rule, output 11.5/17 muted, max 240. Diff: `diff_*_bg/_fg`. IN/OUT: a 34 px label column (10.5 uppercase dim) |
| **Thinking** | Inside the fold: one line with the latest finished paragraph (italic off, `fg_subtle`); click opens the text |
| **Harness card** (memory, bg-task done, kanboard, notify, dream, subagent/sidekick completion, plan-mode reminder) | Collapsed by default to one line. Radius 14, transparent, inset 1 px `line` at 75 %, padding 8 12. Head: icon (claw) + source 600 `fg` + title `fg_subtle` 13 + chevron. Open body 13 muted, max 320, OUS or markdown. Replaces ProvenanceCard and fixes GA-6 |
| **Turn end** | `Done in 1s · $0.001` 11.5 `fg_subtlest` tabular, padding 0 4. "Stopped" shows as an amber 11 px chip on surface-2 |
| **Changes this turn** | Card surface-1, radius 14, inset line. Head "Changes this turn · 1 file" 12/600 + total `+7 −3`. File rows mono 12 with stats, 👁 review and ↗ open. Clipped to about 2.5 rows with a fade; "Show all" (gold pill) opens the review pane |
| **Error** | Inline row: `danger` x + message in `danger_surface_fg` + Retry pill. Not a banner card |
| **Agent row** | Fold-line style, with `bot` (or `users`) in `claw`, the agent name 500, the task and elapsed time. Click opens the Agent tab in the side pane |
| **Jump to latest** | 38 circle surface-3, bottom-right 16 px above the composer, shadow |
| **Find** | ZCode FindBar restyled: 40 high, `sheet` bg, radius 16 |

The query rail (§4.10) stays, with bars in `fg_subtlest` and the current one in `shell`. The
**start header** (info-screen splash, `setHeader`) shows only in an empty session (A2.17).
In a non-empty session it is one collapsed harness card ("Unicrab · Oino 1.0 · 23
extensions") at the top. It is never pinned (GA-5).

#### A2.11 Composer and toolbar capsule

```text
 [queue strip: radius 16 16 0 0, surface-1, rows 13 px with kind tag (Steer amber / Queue gold) + ↑ ✎ ✕ 30px]
┌ composer shell: input (surface-2), R 26, padding 8 8 8 0, inset 1px line 80 %, shadow composer ───────────┐
│ [glance top segments: UNIPI wordmark · Regular Mode · ⎇ branch …            auto]  (Sm, fg_subtlest)         │ ← setFrame top, only when present
│  Message Oino, / commands, @ files                                (Prose 15, 1–12 lines)                 │
│  [⊕ 34] [mode chip 30 ▾] [◔ 1 %  DeepSeek Flash  low ▾ (chip 30)]  …            [↑ send 38][⌄ 22]          │ ← toolbar capsule
└──────────────────────────────────────────────────────────────────────────────────────────────────────────┘
  glance bottom: setFrame bottom segments left · stats strip (setFooter) centred, toned, 11.5 tabular         ← ≤ 1 line, drops by priority
```

- **Shell**: the bottom area has padding 0 24 14, max width 884, centred. The editor has no
  inner border.
- **Placeholder**: "Message Oino, / commands, @ files" (`fg_subtlest`). While a run is in
  progress it reads "Steer Oino…".
- **Attach**: a 34 circle, surface-3, `paperclip`. Its menu is Files…, Image from
  clipboard.
- **Mode chip**: the workflow permission mode, tinted per A2.2. Its menu lists the modes.
  It reads the `unipi:workflow:status` topic, never footer segments (GA-25).
- **Model chip**:
  - A 30 pill on surface-1 holding the context ring (18, stroke 2.5, `success` → `warning`
    ≥ 80 % → `danger` ≥ 95 %), the context %, the model name (12/500) and the thinking level
    (`fg_subtlest`).
  - A click opens the **control centre** dialog/sheet. It holds the model list,
    thinking-level segments, the fusion lead/sidekick, context and compaction.
  - Ctrl+M opens it too.
- **Send / stop**:
  - **Send** is a 38 circle in `shell` with an `arrow-up` in `face`. It is disabled at 35 %
    mix with surface-2.
  - While a run is in progress and the editor is empty it becomes **stop** (`square`).
  - The ⌄ chevron (22 × 38) opens the Steer / Queue choice, which is also Ctrl+Enter.
  - A pick shows as the send-choice bar: a pill, surface-1, with an inset claw border at
    40 % and S / Q pills.
- **Toolbar capsule geometry** (shared with phone; unipi Moshi proportions): items 30–38
  high, 6–8 gap, 8 inner padding, all pill-shaped. When the shell is under 520 px the model
  name drops first, then the thinking label, then the mode label (icon only).
- **Glance frame**: the footer `setFrame` top/bottom segments and the `setFooter` strip
  render **inside or right under the shell**, as in the TUI shot (`tui-glance-footer.png`).
  - The "UNIPI" brand uses the wordmark letters (white, cyan, green, yellow, red).
  - Segments are separated by 1 px `line` dividers.
  - Numbers keep their tones (cyan counts, amber times, green cache).
  - It is never wider than the shell and drops by `priority`, never wrapping (GA-26).
- **Completion menu**:
  - Shape: a `sheet` popover, radius 16, 360 wide, anchored above the caret, rows 40,
    radius 10, at most 8 visible.
  - Sections use **section labels**.
  - Package tags are colour pills (`categorical` 13 % bg + 45 % border, `Xs`).
  - **One group per source**: enchanted groups replace the base group (GA-4).
- **Interaction card in the composer slot**:
  - It replaces the shell until it resolves, and the draft is kept.
  - Shape: radius 24, surface-2, inset 1 px `claw` at 45 %, shadow composer, padding 12 14,
    max min(64 % of frame, 560), then it scrolls.
  - **Head**: an eyebrow icon and label in `claw` (12/600, e.g. "Approve", "Plan", "2
    questions"), then "1 of 3" `fg_subtlest`, the countdown pill (surface-3, 12/600
    tabular; only when the extension set one), and ✕ (34 round).
  - **Body**:
    - The question in 16/22/550 (`MEDIUM`).
    - The message in 14/20 muted.
    - Options: rows min 46, radius 14, surface-3, a number tile (22, radius 7, surface-1),
      the label 15/500 and a description 13 dim.
    - The selected option gets an inset 1.5 `claw` ring + claw 10 % fill.
    - "Type your own answer" row: a ✎ tile with a field (radius 14, surface-1, inset 1.5
      claw ring when focused).
  - **Footer**: ‹ › page buttons (36 round), then right-aligned buttons that are 40-high
    pills 15/600. The primary is `shell`/`face`, the ghost is surface-3. Approvals map
    allow-once → primary, always → secondary, reject → danger pill.
  - The danger strip and keys (1–9, Enter, Esc, Tab) are as in §4.12–§4.15.
  - The dialog never auto-times out on the frontend (GA-30).

#### A2.12 Cards, sheets, dialogs, menus, toasts

- **Card** (`components/card.rs`, new): `card`, radius 16, inset 1 px `line`, padding 12–16.
  A **grouped list** is a card whose rows have inset dividers.
- **Dialog** (desktop rendering of every unipi sheet):
  - Centred, 560 wide (760 for large: shortcuts, info dashboard, hints), max
    min(80 %, 760).
  - `sheet` (surface-1) fill, radius 16, dialog elevation, scrim 45 %.
  - Header: title 20/26/600 with padding 14 20 10 and ✕ 34 at the right. Body padding 0 20.
  - **Sticky footer** with right-aligned actions (`dialog-actions`: gap 8, min width 96
    each), primary last on the right. This follows the unipi order and overrides §4.24
    "primary first".
  - Dialogs never exceed the window, and the body scrolls (GA-7).
  - Entry is a fade with a scale from 0.97 over 160 ms.
- **Phone sheet**: bottom, `R_SHEET` 24 on the top corners, a 36 × 4 handle, swipe down to
  dismiss, capped to the visual viewport. **Pickers drill in instead of stacking sheets.**
- **Anchored menu**: `menu` (surface-2), radius 16, padding 6, rows 40 radius 10 (icon 16 +
  label 14 + trailing kbd or check), hover surface-3. It opens at the trigger or the
  pointer (right-click), and never shifts the layout.
- **Overlay panels** (info dashboard, hints, MCP, readme/changelog, agents, model picker):
  - They use the dialog look, header = title + ✕ only.
  - Panel `keys` are **not** buttons. They show as one footer legend line (`Kbd` + label
    12 dim, as in the palette footer) and are bound while the dialog has focus (GA-22,
    GA-23).
  - Panel tabs are segmented pills (`CHIP` 38, options 32). The active option is surface-3.
- **Toast**:
  - Bottom-centre of the detail on desktop (phone: above the composer/FAB, never over it).
  - Max width 420, `sheet` fill, radius 16, toast elevation, padding 10 14.
  - Icon in tone, message 14 (up to 3 lines; "More" expands), at most one action pill
    (Undo / Open).
  - Multi-line `notify` (status reports) opens as a **result card dialog** instead of
    clipping (GA-24).

#### A2.13 Mascot, header crab and Unicrab

- **States and priority** follow unipi §1.5.1 exactly: needs-you > done > working > offline
  > sleep (5 min idle) > idle.
- **Rig**:
  - Port unipi's sprite + animation tables as data: `unicrab.sprite.json` (24 × 20 cells,
    11 colours) and `unicrab.anim.json` (frameMs, whole-cell `[dx,dy]` per track `all`,
    `torso`, claws, legs, `zzz`, eye masks, blink, opacity).
  - Store them as Rust consts in `design/brand.rs`, generated once by a script from the
    JSON with the source path noted. A1.2 already allows unipi's palette and brand.
  - Renderers move parts and never redraw them. Use whole device pixels per cell. Random
    blink. Animation runs only at ≥ 28 px, and reduce motion holds frame 0.
- **Header crab** (28) sits at the end of every page header's tools and in the detail
  header on narrow windows.
  - Badge: amber, `Xs` 600, with the needs-you count.
  - Click opens the needs-you drawer, an anchored popover with rows for waiting and
    done-unseen items, Open per row, and "Mark all read".
  - **The window title becomes "● Oino" while something needs you.**
- **Mascot window** (opt-in, Settings → Unicrab → Mascot window; tray item):
  - The window is transparent, frameless, shadowless and always-on-top, bottom-right of the
    primary monitor. It shows only the crab (72/96/128, default 96) plus the fade bubble.
  - Bubble: surface-2, radius 16 with the corner nearest the crab at 4, head 11/700 `claw`
    (session title), text 14/1.3 clamped to 3 lines, 6 s fade-in, hold and fade-out, one at
    a time, unread only.
  - Clicking the crab brings Oino to the front and opens the drawer. Clicking a bubble opens
    that session. Dragging moves the window.
  - It reads its state from a snapshot passed in. It never leases the shell entity (GA-32).
- **Phone floating crab**:
  - 56 px, draggable, snaps to the nearest side, half-transparent after 3 s unless
    something needs you. It has the same bubble.
  - Tap opens the needs-you drawer. Long-press opens quick actions (Needs you, Sessions,
    Mark all read, Hide the crab).
  - Settings → Unicrab → Floating crab turns it off.
- **Unicrab settings section**: mascot window on/off, crab size (Small/Medium/Large
  segmented), floating crab (phone), and bubble detail (Summary / Minimal).

#### A2.14 Empty states and lore

- **`EmptyState`** (`components/empty.rs`):
  - Shape: top offset 64, gap 10, centred. Either the crab 88 (pixel, state pose) or an
    icon tile 56 (radius 16, surface-2, `fg_subtle` icon 24).
  - Text: title 20/26/600, body 14/21 muted with a max width of 340, actions (gap 8, pills
    44).
  - `compact` variant (list columns, detail placeholders, panels): crab 56 or tile 44
    (radius 14), title 16/22, body 13/19, buttons 38.
  - `tone=error`: the crab is greyscaled (85 %) and dimmed (75 %).
- **Lore lines** appear only where there is genuinely nothing to do, and never the same line
  under every state. Rotate through `design/lore.rs`, a copy of unipi `lore.ts` (6 lines, e.g.
  "Unicrab walks sideways so it can read your diffs from both ends."). Oino may add lines.
  - **Yes**: needs-you drawer empty ("Unicrab is watching the tide. Nothing needs you."),
    board lane Done empty, Settings → About, mascot bubble when idle-clicked, splash.
  - **No**: Sessions empty, Board with no projects, errors, search no-results (these say
    what to do next).
- **Fixed copy**:

  | Where | Title | Body | Actions |
  |---|---|---|---|
  | No session open | "No session open" | "Pick a session on the left. It keeps running while you look at the others." | — |
  | New empty session | "New session" (sparkles icon in `claw`) | "Say what you need, or pick up where you left off." | — |
  | Board without kanboard | "Board is off" | "Turn on the kanboard extension to plan work here." | "Open Extensions" |
  | Lane filtered empty | — | "No tasks here match the filter" | "Clear filters" |
  | Tasks tab | "No background tasks" | — | — |

#### A2.15 Section labels, buttons, chips

- **Section label** (`components/section_label.rs`, new): `Sm` 12/16, 500, uppercase,
  `fg_subtlest`. Margins are 18 0 10, or 4 0 10 when first. `section_row` puts an aside on
  the right (count / link, 12 dim tabular). Section labels are used by the palette groups,
  the completion menu, settings sections (with a 14 icon), lanes ("Todo, 3 tasks"), session
  project groups, shortcut groups and status sections.
- **Buttons** (`components/button.rs`): all pill-shaped.
  - Primary `shell`/`face` 600; hover `shell_hi`. **One per view.** Never `danger` red for a
    primary.
  - Secondary surface-3/`fg` 500.
  - Ghost: transparent, `fg_subtle`; hover surface-2 + `fg`.
  - Danger: `danger_surface` + `danger`. Destructive confirms use `deep` with an icon.
  - Sizes: `BUTTON` 44 (dialog footers, empty states, page CTAs, padding 0 20, 15/600),
    `BUTTON_SM` 34 (toolbars, inline, padding 0 14, 13.5/500), icon buttons 40 / 32 / 28
    round.
  - Loading keeps the width and swaps the icon for a spinner.
- **Chips** (`components/chip.rs`, new):
  - **Tool chip / pill** (toolbars): 38 high, padding 0 14, surface-2, 14/500, gap 6, with
    an invisible hit-slop of 3 px. Active state: `chip_active` + inset `chip_active_border`.
  - **Segmented**: a 38 track on surface-2. Options are 32 pills and the active one is
    surface-3.
  - **Count badge**: 16–18 high, amber or surface-3, `Xs` 600 tabular.
  - **Status capsule** (state colour 13 % + word): 22 high.
  - **Package/tag pill**: tone 13 % + 45 % border.
  - **Kbd**: 20, radius 5, surface-3, inset line + bottom shadow, 11/500.

#### A2.16 Settings layout

- `PageHeader` "Settings" with a search tool (Ctrl+F). Below it are two columns:
  - **Section nav**: 200 px, rows 38 radius 10, 14/500 `fg_subtle`, active `selected` with
    `fg`.
  - **Content**: max 760, with sections stacked.
- **Each section**: a section label with an icon (e.g. 🔔 DESKTOP) and then a grouped card.
- **Rows**:
  - Min 56, padding 12 16, an icon tile 28 (radius 8, surface-2, icon 16 muted), the label
    15/500, the description 12.5 dim, the control at the right.
  - Rows have inset dividers starting at 58 px.
- **Controls**:
  - **Switch**: 44 × 26, on = `shell` with a cream knob, off = surface-3 with a muted knob.
  - Select = anchored menu trigger (chip look).
  - Text fields: 40 high, radius 12, surface-2, inset 1.5 shell ring on focus.
  - "Add …" rows: an icon tile with a shell 12 % tint and the label in `fg`.
  - Danger zone: a ghost danger pill.
- **Nav order**:
  - General (default model, default folder, startup)
  - Appearance (theme, sizes, reduce motion, status panel, tool rows)
  - Unicrab
  - Models & providers
  - Sessions
  - Permissions
  - Keyboard shortcuts (app + extension, GA-16)
  - Extensions settings (a hub section label per namespace)
  - About & updates (version, Check for updates, changelog, licences, crab lore)
- **Remote access is removed from the nav until A2.18 lands.**
- **Scope**: the hub keeps the Global | Project scope segmented chip in the section head.

#### A2.17 Board, welcome, palette, extensions, onboarding

- **Board** (`m7-desktop-board.png`):
  - **Header**: `PageHeader` with lead = project avatar tile (28, radius 8, first letter,
    shell 14 % tint), title = project name + ▾ (picker), meta "5 tasks · ● 1 running"
    (`shell` dot). Tools: search, ⋮ (Refresh, Review queue, Blocked, Summarize board, Board
    settings, Archive project), crab.
  - **Toolbar pills**: Board | List segmented (icons), Filter, Display.
  - **Lane strip**: pills 38 with status glyph + name + count badge. The active pill takes
    the lane colour at 13 %. The strip fades at the right edge.
  - **Lanes**: 280 wide, gap 12. Head: section label "Todo, 3 tasks", about text 12/16 dim
    (2 lines), ⋮ lane menu.
  - **Cards**: `card` radius 16, padding 12; id mono 11 dim; title 15/600 (2 lines); excerpt
    13 dim; agent chip (a `shell` pill at 14 % "● agent 1h 51m"); priority and dependency
    tags; blocked reason in `danger_surface_fg`. Hover is surface-2 + border.
  - **Empty lane**: a dashed 1 px `line` card, radius 16, with the lane glyph + "Nothing
    blocked".
  - **Add task**: a dashed pill row "+ Add task".
  - **FAB**: 52, radius 16, `shell`, `plus` in `face`, bottom-right 24/24, opens New task
    (dialog).
  - **Detail**: the task page opens in the detail column with a `DetailHeader`.
  - **Keys**: the kanboard keys (C, /, J/K, Enter, B/L, R/X, ?).
  - It runs `/unipi:kanboard open`, not the bare `/kanboard` (GA-17).
- **Welcome** (empty session):
  - Centred in the chat column. The info-screen splash (setHeader) is drawn as an
    **EmptyState with the crab 88 + one-line build info** ("Oino 1.0 · 23 extensions ·
    DeepSeek Flash"). The drain meter is a 2 px bar under it.
  - The greeting title 20/26 is local-time aware (GA-12 fixes the UTC hour).
  - The composer comes next, then the hint line (12 dim, `lightbulb` + ‹ ›), then up to 3
    suggestion cards (card radius 16, padding 14, icon + 15/500 title + 2-line 13 dim body).
  - The ZCode Display greeting and the 128 px pixel art are **dropped**: the crab is drawn
    once, never as two crabs (GA-5).
- **Palette** (`live-desktop-palette.png`):
  - 620 wide, top 14 %, `sheet` fill, radius 16, palette elevation.
  - Search row: 52 high, ⌕ + 16 px input + `Esc` kbd, bottom rule.
  - Groups use section labels: Go to, Session, Board, Extensions (commands + **extension
    shortcuts**, GA-16), App.
  - Rows: 40, radius 10, icon 18 muted + label 14 + right kbd chips. The active row is
    `selected` + `selected_border_strong`.
  - Footer legend: `↑ ↓ move · ↵ run · Esc close`.
  - Prefixes `>` `#` `@` stay.
- **Extensions page**:
  - `PageHeader` "Extensions", meta "32 loaded · 1 failed", tools "Install from file…".
  - A grouped card per kind (Providers / unipi / User). Rows 64: icon tile, name 15/500 +
    id mono 12 dim, the version, a state capsule (Loaded success / Failed danger / Off),
    a switch, and a ⋮ menu (Settings, Readme, Reveal folder).
- **Onboarding**: a full-window centred card column (max 560) with the crab 88, "Welcome to
  Oino" 30/36, and the steps as a grouped card with numbered rows. Buttons are pills.

#### A2.18 What to remove (herdr / tmux / multiplexer concepts)

| Remove | Where today | Instead |
|---|---|---|
| Split panes (≤ 4), `layout: Vec<u64>`, ⌘\ "Split pane", "Open in new pane", ⌘Enter "open in new pane", "Run in new pane" palette rows | `views/shell.rs` (`split`, `layout`, `Pane`), `components/sidebar.rs:605,1040`, `components/keys.rs`, `model/palette.rs` | One visible session. Background sessions keep running (in-process agents) and are switched, never tiled |
| `Workspace` (root view type name), `Pane`, `MAX_LIVE_PANES` and "pane" wording | `views/shell.rs:75-140`, `views/{mod,board,mascot,soak,extensions_page,onboarding,session_pane}.rs`, `mobile/sheets.rs`, `model/sessions.rs` | Rename to `AppShell`, `LiveSession`, `MAX_WARM_SESSIONS` (same release rule: hidden idle sessions beyond 8 shut down and reopen from the list) |
| `SidebarView { Group, Recent }` view switcher, workspace groups, "New session in folder" wording | `views/shell.rs:57`, `components/sidebar.rs` | Project **sections** in the list column (A2.8), each with a `+` for "New session in <project>" |
| ZCode independent frames, frame gaps, `WINDOW_INSET`, `FRAME_GAP` | `views/shell.rs`, `design/tokens.rs` | Flush columns (L1) |
| Perf budget "4 sessions streaming in split panes" | §2.11, `views/soak.rs`, `scripts/perf.sh` | "1 visible + 3 background sessions streaming" at the same budget |
| Remote access settings page, palette "Remote access", desktop serve/QR pairing | `views/settings_page.rs` (`SettingsSection::Remote`), palette | **Later** (post-v1): "Pair a phone" / "Add a PC" (unipi wording) when `oino serve` lands; hidden until then |
| Mobile "Pair a desktop" wording | `mobile/hosts.rs` | Keep M10 behind a "later" flag; wording becomes "PCs" / "Pair a PC" / "No PC paired yet · Add a PC" |
| `skill:herdr` in the slash menu (from `~/.agents/skills`) | `crates/oino-resources/src/discovery.rs:48` (GA-38) | Load user skills only from Oino's configured skill dirs; the scratch agent dir must not pick up `~/.agents/skills` |
| `tmux capture-pane` allow rule, `rename.herdrSync` default | `extensions/unipi/fusion/src/policy.rs:193`, `extensions/unipi/utility/src/settings.rs:226` | Delete (dead in a native app) — request to the extensions owner, not a GUI batch |
| §8 rows 7 (frames) and 48 (split panes) | this doc | Row 7 → **Adapt** (unipi flush panes); row 48 → **Diverge** (owner: no multiplexer) |

Never call a session a "pane", "tab", "window" or "workspace". The bottom frame keeps
**terminal** tabs (PTYs are terminals, not sessions).

#### A2.19 Mobile, where it differs

- **Header**: the phone page header has no lead row. Tools sit on the title row, and 52 px
  is kept free at the top-right for the floating crab. Gutter 16.
- **Navigation**: a bottom nav (surface-1, top `line`, labels 12, active `shell`) with
  Sessions · Board · Settings. Search is a header tool.
- **Chat header**: 52 px (back, title 16/600 + sub-line, ⋮, crab space). The session strip is
  a centred band below the header (11.5 dim, bottom rule at 40 %), not a pill.
- **Rows**: 64. Swipe left on a row for Pin / Delete. Long-press for the anchored menu.
- **Dialogs**: every dialog is a bottom sheet (radius 24 top, handle). Interaction cards
  take the composer slot as on desktop. Option rows are 46, and buttons are 40–44 pills.
- **Floating crab**: 56 (A2.13) replaces the app-bar glyph.
- **Toolbar capsule**: the composer toolbar uses the capsule proportions (keys 34, gaps 6,
  inner padding 8). There is no terminal key capsule (Oino mobile has no terminal).
- **Toasts**: they sit above the composer or FAB.

#### A2.20 Per-file work plan (fix batches)

Rules for all batches:
- **Branch and gates**: commit on `v1-alpha` with the batch id prefix. Apps gates are
  `cargo fmt`, `clippy -D warnings -W clippy::pedantic`, `cargo test` in `apps/`, `-j 2`, and
  `TMPDIR=$PWD/target/tmp`.
- **File ownership**: each file belongs to exactly one batch (listed). Another batch that
  needs a change there asks the owner, or lands a minimal hunk with the owner named in the
  commit body.
- **First landing**: `gui-fix-shell` lands **S0 tokens + primitives first**: A2.2–A2.5 in
  `design/*`, plus `button`, `chip`, `card`, `section_label`, `kbd`, `page_header`,
  `DetailHeader` and dialog. Others build on them.
- **What can start at once**: the crash and freeze fixes (GA-1 + core GA-40, GA-30 + core
  GA-42, GA-32) start immediately, because they need no tokens.
- **Area E (core / backend) ids GA-40…GA-46** belong to the fifth batch **core-gui-gaps**
  (below). The GUI batches only consume its results.
- **Mobile**: `apps/oino-ui/src/mobile/**` is not in these batches (follow-up
  `gui-fix-mobile` per A2.19). Shared component changes must keep it compiling.
- **Screenshots**: refresh `docs/dev/v1/design/shots/desktop/*` for every visual change, and
  compare side by side with `design/ref/unipi/*`.

**gui-fix-conversation**: conversation, timeline, composer, interaction.

Files owned:
- `apps/oino-ui/src/components/{conversation.rs, composer.rs, dock.rs, query_rail.rs, timeline/{mod,user,tool,cards}.rs, interaction/{mod,approval,question,plan,dock_dialog}.rs}`
- `apps/oino-ui/src/views/session_pane.rs`
- `apps/oino-ui/src/model/{composer,timeline,transcript,tools,interaction,dialog,coalesce}.rs`
- extension: `extensions/unipi/workflow` (active-tools restore only)

(The core files behind GA-40/GA-42 belong to core-gui-gaps.)

GA ids (15): **GA-1** (P0 crash), **GA-30** (P0 orphan approvals), GA-2, GA-3, GA-4, GA-5,
GA-6, GA-7, GA-21, GA-25, GA-27, GA-28, GA-29, GA-8, GA-9.

| Step | Files | Change |
|---|---|---|
| C1 crash | `views/session_pane.rs:966-981` (`on_text_changed`) → spawn on the stored tokio `Handle` (`rt.rs`), never on GPUI's `background_executor` | GA-1 (core side is K1 / GA-40) |
| C2 no orphan cards | `model/dialog.rs`, `components/interaction/mod.rs` (drop a card on `extension_ui_dismiss`; queue "1 of N"; a stale card never blocks send: the composer refuses only while a live request is pending) | GA-30 (handler deadline is K2 / GA-42) |
| C3 input integrity | `views/session_pane.rs` (`pending_text` / `set_value` race after `/`; completion refresh must not overwrite typed keys), `model/composer.rs:42` `intent()` dispatches pi built-ins `/new /compact /model /resume /name /fork /tree /copy /export /reload …` locally; ↓ on empty editor → `emptyArrowDown` editor event → tray panel, `/unipi:subagents` opens the tray tab | GA-2, GA-3, GA-21 |
| C4 composer look | `components/composer.rs` (A2.11 shell R 26, placeholder, attach circle, mode chip from `unipi:workflow:status` topic, model chip with ring → control centre, send/stop circle + chevron Steer/Queue, glance frame slot rendering `setFrame`/`setFooter` provided by `meta_row.rs`), completion menu (one group per source, enchanted replaces base, colour tags) | GA-4, GA-25 (glance slot shared with GA-26) |
| C5 timeline look | `components/timeline/{user,tool,cards,mod}.rs`, `model/{timeline,tools,transcript}.rs` (A2.10: bubble, prose tokens, **fold line per turn** model, tool row 32, tool body, harness card collapsed by default for memory/bg notification/plan-mode reminder/kanboard tables via OUS `table`, turn end, Changes this turn card, inline error row, remove gutter glyph artifact) | GA-6, GA-8, GA-9 (dedupe "Compaction failed:", no "Switch model" for non-model errors) |
| C6 header slot | `components/conversation.rs` (start header only in empty sessions, rendered as EmptyState crab 88 + build line; collapsed harness card otherwise; welcome per A2.17) | GA-5 |
| C7 interaction cards | `components/interaction/*` (A2.11 card in composer slot, radius 24, options 46, stable layout: feedback field reserved at first paint, no height jump; dialogs scroll inside max height), `components/dock.rs` (queue strip, run-state line) | GA-7, GA-27, GA-29 (row labels "· None" → model/provider; list scroll fade) |
| C8 plan-mode tools | `extensions/unipi/workflow` (restore `ask_user`, `bg_*` when leaving plan mode) | GA-28 |

**gui-fix-panes**: side pane, browser host shell only, terminal, diff, log.

Files owned:
- `apps/oino-ui/src/components/{side_pane.rs, browser.rs (host chrome only, not the webview crate), terminal.rs, log_view.rs, tabs.rs}`
- the diff viewer inside `side_pane.rs`

GA ids (4): GA-13, GA-14, GA-15, GA-18. (The `_oino/agents` feed, GA-44, is core-gui-gaps K5.)

| Step | Files | Change |
|---|---|---|
| P1 look | `tabs.rs`, `side_pane.rs` | Side pane = flush right column (rule `line_soft`), default 440, min 320. `DetailHeader` (56) per tab content instead of a ZCode tab bar for Browser/Review/Agent; tab strip only when > 1 tab: 38 px pills (chip look), close ✕ on hover; launcher `+` → anchored menu (Review, Files, Plan, Tree, Browser, Terminal, extension panels) |
| P2 routing | `side_pane.rs:152` (`openPanel sidePane` for `unipi.app-bridge/control`), `side_pane.rs:338-345` (`+` opens the launcher) | GA-18, GA-13 |
| P3 tasks + agents | `side_pane.rs` Tasks tab reads the `unipi:bg:tasks` topic (rows: name, command mono 12, status capsule, elapsed, logs/kill pills; empty state per A2.14); Agent tab = read-only child timeline using the conversation batch's timeline components once K5 lands; until then a compact EmptyState "Live view needs the agents feed" | GA-14, GA-15 |
| P4 browser host | `browser.rs` | `DetailHeader`-style URL row (40 high pill field, back/forward/reload/open-externally 32 round), agent-driving strip "Agent is using the browser · Take over" (`attention_surface`), localhost chip in the chat (`m7-desktop-browser-pane.png`) |
| P5 terminal / diff / log | `terminal.rs`, `log_view.rs`, diff in `side_pane.rs` | Terminal bg surface-1, tab strip 38 (term tabs, not sessions), ANSI from tokens; diff with `diff_*_bg/_fg`, file card radius 16; log view mono 11.5/17, thin scrollbars A2.5 |

**gui-fix-unipi**: OUS, board/kanban, tray, footer/meta row, status panel, Unicrab/mascot,
unipi pages.

Files owned:
- `apps/oino-ui/src/components/{ous/{mod,kanban}.rs, meta_row.rs, status_panel.rs, unicrab.rs, empty.rs}`
- `apps/oino-ui/src/views/{board.rs, mascot.rs}`
- `apps/oino-ui/src/model/{ous.rs, kanban.rs, status.rs, ui_state.rs}`
- `apps/oino-ui/src/design/brand.rs` and the new `design/lore.rs`
- extensions: `extensions/unipi/{info-screen, updater, memory, kanboard}` (only the parts
  named here)

GA ids (11): **GA-17** (P0 board), **GA-19** (P0 readme/changelog), **GA-32** (P0 mascot
crash), GA-20, GA-22, GA-23, GA-26, GA-10, GA-12, GA-31, GA-39.

| Step | Files | Change |
|---|---|---|
| U1 mascot crash | `views/mascot.rs` (render from a `MascotSnapshot` passed in; never read `Workspace`/`AppShell` inside its update), plus a request to shell: `sync_mascot` defers `MascotView::open` with `cx.defer` | GA-32 |
| U2 board | `views/board.rs:96,246` (`/unipi:kanboard open`, New task → native New task dialog or `/unipi:kanboard-add`), `components/ous/kanban.rs` (A2.17 board look: lanes, cards, lane strip, dashed empty, FAB) — needs K3 (GA-41) for the binary | GA-17, GA-20 |
| U3 OUS panels | `components/ous/mod.rs` (dialog look per A2.12; panel `keys` → footer legend + key bindings, never buttons; title vs footer hint drawn once; Esc closes/back; `meter` values formatted (no raw floats); `barChart`/`sparkline` drawn; tabs as segmented pills), `model/ous.rs` | GA-22, GA-23 |
| U4 tray | `model/ui_state.rs` (editorReplace panel lifecycle), `ous/mod.rs` tray rendering in the composer slot (radius 26 shell, segmented tabs, rows 56) | GA-22 (tray part) |
| U5 footer glance | `components/meta_row.rs` (A2.11 glance frame: top/bottom segments with 1 px dividers, wordmark brand, toned numbers, priority dropping, never wraps; session strip summary for `DetailHeader`) | GA-26 |
| U6 shortcuts table | `model/ui_state.rs:603` exposes the per-extension `setShortcuts` table delivered by K4 to shell's key dispatcher (S3) | — (consumes K4 / GA-43) |
| U7 status | `components/status_panel.rs`, `model/status.rs` (plan todos only from workflow plan, not assistant questions; working-mode chip + Progress sheet per L10; status panel opt-in) | GA-10 |
| U8 Unicrab + empty | `design/brand.rs` (sprite + anim tables from unipi JSON), `components/unicrab.rs` (rig, states, header crab 28 with badge, needs-you drawer), `components/empty.rs` (A2.14), new `design/lore.rs` | L7, L8 |
| U9 extension content | `extensions/unipi/updater` (bundled README/CHANGELOG paths vs `oino_bundle::DOCS`), `extensions/unipi/info-screen` (local hour greeting, usage index), `extensions/unipi/memory` (import prompt once, not per session), hint browser "learned" toggle; the Extensions page Readme button (`views/extensions_page.rs:92`, shell-owned file) is a one-line hunk landed by shell in S4 under GA-19 | GA-19, GA-12, GA-39, GA-31 |

**gui-fix-shell**: shell, sidebar, header, palette, settings, extensions, onboarding,
theme/tokens.

Files owned:
- `apps/oino-ui/src/views/{shell.rs, mod.rs, settings_page.rs, extensions_page.rs, onboarding.rs}`
- `apps/oino-ui/src/components/{sidebar.rs, header.rs, window_chrome.rs, palette.rs, keys.rs, menu.rs, dialog.rs, toast.rs, button.rs, badge.rs, controls.rs, tooltip.rs, settings.rs, model_menu.rs, mod.rs}` and the new `components/{page_header,chip,card,section_label,kbd}.rs`
- `apps/oino-ui/src/design/{tokens,theme,type_scale,motion,icons,ticker,tones,tests,syntax,mod}.rs`
- `apps/oino-ui/src/{host.rs, platform.rs, lib.rs}`
- `apps/oino-ui/src/model/{palette,session,sessions}.rs`
- `apps/oino-desktop/src/{main.rs, prefs.rs, demo.rs}`
- `crates/oino-resources/src/discovery.rs`

GA ids (9): **GA-16** (P0 shortcuts dead), GA-35, GA-36, GA-11, GA-24, GA-33, GA-34, GA-37,
GA-38.

| Step | Files | Change |
|---|---|---|
| S0 tokens + primitives | `design/tokens.rs` (A2.2 palette fields + values, A2.4 sizes/radii/elevation, A2.5 motion), `design/type_scale.rs` (`Title`, `Prose`, `Xs` ui−3), `design/theme.rs` (gpui-kit derivation incl. scrollbar thumb, focus off), `design/tests.rs` (contrast for new pairs; guard), new `components/{chip,card,section_label,kbd,page_header}.rs`, `button.rs` (A2.15), `dialog.rs` (A2.12), `menu.rs` (radius 16, rows 40), `toast.rs` (A2.12 + multi-line result card) | GA-36, GA-24 |
| S1 no multiplexer | `views/shell.rs` (delete split/layout/`Pane`; rename `Workspace` → `AppShell`, `MAX_LIVE_PANES` → `MAX_WARM_SESSIONS`; flush columns L1; list column 400 resizable; switch sheet Ctrl+O; startup reopens the last session instead of creating an empty one), `components/sidebar.rs` (A2.8 nav sidebar 224 + list column with project sections and needs-you strip; drop ViewSwitcher and "Open in new pane"), `views/mod.rs` keymap, `model/sessions.rs` | GA-35, GA-11 |
| S2 headers | `components/header.rs` → `DetailHeader` (A2.7), `window_chrome.rs` (client decorations over the flush layout; title "● Oino" on needs-you) | L3 |
| S3 keys + palette | `components/keys.rs` (bind extension `setShortcuts` from GA-43 table; Ctrl+M control centre; resolve `ctrl+b` collision: sidebar = Ctrl+B only when the composer is not focused, else extension wins with a toast once), `components/palette.rs`, `model/palette.rs` (A2.17 palette; Extensions group lists shortcuts), Settings → Keyboard shortcuts lists them | GA-16, GA-34 |
| S4 settings + extensions | `views/settings_page.rs` (A2.16 layout; Permissions finds namespace by extension id `unipi.workflow` not `workflow` (1249-1254); General/Sessions editable; About with Check for updates; Remote hidden), `views/extensions_page.rs` (A2.17 rows; Readme button opens the updater readme panel :92), `views/onboarding.rs` | GA-33, GA-37 (+ the GA-19 Readme hunk for unipi) |
| S5 desktop shell | `apps/oino-desktop/src/main.rs`, `prefs.rs` (guard: a failing mascot open resets `mascotWindow` instead of crash-looping; window min size, client decorations over the flush layout), `demo.rs` (demo emits the A2 surfaces for screenshots) | hardening for GA-32 (id owned by unipi) |
| S6 skills discovery | `crates/oino-resources/src/discovery.rs:48` (no implicit `~/.agents/skills` when an agent dir is set) | GA-38 |
| S7 ticker | `design/ticker.rs` (one app-wide ticker that stops when nothing animates; no view requests a refresh while idle) | — (input to K6 / GA-46) |

**core-gui-gaps**: area E of the audit (core / backend gaps behind the GUI). This is a
non-GUI batch with the same gates plus the root workspace `cargo test`.

Files owned:
- `crates/oino-core/src/session/frontend.rs`, `crates/oino-core/src/extensions/runner.rs`
- `crates/oino-wasm-host/src/instance.rs`
- the modes / frontend-protocol code that delivers `setShortcuts` and `_oino/agents`
  (`crates/oino-modes`, `oino_protocol::ui`)
- `apps/oino-desktop/src/embedded.rs` and the desktop packaging scripts (`apps/oino-desktop/scripts/*`)
- `apps/oino-ui/src/views/soak.rs`, `apps/oino-ui/src/design/perf.rs`

GA ids (7): **GA-40** (P0 runtime panic), GA-41, GA-42, GA-43, GA-44, GA-45, GA-46.

| Step | Files | Change |
|---|---|---|
| K1 runtime-agnostic core | `crates/oino-core/src/session/frontend.rs:447` (bare `tokio::spawn` → the session's stored runtime `Handle`; audit other bare spawns on frontend-called paths) | GA-40 (+ a test calling `editor_event` from a non-tokio thread); unblocks C1 |
| K2 dialog-aware deadline | `crates/oino-wasm-host/src/instance.rs:401` (pause the 30 s handler deadline while the handler awaits a UI dialog), `crates/oino-core/src/extensions/runner.rs:361` (on handler death or timeout send `extension_ui_dismiss{reason:"cancelled"}` for its open dialogs) | GA-42; unblocks C2 |
| K3 kanboard binary | `embedded.rs` (`HostConfig` program search: next to the executable, bundle `libexec`, then `PATH`), packaging scripts bundle `oino-kanboard` | GA-41; unblocks U2 |
| K4 extension shortcuts delivery | frontend protocol + `embedded.rs` (deliver `setShortcuts` to the GPUI frontend and keep the table across reloads; the hunk in `model/ui_state.rs:603` is landed with unipi named) | GA-43; unblocks S3 / GA-16 |
| K5 agents feed | modes / core runtime (`_oino/agents` child-session event stream, STATUS §8.2) | GA-44; unblocks P3 / GA-15 |
| K6 idle cost | `views/soak.rs` (soak = 1 visible + 3 background sessions streaming), `design/perf.rs`, runtime thread pools (core + wasm host worker counts) | GA-46: idle CPU ≈ 0 with GPU, threads ≤ 100 with 2 sessions, RSS slope logged |
| K7 frontend mode | `embedded.rs` (report the GPUI frontend, `ExtensionMode` desktop, to host-info and `/unipi:doctor`) | GA-45 |

**GA assignment check** (every id in [gui-audit.md](gui-audit.md) exactly once):

| Batch | Ids | Count |
|---|---|---|
| gui-fix-conversation | 1, 2, 3, 4, 5, 6, 7, 8, 9, 21, 25, 27, 28, 29, 30 | 15 |
| gui-fix-panes | 13, 14, 15, 18 | 4 |
| gui-fix-unipi | 10, 12, 17, 19, 20, 22, 23, 26, 31, 32, 39 | 11 |
| gui-fix-shell | 11, 16, 24, 33, 34, 35, 36, 37, 38 | 9 |
| core-gui-gaps | 40, 41, 42, 43, 44, 45, 46 | 7 |
| **Total** | GA-1 … GA-46 | **46** |

Order across batches: start K1, K2, C1, C2, U1 and S0 at once. Then run S1/S2 with C4/C5/C7
and U2/U3/U5, with K3 before U2 and K4 before S3. P1–P5 and U8 come next, followed by S3/S4
and K5/K6/K7. Re-run the GUI audit (`gui-audit`) after all five batches land, and close the GA
ids in `gui-audit.md`.


---

## 1. Principles

1. **Calm, dense, operational** (ZCode `DESIGN.md`). Hierarchy through text weight and color
   first, surfaces second, borders third, shadows last. No gradients, no brand fills on large
   surfaces.
2. **One token table.** Every color, size, radius and duration in views comes from
   `oino_ui::design` (§2). No `hsla(..)`, `px(..)` literals or gpui-kit default colors in view
   code outside `design/`. Enforced by a unit test that greps `src/views` and `src/mobile` for
   `hsla(` / `rgb(` / `px(` outside allowed helpers (§9.3).
3. **Speed is a design feature.** Every component states its frame cost; nothing animates
   while text streams except the caret and spinners (§2.10, §2.11).
4. **Blocking interactions dock, they do not cover.** Approvals, questions and plan review
   render as cards docked above the composer (ZCode `ChatBottomDock`), never as modal scrims
   over the transcript. Modals are reserved for app-level tasks (palette, settings dialogs,
   overlays requested by extensions).
5. **Extensions name intents, Oino owns pixels.** OUS tones map to tokens (§2.2); panel
   placements map to fixed regions (§3.3); unknown nodes degrade (`fallback`).
6. **Desktop and mobile share components, not layouts.** Components (§4) live in
   `apps/oino-ui/src/design/` and `src/components/`; desktop and mobile compose them into
   different screens (§5, §6). Mobile uses drawers and bottom sheets where desktop uses side
   pane tabs and popovers (ZCode `/remote` rule).
7. **Keyboard first on desktop, thumb first on mobile.** Every desktop action is reachable from
   the palette; every mobile target is ≥ 44 pt.

---

## 2. Design tokens

All tokens live in `apps/oino-ui/src/design/tokens.rs` as `const` data and are installed into a
GPUI global `OinoTheme` (`design::theme::OinoTheme { mode, palette: &'static Palette, type_scale,
.. }`). gpui-kit's `Theme` is **derived** from the palette on every theme change (§2.3) so stock
gpui-kit components match.

### 2.1 Color palette (Oino Light / Oino Dark)

Values are ZCode **Zai Light / Zai Dark** (`styles.css` `.theme-zai-light` / `.theme-zai-dark`),
resolved to concrete RGBA (Tailwind `neutral-800` = `#262626`, `neutral-300` = `#d4d4d4`,
`color-mix(... 60%, transparent)` = alpha `0x99`, 40 % = `0x66`, 30 % = `0x4d`). Oino-specific
additions are marked **(O)**.

```rust
// apps/oino-ui/src/design/tokens.rs
/// 0xRRGGBBAA, converted with gpui::rgba at theme install time.
pub struct Palette {
    // structure
    pub window: u32,              // window root behind sidebar and frames
    pub sidebar: u32,             // sidebar fill (drawn over `window`)
    pub frame: u32,               // conversation / side pane / bottom frames
    pub header: u32,              // popover headers, sticky headers inside frames
    pub surface: u32,             // low-elevation fill (user bubble, tool detail, code)
    pub surface_hover: u32,
    pub card: u32, pub card_border: u32, pub card_selected: u32,
    pub popover: u32, pub popover_border: u32,
    pub menu: u32, pub menu_hover: u32,
    pub input: u32, pub input_border: u32, pub input_border_hover: u32, pub input_border_focus: u32,
    pub border: u32, pub border_hover: u32,
    pub hover: u32, pub selected: u32,
    pub tab: u32, pub tab_active: u32,
    pub tag: u32, pub tooltip: u32, pub tooltip_fg: u32, pub toast: u32,
    pub scrim: u32, pub resize_handle: u32,
    // text
    pub fg: u32, pub fg_subtle: u32, pub fg_subtlest: u32, pub fg_inverse: u32,
    // actions
    pub primary: u32, pub primary_fg: u32, pub secondary: u32,
    pub brand: u32,               // (O) Oino mark, needs-you crab, focus ring
    pub accent: u32,              // weak emphasis surface
    // semantic
    pub success: u32, pub success_surface: u32, pub success_surface_fg: u32,
    pub warning: u32, pub warning_surface: u32, pub warning_surface_fg: u32,
    pub danger: u32, pub danger_surface: u32, pub danger_surface_fg: u32,
    pub progress_surface: u32, pub progress_surface_fg: u32,
    pub waiting_surface: u32, pub waiting_fg: u32,       // ZCode interaction-confirmation
    pub idle_task: u32, pub idle_task_surface: u32,       // queued / scheduled (kanboard autowork)
    // content
    pub diff_added: u32, pub diff_removed: u32, pub diff_added_bg: u32, pub diff_removed_bg: u32,
    pub git_modified: u32, pub git_added: u32, pub git_deleted: u32, pub git_renamed: u32,
    pub find: u32, pub find_active: u32,
    pub file_node: u32, pub file_node_fg: u32,
    pub skill_node: u32, pub skill_node_fg: u32,
    pub command_node: u32, pub command_node_fg: u32,
    pub agent_node: u32, pub agent_node_fg: u32,         // subagent mention / row
    pub session_node: u32, pub session_node_fg: u32,
    pub harness: u32, pub harness_surface: u32,          // (O) provenance rail (violet)
    pub thinking: u32,                                    // (O) thinking text
    pub ansi: [u32; 16],                                  // terminal / log colors
    pub categorical: [u32; 12],                           // OUS c1..c12
}
```

| Token | Oino Light | Oino Dark | Source / use |
|---|---|---|---|
| `window` | `#ECECEEFF` | `#2B2B2BFF` | Zai `background-win-alt`; window root on all OSes (no vibrancy) |
| `sidebar` | `#00000000` | `#00000000` | transparent over `window` (ZCode sidebar sits on the root) |
| `frame` | `#F8F8F8FF` | `#161616FF` | Zai `background`; conversation, side pane, bottom frames |
| `header` | `#F8F8F8FF` | `#202020FF` | Zai `popover-header` |
| `surface` | `#0D0D0D08` | `#FFFFFF0D` | 3 % / 5 % |
| `surface_hover` | `#0D0D0D0D` | `#FFFFFF1A` | |
| `card` | `#FFFFFFFF` | `#2B2B2BFF` | |
| `card_border` / `border` | `#0D0D0D1A` | `#FFFFFF1A` | 10 % |
| `card_selected` | `#FFFFFFFF` | `#2B2B2BFF` | = input |
| `border_hover` | `#0D0D0D26` | `#FFFFFF26` | 15 % |
| `popover` / `menu` | `#FFFFFFFF` | `#2B2B2BFF` | |
| `popover_border` | `#0D0D0D1A` | `#FFFFFF1A` | |
| `menu_hover` | `#F0F0F0FF` | `#363636FF` | |
| `input` | `#FFFFFFFF` | `#2B2B2BFF` | composer shell, fields |
| `input_border` | `#0D0D0D1A` | `#FFFFFF1A` | |
| `input_border_hover` / `_focus` | `#0D0D0D26` | `#FFFFFF26` | focus is calm: border step, no glow |
| `hover` | `#0D0D0D0D` | `#FFFFFF0D` | rows, ghost buttons |
| `selected` | `#0D0D0D0D` | `#FFFFFF1A` | active session row, selected list row |
| `tab` / `tab_active` | `#F0F0F0FF` / `#FFFFFFFF` | `#202020FF` / `#161616FF` | |
| `tag` | `#E6E6E6FF` | `#363636FF` | badges, inline code bg |
| `tooltip` / `tooltip_fg` | `#F0F0F0FF` / `#0D0D0DFF` | `#2B2B2BFF` / `#F8F8F8FF` | |
| `toast` | `#FFFFFFF2` | `#2B2B2BF2` | 95 % (GPUI has no backdrop blur) |
| `scrim` | `#0000004D` | `#00000080` | modal backdrop |
| `resize_handle` | `#0D0D0D33` | `#FFFFFF33` | 2 px line on hover/drag (= `fg_subtlest`/50) |
| `fg` | `#262626FF` | `#D4D4D4FF` | |
| `fg_subtle` | `#26262699` | `#D4D4D499` | metadata, tool verbs |
| `fg_subtlest` | `#26262666` | `#D4D4D44D` | placeholders, timestamps |
| `fg_inverse` | `#FFFFFFFF` | `#000000FF` | |
| `primary` / `primary_fg` | `#000000FF` / `#FFFFFFFF` | `#FFFFFFFF` / `#000000FF` | primary button, send button |
| `secondary` | `#E6E6E6FF` | `#363636FF` | |
| `brand` **(O)** | `#E0301AFF` | `#FF5A3CFF` | Unicrab shell red; icon/rail/focus only, never fills > 32 px |
| `accent` | `#EBF4FFFF` | `#001D3DFF` | |
| `success` | `#1E8A3EFF` | `#46BF72FF` | |
| `success_surface` / `_fg` | `#EAF7EEFF` / `#166B32FF` | `#46BF7229` / `#87D9A4FF` | |
| `warning` | `#E07B00FF` | `#FF8A30FF` | also "running" |
| `warning_surface` / `_fg` | `#E07B001F` / `#914E00FF` | `#FF8A3029` / `#FFA159FF` | |
| `danger` | `#E03131FF` | `#FF5C5CFF` | |
| `danger_surface` / `_fg` | `#E031311A` / `#B42020FF` | `#FF5C5C29` / `#FF8585FF` | |
| `progress_surface` / `_fg` | `#EFF6FFFF` / `#1D4ED8FF` | `#4682F029` / `#93C5FDFF` | connecting, updating, compacting |
| `waiting_surface` / `waiting_fg` | `#EAF7EEFF` / `#166B32FF` | `#46BF7229` / `#87D9A4FF` | every "Waiting for you" badge (one treatment) |
| `idle_task` / `_surface` | `#9E77EDFF` / `#F5F3FFFF` | `#7B5CE5FF` / `#160D38FF` | queued follow-ups, kanboard autowork |
| `diff_added` / `diff_removed` | `#1E8A3EFF` / `#E03131FF` | `#46BF72FF` / `#FF5C5CFF` | gutter bars, `+n −m` |
| `diff_added_bg` / `diff_removed_bg` **(O)** | `#1E8A3E1A` / `#E031311A` | `#46BF721F` / `#FF5C5C1F` | line backgrounds (10 % / 12 %) |
| `git_modified/added/deleted/renamed` | `#E07B00` `#1E8A3E` `#E03131` `#0B7FFF` | `#FF8A30` `#46BF72` `#FF5C5C` `#4099FF` | file status letters |
| `find` / `find_active` | `#FFF4EBFF` / `#FFB26BFF` | `#542500FF` / `#FF8A30FF` | conversation find |
| `file_node` / `_fg` | `#1A70B81A` / `#1A70B8FF` | `#70AEE029` / `#8FC5EFFF` | `@file` tokens |
| `skill_node` / `_fg` | `#7453B01A` / `#7453B0FF` | `#A689DA29` / `#BDA5E6FF` | `/skill:` tokens |
| `command_node` / `_fg` | `#5662701A` / `#566270FF` | `#9AA6B424` / `#B5C0CCFF` | `/command` tokens |
| `agent_node` / `_fg` | `#6F7A2F1A` / `#6F7A2FFF` | `#B7BD7529` / `#C8CD90FF` | subagent rows/tokens |
| `session_node` / `_fg` | `#14807A1A` / `#14807AFF` | `#68C4BC29` / `#93D8D2FF` | session references |
| `harness` / `harness_surface` **(O)** | `#7453B0FF` / `#7453B014` | `#BDA5E6FF` / `#A689DA1F` | provenance rail = skill violet |
| `thinking` **(O)** | `#26262699` | `#D4D4D499` | = `fg_subtle`, italic off |

`ansi` (black, red, green, yellow, blue, magenta, cyan, white, then bright ×8):
light `5C5C5C E03131 1E8A3E E07B00 0B7FFF 9E77ED 0AA7A7 ADADAD 888888 E03131 1E8A3E E07B00
0066DD 9E77ED 0AA7A7 0D0D0D`; dark `363636 FF5C5C 46BF72 FF8A30 4099FF 7B5CE5 42C8C8 ADADAD
747474 FF9999 87D9A4 FFB26B 80BEFF A888F2 8EE5E5 F8F8F8` (Zai terminal colors).

`categorical` (OUS `c1`…`c12`; usage-chart colors first, then mention-node foregrounds):
light `0B7FFF 1E8A3E 9E77ED E03131 E07B00 0AA7A7 1A70B8 7453B0 566270 6F7A2F 14807A B87A1A`;
dark `4099FF 46BF72 7B5CE5 FF5C5C FF8A30 42C8C8 8FC5EF BDA5E6 B5C0CC C8CD90 93D8D2 EFC58F`.

Theme choice: **System** (default; follows `window.appearance()`), **Light**, **Dark**. If the
platform reports no appearance, Dark (ZCode default). Custom pi theme files remain discovered
but unrendered (STATUS §2); `setTheme{name}` with `light`/`dark`/`system` switches mode, other
names are ignored.

Contrast rules (checked by `design::tests::contrast`): `fg`, `fg_subtle` on `frame`, `card`,
`input`, `menu` ≥ 4.5:1; `*_surface_fg` on `*_surface` over `frame` ≥ 4.5:1; `fg_subtlest` is
never used for content, only placeholders, timestamps and disabled text.

### 2.2 OUS tones → tokens

| OUS tone (ui-protocol §6.5) | Text / icon | Rail / bar | Surface (badge bg) |
|---|---|---|---|
| `default`, `assistant` | `fg` | `border_hover` | `tag` |
| `muted` | `fg_subtle` | `border` | `surface` |
| `thinking` | `thinking` | `border` | `surface` |
| `accent` | `fg` + medium weight | `brand` | `accent` |
| `user` | `fg` | `fg_subtlest` | `surface` |
| `info`, `tool` | `progress_surface_fg` | `ansi[12]` (bright blue) | `progress_surface` |
| `success` | `success_surface_fg` | `success` | `success_surface` |
| `warning` | `warning_surface_fg` | `warning` | `warning_surface` |
| `error` | `danger_surface_fg` | `danger` | `danger_surface` |
| `harness` | `harness` | `harness` | `harness_surface` |
| `c1`…`c12` | `categorical[i]` | `categorical[i]` | `categorical[i]` at alpha `0x1F` |

Text uses the `_fg` column so colored text keeps 4.5:1; bars, dots, meters and rails use the
saturated column. OUS icon names map to Lucide (§2.9).

### 2.3 Derived gpui-kit theme

`design::theme::install(mode, cx)` builds a `ThemeConfig` per mode from the palette and
registers it as gpui-kit themes "Oino Light" / "Oino Dark" (`ThemeRegistry`), then
`Theme::change`. Mapping (non-exhaustive fields default to the nearest row):

| gpui-kit `ThemeColor` | Palette |
|---|---|
| `background` | `frame` |
| `foreground` | `fg` |
| `muted`, `secondary` | `surface`, `secondary` |
| `muted_foreground` | `fg_subtle` |
| `border`, `input`, `ring` | `border`, `input_border`, `input_border_focus` |
| `primary`, `primary_foreground`, `primary_hover`, `primary_active` | `primary`, `primary_fg`, `primary` at 90 %, 80 % |
| `accent`, `accent_foreground` | `hover`, `fg` |
| `popover`, `popover_foreground` | `popover`, `fg` |
| `list_hover`, `list_active`, `list_active_border` | `hover`, `selected`, transparent |
| `sidebar*` | `window` / `fg` / `selected` / `border` |
| `tab_bar`, `tab`, `tab_active`, `tab_foreground`, `tab_active_foreground` | `frame`, `tab`, `tab_active`, `fg_subtle`, `fg` |
| `success*`, `warning*`, `danger*`, `info*` | semantic rows (`info` = `ansi[12]`) |
| `title_bar`, `status_bar` | `window` |
| `scrollbar_thumb`, `scrollbar_thumb_hover` | `border`, `border_hover` |
| `selection` | light `#0B7FFF38`, dark `#4099FF47` |
| `caret` | `fg` |
| `overlay` | `scrim` |
| `chart_*`, `red`…`cyan(_light)` | `categorical` / `ansi` |

Theme fields: `font_family` = UI font (§2.4), `mono_font_family` = JetBrains Mono, `radius` =
8, `radius_lg` = 12, `shadow` = true, `focus_ring` = false (Oino draws focus per §2.6 rules),
highlight theme = `design/syntax.rs` (Zai-like: keywords `ansi[5]`, strings `ansi[2]`,
numbers `ansi[3]`, functions `ansi[4]`, types `ansi[6]`, comments `fg_subtlest`).

### 2.4 Typography

Fonts (desktop): **Inter** (UI, variable TTF, SIL OFL 1.1) and **JetBrains Mono** (mono, OFL
1.1) bundled in `apps/oino-ui/assets/fonts/` and registered with `cx.text_system().add_fonts`
at startup, so Linux, Windows and macOS render identically. CJK falls back to the system
(`PingFang SC`, `Microsoft YaHei UI`, `Noto Sans CJK SC`). Mobile uses the platform UI font
(`.SystemUIFont` on iOS, Roboto on Android) plus bundled JetBrains Mono.

The UI scale follows ZCode `text-ui-*` exactly, as offsets from one setting `ui_font_size`
(default 14 px desktop, 15 px mobile; range 12–18, Settings → Appearance). Code, diff and
log/terminal content use an **independent** `code_font_size` (default 13 px desktop, 13 px
mobile; range 10–20).

```rust
// apps/oino-ui/src/design/type_scale.rs
pub enum TextRole { Display, Xl, Lg, Base, Caption, Sm, Xs }
pub const fn size(role: TextRole, ui: f32) -> (f32 /*size*/, f32 /*line height*/) {
    match role {
        TextRole::Display => (ui + 16.0, ui + 22.0), // 30/36: welcome greeting, settings page title
        TextRole::Xl      => (ui + 4.0,  ui + 12.0), // 18/26: markdown h1, dialog titles
        TextRole::Lg      => (ui + 2.0,  ui + 10.0), // 16/24: markdown h2, sheet titles (mobile)
        TextRole::Base    => (ui,        ui + 6.0),  // 14/20: body, buttons, rows, markdown h3–h6
        TextRole::Caption => (ui - 1.0,  ui + 4.0),  // 13/18: compact supporting copy
        TextRole::Sm      => (ui - 2.0,  ui + 2.0),  // 12/16: metadata, tooltips, inline code, meta row
        TextRole::Xs      => (ui - 4.0,  ui),        // 10/14: badges, counters, kbd hints
    }
}
```

Weights: 400 body, 500 labels/titles (`FontWeight::MEDIUM`), 600 only for markdown h1–h4 and
the Display role. No 700 in UI chrome. Mono is used for paths, commands, model ids, shortcut
keys, hashes and code; inline code uses `Sm` mono on `tag`. Markdown: h1 `Xl`/600, h2 `Lg`/600,
h3–h4 `Base`/600, h5 `Base`/500, h6 `Base`/400; paragraph spacing 8 px; list indent 20 px;
code blocks `code_font_size`, header row `Sm`. Mobile editable inputs use 16 px minimum (iOS
zoom rule) regardless of `ui_font_size`.

### 2.5 Spacing grid

Base unit **4 px**. Allowed spacing values: `S0=0, S1=4, S2=8, S3=12, S4=16, S5=20, S6=24,
S8=32, S10=40, S12=48`. Use: 4 icon–text gaps; 8 control padding and inline gaps; 12 dense list
rows and menu rows; 16 card/panel padding and conversation gutters; 20–24 dialog interiors;
32–48 empty-state and welcome rhythm. Frame gaps (between sidebar/frames) are 4 px (the resize
hit area). Window outer inset: 4 px Linux/macOS, 6 px Windows (ZCode
`workspace-independent-panels.md`).

### 2.6 Radii

Tailwind v4 values: `R_SM=4, R_MD=6, R_LG=8, R_XL=12, R_2XL=16, R_FULL=9999`. ZCode nesting rule:
the first rounded container is `R_XL`; each nested rounded container steps down
(`XL → LG → MD → SM`, floor `SM`). Basic controls (buttons, inputs, select triggers) are `R_LG`
unless their nearest rounded parent is `R_LG` (→ `R_MD`) or `R_MD`/`R_SM` (→ `R_SM`).
Approved `R_2XL` exceptions: composer shell, status panel, dialogs and sheets (desktop), toasts,
the welcome brand mark. Menus/popovers/selects: shell `R_LG`, items `R_MD`. Frames: `R_XL`
(Windows `R_MD`; maximized window edges 0). Pills (`R_FULL`) only for status capsules, dots,
the Unicrab FAB and avatar circles. Mobile sheets: `R_2XL` top corners only.

Focus: keyboard focus draws a 2 px `input_border_focus` ring outside the control's radius
(+2 px); mouse focus draws nothing. Composer and inputs show focus by border step only.

### 2.7 Sizing

Icons: 12, 14, **16 (default)**, 20, 24; stroke 1.5 px (Lucide). Controls: heights 20 (xs), 24
(sm), **28 (default button)**, 32 (lg / menu rows / sidebar rows), 36 (form inputs). Icon
buttons square 24/28/32. Sidebar: default 264, min 264, max 50 % of window. Side pane: default
440, min 320, max 60 %; auto-collapses when the conversation frame would drop below 480.
Bottom frame: default 240, min 120, max 60 % of the column. Conversation reading column: max
896 px (ZCode `max-w-4xl`) centred, 16 px gutters; user bubble max 576 px. Dialog widths:
small 420, medium 560, large 760, full = window − 64. Mobile: rows 56, app bar 52, targets
≥ 44, bottom sheet max height 90 % of the safe area.

### 2.8 Elevation

| Level | Use | GPUI `BoxShadow` (light / dark) |
|---|---|---|
| E0 base | window, frames, sidebar | none; separation by `window`/`frame` contrast + `border` |
| E1 surface | cards, tool details, bubbles | none; `card_border` 1 px |
| E2 overlay | menus, popovers, palette, dialogs, status panel, docked interaction cards | `0 4 12 -2 #0000001F` + `0 2 4 -2 #00000014` / `0 4 12 -2 #00000066` + `0 2 4 -2 #0000004D` |
| E3 attention | toasts, drag previews | `0 12 32 -4 #00000029` / `0 12 32 -4 #00000080` |

### 2.9 Icons and licensing

Icon set: **Lucide** (ISC license), via `gpui_kit::assets::IconName` (Lucide 1.43.0 catalog,
`LICENSE-LUCIDE` shipped in gpui-kit-assets). Oino registers its own `AssetSource`
(`apps/oino-ui/src/design/icons.rs`) that embeds only the icons it uses (≈ 70, ≈ 25 KiB), falling
back to gpui-kit `Assets`; never `AllAssets`. Oino-only glyphs (the Oino mark, the pixel Unicrab)
are drawn as GPUI quads from data (`design/brand.rs`), not SVG. `THIRD-PARTY-NOTICES` in the
desktop bundle lists Lucide (ISC), Inter (OFL 1.1), JetBrains Mono (OFL 1.1), gpui-kit
(Apache-2.0). ZCode assets (logo, illustrations) are **not** used.

OUS icon names → Lucide: `check`→`check`, `x`→`x`, `dot`→`dot`, `warn`→`triangle-alert`,
`info`→`info`, `clock`→`clock`, `gear`→`settings`, `branch`→`git-branch`, `bolt`→`zap`,
`bot`→`bot`, `folder`→`folder`, `file`→`file`, `search`→`search`, `cloud`→`cloud`,
`lock`→`lock`; unknown → none.

App icon vocabulary (fixed): new session `square-pen`; search `search`; board `kanban`;
extensions `blocks`; settings `settings`; sidebar toggle `panel-left`; side pane
`panel-right`; bottom frame `panel-bottom`; model `box`; thinking `brain`; attach `plus`;
send `arrow-up`; stop `square`; queue `list-ordered`; approve `shield-check`; question
`message-circle-question`; plan `list-checks`; subagent `bot`; sidekick (fusion) `users`; btw
`message-square-dashed`; background task `terminal`; memory `brain-circuit`; dream `moon`;
notify `bell`; update `arrow-down-circle`; app bridge `smartphone`; kanboard card
`square-kanban`; tool read `file-text`; edit/write `file-pen`; bash `square-terminal`; grep/find
`search`; ls `folder-open`; web `globe`; mcp `plug`.

### 2.10 Motion

Durations from gpui-kit `MotionTokens`, which match ZCode's 100–300 ms range: `FAST=120 ms`
(hover/press feedback, chevron rotation, tooltip in), `NORMAL=180 ms` (menu/popover/dialog
enter: fade + 4 px slide or 0.98→1 scale; drawer and side pane open), `SLOW=280 ms` (mobile
sheet enter, status panel expand). Exit = `EXIT=120 ms` fade + 4 px drop
(`design::motion::exit`); a dismissed overlay stays mounted for that long and is then
dropped (toasts: `Toasts::dismiss` marks, `reap` drops; the run-state line: `run_exit`).
Easing enter `cubic-bezier(0.16,1,0.3,1)`, exit `ease-out`. Rules:

- Nothing animates layout while a session streams except: caret blink (530 ms), spinners
  (shared ticker, 12 fps), the composer running shimmer (one 1.6 s sweep loop on the run-state
  label only), OUS `effect: shimmer|drain` on header/splash cards.
- One app-wide ticker (`design::ticker::Ticker`, GPUI global) drives every spinner and
  relative timestamp (1 s for "now/3s", 30 s for minutes); it stops when nothing visible needs
  it (idle CPU 0).
- Setting **Reduce motion** (default: follow OS) sets every duration to 0 and freezes spinners
  on frame 0 (they become a static `loader` icon); the Unicrab holds its idle frame.
- Scrolling never animates programmatically except "jump to latest" (180 ms) and query-rail
  jumps (instant when > 3 screens away).

### 2.11 Latency and frame budgets

| Interaction | Budget (Linux dev machine, release build) | How it is met |
|---|---|---|
| Input to paint (typing in composer) | < 16 ms p99 at 60 Hz, < 8.3 ms p99 at 120 Hz | composer owns its entity; keystrokes never notify the transcript |
| Scroll, 1 session streaming | 120 fps (8.3 ms frame) on 120 Hz displays, 0 dropped frames over 10 s | `list` virtualisation, cached row layouts, markdown parse off the paint path |
| Scroll, 4 sessions streaming (split panes) | ≥ 100 fps | per-frame coalescing per session (gpui-app §3.1) |
| Streaming delta to paint | ≤ 1 frame after arrival | tail row patched only; incremental markdown (`TextViewState::push_str`) |
| Session switch (warm, ≤ 2 000 rows) | ≤ 50 ms to first frame of the new transcript | `SessionModel` kept warm 30 s; rows pre-measured |
| Session open (cold, 2 000 rows) | ≤ 150 ms to first frame (tail first, history paged in) | load tail 200 rows, then page |
| Palette open / filter keystroke (10 000 items) | ≤ 16 ms / ≤ 4 ms | fuzzy matcher over pre-lowered strings |
| Panel `updatePanel` (≤ 20/s) | ≤ 2 ms diff + layout | OUS diff by node `key` |
| Approval card appear | same frame as `extension_ui_request` | no animation on blocking cards |
| First frame / interactive after launch | ≤ 150 ms / ≤ 400 ms (gpui-app §1) | |
| Idle CPU | ≈ 0 % | no polling; ticker stops |

Debug builds show a frame-time overlay (`Ctrl+Alt+Shift+F`, `design::perf::FrameOverlay`):
last-120-frames bar chart, p50/p99, dropped frames. `apps/oino-desktop/scripts/perf.sh` runs the
demo backend with 4 streaming panes and fails if p99 > 10 ms.

---

## 3. Layout system

### 3.1 Desktop window (independent frames, ZCode)

```text
window (bg: window) ─ 4px inset ───────────────────────────────────────────────────────────────┐
│ ┌ sidebar 264 (transparent) ┐4│┌ conversation frame (frame, R_XL, border) ──┐4│┌ side frame ───┐│
│ │ ● ● ●  [◧] [←][→]   ⟨crab⟩ │ ││ header 44                                  │ ││ tab bar 36    ││
│ │ ✎ New session      ⌘N     │ ││────────────────────────────────────────────│ ││               ││
│ │ ⌕ Search           ⌘K     │ ││ ▏query rail │ timeline (max 896, centred)   │ ││ tab content   ││
│ │ ▦ Board                    │ ││ ▏           │                 ┌status ┐    │ ││               ││
│ │ ⧉ Extensions               │ ││ ▏           │                 │ panel │    │ ││               ││
│ │ [# Group][▢ Project] ⇅ ⊟   │ ││             │                 └───────┘    │ ││               ││
│ │ ▾ json-lite          3  ⊕ │ ││  dock: interaction cards · widgets · queue  │ ││               ││
│ │   Make the JSON …    now   │ ││  composer shell (R_2XL)                     │ ││               ││
│ │   Kanboard TODO …    4m    │ ││  meta row: frame segments · footer strip    │ ││               ││
│ │ ▾ unipi              1  ⊕ │ │└─────────────────────────────────────────────┘ ││               ││
│ │ ⚙ Settings  ⬇ Update  ◷   │ │┌ bottom frame (optional, R_XL) ──────────────┐ ││               ││
│ └────────────────────────────┘ ││ Output · Tasks tabs                          │ │└───────────────┘│
│                                │└─────────────────────────────────────────────┘ │                 │
└────────────────────────────────────────────────────────────────────────────────────────────────┘
```

- Frames: `frame` bg, 1 px `border`, `R_XL`, 4 px gaps carrying resize handles (transparent
  4 px hit area; 2 px `resize_handle` line on hover/drag, inset by the radius at both ends).
- Title bar: client-side decorations on Linux/Windows (gpui-kit `TitleBar` merged into the
  sidebar top row: window controls right of the side frame on Windows/Linux, traffic lights
  left on macOS with a 78 px safe area). The sidebar top row holds sidebar toggle and
  back/forward session history.
- Sidebar collapsed (`⌘B`): frames take the full width; a persistent `panel-left` button sits
  at the header's left edge.
- Side frame closed: conversation fills the row. Bottom frame closed: conversation fills the
  column.

### 3.2 Breakpoints (window width)

| Width | Behaviour |
|---|---|
| ≥ 1280 | status panel expanded (floating, right of the timeline; timeline shifts left 168 px when it would overlap) |
| 864–1279 | status panel as mini capsule; reading column `max 896` |
| 720–863 | side pane auto-collapses when opened content would squeeze the conversation below 480; sidebar stays |
| < 720 | sidebar auto-collapses to the button; side pane opens as an overlay sheet from the right (360 px) |

### 3.3 Region ownership (where extension placements land)

| Placement / source | Desktop region | Mobile region |
|---|---|---|
| `openPanel overlay` | modal dialog (size from spec: `large` default) | full-height bottom sheet |
| `openPanel sidePane` | side frame tab (icon + title) | right drawer tab |
| `openPanel editorReplace` | bottom dock replaces the composer shell, same `R_2XL` shell | same, above keyboard |
| `openPanel bottomPane` | bottom frame tab | bottom sheet (half height) |
| `openPanel fullscreen` | covers conversation frame (header kept; Esc closes) | full-screen page |
| `setHeader` | welcome hero (empty session) / slim card at top of timeline | same |
| `setFooter` | meta row right (stats strip) | composer meta row (collapsed to 2 segments) |
| `setFrame top` | composer context header (inside shell, top) | same |
| `setFrame bottom` | meta row left | hidden behind "⋯" in meta row |
| `setWidget aboveEditor` | dock strips above composer | same |
| `setWidget belowEditor` | below meta row | same |
| `setWidget header/footer` | timeline top card / meta row | same |
| `setStatus` | meta row chips (by priority) + sidebar session row tooltip | actions sheet "Status" section |
| `setWorkingMessage` | run-state line in the dock | same |
| `notify` | toast (bottom-right of the conversation frame) | toast (top, below app bar) |
| `osNotify` | OS notification when window unfocused; toast otherwise | platform notification |
| dialogs `select/confirm/input/editor/multiSelect` | popover dialog anchored in the dock (small) | bottom sheet |
| `form`, `approval`, plan review `custom` | docked interaction card (§4.12–4.14) | docked card / full sheet when > 50 % height |
| `custom` (other) | modal dialog with OUS body | bottom sheet |
| `openSettings` | Settings page, namespace selected | Settings screen |

---

## 4. Component library

Components live in `apps/oino-ui/src/components/` (one module each, GPUI `RenderOnce` unless
stated) and only read tokens from `design::`. "Keys" lists keyboard behaviour when the component
(or its container) has focus; `⌘` means the platform primary modifier (Cmd on Apple, Ctrl
elsewhere; ZCode `platform-primary-keyboard-shortcuts.md`: never both).

### 4.1 `Button`

- Anatomy: [icon 16?] label [kbd?]; padding x 8 (sm) / 10 (default) / 12 (lg); gap 6; `Base`
  text, weight 500; radius per §2.6.
- Variants: `Primary` (`primary`/`primary_fg`), `Secondary` (`secondary`/`fg`), `Outline`
  (transparent, 1 px `border`, hover `hover`), `Ghost` (transparent, hover `hover`), `Danger`
  (`danger`/white), `Link` (text `fg`, underline on hover). Sizes `Xs 20`, `Sm 24`, `Default 28`,
  `Lg 32`, `Icon{24|28|32}` square.
- States: rest, hover (+`hover` overlay or primary at 90 %), pressed (80 %), focus-visible
  (ring), disabled (text `fg_subtlest`, no hover), loading (spinner replaces icon, label kept,
  width locked).
- Keys: Enter/Space activate. One primary per panel.

### 4.2 `IconButton`, `Tooltip`, `Kbd`

- IconButton = Ghost button, square, icon 16, mandatory tooltip.
- Tooltip: `tooltip` bg, `Sm` text, `R_LG`, padding 6×8, delay 500 ms in / 0 out, shortcut as
  `Kbd` (`Xs` mono on `tag`, `R_SM`, padding 1×4). Interactive overlays always render above
  tooltips.
- Keys: tooltips also show on keyboard focus after 500 ms.

### 4.3 Form controls

- `TextField` (gpui-kit `Input`): height 36 (forms) / 28 (inline filters), `input` bg,
  `input_border`, hover `input_border_hover`, focus `input_border_focus` + `input` bg; error
  state: `danger` border + `Sm` message in `danger_surface_fg`. `secret` fields have a reveal
  eye toggle.
- `TextArea` (gpui-kit `Textarea`, auto-grow 1–12 lines).
- `Select` trigger = input styling + `chevron-down`; popup = `Menu` (§4.4) with check marks.
- `Switch` 28×16, `Checkbox` 16 (`R_SM`), `Radio` 16 circle; `Segmented` (gpui-kit
  `tab_bar_segmented`) height 28 for 2–4 choices.
- `ThinkingLevel` control: discrete level picker rendered as 1–6 bars (4×10 px each, gap 2;
  filled = `success`, empty = `border`), label `Sm` `fg_subtle` (off/minimal/low/medium/high/xhigh).
- Keys: Tab order follows reading order; Space toggles; arrows move in Segmented/Radio;
  Esc in a text field inside a dialog cancels the dialog only if the field is empty.

### 4.4 `Menu` (dropdown, context menu, select popup)

- Shell: `menu` bg, 1 px `popover_border`, `R_LG`, padding 4, E2, min width 192, max 320 (model
  menu 280). Rows: height 32, padding x 8, gap 8, `R_MD`, 2 px vertical gap between rows;
  [icon 16] label `Base` [trailing: kbd / chevron-right / check]. Group label row: `Sm`
  `fg_subtle`, height 28, optional badge (`Xs` on `tag`, `R_FULL`). Separator: 1 px `border`
  with 4 px margin.
- States: hover/keyboard-active `menu_hover`; selected = trailing `check` (no fill);
  disabled = `fg_subtlest`, still focusable for tooltips; submenu opens on hover after 120 ms
  or →.
- Placement: 4 px offset from trigger; aligned to the trigger's leading edge, flips to stay
  on screen; context menus at the pointer.
- Keys: ↑/↓ move (wrap), → open submenu, ← close submenu, Enter/Space activate, type-ahead
  jumps by label prefix, Esc closes (returns focus to the trigger).

### 4.5 `Tabs`

- `FrameTabBar` (side and bottom frames): height 36, tabs min 60 px, `Sm`/500, icon 14 + title,
  close `x` on hover/active, active tab `tab_active` bg + `fg`, inactive `fg_subtle`; overflow
  → `chevrons-right` menu; trailing `+` launcher menu (Review, Files, Output, Tasks, extension
  panels) and the frame close button.
- `Segmented` (above) for in-content switches (`Group | Project`, palette scopes, Board/List).
- OUS `tabs` → `Segmented` when ≤ 5 tabs and width allows, else underline tabs (2 px `fg`
  underline).
- Keys: ⌘1–9 select side tabs when the side frame has focus; Ctrl+Tab / Ctrl+Shift+Tab cycle;
  ⌘W closes the active closable tab.

### 4.6 `Badge`, `StatusCapsule`, `Dot`

- Badge: `Xs` 500, height 18, padding x 6, `R_SM` (inside rows) — tone from §2.2 surface/text
  columns.
- StatusCapsule (`R_FULL`, height 22, padding x 8, `Sm`): `Running` (spinner 12 + text,
  `warning` dot), `Waiting` (`waiting_surface`/`waiting_fg`, `message-circle-question`
  icon) — one treatment for approvals, questions and plan review; `Error` (`danger_surface`);
  `Compacting`/`Retrying` (`progress_surface`).
- Dot 6 px: running `warning` (pulses 1.2 s unless reduce motion), waiting `brand`, unread
  `success`, error `danger`, idle none. Color is always paired with a tooltip/text label.

### 4.7 Sidebar parts

- `NavItem`: height 32, padding x 8, `R_LG`, icon 16 `fg_subtle` + label `Base` + right `Kbd`
  `fg_subtlest`; hover `hover`; active route `selected`.
- `ViewSwitcher`: Segmented `Group` (by workspace) | `Recent` (flat by time), plus
  `⇅ sort` and `⊟ collapse all` icon buttons.
- `WorkspaceGroup` header: height 28, `folder` icon (`categorical` color by stable hash of the
  cwd), name `Sm`/500, running count badge, hover reveals `⊕` new session in this cwd and `⋯`
  (Reveal in file manager, Copy path, Remove from sidebar).
- `SessionRow`: height 32, indent 24, title `Base` truncated (marquee on hover after 800 ms
  when truncated), right: relative time `Xs` `fg_subtlest` or a `StatusCapsule` when
  running/waiting; leading 6 px `Dot`. Pinned rows show `pin` on hover. Active session
  `selected`. Context menu: Rename, Pin/Unpin, Fork, Open in new pane, Copy session id, Reveal
  file, Delete.
- `SidebarFooter`: height 40: `settings` icon button, update badge (`arrow-down-circle` +
  "Update" in `progress_surface` when `unipi.updater` reports one), current provider/model
  short label, Unicrab glyph (16 px, §4.28) showing the aggregate attention state.
- Keys (sidebar focused): ↑/↓ move between rows, ←/→ collapse/expand groups, Enter opens,
  ⌘Enter opens in a new pane, F2 renames, Delete asks to delete, `/` focuses the filter
  (sidebar filter = palette `#` scope inline).

### 4.8 `WorkspaceHeader`

Height 44, padding x 12, inside the conversation frame, bottom border `border`.
Left→right: [sidebar re-open button when collapsed] session title (`Base`/500, click =
rename inline) · `ProjectChip` (`folder` + cwd basename, `Sm`, `R_LG` ghost; menu: Open in
file manager, Copy path, Open terminal here, New session here) · `BranchChip` (`git-branch` +
branch, from `unipi.footer` git topic; menu read-only list) · `⋯` (Copy session id, Copy file
path, Export HTML, Fork, Tree view, Compact now). Right: `StatusCapsule` (running/waiting),
Unicrab glyph when this session needs you, `panel-bottom` toggle, `panel-right` toggle.
Header is a drag region on Linux/Windows when it is the top-most frame edge.

### 4.9 Timeline rows (`components/timeline/`)

All rows sit in the 896 px reading column; vertical rhythm 12 px between rows of a turn, 24 px
between turns; every row has a stable id (entry id or tool call id) and its own cached layout.

| Row | Anatomy | States / behaviour |
|---|---|---|
| `UserBubble` | right-aligned, max 576, `surface` bg, 1 px `border`, `R_XL` with top-right `R_SM`, padding 12×16, `Base`; attachments as 48 px thumbnails / file chips above text; mention tokens rendered as node chips | hover: `copy`, `pencil` (edit & resend = fork), `rotate-ccw` (rewind files to here) icon buttons under the bubble; steer/follow-up badges (`Xs`) for queued-then-sent messages |
| `AssistantBlock` | full width, markdown `Base` (§2.4), no bubble; code blocks `surface` bg `R_LG` with header row (language `Sm`, `copy`) | streaming: tail-only incremental markdown + caret; selection enabled; hover: copy turn |
| `ThinkingBlock` | one line `brain` 14 + "Thinking" / "Thought for 12s" `Sm` `fg_subtle` + chevron; expanded body `thinking` color, left 2 px `border` rail | collapsed by default (`hideThinkingBlock` setting = hidden entirely); streaming shows the running shimmer on the label |
| `ToolRow` (collapsed) | single line, height 24: status icon 14 (spinner / `check` `fg_subtlest` / `x` `danger`) · verb `Base` `fg_subtle` (Read, Edited, Ran, Searched, Listed, Fetched…) · subject (file icon + basename `fg` + dir `fg_subtlest`, or command mono) · trailing meta `Sm` mono `fg_subtlest` (`+8 −3`, `3.4s`, `14 matches`) · chevron on hover | click/Enter toggles; consecutive read-only tools (read/grep/find/ls) of one turn group into "Explored 6 files ›" when > 3 |
| `ToolRow` (expanded) | detail block under the line, left-indented 22, `surface` bg `R_LG`, max height 320 then scroll | edit/write → `InlineDiff`; bash → `BashTail`; others → OUS from renderer or args/result `code` |
| `InlineDiff` | lightweight unified diff, first 40 lines, line bg `diff_*_bg`, 2 px gutter bar `diff_added/removed`, mono `code_font_size`, hunk header `fg_subtlest` | footer link "View diff" → side pane Review tab; streaming tool input shows path + live line count (≤ 1 update/s) before the call executes |
| `BashTail` | last 5 lines of output (ANSI colors), mono, exit status chip (`check`/`x code 1`) | "Open output" → bottom frame Output tab; running = live tail |
| `AgentRow` | one line: `bot` (or `users` for sidekick) in `agent_node_fg` · agent name `Base`/500 · primary text (task) · status capsule/elapsed | click/Enter opens the child session read-only tab in the side pane (mobile: right drawer); static until the child id is known (ZCode `agent-tool-call-renderer.md`) |
| `SummaryCard` | compaction / branch summary: `card`, `R_XL`, header `archive` 14 + "Context compacted · 41k → 6k" `Sm`, collapsible markdown body | collapsed by default |
| `ProvenanceCard` | `card` with 3 px left rail `harness`, header `source` label `Xs` 500 `harness` + title, OUS body | used for every `details.oinoProvenance` and unipi custom messages (memory recall, dream report, notify) |
| `CustomRow` | OUS from `render` (cached by `(id, density, expanded, width, hash)`), else markdown of `content` | density follows Settings → Tool rows (simple/regular/advanced) |
| `ErrorBanner` | `danger_surface` card `R_XL`, `circle-alert`, message, actions (Retry, Copy details, Switch model) | provider errors, overflow-recovery notices |
| `TurnFooter` | after the last assistant row of a turn: "3 files changed +24 −9" `Sm` + per-file chips, actions Review (side pane), Undo (rewind files of this turn) | only when the turn changed files |
| `RunStateLine` | in the dock (not the list): spinner + working message (`setWorkingMessage`, default "Working…") + elapsed `Sm` mono + `Esc to stop` hint | hidden when idle |

Keys (timeline focused): ↑/↓ move a focus cursor across rows (focus ring on the row), Enter
toggles expand, `c` copies the row, `o` opens detail in side pane, Home/End jump, ⌘↑/⌘↓ previous /
next user query, `PageUp/PageDown` scroll. While the composer is focused, ⌘↑/⌘↓ still jump queries.

### 4.10 `QueryRail`

Left of the timeline (inside the frame, 16 px wide). One 12×2 px bar per visible user query
(spaced evenly, not by height), color `fg_subtlest`, current turn `fg`. Hover/focus: tooltip
(user query excerpt 2 lines + assistant excerpt 2 lines, `Sm`, max width 320). Click jumps
to the exact row. Virtualised (only bars in view ± overscan mounted). Hidden when < 2 queries
or frame < 600 px.

### 4.11 `Composer` (`components/composer/`)

```text
┌ shell: input bg, 1px input_border, R_2XL, padding 12 (E0) ──────────────────────────────┐
│ context header (optional): [⧉ json-lite ▾] [⎇ v1-alpha ▾] · frame-top segments        │
│ editor: "Ask Oino anything, @ files, / commands, $ skills"  (1–12 lines, then scroll)    │
│   tokens: [▦ parser.rs] [⚡/skill:tdd] [img 48px]                                         │
│ toolbar: [+] [🛡 Ask before changes ▾]                [▢ Oino Large ▾] [🧠 Medium ▾] (◔) [↑] │
└──────────────────────────────────────────────────────────────────────────────────────────┘
 meta row (24, Sm): frame-bottom segments · status chips              footer strip segments
```

- Context header: shown on the welcome screen always; in a running session only when frame
  top segments exist. Project and branch chips are `Sm` ghost buttons.
- Editor: gpui-kit `Textarea` with atomic tokens (`insertEditorToken`, `@file` picks, pasted
  images, long paste > 2 000 chars → text attachment token). Token chip: height 22, `R_MD`,
  node colors (§2.1 `*_node`), icon 12 + label `Sm`, `x` on hover. Placeholder `fg_subtlest`.
- Toolbar left: `+` attach menu (Files…, Image from clipboard, Paste as attachment); **mode
  chip** = workflow permission mode from `unipi.workflow` (`Ask before changes` /
  `Plan` / `Auto` / `Full access`, icons `hand` / `list-checks` / `sparkles` / `shield-off`),
  menu lists modes (invokes the extension's command); hidden when unipi.workflow is absent.
- Toolbar right: **model trigger** (`box` icon only when narrow; label = model name, no
  provider prefix), **thinking trigger** (`brain` + level label; menu with levels and the
  bars); **context ring** 18 px (arc = context %; `fg_subtle`, ≥ 80 % `warning`, ≥ 95 %
  `danger`; tooltip "41k / 200k tokens · Compact now"); **send** (28 px `R_LG` `primary`,
  `arrow-up`; disabled `secondary`) which becomes **stop** (`square`) while running and the
  editor is empty.
- Compression when narrow (< 520 px shell): model label → icon; thinking label → bars only;
  mode chip → icon; context ring stays; send stays.
- `CompletionMenu` (`@`, `/`, `$`, `#`): `Menu` shell anchored above the caret, width 360,
  max 8 rows visible; sections Files / Commands / Skills / Prompts / Sessions / Extension
  completions (from `get_completions`), each row: icon, label, description `Sm` `fg_subtle`,
  source tag `Xs`. Tab/Enter accept, ↑/↓ move, Esc closes.
- Keys: Enter send; Shift+Enter newline; while running Enter = steer, Alt+Enter = follow-up
  (pi); ↑ in an empty editor = previous prompt (history per session) unless an extension
  consumes `emptyArrowDown`/↓ (work tray); ⌘. or Esc (editor empty) = abort; ⌘L focus
  composer; ⌘M opens the model menu; ⌘⇧M thinking menu; drafts persist per session.

### 4.12 `ApprovalCard` (docked)

```text
┌ card R_XL, E2, header bg ─────────────────────────────────────────────── unipi.workflow ┐
│ 🛡 Run shell command?                                                   ⏱ 0:42 (timeout) │
│ bash · cargo test parser                                                                 │
│ ┌ detail (OUS: code/diff), max 240 then scroll ──────────────────────────────────────┐  │
│ │ $ cargo test parser                                                                  │  │
│ └──────────────────────────────────────────────────────────────────────────────────────┘  │
│ [ Tell the agent what to do instead (optional) …                                       ] │
│ [1 Allow once]  [2 Always allow in project]          [3 Reject]  [4 Always reject]      │
└──────────────────────────────────────────────────────────────────────────────────────────┘
```

- Docked above the composer, full composer width; the composer stays visible but its send is
  disabled until the card resolves (typing allowed; draft kept).
- Buttons by option kind: `allow_once` Primary, `allow_always` Outline, `reject_once` Outline
  with `danger_surface_fg` text, `reject_always` Ghost danger. Order = protocol order; number
  keys 1–9 shown as `Kbd` prefixes.
- Dangerous commands (rm -rf, git clean -f, force push, mkfs, dd, robocopy /MIR… — list in
  `components/approval/danger.rs`) show a `warning_surface` strip "Destructive command" and
  move the default focus to Reject.
- Timeout: countdown chip `Sm` mono; last 10 s fill the card header with `waiting_surface`
  draining bar.
- Multi-frontend: on `extension_ui_dismiss{reason:"answered"}` the card collapses with a 120 ms
  fade and a toast "Answered on another device".
- Keys: 1–9 choose; Enter = focused option (default first `allow_once`, or Reject when
  dangerous); Tab cycles feedback ↔ buttons; Esc = cancel (resolves `cancelled`, protocol
  treats it as reject); ⌘Enter with feedback text = reject with feedback.

### 4.13 `QuestionCard` (form / ask_user, docked)

- Header: `message-circle-question` + "Oino has N questions" + step chips (one per question,
  label = `header` ≤ 16 chars, done ✓, current `selected`).
- Body per question: question `Base`/500; options as rows (height 36, `R_LG`, `hover`,
  number `Kbd` 1–9, label + description `Sm` `fg_subtle`; single = radio, multi = checkbox);
  "Other" row expands a TextField; attachments row (`paperclip`) when allowed; option
  `action: end_turn|new_session` shows a trailing tag.
- Footer: Back (Ghost), Skip (Ghost, only when not required), Next / Submit (Primary).
- Keys (ZCode `elicitation-keyboard-behavior.md`): 1–9 toggle/choose; Enter in "Other" →
  next question, never submits on the last one; ⌘Enter submits; Esc → previous question, on
  the first question cancels; IME composition Enter never advances.
- Notifies via OS when unfocused (`notifyOnAsk`).

### 4.14 `PlanReviewCard` (workflow plan approval `custom`)

Docked card with header `list-checks` "Plan ready for review", markdown body (max 50 % of the
frame height, scroll), "Open in side pane" link (Plan tab), feedback TextField, actions from
the dialog (`Approve` Primary, `Approve & auto` Outline, `Keep planning` Ghost). Keys: Enter in
feedback submits the primary action, Esc cancels (ZCode ExitPlanMode semantics).

### 4.15 `DockDialog` (select / confirm / input / editor / multiSelect)

Small docked card (same shell as §4.12, width = composer, max height 50 % of the frame):
title `Base`/500, optional message; `select`/`multiSelect` → option rows with numbers and
descriptions (`optionDetails`), filter field when > 8 options; `input` → TextField (secret
supported, prefill); `editor` → TextArea 8 lines with `language` highlight and "Open in
external editor"; `confirm` → Yes (Primary) / No (Outline). Keys: 1–9, ↑/↓, Enter, Space
(multi toggles), Esc cancel; timeout chip as §4.12.

### 4.16 `QueuePanel`

Above the composer, collapsible strip "2 queued" (`list-ordered`) → rows (height 28): kind tag
(`Steer` `warning_surface` / `Follow-up` `idle_task_surface`), text 1 line, actions on hover:
`arrow-up-to-line` send now (= steer), `pencil` edit (moves the text back into an empty
composer; refused with a toast if the composer has a draft), `x` remove, drag handle to
reorder. Keys: when focused, ↑/↓, Enter edit, ⌘Enter send now, Delete remove, Alt+↑/↓ reorder.

### 4.17 `DockStrip`

Generic strip above the composer for `setWidget aboveEditor` (OUS or lines), unipi hints
(dim one-liner with `lightbulb`, ‹ › to cycle = `alt+h`/`alt+shift+h`), quota/overflow
banners, and the fusion wake widget. Height auto (max 96, then "Show more"), `surface` bg,
`R_LG`, `Sm` text, close `x` only when the producer allows clearing. Stack order (top→bottom):
banners, widgets by extension load order, hints, queue, interaction card, run-state line.

### 4.18 `MetaRow`

24 px row under the composer shell, `Sm`, `fg_subtle`. Left: frame-bottom segments then
`setStatus` chips; right: `setFooter` segments (unipi stats strip: ctx %, ↑in ↓out tokens,
cost, tps, model). Segments render text with tone color and optional icon; when the row is too
narrow, the lowest `priority` segments drop first (ui-protocol §4.4), across left and right
together; a trailing `⋯` reveals dropped segments in a popover. Click on a segment with an
associated command (`segment.command`, see §10 R2) runs it.

### 4.19 `StatusPanel` (floating, ZCode conversation status panel)

```text
expanded (≥1280, w 300, R_2XL, E2, popover bg, top-right of timeline, 12 px inset)
┌ Status ──────────────────────────────────── [⤡] ┐
│ Goal ▾  Ship trailing commas  ⏸  12m · 41k tok  │  ← long-horizon goal / ralph / swarm
│ Progress ▾ 2/3                                   │  ← plan steps (workflow), todo items
│   ✓ Locate token loop                            │
│   ◌ Patch handling                               │
│ Run ▾  2 tasks · 1 agent                         │  ← background tasks + subagents + sidekick
│   ▸ cargo test --watch   4m12s   [logs][kill]    │
│   ▸ bot explorer: map crate deps  38s  [open]    │
│ Git ▾  v1-alpha · 3 changed                      │
└──────────────────────────────────────────────────┘
mini (capsule, R_FULL, h 28): [◎ 2/3] [▶ 2] [⎇ 3]  → click expands
```

Section header rule (ZCode): title, chevron, then state right after the chevron (not pushed
right). Sections appear only when they have data. Data: `unipi:long-horizon:state`,
`unipi:workflow:status` (plan), `unipi:bg:tasks`, `_oino/agents`, `unipi:fusion:status`,
`footer` git topic — consumed through sticky topics forwarded to the frontend (§10 R3).
Mode setting: Auto (default) / Always expanded / Always mini. Keys: ⌘⇧S toggle expand; inside,
↑/↓ rows, Enter acts (open/expand), Delete kills a background task (confirm).

### 4.20 Side pane (`components/side_pane/`)

Frame with `FrameTabBar` (§4.5). Built-in tabs: **Review** (session file changes → DiffViewer),
**Files** (workspace tree, read-only code viewer), **Plan** (plan markdown), **Agent: <name>**
(read-only child session timeline), **Tree** (session tree / fork navigator), plus one tab per
`sidePane` panel (title from the panel, icon from extension id: kanboard `kanban`, app-bridge
`smartphone`, others `blocks`). Panel toolbar row (height 32) shows the panel's `keys` as
Ghost buttons with `Kbd`; the keys are bound while the tab is focused. Closing a panel tab
sends `$close`. Empty state: launcher grid of available tabs.

### 4.21 `DiffViewer`

Virtualised unified diff (gpui-kit `diff` component restyled): per-file card header (`R_XL`
card, git status letter in `git_*`, path mono `Sm` with dir `fg_subtlest`, `+n −m`, actions:
Open file, Copy path, Revert file), hunk separators (`Sm` `fg_subtlest` "@@ … @@" with expand
arrows when full context is available), 2 px gutter bars (`diff_added/removed`), word-level
highlight (`diff_*` at alpha 0x33), old/new line numbers `fg_subtlest`. Toggle unified/split
(split only ≥ 900 px). Keys: n/p next/previous hunk, ]/[ next/previous file, `u` toggle
unified/split, ⌘C copies selection without +/- markers.

### 4.22 `LogView`

Monospace virtual list (OUS `log`, bash output, background task logs): ANSI SGR colors from
`ansi`, `code_font_size`, line numbers optional, tail-follow toggle (auto when scrolled to
bottom), search (⌘F within the view), wrap toggle, copy all. Capped at 5 000 lines in OUS,
50 000 for local logs (ring buffer).

### 4.23 `Toast`

Bottom-right of the conversation frame (desktop) stacked upward, max 3 visible; width 360;
`toast` bg, `R_2XL`, E3, padding 12×16; icon by type (`info` / `triangle-alert` /
`circle-x` / `check`), message `Base` (2 lines, expand on hover), one optional action (Undo,
Open, Retry), close `x`. Auto-dismiss 4 s info / 6 s warning / sticky error; hover pauses.
`osNotify` uses the OS when the window is unfocused, else a toast with the title bold.

### 4.24 `Dialog` and `Sheet`

- Dialog: centred (palette: top 80 px), `popover` bg, `R_2XL`, E2, scrim; header title
  `Lg`/500 + `x` close, body padding 20, footer right-aligned
  actions in ZCode order on every OS: **primary first, then Cancel** (`[Save] [Cancel]`), a
  destructive extra action alone on the left. Sizes per
  §2.7. Enter = primary when focus is not in a multiline field; Esc closes (if
  `closeOnEscape`).
- Sheet (mobile; desktop narrow side pane): §6.2.

### 4.25 `CommandPalette` (⌘K, also ⌘⇧P)

```text
┌ dialog 640×min(560, 70vh), top 80, R_2XL ────────────────────────────────┐
│ ⌕ [ Search commands, sessions, files…                         ]  esc    │
│ [All] [Actions] [Sessions] [Files]                                      │
│ history: (trailing commas) (kanboard) (+3)                               │
│ RECENT CHANGES                                                           │
│   ▦ src/parser.rs                                     +8 −3              │
│ RECENT SESSIONS                                                          │
│   ● Make the JSON parser…                    json-lite · now             │
│ SUGGESTED  ✎ New session ⌘N · ⧉ Open folder · ⚙ Settings ⌘,             │
│ CHAT  ⤓ Compact · ⑂ Fork · ⌥ Tree ⌘⇧T · ✕ Abort ⌘.                       │
│ PANELS  ◧ Toggle sidebar ⌘B · ◨ Side pane ⌘J · ▭ Bottom ⌘` · Board      │
│ EXTENSIONS  /info · /btw · Toggle plan mode ⌥P …            │
│ APP  Theme · Check for updates · Docs · Remote access                    │
└──────────────────────────────────────────────────────────────────────────┘
```

Prefixes `>` actions, `#` sessions, `@` files (ZCode). Empty query shows the sections above
(3 rows each in All, full lists in scoped tabs). Query: groups in fixed order Actions,
Sessions, Files, each 3 rows + "Show N more". Rows: icon, label with matched chars in `fg`
500 (others `fg_subtle`), description, right `Kbd`. Extension slash commands and shortcuts
(ui-protocol §8) are Actions. History per workspace, 20 entries. Keys: ↑/↓, Enter run,
⌘Enter run in new pane (sessions), Tab cycles scope tabs, ⌘1–4 select scope, Backspace on an
empty query clears the prefix, Esc closes.

### 4.26 `ModelMenu`

`Menu` 280 wide anchored to the model trigger (centre aligned). With one provider: flat model
list; with several: the provider in use is pinned at the top and rendered directly (group
label = current provider "Oino Router" + auth badge `API`/`OAuth`, then its models), separator,
then the other providers as submenu rows (model count trailing, `chevron-right`) that open that
provider's model list with an "All providers" row to come back — a level inside the same menu
(drill-down), because one GPUI popover holds the focus; separator, then "Manage models…"
(Settings → Models). Row: model name `Base`, capability badges `Xs` (`Vision`, `1M`), trailing
`check`. A separate "Pinned" group is not drawn: pi's `enabledModels` / `--models` scope is
already applied to `get_available_models`, so the list *is* the pinned set on desktop
([zcode-details.md](zcode-details.md) §6). Models disabled for the running session show `lock` + tooltip.
Keys: §4.4 plus ⌘M toggles, type-ahead filters, →/Enter opens a provider, ←/Esc goes back one
level, Esc at the top level closes.

### 4.27 OUS renderer (`components/ous/`)

| `t` | Component |
|---|---|
| `text` | StyledText with spans (tone, bold, italic, dim = `fg_subtle`, underline, code = inline code, link = `Link`) |
| `markdown` | TextView markdown (§2.4) |
| `code` | code block (§4.9 AssistantBlock) |
| `diff` | InlineDiff (≤ 40 lines) or DiffViewer when in a side pane/overlay |
| `image` | img, `R_LG`, max width, click → lightbox |
| `log` | LogView |
| `kv` | two-column grid, keys `fg_subtle` `Sm`, values `Base` |
| `table` | gpui-kit `table`, header `Sm`/500 `fg_subtle`, row 32, selectable rows `hover`/`selected` |
| `list` | rows 32 (label, detail `Sm`, badge, tone dot); `selectable` sends the item id |
| `tree` | gpui-kit `tree`, 24 px rows, chevrons |
| `tabs` | §4.5 |
| `split` | resizable, 4 px handle |
| `stack` | flex, `gap` × 4 px |
| `section` | title `Sm`/500 `fg_subtle`, chevron when collapsible |
| `card` | `card` `R_XL` (nested → step down), rail 3 px tone, `source` label `Xs` |
| `badge` | Badge |
| `meter` | 6 px bar `R_FULL`, track `border`, fill tone; label left, value right `Sm` mono |
| `gauge` | 32 px ring |
| `sparkline` / `area` / `barChart` | gpui-kit chart with `categorical` colors, axes `fg_subtlest` `Xs` |
| `spinner` | shared ticker spinner 14 |
| `divider` | 1 px `border`, optional label `Xs` centred |
| `keyHints` | row of `Kbd` + label `Sm` `fg_subtle` |
| `input`, `toggle`, `select`, `button` | §4.1, §4.3 |
| `empty` | EmptyState compact |
| `responsive` (O, §10 R1) | first variant whose `minWidth` (columns ≈ width / 8 px) fits; else `fallback` |
| `effect: shimmer` on `card` (O) | one 1.6 s highlight sweep, then static (reduce motion: none) |
| `effect: drain` + `durationMs` on `meter` (O) | value animates 1 → 0 over `durationMs` (auto-close timers) |

### 4.28 `EmptyState` and the Unicrab

- EmptyState: centred icon tile (40, `surface`, `R_XL`) or the Unicrab (64), title `Lg`/500,
  one line `Base` `fg_subtle`, up to two actions. `compact` variant: icon 20, `Base` title,
  inline.
- Unicrab (Oino's mascot from unipi; `design/brand.rs` draws the 24×20 cell sprite as quads,
  palette from unipi-app DESIGN §1.1 — shell `#F03018`, claw `#F07818`, amber `#F0A818`,
  gold `#F0C030`, cream `#F0F0A8`, face `#181818`): states and priority from unipi-app §1.5.1 —
  needs-you (hop, claws wave), done (happy bounce), working (claws clack), offline (dim 45 %,
  eyes closed), sleep (after 5 min idle, Z's), idle (breathe + blink). Sizes 16 (glyph:
  static silhouette in the state color — `brand` needs-you, `warning` working, `success` done,
  `fg_subtlest` idle), 64 (empty states), 128 (splash). Animated only at ≥ 64 px; ticker at
  8 fps; reduce motion holds frame 0.

### 4.29 Settings parts

`SettingsPage` (full main-area page, §5.7): left nav 220 (NavItem), content max 720, page
title `Display`/600 (24/32 in narrow, 30/36 ≥ 1024), section titles `Lg`/500, `SettingRow`
(label `Base`/500, description `Sm` `fg_subtle`, control right, min height 48, divider
`border`), forms in transparent `R_XL` bordered groups padding 16 (ZCode
`settings-form-layout.md`). Hub field types (ui-protocol §6.6): boolean → Switch, enum →
Select (Segmented if ≤ 3), multiselect → checkbox list, string → TextField/TextArea,
number → TextField + unit + stepper, secret → masked field + "Stored" badge, order → draggable
list with ↑/↓ buttons, model → ModelMenu trigger, action → Outline button, page → chevron row
opening a sub-page with breadcrumb. Scope toggle (Segmented `Global | Project`) in the page
header; "Reset to default" per field (↺ on hover when changed); "Modified" dot on changed
fields; search field filters across namespaces. Keys: ⌘F focuses search, ↑/↓ moves between
rows, Space/Enter edits.

### 4.30 `KanbanBoard` (native view of the kanboard panel)

Lanes (Backlog hidden by default · Todo · In Progress · Blocked · In Review · Done ·
Cancelled), lane width 280, gap 12, header (status glyph colored by lane: Todo `fg_subtle`,
In Progress `warning`, Blocked `danger`, In Review `ansi[12]`, Done `success`, Cancelled
`fg_subtlest`; name `Sm`/500; count badge; `⋯` lane menu). Card: `card` `R_XL`, padding 12,
id mono `Xs` `fg_subtlest`, title `Base` (2 lines), up to 2 label badges (+N), agent chip
(`bot` + elapsed) when claimed, priority tag, dependency tag (ready/waiting/locked), blocked
reason `Sm` `danger_surface_fg`. Toolbar: project Select, Filter, Display, search, New task.
Rendered from the panel's OUS (`split` of `list` columns) by a dedicated recogniser
(`components/kanban.rs` detects `panelId == "unipi.kanboard/board"` and maps list items
with `badge`/`tone` to cards); actions go back as `extension_ui_action`. Keys: h/j/k/l or
arrows move the card focus, Enter opens (task detail as an overlay panel from the
extension), m opens the move menu (allowed moves), c new task, / filter, Esc clears focus.

### 4.31 `FindBar`

⌘F in the conversation: 36 px floating bar top-right of the timeline (`popover`, `R_XL`, E2):
field, `3/12`, ↑/↓, `Aa` case toggle, `x`. Matches `find`, current `find_active`. Enter/⇧Enter
next/previous, Esc closes and returns focus.

---

## 5. Desktop screens

Window default 1400×900, min 720×480. Every screen keeps the sidebar (§3.1) unless stated.

### 5.1 D1 Welcome / new session (empty session)

```text
┌ sidebar ┐┌ conversation frame ─────────────────────────────────────────────────────────┐
│         ││ header: "New session" · [⧉ json-lite ▾]                     [▭] [◨]          │
│         ││                                                                               │
│         ││        ┌ setHeader hero (unipi info-screen splash, responsive card) ┐         │
│         ││        │  [pixel Unicrab 128]   Oino 1.0.0-alpha.1 · 23 extensions  │         │
│         ││        │                        model Oino Large · ctx 200k          │         │
│         ││        └─ drain meter (auto-close) ──────────────────────────────────┘         │
│         ││                    "Good afternoon! Leave the rest to me."  (Display)         │
│         ││     ┌ composer (max 720) with context header [⧉ json-lite ▾] [⎇ v1-alpha ▾] ┐ │
│         ││     │ Ask Oino anything, @ files, / commands, $ skills                       │ │
│         ││     │ [+] [🛡 Ask before changes ▾]               [Oino Large ▾] [🧠 Med ▾] [↑]│ │
│         ││     └────────────────────────────────────────────────────────────────────────┘ │
│         ││     hint strip (unipi.core hints): 💡 Press alt+h for the next tip       ‹ ›   │
│         ││     ┌ suggestion card ┐ ┌ suggestion card ┐ ┌ suggestion card ┐  (R_XL)        │
│         ││     │ ⟳ Resume "Fix…" │ │ ▦ Board: 3 ready│ │ ✎ Prompt templ. │               │
└─────────┘└───────────────────────────────────────────────────────────────────────────────┘
```

- Greeting by local hour (ZCode `chat-empty-time-greeting.md` English strings: 05–08 "Morning,
  ready when you are", 09–11 "Morning, how can I help?", 12–13 "Noon break?", 14–17 "Good
  afternoon! Leave the rest to me.", 18–22 "Evening, nice work today", 23–04 "It's late —
  remember to rest."), `Display`/600, `fg`.
- With no `setHeader`: the Oino mark (64) at 40 % `fg_subtlest` above the greeting instead of
  the hero.
- Suggestion cards (3, `card` `R_XL`, padding 16, icon 16 + title `Base`/500 + 2-line `Sm`
  description): sources in order — most recent unfinished session in this cwd ("Resume"),
  kanboard ready tasks ("Board: N ready" → opens D8), prompt templates (first two by name).
  Hidden when no source has data.
- Project chip menu: recent cwds, "Open folder…"; branch chip shows the git branch (read-only).

### 5.2 D2 Session workspace (running)

```text
┌ sidebar ┐┌ conversation frame ──────────────────────────────────────────┐┌ side frame ─────────┐
│ ✎ New   ││ Make the JSON parser… · [⧉ json-lite] [⎇ v1-alpha] ⋯  (●Running) [▭][◨] ││ [Review][Agent: x]+ │
│ ⌕ Search││▏                                       ┌ Status ───────[⤡]┐   ││ 1 file changed      │
│ ▦ Board ││▏               ┌──────────────────────┐│ Progress 2/3      │   ││ ┌ src/parser.rs  M ┐│
│ ⧉ Ext.  ││▏               │ Make the JSON parser…││  ✓ Locate loop    │   ││ │ @@ -41,9 +41,12  ││
│         ││▏               └──────────────────────┘│  ◌ Patch          │   ││ │ - if self.peek…  ││
│ ▾ json… ││▏ › Thought for 4s                      │ Run 1 task        │   ││ │ + match self.pe… ││
│  ● Make ││▏ I'll make the parser tolerant…        └───────────────────┘   ││ └──────────────────┘│
│    Kanb ││▏ ✓ Read  ▦ parser.rs  src/                                      ││                     │
│    Fix  ││▏ ✓ Edited ▦ parser.rs  src/                       +8 −3 ›       ││                     │
│         ││▏ ◌ Ran   $ cargo test parser                       3.4s ›       ││                     │
│         ││▏   │ test parser::arrays ... ok                                  ││                     │
│         ││▏   │ test parser::objects ... ok                                 ││                     │
│         ││  ◌ Running tests…  0:12   esc to stop                           ││                     │
│         ││ ┌ composer ───────────────────────────────────────────────────┐ ││                     │
│         ││ │ Steer Oino…                                                  │ ││                     │
│         ││ │ [+][🛡 Ask ▾]                      [Oino Large ▾][🧠 Med ▾](◔)[■]│ ││                     │
│ ⚙ ⬇ ◷   ││ └──────────────────────────────────────────────────────────────┘ ││                     │
│         ││  ⎇ v1-alpha · plan 3 steps · workflow build     ctx 4% ↑18k ↓950 $0.07 │└─────────────────────┘
└─────────┘└──────────────────────────────────────────────────────────────┘
```

- Timeline per §4.9; user bubble right; tool rows one line; status panel floating (≥ 1280).
- "Jump to latest" pill (`arrow-down`, `R_FULL`, E2) appears bottom-centre above the dock when
  scrolled up > 1 screen; new content while scrolled up shows a dot on it.
- Multi-pane: ⌘\ splits the conversation frame (≤ 4 panes, each with its own header and dock;
  focused pane header title in `fg`, others `fg_subtle`). Closing a pane never stops a session.

### 5.3 D3 Blocking interaction states

- Approval: §4.12 card docked; sidebar row and header show `Waiting`; Unicrab needs-you;
  OS notification when unfocused ("Oino needs approval: Run shell command?").
- Question: §4.13. Plan review: §4.14 (Plan tab opens automatically in the side pane when
  ≥ 1280 px).
- Several pending interactions (different extensions): one card at a time, header shows
  "1 of 3"; others queue in arrival order.

### 5.4 D4 Side pane contents

Review (DiffViewer of session changes grouped per turn; "All turns" Select), Files (tree +
read-only code viewer with syntax highlight, `@`-mention button "Add to prompt"), Plan
(markdown, check states), Agent tabs (read-only child timelines with their own header "bot
explorer · running · 38s"; "Stop agent" Danger Ghost), Tree (session tree: nodes with labels,
current path in `fg`, other branches `fg_subtle`; Enter = switch, f = fork, l = label),
extension `sidePane` panels (kanboard as D8 when width ≥ 600, app-bridge control centre).

### 5.5 D5 Command palette — §4.25. D6 Model menu — §4.26.

### 5.6 D7 Bottom frame

Tabs: **Output** (LogView of `!`/`!!` shell commands and bash tool calls of the focused session,
one sub-tab per command with exit chip) and **Tasks** (background tasks of `unipi.background-tasks`:
table — name, command mono, status capsule, elapsed, actions logs/kill; selecting a row shows
its LogView), plus `bottomPane` panels. An interactive PTY terminal is **not** part of v1
(ZCode has one; Oino diverges, §8).

### 5.7 D8 Settings (full main-area page, ⌘,)

```text
┌ sidebar (settings nav replaces session list) ┐┌ settings frame ───────────────────────────────┐
│ ← Back to sessions                            ││ Appearance                       (Display)   │
│ General                                       ││ ┌ group R_XL ─────────────────────────────┐  │
│ Appearance                                    ││ │ Theme            [System|Light|Dark]     │  │
│ Models & Providers                            ││ │ Interface size   [ 14 px ▾ ]             │  │
│ Sessions                                      ││ │ Code font size   [ 13 px ▾ ]             │  │
│ Permissions                                   ││ │ Reduce motion    [Follow OS ▾]           │  │
│ Keyboard shortcuts                            ││ │ Tool rows        [Simple|Regular|Adv.]   │  │
│ Extensions                                    ││ │ Thinking blocks  [Collapsed ▾]           │  │
│ EXTENSION SETTINGS  ⌕                          ││ └──────────────────────────────────────────┘  │
│   Footer · Memory · Notify · Workflow · …     ││                                               │
│ Remote access                                 ││                                               │
│ About & updates                               ││                                               │
└───────────────────────────────────────────────┘└───────────────────────────────────────────────┘
```

Pages: General (default cwd, startup behaviour, language — English only in v1, telemetry: none),
Appearance (above + status panel mode + sidebar density), Models & Providers (provider list with
auth state, Log in / Log out / API key dialog, default model, scoped models order, thinking
default), Sessions (session dir, keep-warm, auto-rename, compaction setting from core),
Permissions (unipi.workflow saved rules table: tool, pattern, decision, scope, delete),
Keyboard shortcuts (searchable table of app + extension shortcuts, rebind by recording,
conflicts in `warning`), Extensions (D12), Extension settings hub (one nav entry per namespace,
fields per §4.29), Remote access (serve on/off, QR pairing card 200 px, tokens list with
revoke), About & updates (version, `unipi.updater` state, changelog link, licenses).

### 5.8 D9 Board (sidebar "Board", kanboard)

Main area becomes a board frame (replacing the conversation frame; side frame still
available): header "Board · <project ▾>" + toolbar (Filter, Display, search, `+ New task`),
lanes per §4.30, horizontal scroll with shift+wheel. The view is the `unipi.kanboard/board`
panel promoted from `sidePane` to the main area (frontend-local promotion: opening the panel
from the Board nav item sends the kanboard open command and renders it here). Without
`unipi.kanboard` the nav item is hidden.

### 5.9 D10 Overlay panels (info-screen, updater, MCP, skill editor, hints browser)

Modal dialog `large` (760×min(680, 85vh)), header = panel title + panel key toolbar
(`r Refresh` …) + `x`; body = OUS. Info-screen (`unipi.info-screen/dashboard`) tabs render as
a left vertical tab list (≥ 760) — Overview, Usage, Modules, Tools, Settings, extension
groups — with charts (`barChart`, `sparkline`, `area`) at full width.

### 5.10 D11 Editor-replace panels (work tray, btw)

The composer shell is replaced by the panel in the same `R_2XL` shell, height up to 50 % of
the frame: header row (title `Base`/500, panel keys as Ghost buttons with `Kbd`, `Esc to
return`), OUS body. Work tray (`unipi.core/tray`) = `tabs` (Subagents, Background, Dream,
App bridge, …) rendered as Segmented + list rows with actions. btw (`unipi.btw/panel`) =
"Side chat" — streaming markdown answer above an inline input (ZCode side chat), history
within the panel.

### 5.11 D12 Extensions page

List (table): icon, name, id mono `Sm`, version, kind (provider / unipi / user), state (Loaded
`success`, Failed `danger` with error tooltip, Disabled), permissions summary; row actions
Enable/Disable, Settings (jump to hub), Readme (`/readme` overlay). Header actions:
Install from file…, Open extensions folder.

### 5.12 D13 Onboarding / first run

Full-window page (no sidebar): Oino mark + "Welcome to Oino" (`Display`), steps as a
vertical stepper card (max 560): 1 Preparing extensions (progress bar while the wasm cache
compiles, "≈ 7 s once"), 2 Connect a model (provider list rows → OAuth/API key dialog), 3
Choose a folder (recent folders / Open folder…), 4 Start (opens D1). Skippable after step 2.

### 5.13 D14 Startup splash

First frame ≤ 150 ms: window `window` bg + centred Oino mark (64) only; replaced by D1/D2 as
soon as the session list loads. The Unicrab splash is the info-screen `setHeader` (D1), never
a blocking screen.

---

## 6. Mobile screens (Android + iOS, `oino_ui::mobile`)

Same tokens, `ui_font_size` 15, editable text 16, targets ≥ 44, rows 56. Phones in portrait are
the design target; landscape and tablets (≥ 720 dp wide) reuse the desktop layout with the
sidebar as a drawer. Safe areas pad every screen; the keyboard inset lifts the dock. Android
Back / iOS edge swipe pop in the order: menu → sheet → drawer → page.

### 6.1 M1 Sessions (home)

```text
┌ safe area ─────────────────────────────┐
│ ⟨crab⟩                     ⌕   ⚙       │  app bar 52 (glyph = aggregate Unicrab state)
│ Sessions                    (Display)  │
│ ┌ Needs you (2) ─────────────────────┐ │  waiting_surface card, R_XL; hidden when empty
│ │ ◆ Make the JSON parser…  Approve › │ │
│ │ ◆ Port watchdog caps     Answer  › │ │
│ └────────────────────────────────────┘ │
│ JSON-LITE                         ⊕    │  section label Sm 500 fg_subtle
│ ● Make the JSON parser…   Running  ›   │  row 56: dot, title Base, sub Sm (last msg), time/capsule
│   Kanboard TODO summary   4m       ›   │
│ UNIPI                             ⊕    │
│   Port watchdog caps      1d       ›   │
│                                        │
│                      [ ✎ New session ] │  FAB-style Primary pill, bottom-right, 48 high
└────────────────────────────────────────┘
```

Swipe a row left: Pin, Delete (confirm). Long press: Rename, Fork, Copy id. Pull to refresh.

### 6.2 M2 Conversation

```text
┌────────────────────────────────────────┐
│ ‹  Make the JSON parser…   ●  ◨  ⋯     │  app bar: back, title (Base 500) + sub "json-lite · Oino Large" (Sm), status dot, side drawer, actions
│ [◎ 2/3] [▶ 1]                          │  status capsule row (mini status panel), tap → status sheet
│            ┌────────────────────────┐  │
│            │ Make the JSON parser…  │  │  user bubble max 85 %
│            └────────────────────────┘  │
│ › Thought for 4s                       │
│ I'll make the parser tolerant…         │
│ ✓ Edited ▦ parser.rs        +8 −3 ›    │  tool rows 32 high (touch), tap expands inline
│ ◌ Ran $ cargo test parser   3.4s ›     │
│  ◌ Running tests… 0:12                 │  run-state line
│ ┌ composer R_2XL ────────────────────┐ │
│ │ Steer Oino…                        │ │
│ │ [+] [🛡]          [Large ▾][🧠] (◔)[■]│ │  compressed toolbar (§4.11)
│ └────────────────────────────────────┘ │
│ ctx 4% · $0.07                    ⋯    │  meta row: top-2 priority segments + overflow
└────────────────────────────────────────┘
```

- Timeline gutters 12; query rail hidden; find via actions sheet.
- Tool row expanded detail max height 50 % screen; "View diff" / "Open output" open full-screen
  pages (M5).
- AgentRow tap opens the child timeline in the right drawer (ZCode `/remote`).

### 6.3 M3 Interactions

Approval, question and plan cards dock above the composer exactly as desktop (§4.12–4.14) with
44 pt buttons stacked vertically when > 2 options; when the card exceeds 50 % of the visible
height it becomes a full-height bottom sheet (drag handle, not swipe-dismissible while
pending). Small dialogs (§4.15) are bottom sheets with option rows 56 high. Number keys are not
shown. A pending interaction triggers a local notification when the app is backgrounded
(platform notifications package).

### 6.4 M4 Right drawer (side pane)

88 % width from the right, `frame` bg, `R_2XL` leading corners, scrim on the rest; Segmented
tab strip (scrolls horizontally): Review, Plan, Agent tabs, `sidePane` panels (kanboard,
app-bridge). Swipe right or Back closes.

### 6.5 M5 Full-screen pages

Diff (DiffViewer, unified only), Output (LogView with tail follow), overlay panels (info-screen,
updater, MCP, skills: OUS in a page with a detail header 52: back, title, panel keys as icon
buttons/⋯), `fullscreen` panels, child agent timeline when opened from the status sheet.

### 6.6 M6 Bottom sheets

`frame` bg, `R_2XL` top corners, drag handle 36×4 `border_hover`, title `Lg`/500, max 90 %:
Model (§4.26 rows 56, providers as sections, thinking level as a 6-segment control at the
bottom), Actions (Compact, Fork, Tree, New session, Abort, extension commands and shortcuts
grouped like the palette, Status chips list), Status (expanded status panel content), Work
tray / btw (`editorReplace` panels render in the composer area as on desktop, keyboard-aware),
Settings sub-pickers.

### 6.7 M7 Search

Full-screen page from ⌕: field at top (auto-focus), scope Segmented (All / Actions / Sessions /
Files), same sections as the palette (§4.25), rows 56.

### 6.8 M8 Settings

iOS/Android-style stacked list pages using §4.29 SettingRow at 56 high: Appearance, Models &
Providers, Sessions, Permissions, Extensions, Extension settings (namespace list → fields),
Remote (M10), About. Scope toggle at the top of each namespace page.

### 6.9 M9 Board

unipi-app DESIGN §3.9 phone pattern in ZCode styling: project title as picker, lanes are
full-width pages in a horizontal snap pager with a lane pill strip above (glyph, name, count;
active pill `selected`), cards per §4.30 full width, New task Primary pill bottom-right,
long-press card → anchored menu (open, allowed moves, priority). Pull to refresh.

### 6.10 M10 Remote attach (gpui-app §4)

Hosts list (paired desktops: name, host, last seen, status dot), "Pair a desktop" page: an
instruction card ("Scan the QR shown in Oino Desktop → Settings → Remote access with your camera
app") — the system camera opens the `oino://` deep link into the app (gpui-mobile `deeplink`) —
plus a paste field for the URL. No in-app camera scanner in v1.
Embedded (local core) mode keeps today's behaviour; the Hosts list shows "This phone" first.

### 6.11 Mobile notifications and toasts

Toasts at the top under the app bar, full width − 24, `R_2XL`, swipe up to dismiss. `osNotify`
→ platform notification when backgrounded. No floating Unicrab overlay (unipi-app §1.5.1
floating crab is dropped): the crab glyph lives in the M1 app bar and its state drives the app
icon badge count (iOS) / notification dot (Android).

---

## 7. unipi surface → component / screen mapping

Every surface emitted by the 23 bundled unipi components (and the not-yet-ported `utility`)
has exactly one primary rendering. "D"/"M" = desktop/mobile screens (§5, §6); "C" = component
(§4).

| Package | Surface (protocol) | Desktop | Mobile |
|---|---|---|---|
| core | hint line (`setWidget aboveEditor`, `unipi.core/hints`) | C4.17 DockStrip hint (‹ › cycling, `alt+h`) | same, one line, tap cycles |
| core | hints browser (`openPanel overlay` `unipi.core/hints`) | D10 overlay dialog | M5 page |
| core | header / crab (`setHeader` `unipi.core/header`, `responsive`) | D1 welcome hero; slim card at timeline top in non-empty sessions | same |
| core | work tray (`openPanel editorReplace` `unipi.core/tray`, `↓` on empty composer) | D11 editor-replace shell with Segmented tabs | M6 composer-area panel |
| footer | glance frame (`setFrame` top/bottom) | C4.11 context header (top) + C4.18 MetaRow left (bottom) | top segments in context header; bottom behind `⋯` |
| footer | stats strip (`setFooter` segments: ctx, tokens, cost, tps, model) | C4.18 MetaRow right, priority dropping | MetaRow, top-2 segments |
| footer | process line (`setWidget aboveEditor` from `unipi:bg:tasks`) | C4.17 DockStrip (one line) — suppressed when the StatusPanel is expanded (same data in Run) | DockStrip |
| info-screen | dashboard (`openPanel overlay` `unipi.info-screen/dashboard`, tabs/charts) | D10 large dialog with vertical tabs | M5 page, Segmented tabs |
| info-screen | Unicrab splash (`setHeader`, `effect: shimmer`, drain meter auto-close) | D1 hero (C4.27 effects, C4.28 Unicrab 128) | D1-equivalent hero above greeting |
| workflow | tool gate (`approval`) | C4.12 ApprovalCard | M3 docked card / sheet |
| workflow | plan review (`custom`: markdown + 3 actions) | C4.14 PlanReviewCard + Plan side tab | M3 sheet |
| workflow | mode / status (`setStatus`, shortcuts `alt+p`) | composer mode chip (C4.11) + MetaRow chip | mode icon in composer |
| workflow | saved rules | D8 Permissions page | M8 |
| ask-user | `form` (questions, other, attachments, end_turn/new_session) | C4.13 QuestionCard | M3 |
| ask-user | tool renderer, `osNotify` on ask | ToolRow OUS "Asked 3 questions" + answers; OS notification | same + local notification |
| long-horizon | goal / ralph / swarm state (topic `unipi:long-horizon:state`) | C4.19 StatusPanel Goal (pause/resume, elapsed, tokens) | status capsule row + Status sheet |
| long-horizon | graph visualisation (`openPanel overlay` `unipi.long-horizon/visualize`) | D10 large dialog (tree/table OUS) | M5 |
| long-horizon | select dialogs, notifications | C4.15 DockDialog, C4.23 Toast | sheets, toasts |
| memory | recall / save cards (custom entries `CARD_TYPE`, `SAVE_CARD_TYPE`, `BADGE_ENTRY`, `NOTICE_ENTRY`) | ProvenanceCard (rail `harness`, source "memory"), badge entries as inline Badge rows | same |
| memory | confirm/select (forget, consolidate) | C4.15 DockDialog | sheet |
| dream | report card (`REPORT_CARD` custom entry with proposals) | ProvenanceCard with proposal list + Accept/Reject buttons (OUS buttons) | same, buttons stacked |
| dream | tray tab, running state | work tray tab; StatusPanel Run row "Dream" (`moon`) | same |
| subagents | `run_subagent` / `read_subagent` tools | C4.9 AgentRow → side pane Agent tab (read-only child timeline) | AgentRow → right drawer |
| subagents | `subagent-step` entries, `subagent-completion` message | child timeline rows; completion = compact ProvenanceCard | same |
| subagents | `/agents` profile manager (`custom`), `/subagents` tray tab | D10 dialog (medium); work tray tab | M5; tray |
| fusion | `sidekick` tool, `sidekick-step`, `sidekick-completion` | AgentRow (`users` icon, "Sidekick") → Agent tab | drawer |
| fusion | wake widget (`setWidget aboveEditor` spinner + text) | DockStrip with spinner | same |
| fusion | `/fusion` picker (`custom`) | D10 dialog medium (lead/sidekick focus, filter, thinking) | M6 sheet |
| fusion | status topic | StatusPanel Run row "Sidekick" | Status sheet |
| btw | side chat (`openPanel editorReplace` `unipi.btw/panel`, `updatePanel` ≤ 20/s) | D11 "Side chat" shell | composer-area panel |
| background-tasks | `bg_run` / `bg_logs` / `bg_kill` tools | ToolRow with OUS (log tail) | same |
| background-tasks | task list (topic `unipi:bg:tasks`), tray tab, `ctrl+alt+c` clear | StatusPanel Run + D7 bottom Tasks tab + tray tab | Status sheet; Output page |
| background-tasks | completion notification (custom message `NOTIFICATION_TYPE`) | ProvenanceCard + Toast | card + local notification |
| notify | `notify` / `osNotify` | C4.23 Toast / OS notification | toast / platform notification |
| notify | setup wizards (`form`, `select`) | C4.13 QuestionCard / C4.15 | M3 |
| kanboard | board (`openPanel sidePane` `unipi.kanboard/board`) | D9 Board view (main area) or side tab (C4.30) | M9 board pager |
| kanboard | `unipi:kanboard-show` custom message, progress entries | ProvenanceCard / CustomRow (task card mini) | same |
| kanboard | status topic (claims, autowork) | sidebar Board nav badge (ready count), StatusPanel Run row when an agent claims | M1 "Needs you" when a task waits for review |
| mcp | servers panel (`openPanel overlay` `unipi.mcp/servers`), confirm | D10 dialog; DockDialog | M5; sheet |
| mcp | tools | ToolRow (`plug` icon, `<server>__<tool>` shown as server chip + tool) | same |
| skill-registry | skill settings editor (`openPanel overlay` `unipi.skill-registry/editor`), `/skills` → `openSettings` | D10 dialog; D8 hub page "Skills" | M5; M8 |
| command-enchantment | completions + package colors (`c1…c12`) | C4.11 CompletionMenu with colored source tags | same |
| compactor | dialogs (`custom` ×3, `select`) | D10 dialog medium / DockDialog | sheet |
| compactor | compaction result | C4.9 SummaryCard | same |
| updater | update / changelog / readme (`openPanel overlay` `unipi.updater/*`) | D10 dialog; sidebar footer "Update" badge opens it | M5; M8 About badge |
| app-bridge | control centre (`openPanel sidePane` `unipi.app-bridge/control`), tray tab | side pane tab (C4.20) | M4 drawer tab |
| input-shortcuts | editor intents, `setShortcuts` | composer `editor_event`; palette Actions + D8 shortcuts page | Actions sheet |
| watchdog | stuck / cap warnings (`notify`, `setStatus`) | Toast + MetaRow chip (`warning`); hard stop → ErrorBanner | toast |
| web-api | confirm / select (domains, downloads) | C4.15 DockDialog | sheet |
| web-api | tools (`web_fetch`, search) | ToolRow verb "Fetched" / "Searched web", `globe` | same |
| all | settings hub namespaces (`openSettings`, schema §6.6) | D8 Extension settings | M8 |
| all | provenance (`details.oinoProvenance`) | ProvenanceCard | same |
| all | Unicrab aggregate state (needs-you > done > working > offline > sleep > idle) | sidebar footer glyph + header glyph; window title prefix "● " when needs-you | M1 app bar glyph + badge |
| utility (port pending) | render styles → tool render hints + `ui.density`; `/answer` reply panel; image tools | Settings Tool rows density (C4.9); reply panel = D11 shell | same |

---

## 8. ZCode parity checklist

Status column: **Match** = same behaviour; **Adapt** = same intent, Oino-specific form;
**Diverge** = intentionally not built (reason given).

| # | Item | ZCode behaviour | Oino target | Status |
|---|---|---|---|---|
| 1 | Theme modes | System / Zai Light / Zai Dark, default dark | System / Oino Light / Oino Dark with Zai values (§2.1); default System→dark | Match |
| 2 | Type scale | `text-ui-*` offsets from `--ui-font-size` 14 | `TextRole` offsets from `ui_font_size` 14 (§2.4) | Match |
| 3 | Independent code/diff font size | separate settings | `code_font_size` | Match |
| 4 | Spacing / radius nesting | 4 px grid; xl→lg→md→sm; 2xl exceptions | same rules and exceptions (§2.5–2.6) | Match |
| 5 | Icons | Lucide, stroke 1.5 | Lucide via gpui-kit assets, stroke 1.5 | Match |
| 6 | Motion | 100–300 ms, low drama, hidden-window pause | 120/180/280 ms, reduce motion, ticker stops when idle | Match |
| 7 | Workspace frames | sidebar + independent conversation / terminal / side frames, 4 px gaps, xl radius | flush unipi columns (A2 L1) | Adapt |
| 8 | Sidebar width | 264 default/min, ≤ 50 % | same | Match |
| 9 | Sidebar nav order | New Task → Search → Automations → Plugins → view switcher | New session → Search → Board → Automations → Extensions → view switcher | Adapt (Automations after Board) |
| 10 | Workspace groups, counts, pinned, unread dots | yes | §4.7 | Match |
| 11 | Sidebar footer | settings, plan/usage badge, update | settings, update badge, model label, Unicrab glyph | Adapt |
| 12 | Header | title, project, branch switcher, more menu, terminal/side toggles | §4.8 (branch read-only) | Adapt (no branch switching) |
| 13 | Welcome / new task | logo backdrop, time greeting, composer with project/branch header, suggested cards | §5.1 with Unicrab hero (setHeader) and Oino-sourced suggestions | Adapt |
| 14 | Conversation timeline | virtualised rows, row-level updates, incremental markdown | §4.9, §2.11 | Match |
| 15 | Reading width | `max-w-4xl` ≥ 864 container | 896 max | Match |
| 16 | User bubble | right, surface, rounded-xl + tr-xs, max-w-xl | same | Match |
| 17 | Query navigator rail | left bars, hover tooltip query + answer excerpt, full catalog | §4.10 | Match |
| 18 | Find in conversation | in-page find with highlight tokens | §4.31 | Match |
| 19 | Tool rows | one-line summary, expand on demand | §4.9 (+ grouping "Explored N files") | Match |
| 20 | Inline edit diff + View diff | lightweight diff, open in side pane | InlineDiff 40 lines + Review tab | Match |
| 21 | Streaming tool input preview | path + line count sampled ≤ 1/s | same from `toolcall_delta` | Match |
| 22 | Bash output | details, background bash parity | BashTail + Output tab | Match |
| 23 | Subagent rows | single-line, opens read-only child tab | AgentRow → Agent tab | Match |
| 24 | Turn file change summary + undo | per-turn summary, file-level undo without truncating chat | TurnFooter Review / Undo (rewind files) | Match |
| 25 | Rewind any message | rewind button on user messages | `rotate-ccw` on UserBubble (pi fork + file rewind) | Match |
| 26 | Thinking blocks | collapsed, level control | ThinkingBlock collapsed + thinking trigger | Match |
| 27 | Composer shell | rounded-2xl, context header, mode, model, thinking, send | §4.11 | Match |
| 28 | Model menu | provider groups + badge, submenus, check, Manage models, ⌘M opens (no cycling) | §4.26 | Match |
| 29 | Context usage | context ring with refresh action | context ring + Compact now | Match |
| 30 | Mentions | `@` files, `/` commands/agents, `$` skills, `#` conversations; colored nodes | same prefixes, node tokens §2.1 | Match |
| 31 | Attachments, long paste | chips, paste-as-attachment | tokens via `insertEditorToken` | Match |
| 32 | Prompt history, drafts | per task | per session | Match |
| 33 | Queue panel | edit (recall to composer), delete, send now, reorder | §4.16 | Match |
| 34 | Steer / follow-up | guide while running | Enter = steer, Alt+Enter = follow-up (pi) | Adapt |
| 35 | Permission dialog | allow once / allow project / deny + feedback, docked | ApprovalCard with ACP kinds incl. reject always | Match (+1 option) |
| 36 | Dangerous command guard | recognises destructive commands | danger strip + default focus Reject | Match |
| 37 | Ask-user / elicitation | stepped questions, Enter/Esc rules | QuestionCard, same keys | Match |
| 38 | Plan mode approval | ExitPlanMode card, plan side pane | PlanReviewCard + Plan tab | Match |
| 39 | Waiting badge | one green confirmation treatment | `waiting_surface/fg` everywhere | Match |
| 40 | Status panel | floating Git/Goal/Progress/Run, mini capsule < 1280 | §4.19 | Match |
| 41 | Side pane tabs | Git review, browser, code viewer, terminal, plan, subagents, plugins | Review, Files, Plan, Agent, Tree, Browser, extension panels | Match |
| 42 | Embedded browser | browser tab / browser-use | gpui-webview Browser tab (A1.4); browser-use agent control later | Match |
| 43 | Interactive terminal | xterm PTY bottom frame, tabs | Terminal tab with alacritty_terminal PTYs + Output/Tasks (A1.5) | Match |
| 44 | Command center | ⌘K, scopes All/Actions/Tasks/Files, prefixes, history, sections | §4.25 | Match |
| 45 | Platform primary modifier | Cmd on Apple, Ctrl elsewhere, never both | same | Match |
| 46 | Settings | full page, nav, forms in bordered groups | §5.7 + extension hub | Match |
| 47 | Toasts | bottom stack, rounded-2xl, blur | bottom-right, `R_2XL`, 95 % opaque (no blur in GPUI) | Adapt |
| 48 | Split panes | ≤ 4 panes, close ≠ stop | none: one visible session, others keep running (A2.18) | Diverge |
| 49 | Automations / scheduled tasks | main view | Native Automations page (cron view: job list, state colour+icon+word, run history + session links, create/edit with schedule presets) next to Board | Ported |
| 50 | Bots (Feishu), coding-plan purchase, rewards, share links, whiteboard, office previews, CUA | product features | not built (out of Oino scope) | Diverge |
| 51 | Mobile remote | single column, drawers instead of side tabs, QR pairing | §6 (drawers, sheets, M10 pairing) | Match |
| 52 | Performance bar | 4 panes streaming without dropped frames | §2.11 (≥ 100 fps, 120 fps single) | Match |
| 53 | i18n | zh/en | English only in v1; layouts tolerate +40 % text length | Adapt |
| 54 | Window chrome | per-OS controls, Linux 16 px shell radius | client decorations, per-OS insets (§3.1) | Match |

Summary (after A2): 54 items — **43 Match, 9 Adapt, 2 Diverge** (row 50: ZCode-only product
features such as bots, coding-plan purchase, share links and CUA; row 48: split panes, owner
decision A2).

---

## 9. Current gaps (apps code at `c314fc8`)

Observed in `apps/oino-ui/src/**` and the `--demo` screenshots (`design/ref/oino-current-*.png`).

### 9.1 Shared / desktop

| # | Gap | Where today |
|---|---|---|
| G1 | No token table: gpui-kit default theme colors, ad-hoc `hsla` (harness, scrim, palette) | `views/theme.rs` |
| G2 | System fonts; no bundled Inter / JetBrains Mono; no type roles (uses `text_xs`/`text_sm`) | all views |
| G3 | One bordered window, no independent frames, window-wide status bar | `views/workspace.rs` `render`, `render_status_bar` |
| G4 | Sidebar: filter field inline, nav lacks Board/Extensions placement and view switcher, no group counts / context menus / pinned / update badge / crab; Extensions+Settings in a footer row | `workspace.rs` `render_sidebar`, `session_row` |
| G5 | Header shows raw cwd path, no project/branch chips, no `⋯` menu, no side/bottom toggles beyond one icon | `workspace.rs` `render_header` |
| G6 | Dialogs and approvals are **modal** over the transcript; no docked cards, no number keys, no timeout chip, no danger guard | `views/dialog.rs`, `session_pane.rs` `sync_dialog` |
| G7 | Tool rows render expanded diffs/outputs inline by default; no verb/subject layout, file icons, grouping, TurnFooter | `session_pane.rs` `render_tool`, `render_bash` |
| G8 | User bubble/assistant styling off-spec (no border/tr-xs radius); thinking row plain | `session_pane.rs` `render_user`, `render_assistant` |
| G9 | Composer: hint-text placeholder, dot-style thinking, no mode chip, no context ring, no context header, no compression, completions minimal | `session_pane.rs` `render_composer`, `render_model_control`, `render_completions` |
| G10 | No StatusPanel; plan state only as a frame segment | — |
| G11 | Side pane = one `SideTab` at a time (Diff/Output/Panel); no tab bar, Review aggregation, Agent/Tree/Files/Plan tabs | `session_pane.rs` `SideTab`, `render_side`; `workspace.rs` `render_side_pane` |
| G12 | Palette: flat list, no scopes/sections/history/prefix groups | `views/palette.rs`, `model/palette.rs` |
| G13 | Settings: 860×600 modal overlay, no full page, no hub | `workspace.rs` `render_settings`, `settings_nav` |
| G14 | OUS: `responsive`, `effect`, `durationMs` unsupported; `setFooter` segments rendered as OUS only | `model/ous.rs`, `model/ui_state.rs`, `views/ous.rs` |
| G15 | Query rail: no tooltip, no virtualisation, always 12 px | `session_pane.rs` `render_rail` |
| G16 | Empty session screen minimal; no greeting/hero/suggestions | `session_pane.rs` `render_empty` |
| G17 | No FindBar, no jump-to-latest pill | — |
| G18 | Toasts use gpui-kit defaults (top-right, default styling) | `workspace.rs` `push_notification` |
| G19 | No bottom frame (Output/Tasks) | — |
| G20 | No Unicrab, no Oino mark component | — |
| G21 | Kanboard panel rendered as generic OUS in the side pane | `views/ous.rs` |
| G22 | No frame-time overlay / perf script | — |
| G23 | Panel keys rendered as a hint row, not a toolbar; editor-replace panels not in the composer shell | `session_pane.rs` `render_overlay_panel`, `render` (editor_panel) |

### 9.2 Mobile

| # | Gap | Where today |
|---|---|---|
| MG1 | Sessions screen lacks Needs-you card, section labels, swipe actions, crab glyph; "New session" full-width button | `mobile/shell.rs` `render_sessions`, `session_row` |
| MG2 | Conversation app bar lacks sub-line and status capsule row; no right drawer (side sheet is full-screen) | `mobile/shell.rs` `app_bar`, `render_session` |
| MG3 | Interactions inherit desktop modal dialogs | via `SessionPane` |
| MG4 | Sheets: model/actions exist but not per §6.6 (handle, rows 56, thinking segments, status section) | `mobile/shell.rs` `sheet_frame`, `render_model_sheet`, `render_actions_sheet` |
| MG5 | No search page, no board pager, no full-screen pages for diff/output/panels with detail header | — |
| MG6 | Settings list is a single page (theme, toggles, extensions) | `mobile/shell.rs` `render_settings` |
| MG7 | Touch sizes: shared tool rows are 24 px | `SessionPane` rows |
| MG8 | No remote attach (M10); local embedded only | `apps/oino-mobile/src/app.rs` |
| MG9 | Notifications: `osNotify` not routed to platform notifications | `apps/oino-mobile/src/platform.rs` |

### 9.3 Guard test

`apps/oino-ui/src/design/tests.rs::no_literal_styles` scans `src/views/**`, `src/components/**`
and `src/mobile/**` for `hsla(`, `rgb(`, `rgba(` and `px(<number>` outside `design/`; allowed
exceptions are listed in the test with a reason. Runs in the `--no-default-features` test set
(reads files, no GPUI).

---

## 10. Work packages

Both packages commit on `v1-alpha` with their work id prefix, keep the apps gates green
(`cargo fmt`, `clippy -D warnings -W clippy::pedantic`, `cargo test` in `apps/`, build with
`-j 2`, `TMPDIR=$PWD/target/tmp`), and refresh screenshots in `apps/oino-desktop/screenshots/`
(desktop) or `apps/oino-mobile/dist/screenshots/` (mobile) for every visual change.
`design-mobile` starts after `design-desktop` phases P1–P4 have landed (it consumes the
tokens and components).

### 10.1 `design-desktop` — paths: `apps/oino-ui/` (except `src/mobile/`), `apps/oino-desktop/`

| Phase | Files | Change |
|---|---|---|
| P1 tokens & theme | **new** `apps/oino-ui/src/design/{mod,tokens,type_scale,theme,icons,syntax,ticker,perf,tests}.rs`; **delete** `src/views/theme.rs` (callers move to `design::theme`); **new** `apps/oino-ui/assets/fonts/{Inter-Variable.ttf,JetBrainsMono-Regular.ttf,JetBrainsMono-Medium.ttf,OFL-Inter.txt,OFL-JetBrainsMono.txt}`; `src/lib.rs` (module map), `Cargo.toml` (fonts embedded with `include_bytes!`, no new dependency) | Palette LIGHT/DARK exactly as §2.1; tone map §2.2; gpui-kit theme derivation §2.3 registered as "Oino Light/Dark"; System mode follows window appearance; type roles §2.4 with `ui_font_size`, `code_font_size`, `reduce_motion`, `status_panel_mode` added to `AppSettings` (`src/host.rs`) and persisted by `apps/oino-desktop/src/prefs.rs`; `design::Density { Desktop, Touch }` global (default Desktop); icon `AssetSource` with the §2.9 vocabulary; ticker global; contrast + no-literal-style tests (§9.3) |
| P2 shell | `src/views/workspace.rs` (split into) **new** `src/views/shell.rs` (frames, resize handles, breakpoints §3.2), `src/components/{sidebar,header,meta_row,toast,button,controls,menu,tabs,badge}.rs`; `apps/oino-desktop/src/main.rs` (font registration, client decorations, min size 720×480, per-OS insets) | §3.1 frames; sidebar §4.7 (nav order, view switcher, groups, rows, context menus, footer with update badge + crab glyph); header §4.8; remove `render_status_bar` (→ MetaRow); toasts §4.23 bottom-right |
| P3 timeline & composer | `src/views/session_pane.rs` (keep as orchestration; move rendering out) → **new** `src/components/timeline/{mod,user,assistant,thinking,tool,inline_diff,bash_tail,agent,cards,turn_footer,run_state}.rs`, `src/components/composer/{mod,toolbar,tokens,completion,context_ring}.rs`, `src/components/{query_rail,queue,dock}.rs`; `src/model/tools.rs` (verbs, subjects, grouping "Explored N files"), `src/model/composer.rs` (mode chip state, token model, compression), `src/model/transcript.rs` (turn file summaries) | §4.9–§4.11, §4.16–§4.17; collapsed one-line tool rows by default; jump-to-latest pill; multi-pane split ⌘\ |
| P4 docked interactions | `src/views/dialog.rs` → **replace** with `src/components/interaction/{mod,approval,question,plan,dock_dialog,danger}.rs`; `src/model/dialog.rs` (interaction queue "1 of N", timeout countdown, dismiss reasons) | §4.12–§4.15; modal only for `custom` dialogs that are not plan review and for overlay panels |
| P5 panes | **new** `src/components/{side_pane,diff_viewer,log_view,status_panel}.rs`, **new** `src/model/status.rs` (StatusPanel model from topics/agents/background tasks); `session_pane.rs` `SideTab` → `SidePaneState { tabs: Vec<SideTabKind>, active }` | §4.19–§4.22, D4, D7 bottom frame (Output/Tasks); Agent tabs read-only child timelines when `_oino/agents` exists (R5), else hidden |
| P6 navigation | `src/views/palette.rs` → `src/components/palette.rs`; `src/model/palette.rs` (scopes, sections, prefixes, history per workspace, 3-row groups); **new** `src/components/{model_menu,find}.rs` | §4.25, §4.26, §4.31; keymap in `src/views/mod.rs` `init` per §4 Keys tables |
| P7 pages | **new** `src/views/{welcome,settings_page,extensions_page,onboarding,board}.rs`, `src/components/settings.rs`; `workspace.rs` `render_settings` removed | D1, D8 (hub fields wired to `get_settings_schema` etc. when R4 lands; until then the hub nav lists namespaces read-only), D9, D12, D13 |
| P8 OUS & brand | `src/model/ous.rs` (`Responsive{variants}`, `effect`, `durationMs`), `src/model/ui_state.rs` (`setFooter` segments, segment `command`), `src/views/ous.rs` → `src/components/ous/{mod,kanban}.rs`; **new** `src/design/brand.rs`, `src/components/{empty,unicrab}.rs` | §4.27 table complete; kanban recogniser §4.30; Unicrab sprite (24×20 cells, data copied as numbers from `extensions/unipi/info-screen/src/splash.rs` crab grids, not from unipi-app assets) with the six states |
| P9 perf & proof | `apps/oino-desktop/src/demo.rs` (demo script emits: splash header, footer segments, status topics, a subagent row, a question form, plan review, kanboard panel, toasts), **new** `apps/oino-desktop/scripts/perf.sh`, `scripts/screenshot.sh` (shots: 01-welcome, 02-streaming, 03-approval-docked, 04-question, 05-palette, 06-settings, 07-board, 08-light-theme, 09-info-overlay), `apps/oino-desktop/assets/THIRD-PARTY-NOTICES` | §2.11 budgets measured and recorded in `docs/dev/v1/deviations/design-desktop.md` |

Acceptance (design-desktop): every row of §8 marked Match/Adapt is visible in the demo
screenshots or covered by a view test; G1–G23 closed; §2.11 p99 ≤ 10 ms with 4 streaming panes
on the Linux dev machine; guard test green; gpui-app.md §5 replaced by a pointer to this file
(request to its owner if not gpui-desktop).

### 10.2 `design-mobile` — paths: `apps/oino-mobile/`, `apps/oino-ui/src/mobile/`

| Phase | Files | Change |
|---|---|---|
| M-P1 density | `apps/oino-ui/src/mobile/mod.rs` (sets `design::Density::Touch` global; `ui_font_size` 15; editable 16) | components read density (provided by design-desktop P1 as `design::Density { Desktop, Touch }`): rows 56 / tool rows 32 / targets 44 |
| M-P2 screens | `apps/oino-ui/src/mobile/shell.rs` → split into `mobile/{shell,sessions,conversation,drawer,sheets,pages,search,settings}.rs` | M1 (Needs-you card, sections, swipe actions, crab glyph, New session pill), M2 (app bar with sub-line, status capsule row, compressed composer, meta row top-2), M4 right drawer, M5 full-screen pages with detail header, M6 sheets (handle, rows 56, thinking segments, Status section), M7 search, M8 stacked settings |
| M-P3 interactions | `mobile/conversation.rs` | docked cards from `components/interaction` with stacked buttons; > 50 % height → non-dismissible full sheet; Back = cancel only for small dialogs |
| M-P4 board & panels | **new** `mobile/board.rs` | M9 lane pager (snap, pill strip, long-press menu) over the kanban recogniser; overlay/sidePane/fullscreen placement mapping per §3.3 |
| M-P5 platform | `apps/oino-mobile/src/{platform.rs,app.rs,prefs.rs,android.rs,ios.rs}`, `android/` (deeplink intent filter `oino://`), `ios/` (URL scheme) | `osNotify` and pending-interaction local notifications (gpui-mobile `notifications`), app badge from Unicrab needs-you count, JetBrains Mono registration, theme follows system dark mode, hosts list + pairing page (M10) persisted in `mobile.json` |
| M-P6 proof | `apps/oino-mobile/scripts/build.sh` (`--screenshots`: `adb exec-out screencap` of M1, M2, M3, M6, M9 in light and dark), `docs/dev/v1/deviations/design-mobile.md` | screenshots from a physical Android device; iOS via `ios-remote.sh` when desmondgiam is reachable |

Acceptance (design-mobile): MG1–MG9 closed; every §7 row has its Mobile rendering in the
demo backend (`<agent>/mobile-demo`); 120 Hz scroll on a 120 Hz Android device with no dropped
frames over 10 s (GPU profiler bars), input-to-paint < 16 ms.

---

## 11. Requests for other work

| # | Owner | Request |
|---|---|---|
| R1 | core-types-session (`oino_protocol::ui`) + ui-protocol.md | Document and type OUS additions already emitted by unipi ports: `responsive{variants:[{minWidth, node}], fallback}`, `card.effect: shimmer`, `meter.effect: drain` + `durationMs` (`extensions/unipi/info-screen/src/splash.rs`, `extensions/unipi/core/src/hints/render.rs`) |
| R2 | core-types-session + unipi-ux (footer) | `setFooter` priority `segments` (`extensions/unipi/footer/src/strip.rs`) into ui-protocol §4.4; optional `Segment.command` (command run on click) |
| R3 | modes + core-runtime | Frontend topic feed: `_oino/topics{subscribe:[...]}` delivering sticky topics `unipi:long-horizon:state`, `unipi:workflow:status`, `unipi:bg:tasks`, `unipi:fusion:status`, `unipi:kanboard:status`, footer git info — needed by the StatusPanel and sidebar badges (§4.19) |
| R4 | modes | `_oino/*` frontend commands from STATUS §4 item 4 (`ui_action`, `get_completions`, `invoke_shortcut`, `editor_event`, settings schema/get/set, `service_call`, `login`) — panel buttons, completions, hub, onboarding depend on them |
| R5 | core-runtime | Nested agents + `_oino/agents` child session event stream (STATUS §4 item 1) for AgentRow → Agent tab |
| R6 | integration | Replace gpui-app.md §5 with a pointer to this document after design-desktop P7 lands |
