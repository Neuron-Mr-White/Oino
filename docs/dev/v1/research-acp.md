# Research: Agent Client Protocol (ACP) for Oino v1

Status: research note (work id `audit-oino`, 2026-10-26). No code changes.
Charter goal: "ACP (Agent Client Protocol) on top of RPC" — `oino-core` first speaks a
pi-compatible JSONL RPC mode, then an ACP agent adapter is layered over the same
runtime API.

Sources (fetched 2026-10-26):

- Spec repo `github.com/agentclientprotocol/agent-client-protocol` (moved from
  `zed-industries/agent-client-protocol`), HEAD `66ff807` (2026-10-25). Schema
  release **1.12.0** (2026-10-16). Files read: `schema/v1/meta.json`,
  `schema/v1/meta.unstable.json`, `schema/v1/schema.json`, `schema/v2/meta.json`,
  `docs/protocol/v1/*.mdx`, `docs/protocol/v2/migration.mdx`.
- Website: <https://agentclientprotocol.com/protocol/overview>, `/initialization`,
  `/session-setup`, `/prompt-turn`, `/tool-calls`, `/file-system`, `/terminals`,
  `/extensibility`, `/elicitation`, `/transports`.
- Rust SDK `github.com/agentclientprotocol/rust-sdk`; crate
  [`agent-client-protocol`](https://crates.io/crates/agent-client-protocol) **3.5.0**
  (2026-10-22, MSRV 1.88, edition 2024) depending on `agent-client-protocol-schema =1.12.0`.
- pi RPC docs (pi 1.4.0 install): `docs/rpc.md`, `docs/rpc-commands.md`,
  `docs/rpc-extension-ui.md`, `docs/json.md`.
- Existing pi adapter: <https://github.com/svkozak/pi-acp> (npm `pi-acp` 0.0.27; spawns
  `pi --mode rpc` and translates). Also `Attamusc/pi-acp`, `@wbline/pi-acp-jetbrain`.

## 1. Protocol basics

| Item | Value |
|---|---|
| Wire | JSON-RPC 2.0, requests + notifications both directions |
| Transport | **stdio**: client spawns agent; UTF-8, one JSON message per line, `\n` delimited, no embedded newlines; stderr free for logs (`transports.mdx`). Streamable HTTP is draft. |
| Versions | `protocolVersion: 1` = **stable**. `protocolVersion: 2` = stable-schema-but-"draft surface" (migration guide says gate v2 behind flags). Negotiated in `initialize`; agent answers highest version ≤ requested. |
| Extensibility | `_meta` object on most types; any method or notification name beginning with `_` is a custom extension (e.g. `_zed.dev/workspace/buffers`); advertise via `_meta` in capability objects. Cancellation of any request: `$/cancel_request`. |
| Paths | Absolute paths; line numbers 1-based. |

Framing is identical to pi RPC (LF-delimited JSON), so a single stdio reader/writer
implementation in Oino can serve both `--mode rpc` and `--mode acp`.

## 2. Methods (v1 stable, `schema/v1/meta.json`)

### Agent side (client → agent)

| Method | Params (brief) | Result | Notes |
|---|---|---|---|
| `initialize` | `protocolVersion`, `clientCapabilities?`, `clientInfo?` | `protocolVersion`, `agentCapabilities?`, `authMethods?`, `agentInfo?` | Mandatory first call |
| `authenticate` | `methodId` | `{}` | Only if `authMethods` non-empty (`terminal` / `agent` kinds) |
| `logout` | — | — | gated by `agentCapabilities.auth.logout` |
| `session/new` | `cwd`, `mcpServers[]` (required, may be empty), `additionalDirectories?` | `sessionId`, `modes?`, `configOptions?` | |
| `session/load` | `sessionId`, `cwd`, `mcpServers` | `{modes?, configOptions?}` | gated by `loadSession`; agent **replays** whole history as `session/update` notifications before responding |
| `session/resume` | `sessionId`, `cwd`, `mcpServers?` | — | gated by `sessionCapabilities.resume`; reattach without replay |
| `session/list` | `cwd?`, `cursor?` | `sessions[{sessionId,cwd,title?,updatedAt?}]`, `nextCursor?` | gated by `sessionCapabilities.list` |
| `session/close` | `sessionId` | — | gated by `sessionCapabilities.close` |
| `session/delete` | `sessionId` | — | gated by `sessionCapabilities.delete` |
| `session/prompt` | `sessionId`, `prompt: ContentBlock[]` | `stopReason` | **Response ends the turn** in v1 |
| `session/cancel` | `sessionId` | (notification) | agent must finish pending work and reply to `session/prompt` with `cancelled` |
| `session/set_mode` | `sessionId`, `modeId` | — | legacy; modes become config options in v2 |
| `session/set_config_option` | `sessionId`, `configId`, `value` | `configOptions` | select/boolean options (model picker, thinking level, …) |

