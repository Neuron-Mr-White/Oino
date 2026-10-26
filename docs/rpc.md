# RPC mode

`oino --mode rpc` speaks pi 1.4.0's RPC protocol on stdin/stdout: one JSON object per line.
Commands carry an optional `id`; every command gets a `response`
(`{"id","type":"response","command","success","data"|"error"}`); session events are streamed
as they happen (`agent_start`, `message_update`, `tool_execution_end`, `agent_settled`, …),
exactly like `--mode json`.

```bash
printf '%s\n' '{"id":"1","type":"prompt","message":"hello"}' | oino --mode rpc
```

## Commands (pi)

`prompt`, `steer`, `follow_up`, `abort`, `new_session`, `get_state`, `set_model`,
`cycle_model`, `get_available_models`, `set_thinking_level`, `cycle_thinking_level`,
`get_available_thinking_levels`, `set_steering_mode`, `set_follow_up_mode`, `compact`,
`set_auto_compaction`, `set_auto_retry`, `abort_retry`, `bash`, `abort_bash`,
`get_session_stats`, `export_html`, `switch_session`, `fork`, `clone`, `get_fork_messages`,
`get_entries`, `get_tree`, `get_last_assistant_text`, `set_session_name`, `get_messages`,
`get_commands`, `clear_queue`.

## Oino commands (`_oino/*`)

| Command | Purpose |
|---|---|
| `_oino/hello` | capability handshake (frontend capabilities such as `images`, `panels`) |
| `_oino/ui_snapshot` | current extension UI state (status, widgets, frame, panels, pending dialogs) |
| `_oino/navigate_tree` | move to another entry of the session tree, optionally summarising |
| `_oino/set_label` | label an entry |
| `_oino/get_context_usage` | context window usage |
| `_oino/import_jsonl`, `_oino/export_jsonl` | session import/export |

## Extension UI sub-protocol

Extensions talk to the user through `extension_ui_request` events (pi `rpc-extension-ui`):
dialogs (`select`, `confirm`, `input`, `editor`, plus Oino's `form`, `multiselect`,
`approval`, `custom`) expect an `extension_ui_response` with the same `id`; fire-and-forget
requests (`notify`, `setStatus`, `setWidget`, `setFrame`, `setTitle`, panels, `setPlan`) need
no answer. Unknown methods may be ignored. The full vocabulary, including the Oino UI Spec
(OUS) node tree used by panels and widgets, is in
[dev/v1/ui-protocol.md](dev/v1/ui-protocol.md).

## Not available yet

`oino serve` (RPC over TCP + TLS with pairing) and `--remote` are specified but not
implemented; neither are the `_oino/*` commands for settings, completions, rendering,
extension UI actions and shortcuts listed in [dev/v1/modes.md](dev/v1/modes.md) §4.
