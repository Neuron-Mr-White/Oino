# Superagent host protocol

This is the target contract for the redesign. Step 1 implements status, configuration and presets plus single-ticket start; multi-ticket leads, delivery selection and mode switching arrive in Step 2. Memory arrives in Step 3. Clients must not assume future operations already exist.

Requests use the authenticated host control stream: `{"type":"hello","proto":"2.0.0","req":REQUEST}`. Successful replies are `{"type":"ok","data":REPLY}`; errors are `{"type":"err","error":{"code":"E_BOARD_REJECTED","message":"...","retryable":false,"detail":{"code":"E_..."}}}`. Fields shown as `null` are nullable; optional request fields may be omitted. Examples use illustrative values. No operation broadcasts a separate status event; clients poll status. Existing session events, including approval events, remain on their session streams.

## Status

Request: `{"op":"superagent.status"}`

Reply data:
```json
{"enabled":true,"concurrencyCap":1,"leads":[{"projectId":"heartpick","sessionId":"session","presetId":"bug-batch","model":"provider/model","thinking":"high","autonomy":"high","mode":"swarm","ticketIds":["HP-1"],"step":"running","tool":null,"startedAt":"2026-11-07T00:00:00Z","cost":0.01,"progress":{"running":1,"queued":0,"done":0}}],"needsYou":[{"projectId":"heartpick","taskId":"HP-2","status":"blocked","question":"Which API?"},{"projectId":"heartpick","taskId":"HP-1","status":"blocked","workerSessionId":"session","requestId":"request","sessionId":"session","tool":"bash","command":"git push","summary":"Push branch","risk":"high","ceiling":"medium","reason":"needs Autonomy High"}],"review":[{"projectId":"heartpick","taskId":"HP-3","status":"in_review"}],"projects":[{"projectId":"heartpick","name":"Heartpick","autoStart":false,"lead":"session"}]}
```
`lead` is the active lead session id or null. `model`, `thinking`, `presetId`, `step`, `tool`, `startedAt`, and `cost` may be null if unavailable. Progress uses graph/swarm state when available, otherwise ticket counts. Only pending approvals and blocked tickets with a question appear in `needsYou`; review is separate. Approval items retain the fields defined by `worker-approvals-protocol.md`.

## Start and control

`{"op":"superagent.start","projectId":"heartpick","taskIds":["HP-1","HP-2"],"presetId":"bug-batch","overrides":{"model":"provider/model","thinking":"high","autonomy":"high","mode":"swarm","maxParallel":4,"review":"light","profiles":["work"]}}` → `{"sessionId":"session"}`. `presetId` and `overrides` are optional; overrides are partial preset fields. A running project lead receives new tasks rather than spawning another lead. Step 1 accepts legacy `taskId` and only one task.

`{"op":"superagent.stop","projectId":"heartpick","taskId":"HP-1"}` → `{"stopped":true}`. The task identifies the lead's claim; false means no matching claim.

`{"op":"superagent.relay","projectId":"heartpick","taskId":"HP-1","text":"Please also check tests","delivery":"queue","taskIds":["HP-2"],"priority":false}` → `{"commentId":"comment","sessionId":"session","delivered":true}`. `delivery` is `queue` or `steer`; `taskIds` and `priority` are optional, defaulting to `[]` and false. Queue is a follow-up; steer is read at the next step. A failed delivery is not reported as successful.

`{"op":"superagent.set_mode","projectId":"heartpick","mode":"graph"}` → `{"sessionId":"session","mode":"graph"}`. Mode is `none`, `goal`, `ralph`, `swarm`, or `graph`; mid-run application occurs at a step boundary.

`{"op":"superagent.configure","enabled":true,"concurrencyCap":1}` → `{"enabled":true,"concurrencyCap":1}`. Cap is 1–3, defaults to 1, and lowering it never stops existing sessions. `enabled` defaults false and is the watcher master switch.

`{"op":"superagent.configure_project","projectId":"heartpick","autoStart":true}` → `{"projectId":"heartpick","autoStart":true}`. Per-project auto-start defaults false; manual start is independent of both switches.

## Presets

A preset is `{"id":"bug-batch","label":"Bug batch","description":"Batch independent bug fixes","model":null,"thinking":null,"autonomy":"high","mode":"swarm","maxParallel":4,"review":"light","profiles":null,"priority":false}`. Model is `provider/id`; null model or thinking means the global default. Autonomy is `low|medium|high`, review is `none|light|full`, and mode has the values above. `maxParallel` and `profiles` are nullable. Priority is true for the urgent shipped preset and otherwise defaults false.

`{"op":"superagent.presets.list"}` → `{"defaultPresetId":"quick-fix","presets":[PRESET]}`.

`{"op":"superagent.presets.save","preset":PRESET}` → `{"preset":PRESET}`. Saves replace by id.

`{"op":"superagent.presets.delete","presetId":"custom"}` → `{"deleted":true}`. Deleting the default preset is refused. Shipped presets are ordinary persisted entries and may be edited or deleted when not default.

`{"op":"superagent.presets.set_default","presetId":"feature"}` → `{"defaultPresetId":"feature"}`. The referenced preset must exist.

Shipped defaults: quick-fix (none, high, review none), bug-batch (swarm, high, maxParallel 4, light), feature (graph, high, maxParallel 3, full), urgent (none, high, priority), research (none, medium, review none). Unspecified review is none.

## Approvals and events

`{"op":"superagent.approval_response","projectId":"heartpick","taskId":"HP-1","workerSessionId":"session","requestId":"request","decision":"allow_once"}` → `{"accepted":true}`. Decisions are `allow_once`, `always`, or `deny`; responses are single use and require a live claim. Session streams deliver the existing events in batches (other session events may share the batch):

```json
{"type":"events","items":[{"session":"session","seq":1,"event":{"type":"approval_request","requestId":"request","sessionId":"session","tool":"bash","arguments":{"command":"git push"},"riskLevel":"high","ceiling":"medium","reason":"needs Autonomy High"}}]}
{"type":"events","items":[{"session":"session","seq":2,"event":{"type":"approval_cleared","requestId":"request"}}]}
```

The existing approval parking behavior is unchanged; see `worker-approvals-protocol.md`.