Unstable v1 additions (`meta.unstable.json`, Rust features `unstable_*`):
`session/fork` (`unstable_session_fork`), `providers/list|set|disable`
(`unstable_llm_providers`), `mcp/message` (MCP-over-ACP), session system prompt,
subagents, plan operations, end-turn token usage.

### Client side (agent → client)

| Method | Params | Result |
|---|---|---|
| `session/update` (notification) | `sessionId`, `update: SessionUpdate` | — |
| `session/request_permission` | `sessionId`, `toolCall: ToolCallUpdate`, `options[{optionId,name,kind}]` | `outcome: {outcome:"selected", optionId} \| {outcome:"cancelled"}` |
| `fs/read_text_file` | `sessionId`, `path`, `line?`, `limit?` | `content` |
| `fs/write_text_file` | `sessionId`, `path`, `content` | — |
| `terminal/create` | `sessionId`, `command`, `args?`, `env?`, `cwd?`, `outputByteLimit?` | `terminalId` |
| `terminal/output` | `sessionId`, `terminalId` | `output`, `truncated`, `exitStatus?` |
| `terminal/wait_for_exit` | `sessionId`, `terminalId` | `exitCode?`, `signal?` |
| `terminal/kill` | `sessionId`, `terminalId` | — |
| `terminal/release` | `sessionId`, `terminalId` | — (kills if running, frees id) |
| `elicitation/create` / `elicitation/complete` | form mode (restricted JSON Schema) or URL mode | `accept`/`decline`/`cancel` + content |

`fs/*` require `clientCapabilities.fs.readTextFile|writeTextFile`; `terminal/*` require
`clientCapabilities.terminal: true`; elicitation requires
`clientCapabilities.elicitation.{form,url}`. Agents must not call what is not advertised.

### `session/update` variants (v1)

`user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk` (ContentBlock),
`tool_call` (create), `tool_call_update` (patch by `toolCallId`), `plan` (full list of
`{content, priority, status}`), `available_commands_update` (`[{name, description,
input?}]`), `current_mode_update`, `config_option_update`, `session_info_update`
(title/updatedAt), `usage_update` (`used`, `size`, `cost?` — context window), `notice`,
`compaction_update`, `compaction_summary_chunk` (last three stabilised in 1.11.0).

Tool calls: `ToolCall {toolCallId, title, kind?, status?, content?, locations?,
rawInput?, rawOutput?}`; `kind ∈ read|edit|delete|move|search|execute|think|fetch|
switch_mode|other`; `status ∈ pending|in_progress|completed|failed`; content items
`content` (ContentBlock) | `diff {path, oldText?, newText}` | `terminal {terminalId}`;
`locations[{path, line?}]` drive "follow-along" in editors.

Prompt content blocks: `text`, `image`, `audio`, `resource_link`, `resource`
(embedded); support for non-text is advertised via `promptCapabilities.{image,audio,
embeddedContext}`. `StopReason ∈ end_turn|max_tokens|max_turn_requests|refusal|cancelled`.

## 3. Session lifecycle (v1)

```text
client                                   agent (oino --mode acp)
  initialize(v1, caps{fs,terminal,elicitation}) ─▶
                         ◀─ {v1, agentCaps{loadSession, prompt{image,embeddedContext},
                                 mcp{http,sse}, session{list,resume,close}}, authMethods}
  [authenticate(methodId)] ─▶ (only when agent requires login)
  session/new(cwd, mcpServers) ─▶        ◀─ {sessionId, configOptions[model, thinking]}
                         ◀─ session/update available_commands_update
  session/prompt(sessionId, blocks) ─▶
                         ◀─ session/update agent_thought_chunk / agent_message_chunk …
                         ◀─ session/update tool_call(pending)
                         ◀─ session/request_permission(toolCall, options)   (optional)
  {outcome: selected allow_once} ─▶
                         ◀─ session/update tool_call_update(in_progress → completed, diff)
                         ◀─ session/update usage_update
  [session/cancel] ─▶ (notification; any time)
                         ◀─ result {stopReason: end_turn | cancelled}
  session/load | session/resume | session/list | session/close …
```

