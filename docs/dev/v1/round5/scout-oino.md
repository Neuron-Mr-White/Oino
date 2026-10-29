# Round 5 scout — Oino code map for the 27 owner issues

Read-only scout of `v1-alpha` @ `12622c0` (2026-10-30). Inputs: [CHARTER.md](../CHARTER.md),
[STATUS.md §1–1b](../STATUS.md), [round5-feedback.md](../round5-feedback.md).
All paths relative to the repo root. "shell" = `apps/oino-ui/src/views/shell.rs`, "pane" =
`apps/oino-ui/src/views/session_pane.rs`. The app was **not** run for this scout (static
reading only); the few claims that need a live GUI check are marked ⚠︎.

---

## 1. Issue-by-issue map

| # | Issue | Where the code lives today |
|---|---|---|
| 1 | Model picker must be a modal (search, fusion, full names, pin) | Popover menu, not a modal: `apps/oino-ui/src/components/model_menu.rs:85-199` (drill-down provider → models; the only "pin" is the current provider, :117-119). Opened from the composer model chip (`composer.rs:472`, `PaneMenu::Model`) and the header strip pill (`header.rs:316-334`, `PaneEvent::PickModel`). No search field anywhere in the menu. Fusion lead/sidekick is a separate `/fusion` command dialog owned by `extensions/unipi/fusion` (STATUS §1b.1 "fusion"); the model menu has no fusion entries. Settings → Models has a separate flat picker (`settings_page.rs:692-757`). |
| 2 | Input box too complex; remove "unipi" text; mode switcher; remove permission | `apps/oino-ui/src/components/composer.rs`: context header with project button + menu (:210-292), toolbar with attach (:342), **mode chip = permission-mode menu** (:384-439, `mode-menu` with Ask/Plan/Auto/Full), model chip with context ring + thinking (:440-...). Completion menu section labels show raw extension ids, e.g. `unipi.command-enchantment` (`apps/oino-ui/src/model/composer.rs:367-376`, test :692) — the visible "unipi" text candidate. Mode state comes from the sticky `unipi:workflow:status` topic (`model/composer.rs:291-306`). |
| 3 | Remove the input's footer; context inside the input | Composer shell: `composer.rs:91-...` (`render_composer`), context header :210, toolbar :330-620. The strip **under** the composer is the glance frame: `apps/oino-ui/src/components/meta_row.rs:100-179` (`glance_segments`) + :181 (`render_meta_row`), fed by `setFrame`/`setFooter` requests (`apps/oino-ui/src/model/ui_state.rs:595-610`, `:426-441`). |
| 4 | `+` creates duplicate sessions | **Root cause below (§2.1).** `shell.rs:1392-1398` (`new_session` → `OpenRequest::New`), `shell.rs:678-733` (`open`), sidebar plus button `apps/oino-ui/src/components/sidebar.rs:684-690`, ⌘N binding `apps/oino-ui/src/views/mod.rs:120`. In-pane New: `pane:2139-2146` (`PiCommand::NewSession`), `pane:1681` (`Builtin::New`). |
| 5 | "Automation" sidebar item has no icon | `sidebar.rs:1130-1148` uses `IconName::CalendarClock`. `CalendarClock` is **not embedded**: `apps/oino-ui/src/design/icons.rs:14-140` (`icon_assets!` list) lacks it, and gpui-kit's default set (`gpui-kit-assets-0.8.1/default-icons.txt`) has only `calendar.svg`. `AppIcons` falls back to gpui-kit `Assets` (icons.rs:143-155) which misses the path → blank icon. |
| 6 | Top new-session icon → ZCode-like new chat page (pick project or none) | Today `+`/⌘N immediately opens a session with the active pane's cwd (`shell.rs:1392-1398`); `open_folder` uses a native path prompt (`shell.rs:1410-1429`). No new-chat page exists; the onboarding flow (`apps/oino-ui/src/views/onboarding.rs`) and `startup_request` (`shell.rs:341-356`) are the closest surfaces. Mobile has `new_session(cwd: Option)` (`apps/oino-ui/src/mobile/sessions.rs:411`). |
| 7 | Kanboard must not be a daemon; bring in-process | See §5. |
| 8 | Kanboard cards draggable, unipi-like animation | No drag anywhere: cards are click-select only (`apps/oino-ui/src/components/ous/kanban.rs:224-300`); the only `Drag` in the shell is the sidebar resize (`shell.rs:2074-2086`). Lane strip = `ous/kanban.rs:lane_strip`; board page = `apps/oino-ui/src/views/board.rs:720-731`. |
| 9 | Kanboard modal (new-task) ugly | `board.rs:136` `new_task_dialog` → `modal_frame` (`apps/oino-ui/src/components/dialog.rs:93-...`, layout `modal_with_toolbar_focused` :120-252). Inputs are gpui-kit `Input`. ⚠︎ exact ugliness needs the screenshot/live look. |
| 10 | Kanboard side panel like unipi phone; full page like unipi PC board | Side pane renders the same OUS board inline: `apps/oino-ui/src/components/side_pane.rs:1871-1878` (`inline_detail: true`); full page `board.rs:613-731` with its own toolbar pills (Filter/Display/search/Move, `board.rs:631-648`). The TUI-inherited OUS lane/list widgets (`ous/kanban.rs:135-320`) are the "weird buttons". |
| 11 | Board overlay must show full text | Detail overlay = `board.rs:817-845` (`modal_frame`, body `ous::render(detail)`); lane cards clamp title/detail to 2 lines (`ous/kanban.rs:265`, `:281` — `line_clamp(2)`); modal title truncates (`dialog.rs:199-203`). The extension's detail section sends only the **last** activity line (`extensions/unipi/kanboard/src/panel.rs:104-110`, `task.activity.last()`). ⚠︎ which of these the screenshot meant needs a live check. |
| 12 | Centralize the host ("pc") | See §6 — desktop has **no** host picker at all; what exists are per-page *project* pickers. |
| 13 | Cron icon missing in Automations | Same root cause as #5: `apps/oino-ui/src/views/cron.rs` uses `IconName::CalendarClock` (two sites), not embedded (icons.rs list). |
| 14 | Settings too compact; follow ZCode layout | `apps/oino-ui/src/views/settings_page.rs:1-9` (module doc): 200 px nav (`tokens.rs:574 SETTINGS_NAV=200`) + content max 760 (`tokens.rs:576 SETTINGS_CONTENT=760`, applied `settings_page.rs:1849`); grouped rows of icon tile + label + control (`components/settings.rs`). |
| 15 | Esc twice to stop, with hint | One Esc aborts immediately: `pane:2354-2357` (`"escape" if self.model.is_busy() && text.is_empty() => self.abort(cx)`); `abort` = `pane:1587-1589` (`PiCommand::Abort`). No two-stage state, no hint toast. |
| 16 | Virtualized conversation? | **Yes, already virtualized**: GPUI `list` + `ListState` (`conversation.rs:627`, state created `pane:474-476` with `FollowMode::Tail`). Only visible rows render; patch application splices + remeasures single rows (`pane:875`, `:906-909`, `:1162`). The query rail samples to ≤120 bars (`components/query_rail.rs:23-24`). |
| 17 | unipi markdown colors | Assistant markdown = gpui-component `TextViewState::markdown` with default styling (`pane:1013`, also `components/ous/mod.rs:498`, `views/extensions_page.rs:144`); no markdown palette of our own exists (`design/theme.rs` has none). unipi's colors live in the reference app (`/home/oi/Projects/Personal/archived/unipi`). |
| 18 | Keep bottom-most footer, remove second (`oino`, `0 mem…`) | Bottom-most = native glance line (`meta_row.rs:181-...`). Second = the footer extension's stats strip + brand ("Oino" wordmark segment, rainbow effect `extensions/unipi/footer/src/frame.rs:268-269,424-425`; strip segments `strip.rs:29` turns/time/speed/tokens/cost/compactions/cache). Same removal as #3. |
| 19 | Project tab must label the project | `settings_page.rs:1555-1575`: the scope segmented control is literally `["Global", "Project"]` with no project name; scope is a view-level field (`settings_page.rs:167`, default Global :220). |
| 20 | Settings doesn't need an "Extensions" page | The nav's "Manage extensions" row (`settings_page.rs:643-662`, `SettingsEvent::OpenExtensions`) opens the full Extensions route (`apps/oino-ui/src/views/extensions_page.rs`; sidebar nav item `sidebar.rs:1150-1170`, ⌘3). Plus per-extension settings pages listed under "Extension settings" (`settings_page.rs:586-620`). |
| 21 | Chat bounces repeatedly | **Root cause below (§2.2).** |
| 22 | Does every setting do what it says? (footer no-op) | See §3 (full inventory). Footer root cause: §3.3 / §2.5. |
| 23 | Scrolling with search overlay open scrolls board + search | **Root cause below (§2.3).** |
| 24 | No files view / editor (Zed-like, LSP) | Files side tab is a **read-only** tree + 2000-line viewer (`components/side_pane.rs:1-2`, `:39-40 VIEW_LINES`); no editor, no LSP anywhere in `apps/`. |
| 25 | Unreadable low-contrast text | **Root cause below (§2.4).** |
| 26 | unipi logo wrong (grey icon) | The brand glyph is the pixel-art Unicrab, not a logo: `apps/oino-ui/src/design/brand.rs:5-...` (`CRAB_22`/`CRAB_14`), grey when idle (`components/unicrab.rs:21` `Idle => t.fg_subtle()`, dim :84), rendered in the sidebar brand row (`sidebar.rs:992`). |
| 27 | Refresh button at top of chat is useless | The Sessions column header refresh (`sidebar.rs:679-682`, `refresh_sessions` reloads the session index, `shell.rs:314`). The chat header itself has no refresh (`components/header.rs:275-356` is the full tool set). |

