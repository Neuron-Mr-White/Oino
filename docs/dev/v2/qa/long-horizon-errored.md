# Long-horizon: `errored` work, continue, Retry, event logs (schema for the UI)

Owner, 2026-11-04: *"Upstream errors or network errors break a node, and then the agent has
to create a new node or clean up. Only a node the main agent marks as failed counts as
failed. The error state should look different from failed and have a Retry button."*

Extension `unipi.long-horizon` (graph, swarm, goal, ralph). Source:
`extensions/unipi/long-horizon/src/graph.rs` (engine), `lib.rs` (wiring, service),
`goal.rs` (goal / ralph). No host change: the app reaches the extension through the existing
`_oino/service_call`.

## 1. Statuses

| status | meaning | counts as | dependents | app |
|---|---|---|---|---|
| `queued` | waiting for inputs, capacity or a scheduled continue (`nextRetryAt`) | live | wait | |
| `running` | a subagent runs it | live | wait | |
| `blocked` | view-only: queued work whose input failed / was stopped | live | — | |
| **`errored`** | the run was cut off: provider / network / upstream error, cut stream, crashed worker, host restart, refused start. Keeps its **operator session** and **partial output** (no committed result). | **live, not failed** | **wait** | distinct look + **Retry** |
| `completed` | committed result `r-<workId>` | settled | run | |
| `failed` | **only** set by the lead (`operation=fail`) | settled | blocked | |
| `stopped` / `cancelled` / `superseded` | obsolete, user cancel, replaced | settled | blocked | |

Display order (also the `counts` keys): `queued, running, blocked, errored, completed, failed,
stopped, cancelled, superseded`.

Before `errored` the engine **auto-continues** a transient error on the same operator
session: setting `graph.autoContinue` (Long-Horizon › Errors, default **1**, 0–3), backoff
30 s / 2 min / 10 min; a Retry-After hint replaces the backoff while the cumulative wait stays
under 1 h (a longer hint → errored at once). Permanent errors (4xx, auth, context overflow,
unknown) are errored at once. While an auto-continue waits, the item is `queued` with
`nextRetryAt` and its last event is `continued` by `auto`.

A restart requeues running work on its own operator after 3 s (`continued` by `host`, at
most 3 times per item, then `errored`). The same applies to goal / ralph turns (§6).

## 2. Event log

Per work item (`work[].events`, oldest first, max **24**, older ones dropped and counted in
`eventsDropped`):

```json
{"ts": 1793760060000, "status": "continued", "by": "auto", "detail": "auto-continue 1/1 in 30s after transient (network): connection error: reset by peer"}
```

- `ts`: epoch ms. `detail`: ≤ 300 chars, single line, omitted when empty.
- `status` ∈ `queued | running | errored | continued | completed | failed | stopped | superseded`.
- `by` ∈ `lead | user | auto | host`.

| transition | status | by | detail |
|---|---|---|---|
| `add_work` | `queued` | lead | `replaces <id>` or empty |
| run starts | `running` | who asked for it (`host` for a normal start; `lead`/`user`/`auto`/`host` after a continue) | `operator <id>` / `resumed operator <id>` |
| run ends OK | `completed` | host | |
| transient error within budget | `continued` | auto | `auto-continue n/N in Ns after <error>` |
| error, budget used / permanent / refused start | `errored` | host | the error |
| lead `continue` | `continued` | lead | note, else `continued by lead` |
| user Retry | `continued` | user | note, else `continued by user` |
| restart | `continued` | host | `interrupted by a restart; resuming (n/3)` |
| lead `fail` | `failed` | lead | reason |
| `stop`, `finish`, `/graph stop` | `stopped` | lead / user | reason (`graph finished`) |
| a cancel from the Running tab | `stopped` | user | `cancelled` |
| replaced (`replaces=`) | `superseded` | lead | `replaced by <id>` |

Per graph (`events`, max **48**, `eventsDropped`):

```json
{"ts": 1793760120400, "kind": "checkpoint", "by": "host", "detail": "needs_attention: Agent graph \"Port the parser\" checkpoint: work needs attention (1 queued, 1 errored)."}
```

`kind` ∈ `opened` (lead) · `checkpoint` (host; detail = `<settled|needs_attention|quiescent>: <first line>`) ·
`finished` (lead; detail = reason) · `stopped` (user / lead; detail = reason).

## 3. The app's op: service `long-horizon.graph`

