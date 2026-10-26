# design-desktop status

Work id `design-desktop` (docs/dev/v1/design.md §10.1). This file records the parity checklist,
gaps, perf numbers and requests. `docs/dev/v1/deviations/design-desktop.md` is owned by
integration and should be created from this file (request D1 below).

Screenshots: `docs/dev/v1/design/shots/desktop/*.png` (`scripts/screenshot.sh`, `--demo`,
1400×900, Xvfb + lavapipe; `16-narrow` at 860×760).

| Shot | Screen |
|---|---|
| 01-welcome | D1 welcome: splash hero (`setHeader` responsive card, drain meter), greeting, context header, hint strip, meta row, suggestion cards |
| 02-streaming | D2 running: query, thinking, AgentRow, streaming edit preview, running bash, status panel, run-state line |
| 03-approval-docked | D3 approval card docked above the composer, `Waiting` capsule, Unicrab needs-you, timeout chip |
| 04-question | D3 question card (stepped `form`, numbered options, Other) |
| 05-palette | D5 command center (scopes, sections) |
| 06-settings | D8 settings page (Appearance) |
| 07-board | D9 kanboard board in the main area |
| 08-light-theme | Oino Light, finished run with TurnFooter, provenance card |
| 09-info-overlay | D10 info-screen overlay with tabs and charts |
| 10-plan-review | D3 plan review card |
| 11-side-review | D4 side pane Review tab (DiffViewer), mini status capsule |
| 12-model-menu | D6 model menu |
| 13-settings-hub | D8 extension settings hub (schema fields, scope toggle, reset) |
| 14-extensions | D12 extensions page |
| 15-split-panes | ⌘\ four streaming panes |
| 16-narrow | < 1280 / < 864 breakpoints |

## ZCode parity checklist (design.md §8)

✅ done · 🟡 partial · ⛔ diverge (by design)

| # | Item | State | Notes |
|---|---|---|---|
| 1 | Theme modes | ✅ | System/Light/Dark, Zai values, follows `window.appearance()` |
| 2 | Type scale | ✅ | `TextRole` offsets from `ui_font_size` |
| 3 | Code font size | ✅ | `code_font_size` |
| 4 | Spacing / radius nesting | ✅ | token-only (guard test) |
| 5 | Icons | ✅ | Lucide vocabulary in `OinoAssets` |
| 6 | Motion | 🟡 | shared ticker stops when idle, reduce motion; enter/exit animations of menus not animated |
| 7 | Workspace frames | ✅ | independent frames, 4 px gaps with resize handles |
| 8 | Sidebar width | ✅ | 264 min, ≤ 50 %, draggable |
| 9 | Sidebar nav order | ✅ | New · Search · Board · Extensions · view switcher |
| 10 | Groups, counts, pinned, unread | ✅ | virtualised `uniform_list` |
| 11 | Sidebar footer | 🟡 | settings, update badge (from updater notify), model label; crab glyph moved to the top row |
| 12 | Header | ✅ | title rename, project/branch chips, ⋯ menu, capsule, frame toggles |
| 13 | Welcome | ✅ | |
| 14 | Timeline | ✅ | `list` virtualised, row-level remeasure, incremental markdown |
| 15 | Reading width | ✅ | 896 |
| 16 | User bubble | ✅ | |
| 17 | Query rail | ✅ | tooltip excerpts, sampled bars |
| 18 | Find | ✅ | ⌘F, case toggle, row highlight |
| 19 | Tool rows | ✅ | verb · subject · meta, "Explored N files" |
| 20 | Inline diff + View diff | ✅ | |
| 21 | Streaming tool input preview | ✅ | `−n +m` while preparing |
| 22 | Bash output | ✅ | BashTail with ANSI colors, Output tab |
| 23 | Subagent rows | 🟡 | AgentRow → Agent tab shows task + result; live child timeline needs `_oino/agents` (R5) |
| 24 | Turn summary + undo | 🟡 | TurnFooter Review ✅; Undo needs core file checkpoints (request) |
| 25 | Rewind message | 🟡 | edit & resend (fork) ✅; file rewind needs core checkpoints |
| 26 | Thinking blocks | ✅ | |
| 27 | Composer shell | ✅ | |
| 28 | Model menu | ✅ | provider groups, badges, check, Manage models |
| 29 | Context ring | ✅ | click = compact |
| 30 | Mentions | ✅ | `@ / $ #` via `_oino/get_completions`, colored token chips |
| 31 | Attachments | 🟡 | `insertEditorToken`, file picker; pasted images not converted |
| 32 | History, drafts | ✅ | per session |
| 33 | Queue panel | 🟡 | edit, clear; send-now/remove/reorder of single items need core queue commands |
| 34 | Steer / follow-up | ✅ | Enter / Alt+Enter |
| 35 | Permission dialog | ✅ | docked, 4 kinds, feedback, number keys |
| 36 | Dangerous command guard | ✅ | |
| 37 | Elicitation | ✅ | stepped QuestionCard, keys |
| 38 | Plan approval | ✅ | PlanReviewCard + Plan tab |
| 39 | Waiting badge | ✅ | |
| 40 | Status panel | 🟡 | Goal/Progress/Run/Git from session state; sticky-topic feed (R3) pending |
| 41 | Side pane tabs | ✅ | Review, Files, Plan, Agent, Tree, panels |
| 42 | Embedded browser | ⛔ | |
| 43 | PTY terminal | ⛔ | Output + Tasks tabs instead |
| 44 | Command center | ✅ | scopes, prefixes, sections, history |
| 45 | Primary modifier | ✅ | `secondary-*` bindings, `⌘` labels become `Ctrl+` off Apple |
| 46 | Settings | ✅ | page + hub |
| 47 | Toasts | ✅ | bottom-right, max 3, hover pauses |
| 48 | Split panes | ✅ | ≤ 4, close ≠ stop |
| 49 | Automations → Board | ✅ | |
| 50 | ZCode product features | ⛔ | |
| 51 | Mobile | — | design-mobile |
| 52 | Performance bar | 🟡 | see below |
| 53 | i18n | ✅ | English |
| 54 | Window chrome | 🟡 | min size, per-OS insets, macOS transparent title bar; Linux/Windows still use server-side decorations |

