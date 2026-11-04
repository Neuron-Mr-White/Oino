# owner-fix-3 — strict UI/UX QA (real app, isolated rig)

**Verdict: NOT SHIP.** One P0: opening the **Tree** tab on a long chat crashes the host with a stack overflow, again on every restart. Four commits later the host was in safe mode (extensions off) and every chat was offline. 9 of 16 criteria pass. 5 pass only in part. 2 fail: Tree (5) and stepper (12).

## Rig

- **Build:** `~/oino-qa-release/owner-fix-3/unipi-oino_amd64-7b8e44a3.deb`. I checked its sha256 `ba1b9f3b…05804993` against `.deb.sha256`.
- **Isolation:** the .deb file tree ran under the integrate rig's `QA_HOME=~/oino-qa-release/owner-fix-3/qa`, on display `:181` with WebDriver on port `4481`. I never touched `~/.oino`, `~/.unipi` or the owner's app.
- **Model:** the fake model `scripts/qa/phone-fakellm.py` on `127.0.0.1:11434`. Its log is at `~/oino-qa-release/owner-fix-3/myqa/fakellm.log`.
- **Chats:** copies of the owner's chats. The main test chat is "Frontend Whitelabeling Strategy" (`01a19f58…`, 3918 session entries).
- **My screenshots:** `~/oino-qa-release/owner-fix-3/qa/shots/q-*.png` (called `S/` below). The integrate shots are in `docs/dev/v2/design/shots/owner-fix-3/` (called `R/` below).
- **Host crash log:** excerpt at `~/oino-qa-release/owner-fix-3/myqa/host-crash-excerpt.log`, plus `start-history.json`.
- **Cleanup:** `rig.sh down` done, and the fake model and the leaked rig dbus-daemon are killed.

## Acceptance criteria