## 2. Bug root causes

### 2.1 #4 — `+` creates duplicate sessions

- Every press runs `Workspace::new_session` (`shell.rs:1392`) → `open(OpenRequest::New { cwd })`.
- `Workspace::open` (`shell.rs:678-733`) **dedupes only `OpenRequest::Resume`** (by session-file path, :690-700). `New` is unconditional: it saves the cwd, spawns `host.open(request)`, and `add_pane` (:724-733) pushes another live pane.
- So N presses of `+` (or ⌘N, `views/mod.rs:120`) = N new core sessions and N panes in the workspace/sessions list; nothing checks "is there already an empty/untitled session for this cwd?".
- Secondary path: the pane's own `Builtin::New` → `PiCommand::NewSession` (`pane:1681`, `:2139`) replaces the pane's runtime, but the shell-side `+` is the one that duplicates.

**Fix shape:** in `open()`, when `request` is `New`, first look for an existing pane whose session is empty (no user messages) with the same cwd and activate it instead; or make `new_session` focus an existing untitled session per cwd.

### 2.2 #21 — chat bouncing

Verified facts:
- The transcript is a GPUI `list` over `ListState` (`conversation.rs:627`); heights are estimated from content and corrected via `remeasure_items` after patches (`pane:875`, `pane:906-909`, `pane:1162`; also `components/keys.rs:110,256`, `components/timeline/mod.rs:519`, `components/timeline/cards.rs:214`).
- Streaming patches are coalesced on a debounce (`pane:794` `coalesce(std::mem::take(&mut batch))`, `model/coalesce.rs`), each batch splicing rows and remeasuring the changed ones.
- `FollowMode::Tail` is (re)set at pane creation (`pane:476`), after `absorb`/history load (`pane:1304`) and on submit (`pane:1582`); the jump pill re-engages it (`conversation.rs:558-560`).
- Per-frame UI work continues while a run streams: scoped ticker re-renders the pane each frame for spinners/shimmer (`pane render` → `Ticker::scoped`, `conversation.rs:607-612`; `design/ticker.rs`), and markdown rows re-layout as text grows (`pane:1013` creates a `TextViewState` per row, refreshed by row id).