Concurrency: one active prompt per session; v1 has no steering/follow-up queue — a new
`session/prompt` while one is running is an agent-defined behaviour (pi-acp queues
client-side, "one-at-a-time").

**v2 (draft) changes worth designing for now** (`docs/protocol/v2/migration.mdx`):
`session/prompt` response only acknowledges acceptance and turn state arrives as
`state_update` (`running|idle|requires_action` + stop reason) — this is exactly pi's
`prompt` → `agent_settled` model; updates become ID-keyed upserts; `tool_call` merged
into `tool_call_update`; `session/load` → `session/resume {replayFrom}`; `set_mode` →
config options; **client `fs/*` and `terminal/*` removed** (use client-provided MCP
servers instead); capabilities become presence-objects under `capabilities.session`.
Recommendation: implement v1 now, keep the internal event model upsert-shaped so v2 is
a projection change, not a runtime change.

## 4. Permission model

- Agent-initiated, per tool call: `session/request_permission` carries the
  `ToolCallUpdate` (so the client can render title/kind/diff) and a list of
  `PermissionOption {optionId, name, kind}` where
  `kind ∈ allow_once | allow_always | reject_once | reject_always`.
- Client may auto-decide from user settings. Result is `selected(optionId)` or
  `cancelled` (must be returned when the turn is cancelled).
- "Always" persistence is the **agent's** job (ACP only transports the choice).
- Mapping to Oino: pi has no built-in permission gate; permissions come from extensions
  via `tool_call` hooks that can block (`{block: true, reason}`) and UI dialogs
  (`ctx.ui.confirm/select`). Oino should model a core `ToolApproval` request (emitted
  by the extension runner when a `before_tool_call` hook asks for confirmation) and
  render it as ACP `request_permission` in ACP mode, `extension_ui_request{method:
  "select"|"confirm"}` in RPC mode, and native dialogs in TUI/GPUI. Unipi's sandbox /
  mode extensions then supply the policy and remember `allow_always` decisions.

## 5. Capability negotiation — what Oino should advertise

```jsonc
// InitializeResponse (v1)
{
  "protocolVersion": 1,
  "agentInfo": {"name": "oino", "title": "Oino", "version": "<crate version>"},
  "agentCapabilities": {
    "loadSession": true,                       // pi session files replay
    "promptCapabilities": {"image": true, "audio": false, "embeddedContext": true},
    "mcpCapabilities": {"http": false, "sse": false},  // until an MCP extension exists
    "sessionCapabilities": {"list": {}, "resume": {}, "close": {}, "delete": {}},
    "_meta": {"oino": {"extensions": ["_oino/extension_ui", "_oino/rpc"]}}
  },
  "authMethods": []                           // provider auth is extension-owned
}
```

Client capabilities Oino can exploit: when `fs.readTextFile/writeTextFile` are present,
route the built-in `read`/`write`/`edit` tools through the client (unsaved editor
buffers, editor-side undo); when `terminal` is present, run `bash` via `terminal/create`
and attach `{type:"terminal"}` content to the tool call. This needs an
`ExecutionEnv`-style seam in core (the current `oino-env::ExecutionEnv` trait is the
right shape: file read/write/stat + command exec) with a local implementation and an
ACP-client implementation.

## 6. MCP servers in `session/new`

`mcpServers: McpServer[]` (`stdio {name, command, args, env}`, `http {name,url,headers}`,
`sse {…}`) is **required** (may be `[]`). Charter puts pi-mcp out of scope for v1, so Oino
v1 should accept and store them in session state, report `mcpCapabilities` false for
http/sse, and surface a `notice` if non-empty servers are ignored (same as pi-acp). A
later unipi/MCP WASM extension can consume them through a core hook.

## 7. pi RPC → ACP mapping (the adapter Oino needs)