| # | Criterion | Result | Evidence |
|---|---|---|---|
| 1 | Sidebar list < 1 s | **PASS** | After a reload, the first row appeared at 339 / 464 / 490 / 445 ms (`sbtime.mjs`). On a cold app launch with a host restart (`myqa/cold.mjs`, 4 runs), it appeared 470–929 ms after navigation start, or 0.8–1.3 s from process spawn. |
| 2 | Running graph workers / subagents / waking bg task → Working + working line | **PASS** | **Graph:** `graphwork` gave the sidebar row "Working" and the line "Waiting on graph: 2 running, 1 starting · 2 subagents working" (`S/q-graphwork-6s.png`). **Subagents + bg:** a `subagents` chat gave "Waiting on bg: Watch the test suite · subagent working" with the row still Working (`S/q-sub1.png`). The capsule's count is wrong (P2-2). |
| 3 | Git tab only git, consistent padding; header Graph button removed; capsule segments open their own pane part | **PASS** (with defects) | **Header:** only ⋯ / Terminal / Side pane (eval of the header buttons). **Capsule Graph segment:** opens the Graph tab (`S/q-cap-lh.png`). **Capsule "N running":** opens the Running section (`S/q-cap-running.png`). **Git-only pane:** `R/rig-git-only-pane.png`. I couldn't drive Git live because the copied project folder doesn't exist in the QA home. The capsule overlaps chat content (P2-1). |
| 4 | LH diagram: elk, readable non-overlapping edges, whole-canvas zoom, distinct Fit icon, Full window, errors toggle with no dangling edges, no duplicated title; replaced-but-running worker visible | **PARTIAL** | **Pass:** orthogonal elk edges with no overlaps, zoom scales the whole canvas, the Fit (focus) icon differs from Full window (expand), Full window works, Errors off shows "7 errored / failed / replaced steps hidden" with no dangling edges, and the title appears once (`S/q-diagram*.png`, `S/q-diagram-full.png`). **Fail:** the first view isn't fitted, so nodes are cut off right and bottom. At Fit, a 23-node graph in the pane has about 5 px text (`S/q-diagram.png`, `S/q-fit-vs-zoom.png`), and the zoom controls sit on top of nodes (P2-5). **Not verified live:** the orphaned "Still running" state. Stop settles within the same second (`R/rig-lh-stop-settles.png`), so it only exists in unit tests (`lhGraph.test`, harness `a_stopped_worker_that_hangs…`). |
| 5 | Tree tab loads | **FAIL (P0)** | Opening Tree on the whitelabel chat crashed the host ("thread 'tokio-rt-worker' has overflowed its stack"), 10 times in about 5 min, including in safe mode. The tab then shows "Couldn't load the tree: [object Object]" and the chat shows "Lost the connection · reconnecting…" (`S/q-tree.png`, `S/q-after-crash.png`, `S/q-tree2.png`). See P0-1. |
| 6 | Fisheye turn rail + hover list + click jump | **PASS** (with defects) | Hovering sets per-tick `--s` to 1.8–3.7 and opens the list. Clicking "Check if agent stucked or failed" scrolled the chat to that turn (`S/q-rail-hover.png`, `S/q-rail-jump.png`). See P2-6 and P2-7. |
| 7 | Compact vs normal visibly different, title never hidden | **PASS** | Rows are 28 px in compact and 44 px in normal (eval). With a Working badge the title still shows, truncated ("Frontend Whitelabelin…") (`S/q-compact-crop.png`, `R/rig-sidebar-normal.png` / `-compact.png`). |
| 8 | Subagent tab renders like main chat, follows the bottom | **PASS** | Same parts as the main chat: System row, user bubble, "Working for 6m 26s" fold and bash row, with a Running chip and Stop (`S/q-subagent-tab.png`). Bottom lock comes from the shared Timeline (3ceb9637). I didn't see it follow live output, because the fake worker only sleeps. |
| 9 | Project menu Settings / Archive / Dream; session Dream | **PASS** | **Session menu:** includes Dream (`S/q-session-menu.png`). **Project menu:** Project settings / Archive / Dream (`R/rig-project-menu.png`). On a project whose only chat is running, Archive is missing (`S/q-project-menu.png`, P3-3). |
| 10 | Session Info = this session only; ZCode-like Usage page | **PARTIAL** | **Usage page:** passes (`S/q-usage.png`: 7/30/90 days, stat strip, daily bars, by model, by project). **Info page:** the Models rows and Duration are computed from the *loaded window* only, so they contradict the session's own totals. Cost shows $133.42, but the Models rows show claude-opus $4.36 and 11 replies, while Usage shows claude-opus $127 and cloud-agent $133. Duration reads 3h 27m for a chat that started at 02:59 (`S/q-info.png`). See P1-1. |
| 11 | Neutral note when subagent_general is disabled | **PASS** | Turning it off shows "Work that names no profile now goes to subagent_explore." (`S/q-profiles-off.png`). I turned it back on. |
| 12 | Consistent number stepper | **FAIL** | **Max running at once:** the −/+ glyphs sit about 6 px below centre (`S/q-stepper-crop.png`). The step buttons are 44 px tall inside a 32 px box. **Inconsistency:** the app has 3 stepper implementations plus plain number inputs (P2-3). |
| 13 | Tray opens main window | **PASS (code review only)** | `desktop.rs:79-101` puts "Open Unipi" first. `opens_main` (`:271-287`) treats a left click or double click as open. `show_main` (`:452`) shows, unminimizes and focuses the window, and `ensure_main` (`:117`) rebuilds a destroyed one. I couldn't test it in Xvfb, which has no StatusNotifier host. On GNOME a single click still opens the menu (P3-10). |
| 14 | Windows no-console fix | **PASS (code review only)** | `no_window.rs` adds `CREATE_NO_WINDOW` to std and tokio commands. It is used by `host_supervisor.rs:183` (the host child), `about.rs:149`, `pair_phone.rs:101,208`, `sharing.rs:426`, `shell_path.rs:179` and `push.rs:573`. `main.rs:2` sets `windows_subsystem="windows"`. The host is a console-subsystem process started hidden, so its git and shell children share its hidden console. Not run on Windows. |
| 15 | Queue send: queued → steer → send now | **PASS** | While `sleep 900` ran, I queued 2 notes. "Steer now" on the second moved it to the top as STEER, and its button became "Send now (interrupts)". Clicking that stopped the turn, and both notes were then sent in order (rail shows: slow please / second queued note / first queued note). (`S/q-q1.png`, `S/q-q2.png`, `S/q-q3.png`) |
| 16 | Running tab: subagents and bg tasks in the same style; click opens subagent tab / bg terminal tab | **PARTIAL** | **Session modal › Running:** passes. Subagents and Background tasks share one row style. A row click opened the "Graph · scan-app-85591" subagent tab and the "Watch the test suite" live terminal tab (`S/q-run-modal.png`, `S/q-subagent-tab.png`, `S/q-bgtab.png`). **Side-pane Running section** (where the capsule's "N running" goes): it uses a different style, collapses bg tasks into one "Background tasks · 1 running · 1" row, and a row click opens the modal instead of the tab (`S/q-run-pane.png`, `S/q-pane-running-click.png`). See P1-2. |

## Defects

### P0

**P0-1: the Tree tab crashes the host, and it keeps crashing on restart.**

- **Repro:**
  1. Open "Frontend Whitelabeling Strategy" (3918 entries).
  2. Side pane → + → Tree.
- **Expected:** the tree loads, or the tab shows a readable error.
- **Actual:**
  - Within about 5 s the host aborts: `thread 'tokio-rt-worker' has overflowed its stack / fatal runtime error: stack overflow, aborting`.
  - The app restarts it, reopens the chat and the tab asks again. That happened 10 times in 10:22:50–10:27:35 UTC.
  - `start-history.json` lists 4 fast-fails, and the host log shows safe mode ("extensions are off after 6 failed host starts").
  - The crash also happens in safe mode, so the cause is in core, not an extension.
  - Every chat shows "Lost the connection · reconnecting…", and the composer loses its Auto/Plan chips.
  - Closing the tab stops the loop.
  - The owner's real copy of this chat should hit the same crash on his own host. That is inferred, not tested.
- **Root cause:** `crates/oino-session/src/manager/tree.rs:32-45`. `impl Serialize for SessionTreeNode` serializes `children` recursively, one level per entry, while `build` and `Drop` already avoid recursion. `crates/oino-client/src/in_process.rs:494-496` then calls `json!({"tree": tree})` on a 2 MB tokio worker stack.
- **App side:** `apps/unipi/src/components/panes/tabs/TreeTab.tsx:57-70` already says that get_tree "never answers" on long chats. It still sends get_tree on every open, with a 2.5 s head start. "Never answers" was really the host crashing.
- **Fix direction:** don't call `get_tree` from the app; the flat `get_entries` path exists. Also make the host serialization flat or iterative and add a 5k-deep chain test.

### P1

**P1-1: Session Info › Session mixes whole-session totals with loaded-window numbers.**

- **Repro:** whitelabel chat → ⋯ → Session info → Info.
- **Actual:**
  - Cost is $133.42 and Tokens 264.3M, which matches Settings › Usage (cloud-agent $133).
  - The Models rows show only 11 claude replies and $4.36, against Usage's $127 and 1267 replies.
  - Duration reads 3h 27m.
- **Root cause:** `components/session/InfoTab.tsx:88-89`. `sessionModelUsage(state().entries)` and `sessionSpanMs(state().entries)` read the paged timeline window, not the host stats.
- **Expected:** every number on the page comes from the same full-session source.

**P1-2: the side-pane Running section is a second, different Running UI.**

- **Repro:** close the side pane, then click the capsule "2 running".
- **Actual:**
  - Rows are monospace with a dot and the profile id ("Graph · scan-app-85591 / subagent_general").
  - All bg tasks collapse into a single "Background tasks · 1 running · 1" row.
  - The header reads "1 agent · 1 tray · 2", which is jargon, and it wraps onto 3 lines next to a wrapping "Open in Session".
  - A row click opens the session modal, not the subagent tab or bg terminal tab (`S/q-run-pane.png`, `S/q-cap-running.png`).
- **Expected:** the shared `RunRows`/`WorkRow` style, one row per bg task, and a row click opens its tab.
- **Code:** `components/status/StatusPane.tsx` running section (`data-testid=running-summary` / `running-row`) versus `components/session/RunRows.tsx`.

### P2

1. **The capsule floats over chat content.** At the top right of the timeline it covers user bubbles and the Mode/Memory card chevrons. Example: "Check if agent stucked or [failed]" is cut off under "Graph 15/23 · 1 running" (`S/q-rail-jump.png`, `S/q-capsule.png`). Code: `Workspace.tsx:222-225` and the `.status-capsule` position in `status.css`. The capsule needs its own gutter, or the timeline needs top padding while it shows.
2. **The capsule's running count disagrees with the Running list.** The capsule said "1 running" while the Running section listed 2 agents and the working line said "2 running · 2 subagents working" (`S/q-capsule.png` vs `S/q-cap-running.png`). Code: `topicRunningCount` in `lib/status/running.ts`.
3. **The number stepper is misaligned, and there are several different steppers.**
   - **Misalignment:** `.settings-number-step` has no min-height reset, so the base rule `button { min-height: 44px }` (`style.css:37`) makes the step buttons 44 px inside a 32 px, overflow-hidden `.settings-number` (`ui/settings/settings-form.css:196-202`). The glyphs are bottom-offset (`S/q-stepper-crop.png`).
   - **Other implementations:** `screens/settings/rows.tsx:57` `Stepper` (`.settings-stepper`), `screens/board.css:547` `.num-stepper`, and `components/phone/phone.css:258` `.ph-stepper`.
   - **No stepper at all:** Session › Limits uses bare `<input type=number>` (`components/session/LimitsSection.tsx:121`).
4. **The working-line timer resets when the layout remounts.** "Waiting on graph … 9m 15s" became "0s" after a resize to phone width and back (`S/q-narrow.png`, `S/q-after-resize.png`). The elapsed time should come from the session's working-since time, not a component-local start.
5. **The LH diagram in the side pane isn't fitted or readable.**
   - The first render isn't Fit, so nodes are cut off right and bottom (`S/q-diagram.png`).
   - At Fit, a 23-node graph's labels and chips are about 5 px.
   - The +/−/Fit/Full controls overlay nodes and edges, with no reserved corner.
   - Node chips clip without an ellipsis ("medi", "sonnet-u", "opus-ui-q").
6. **The turn rail is limited.**
   - It's hidden whenever the conversation is narrower than 720 px (`TurnNavigator.tsx:20,67`), which is always the case at 1280 px with the side pane open (`S/q-graphwork-6s.png`).
   - It lists only the *loaded* turns: 6 entries on a chat of more than 1000 messages (`navigatorItems(turns)`), so you can't jump to older turns.
7. **The turn rail overlaps content in the narrow layout.** At 760 px wide (phone shell), the ticks draw over the Memory card's left edge (`S/q-narrow.png`).
8. **Scratch ("No workspace") chat titles are cut off at the top** (`S/q-q1.png`). `.conv-sub-btn` inherits `min-height: 44px`, so `.conv-head-text` is 65 px tall inside a 56 px header, and the title sits at y = −5. Code: `components/panes/panes.css:20`, `ConversationHeader.tsx:166`. This dates from 6e7c3232 (before this wave) but shows on every new chat.
9. **Settings › Subagents has redundant entries.** "Manage agents… · Open…" and "Agent profiles ›" both open agent management (`S/q-subagents-settings.png`).
10. **"Running" names two places.** The pane tab "Running" and the modal tab "Running" look different (see P1-2). The pane's Running tab also uses the same graph icon as the Graph tab (`S/q-cap-running.png`).

### P3

1. The chat ⋯ menu says "Consolidate memories", while the sidebar menus say "Dream" for the same idea (`S/q-session-menu.png`).
2. Graph progress changes its total with the Errors toggle ("10/23" becomes "10/16", and "2 error" disappears). A view filter shouldn't change the headline metric.
3. The project menu drops Archive instead of disabling it when nothing can be archived (`HostGroup.tsx:175`). Disabled with a reason would keep the menu's shape stable.
4. The "List properties: sort, filter, group" tooltip stays visible after the menu closes (`S/q-after-resize.png`).
5. Usage stat sub-lines are truncated ("1.1M in · 1.3M out · 9…", "+ 40.7M tokens unpri…") (`S/q-usage.png`).
6. After a queue reorder, the first item's Steer button has no `title` (eval of the queue buttons).
7. Memory rows in scratch chats show the raw scratch id as the project ("0 memories · 01a1a60c_5165_…") (`S/q-q1.png`).
8. Error toasts sit over the modal's filter row and over chat content (`R/rig-running-tab-unified.png`, `R/rig-lh-stop-settles.png`).
9. The Git source picker ("Unstaged") is a light native-looking select in the dark theme (`R/rig-git-only-pane.png`). The Full-window diagram has no title or graph name for context (`S/q-diagram-full.png`).
10. On GNOME's AppIndicator, a single tray click still opens the menu, not the window (a documented limit, `desktop.rs:253-259`).

## Placeholder, hardcoded-data and reachability sweep

- I found no "coming soon" text or buttons that do nothing in the flows I drove.
- **Not reached from the UI in practice:** `long-horizon.graph stop_operator` (the Stop on an orphaned step) only shows when a worker outlives its stop. That never happened in the rig, so the button has only been exercised in tests.
- **No visible use:** `get_tree`, which the app treats as best-effort, is the call that crashes the host (P0-1).

## Recommended next step

1. Fix P0-1 at both ends: the app stops calling `get_tree`, and the host serializes the tree without recursion and has a deep-chain test.
2. Then fix P1-1 (use host stats for the Info models and duration) and P1-2 (one Running row component with row → tab).
3. Re-run Tree on the whitelabel copy and confirm `grep -c overflowed host.log` stays at 0.
