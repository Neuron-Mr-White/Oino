# ZCode micro-behaviour detail pass (Oino Desktop)

Work id `[zcode-perf]`, `v1-alpha`, 2026-10-29. Owner priority #2 (after unipi): ZCode UX and
interaction parity, **without changing unipi theming** ([design.md](design.md) A2 still owns
the look).

Sources, read for this pass (`/tmp/zcode-src`, `git clone --depth 1
https://github.com/zai-org/ZCode`): `DESIGN.md` (Motion, Depth, Responsive), `docs/ui/*.md`
(~180 specs) and `packages/ui/src` (`v4/ConversationRowView.tsx`, `v4/ConversationComposer.tsx`,
`v4/messageTimeLabel.ts`, `ModelConfigSelect.tsx`, `styles.css`, `CommandCenter*`,
`ToolCallBlocks/*`).

Round 2 (same work id, 2026-10-29) added the rows marked 🆕🆕: answer timestamps, the
session context-menu parity items, the queue-recall draft guard and the per-pane pacing
phase (§1 of the perf row in [DESIGN-STATUS.md](../../apps/oino-desktop/DESIGN-STATUS.md)).

Format per row: **ZCode behaviour** (doc) → **Oino today** → state.
✅ parity · 🆕 added in this round · ⛔ deferred (reason) · ➖ not applicable to Oino.

## 1. Motion (design.md §8 row 6)

| # | ZCode | Oino |
|---|---|---|
| M1 | Overlays enter with fade (+ small rise/zoom), 100–300 ms, `--ease-out`; nothing springy (`DESIGN.md` Motion) | ✅ `design::motion::enter` (180 ms, 4 px rise), `enter_ms`, `pane_in` (16 px slide) |
| M2 | **Exit** is animated, not a removal: `zcode-collapsible-up` + `zcode-collapsible-fade-out` (300 ms) for collapses, toasts animate out | 🆕 `design::motion::exit` / `exit_ms`: fade 1→0 + 4 px drop over `EXIT_MS` (120 ms). Used by toasts (`Toasts::dismiss` only marks, `reap` after the exit) and the run-state line (`SessionPane::run_exit`) |
| M3 | Running label gets a sweep/shimmer (`zcode-stream-text-in`, kind-label "扫光") | ✅ 1.6 s opacity sweep on the fold line, the live tool row, the thinking label (🆕 while `Thinking`) and the run-state label (`dock::render_run_state`) |
| M4 | `prefers-reduced-motion: reduce` turns every keyframe off and keeps the end state | ✅ `OinoTheme::reduce_motion` (Follow OS / On / Off) → `enter`/`exit` return the static element, `Ticker::frame` is 0, Unicrab/ous effects static |
| M5 | Ticker/idle: animations only run while something animates | ✅ one app-wide 12 fps ticker, notifies only the views that asked, stops when nothing asks (GA-46) |
| M6 | Tool-call rows keep a static kind icon while running (no spinner per tool) | ✅ verb + `spinner` only on the single live line |

## 2. Timeline and scroll (rows 14–26)