Call through the existing frontend command (no new host RPC):

```json
{"type": "_oino/service_call", "target": "unipi.long-horizon", "name": "long-horizon.graph", "payload": {"op": "…"}}
```

(`serviceCall(s, 'long-horizon.graph', payload, 'unipi.long-horizon')` in
`components/session/modalData.ts`.) Session-scoped like the rest: call it on the chat's
session handle.

### 3.1 `snapshot` — one graph as data

Request `{"op": "snapshot"}` (current graph, else the newest earlier one) or
`{"op": "snapshot", "graphId": "graph-…"}` (current or any earlier graph).

Reply (`graph: null` when the project has no graph):

```json
{
  "ok": true,
  "graph": {
    "v": 1,
    "graphId": "graph-mvz9-ab12",
    "mode": "graph",
    "task": "Port the parser",
    "state": "open",
    "revision": 1,
    "createdAt": 1793760000000,
    "updatedAt": 1793760120000,
    "workCount": 2,
    "counts": {"queued": 1, "running": 0, "blocked": 0, "errored": 1, "completed": 0, "failed": 0, "stopped": 0, "cancelled": 0, "superseded": 0},
    "events": [
      {"ts": 1793760000000, "kind": "opened", "by": "lead", "detail": "Port the parser"},
      {"ts": 1793760120400, "kind": "checkpoint", "by": "host", "detail": "needs_attention: Agent graph \"Port the parser\" checkpoint: work needs attention (1 queued, 1 errored)."}
    ],
    "eventsDropped": 0,
    "work": [
      {
        "workId": "scan",
        "status": "errored",
        "instruction": "Map the parser modules",
        "profile": "subagent_explore",
        "inputIds": [],
        "attempts": 2,
        "autoContinues": 1,
        "continues": 0,
        "restartResumes": 0,
        "toolCalls": 9,
        "canRetry": true,
        "hasOutput": true,
        "events": [
          {"ts": 1793760000000, "status": "queued", "by": "lead"},
          {"ts": 1793760001000, "status": "running", "by": "host", "detail": "operator 8c00ebe4"},
          {"ts": 1793760060000, "status": "continued", "by": "auto", "detail": "auto-continue 1/1 in 30s after transient (network): connection error: reset by peer"},
          {"ts": 1793760090000, "status": "running", "by": "auto", "detail": "resumed operator 8c00ebe4"},
          {"ts": 1793760120000, "status": "errored", "by": "host", "detail": "transient (5xx), auto-continue used: 503 Service Unavailable"}
        ],
        "eventsDropped": 0,
        "model": "zai/glm-5.3",
        "thinking": "high",
        "operatorId": "8c00ebe4",
        "failureClass": "transient",
        "error": "transient (5xx), auto-continue used: 503 Service Unavailable"
      },
      {
        "workId": "port",
        "status": "queued",
        "instruction": "Port the parser",
        "profile": "subagent_general",
        "inputIds": ["r-scan"],
        "attempts": 0, "autoContinues": 0, "continues": 0, "restartResumes": 0, "toolCalls": 0,
        "canRetry": false,
        "hasOutput": false,
        "events": [{"ts": 1793760000000, "status": "queued", "by": "lead"}],
        "eventsDropped": 0
      }
    ]
  }
}
```

Work fields (always present): `workId, status, instruction (≤ 600 chars), profile,
inputIds, attempts, autoContinues, continues, restartResumes, toolCalls, canRetry, hasOutput,
events, eventsDropped`. Optional: `model, thinking` (what it ran on), `operatorId` (open the
worker session with it), `resultId` (completed only), `failureClass`
(`transient | permanent | start | lead`), `error` (the last error / failure text, ≤ 600 chars —
still present while a continue is queued), `stopReason`, `replaces`, `supersededBy`,
`blockedBy` (why a `blocked` item can never run), `nextRetryAt` (epoch ms of a scheduled
continue).

Graph fields: `v, graphId, mode (graph|swarm), task, state (open|finished|stopped), revision,
createdAt, updatedAt, workCount, counts, events, eventsDropped, work`; optional
`finish {resultIds, reason, at}`, `abandoned` (reason), `omittedWork` (count of the oldest
settled items left out). Bounded: every live item (`queued|running|errored`) plus the newest
settled ones, at most **120** items.

`canRetry` is true exactly when `status == "errored"`. Use `hasOutput` to offer "show partial
output" (§3.4 `output`) and `operatorId` to open the worker session.