Unipi surfaces (design.md §7): every row renders through the OUS renderer, the docked
interactions, DockStrip, MetaRow, StatusPanel, side/bottom tabs or overlays; kanboard has the
native board; panel buttons, list selection, inputs, tabs and keys send `_oino/ui_action`;
shortcuts go through `_oino/invoke_shortcut` (palette Actions); editor intents
(`changed`/`cleared`/`emptyArrowDown`/`submit`) through `_oino/editor_event`; the settings hub
uses `_oino/get_settings_schema` + `get/set/unset_extension_setting`; Models & Providers uses
`_oino/auth_providers` / `login` / `logout`. `service_call` is exposed on the pane
(`SessionPane::frontend`) but no native widget needs it yet.

## Performance (design.md §2.11)

Measured with `scripts/perf.sh` (release, demo `stream` scene, Xvfb + lavapipe software
Vulkan on the 32-core dev machine, 25 s). The cost is UI-thread time from the root `render` to
the end of paint; GPU rasterisation is not included.

| Run | frames | p50 | p99 (median of 120-frame windows) | worst p99 | max | dropped (> 16.7 ms) |
|---|---|---|---|---|---|---|
| 1 pane streaming | 1440 | 2.0 ms | 3.1 ms | 3.7 ms | 3.8 ms | 0 |
| 4 panes streaming (before caching) | 1320 | 6.5 ms | 11.2 ms | 14.4 ms | 14.5 ms | 0 |
| 4 panes streaming (cached panes) | 1320 | 6.1–6.2 ms | 9.9–10.7 ms | 12.8–14.9 ms | 14.5–15.9 ms | 0 |

Speed work in this package: panes are `cached` views (a pane re-renders only when notified),
the event pump coalesces per frame and reuses its batch buffer, rows render from borrowed
transcript rows (no per-frame row clones), streaming text deltas skip the status-panel
rebuild, plan scanning is capped at 4 KiB, the sidebar, palette and timeline are virtualised,
the ticker stops when nothing animates, palette matching allocates nothing per item.

Open: the 4-pane p99 sits at the 10 ms budget under lavapipe; a real-GPU run is needed.
Input-to-paint, session switch and cold open were not measured.

## Requests

- D1 (integration): copy this file to `docs/dev/v1/deviations/design-desktop.md`; replace
  gpui-app.md §5 with a pointer to design.md (R6).
- core-runtime: file checkpoints for TurnFooter Undo and UserBubble rewind; single-item queue
  commands (remove / send now / reorder).
- modes (R3): `_oino/topics` sticky-topic feed for the StatusPanel; (R5) `_oino/agents` child
  event stream for live Agent tabs.
- design-mobile: `src/mobile/shell.rs` was adapted minimally to the new `design::theme` and
  `SidePane` APIs; it still uses literal styles (excepted in the guard test).