| # | ZCode | Oino |
|---|---|---|
| T1 | Bottom pinning is an intent state machine: FOLLOWING follows stream growth, DETACHED keeps the reader's position, `> 48 px` from the bottom detaches, back-to-bottom/new prompt re-attach, find/turn-nav/fork anchors detach first (`chat-scroll-bottom-pinning`) | 🆕 same split on `ListState`: find (open + step), query-rail jump and keyboard row moves `pause_following_tail()`; the jump pill and a sent prompt `set_follow_mode(FollowMode::Tail)` |
| T2 | "Back to bottom" only shows when detached **and** content overflows | ✅ `jump_pill` returns `None` when `max - at <= viewport`; busy runs add the unread dot |
| T3 | `scrollbar-gutter: stable` so content does not shift when the bar appears | ➖ GPUI scrollbars float over the content; no gutter needed |
| T4 | Thinking block label: `Thinking` while streaming, `Thought for 12s` when done, level control in the composer (`chat-thought-level-control`) | 🆕 `Thinking` (shimmer) → `Thought for 12.3s` (`AssistantRow::thinking_ms`), `Thought` for history rows (no timing on disk) |
| T5 | Assistant fold: `Working for …` while running, `Worked for …` collapsed after (`assistant-message-history-layout`) | ✅ one fold per run with `Ran commands · 2 steps · 3.4s`, live action shimmer |
| T6 | Copy on a row: glyph → check for 1200 ms (`CopyRowAction` in `v4/ConversationRowView.tsx`) | 🆕 `SessionPane::mark_copied` / `copied_check`; the user bubble Copy and a new hover bar on every finished assistant answer |
| T7 | Message actions appear on row hover (ghost icon buttons) | ✅ user bubble bar (Copy / Edit & resend / Rewind), 🆕 assistant bar (Copy) |
| T8 | Inline edit diff, "View diff", `+N −N` per turn, turn undo | ✅ InlineDiff 40 lines, Review tab, TurnFooter, Undo (files) |
| T9 | Tool row: one line, kind label with a static icon, hover reveals the expand chevron, details on demand | ✅ same; explore/terminal/change folds |
| T10 | Query rail: one bar per visible user query, ≥ 2 queries and ≥ 864 px wide, tooltip = query + owning answer excerpt, click = exact anchor (`conversation-turn-navigator`) | ✅ `render_rail` gates on ≥ 2 prompts and the free margin left of the 896 px column; tooltip `query\n— excerpt`; 🆕 the click detaches pinning first |
| T11 | Find: ⌘F inside the conversation column, case toggle, wrap-around, Esc yields to an open nested overlay first (`chat-find-placement`) | ✅ `FindBar` in the transcript column, wraps, `case` toggle; Esc closes the bar first (pane key handler) |
| T12 | Virtualised rows, row-level re-measure, incremental markdown | ✅ `list` + per-row `remeasure_items`, `TextViewState` per row |
| T13 | Assistant actions bar ends in the arrival time: `HH:MM` today, `Yesterday HH:MM`, `M/D HH:MM` this year, `Y/M/D HH:MM` older (`v4/messageTimeLabel.ts`; history rows have no timestamp) | 🆕🆕 `message_time_label` on the hover bar (`as-time-N`, caption colour); `AssistantRow::created_ms` stamped at `message_start`, `None` for history rows |

## 3. Composer (rows 27–34)

| # | ZCode | Oino |
|---|---|---|
| C1 | Placeholder by state: empty session (capabilities hint) / "Ask for follow-up changes" / "Keep typing to queue…" while running (`chat-composer-placeholder`) | 🆕 `PLACEHOLDER` / `PLACEHOLDER_FOLLOWUP` / `PLACEHOLDER_STEER` ("Steer Oino, Alt+Enter queues"), switched in `flush_composer` |
| C2 | Model switch confirms with a toast `Switched from X to Y` (`model-switch-toast`) | 🆕 `SessionPane::set_model` emits `UiEffect::Notify` |
| C3 | Model menu: provider level when there are several providers, model level otherwise; group label + connection badge; selected model checked; capability badge (`Vision`); "Manage models" action (`chat-model-select-menu`) | 🆕 same shape: the provider in use is pinned first and rendered directly, other providers are submenu rows (count badge, chevron) that drill into their model list with an "All providers" back row; one provider skips the level. ⛔ flyout instead of drill-down (one focus-taking popover in GPUI; a nested panel would be clipped by the scroll container) |
| C4 | Thought level control: label + ladder bar, `Ctrl+T` cycles, click opens the list, tooltip switches with the label visibility | ✅ level menu ⌘⇧M, 🆕 `alt-t` steps through `LEVELS` without opening the menu (`SessionPane::cycle_thinking`); no ladder bar (unipi chip shows the level text) |
| C5 | Prompt history: `↑`/`↓`, consecutive duplicates filtered on submit, per-workspace storage | ✅ `Drafts` per session with ↑/↓ recall |
| C6 | Draft survives session switches (`task-scoped-composer-drafts`) | ✅ `Drafts::save` per session id |
| C7 | Mentions `@ / $ #` as coloured tokens, completion panel, Tab accepts (`mention-*`) | ✅ `_oino/get_completions`, token chips, Tab accept |
| C8 | Long paste becomes a text attachment; images become chips (`chat-file-attachments`) | ✅ images pasted/dropped (≤ 16 MiB), other files `@path`; long text stays inline (pi behaviour) |
| C9 | Queue: reorder, send now, edit in place, recall, delete (`chat-message-queue`) | ✅ `QueuePanel` + `_oino/queue_edit` |
| C10 | Steer while running, Enter sends, Alt+Enter queues | ✅ pi semantics (Adapt, row 34) |
| C11 | Queue recall never clobbers a held draft: with text in the composer the item stays queued and the user is told to clear the draft first (`chat-message-queue` edit boundary) | 🆕🆕 `queue_recall` toasts “The composer has a draft — clear it first” and keeps the item (the old code silently merged the two texts) |