### 3.2 `history` — earlier graphs as data

Request `{"op": "history"}`. Reply — the current graph first (when any), then the earlier
ones, newest first (≤ 21 rows); a row's full snapshot comes from
`{"op":"snapshot","graphId":…}`:

```json
{
  "ok": true,
  "graphs": [
    {"graphId": "graph-mvz9-ab12", "mode": "graph", "task": "Port the parser", "state": "open",
     "revision": 1, "createdAt": 1793760000000, "updatedAt": 1793760120000, "workCount": 2,
     "counts": {"queued": 1, "running": 0, "blocked": 0, "errored": 1, "completed": 0, "failed": 0, "stopped": 0, "cancelled": 0, "superseded": 0}},
    {"graphId": "graph-mvx1-77aa", "mode": "swarm", "task": "Audit the sidebar", "state": "finished",
     "revision": 4, "createdAt": 1793700000000, "updatedAt": 1793703000000, "workCount": 3,
     "counts": {"queued": 0, "running": 0, "blocked": 0, "errored": 0, "completed": 3, "failed": 0, "stopped": 0, "cancelled": 0, "superseded": 0},
     "finish": {"resultIds": ["r-a", "r-b"], "reason": "done", "at": 1793703000000}}
  ]
}
```

A stopped graph carries `"abandoned": "stopped(user_requested)"` instead of `finish`.

### 3.3 `retry` — the Retry button (op name: **`retry`**)

Graph / swarm work:

```json
{"op": "retry", "workId": "scan", "graphId": "graph-mvz9-ab12", "note": "optional, ≤ 2000 chars"}
```

→ `{"ok": true, "workId": "scan"}`. Same semantics as the lead's `continue`, recorded as
`by: "user"`: the item goes `queued` → `running` on the **same operator session** (no new work
id, no supersede chain); the operator receives *"Your previous run was cut off by an error
(…); continue from where you stopped …"* plus *"Note from the user: …"*. The start happens
before the reply returns (the next snapshot shows `running`, or `errored` again with a new
`errored` event when it could not start). The lead is not re-woken for it; it learns the
outcome at its next checkpoint. `graphId` is optional (default: the open graph).

Goal / ralph loop:

```json
{"op": "retry", "target": "goal", "note": "optional"}
```

→ `{"ok": true}`; clears the goal's error and sends the continue prompt.

### 3.4 `output` — a step's stored text (result or partial output)

Request `{"op": "output", "workId": "scan", "graphId": "graph-…"}` (`graphId` optional:
current, then history). Reply:

```json
{"ok": true, "output": {"graphId": "graph-mvz9-ab12", "workId": "scan", "status": "errored",
  "resultId": null, "operatorId": "8c00ebe4", "instruction": "Map the parser modules",
  "text": "Found 3 modules so far: lexer, ast", "error": "transient (5xx), auto-continue used: 503 Service Unavailable",
  "truncated": false}}
```

`text` ≤ 64 KB (earlier graphs ≤ 16 KB, `truncated: true` when clipped); `null` when none.

### 3.5 Errors

Every refusal is `{"ok": false, "code": "<code>", "reason": "<human text>"}`. The central
table (`graph::LhCode`, map each one in the app):

| code | when | suggested UI text |
|---|---|---|
| `lh.graph.not_found` | `graphId` unknown | "That graph is gone." |
| `lh.graph.closed` | the graph is finished / stopped (or none is open) | "This graph is closed." |
| `lh.work.not_found` | `workId` unknown | "That step is gone." |
| `lh.work.not_continuable` | not errored/failed (running, a continue already queued, completed, replaced) | "Already retrying." / reason |
| `lh.work.not_failable` | (lead `fail` only) already settled | — |
| `lh.work.continue_limit` | 50 continues on one item | "Retried too often — ask the agent to replace it." |
| `lh.request.invalid` | unknown op, missing `workId`, note too long | generic + code |
| `lh.goal.not_found` | `target:"goal"` without an active goal / loop | "No goal is running." |
| `lh.goal.not_errored` | the goal is not stopped on an error | "Nothing to retry." |

A transport error (`_oino/service_call` rejects: older extension build without the service) →
hide Retry and fall back to the slash command below.

### 3.6 Slash-command fallback (TUI, older apps)

