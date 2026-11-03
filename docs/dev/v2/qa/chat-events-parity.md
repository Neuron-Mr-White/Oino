# Chat events: inventory and parity with the old UniPi app

Owner report (2026-11-03): "features missing in chat like memory and other types of
events: they can't expand and don't render differently."

This page lists every session entry and event kind that can reach a chat, how the old UniPi
app (`unipi-app/apps/mobile/src/components/chat/Cards.tsx`, `lib/chat/model.ts`) drew it, how
Unipi drew it before this fix (paths at `259c5e76`), and what changed.

## 1. Inventory

### Where the kinds come from

- **Core (pi format, `crates/oino-session/src/entry.rs`)**: `message` entries with the roles
  `user`, `assistant`, `toolResult`, `bashExecution`, `custom`, `system`, `compactionSummary` and
  `branchSummary`, plus the entries `compaction`, `branch_summary`, `model_change`,
  `thinking_level_change`, `custom`, `custom_message`, `label`, `session_info`, `usage` and
  `context_edit`. Core also writes `oino.tool-dispatch`, `oino.command-echo` and `oino.cwd_change`.
- **Extensions (`extensions/unipi/*`)**. I grepped `session::send_message`,
  `session::append_entry`, `before_agent_start` `{"message": …}` replies and `cx.renderer(…)`:

| Extension | Chat messages (`custom_message`) | State entries (`custom`) | Renderers it registers |
|---|---|---|---|
| memory | `unipi-memory-recall-reminder` (recall at start + wake-up), `unipi-memory-retro-reminder` (save nudge) | `unipi-memory-session-card` (wake-up card), `unipi-memory-save-card` (side-agent save) | customMessage recall/retro, customEntry session/save cards, tools `memory_*` |
| subagents | `subagent-completion` | `subagent-step` | completion, step, `run_subagent`, `read_subagent` |
| fusion | `sidekick-completion` | `sidekick-step`, `unipi:fusion-sidekick` (state) | step, completion, tool `sidekick` |
| background-tasks | `background-task-notification` | `unipi:bg-tasks` (state) | notification, `bg_run`/`bg_logs`/`bg_kill` |
| long-horizon | `unipi-response` (mode command replies), `unipi:lh-continue` | `long-horizon-decision` (mode switch), `unipi-progress` (goal %) | — |
| workflow | `unipi:plan-mode-message` | `unipi:plan-mode`, `unipi:permission-mode` (state) | — |
| watchdog | `unipi-watchdog` (warnings, `before_agent_start`) | `watchdog.session-caps` (state) | — |
| compactor | `compactor-recall` | `unipi-compaction` (card), `compactor-origin`, `unipi:compactor-pending` (state) | customEntry `unipi-compaction` |
| skill-registry | `unipi-skills-revealed` | `unipi:skills-judged`, `unipi:skills-revealed` (state) | — |
| kanboard | `unipi:kanboard-continue/-next/-show/-help/-doctor/-reminder` | `unipi:kanboard-badge`, `unipi:kanboard-notice`, `unipi-progress` | show, badge, notice, progress |
| dream | — | `unipi-dream-report-card` | customEntry report card |
| utility | `unipi-response` (command replies), `unipi-continue` (empty) | `unipi:attachments`, `unipi:auto-name` (state) | harness panels for 10 known types, attachments, built-in tools |
| file-summary | — | `unipi.file-summary/turn`, `/undo` (turn footer) | turnFooter |
| browser | — | `unipi.browser/turn-shot` | customEntry turn shot |
| info-screen | — | `unipi-child-usage` (cost accounting) | — |
| btw | — | `btw-thread-entry` (side question + answer) | — |
| ask-user, web-api, mcp, notify, cron, updater, core | nothing in the chat (tools, toasts, bus events, Settings, the cron "Sent by cron" label) | — | ask-user tool |

The memory consolidate action sends a prompt into the chat. It shows up as a harness user
message (`HarnessRow`), not as an event.

### What real chats contain

I counted kinds in read-only copies (`~/.cache/chat-events-qa/`). The first copy has 41 Unipi
chats from `~/.oino/sessions`. The second has 179 recent old-app chats from
`~/.pi/agent/sessions`, the ones under 40 MB.