## 4. Navigation, palette, settings

| # | ZCode | Oino |
|---|---|---|
| N1 | ⌘K command centre: scope tabs All/Actions/Tasks/Files, prefixes, grouped sections, search history, `Esc` closes, arrows move (`command-center-cmd-k`) | ✅ scopes (⌘1–⌘4 + Tab), prefixes, sections, fuzzy 10k in 0.98 ms |
| N2 | Search history survives restarts and is visible on an empty query | ✅ persisted `palette_history`, 3 chips, `+N` overflow; 🆕 clock icon, ⌥1–⌥3 applies a chip, a per-chip `×` forgets one (persisted, palette stays open) |
| N3 | Sidebar tooltips: 300 ms delay, cancelled on leave, per-trigger timer (`sidebar-tooltips`); task titles use a marquee instead of a tooltip (`task-item-title-overflow`) | ✅ gpui tooltip delay; 🆕 a session title longer than 32 characters gets a whole-title tooltip. ⛔ marquee + right-edge fade mask (GPUI has no clip-path mask; `…` truncation stays) |
| N4 | Settings: full page, section nav, search, bordered groups, empty state (`settings-*`) | ✅ page + search + hub |
| N5 | Model provider rows: enablement state + action on the title line, context-window pill inside the id field (`model-provider-*`) | ✅ settings hub rows; context pill shown on model rows in the menu |
| N6 | Platform primary modifier: Cmd on Apple, Ctrl elsewhere (`platform-primary-keyboard-shortcuts`) | ✅ `cmd-*` / `ctrl-*`, ⌘ labels become `Ctrl+` |
| N7 | Session context menu: Pin persists, Mark as unread persists (blue dot until opened), Copy path of the task file (`task-list-context-menu`) | 🆕🆕 menu has Pin / Mark as unread ↔ Mark read / Copy session id / Copy session path / Reveal / Delete; pins and unread marks persist in `desktop.json` (`AppSettings.sidebar_pinned`/`sidebar_unread`, seeded at startup, `mark_read` on open — done-unseen and manual marks share the dot). “Open in new pane” is not built: §8 row 48 diverges (no multiplexer); ZCode's Archive/Go to config have no Oino equivalent (no archive state, config is settings) |

## 5. Feedback, empty and error states

| # | ZCode | Oino |
|---|---|---|
| F1 | Toasts: bottom stack, 4 s (info) / 6 s (warning) / sticky error, hover pauses, action button, "More" for long text | ✅ same timings, max 3, hover pause, action pill, `More` → result card; 🆕 exit fade |
| F2 | Long status reports open as a card instead of a clipped toast | ✅ `is_long` → result card (GA-24) |
| F3 | Empty states: no messages (hero + greeting + suggestions), no results (palette), no credentials (model menu), empty side pane launcher, empty board lane | ✅ welcome (§5.1), palette empty row, "No models with credentials", side-pane launcher, dashed empty lane |
| F4 | Loading states: streaming marker, tool progress, skeleton for images (`markdown-image-loading-shimmer`) | ✅ spinner + shimmer lines; ⛔ image skeleton (attachments render as chips, not inline images) |
| F5 | Error states: assistant error banner, failed tool row, entitlement/quota failure card (`coding-plan-entitlement-failure-state`) | ✅ error banner + red tool meta; provider errors arrive as assistant text (pi parity) |
| F6 | Unread dot on a session row while it streams in the background (`task-unread-state`) | ✅ `unread` flag + dot, cleared on focus |