`/graph retry <workId> [note]`, `/swarm retry <workId> [note]`, `/goal continue [note]`,
`/ralph continue [note]` — the same path (notify with the result / the code). Hidden runner
`lh-graph` also accepts `{"op":"snapshot"|"retry", …}` with the service's payload (its older
`output` op keeps its own shape).

## 4. Live pushes (no polling needed)

- Sticky `unipi:long-horizon:progress` (`LH_PROGRESS`), `graph` gains `counts.errored` and,
  when any, `errored: [{"workId": "scan", "error": "≤ 200 chars"}]` (≤ 5). Refetch the
  snapshot when `counts` change. `goal.status` is `"errored"` (plus `goal.error`) while a goal
  / ralph loop waits for a continue.
- Sticky `unipi:long-horizon:work-errored`:
  `{"graphId", "workId", "status": "errored", "error"}` (one per newly errored item; a
  `unipi:long-horizon:work-failed` with the same shape remains for failures).
- A notification + OS notification: *"Agent graph work scan stopped on an error: … It is not
  failed — its session is kept; the lead was woken to continue it, or press Retry."*

## 5. `/visualize-progress` reserved entry (`unipi.long-horizon/visualize`) params

`params.nodes[]` (existing shape) gains `canRetry: true` (errored only), `continues` (> 0) and
`events` (the node's newest 8, same event shape); `params.graph` gains `events` (newest 16);
`params.history` = the §3.2 rows. Mermaid: `classDef errored stroke:#E07B39,stroke-width:2px,stroke-dasharray:6 3`
(amber dashed — failed stays solid red `#F03C32`, blocked amber `#F0A818` short dashes).

## 6. Goal / ralph

A lead turn whose `agent_before_settle` outcome is `error` does not count as a goal turn (no
turn, stall or not-met step). Transient → auto-continue on the same budget/backoff (timer
`lh-goal-continue`, prompt *"Your previous turn was cut off by an error (…); continue from
where you stopped."*); otherwise the goal is **errored** (status stays `active` in
`goal.json`, `errored: "<error>"`; `LH_PROGRESS.goal.status = "errored"`) until `/goal
continue`, `/ralph continue`, the app's `{"op":"retry","target":"goal"}`, or the user simply
sending the next prompt. `goal.json` `events` uses the §2 shape (`continued` auto/user,
`errored` host).

## 7. The lead (model side)

`update_agent_graph`:

```json
{"operation": "continue", "continue": [{"target_id": "scan", "note": "skip the vendored lexer"}]}
{"operation": "fail", "fail": [{"target_id": "scan", "reason": "the API is gone"}]}
{"operation": "stop", "stop": [{"target_id": "scan", "reason": "obsolete", "fail": true}]}
```

`target_id`: work id, result id or operator id. Tool descriptions and the Graph / Swarm
orchestration prompts say: errored ≠ failed, prefer `continue`, `fail` only when giving up,
`replaces` only when the instruction was wrong. Checkpoint line for errored work:

```
- scan errored (not failed; its session and partial output are kept): transient (5xx), auto-continue used: 503 Service Unavailable — continue it (update_agent_graph operation=continue continue=[{target_id:"scan"}]), or fail it (operation=fail) only if you give up
```

`yield_agent_graph` refuses when only errored work is left ("Nothing can progress: …"), so the
lead never sleeps on a graph that cannot move. `view_agent_graph` shows each item's newest 6
events and the graph's newest 10.

## 8. Tests

Rust unit (`cargo test -p unipi-long-horizon`): `transient_auto_continues_once_then_errored`,
`auto_continue_is_configurable_and_bounded`, `retry_after_replaces_backoff_within_the_cap`,
`permanent_error_is_errored_at_once_with_partial_output`,
`lead_continue_resumes_the_same_operator`,
`continue_by_operator_id_and_fresh_start_when_never_started`,
`explicit_fail_and_stop_semantics`, `user_retry_same_semantics_recorded_as_user`,
`restart_resumes_interrupted_work_on_its_operator`, `start_errors_are_bounded`,
`event_log_covers_every_transition_and_graph_events`, `snapshot_and_history_are_data`,
`swarm_attention_includes_errored`, `goal::turn_errors_auto_continue_then_errored_then_continue`,
`modes::settings_and_fragment`, `tools::schemas`.
Wasm-host harness (`extensions/unipi/subagents/harness/run.sh`):
`long_horizon_errored_node_user_retry_runs_the_graph_on`,
`long_horizon_restart_resumes_interrupted_work`.
