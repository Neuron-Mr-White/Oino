# Deviations — unipi-agents

Work item `unipi-agents` (unipi-port §3.9–§3.14): `unipi.background-tasks`, `unipi.watchdog`,
`unipi.subagents`, `unipi.btw`, `unipi.fusion`, `unipi.long-horizon` under `extensions/unipi/`.

## Deviations

| # | Spec section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | §3.9 | Agent-task telemetry wrapping (`isAgent` → wrapped pi child reporting context/tokens/tools, `turndown`) is not ported; `isAgent` is kept in the schema/snapshot and `telemetryUnavailableReason` explains it. | Host tasks run plain shell commands; nested agents replace pi children (D8). | `bg_status` lacks `ctx=… tokens=… tools=…` for agent tasks. |
| 2 | §3.9 | Task ids, output paths and the output cap come from the host task service (`t…` ids, `~/.oino/tmp/tasks/…/output.log`); "dismiss" hides finished tasks in this extension only; `maxFinishedTasks` caps what is shown (host keeps 30). Stop = host kill (no SIGTERM→SIGKILL grace; the host kills the process group). | Host owns processes (extension-api §5.5). | Kill texts are unipi's; the 5 s stop wait polls host state. |
| 3 | §3.9 | The host renders the `<background-task-notification>` through `tasks.notification`, but sends it without `deliverAs: followUp` and with `details: {taskId}` (renderer re-reads the task). | Host `task_finished` builds the message (crates/oino-wasm-host/src/hub.rs). | Request R1. |
| 4 | §3.9 | Tray pane keys become OUS buttons/list actions (`bg:open`, `bg:stop:<id>`, `bg:kill:<id>` armed twice, `bg:stopall` armed twice, `bg:dismiss…`, `bg:rerun:<id>`, `bg:full:<id>`, `bg:path:<id>`); live log refresh = 1 s `work-tray:changed` ticks while the tray is open. | No raw key input for extensions. | Frontends bind keys to the actions. |
| 5 | §3.10 | Process sample uses host `process-stats`/`process-tree` (cpu ms, read/write bytes, state) for the pid from `control.running-bash`; `comm`, `wchan`, sockets, stdin are unavailable, so the sleeper veto also looks at the command text. `kill` action = `control.bash-detach` + `tasks.kill` (no `ps` scan); without detach support the item downgrades to warn. | Guests cannot read `/proc`. | `process.spawn` allow list is empty (no `ps`/`getconf`). |
| 6 | §3.10 | Session caps (D22) implemented as `sessionCaps.{maxTurns,maxCostUsd,maxWallMinutes,maxConsecutiveToolErrors,summaryTurn}` + flags; cap = `control.abort` + notify + `watchdog.session-cap` entry + one follow-up summary message; the summary run is aborted if it keeps calling tools. | New Oino feature (no unipi source). | — |
| 7 | §3.10 | `UNIPI_DEBUG_WATCHDOG` → setting `watchdog.debug` (extension log); `UNIPI_WATCHDOG_FORCE_ACT` dropped. | unipi-port §2.3. | — |
| 8 | §3.11 | `UiPolicy` is fixed at spawn: foreground runs forward dialogs, background runs auto-deny with unipi's note. Moving a run between foreground and background (Ctrl+B, tray `f`) cannot switch the policy. | `agents` has no policy setter. | Request R2. |
| 9 | §3.11 | Live foreground card = `subagents-foreground` widget (OUS cards) refreshed on child events (≤ 4/s); "Esc cancels" = tool cancellation; a pending user message moves the run to the background (unipi behaviour). | No TUI components. | — |
| 10 | §3.11 | Child usage is not re-recorded (host appends `child:<label>` usage entries, runtime §12). Nesting: `run_subagent` is not registered in child sessions; profiles with `max-nesting` raise the spawn `maxDepth` but the child cannot see its own allowance. | `call-context` has no depth. | Request R3. |
| 11 | §3.11 | Global profiles live in `/ext-global/profiles/` (= `~/.oino/state/unipi.subagents/global/profiles/`), project profiles in `.oino/agents/` (manifest `fs.project: write` for `/unipi:agents` create/copy/edit). The model picker in `/unipi:agents` uses `control.models-list` + unipi's fuzzy match. | unipi-port §2.3. | — |
| 12 | §3.11 | `read_subagent` dispatch to other owners: an owner calls `subagents.register-external {owner}` and serves `subagent-reader.read` (`{probe:true, agent_id?}` → `{owns, latest}`; otherwise `{agent_id?, block?, timeout?}` → `AgentToolResult`). | Replaces unipi's in-process reader registry. | Fusion implements it. |
| 13 | §3.11 | Graph runs: `subagents.graph-run {title, task, profile, resume?}` → `{runId}` or `{error, retryable}`; completion on bus `unipi:subagents:run-done {runId, result:{status,text,error,toolCalls,usage}}`; `subagents.profiles` lists runnable profile ids (empty when disabled/child). | Spec named only the service. | long-horizon uses it. |
| 14 | §3.12 | btw's side session uses the default system prompt + appended btw prompt (`systemPrompt: append`) instead of the stripped main system prompt; legacy `btw-note` messages in the branch copy are not filtered inside the child. Answers stream into the panel (≤ 20 updates/s); keys = panel keys (`ctrl+c`, `up`, `down`) + input submit. | `AgentSpec` cannot replace the prompt with the parent's. | Minor wording difference in the aside's system prompt. |
| 15 | §3.13 | `/unipi:model` is a dialog sequence (row `select` with `optionDetails` pricing → lead/sidekick/effort selects → "Confirm" / "Confirm and use at startup"), not the single interactive picker; the preset editor is two `multiSelect` dialogs + target. | Custom dialogs resolve on the first action and cannot carry several field values. | Same capabilities, more steps. |
| 16 | §3.13 | "Alt+Enter startup default" writes `startup {model, thinking, active}` into the global preset and applies it at the next fresh session start (plus `fusion.startup.*` settings); pi's `settings.json` `defaultModel` is not written. | Guests cannot write core settings. | Request R4. |
| 17 | §3.13 | The sidekick spawns with `ui: forward` (blocking default); non-blocking handoffs therefore also forward approvals instead of auto-denying. | Same as #8. | Request R2. |
| 18 | §3.14 | Payload surgery replaced by `set-active-tools` (spec §2.5); hidden mode tools are also blocked in `tool_call`. Tools register inactive and the gate activates the mode's set each turn. | Spec. | — |
| 19 | §3.14 | Continuation runs in `agent_before_settle` (manifest `limits.hookTimeoutMs: 90000` for the verifier) and is delivered by the nudge provider; when the session is idle (commands, timers, graph checkpoints) messages go out as harness user messages (provenance `Long-horizon`). | Host arbiter (runtime §7.6). | — |
| 20 | §3.14 | `/unipi:visualize-progress` shows mermaid source (`code` node, `language: mermaid`) plus a text summary in an overlay; no in-guest mermaid layout / resvg PNG. | Size/time; frontends may render mermaid. | Request R5 (OUS `mermaid`/image rendering). |
| 21 | §3.14 | The goal progress estimate uses an Oino prompt (`{percent, summary}` JSON) and shows a `lh-progress` status + `unipi-progress` entry; unipi's progress-chart/snapshot views are reduced to the `unipi:long-horizon:progress` sticky snapshot. | Not ported line-for-line. | Footer/app consumers read the snapshot. |
| 22 | §3.14 | Lease/generation checks on goal tool calls, the judge's chat-model fallback (`typesafe.ts`) and the graph's per-operator usage accounting are not ported; the judge uses `unipi-kit::jev` (fail-open). `swarm/graph` checkpoint wake aggregation = 400 ms timer; capacity refusals retry every 5 s. | Simplification; jev decisions transport covers the default. | — |
| 23 | §3.14 | Owner/goal/graph state is workspace-scoped (`/ext-workspace/<wsid>/{state,goal,graph}.json`) like unipi; ralph state sits next to the task file (`.oino/ralph/<name>.state.json`). `/unipi:continue` (mentioned in the mode section) belongs to utility. | Spec paths. | — |
| 24 | all | Native tests are the ported unipi unit tests for pure logic; host-behaviour coverage is in the wasm-host harness `extensions/unipi/subagents/harness` (standalone crate, `run.sh`). | `MockHost` does not exist (unipi-core R5). | Integration: add the harness to CI. |

## Requests for other work

- **R1 (wit-v1 / core-runtime):** in `LocalServices::task_finished`, send the rendered
  notification with `deliverAs: followUp`, and pass the `tasks.notification` service result's
  optional `details` (or the TaskInfo) through, so the message renderer gets unipi's snapshot.
- **R2 (wit-v1 + core-runtime):** `agents.agent.set-ui-policy(forward | auto-deny{note})` so
  foreground/background moves (subagents Ctrl+B, fusion `block:false`) switch approval routing.
- **R3 (wit-v1):** expose nesting depth / max depth on `call-context` for child sessions.
- **R4 (core-runtime):** a way for extensions to persist the startup model/thinking
  (pi `setDefaultModelAndProvider`), e.g. `control.set-model(…, persist: bool)`.
- **R5 (ui-protocol / frontends):** render `code{language:"mermaid"}` (or a `diagram` node) natively.
- **R6 (integration):** bundle the six components in unipi load order, regenerate checksums,
  and run `extensions/unipi/subagents/harness/run.sh` in CI.
