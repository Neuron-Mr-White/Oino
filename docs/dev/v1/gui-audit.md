# Oino Desktop: click-through audit (v1.0.0-alpha.2)

Date: 2026-10-27 · branch `v1-alpha` @ `cafd7ae` · work id `gui-audit` · no code changed.

## Scope and method

- Binary: `apps/target/release/oino-desktop`, built with `cargo build --release -p oino-desktop`.
  This is the real embedded host with 32 bundled extensions: 8 providers and 24 unipi packages.
  It is not `--demo`.
- Config: scratch agent dir `/tmp/oino-audit` holding only `auth.json` (Oino Router key) and
  `settings.json` (`oino/deepseek/deepseek-flash`, thinking off). Project: the scratch git repo
  `target/tmp/gui-audit/repo` (`README.md`, `calc.py`). Real model calls were kept small: about
  10 prompts plus btw, one subagent and the auto-rename child.
- Driver: python-xlib XTEST (`xdotool` is not installed). Window shots come from
  `gst-launch-1.0 ximagesrc xid=…`.
- Display:
  1. The first two runs used the user's GNOME XWayland display (`:0`, NVIDIA Vulkan). Mutter
     ignores XTEST pointer motion on that display: the pointer never moved, and keys reached a
     different focused window. Click-through on `:0` is not possible without a portal.
  2. The rest of the audit ran on a private `Xvfb :97` (1600×1000) with lavapipe
     (`VK_ICD_FILENAMES=…/lvp_icd.json`). The app, size (1400×900) and agent dir were the same.
  3. One desktop instance at a time. Only the audit's own PIDs were killed.
- Patched binary for most of the audit:
  1. **The stock binary crashes within about a minute of start (GA-1).** To continue, the audit
     used a scratch copy of the tree (`target/tmp/gui-audit/src`, not committed) with a one-line
     workaround: `on_text_changed` runs `editor_event` on the tokio handle.
  2. Every other finding below comes from that binary. The fix itself is GA-1.
- Screenshots: `docs/dev/v1/gui-audit/*.png`, at most 1000 px wide, 96-colour PNG.
- Interruptions: two later process losses had nothing to do with the app. One was a host reboot.
  The other was the agent process being restarted, which killed its child tasks.
- unipi references:
  - packages under `/home/oi/Projects/Personal/archived/unipi/packages/*`
  - TUI screenshots in `archived/unipi/docs/assets/screenshots/`
  - [audit-unipi.md](audit-unipi.md), [unipi-port.md](unipi-port.md), [design.md](design.md) §7
    and Amendment A1

Severity: **P0** = crash, freeze, or a whole feature blank or unusable · **P1** = broken or
wrong behaviour · **P2** = polish.

## Summary

| Area | P0 | P1 | P2 | Total |
|---|---|---|---|---|
| A conversation / composer / timeline | 1 | 6 | 5 | 12 |
| B side pane / browser / terminal | 0 | 2 | 1 | 3 |
| C unipi panels / board / OUS / tray / footer | 5 | 10 | 1 | 16 |
| D settings / extensions / sidebar / shell / navigation | 1 | 4 | 3 | 8 |
| E core / backend gaps | 1 | 4 | 2 | 7 |
| **Total** | **8** | **26** | **12** | **46** |

These work end to end in the real app:
- prompt streaming, with tool rows for bash, read and edit
- turn diff card and Review pane, header ⋯ menu, Tree tab, PTY terminal, Output tab
- slash completion (unipi commands) and the command palette
- `/unipi:info` and `/unipi:settings` (hub with 25 namespaces and global/project scopes), MCP
  servers dialog and catalog, Extensions page
- `/unipi:bg` with toast, work tray (bg tab, task detail), `/unipi:btw` (real answer),
  `/unipi:hint` browser
- `/unipi:model` picker, `/unipi:agents` profiles, `run_subagent` with AgentRow, Agent tab and
  Status panel "1 agent"
- plan mode (`/unipi:plan`: PLAN badge, Plan chip, plan file toast), the approval card for
  write and bash, auto-rename of the session title
- `/unipi:doctor`, `/unipi:compact-vcc` (error path), `/unipi:footer off/on`, status toasts for
  memory, dream, mcp, compactor, fusion and visualize

## Crash / freeze / resource observations

1. **Crash 1: tokio reactor panic (GA-1, P0). Reproduced 3/3 with the stock binary.**
   - When it happens: about 10 to 60 s after start, with or without typing. The debounced
     `changed`/`cleared` editor event fires on programmatic composer edits too.
   - Panic chain:
     1. Worker thread: `there is no reactor running, must be called from the context of a Tokio
        1.x runtime` at `crates/oino-core/src/session/frontend.rs:447`.
     2. Main thread: `Task polled after completion`, exit 101.
   - Called from `apps/oino-ui/src/views/session_pane.rs:966-981`. The future runs on GPUI's
     `background_executor`, not on the tokio runtime.
   - This alone explains "the app often crashes". The panic needs only one subscriber to
     `oino_editor` (`unipi.core`, `unipi.utility`, `unipi.notify`, `unipi.input-shortcuts`). With
     those four disabled it still crashed, because the `cleared` path runs on every send.