Likely root cause (inference, consistent with the known round-4 gap "D11 synthetic wheel-scroll not accepted by GPUI's list hitbox" and the 4-pane perf budget): during a streaming run, repeated splice+remeasure changes the estimated total height between frames while `FollowMode::Tail` re-pins the bottom each frame; when a row's estimated height differs from its laid-out height (markdown, tool cards, thinking blocks collapsing/expanding via `thinking_blocks`, `timeline/mod.rs:566`), the correction moves the anchor and the whole viewport visibly jumps — experienced as bouncing. The shimmer/ticker re-render amplifies it (a re-layout every frame while heights are still settling). Zed's agent panel avoids this by anchoring to the bottom item identity rather than a px offset and by not re-measuring mid-stream. ⚠︎ needs a live repro (stream a long faux response on Xvfb and watch `logical_scroll_top`) to confirm which of splice-vs-remeasure dominates.

### 2.3 #23 — scroll leaks through the search overlay over the kanboard

Verified facts:
- The board page's search is an inline toolbar input (`board.rs:485-494`, rendered at :644); `/` focuses it (`board.rs:291-296`).
- Scrollable surfaces on the board page: the lanes container `.overflow_x_scroll()` (`ous/kanban.rs:442`) and the toolbar/nav containers (`sidebar.rs`, `settings_page.rs:669` etc. use `overflow_y_scroll`). GPUI dispatches scroll-wheel events to **every** scrollable whose hitbox contains the pointer and is not occluded; none of the board elements call `on_scroll_wheel` / `stop_propagation` (grep: no `on_scroll_wheel` anywhere in `apps/oino-ui`).
- Overlays (task detail, new-task, palettes) use `modal_frame`/scrim with `.occlude()` (`dialog.rs:137`), which blocks hit-tested mouse events but the app still has overlays that are *not* scrims (completion menu, popovers in `components/menu.rs:279` with `max_h(...).overflow_y_scroll()`), and menus stop *keyboard* events only (`menu.rs:302,309,346,497`); `palette.rs` stops keys (:189,198) but has no wheel handler either.

