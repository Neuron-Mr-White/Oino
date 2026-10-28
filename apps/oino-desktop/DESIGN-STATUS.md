# design-desktop status

Work id `design-desktop` (docs/dev/v1/design.md §10.1). This file records the parity checklist,
gaps, perf numbers and requests. `docs/dev/v1/deviations/design-desktop.md` is owned by
integration and should be created from this file (request D1 below).

Screenshots: `docs/dev/v1/design/shots/desktop/*.png` (`scripts/screenshot.sh`, `--demo`,
1400×900, Xvfb + lavapipe; `16-narrow` at 860×760). The driver scenarios add their own
folders under `docs/dev/v1/design/shots/desktop/<scenario>/` (`agents-live`, `status-panel`,
`turn-undo`, `queue-edit`, `image-attach`; [gui-driver.md](../../docs/dev/v1/gui-driver.md) §7b).

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
| 17-terminal | bottom Terminal tab (real PTY shell, ANSI colours) |
| 18-browser | side-pane Browser tab (gpui-webview, `file://` preview page) |
| 19-light-welcome | Oino Light welcome (derived unipi hues) |
| 20-light-approval | Oino Light docked approval |
| 21-find | ⌘F find bar with matches |
| 22-completion | composer `/` completion menu |
| 23-toasts | toast stack (info / warning / error) |
| 24-bottom-output | bottom frame Output tab |
| 25-side-files | side pane Files tab |
| 26-side-launcher | empty side-pane launcher (incl. Browser) |
| 27-session-menu | sidebar session context menu (Rename, Pin, Fork, Open in new pane, Copy id, Reveal, Delete…) |
| 28-delete-confirm | destructive-action confirmation dialog |
| 29-onboarding | D13 onboarding (forced with `OINO_DESKTOP_ONBOARDING=1`) |
| 30-720 | 720 × 600 minimum-ish window, mini status strip above the timeline |

## ZCode parity checklist (design.md §8)

✅ done · 🟡 partial · ⛔ diverge (by design)

| # | Item | State | Notes |
|---|---|---|---|
| 1 | Theme modes | ✅ | System/Light/Dark; A1.2 unipi warm OLED dark + derived light (shell primary, wordmark semantics), follows `window.appearance()` |
| 2 | Type scale | ✅ | `TextRole` offsets from `ui_font_size` |
| 3 | Code font size | ✅ | `code_font_size` |
| 4 | Spacing / radius nesting | ✅ | token-only (guard test) |
| 5 | Icons | ✅ | Lucide vocabulary in `OinoAssets` |
| 6 | Motion | 🟡 | menus, dialogs, toasts fade + 4 px rise (180 ms, `design::motion`); needs-you pulse; ticker notifies only the views that asked, staggered; exit animations and run-label shimmer not done |
| 7 | Workspace frames | ✅ | independent frames, 4 px gaps with resize handles |
| 8 | Sidebar width | ✅ | 264 min, ≤ 50 %, draggable |
| 9 | Sidebar nav order | ✅ | New · Search · Board · Extensions · view switcher |
| 10 | Groups, counts, pinned, unread | ✅ | virtualised `uniform_list` |
| 11 | Sidebar footer | ✅ | settings, update badge, model label + ⌘M, aggregate Unicrab glyph |
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
| 23 | Subagent rows | ✅ | AgentRow live status (state, steps, elapsed, latest action, Stop) from `_oino/agents`; Agent tab is a read-only child timeline (live feed + `_oino/agent_messages` backlog); `btw` / dream children have no tool row and open from the StatusPanel Run section; scenario `agents-live` |
| 24 | Turn summary + undo | ✅ | TurnFooter Review + Undo; Undo = files-only `_oino/rewind` behind a confirmation (`turn-undo`) |
| 25 | Rewind message | ✅ | user bubble hover → Rewind: forks before the message, restores its files and puts the text back in the composer, behind a confirmation (scenario `rewind-real`) |
| 26 | Thinking blocks | ✅ | |
| 27 | Composer shell | ✅ | |
| 28 | Model menu | ✅ | provider groups, badges, check, Manage models |
| 29 | Context ring | ✅ | click = compact |
| 30 | Mentions | ✅ | `@ / $ #` via `_oino/get_completions`, colored token chips |
| 31 | Attachments | ✅ | pasted (⌘V) and dropped images become image attachments sent with the prompt (≤ 16 MiB, png/jpeg/webp/gif), other dropped files `@path` mentions; an attachment strip shows what will be sent (`image-attach`) |
| 32 | History, drafts | ✅ | per session |
| 33 | Queue panel | ✅ | per item: ↑ / ↓ reorder (`queue_move`), send now, edit in place, recall to the composer, remove (`_oino/queue_edit`); scenario `queue-edit` |
| 34 | Steer / follow-up | ✅ | Enter / Alt+Enter |
| 35 | Permission dialog | ✅ | docked, 4 kinds, feedback, number keys |
| 36 | Dangerous command guard | ✅ | |
| 37 | Elicitation | ✅ | stepped QuestionCard, keys |
| 38 | Plan approval | ✅ | PlanReviewCard + Plan tab |
| 39 | Waiting badge | ✅ | |
| 40 | Status panel | ✅ | Goal/Progress/Run/Git from session state + the sticky topics (`long-horizon` state/progress, `unipi:bg:tasks`, `unipi:fusion:status`, `unipi:kanboard:status`, `oino:git`); Run rows open the child's Agent tab or the Tasks tab; keys ⌘⇧S expand, ↑/↓, Enter, Delete (confirm), Esc; scenario `status-panel` |
| 41 | Side pane tabs | ✅ | Review, Files, Plan, Agent, Tree, panels |
| 42 | Embedded browser | ✅ | side-pane Browser tab (gpui-webview 0.8.1): URL bar, back/forward/reload, devtools (debug), open externally; `openUrl`, transcript links, terminal ⌘-click and the `localhost:*` chip open there; Linux on X11/XWayland unless "Native Wayland" |
| 43 | PTY terminal | ✅ | bottom Terminal tabs (`alacritty_terminal` + platform PTY): tabs, split, copy/paste, ⌘-click links, 10k scrollback, token colours |
| 44 | Command center | ✅ | scopes, prefixes, sections, history |
| 45 | Primary modifier | ✅ | `cmd-*` on macOS / `ctrl-*` elsewhere (gpui 0.3.10 dropped `secondary-`), `⌘` labels become `Ctrl+` off Apple, incl. DockDialog |
| 46 | Settings | ✅ | page + hub |
| 47 | Toasts | ✅ | bottom-right, max 3, hover pauses |
| 48 | Split panes | ✅ | ≤ 4, close ≠ stop |
| 49 | Automations → Board | ✅ | |
| 50 | ZCode product features | ⛔ | |
| 51 | Mobile | — | design-mobile |
| 52 | Performance bar | 🟡 | RTX 4070 Ti numbers below; 4-pane whole-run p99 9.3–14.9 ms vs the 10 ms budget |
| 53 | i18n | ✅ | English |
| 54 | Window chrome | ✅ | client-side decorations on Linux/Windows: sidebar top row + headers drag (double-click zooms, right-click window menu), window controls in the right-most header / side tab bar, resize edges, Linux 16 px radius while floating; macOS transparent title bar |