2. **Crash 2: Mascot window (GA-33, P0).**
   - Trigger: Settings → Appearance → Mascot window ON.
   - Panic: `cannot read oino_ui::views::shell::Workspace while it is already being updated`.
     Frames: `MascotView::render` ← `MascotView::open` ← `Workspace::sync_mascot` ←
     `update_settings`.
   - `mascotWindow: true` is saved to `desktop.json` before the crash. **Every later launch
     crashes at startup** until the file is edited by hand. Logs: `crash-mascot.log` and
     `crash-mascot-startup.log` under `target/tmp/gui-audit/`, not committed.
3. **Soft freezes (GA-30, P0).**
   - An approval card left unanswered for 30 s makes the extension `tool_call` handler time out.
     The tool call fails.
   - The orphaned approval cards stay docked. Every send is refused with "Answer the pending
     request first" until each stale card is denied by hand.
   - The Board view stays on "Opening the board..." forever (GA-17).
4. **CPU and RAM.**
   - GPU run (`:0`, NVIDIA): RSS 865 to 880 MB right after start, 94 threads, 100 to 300 % CPU
     while the bundle compiled and initialised, then 1.2 % average once idle (`rss2.log`).
   - lavapipe run: 18.6 % of one core while idle with nothing streaming (186 ticks in 10 s),
     185 threads.
   - lavapipe RSS: 465 MB idle, growing to 558 MB over 27 minutes in one session with about
     10 turns (`rss-r5.log`).
   - There was no unbounded growth. The thread count is high: 185 with one or two sessions.
5. **herdr/tmux.** `grep -rniE 'herdr|tmux' apps/` (excluding `target/`) finds nothing. Related
   concepts that are present are listed in GA-36. Outside `apps/`:
   - `extensions/unipi/fusion/src/policy.rs:193` (`tmux capture-pane` allow rule)
   - `extensions/unipi/utility/src/settings.rs:226` (`rename.herdrSync: true`, a dead default)
   - the `skill:herdr` user skill from `~/.agents/skills`, shown in the slash menu even with a
     scratch agent dir

## A. Conversation / composer / timeline