Likely root cause: wheel events are not consumed by the topmost overlay/field — the search input (a gpui-kit `Input`, internally scrollable text area) and the underlying lanes container both react because nothing calls `stop_propagation` on `ScrollWheel` and there is no scroll-containment rule in the app. The fix is one app-level rule: the topmost focused/hovered scrollable consumes the wheel (`on_scroll_wheel` + `stop_propagation` on the find bar/palette/popovers, and/or `.occlude()` on the overlay wrapper that includes wheel). ⚠︎ confirm live which overlay the owner meant ([image 3] unavailable); the two candidates are the board's `/` search with the detail overlay and the ⌘K palette over the board.

### 2.4 #25 — unreadable low-contrast text

Token-level math (WCAG contrast, `apps/oino-ui/src/design/tokens.rs`):

| Token pair | Values | Ratio |
|---|---|---|
| `fg_subtlest` on `window` (dark) | `#6E655D` on `#0A0908` (tokens.rs:310-311, :262) | **3.49:1** — fails AA 4.5:1 |
| `fg_subtlest` on `card` (dark) | on `#141211` (:266) | **3.28:1** |
| `idle_task` on `card` (dark) | `#5E554D` on `#141211` (:291) | **2.56:1** |
| `attention_fg` on `attention_surface` (dark) | `#8A5A00` on `#F0A818`-ish blend (:287-289) | ≈3.7:1 |
| `primary_fg` on `primary` (both themes) | `#181818` on `#F03018` (FAB "New task", board fab `board.rs:873-902`) | 4.35:1 (large text only) |

`fg_subtlest` is the default for captions/meta/build-info/empty states across the app (grep hits in `sidebar.rs:1204` build info, `header.rs:253,319` strip, `board.rs` empty states, `settings_page.rs` rows). Dark-theme `fg_subtlest` was derived from the unipi OLED palette without a contrast pass (light theme comments say "darkened where text needs 4.5:1", tokens.rs:136 — dark has no such note). Fix: raise dark `fg_subtlest` to ≥ `#8A8078`-level (the current `fg_subtle` value passes at 4.9:1) or re-map caption roles to `fg_subtle`; audit `idle_task` and the red-FAB label. ⚠︎ [image 6] unavailable — the exact instance the owner saw is unknown; the token audit covers all of them.

### 2.5 #22 example — "disabling the footer setting did nothing visible"

The wiring works; the **visual result is a no-op by design of the fallback**:
- Toggle writes `footer.enabled=false` (`settings_page.rs:433-452` → `_oino/set_extension_setting`); the registry publishes a `SettingsChange` (`crates/oino-core/src/extensions/settings.rs:577-584`), services forward it (`crates/oino-core/src/services.rs:745-769`, `deliver_settings_change` :241-303), the footer handles `OinoSettingsChanged` (`extensions/unipi/footer/src/lib.rs:453-463`) → `start()` → `paint()` sees `enabled=false` → `clear_all` (:231-234, :179-182) → `setFrame`/`setFooter` clears, which the desktop honors (`model/ui_state.rs` tests :800-828).
- But the desktop then re-fills the same glance line with its **own fallback segments** whenever the extension's line is empty: "Without a footer extension the session's own numbers keep the glance useful" pushes `ctx N%`, `↑in ↓out`, `$cost` (`meta_row.rs:122-153`). So the row under the composer never disappears — it switches from unipi segments to app segments. To the user: "the setting did nothing".
- Compounding factors: (a) with the **Project** scope selected the write lands in the project file and a stale project value shadows the global one on later reads (`extensions/sdk/unipi-kit/src/settings/values.rs:203-215` deep-merge); (b) `SettingsView::write` silently returns when no session backend exists (`settings_page.rs:433-435`) — no toast, unlike `set_core` (:279-285).