| Kind | Unipi chats | Old-app chats |
|---|---:|---:|
| `subagent-step` (custom) | 194 | 55,657 |
| `unipi-child-usage` (custom, state) | 0 | 36,592 |
| `unipi-memory-recall-reminder` | 152 (147 `display:false`) | 203 |
| `unipi-response` | 115 (104 are `<task-notification>` XML) | — |
| `subagent-notification` (`<task-notification>` XML) | 99 | — |
| `oino.tool-dispatch` (state) | 98 | — |
| `unipi-continue` (empty, `display:false`) | 56 | 43 |
| `sidekick-completion` | 50 | 2 |
| `unipi-ralph-loop-reminder` (`display:false`) | 38 | — |
| `unipi-compactor-resume` (`<session_resume>` XML, `display:false`) | 34 | — |
| system role messages (agent prompt sections) | 29 | — |
| `unipi-memory-retro-reminder` | 26 | 1 |
| `background-task-notification` | 20 | 127 |
| `long-horizon-decision` | 7 | 209 |
| `unipi-memory-session-card` | 6 | 94 |
| `unipi-memory-save-card` | 4 | 327 |
| `unipi:auto-name`, `oino.command-echo`, `unipi.file-summary/turn`, `compactor-origin` | 3/4/2/1 | 34/83/–/559 |
| `btw-thread-entry`, `compactor-auto-continue` | 1/1 | — |
| `unipi-compaction`, `unipi-progress`, `unipi-command-output`, `unipi-skills-revealed` | — | 19/19/35/3 |

## 2. Parity table

Before this fix, Unipi had three problems. Together they produce the owner's report:

1. **Hidden in the fold.** Every card went into the turn's "Worked for …" fold
   (`lib/session/turns.ts:526`). The fold is closed once the run is done. Only model and
   thinking changes and the workspace marker were lifted out after the answer
   (`turns.ts:586-609`). Memory recall, save cards, background-task results and the rest
   could only be seen by opening the fold.
2. **`display:false` dropped.** Messages with `display:false` were skipped completely
   (`turns.ts:525`, `CardRow.tsx:47`). The old app showed every harness message, which
   covers 147 of 152 recall reminders, all Ralph reminders and all compactor resumes.
3. **No expand and no distinct look.** Custom messages rendered through `RendererCard`
   with `expanded: false` and no toggle (`RendererCard.tsx:80`). While a render answer was
   in flight or `null`, the fallback was the same one-line spark row
   ("Unipi · <first line>") for every kind (`RendererCard.tsx:148`). XML notifications
   showed as `<task-notification>` text. A state entry without a renderer answer showed
   nothing (`RendererCard.tsx:109`). A `!cmd` row read `message.content`, which a
   `bashExecution` does not have, so it showed an empty row (`CardRow.tsx:24`).

Old UniPi used one card shape for all of these: `ucard` (icon, bold source, muted title, dim
meta, chevron; body in markdown or `pre`; tone ok/warning/error).

