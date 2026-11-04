# owner-fix-4/5: strict UI/UX QA (real app, isolated rig)

**Verdict: NOT SHIP.** There is no P0. The owner-fix-3 P0 (the Tree tab overflowing the host stack) is fixed. There are five P1s:

1. The desktop folder browser can't be used with a mouse.
2. Opening a running cron chat kills the run.
3. The hub sometimes evicts the chat it just woke, so the chat dies with "could not start: backend closed".
4. Tree › All goes blank after the first 20 rows.
5. Session Info model rows still disagree with Cost and Usage.

**Score:** 15 of 22 criteria pass, 5 pass only in part, and 2 fail.

## Rig

- **Build:** `~/oino-qa-release/owner-fix-5/unipi-oino_amd64-9bb745e6.deb`. Its sha256 `d3004ef1…a100d` matches, and the rig's `app/.from` reads `d3004ef178d1e517`.
- **Isolation:** a fresh `QA_HOME=~/oino-qa-release/owner-fix-5/qa`, copied from the owner-fix-3 rig home. It holds the owner's chat copies, including "Frontend Whitelabeling Strategy" `01a19f58…` (3918 entries), and the start history was reset. It ran on Xvfb `:185` with WebDriver `:4485` and a private dbus. I never touched `~/.oino`, `~/.unipi` or the owner's app (pid 1482365 left alone).
- **Model:** the fake model `scripts/qa/phone-fakellm.py` on `127.0.0.1:11434`. Its log is at `~/oino-qa-release/owner-fix-5/myqa/fakellm.log`.
- **Input:** real X input through `scripts/qa/chat-stability/xin.py`. Screen = viewport + (80, 40).
- **Test folders:** `qa/home/qa-projects/{alpha (git), beta (git), gamma-nogit}`.
- **Screenshots:** `~/oino-qa-release/owner-fix-5/qa/shots/f5-*.png`, called `S/` below.
- **Logs:** host log copy at `~/oino-qa-release/owner-fix-5/myqa/host.log`, plus `start-history.json` (1 start, success).
- **Teardown:** `rig.sh down`, fake model killed by PID, and 2 leaked rig dbus-daemons (found by their environ) killed. I checked that ports 11434/4485 are free and that `:185` is gone.

## A. owner-fix-3 defects re-checked