## 6. Deferred (with reason)

| # | Item | Why not now |
|---|---|---|
| D1 | Model-menu flyout submenu | GPUI menu is one focus-taking popover; a flyout would be clipped by the `max_h` scroll container on short windows and needs a second focus owner. The drill-down gives the same access with one keyboard path. |
| D2 | Sidebar title marquee (1 s hover) + right-edge fade mask | No `clip-path`/mask in GPUI; a marquee would need a per-row animation task. `…` + tooltip covers the information need. |
| D3 | User-bubble 120 px fold with fade + expand (`v4-user-message-display`) | Long prompts are rare in the pane and the fold needs measured height; the bubble is capped at 80 % width and the transcript virtualises rows. Revisit if long prompts show up. |
| D4 | Thought-level ladder bar, tooltip that switches with the label visibility | unipi A2 chip shows the level word; a ladder bar is a ZCode look element that A2 keeps out. The `alt-t` cycle exists. |
| D5 | Sidebar 1 s hover marquee, remote workspace HoverCard | No remote workspaces in v1 (no multiplexer, A2.2). |
| D6 | Search-history *recent changes / recent tasks* groups in the palette | Oino's palette already lists sessions and files in scope tabs; the history chips cover the recency need. |
| D7 | A separate "Pinned" model group (design.md §4.26) | pi's `enabledModels` / `--models` scope is already applied to `get_available_models`, so the pane's list **is** the scoped (pinned) set — a second group would repeat it. The menu pins the provider in use instead. The scope itself is not exposed to the frontend (`get_available_models` returns models only); filed as a request if the group is wanted anyway. |
| D8 | Model-menu ←/Esc at a provider level closing the whole menu | Fixed in round 2: `popover_key` routes Esc/← through a key-aware close for the model menu — a provider level goes back one level, the top level closes, clicks outside always close. |
| D9 | Sidebar time groups (Today / Yesterday / This week…, `sidebar-task-timeline-groups`) | A2 owns the sidebar: rows group by project (§4.7), which is the unipi look the amendment keeps; a second, time-based grouping would fight it. |
| D10 | Task empty-result state (`task-empty-result-state`) | An Oino session with a sent prompt always has its user row (the welcome shows only for truly empty sessions), and load/restore states already have their own treatment, so the extra “no visible content this turn” hint has no reachable state. |
| D11 | Driving a wheel-scroll detach in a GUI scenario | The automation driver's synthetic `Scroll` event is not accepted by GPUI's `list` (its `should_handle_scroll` hitbox test never matches the synthetic dispatch), so the DETACHED leg of `chat-scroll-bottom-pinning` cannot be driven end-to-end yet; the detach itself is gpui's own unit-tested behaviour (`test_follow_tail_disengages_on_user_scroll`) plus Oino's pause-before-reveal paths (T1). Driver gap, not a product gap. |

## 7. Proof

- Unit: `components::toast::tests` (exit before drop, hover pause, sticky errors) — 2 tests;
  `components::timeline::tests::label_graduates_with_distance` (the four time-label
  graduations); `model::sessions::tests::marked_unread_survives_the_live_map_and_reads_on_open`
  (marks, live-map precedence, read-on-open, persistence ids).
- Gates: `cargo fmt --all --check`, `cargo clippy -p oino-ui --all-targets -- -D warnings
  -W clippy::pedantic`, `cargo test -p oino-ui --lib` (120 passed) in `apps/`.
- Driver scenarios (`zcode` suite, `docs/dev/v1/gui/suites/zcode.json`): `zcode-model-menu`,
  `zcode-thinking-cycle`, `zcode-palette-history`, `zcode-row-actions`, `zcode-session-menu`,
  `zcode-queue-guard` — all PASS (2026-10-29; `ti` was blocked by a Tailscale re-auth check, so
  the round ran on the charter's local fallback: one app instance on a private Xvfb + lavapipe,
  `apps/oino-desktop/scripts/local-app.sh`); screenshots under
  `docs/dev/v1/design/shots/desktop/zcode-*/`. See [gui-driver.md](gui-driver.md) §7c and the
  `[zcode-perf]` commit messages.
