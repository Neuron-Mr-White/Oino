# Superagent worker approval protocol

Authenticated `superagent.status` replies include the following item in `needsYou` while a worker is parked. `sessionId` identifies the calling session (possibly a foreground child); `workerSessionId` identifies the claiming worker. `summary`, `command`, and `risk` may be null for a non-shell permission request.

```json
{"projectId":"project","taskId":"TASK-1","status":"blocked","workerSessionId":"worker-id","requestId":"request-id","sessionId":"child-or-worker-id","tool":"bash","command":"git push","summary":"Push branch","risk":"high","ceiling":"medium","reason":"needs Autonomy High"}
```

The authenticated host request (inside the normal hello request envelope) is:

```json
{"op":"superagent.approval_response","projectId":"project","taskId":"TASK-1","workerSessionId":"worker-id","requestId":"request-id","decision":"allow_once"}
```

`decision` is `allow_once`, `always`, or `deny`. Success returns `{"accepted":true}`. The host validates the active dispatcher claim, live worker and single-use request ID. A worker chat receives `approval_cleared` with `requestId` whenever either surface answers. The desktop bridge command is `superagent_approval_response` with the same fields plus `hostId`.