| # | Criterion | Result | Evidence |
|---|---|---|---|
| A1 | Tree tab loads on the whitelabel copy; host log has no stack overflow; no restart loop | **PARTIAL** | **Pass:** Default loads in under 0.5 s (`S/f5-A1-tree.png`). The host pid was the same before and after the 20 s wait. `grep -ciE "overflowed\|stack overflow\|safe mode" host.log` = 0. `start-history.json` has 1 start, `success`. **Fail:** the **All** filter (and any filter over 200 rows) shows a blank pane (`S/f5-A1-tree-all.png`, `-all-mid.png`, `-all-wheel.png`). See P1-4. |
| A2 | Session Info: per-model rows + duration match Usage | **FAIL** (duration passes) | Duration 31h 2m equals the file span 02:59 → 10:01 (+1 day). Cost $133.42 matches Usage (cloud-agent $133). The **Models** rows still show claude-opus-5-5 95 replies **$15.14** and fake-1, with no sonnet or glm. That sums to $15 against $133. Usage shows claude-opus $127 / 1267 replies (`S/f5-A2-info.png`, `S/f5-A2-usage.png`). The alpha chat has the same gap: Tokens In 13.2k against the model row's 4.8k. See P1-5. |
| A3 | Side-pane Running uses the modal's rows; subagent row → subagent tab; bg row → terminal tab | **PASS** | The side pane and the modal render the same `sm-run-row` rows, with the same sections, filters, Stop buttons and open icons (`S/f5-A3-pane-running.png` vs `S/f5-A3-modal-running.png`). Clicking "Audit the settings schema" opened the subagent tab "Subagent … Running" (`S/f5-A3-sub-tab.png`). Clicking "Watch the test suite" opened a live terminal tab (`S/f5-A3-bg-tab.png`). |
| A4 | Capsule sits in flow; its count matches | **PASS** | The capsule is in its own strip under the header (y 62–90) and the timeline starts below it (`S/f5-A4-capsule.png`). "2 running" = 1 subagent + 1 bg task, matching the Running list and the working line "Waiting on bg … · subagent working". |
| A5 | One number stepper with centred buttons | **PASS** | "Max running at once" glyph centre offset is 0 px on both axes; buttons are 30 px in a 32 px box (`S/f5-A5-subagents-settings.png`, `S/f5-A5-stepper-crop.png`). The desktop uses only `ui/NumberStepper.tsx` (rows.tsx, fields.tsx, LimitsSection). The phone `.ph-stepper` (`components/phone/controls.tsx:221`) remains, which is P3. |
| A6 | LH diagram opens fitted; controls don't cover nodes | **PARTIAL** | **Pass:** the controls now sit in a bar above the canvas, so nothing overlaps. **Fail:** the opening view only fits the width. Its layer is 540 px tall in a 440 px view, so the last row (scan-host-85591 / scan-app-85591) is cut mid-node (`S/f5-A6-diagram.png`). Fit shows everything, but the text is about 5 px (`S/f5-A6-diagram-fit.png`). This is by design in `LhDiagram.tsx` `openView` (`OPEN_MIN_Z=0.6`), but it doesn't meet "opens fitted". See P2-2. |
| A7 | Working-line timer survives a side-pane toggle | **PASS** | Toggling read 9m 0s → 9m 1s → 9m 2s (`S/f5-A7-wl-toggle1/2.png`). Resizing to the 760 phone shell and back read 10m 57s → 10m 59s → 11m 2s. |
| A8 | Turn rail has its gutter at 760 px and is visible at ≥ 560 px chat width | **PASS** | **760 phone shell:** the rail is at x 8–38, `.tl` padding is 26 px and content starts at 42 (`S/f5-A8-rail-760.png`). **Chat 576 px (side pane open, window 1320):** the rail is visible (`S/f5-A8-rail-side-1320.png`). **Chat 556 px:** the rail is hidden. **Note:** at the default 1280 window with the side pane open the chat is 536 px, so the rail is still hidden there. That matches the spec. |
| A9 | A single Subagents agent-management entry | **PASS** | MANAGE has only "Agent profiles" (`S/f5-A5-subagents-settings.png`). |
| A10 | 'No workspace' chat title not cut | **PASS** | The title rect is y 9–29 inside the 56 px header (`S/f5-A9-scratch-title.png`). |

## B. Newly committed work