pi RPC (1.4.0): commands on stdin `{id?, type, …}`, `response` records, session events
(`json.md`), and `extension_ui_request/response`. The ACP adapter is a translator from
the same in-process `AgentSession` API — not a subprocess like pi-acp.

### Commands

| pi RPC command | ACP | Notes |
|---|---|---|
| (process start) `--mode rpc` | `initialize` | ACP needs explicit version/caps handshake |
| `new_session` | `session/new` | `cwd` from params; `mcpServers` stored |
| `switch_session` | `session/load` (replay) / `session/resume` (no replay) | replay = emit history as `user_message_chunk`/`agent_message_chunk`/`tool_call`… |
| `get_state` / `get_session_stats` | `session/list`, `usage_update`, `session_info_update` | no direct "get state"; push instead |
| `prompt` (`disposition: started`) | `session/prompt` | v1 response = end of turn ⇒ resolve on pi `agent_settled`, not `agent_end` |
| `steer` | — | no v1 equivalent; `_oino/steer` extension or v2 prompt insertion (unstable) |
| `follow_up` | — | queue client-side or `_oino/follow_up` |
| `abort` | `session/cancel` | respond to pending prompt with `cancelled` |
| `clear_queue` | — | `_oino/clear_queue` |
| `set_model` / `cycle_model` / `get_available_models` | `session/set_config_option` (`configId: "model"`, select) + `config_option_update` | providers come from WASM extensions; options list built from model registry |
| `set_thinking_level` / `cycle_thinking_level` | config option `thinking` (select) | pi-acp maps it to a "mode" selector |
| `set_steering_mode` / `set_follow_up_mode` | config options (select) | |
| `compact` / `set_auto_compaction` | slash command `/compact` via `available_commands_update`; `compaction_update` + `compaction_summary_chunk` notifications | |
| `set_auto_retry` / `abort_retry` | config option / `_oino/abort_retry`; retries reported as `notice` | |
| `bash` / `abort_bash` (user `!cmd`) | — | `_oino/bash`; or client runs its own terminal |
| `export_html` | `/export` command | |
| `fork` / `clone` / `get_fork_messages` | `session/fork` (unstable) | otherwise `_oino/fork` |
| `get_entries` / `get_tree` / `get_messages` / `get_last_assistant_text` | — | `_oino/*` read APIs (GPUI tree view needs them anyway) |
| `set_session_name` | `session_info_update {title}` (agent→client) | client-initiated rename: `_oino/set_session_name` |
| `get_commands` | `available_commands_update` (push, on session start and when extensions change) | prompt templates, skills (`/skill:name`), extension commands |

### Events → `session/update`

| pi event (`json.md`) | ACP update |
|---|---|
| `agent_start`, `turn_start` | — (v2: `state_update running`) |
| `message_start/update/end` (user) | `user_message_chunk` (only during load/replay) |
| `message_update` `text_delta` | `agent_message_chunk {text}` |
| `message_update` `thinking_delta` | `agent_thought_chunk` |
| `message_update` `toolcall_start/delta/end` | `tool_call {status: pending, rawInput}` early (streaming input preview) |
| `tool_execution_start` | `tool_call_update {status: in_progress, kind, title, locations}` |
| `tool_execution_update` (`partialResult`) | `tool_call_update {content}` (bash: `terminal` content if client terminal used) |
| `tool_execution_end` | `tool_call_update {status: completed|failed, content, rawOutput}`; for `edit`/`write` add `diff {path, oldText, newText}` (pi-acp snapshots the file before the tool runs) |
| `turn_end` | `usage_update {used, size, cost}` |
| `agent_end` | — (wait for settled) |
| `agent_settled {aborted}` | resolve `session/prompt` → `stopReason: end_turn | cancelled` (`max_tokens` when the last stop reason was length) |
| `queue_update` | `_oino/queue_update` notification |
| `compaction_start/end` | `compaction_update` / `compaction_summary_chunk` |
| `auto_retry_start/end` | `notice` |
| `extension_error` | `notice` (severity error) |
| `session_info` / name change | `session_info_update` |

### Extension UI (`rpc-extension-ui.md`)