| ID | Sev | unipi pkg | Steps | Expected (unipi) | Actual | Shot | Suspected file | Resolution |
|---|---|---|---|---|---|---|---|---|
| GA-1 | P0 | core (+ utility, notify, input-shortcuts subscribers) | Start the stock binary; type `/` or just wait | Composer edits feed `oino_editor` (`onTerminalInput` equivalent) without side effects | Process aborts within about a minute: tokio `spawn` outside the runtime, then `Task polled after completion` on main (§ Crash 1) | `02-gpu-slash-before-crash.png` | `apps/oino-ui/src/views/session_pane.rs:966-981` (`background_executor().spawn` → use `Tokio::spawn`); `crates/oino-core/src/session/frontend.rs:447` | Fixed ([stability] + [core-gui-gaps] GA-40): editor events run on the stored tokio handle; re-verified [gui-fix-conversation] on Xvfb — hours of typing across sessions, no crash log |
| GA-2 | P1 | command-enchantment / all | Click the composer and type `/unipi:plan` quickly (15 ms/key, as from paste or a fast typist) | Text arrives intact | Keys typed right after `/` are lost: `pi:plan` and `pi:bg-tasks` were sent to the LLM as prompts and became session titles | `41-plan-prefix-dropped.png`, `26-pibgtasks-and-notification.png` | `session_pane.rs` `on_text_changed` / completion refresh / `pending_text` `set_value` race | Fixed [gui-fix-conversation]: a late completion answer for older text never replaces newer keys (staleness guard + active-row preservation); verified on Xvfb at 20 ms/keystroke with the `/` menu open — every key arrives (instrumented composer log) |
| GA-3 | P1 | (pi built-ins) | Type `/new` (also `/compact`, `/model`, `/resume`, `/name`…) and press Enter | Built-in commands act locally (pi interactive mode; unipi TUI) | `/new` goes to the model as a user message, so the session is titled "/new". The slash menu lists `/new` only from the command-enchantment group | `07-slash-new-not-builtin.png`, `08-new-sent-to-llm.png` | `apps/oino-ui/src/model/composer.rs:42` `intent()` has no slash dispatch | Fixed [gui-fix-conversation]: `intent()` dispatches pi built-ins locally (`Intent::Builtin` → `run_builtin`: /new /compact /model /thinking /name /session /tree /fork /clone /copy /export /import /settings /resume /reload /changelog /hotkeys /trust /extensions /quit); `/new` verified live — nothing reaches the model |
| GA-4 | P1 | command-enchantment | Type `/comp` | One enchanted group that replaces the base group (`replaces: "commands"`, deviation ux#1), with coloured package tags `c1…c12` | Two groups ("Commands" and "unipi.command-enchantment") list the same commands. Tags are grey, with no tone colours | `06-slash-duplicate-groups.png` | `session_pane.rs` completion merge; `components/composer.rs` | Fixed [gui-fix-conversation]: `merge_completions` folds an enchanted group into one "Commands" group that replaces its base rows, tags render as colour pills (`chip::tag`, categorical c1–c12); `39-conv-completion-menu.png`; unit test |
| GA-5 | P1 | core (header) | Open any session that has messages | unipi prints the start header once and it scrolls away; design §7: slim card in non-empty sessions | The full Unicrab header (about 200 px, large crab and text) is pinned above the timeline in every session. In the welcome view the large crab art is broken into a tall column at some widths, and two crabs show (pixel crab and art) | `09-run-tools.png`, `04-welcome.png`, `35-subagent-done.png` | `components/conversation.rs` header slot; OUS `responsive` variant pick | Fixed [gui-fix-conversation]: the splash is an EmptyState (crab 88 + build line) shown only in an empty session; a non-empty session shows one collapsed harness card at the top that scrolls away, never pinned, one crab total; `37-conv-welcome.png`, `42-conv-turn-done.png` |
| GA-6 | P1 | memory, background-tasks, kanboard, workflow | Start a session; run `/unipi:bg …`; run `/kanboard` | Compact harness cards: memory recall card; “✓ Background task … completed” card; kanboard help table | The memory reminder is a huge markdown card with H1/H2 (“Wake-up text”) at the top of each session. The bg notification is raw text (`t7b5… sleep 45; echo done completed 0 /tmp/…`). The kanboard help table is flattened into one paragraph. The plan-mode reminder is a full-length card | `08-new-sent-to-llm.png`, `26-pibgtasks-and-notification.png`, `40-board-newtask-help.png`, `42-plan-mode.png` | `components/timeline` custom-message / renderer lookup; `tasks.notification` and `utility` harness renderers not used | Fixed [gui-fix-conversation]: harness cards (`model::timeline::harness_card`) collapse to one line (radius 14, inset line, claw icon + source + title + chevron), tagged `<background-task-notification>` bodies parse to a summary + `key: value` body; plan-mode reminder and memory cards included; unit tests |
| GA-7 | P1 | utility (answer) | Reply with questions, then `/unipi:answer` | Fixed input box under a scrollable reply (unipi `answer`) | The dialog is taller than the window and is clipped at the top; its title and first lines are off-screen | `60-answer-clipped.png` | `components/interaction/dock_dialog.rs` / `components/dialog.rs` (no max height / scroll) | Fixed [gui-fix-conversation]: the interaction card takes the composer slot capped at min(64 % of frame, 560) (`R_DIALOG_CARD` 24), the body scrolls inside the cap and the footer stays pinned; `52-conv-memory-import-card.png`, `60-conv-card-45s.png` |
| GA-8 | P2 | — | Run any tool turn | — | Stray `—` / `☰` glyphs in the left gutter (x≈285) overlap the transcript | `09-run-tools.png`, `29-tray-task-detail.png` | `components/timeline` turn rail marker | Fixed [gui-fix-conversation]: the turn rail marker is gone — the fold-line model (A2.10) replaces the explore rail, and nothing draws in the reading-column gutter |
| GA-9 | P2 | compactor | `/unipi:compact-vcc` on a small session | Short notice | "Compaction failed: Compaction failed: Nothing to compact…" with "Switch model" offered for a non-model error | `61-compact-error.png` | timeline error row; compactor error text | Fixed [gui-fix-conversation]: the timeline dedupes the `Compaction failed:` prefix, "Nothing to compact" renders as an info notice, and "Switch model" is offered only for model/provider errors (`is_model_error`); `55-conv-compact-error.png`; unit tests |
| GA-10 | P2 | workflow / status | The model asks 3 clarifying questions in plan mode | Progress = plan todos | The Status panel shows "Progress 3/3" with the assistant's questions as struck-through done items, and it persists | `42-plan-mode.png` | `model/status.rs` todo extraction | Fixed [gui-fix-unipi]: Progress reads only the workflow's approved-plan handoff (the "Implement the approved plan below." user message); assistant lists/questions are never todos and a finished turn no longer strikes everything through; unit tests (`assistant_questions_are_not_plan_steps`) |
| GA-11 | P2 | — | Restart the app | Reopen the last session | Every launch adds another empty "New session" row in the sidebar | `64-session-context-menu.png` | `views/shell.rs` startup open | Fixed `03c20c5` [gui-fix-shell]: startup reopens the last shown session (`lastSession`, Settings → General → Reopen the last session); no new empty row per launch |
| GA-12 | P2 | core (hints) | Start at 10:56 local | Local greeting | Splash says "It's late — remember to rest." (UTC hour; known R3) | `04-welcome.png` | `extensions/unipi/info-screen`, `host-info` UTC offset | Open (blocked): `host-info` exposes no local time / UTC offset (unipi-core request R3, STATUS §6); the guest only has `timers.now-ms` (UTC). Needs a WIT addition (`local-offset-minutes`), owned by core-runtime |

## B. Side pane / browser / terminal

| ID | Sev | unipi pkg | Steps | Expected | Actual | Shot | Suspected file | Resolution |
|---|---|---|---|---|---|---|---|---|
| GA-13 | P1 | — (A1.4 browser) | Side pane → `+` ("Open a tab") | Launcher menu: Files, Plan, Browser, Tree… | Only re-selects Review. There is no menu, so the Browser tab cannot be opened by hand (only through extension `openUrl`) | `12-sidepane-plus-noop.png` | `components/side_pane.rs:338-345` | Fixed [gui-fix-panes]: `+` opens an anchored launcher menu (Review, Files, Plan, Tree, Browser, Terminal, `sidePane` panels); `31-side-launcher-menu.png`, `32-side-strip-terminal.png` |
| GA-14 | P1 | background-tasks | Run `/unipi:bg sleep 45`, then open the bottom frame → Tasks | D7 bottom Tasks tab lists tasks from `unipi:bg:tasks` | "No background tasks" while the footer says "Background tasks 1" | `38-tasks-tab-empty.png` | `components/side_pane.rs` Tasks tab feed | Fixed [gui-fix-panes]: Tasks tab reads the `unipi:bg:tasks` topic (name, command, status capsule, elapsed, Logs / Stop); tab title counts running tasks; `34-bottom-tasks.png` |
| GA-15 | P2 | subagents | Click the AgentRow | Live read-only child timeline (§7) | Final text only, plus "Live child timelines need the `_oino/agents` feed" | `36-agent-tab.png` | see GA-44 | Fixed [gui-fix-panes] (needs gui-fix-conversation's `AgentFeed` to land for the live part): Agent tab renders the live read-only child timeline from `_oino/agents` with state, steps, elapsed, Stop agent; `33-agent-tab.png` |

The PTY terminal (`echo` round trip, prompt, colours), Output tab, Review diff and Tree tab
worked (`37-terminal.png`, `11-review-pane.png`, `14-tree-tab.png`).

## C. unipi panels / board / OUS / tray / footer

| ID | Sev | unipi pkg | Steps | Expected (unipi) | Actual | Shot | Suspected file | Resolution |
|---|---|---|---|---|---|---|---|---|
| GA-16 | P0 | input-shortcuts, workflow, core, subagents, background-tasks | Press `alt+p`, `alt+m`, `alt+s`, `alt+h`, `shift+down`, `ctrl+alt+c` in the composer | Plan toggle, permission cycle, chord overlay, next hint, tray, bg clear (`setShortcuts`) | Nothing happens. Extension shortcuts are not key-bound, not listed in the palette ("plan" finds only the slash command) and not on Settings → Keyboard shortcuts. `invoke_shortcut` is reachable only from palette entries that never appear. The whole input-shortcuts package is unusable. `ctrl+b` (subagents foreground) collides with "Toggle sidebar" | `43-altp-noop.png`, `45-palette-no-ext-shortcuts.png`, `46-shortcuts-page.png` | `components/keys.rs` (no dispatch of `ui.shortcuts`); `views/shell.rs:982`; `views/settings_page.rs:948` | Fixed `03c20c5` [gui-fix-shell]: `setShortcuts` keys bound by the shell keystroke interceptor (alt+p toggles plan mode, verified on Xvfb); listed in the palette (Extensions group) and Settings → Keyboard shortcuts; ctrl+b goes to the extension only while typing (one-time note) |
| GA-17 | P0 | kanboard | Sidebar → Board (also the "Board" starter card) | Board panel / D9 board with lanes (`/unipi:kanboard open`) | Stuck on "Opening the board..." forever. It runs `/kanboard` (bare = help), which posts the help text into the chat. The daemon never starts (`oino-kanboard` not found, GA-41) | `39-board-stuck.png`, `40-board-newtask-help.png` | `views/board.rs:246`, `views/shell.rs:1112`, `components/conversation.rs:283` (`/kanboard` → `/unipi:kanboard open`) | Fixed [gui-fix-unipi]: Board runs `/kanboard open` once per session (`ensure_board`, A2.17 page: lane strip, lanes, dashed empties, FAB), `oino-kanboard` is found next to the binary (GA-41), and an unregistered project gets a "Set up the board" panel; verified live on Xvfb (daemon `serve --port 0`, lanes, task added) — `07-board.png`, `65-unipi-board-newtask.png` |
| GA-18 | P0 | app-bridge | `/unipi:app` | Control centre side pane (`unipi.app-bridge/control`) | Nothing appears: no panel and no toast | `55-unipi-app-noop.png` | `components/side_pane.rs:152` panel routing; app-bridge `openPanel` | Fixed [gui-fix-panes]: `/app` and its shortcut re-send `openPanel` with `focus: true` (app-bridge), so the Control centre side tab shows even after the silent session-start auto-open; `35-app-control-centre.png` |
| GA-19 | P0 | updater | `/unipi:readme`, `/unipi:changelog`; Extensions → Readme icon | README browser / CHANGELOG panel | "No bundled READMEs found." and "No changelog found." The Extensions Readme button does nothing | `51-readme-empty.png`, `52-changelog-empty.png`, `50-ext-readme-noop.png` | `extensions/unipi/updater` doc paths vs `oino_bundle::DOCS` extraction; `views/extensions_page.rs:92` | Fixed [gui-fix-unipi]: `/readme` finds the extracted `oino.docs` README (list → markdown panel, `64-unipi-readme.png`); `/changelog` is the updater's hidden runner — the core built-in forwards to it when the updater is loaded (`has_command("readme")`), showing the version list from the bundled/remote changelog (routing verified live: `oino.command-echo changelog`); the Extensions Readme hunk landed in `03c20c5` [gui-fix-shell] |
| GA-20 | P1 | kanboard | Board → "New task" | `/unipi:kanboard-add` prefilled, or a native form | Composer gets `/kanboard add ` (wrong command) | `40-board-newtask-help.png` | `views/board.rs:96` | Fixed [gui-fix-unipi]: "New task" (FAB, `c`, the lane's dashed Add-task row) opens a native dialog (title/description) that submits `/kanboard-add <title>[\n<body>]`; verified live — a task landed in Backlog (`65-unipi-board-newtask.png`) |
| GA-21 | P1 | core (tray), subagents | `↓` on an empty composer; `/unipi:subagents` | Work tray opens (`emptyArrowDown`); `/unipi:subagents` opens the Subagents tab | `↓` does nothing (with a bg task present). `/unipi:subagents` shows only the toast "No subagents in this session yet." Only `/unipi:bg-tasks` opens the tray | `24-arrow-down-no-tray.png`, `27-subagents-toast-only.png` | `session_pane.rs:1268-1282` (consumed intent → no panel); core tray `openPanel` | Fixed [gui-fix-conversation]: `↓` sends `emptyArrowDown` and falls back to the `core.open-tray` service when no extension consumes it; `/subagents` opens the work tray on its tab (consumed-intent path kept); the tray renders in the composer slot with one legend (GA-22 hunks land with [gui-fix-unipi]) |
| GA-22 | P1 | core (tray), btw | Open the tray / btw | One hint legend; Esc closes or goes back | The key legend is printed twice ("↔ tabs · esc close" as title and again under the buttons; btw legend twice). Esc does not close the tray or leave task detail; only ✕ works | `28-work-tray.png`, `29-tray-task-detail.png`, `30-btw.png` | OUS panel renderer (title + `footer` hint both drawn); panel key routing | Fixed [gui-fix-unipi]: overlay panels render through `ous::panel_dialog` — one footer legend from the panel `keys` (digits collapse: `1–9 page`), keys bound while the dialog has focus, Esc closes; TUI `keyHints` rows are stripped and hint-only titles become real titles ("Work tray") — verified live (`61-unipi-tray.png`: segmented tabs, one legend, Esc closes); unit tests in `model/ui_state.rs` |
| GA-23 | P1 | info-screen | `/unipi:info` | unipi dashboard: tab row, big-digit spend, bar charts, `r/R/q/1-9` keys (`info-usage.png`) | Panel keys render as a row of buttons "Refresh r, Refresh all R, Close q, Page 1…Page 9" running past the dialog edge. Context bars print raw floats (`4591.231092902226`). Usage shows "$0 · 0 sessions" with 30 text rows instead of a bar chart. The dialog stays over Settings after `/unipi:settings` | `15-info-session.png`, `16-info-usage.png`, `18-info-over-settings.png` | `components/ous` (`barChart`, `meter` number format, panel `keys` → buttons); info-screen usage index | Fixed [gui-fix-unipi]: panel keys are a footer legend (no buttons); stat tiles render big-digit values (`Title` role); meters format `11.6k / 1M · 1%`; `barChart` with `columns: true` (or >12 bars) draws the 30-day column chart with sparse labels; `2`/`r`/`q` panel keys verified live — `09-info-overlay.png` |
| GA-24 | P1 | memory, dream, fusion, mcp, compactor | `/unipi:memory status`, `/unipi:dream status` | Full multi-line status | Shown as small toasts that clip after 4 lines ("migration: needed — run /unipi:memory r…") and auto-dismiss | `54-dream-status-toast.png` | `components/toast.rs` (no expand / no card for multi-line notify) | Fixed `03c20c5` [gui-fix-shell]: multi-line notify opens a result card dialog (title, lines, Copy / Done); clipped toasts get More |
| GA-25 | P1 | footer, workflow | `/unipi:footer off` | Footer off, plain editor; mode unaffected | The permission chip changes from "Ask before changes" to "Auto": the mode chip is derived from footer segments | `62-footer-off-mode-chip.png` | `components/composer.rs` mode chip source (`unipi:workflow:status` topic, not the footer frame) | Fixed [gui-fix-conversation]: the mode chip reads the sticky `unipi:workflow:status` topic (`ModeChip::from_topic`; footer segments are only a remembered fallback), so `/footer off` leaves the mode intact; `56-conv-footer-off-mode-chip.png`; unit tests |
| GA-26 | P1 | footer | Any run | unipi glance frame: boxed editor, rainbow UNIPI brand, stats strip (`glance-footer.png`) | No frame box, no brand effect. The stats strip overflows on the right ("66% cache⇢hit" clipped) and wraps under the composer | `09-run-tools.png`, `26-pibgtasks-and-notification.png` | `components/meta_row.rs`, `components/composer.rs` (A1 look) | Fixed [gui-fix-unipi]: the glance frame under the composer draws the frame line (left/right groups, 1 px dividers, toned spans) and the `setFooter` stats strip on its own centred line; the rainbow wordmark renders as toned letters, lowest priorities drop (never wraps) with `⋯` for the rest; verified live — segments `UNIPI · Regular Mode · repo · main · auto`, strip `↑82k in · 772 out · 21s avg ttft · 143 tok/s · 1 turn · 14 steps · 5:44:37 · tool 00:00`; unit test `strip_drops_low_priority_instead_of_wrapping` |
| GA-27 | P1 | workflow | Ask mode, model calls `write` | Approval card; the click lands on the chosen action | The card grows by about 36 px after first paint. A click aimed at "Allow once" landed in "Why deny?", the card waited and then timed out (GA-30) | `57-approval.png`, `58-approval-orphan-timeout.png` | `components/interaction/` approval layout (feedback input appears late) | Fixed [gui-fix-conversation]: the card is laid out on the first paint — the feedback field's height is reserved from the start (`field_box`), options are fixed 46 px rows and the footer never moves; approvals map allow-once → primary, always → secondary, reject → danger pill (A2.11) |
| GA-28 | P1 | ask-user, workflow | Leave plan mode → ask mode, then ask for `ask_user` | All tools active again after plan mode | The model reports tools = read/bash/edit/write/plan_submit/memory/notify/web/images/subagents: no `ask_user`, no `bg_*`. The QuestionCard could not be exercised | `59-workflow-timeout.png` | `extensions/unipi/workflow` `set-active-tools` restore on mode change | Fixed [gui-fix-conversation]: every plan-mode toggle re-adds dropped interaction tools (`restored_tools`: `ask_user`, `bg_run/status/logs/kill` — only registered ones), healing stale-snapshot `set-active-tools` races between extensions; unit test; live model check was blocked by the router 402 (quota) |
| GA-29 | P1 | fusion | `/unipi:model` | Picker with lead/sidekick focus, filter, thinking | Works, but every row reads "· None", the current model shows "no pricing", and there is no scroll affordance past row 9 | `32-unipi-model.png` | fusion row labels; `dock_dialog.rs` list | Fixed [gui-fix-conversation]: option rows render through the A2.11 `option_row` (a missing description draws nothing instead of "· None", number tile + label + description), and the option list scrolls inside the card's capped body with the footer pinned; `57-conv-fusion-picker.png` |
| GA-30 | P0 | workflow | Ask mode; leave an approval for more than 30 s (or mis-click) | unipi waits for the user indefinitely | `unipi.workflow (tool_call): handler timed out after 30000 ms`, so the tool fails. The card stays docked as an orphan ("1 of 2") and blocks every send until denied by hand | `58-approval-orphan-timeout.png`, `59-workflow-timeout.png` | `crates/oino-wasm-host/src/instance.rs:401` deadline; `crates/oino-core/src/extensions/runner.rs:361`; dialog cleanup on handler timeout | Fixed ([core-gui-gaps] GA-42 + [gui-fix-conversation]): the deadline pauses while a dialog is open and orphan dialogs are withdrawn on handler death; the frontend drops cards on `extension_ui_dismiss`, reconciles the queue against the UI snapshot on reload, and never auto-times out — verified live, a docked card is pixel-identical after 45 s (`60-conv-card-45s.png`) |
| GA-31 | P2 | info-screen / core | `/unipi:hint`, MCP catalog | — | Both work; the dialogs are fine. The hint browser has no "learned" toggle action | `31-hints.png`, `23-mcp-catalog.png` | — | Fixed [gui-fix-unipi]: every hint row in the `/hint` browser has a learned pill ("mark learned" / "✓ learned") backed by the new OUS list-item `action`; toggling persists in the hint store and refreshes the panel — verified live (`63-unipi-hints.png`); unit tests in `hints/{store,render}.rs` |

## D. Settings / extensions / sidebar / shell / navigation

| ID | Sev | unipi pkg | Steps | Expected | Actual | Shot | Suspected file | Resolution |
|---|---|---|---|---|---|---|---|---|
| GA-32 | P0 | core (mascot, A1.2) | Settings → Appearance → Mascot window ON | Floating Unicrab window | Panic (double lease, § Crash 2), then a crash loop at every launch | — (logs) | `apps/oino-ui/src/views/mascot.rs` `render` reads `Workspace` inside `Workspace::sync_mascot` update; `views/shell.rs` | Fixed [gui-fix-unipi]: the mascot window renders a `MascotSnapshot` pushed by the shell (never reads `Workspace` while drawing), opens in a deferred callback outside the shell update, fades its bubble and reopens on size change; verified live — `mascotWindow: true` starts without a crash, the 372×176 always-on-top window shows the animated crab and no crash-loop (`62-unipi-mascot.png`); unit tests for `split_bubble` / `bubble_opacity` |
| GA-33 | P1 | workflow | Settings → Permissions | Saved rules + mode (D8 Permissions page) | "unipi.workflow is not loaded" while workflow runs (plan/ask modes work). The check looks for namespace `workflow`; the hub namespace is listed as "Permissions" | `47-permissions-not-loaded.png` | `views/settings_page.rs:1249-1254` | Fixed `03c20c5` [gui-fix-shell]: Permissions resolves the hub namespace by extension id `unipi.workflow` |
| GA-34 | P1 | — | `Ctrl+M` in the session | Model menu (shortcut page lists Ctrl+M) | Nothing opens | `65-ctrl-m-noop.png` | `components/keys.rs` | Fixed `03c20c5` [gui-fix-shell]: Ctrl+M opens the model menu from any focus (verified on Xvfb) |
| GA-35 | P1 | — (owner goal 3) | `Ctrl+\`, palette "Split pane", session menu "Open in new pane" | Single native session view (owner: no herdr/tmux-style workspace/pane concepts) | Up to 4 split panes, hidden live panes (`MAX_LIVE_PANES`), a "Workspace" type grouping sessions, "New session in folder". This is a multiplexer model | `63-split-panes.png`, `64-session-context-menu.png` | `views/shell.rs:1-140` (`Workspace`, `Pane`, `split`) | Fixed `03c20c5` [gui-fix-shell]: split panes, layout, `Ctrl+\`, Open in new pane and the Workspace/Pane/MAX_LIVE_PANES model removed (`AppShell`, `LiveSession`, `MAX_WARM_SESSIONS`); one visible session, background sessions keep running |
| GA-36 | P1 | all (owner goal 1) | Compare with unipi screenshots | Look close to UNIPI (A1): glance frame, brand rainbow, unipi dashboard visuals | ZCode-style chrome dominates: frameless composer, plain toasts, dashboard without the unipi typography/charts; only the palette and crab come from unipi | `04-welcome.png`, `16-info-usage.png` vs `archived/unipi/docs/assets/screenshots/*` | `apps/oino-ui/src/design/*`, `components/composer.rs`, OUS renderer | Partly fixed `03c20c5` [gui-fix-shell]: A2 tokens, flush shell, sidebar, list column, page/detail headers, palette, dialogs, menus, toasts, settings, extensions, onboarding in the unipi look; composer, timeline, OUS and glance frame belong to gui-fix-conversation / gui-fix-unipi |
| GA-37 | P2 | updater | Settings → About & updates | Version + "Check for updates" (updater badge) | Read-only key/value rows; General and Sessions pages are read-only too (no default model, no session dir setting) | `48-about.png` | `views/settings_page.rs` simple pages | Fixed `03c20c5` [gui-fix-shell]: About has Check for updates and Readme; General (default model, thinking, folder, startup) and Sessions (session folder, steering / follow-up delivery, auto-compact, retry) are editable |
| GA-38 | P2 | — | Slash menu with a scratch agent dir | Skills from the agent dir | `~/.agents/skills` (`skill:herdr`, `skill:higgsfield-*`…) always load, including a herdr skill | `05-slash-menu.png` | `crates/oino-resources/src/discovery.rs:48` | Fixed `03c20c5` [gui-fix-shell]: with a custom `OINO_AGENT_DIR` the implicit `~/.agents/skills` is not discovered (`oino-resources` discovery) |
| GA-39 | P2 | memory | First start with `~/.unipi/memory` present | One prompt | "Import your unipi memories?" docked card, plus a "Needs you" badge, plus a toast on every new session | `01-gpu-start-real-display.png`, `63-split-panes.png` | `extensions/unipi/memory` start notice | Fixed (no longer applies): the legacy import prompt was removed with `unipi-native` (nothing reads `~/.unipi`); verified live with `~/.unipi` present — no import card, no Needs-you badge, no toast; the per-session wake-up renders as one collapsed harness card ([gui-fix-conversation] GA-6) plus the `🧠 0 mem` status chip |

## E. Core / backend gaps

| ID | Sev | Area | Finding | Evidence | Suspected file |
|---|---|---|---|---|---|
| GA-40 | P0 | core runtime | `AgentSession::editor_event` calls bare `tokio::spawn` for non-consumable intents, so it panics when polled off the runtime. The in-process backend is not runtime-agnostic (same root as GA-1) | § Crash 1 | `crates/oino-core/src/session/frontend.rs:447` (use the stored runtime `Handle`) |
| GA-41 | P1 | bundling | `oino-kanboard` is not found by the desktop app (not bundled next to `oino-desktop`, not on PATH), so the kanboard daemon cannot start | GA-17 | `apps/oino-desktop` packaging / `HostConfig` program search path |
| GA-42 | P1 | wasm host | Extension handler deadline (30 s) also applies to handlers blocked on a user dialog (`approval`, `form`, `select`). Dialogs are not withdrawn when the handler dies | GA-30 | `crates/oino-wasm-host/src/instance.rs:401`, `crates/oino-core/src/extensions/runner.rs:361` |
| GA-43 | P1 | frontend protocol | Extension shortcuts (`setShortcuts`) are not delivered to, or kept by, the desktop shortcut table | GA-16 | `model/ui_state.rs:603` vs `views/settings_page.rs` |
| GA-44 | P1 | frontend protocol | `_oino/agents` feed missing, so there are no live child timelines or child tool rows | `36-agent-tab.png` | modes / core (STATUS §8.2) |
| GA-45 | P2 | host-info | `/unipi:doctor` reports `Mode::Rpc; frontends: none` inside the desktop app, so extensions cannot tell a GPUI frontend is present | `53-doctor.png` | `apps/oino-desktop/src/embedded.rs` (`mode: ExtensionMode::Rpc`, frontend list) |
| GA-46 | P2 | perf | 18.6 % of one core while idle on software GL; 185 threads; RSS 465 to 558 MB in 27 min. On GPU the start RSS is 880 MB | § Resources | GPUI refresh loop (`Refreshing every 8ms`), status timers |

## Not covered (blocked or out of budget)

- **ask_user QuestionCard**: blocked by GA-28.
- **Goal / ralph / swarm**: owner flows would spend real model budget.
- **Notify, memory and web tools, image tools, `/unipi:move`, dream run, fusion sidekick
  handoff**: model or external cost, and the core e2e already covers them.
- **Browser tab**: unreachable (GA-13; fixed by [gui-fix-panes]: the `+` launcher opens it).
- **Light theme, onboarding, Windows and macOS**: not exercised.
- **Click-through on the GNOME display**: XTEST motion is ignored by mutter.

## Top 15 (fix order)

1. GA-1 / GA-40: `editor_event` tokio panic, crashes every run.
2. GA-32: mascot toggle panic and startup crash loop.
3. GA-30 / GA-42: approvals time out after 30 s and orphan cards block the composer.
4. GA-16 / GA-43: no extension shortcuts at all (input-shortcuts dead; alt+p, alt+m, alt+h,
   shift+down).
5. GA-17 / GA-41 / GA-20: Board blank, wrong commands, kanboard binary missing.
6. GA-18: `/unipi:app` control centre blank (fixed by [gui-fix-panes]).
7. GA-19: readme / changelog / Extensions Readme empty.
8. GA-2: keystrokes lost after `/`, so commands are sent to the LLM.
9. GA-3: pi built-in slash commands not handled (`/new` goes to the model).
10. GA-28: extension tools stay inactive after plan mode (ask_user, bg_*).
11. GA-21 / GA-22: work tray not on `↓`, `/unipi:subagents` toast only, duplicate legends,
    Esc dead.
12. GA-23: info dashboard rendering (buttons for keys, raw floats, no charts, empty usage).
13. GA-6: harness / custom messages render raw (bg notification, memory card, kanboard
    tables).
14. GA-35 / GA-36: split-pane multiplexer model and ZCode look, against owner goals 1 and 3.
15. GA-5 / GA-26: pinned Unicrab header in every session, and the footer glance frame missing
    or overflowing.