## 3. Settings inventory (issue #22)

Three owners: **app** (`desktop.json`, `AppSettings`, `apps/oino-ui/src/host.rs:235-289`), **core**
(`settings.json` via `_oino/set_setting`, read by `oino-core`), **extension** (`extensionSettings.<ns>`
via `_oino/set_extension_setting`, schema from each component's `settings_namespace`). The
Settings page needs an open session for extension namespaces (`settings_page.rs:229-236`,
silent no-op writes :433-435).

### 3.1 App settings (Settings → Appearance / Unicrab / General)

| Setting | Written at | Effect in desktop today | Evidence |
|---|---|---|---|
| theme (System/Light/Dark) | settings_page.rs:1104-1123 | **yes** (live re-theme) | `design/theme.rs` reads `ThemePref`; 121 uses |
| interface size `ui_font_size` | :1124-1146 | **yes** | `theme.rs:454` clamp_ui |
| code size `code_font_size` | :1134-1141 | **yes** | `theme.rs:455` clamp_code |
| **`sidebar_compact`** | :1147-1153 | **NO — dead**. No consumer outside the settings row | 0 reads in views/components/model |
| reduce_motion | :1162-1181 | **yes** | `design/motion.rs`, 15 uses |
| tool_density | :1184-1208 | **yes** | `timeline/mod.rs:161` |
| thinking_blocks | :1209-... | **yes** | `timeline/mod.rs:566` |
| **`native_wayland`** | :1315-1316 | **yes, but only at next launch** | `apps/oino-desktop/src/main.rs:144` (platform pick at startup) |
| mascot_window / mascot_size | :1294-1313 | **yes** | `views/mascot.rs:243`, shell opener |
| reopen_last | :817-820 | **yes** | `shell.rs:343` startup_request |
| default folder `default_cwd` | :805-815 | **yes** | `shell.rs:355` startup cwd |
| **`hide_thinking`** | mobile only (`mobile/settings.rs:560`) | **NO on desktop — dead** (no desktop reader, not in desktop settings page) | grep: host.rs + mobile only |
| **`compact`** (AppSettings) | nothing in the UI | **NO — dead** (no reader, no toggle) | host.rs:241 only |
| **`expand_tools`** | no toggle in UI | **yes** (read) but **unreachable** — OR'd into tool_density==Advanced (`timeline/mod.rs:161`); nothing sets it to true | host.rs:243 |
| `list_width` | sidebar drag (`shell.rs:2074-2086`) | **yes** | sidebar width render |
| `recent_cwds`, `palette_history`, `sidebar_pinned/unread`, `last_session*`, `onboarded` | internal state | **yes** (bookkeeping) | shell.rs persist paths |

### 3.2 Core settings (Settings → General / Sessions; `_oino/set_setting`, global)

| Setting | Effect | Evidence |
|---|---|---|
| defaultProvider / defaultModel | **yes**, applies to **new sessions** (toast says so, `settings_page.rs:293`) | `:751-752`; core model registry reads them at session create |
| defaultThinkingLevel | **yes**, new sessions | `:775-795` |
| sessionDir | **yes**, new sessions; Reveal button works | `:869-894`; `crates/oino-session` layout |
| steeringMode / followUpMode | **yes** (live, agent queue modes) | `crates/oino-core/src/session/mod.rs:323-324,498-501` |
| compaction.enabled | **yes** | `crates/oino-core/src/session/compaction_flow.rs:213` |
| retry.enabled | **yes** | core retry path (STATUS §3 "AgentSession retry ✅") |
| disabledExtensions (Extensions page switches) | **yes, new sessions** | `extensions_page.rs:36-58`; `host.rs:362`; wasm-host skips them (`crates/oino-wasm-host/src/core_bridge.rs:1588`) |

### 3.3 Extension settings (Settings → "Extension settings" pages)

Mechanics: every component registers a schema (`cx.settings_namespace`, e.g. `extensions/unipi/footer/src/lib.rs:399`); every write publishes the sticky `oino:settings:<ns>` topic **and** an `oino_settings_changed` event to the owner (`crates/oino-core/src/services.rs:241-260`, `extensions/settings.rs:577-584`). 23 of 25 packages subscribe to `OinoSettingsChanged` and refresh their `Store` (`extensions/sdk/unipi-kit/src/settings/values.rs:185-201`); `btw` has no settings; `mcp`'s namespace is **action-only** (`extensions/unipi/mcp/src/lib.rs:99-126`).

So the wiring is sound; the caveats that produce "did nothing" reports:

1. **footer.enabled** — visually a no-op because of the native fallback glance line (§2.5). The other footer fields (strip.*, badges.*, processLine, iconStyle, colorMode, rainbow — `extensions/unipi/footer/src/settings.rs:182-218`) do change what the extension emits and the desktop renders (meta_row) → **yes**.
2. **core hints `header` ("Unicrab start screen")** — partial: the setHeader start card disappears (`hints/render.rs:192-238`, rendered `conversation.rs:275-...`), but the desktop's own native welcome (crab + build line, `conversation.rs:162-240`) still shows for empty sessions → **partially**.
3. **Scope trap** — Project-scope writes shadow Global (deep-merge, `values.rs`), so toggling Global while a stale Project value exists does nothing → looks dead.
4. **No-session trap** — namespace pages only load/validate with a live session backend (`settings_page.rs:229-236`, silent return :433-435).
5. **Delivery target** — `deliver_settings_change` runs the owner's handler with `cx.session_id = any_root_session()` — one arbitrary session (`services.rs:277`, `:358`); state is per-session in extensions (`footer` SessionMap), so other panes catch up only on their next tick (≤1 s for footer). Latency, not a no-op.

Per-namespace field inventory (from each `settings.rs` / `Field::…` declarations), effect = does the component change behaviour/rendering when the field changes (all via the mechanism above):

| Namespace | Fields | Effect |
|---|---|---|
| footer | enabled, colorMode, iconStyle, rainbow, processLine, strip.{turns,time,speed,tokens,cost,compactions,cache}, badges.{mode,planPermission,fusion,kanboard} | yes (see caveat 1) |
| core (Hints) | enabled, header, crab | yes; `header` partial (caveat 2) |
| workflow (Permissions page) | mode, defaultMode, jevConfidence, rulesCount(action), debug | yes (permission gating + plan mode, `unipi-workflow` scenario) |
| ask-user | enabled, notifyOnAsk, maxQuestions, digitAdvance, helpLine | yes (tool behaviour) |
| subagents | enabled, defaultModel, defaultThinking, maxConcurrent, manageAgents(action) | yes |
| background-tasks | enabled, notifyOnCompletion, triggerOnCompletion, maxFinishedTasks, maxOutputBytes | yes |
| memory | write, autoStartDaemon, mempalaceAutoUpdate, debug, mempalaceBin | yes (STATUS §1b.5 verified) |
| mcp | actions only (configure/add/sync/reload) | yes (actions run commands) |
| notify | native.{enabled,suppressWhenFocused}, recap.*, silenceAfterInput.*, renotify.*, gotify.*, telegram.*, ntfy.* | yes (needs OS side; verified in `unipi-notify` scenario) |
| kanboard | reminders, openBrowser, requireAuth, keepToken, open/stopDaemon/rotateToken (actions) | yes (daemon flags applied at next `serve` spawn, `extensions/unipi/kanboard/src/lib.rs:589-640`) |
| long-horizon | judge.enabled, judge.threshold, defaultMode, goalProgress | yes (arbiter continuation) |
| dream | enabled, minSessions, minGapHours, maxRuntimeMin | yes |
| compactor | method, piCompact, thresholdPercent, notify, smartKeepTail, summaryBudgetTokens, debug | yes (STATUS §1b.1b item 2) |
| watchdog | enabled, watchBash, watchBgTasks, debug, sessionCaps.summaryTurn | yes |
| skill-registry | proxy, states(action), exposure.threshold, exposure.maxSkills, exposure.recheck | yes |
| web-api | providers.model.model, smartFetch.removeImages, cache.clear(action) | yes |
| utility | rename.auto, rename.model, rename.now(action), answer.hint, answer.port, attachments.{enabled,preview}, debug, generate.*, edit.enabled, recognize.*, keys.{openrouter,fal} | yes |
| updater | (fields via About page: update check) | yes (never installs by design, STATUS §4) |
| fusion | startup.thinking, presets.editor(action) | yes (R4: persists via `control.set-default-model`) |
| info-screen | bootTimeoutMs, groupOrder(order) | yes (dashboard layout) |
| cron | concurrency, missedWindowS, retentionRuns, retentionDays, defaultTimeLimitS, paused | yes (scheduler crate reads them, `crates/oino-cron`) |
| input-shortcuts | (no fields) | n/a |
| btw | (no namespace) | n/a |

## 4. Bundled extension inventory (D1/D3 classification)

24 unipi packages + `unipi.cron` (Oino-native) + `oino.browser` + 8 providers. Load order
`extensions/sdk/unipi-kit/src/suite.rs` (`LOAD_ORDER`). Desktop surfaces per STATUS §1b.1.

**Harness** (affects agent behaviour; keep — D3 harness parity):
workflow (permission modes, plan mode, approval gating), ask-user (`ask_user` tool + forms/ACP elicitation), subagents (`run_subagent`, profiles, arbiter), background-tasks (`bg_run`, adoption, follow-up notifications), memory (`memory_*` tools, recall, side-save, MemPalace), web-api (`web_search`/fetch tools), mcp (mcp.json servers, tool surface), compactor (compaction methods incl. LLM+previous-summary), long-horizon (goal continuation via turn arbiter, `/goal`/`/ralph`/`/graph` modes), watchdog (stuck-tool detection, kill, session caps), dream (nested-agent reflection runs), fusion (lead/sidekick model pairs, startup pair persistence), skill-registry (skill exposure control), kanboard (task tools + monitor nudges — harness half), cron (scheduler, `cron_*` tools — Oino-native), utility (auto-rename, `/answer` server, image tools/attachments, `/doctor`/`/cleanup`/`/move`), oino.browser (19 `browser_*` tools, approval, prompt section), providers (chat itself).

**GUI-surfaced features** (function lives in the extension, value is the desktop rendering; keep but re-skin per D2):
kanboard (board panel + native Board page + side pane), btw (side-chat child session panel), info-screen (`/info` dashboard), notify (native OS notifications + settings page), updater (`/readme`, `/changelog`, About page), app-bridge (control-centre side tab; STATUS §4 notes "no native GPUI consumer yet" for `service_call`), core (work tray ↓, hint widgets, start header card).

**TUI-only / presentational** (D1 removal candidates — little or no agent-behaviour effect, exist to reproduce unipi's TUI chrome):
- **footer** — computes glance/strip/process segments for an editor status line (its own schema says "Off leaves the plain pi editor"). All of its data (model, ctx%, cost, tokens, git branch) already exists natively (`header.rs strip_text`, `meta_row.rs` fallback, status topics). Remove extension + the meta_row extension paths.
- **command-enchantment** — enriches the `/` completion menu (GUI eye-candy; core completions work without it; its package-colour tags are the "unipi" look D2 drops).
- **input-shortcuts** — alt+h/m/p editor intents; the desktop binds the same chords natively already (`shell.rs`/composer chords; scenario `unipi-input-shortcuts` proves the extension path, but ZCode-style switcher (#2) supersedes it).
- **app-bridge** — mostly presentational control-centre; its unique capability (`_oino/service_call` mirroring to the phone) is dead until `oino serve` (STATUS known issue 4). Merge any live bits (BTW/dream/Info tabs) into native views, then remove.
- **core's hint/header halves** — hints + start screen are unipi TUI chrome; D2 wants ZCode welcome. The work-tray half is GUI-useful (keep that, drop hints/header render).
- **updater's `/readme`+`/changelog`** are conveniences, not harness — could fold into Settings → About.

Reasons recorded per row above; nothing else in `extensions/` is TUI-only (providers/browser/cron are harness by definition).

## 5. Kanboard daemon (#7)

Today's architecture:

- **Binary/crate**: `crates/oino-kanboard` (library + `main.rs` CLI). `oino-kanboard serve` runs an **axum HTTP server** (`src/serve/mod.rs:1-56`): JSON API (`serve/api.rs`, 2.1k lines), SSE (`serve/events.rs`), file-watch (`notify`), idle shutdown (`--idle-min`, default 10), optional token auth (`serve/auth.rs`), UNI-57 undo (`serve/undo.rs`), embedded UI. Single-instance via `daemon.lock` + `daemon.json` (pid/port/token, `src/daemon.rs:23-46`). Board state is plain files under `~/.oino/state/unipi.kanboard` (or `$OINO_KANBOARD_HOME`, `src/store.rs:15-42`).
- **Who starts it**: the `unipi.kanboard` extension, not the app. `ensure`-style spawn: `extensions/unipi/kanboard/src/lib.rs:605-655` — `process::Child::spawn` of `self.binary()` (= `$OINO_KANBOARD_BIN`, else the bundled `oino-kanboard`, `lib.rs:160-169`) with `serve --host --port --idle-min [--require-auth]`, detached, then a 3 s health poll (`/api/health`, `lib.rs:558`). Triggered by `/kanboard onboard|open` and when the board panel opens; the desktop Board page shows "Starting the kanboard daemon for this project." meanwhile (`board.rs:802-805`).
- **Who talks to it**: the extension runs **every** command through the CLI binary (`extensions/unipi/kanboard/src/cli.rs:1-11`, host `process.exec`, 10 s timeout) and follows the SSE stream (`follow_sse`, `lib.rs:1274-1283`) to republish the board panel; the desktop Board view renders that panel (`board.rs:677-731`), hence the round-3 "board refresh can lag tens of seconds" gap (STATUS §1b.10 gap 17). The TUI renders the same panel generically.
- **Key fact for #7**: "the extension and the daemon call the same library" (`crates/oino-kanboard/src/lib.rs:3-10`). All board logic (model, commands, transitions, locks, atomic writes) is already an in-process Rust library.

What "in-process from the start" requires:

1. **A host-side owner**: instantiate the kanboard service inside `oino-desktop` (or `oino-core`'s LocalServices) — no detached process, no `daemon.json`/lock, lifetime = app lifetime (idle shutdown and cross-process singleton become unnecessary for the GUI path).
2. **A transport decision**: either (a) host the axum router in-process on a private port (least code: extension keeps HTTP+SSE, desktop keeps SSE-fed panel), or (b) skip HTTP: expose commands + change events through a WIT host service / bus topic (removes the spawn entirely and fixes the refresh-latency gap: the desktop subscribes to a topic instead of SSE-through-the-extension).
3. **Extension path**: replace `process.exec` CLI calls with the new service (or keep them as a fallback for the `oino` CLI / scripts, which still link the same library).
4. **Keep the CLI**: `oino-kanboard` (add/list/move/…) remains for agent scripts and `cron`; against an in-process app board it would need either the file locks it already uses (works today, `store.rs`) or an RPC to the app.
5. Packaging: GA-41 already ships the binary beside the desktop app (`apps/oino-desktop` packaging); in-process drops that requirement for the GUI but not for CLI users.

## 6. Host pickers (#12)

- **Desktop: there is no host picker anywhere.** No host list, no "pc" label, no remote attach — `oino serve`/`--remote` don't exist (STATUS known issue 4); the desktop embeds core in-process (`apps/oino-ui/src/host.rs:2-4`). Searches for `host picker|Current PC|hostname|HostEntry` over `apps/oino-ui/src` (desktop halves) return nothing.
- What the owner perceives as per-page pickers are **project** pickers:
  - Board page header project menu (`board.rs:344-365`, `BoardMenu::Project`, switches the board's cwd);
  - Composer context-header project button + menu (`composer.rs:217-292`, `PaneMenu::Project`);
  - Sessions page groups by cwd with per-group "new session here" (`sidebar.rs:740-...`, `new_session_in`);
  - Onboarding / `open_folder` native folder prompt (`shell.rs:1410-1429`).
- **Mobile** has the only real host picker: `apps/oino-ui/src/mobile/hosts.rs` (M10 "This phone" + paired desktops, `mobile.json`), explicitly gated on the missing `oino serve` (`hosts.rs:6`).
- So #12 is a green-field build (one host switcher in sidebar/footer/header reading "Current PC" when the host list is empty), plus folding the three project pickers into it or leaving project pickers per-page by design.

## 7. Not determined here (needs a live run)

- Which exact overlay the owner meant in #23, and the exact element of #9/#11 ([images 2,3] unavailable).
- A live confirmation of the #21 bounce mechanism (faux stream on a private Xvfb via `apps/oino-desktop/scripts/local-app.sh start faux -`; never `DISPLAY=:0`, per CHARTER).
- The exact segment the owner read as "0 mem…" in #18 (the footer strip's cache/token segments are the likely source; need a rendered capture to be sure).