Unipi surfaces (design.md §7): every row renders through the OUS renderer, the docked
interactions, DockStrip, MetaRow, StatusPanel, side/bottom tabs or overlays; kanboard has the
native board; panel buttons, list selection, inputs, tabs and keys send `_oino/ui_action`;
shortcuts go through `_oino/invoke_shortcut` (palette Actions); editor intents
(`changed`/`cleared`/`emptyArrowDown`/`submit`) through `_oino/editor_event`; the settings hub
uses `_oino/get_settings_schema` + `get/set/unset_extension_setting`; Models & Providers uses
`_oino/auth_providers` / `login` / `logout`. `service_call` is exposed on the pane
(`SessionPane::frontend`) but no native widget needs it yet.

## Performance (design.md §2.11)

`scripts/perf.sh [binary] [secs] [panes]` (release, demo `stream` scene). It now gates on the
**whole-run p99** (exact 0.1 ms histogram over every frame, `run_p99_ms`), prints the worst
120-frame window, frames over 8.3 ms, presented fps, latency marks (first frame, session
switch, palette open, synthetic keystroke → paint) and idle CPU (welcome scene, after the 60 s
splash drain). Frame cost = UI-thread time from the root `render` to the end of paint.

Real GPU (RTX 4070 Ti, GNOME XWayland `:0`, 25 s, 4 panes, synthetic typing):

| Run | fps | p50 | window p99 median | worst window p99 | **run p99** | > 8.3 ms | dropped |
|---|---|---|---|---|---|---|---|
| before fixes (`a83dd76` pacing) | 72 | 7.1 ms | 11.2 ms | 22.1 ms | 16.0 ms | 551 | 16 |
| after (staggered ticker, 30 fps background panes) | 96–101 | 3.6–3.9 ms | 7.5–8.4 ms | 11.9–15.0 ms | 9.3–11.6 ms | 65–129 | 0–4 |
| after, machine under load (load avg 7–10) | 96–101 | 4.4–5.7 ms | 9.6–12.5 ms | 17.0–19.9 ms | 13.3–14.9 ms | 303–492 | 4–5 |

Latency (same runs): first frame 280–290 ms after process start (cold Vulkan on the NVIDIA
driver), keystroke → paint median 3–5 ms (max 9–15), session switch 6–12 ms, palette 10k
items filter 0.98 ms (`cargo test --release -p oino-ui palette_10k`). Idle CPU: 1–3 % of the UI
thread (lavapipe's worker threads add ~19 % under Xvfb, ~1 % on the GPU).

Speed work: panes are `cached` views; the ticker notifies only the views that asked for a
frame (scoped per pane, round-robin over 4 sub-ticks) instead of `refresh_windows`, which had
re-rendered all four panes 12×/s; unfocused split panes pace their event batches at ≈ 30 fps
with a per-session phase; idle / sleeping crabs do not animate; preview-URL scanning is
cached per transcript version.

Open: the 4-pane whole-run p99 does not reliably meet the 10 ms budget (9.3–14.9 ms across runs
on a shared desktop): frames where all four panes re-lay out in one frame cost 6–12 ms.

## Requests

- D1 (integration): copy this file to `docs/dev/v1/deviations/design-desktop.md`; replace
  gpui-app.md §5 with a pointer to design.md (R6).
- core-runtime: file checkpoints for TurnFooter Undo and UserBubble rewind (the buttons stay
  hidden until then); single-item queue commands (remove / send now / reorder).
- core: honour `disabledExtensions` in the loader (the desktop host filters packages itself).
- modes (R3): `_oino/topics` sticky-topic feed for the StatusPanel; (R5) `_oino/agents` child
  event stream for live Agent tabs.
- design-mobile: `src/mobile/shell.rs` was adapted minimally to the new `design::theme` and
  `SidePane` APIs; it still uses literal styles (excepted in the guard test).