| # | Criterion | Result | Evidence |
|---|---|---|---|
| B1a | Desktop folder pane: recent, crumbs, filter, Miller columns | **PARTIAL** | The pane renders Places, Recent, crumbs, Filter, columns and the footer path (`S/f5-B1-kbd1.png`, `-kbd3.png`). It **can't be used with a mouse:** P1-1. |
| B1b | 'System dialog…' fallback | **FAIL** | There is no button in the footer (I listed the dialog's buttons: Close, crumbs, places, recent, Cancel, Use this folder). See P2-3. |
| B1c | Picking a folder starts the chat there | **PASS** (keyboard only) | ArrowDown / ArrowRight / ArrowDown then Ctrl+Enter set the chip to "alpha" (`S/f5-B1-picked.png`). The first prompt made a chat in `…/qa-projects/alpha` with the project group "alpha" (`S/f5-B1-started.png`). |
| B1d | Phone sheet | **PARTIAL** | At 420 px it works: Recent, Browse crumbs, Use this folder (`S/f5-B1-phone-420.png`). At a 760 px window (phone shell) the sheet sits at x 380–940, half off-screen (`S/f5-B1-phone-browse.png`). See P2-4. |
| B2 | Cron wake: a cron run shows in the sidebar at once | **PARTIAL** | **Pass:** after Run now, the row "QA wake digest" (clock glyph) appeared in a new "beta" group about 1 s after the run's session file landed. For the quick run, the row appeared at 4.2 s while the subprocess spent 3.2 s starting. For a slow run, the row appeared with no restart (`S/f5-B2-cron-run-sidebar.png`, `S/f5-B2-cron-running-row.png`). **Fail:** while the Cron page says **Running**, the row is `data-state="idle"` with no Working badge (P2-1). Opening the running run's chat **kills the run** (P1-2). |
| B3 | Sidebar first paint doesn't wait on the Archived list | **PASS** | On reloads with a live chat open, the first row appeared at 421 / 365 / 302 / 289 ms (`myqa/sbtime.mjs`). The host log shows only `sessions_list archived=false` (took_ms=1). `archived=true` happened only on the archived-chat reload (B4), as designed. |
| B4 | An archived chat still opens read-only; a deleted chat lands on New chat | **PASS** | Archive → "Archived chat · Read-only … Restore" in the neutral colour, and the composer says "Archived chat — restore to reply". After a reload the same view returns (`S/f5-B4-archived-now.png`, `S/f5-B4-archived-reload.png`). Archived › Delete → inline confirm → Delete → `#/new/…` "New chat", and Archived shows "Nothing archived" (`S/f5-B4-delete-confirm.png`, `S/f5-B4-after-delete2.png`). |
| B5 | AgentCard: the status icon sits inline when the sub line has nothing else (e9ba2c5a) | **PASS** (code + unit test) | Normal cards are unchanged: title, then "Explore subagent · fake-1 · ✓ · 1s · 1 tool" (`S/f5-B5-agentcards.png`). The fallback (no chip, no elapsed time, no counters) didn't occur in any live chat (0 `.tl-agent-titleline` across 4 chats). `AgentCard.test.tsx` passes (`vitest run`: 69/69 tests including the panes/tabs tests). |

## C. Quick regression

| # | Criterion | Result | Evidence |
|---|---|---|---|
| C1 | Sidebar list < 1 s | **PASS** | 289–421 ms (see B3). |
| C2 | Working states | **PARTIAL** | A chat with a running subagent and bg task shows the row as "Working" and the working line as "Waiting on bg: Watch the test suite · subagent working" (`S/f5-A3-sub-running.png`). Cron runs never show Working (P2-1). |
| C3 | Git tab only git | **PASS** | It shows alpha/main, Unstaged, Changes/History, UNTRACKED README.md, Stage all and Commit, with nothing else (`S/f5-C-git.png`). |
| C4 | Compact view | **PASS** | Rows are 28 px with `data-compact` and 44 px in normal, and titles show in both (`S/f5-C-compact-toggled.png`). |
| C5 | Queue → steer → send now | **PASS** | I queued 2 notes. "Steer now" on the second moved it to the top as STEER, and its title became "Send now (interrupts) — stops the current turn and sends this". Send now stopped the turn. The fake log shows the model got "second queued note" and then "first queued note" (`S/f5-C-q1.png`, `-q2.png`, `-q3.png`). |

## Defects

### P0

None. The owner-fix-3 P0-1 Tree crash is fixed (A1).

### P1

**P1-1: the desktop folder browser can't be used with a mouse. The chip menu stays open on top, and its backdrop closes both.**

- **Repro:**
  1. New chat → workspace chip → "Browse folders…". This happens with a real X click too.
- **Expected:** the menu closes and the "Choose a workspace" pane is on top and can be clicked.
- **Actual:**
  - The chip menu (`.recent-popover`) stays drawn over the dialog.
  - The side pane paints over the dialog's right third, covering Filter, the close button and the `.git` pill area.
  - `elementFromPoint` anywhere on the dialog returns `.recent-popover-backdrop` (z 44, body level).
  - Any click closes the menu **and** the dialog. Escape also closes both.
  - Only the keyboard reaches the pane: arrows, then Ctrl+Enter.
  - Screenshots: `S/f5-B1-browser.png`, `-browser-x.png`, `-afterbackdrop.png`.
- **Root cause:**
  1. `components/draft/WorkspacePicker.tsx:108-113` `openFolder()` calls `setPane(true)` without `setAnchor(undefined)`. The AnchoredMenu `onDismiss={close}` then also clears `pane`.
  2. `FolderBrowser.tsx:267` renders `.sm-overlay.fb-overlay` (z 70) inline inside `.draft`, which has `isolation: isolate` (`components/draft/draft.css:26`). Its z-index is trapped below the side pane and the body-level popover. It needs a Portal.
- **Coverage gap:** the jsdom tests and the browser-harness shots can't catch either problem.

**P1-2: opening a running cron run's chat kills the run, marks it failed and shows a red error.**

- **Repro:**
  1. Cron → job with prompt `slow: …` → Run now.
  2. Click the new sidebar row while the Cron page says Running.
- **Actual:**
  - On two tries the run ended within 1 s of the click. `c1a1a7219aae0002`: ended_at = 15:17:24.62, open = 15:17:24.23. `c1a1a72407fe0003` behaved the same.
  - Both ended with `status: failed`, `exit_code: 1`, "Request was aborted", and both rows now read "Failed".
  - The chat shows "This chat could not start: backend closed" (try 1, `S/f5-B2-cron-run-open.png`) or "Taken over from the CLI. Read-only here — take it back…" in error red (try 2, `S/f5-B2-cron-open-running.png`).
  - The turn reads "Worked for 0s · Done in 0s" for a run that was busy in `sleep 900`.
  - A stray session lock `01a1a722-….jsonl.lock(.json)` is left in the beta folder.
- **Why this is new:** before d467b230 a running cron chat wasn't in the sidebar until a restart. The fix now invites this click.
- **Host log:** `session_open` → `hub woke a session (E-LAZY) c1a1a7219aae0002` → `hub retired an idle session … (maxServices)` (same id) → `wake failed … backend closed`.
- **Likely area:** `unipi-host` `sessions/stream.rs` session_open wakes or takes over a session whose file the `oino` cron subprocess owns. A cron run with status running should open from its file, read-only, without waking a runtime. See also P1-3.

**P1-3: the hub evicts the session it has just woken, so the chat is dead until the app reloads.**

- **Repro:** visit about 8 projects or chats, then reopen one (here: alpha, openmontage, a cron run).
- **Actual:**
  - "This chat could not start: backend closed".
  - The composer loses Auto/Plan.
  - A message typed there sits at "Not sent · Retry / Waiting for connection…" (`S/f5-C-git.png`, `S/f5-C-alpha-dead-send.png`).
  - Switching away and back doesn't retry; no new wake appears in the log. Only an app reload recovers (`S/f5-C-alpha-after-reload.png`).
  - I hit this 3 times in about 40 minutes (`grep "wake failed" myqa/host.log`).
  - Each time the log has `hub woke a session (E-LAZY) <id>` and, 11 ms later, `hub retired an idle session to free its services (maxServices) session_id=<same id>`.
  - Services for `/home/oi/Projects/Personal/llm/oino` were evicted and recreated 8 times without my opening such a chat. Something keeps loading them.
- **Root cause:** `crates/oino-client/src/hub.rs:760-790` `services_for`. The LRU victim filter `services_set_idle` counts a just-woken session (no stream attached yet) as idle. It should exclude in-flight or just-woken sessions. On the app side, the open error is final, with no retry when the user comes back or presses Retry.

**P1-4: Tree › All (any list over 200 rows) shows only rows 0–19, then a blank pane.**

- **Repro:** whitelabel chat → side pane → Tree → All.
- **Actual:**
  - `.pane-tab-body` scrolls (scrollTop 5482 of 92526), while the virtualizer listens on `.ttab-list-scroll`. That element grew to its full 92412 px and never scrolls.
  - Rendered rows stay at data-index 0–19 at any scroll position (`S/f5-A1-tree-all*.png`).
- **Root cause:** `components/panes/tabs/tabs.css:55` `.ttab { min-height: 100% }` has to be `height: 100%`, so the list is height-bounded. The virtual path starts at `TreeTab.tsx:145` (`TREE_VIRTUAL_AT = 200`) and the scroller at `:255`.
- **Coverage gap:** jsdom has no layout, so the TreeTab tests pass.

**P1-5: Session Info Models rows count only the main assistant's messages, while Cost/Tokens include children (repeat of owner-fix-3 P1-1).**

- **Root cause:** `crates/oino-core/src/session/stats.rs:171-196` builds `models` rows only from `AgentMessage::Assistant`. `add()` also folds in `EntryKind::Usage` (child:session name / memory save / subagents, 1600 entries, $118), compaction and branch summaries. Settings › Usage attributes those by model.
- **Expected:** rows built from every usage source, so they sum to Cost and match Usage per model.

### P2

1. **A running cron run's sidebar row is idle.** It has `data-state="idle"`, a clock glyph and "now", while the Cron page shows Running (`S/f5-B2-cron-running-row.png`). After a kill it shows "Done in 0s". The card announced by `SessionHub::announce_cron_run` (`unipi-host/src/sessions/hub.rs`) carries no running state.
2. **The LH diagram's opening view clips the last row mid-node.** It opens at width-fit z 0.6 with no cue that more lies below (`S/f5-A6-diagram.png`). At Fit, labels and chips are about 5 px. Code: `status/LhDiagram.tsx` `openView`. Fit the height too, or fade the bottom edge and show "+N below".
3. **'System dialog…' is never shown in the real app.** `isOwnPcFile(hostId)` (`lib/openLocal.ts:13`) checks `hostId === 'local'`, but the real app routes with the PC fingerprint (`#/s/710d8791…`). The same check gates the Previewer and ExtensionInstall "open locally" paths.
4. **The phone-shell Sheet is off-screen in wide phone-shell windows.** At 760 px, `[data-testid=workspace-sheet]` is at x 380, w 560 with transform none, so the right half is cut, including "Use this folder" (`S/f5-B1-phone-browse.png`). Likely `left: 50%` without `translateX(-50%)` in the Sheet CSS at `data-shell=phone`.
5. **The side-pane Running rows crush at the 480 px pane.** Titles truncate ("Audit the setting…", "Watch the test su…") and chips shrink to single letters ("G.", "S", "w") (`S/f5-A3-pane-running.png`). Wrap the chips under the title in narrow containers.

### P3

1. **Places duplicates labels.** It lists "tmp" twice (`/tmp` and `/var/tmp`) with identical labels (`S/f5-B1-browser-x.png`).
2. **Phone crumbs start at "/".** The current folder is scrolled off to the right (`S/f5-B1-phone-420.png`).
3. **Cron form and history copy:**
   - The schedule preview shows raw UTC ISO ("next: 2026-11-05T01:00:00+00:00"), not local time (`S/f5-B2-cron-form.png`).
   - Run summaries show raw markdown `**12 passed…**` (`S/f5-B2-state2.png`).
4. **Cron runs are hard to tell apart.** Every run of a job has the same title, so 4 rows read "QA wake digest". The same chat is titled "beta" in the Archived list.
5. **Subagents settings copy reads broken.** "custom agents without a model: line." / "without thinking:." The frontmatter keys need code formatting.
6. **Headline counts disagree.** The capsule shows "Graph 18/23" while the pane header says "13/16 done" (errors hidden).
7. **Leftover tooltips:**
   - "List properties: sort, filter, group" stays after the menu closes.
   - "Fit the graph" covers the hidden-steps line.
8. **Labels are cut or raw:**
   - Usage stat sub-lines are still truncated ("1.1M in · 1.3M out · 9…").
   - Scratch chats show the raw id as the memory project.
   - The chat menu says "Consolidate memories" while the sidebar says "Dream".
9. **The session modal moves between tabs.** The dialog re-centres when switching Info ↔ Running, so the header jumps 16 px.
10. **Lock files pile up.** `.jsonl.lock` files remain after an archive-delete, and a stray `.lock.json` is left by the cron open.
11. **The rail lists only loaded turns.** It shows about 4 ticks on a chat of more than 1000 messages (carried over from owner-fix-3).

## Placeholder / reachability sweep

- I found no "coming soon" text or buttons that do nothing.
- **Built but not reachable in the real app:**
  - The FolderBrowser "System dialog…" footer (P2-3).
  - The pane's `.git` pill is drawn under the side pane while it's open (P1-1).
- **Not seen live:** the AgentCard inline-status fallback (B5) only appears in its unit test.
