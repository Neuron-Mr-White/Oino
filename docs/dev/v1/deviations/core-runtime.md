# Deviations: core-runtime

Owner: `core-runtime` (also used by `core-agents` and `core-settings`). Settled rows #1–#22 and the
subcrate notes are folded into [runtime.md](../runtime.md#appendix-settled-deviations-folded-at-100-alpha2)
(1.0.0-alpha.2). New deviations go here first (README §5).

## Open

| # | Item | Notes |
|---|---|---|
| O1 | Bash detach | `control.bash-detach`, `control.running-bash`, `tasks.adopt-bash`: keep the session's `BashDetachRegistry` and hand the process to the task service (watchdog kill, background-tasks adopt). |
| O2 | Nested agents | `agents.agent.set-ui-policy` (unipi-agents R2); nesting depth on `call-context` (R3); `oino_agent` events reach only the spawning extension. |
| O3 | Shutdown | `ctx.shutdown()` only sets `shutdown_requested()`; modes must exit once idle. |
| O4 | Models host ops | `models.classify`, `models.generate-image`; usage of `models.*` calls is not recorded as `ext:<id>` entries. (`summarize-default` takes the previous summary since 2026-10-29.) |
| O5 | Settings bus | `oino:settings:<ns>` bus topic is not published (the bus lives in the wasm host). |
| O6 | Deferred from v1 | cache warming (§14), command output capture (§7.4), `before_provider_request` / `after_provider_response` / `provider_stream_event` routing, virtual-model `route` reason for summaries (`direct` only), tool-result / prompt image normalisation (#12), `project_trust` event (#9). |
| O7 | Requests from unipi | `control.move-project-state` (U5); tmp GC (U6); model `subscription` flag (unipi-ux Q6); guest read of `_oino/extensions` (Q7). (`tools.override` render-only registration + `render` for built-ins, `control.set-default-model` and the `host-info` UTC offset landed 2026-10-29.) |
| O8 | modes requests | Drain session forwarders on `dispose`; `SessionSource::New{id}` for `--session-id`; `SessionToolContext` rebinding after replacement (modes #19); RPC-visible model catalog refresh; frontend API to publish bus data (acp #2); `ToolInfo.kind` (acp #7). |
| O9 | Desktop requests | File checkpoints (TurnFooter Undo, rewind); single-item queue commands; honour `disabledExtensions` in the loader. |