| pi `extension_ui_request.method` | ACP |
|---|---|
| `confirm` | `session/request_permission` when tied to a tool call; else `elicitation/create` form with a boolean |
| `select` | `elicitation/create` form (enum) |
| `input` / `editor` | `elicitation/create` form (string) |
| `notify` | `notice` |
| `setStatus`, `setWidget`, `setTitle`, `set_editor_text` | no ACP equivalent → `_oino/extension_ui` notification (clients that understand it render natively, others ignore); `setTitle` → `session_info_update` |

Tool-kind mapping for Oino built-ins: `read → read`, `write/edit → edit`,
`bash → execute`, extension tools default `other` unless their manifest declares a kind
(add an optional `kind` to the tool declaration in WIT v1).

## 8. Rust crate `agent-client-protocol` (3.5.0)

- Workspace `agentclientprotocol/rust-sdk`: `agent-client-protocol` (roles, connection
  builders, handlers, types), `-schema` (generated types, pinned `=1.12.0`), `-http`
  (HTTP/SSE/WebSocket), `-rmcp` (MCP bridge), `-conductor` (proxy chains), `-polyfill`,
  `-test`, `-trace-viewer`, `-cookbook`, `yopo` (example client).
- Edition 2024, MSRV 1.88 (Oino toolchain is 1.99 — fine). Core deps: `futures`,
  `futures-concurrency`, `serde`, `serde_json`, `tracing`, `uuid`, `rustc-hash` —
  **runtime-agnostic** (no tokio in the core crate). Optional features: `stdio`
  (via `blocking`), `process` (spawning agents: `async-process`, `rustix`/`windows-sys`),
  `schemars`, `unstable_*`, `unstable_protocol_v2`, `wasm_js`.
- API shape (v3): role builders instead of the old `Agent`/`Client` traits:

  ```rust
  Agent.builder()
      .name("oino")
      .on_receive_request(async move |req: InitializeRequest, responder, _cx| {
          responder.respond(InitializeResponse::new(req.protocol_version)
              .agent_capabilities(AgentCapabilities::new()))
      }, agent_client_protocol::on_receive_request!())
      // .on_receive_request(... NewSessionRequest / PromptRequest ...)
      .connect_to(Stdio::new())
      .await
  ```

  Client side: `Client.builder().on_receive_notification(|n: SessionNotification| …)
  .on_receive_request(|r: RequestPermissionRequest, responder, _| …)
  .connect_with(AcpAgent::from_str("oino --mode acp")?, |cx: ConnectionTo<Agent>| async
  { cx.send_request(InitializeRequest::new(ProtocolVersion::V1)).block_task().await … })`.
  `.v2()` builders select draft v2.
- Release cadence is fast (2.1 → 3.5 between 2026-09-04 and 2026-10-22, with a major
  bump on 2026-10-06). Pin an exact minor and isolate it in one crate.

## 9. Recommendations for Oino

1. **One runtime API, three projections.** `oino-core` exposes an in-process
   `AgentSession` command/event API (typed Rust enums mirroring pi RPC commands and
   `json.md` events). `oino-rpc` serialises it as pi-compatible JSONL (`--mode rpc`);
   `oino-acp` translates it to ACP v1 (`--mode acp`); TUI and GPUI consume it
   in-process. ACP never talks to core directly through RPC JSON.
2. Put ACP in its own crate (`crates/oino-acp`) that depends on `agent-client-protocol`
   (features `stdio`); nothing else in the workspace sees ACP types. That contains the
   SDK's churn.
3. Design core events as **ID-keyed upserts** (message id, tool-call id, plan id) and an
   explicit "settled" state, which serves pi `agent_settled`, ACP v1 prompt responses and
   ACP v2 `state_update` alike.
4. Add an `ExecutionEnv` seam for the four built-in tools so ACP clients with
   `fs`/`terminal` capabilities can host file I/O and terminals (v1 only; v2 drops them).
5. Add a core approval request type (tool-call scoped) used by extensions; ACP mode maps
   it to `session/request_permission` with the four option kinds.
6. Use `_oino/*` methods + `_meta.oino` capability advertisement for steer/follow-up,
   queue, tree/entries, extension widgets/status — the same payloads as RPC records, so
   the GPUI app can be an ACP client of a remote oino and still get full features.
7. Test strategy: golden transcripts (JSON lines in/out) per method, plus an interop
   smoke test with the SDK's `yopo` client and Zed (`agent_servers` custom command).