| Kind | Old UniPi | Unipi before (file:line @259c5e76) | Gap | Unipi now (`data-event`) |
|---|---|---|---|---|
| `bashExecution` (`!cmd`) | Shell card: command, `exit N`, red on failure, output body | `CardRow.tsx:24` row from `message.content` (empty) | blank, no output | `user-bash`: Shell · command · exit/cancelled, error tone, output in `pre` |
| `branch_summary` | — (pi TUI only) | `ModelChangeCard.tsx` "Branch: earlier messages" | summary not viewable | `branch-summary`: Branch · first line, markdown body |
| `model_change` / `thinking_level_change` | dim "Model → X" | `ModelChangeCard.tsx` | none | unchanged, shown after the answer |
| `compaction` | divider + expandable summary | `Compaction.tsx` | none | unchanged |
| system role message | not drawn | dropped (`turns.ts:497-503` has no branch) | invisible | `system`: System · agent prompt / first line, text body |
| `label`, `session_info`, `usage`, `context_edit` | not drawn | not drawn | none (tree/title/cost) | unchanged |
| `unipi-memory-recall-reminder` | Memory · Memory recall, markdown body | hidden (`display:false`) or a spark row in the fold | invisible, can't expand | `memory-recall`: Memory · recalled 5 · first title · project, violet, markdown |
| `unipi-memory-retro-reminder` | Memory · Memory save? | hidden or spark row | same | `memory-nudge`: Memory · save nudge |
| `unipi-memory-session-card` | Memory · project · N memories, wake-up body | `RendererCard` node only if the renderer answered, in the fold | no expand (wake-up said "(expand)" but couldn't) | `memory-wake`: Memory · wake-up · N memories · project, wake-up text body, warning tone on issue |
| `unipi-memory-save-card` | green Memory · Saved N memories | renderer node in the fold | hidden | `memory-saved`: Memory · saved `title` +N · side agent · tokens, green |
| `subagent-completion` | Subagent · title · status/time/cost, report body | spark row or renderer node + Transcript | no body | `subagent-done`, ok/error tone, report markdown, Transcript button kept |
| `subagent-notification`, `unipi-response` with `<task-notification>` | — (pi fleet) | spark row showing `<task-notification>` | raw XML | `subagent-notification`: Subagent · name · done/failed · duration · tokens, `<result>` as markdown |
| `sidekick-completion` | Sidekick · first line, report | spark row with `<subagent_completion_notification …>` | raw XML | `sidekick-done`: wrapper stripped, report markdown |
| `subagent-step` / `sidekick-step` | step rows | `AgentCard` / `StepRow` | none | unchanged |
| `background-task-notification` | Background task · name · status/exit, `$ cmd` + output tail | `BgNotificationCard` (badge, no output) | output not viewable | `background-task`: green/red/amber, `$ command` + output tail |
| `unipi-compaction` | Compactor · 120k → 30k tokens · method · trigger, sections | renderer node in the fold | hidden | `compaction-card`: same summary, sections + kept turns body |
| `compactor-recall` | Compactor · Session recall | spark row | same look | `compactor-recall` |
| `unipi-compactor-resume` | (TS compactor) | hidden (`display:false`) | invisible | `compactor-resume`: resumed · N events · N× compacted, XML in a `pre` |
| `long-horizon-decision` | "long-horizon graph" marker when mode ≠ none | hidden (`turns.ts:150`) | mode switch not in chat | `mode-switch`: Mode · Graph (explicit `/regular` → Plain) |
| `unipi-progress` (goal) | label row | hidden | goal checkpoints invisible | `goal-progress`: Goal · ~40% · summary, objective body. Kanboard bars stay hidden |
| `unipi:lh-continue` | Long-horizon · Continuation | spark row | same look | `lh-continue` |
| `unipi-ralph-loop-reminder` | UniPi · type (generic) | hidden | invisible | `ralph-iteration`: Ralph · iteration 18/30 · task |
| `unipi:plan-mode-message` | Plan mode | spark row | same look | `plan-mode` |
| `unipi-watchdog` | amber Watchdog · Warning | spark row | no warning look | `watchdog`: amber tone, text body |
| `unipi-skills-revealed` | Skills · Skill reveal | spark row | same look | `skills-revealed` |
| `unipi-response` (command reply) | UniPi · Response | spark row | same look | `response`: source from harness meta, icon per source, markdown |
| `unipi:kanboard-*` messages | Kanboard · title | spark row (show: renderer) | same look | `kanboard-*`; `show` draws the extension's board as its body |
| `unipi:kanboard-badge` / `-notice` | dim marker | renderer node | in the fold | `kanboard-badge` / `-notice`, extension body |
| `unipi-dream-report-card` | (sheet) | renderer node in the fold | hidden | `dream-report`: Dream · report ready · N pending, extension body |
| `btw-thread-entry` | (sheet only) | nothing | invisible | `btw`: Side question · question, answer markdown |
| `unipi.browser/turn-shot` | — | renderer node | in the fold | `browser-shot`, extension body (image) |
| `unipi:attachments` | attachment chips | renderer node | in the fold | `attachments`: N files · name, extension body (chips) |
| `unipi-command-output` (old app) | command reply | spark row | — | `command-output` |
| `oino.cwd_change` | — | marker row | none | unchanged |
| unknown `custom_message` | UniPi · type, body markdown or JSON | renderer node or spark row | no expand | `custom`: harness source/title, body markdown (or details as JSON in a code block), extension body if it renders one |
| unknown `custom` (third-party state) | JSON card | renderer node only | none | unchanged: `RendererCard`, extension only |
| hidden state: `oino.tool-dispatch`, `oino.command-echo`, `unipi:auto-name`, `unipi:skills-*` (state), `unipi:plan-mode`, `unipi:permission-mode`, `unipi-child-usage`, `unipi.file-summary/*`, `unipi:bg-tasks`, `unipi:fusion-sidekick`, `unipi:compactor-pending`, `compactor-origin`, `watchdog.session-caps`, `unipi-continue` and `compactor-auto-continue` (empty) | hidden | hidden | none | hidden (`eventKinds.ts` `HIDDEN`) |
| tools `memory_store/search/delete/list` | tool rows | `ToolRow` (Database icon) + the memory extension's tool renderer body | none | unchanged |

## 3. The fix

- `apps/unipi/src/lib/timeline/eventKinds.ts`: `describeEvent(entry)` is a pure function that
  returns the kind id, icon, source, one-line title, meta, tone and body
  (markdown/text/JSON, capped at 60k chars). `eventVisible` replaces the old
  `display:false` drop and the internal list.
- `apps/unipi/src/components/timeline/cards/EventCard.tsx` draws the row: icon and accent per
  family (memory violet, agents orange, tasks green, compactor gold, board amber, modes
  cyan, warnings amber, errors red), "Source · title", meta and a chevron. On desktop it
  expands inline. The state lives in the expansion store, so a virtual-scroll remount keeps
  it and the row's ResizeObserver re-measures. On the phone the body opens in a full sheet,
  like tool rows. For kinds whose extension draws the body (board, dream report, attachment
  chips, browser shot, unknown messages), `_oino/render` is asked with `expanded: true` only
  after the row is opened. The app's own body shows while that request is in flight or
  answers `null`.
- `lib/session/turns.ts` `finishSegment`: event cards at the start of a segment render above
  the fold (`Segment.before`). All cards after the final answer render below it
  (`Segment.after`). Cards in the middle of the work stay in the fold, in order.
- `lib/session/renderCache.ts`: the expanded variant gets its own cache key, under the same
  LRU limit of 500.
- `lib/timeline/snapshotCache.ts`: a cached event keeps its summary fields, each capped at
  2 KB, so the first paint already shows the right rows.

## 4. Verification

- vitest: `src/lib/timeline/eventKinds.test.ts` has golden tests for each kind plus one for
  where the rows go in a turn, 45 tests in all. `src/components/timeline/cards/EventCard.golden.test.tsx`
  has DOM goldens for each kind, collapsed and expanded: the renderer body is requested
  with `expanded:true` only when the row is opened, the open state survives a remount,
  JSON shows in a `<pre>`, and on the phone the row opens a sheet. That is 40 tests.
  The full app suite: 3060 pass and 4 fail, all in files this change does not touch
  (PushAccess, CapabilityPages Providers, the settings form goldens and the
  [chips] PhoneField test).
- Real app: a release .deb built at `04021587` on Agent37, run in the QA rig with the fake
  model. The chat was seeded with `scripts/qa/chat-events-seed.mjs`, and
  `scripts/qa/chat-events-qa.py` drove the run. It found 32 event rows covering all 31
  kinds. Each row is 32 px collapsed on desktop and 40 px on the phone. Every expandable
  row opened its body (inline on desktop, a full sheet on the phone). Neither width
  overflows horizontally. The .deb and the built frontend contain no private-router
  strings.
- Shots: `docs/dev/v2/design/shots/owner-fix/chat-events/`. `d-<kind>-collapsed|expanded.png`
  are desktop 1440 crops, `p-<kind>-sheet.png` are phone 390 sheets, `p-collapsed-NN-390.png`
  are phone timeline pages, and `result.json` holds the measured heights.
  Four desktop crops are missing because the row was outside the viewport when the crop
  was taken: collapsed goal/lh-continue/ralph and expanded system. The phone pages and
  sheets show those rows.
- Known gap: the kanboard badge and the dream report open the app's fallback body (JSON or
  the report path), because their extension renderer did not answer with a node for the
  expanded request on the fake-model rig. The row and its expand still work.
