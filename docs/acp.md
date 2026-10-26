# ACP mode (Agent Client Protocol)

`oino --mode acp` runs Oino as an [ACP](https://agentclientprotocol.com) agent on
stdin/stdout (protocol v1, `agent-client-protocol` 3.5). Logs go to stderr. Every ACP session
loads the same extensions as the terminal UI (bundled providers + unipi, installed packages,
`-e` packages).

## Zed

`settings.json`:

```json
{
  "agent_servers": {
    "Oino": { "type": "custom", "command": "oino", "args": ["--mode", "acp"], "env": {} }
  }
}
```

## Supported

- `initialize`, `authenticate` / `logout` (no-ops: credentials come from `auth.json` / env).
- Sessions: `session/new`, `session/load` (replays history), `session/resume`, `session/fork`,
  `session/list` (pages of 50), `session/close`, `session/delete` (moves the file to
  `~/.oino/trash`).
- `session/prompt` (text, images, embedded resources) → streamed `agent_message_chunk`,
  `agent_thought_chunk`, tool calls with kind, title, locations, diffs (`edit`/`write`) and
  terminal output (`bash`), plans, usage, compaction notices; stop reasons `end_turn`,
  `cancelled`, `max_tokens`. `session/cancel` aborts.
- Config options: model, thinking level and more via `session/set_config_option`;
  `session/set_mode` selects the thinking level.
- Slash commands (`/compact`, `/name`, `/export`, `/session`, extension commands) are published
  as available commands.
- Extension approval dialogs (e.g. `unipi.workflow`'s permission gate) become
  `session/request_permission`; other dialogs become `elicitation/create` forms when the
  client supports them.
- When the client advertises `fs` / `terminal`, file reads/writes and `bash` run through the
  client.
- `_oino/rpc` and `_oino/<command>` extension methods run any RPC command ([rpc.md](rpc.md))
  for a `sessionId`.

## Known gaps

- MCP servers passed in `session/new` are not forwarded to `unipi.mcp` yet.
- The `refusal` stop reason is never returned; extension tools report kind `other`.
- Full list: [dev/v1/deviations/acp.md](dev/v1/deviations/acp.md).
