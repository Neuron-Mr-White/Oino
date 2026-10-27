# Modes: CLI, print, JSON, RPC, serve, ACP

Status: binding. Owners: `modes` (`crates/oino`, `crates/oino-rpc`, `crates/oino-client`),
`acp` (`crates/oino-acp`). Reference: audit-pi §13–§15, research-acp, pi `docs/{cli,rpc,
rpc-commands,rpc-extension-ui,json}.md`.

## 1. CLI (`crates/oino`, clap derive)

### 1.1 Flags (pi 1.4.0, same names and semantics)

`[messages…]` (positional; `@file` attaches text/images to the first message; piped stdin
prepended; `--` ends flags) · `-p/--print` · `--mode text|json|rpc|acp` (Oino adds `acp`) ·
auto print mode when stdout is not a TTY or stdin is piped and messages are given (pi rule) ·
`--export <in> [out]` · `--provider` · `--model <pattern[:thinking]>` · `--api-key` ·
`--thinking <level>` · `--models <patterns>` · `--list-models [search]` · `-c/--continue` ·
`-r/--resume` · `--session <path|id|partial>` · `--session-id <id>` · `--fork <path|id>` ·
`--session-dir <dir>` · `--no-session` · `-n/--name <name>` · `-t/--tools <list|+x,-y>` ·
`-xt/--exclude-tools` · `-nbt/--no-builtin-tools` · `-nt/--no-tools` ·
`-e/--extension <path|builtin:id>` (repeatable) · `-ne/--no-extensions` (D15) · `--skill`
(repeatable) · `-ns/--no-skills` · `--prompt-template` (repeatable) · `-np/--no-prompt-templates`
· `--theme` · `--no-themes` · `-nc/--no-context-files` · `--system-prompt <text|path>` ·
`--append-system-prompt <text|path>` (repeatable) · `-a/--approve` · `-na/--no-approve` ·
`--offline` · `--verbose` · `-h/--help` (lists extension flags) · `-v/--version`.
`--no-mcp` is accepted and ignored (no core MCP). `--tui-mode` is accepted by the TUI.

Oino additions: `--remote <url>` (interactive TUI over `RpcBackend`, `oino://host:port?token=…&fp=…`),
`--agent-dir <dir>` (same as `OINO_AGENT_DIR`). Unknown long flags are offered to extension
flag registrations (pi `unknownFlags`); still-unknown flags are an error with pi's message.

### 1.2 Subcommands

| Command | Behaviour |
|---|---|
| `oino install <source> [--project] [--yes]` / `remove|uninstall <id>` / `update [id]` / `list` / `config` | package manager (runtime §15) |
| `oino auth check [provider]` / `print-api-key <provider>` / `print-bearer-token <provider>` | pi exit codes 0 ok / 1 missing / 2 error |
| `oino ext validate <dir>` | manifest + component validation (extension-api §7.2) |
| `oino serve [--bind addr] [--allow-remote] [--pair]` | §5 |
| `oino update [--check]` | self-update from GitHub releases (Linux/macOS: replace binary atomically; Windows: download installer and exec) |
| `oino sessions migrate` | run legacy migration explicitly (also automatic, session-v3 §5.2) |

### 1.3 Interactive slash commands (frontends)

Frontends implement pi's built-in commands by calling the session API: `/settings` (opens
settings incl. extension hub), `/model [p/m]`, `/thinking [lvl]`, `/scoped-models`, `/login`,
`/logout`, `/new`, `/resume`, `/name [name]`, `/session`, `/tree`, `/fork`, `/clone`,
`/compact [instr]`, `/import <path>`, `/copy`, `/export [path]`, `/trust`, `/reload`,
`/hotkeys`, `/changelog`, `/quit`, `/extensions` (Oino: list/enable/disable/install).
`!cmd` / `!!cmd` → `execute_bash`. Anything else starting with `/` goes to
`AgentSession::prompt` (extension commands, skills, templates).

## 2. Print and JSON modes

Driven directly on `AgentSessionRuntime` (no `oino-client`).

### 2.1 Print (`-p`, `--mode text`)

pi `print-mode.js`: run the initial message (with `@file` attachments) then each extra
positional message via `prompt`; print every text block of the **last assistant message**,
each followed by `\n`; exit 1 with stderr `{errorMessage || "Request {stopReason}"}` on
error/aborted; exit 1 with `Response was truncated: the model reached its output token limit.`
on `length`; SIGHUP → 129, SIGTERM → 143; extension errors to stderr
`Extension error ({path}): {error}`; `ctx.mode = print`, `hasUI = false`. Exit 0 otherwise.

### 2.2 JSON (`--mode json`)

pi `json.md`: first line = session header object (`SessionHeader`); then every
`AgentSessionEvent` (runtime §6.1) as one JSON line, in order, LF framing, stdout backpressure
honoured, logs only to stderr. Wire transform for `message_update`: drop `message` and every
`partial`; add top-level `usage` (latest cumulative usage of the streaming assistant message);
`toolcall_start` carries `id` and `toolName`. Oino-only client events (runtime §6.2) are **not**
emitted in JSON mode. `extension_ui_request` is never emitted (no UI).

## 3. RPC (`--mode rpc`, `oino-rpc`)

### 3.1 Framing (pi, exact)

JSONL over stdin/stdout, split only on `\n` (strip a trailing `\r`), no U+2028/2029 splitting.
Command `{id?, type, …}`; reply `{"type":"response","command":<type>,"id"?:…,"success":true,
"data"?:…}` or `{…,"success":false,"error":"…"}`; malformed JSON →
`{"type":"response","command":"parse","success":false,"error":"Failed to parse command: …"}`.
Events have no `id` except `bash_execution_update{id}`. Stdin EOF → orderly shutdown
(`session_shutdown{reason:"quit"}`), exit 0. `ctx.shutdown()` completes after the current
command/`agent_settled`. `@file` arguments are rejected. No session header line. Events =
runtime §6.1 with the JSON-mode `message_update` transform, plus `extension_ui_request`,
`extension_error`. Oino client events (§6.2 of runtime) are sent **only after** the client
sends `_oino/hello`.

### 3.2 pi commands (exact request fields and response `data`)

`prompt{message, images?, streamingBehavior?}` → `{disposition}` ·
`steer{message, images?}` → `{disposition}` · `follow_up{message, images?}` → `{disposition}` ·
`abort` · `clear_queue` → `{steering, followUp}` · `new_session{parentSession?}` →
`{cancelled}` · `get_state` → `RpcSessionState{model?, thinkingLevel, isStreaming,
isCompacting, steeringMode, followUpMode, sessionFile?, sessionId, sessionName?,
autoCompactionEnabled, messageCount, pendingMessageCount}` · `set_model{provider, modelId}` →
`Model` · `cycle_model` → `{model, thinkingLevel, isScoped} | null` · `get_available_models` →
`{models}` · `set_thinking_level{level}` · `cycle_thinking_level` → `{level} | null` ·
`get_available_thinking_levels` → `{levels}` · `set_steering_mode{mode}` ·
`set_follow_up_mode{mode}` · `compact{customInstructions?}` → `CompactionResult` ·
`set_auto_compaction{enabled}` · `set_auto_retry{enabled}` · `abort_retry` ·
`bash{command, excludeFromContext?}` → `BashResult` (streams `bash_execution_update`) ·
`abort_bash` · `get_session_stats` → `SessionStats` · `export_html{outputPath?}` → `{path}` ·
`switch_session{sessionPath}` → `{cancelled}` · `fork{entryId}` → `{text, cancelled}` ·
`clone` → `{cancelled}` · `get_fork_messages` → `{messages: [{entryId, text}]}` ·
`get_entries{since?}` → `{entries, leafId}` · `get_tree` → `{tree, leafId}` ·
`get_last_assistant_text` → `{text | null}` · `set_session_name{name}` · `get_messages` →
`{messages}` · `get_commands` → `{commands: [{name, description?, source:
extension|prompt|skill, sourceInfo}]}` · `extension_ui_response{id, value|confirmed|cancelled}`
(no reply).

All error texts are pi's (golden-tested).

### 3.3 Typed client

`oino_rpc::RpcClient` (pi `RpcClient` equivalent): `connect(transport) -> (RpcClient,
EventStream)`, one async method per command returning typed `data`, auto-incrementing ids,
`ui_respond(id, UiResponse)`. Transports: `StdioChild{program, args}` (spawns `oino --mode rpc`),
`Tcp{addr, tls: TlsConfig, token}` (serve).

## 4. Oino RPC extensions (`_oino/*`)

Sent as normal commands (`type` begins with `_oino/`); responses use the same envelope. A
client opts in with `_oino/hello`.

| Command | Request | Response `data` |
|---|---|---|
| `_oino/hello` | `{client: {name, version}, ui: UiCapabilities (ui-protocol §10), protocol: 1}` | `{server: {name:"oino", version}, protocol: 1, sessionId}` |
| `_oino/ui_snapshot` | — | `{requests: [UiRequest]}` (retained fire-and-forget state + pending dialogs) |
| `_oino/ui_action` | `{panelId, actionId, value?}` | — |
| `_oino/editor_event` | `{eventId, intent, text, tokenId?}` | `{consumed}` for consumable intents |
| `_oino/invoke_shortcut` | `{id}` | — |
| `_oino/get_completions` | `{text, cursor}` | `{groups}` |
| `_oino/render` | ui-protocol §6.4 request | `{node | null}` |
| `_oino/get_settings_schema` | — | `{namespaces: [SettingsNamespace]}` |
| `_oino/get_extension_settings` | `{namespace, scope?: effective\|global\|project}` | `{values}` |
| `_oino/set_extension_setting` / `_oino/unset_extension_setting` | `{namespace, key, value?, scope}` | — |
| `_oino/get_settings` / `_oino/set_setting` | `{key?}` / `{key, value, scope}` | core settings (frontend keys included) |
| `_oino/get_keybindings` | — | parsed `keybindings.json` |
| `_oino/get_themes` | — | `{themes: [{name, path}]}` |
| `_oino/bus_get` | `{topic}` | `{data | null, publisher?}` |
| `_oino/topics` | `{subscribe?: [pattern], unsubscribe?: [pattern]}` | `{topics: [{topic, data, sticky, publisher}], patterns}`; matching publishes then arrive as `topic` events ([frontend-api.md](frontend-api.md) §2) |
| `_oino/navigate_tree` | `{targetId, summarize?, customInstructions?, replaceInstructions?, label?}` | `NavigateTreeResult` |
| `_oino/set_label` | `{entryId, label?}` | — |
| `_oino/import_jsonl` | `{path}` | `{cancelled}` |
| `_oino/export_jsonl` | `{outputPath?}` | `{path}` |
| `_oino/list_sessions` | `{scope: cwd\|all, cwd?}` | `{sessions: [SessionInfo]}` |
| `_oino/extensions` | — | `{extensions: [{id, kind, enabled, loaded, path?}]}` (implemented subset) |
| `_oino/set_extension_enabled` | `{id, enabled}` | — (writes `disabledExtensions`; new sessions / workspaces) |
| `_oino/login` / `_oino/logout` | `{provider}` | runs provider OAuth via UI dialogs / clears auth |
| `_oino/set_api_key` | `{provider, key}` | stores in auth.json |
| `_oino/get_context_usage` | — | `ContextUsage` |
| `_oino/agents` | `{follow?: bool}` | `{agents: [AgentInfo], follow}`; events `agents_changed`, `agent_event` (runtime §12) |
| `_oino/agent_messages` / `_oino/agent_abort` | `{agentId}` | `{messages}` / — |
| `_oino/queue_edit` | `{op: remove\|edit\|move\|sendNow, queue: steering\|followUp, index \| from+to, text?}` | — (`queue_update` follows) |
| `_oino/checkpoints` | — | `{checkpoints: [{entryId, createdAt, files: [{path, existed}]}]}` |
| `_oino/rewind` | `{entryId, files?: true, conversation?: true}` | `{restored, deleted, failed?, text?, cancelled}` |
| `_oino/running_bash` | — | `{calls: [{toolCallId, command, startedAt, pid?}]}` |

The GUI additions (topics, agents, queue edits, checkpoints, running bash, extensions,
`shutdown_requested`) with Rust signatures and event shapes: [frontend-api.md](frontend-api.md).
Every mode exits with 0 once an extension called `ctx.shutdown()` and the session is idle
(RPC, TUI, print / JSON; ACP leaves the lifecycle to its client).

Implemented additions and differences (`_oino/get_shortcuts`, `_oino/service_call`,
`_oino/auth_providers`, `shortcutId`, completion `from`): [deviations/frontend-cmds.md](deviations/frontend-cmds.md).

`serve` adds session routing (§5.2).

## 5. `oino serve` (remote and multi-client, D16)

### 5.1 Transport and security

- TCP listener (setting `serve.bind`, default `127.0.0.1:7420`; non-loopback requires
  `--allow-remote` or `serve.allowRemote`). TLS mandatory for non-loopback (rustls,
  self-signed certificate generated once at `~/.oino/serve/cert.pem`, key 0600).
- Authentication: first line from the client must be
  `{"type":"_oino/auth","token":"…"}`; tokens are random 32-byte base64url values stored in
  `~/.oino/serve/tokens.json` (`{name, hash (sha256), created}`); `oino serve --pair` prints
  a pairing URL `oino://<host>:<port>?token=<t>&fp=<sha256 cert fingerprint>` and a QR code
  (terminal) that mobile scans; clients pin the fingerprint. Wrong token ⇒ close.
- Framing: identical JSONL (§3.1) after auth.

### 5.2 Sessions

A `SessionHub` holds `AgentSessionRuntime`s keyed by session id. Every command may carry a
top-level `"session": "<sessionId>"`; without it the connection's *attached* session is used.

| Command | Request | Response |
|---|---|---|
| `_oino/open_session` | `{cwd, session?: path\|id, new?: bool, name?}` | `{sessionId, sessionFile?}` (creates or loads a runtime in the hub) |
| `_oino/attach` | `{sessionId}` | `{state: RpcSessionState}` then the client receives that session's events (and a `_oino/ui_snapshot`-equivalent replay) |
| `_oino/detach` | `{sessionId}` | — |
| `_oino/close_session` | `{sessionId}` | — (disposes the runtime when no client is attached and the session is idle) |
| `_oino/hub_sessions` | — | `{sessions: [{sessionId, cwd, sessionFile?, name?, runState, attachedClients}]}` |

Events from a non-default attached session carry `"session": "<id>"`. Multiple clients
attached to one session all receive events; dialogs resolve first-answer-wins (ui-protocol
§3). The desktop app runs the same hub in-process and may expose it via `serve`.

## 6. ACP (`oino --mode acp`, `oino-acp`)

ACP v1 (`protocolVersion: 1`) agent over stdio via `agent-client-protocol =3.5.x` (features
`stdio`, `unstable_session_fork`). The adapter sits on `SessionBackend` (in-process), never on
RPC JSON (research-acp §9.1).

### 6.1 Initialize

Advertise exactly research-acp §5 (`loadSession: true`, prompt image + embeddedContext,
mcp http/sse false, session list/resume/close/delete, `_meta.oino.extensions:
["_oino/extension_ui","_oino/rpc"]`, `authMethods: []`). Record client caps (`fs`,
`terminal`, `elicitation`).

### 6.2 Methods

| ACP | Oino |
|---|---|
| `session/new{cwd, mcpServers, additionalDirectories?}` | `SessionHub::open(new)`; `mcpServers` published on bus topic `oino:acp:mcp-servers` (consumed by unipi mcp); non-empty list without an mcp extension ⇒ `notice` "MCP servers ignored: no MCP extension enabled"; response `configOptions` = `model` (select over available models), `thinking` (select over levels), `steering_mode`, `follow_up_mode`, `auto_compaction` (boolean), `auto_retry` (boolean) |
| `session/load{sessionId, cwd}` | open session file; **replay** the current branch as `session/update` (`user_message_chunk`, `agent_message_chunk`, `agent_thought_chunk`, `tool_call` + `tool_call_update` completed) before responding |
| `session/resume` | open without replay |
| `session/list{cwd?, cursor?}` | `SessionManager::list` (page size 50, cursor = offset) → `{sessionId, cwd, title: name or firstMessage (≤80), updatedAt}` |
| `session/close` / `session/delete` | dispose runtime / delete session file (moves to `~/.oino/trash/`) |
| `session/prompt{prompt}` | convert blocks (text; image → images; `resource` embedded → text block `<resource uri="…">\n…\n</resource>`; `resource_link` → `@path` text for file URIs) → `prompt(text, images, streamingBehavior: followUp if streaming)`; resolve the request on **`agent_settled`** with `stopReason`: `cancelled` if aborted, `max_tokens` if the last assistant `stopReason == length`, `refusal` if `errorMessage` marks a refusal (provider-classified), else `end_turn` |
| `session/cancel` | `abort()` |
| `session/set_config_option` | `set_model` / `set_thinking_level` / `set_*_mode` / `set_auto_*`; respond with all `configOptions`; also push `config_option_update` on changes from other sources |
| `session/set_mode` | maps `modeId` to thinking level (pi-acp compatibility) |
| `session/fork` (unstable) | `fork(entryId = last user message entry, Before)` |
| `_oino/*` | every §4 command is also accepted as an ACP extension method with the same params/result |

### 6.3 Updates

| Oino event | `session/update` |
|---|---|
| `message_update` text_delta | `agent_message_chunk{content: text}` |
| `message_update` thinking_delta | `agent_thought_chunk` |
| `message_update` toolcall_start | `tool_call{toolCallId, title: toolName, kind (tool-spec kind; read→read, write/edit→edit, bash→execute, grep/find/ls→search), status: pending, rawInput: {}}` |
| toolcall_end | `tool_call_update{rawInput: arguments, title: rendered title, locations: [{path, line?}] for file tools}` |
| `tool_execution_start` | `tool_call_update{status: in_progress}` |
| `tool_execution_update` | `tool_call_update{content: [{type: content, content: text}]}` (bash: `terminal` when client terminal used) |
| `tool_execution_end` | `tool_call_update{status: completed\|failed, content, rawOutput: result}`; edit/write add `{type: diff, path, oldText, newText}` from a pre-execution file snapshot (adapter snapshots on `tool_execution_start`) |
| `turn_end` / `agent_settled` | `usage_update{used: contextUsage.tokens, size: contextWindow, cost: {amount: stats.cost, currency: "USD"}}` |
| `compaction_start/_end` | `compaction_update` (+ `compaction_summary_chunk` with the summary) |
| `auto_retry_*`, `extension_error` | `notice` (severity warning/error) |
| `session_info_changed` | `session_info_update{title}` |
| `commands_changed` / session start | `available_commands_update{commands: get_commands → [{name, description, input: {hint}}]}` |
| `queue_update`, `run_state`, UI fire-and-forget | `_oino/queue_update`, `_oino/run_state`, `_oino/extension_ui` notifications |

### 6.4 Dialogs and permissions

| UI request | ACP |
|---|---|
| `approval` | `session/request_permission{toolCall: current tool_call_update, options: approval options (kinds map 1:1)}`; outcome `selected` ⇒ `{optionId}`; `cancelled` ⇒ dialog cancelled |
| `confirm` | if a tool call is executing: `request_permission` with options allow_once/reject_once; else `elicitation/create` form `{confirmed: boolean}` when the client supports elicitation, else resolves `false` |
| `select` / `input` / `editor` / `multiSelect` / `form` | `elicitation/create` (form mode: enum / string / array / object schema) when supported; else cancelled |
| `notify` | `notice` |
| `setTitle` | `session_info_update{title}` |
| other fire-and-forget | `_oino/extension_ui` notification (same JSON) |

Client fs/terminal delegation: when the client advertises `fs.readTextFile/writeTextFile`,
the session is created with an `AcpExecutionEnv` routing `read_file`/`write_file` through
`fs/read_text_file`/`fs/write_text_file`; with `terminal: true`, `bash` runs via
`terminal/create` + `wait_for_exit` + `output` (output truncation still applied by the tool),
`terminal/kill` on abort, `terminal/release` after; the tool result content includes
`{type: terminal, terminalId}` in `tool_call_update`.

### 6.5 Acceptance (acp)

Golden JSON-RPC transcripts per method (quality §4); interop smoke test with the SDK test
client (`agent-client-protocol-test`) and Zed's custom agent server config (manual, recorded).

## 7. Interactive mode (`tui-client`: `crates/oino-tui`, `crates/oino-view`)

### 7.1 Structure

- `oino-view` (no UI toolkit; shared with GPUI): `Transcript` (rows keyed by entry id / tool
  call id; reducer `apply(&ClientEvent)`; streaming coalescer that merges deltas per frame),
  `UiState` (status by key with priority, frame segments, widgets by placement, header/footer,
  panels stack, dialog queue with first-answer-wins dismissal handling, title, working
  message, shortcuts), `SessionListModel`, `TreeModel` (from `get_tree`, fold state, labels,
  filter modes per pi `treeFilterMode`), `ComposerModel` (history, drafts, attachment tokens,
  completion state from `get_completions`), `QueueModel`. All reducers are pure and unit-tested
  with recorded event streams.
- `oino-tui`: ratatui/crossterm renderer + input loop over `Arc<dyn SessionBackend>`; ports
  the legacy renderer pieces (`legacy/crates/oino-tui`: markdown, mermaid, syntax highlight,
  composer, themes) and drops every built-in-extension surface (audit-oino §5). The binary
  (`crates/oino`) constructs the backend (in-process or `--remote`) and calls
  `oino_tui::run(backend, TuiOptions)`.

### 7.2 Behaviour

pi interactive mode is the reference (pi `docs/tui.md`, `keybindings.md`, `slash-commands.md`,
`sessions.md`): keybinding ids and defaults from pi `keybindings.md` verbatim (`app.*`,
`tui.*`, e.g. `app.interrupt` = escape, `app.model.cycleForward` = ctrl+p,
`app.thinking.cycle` = shift+tab, `app.tools.expand` = ctrl+o, `app.message.followUp` =
alt+enter, `app.message.dequeue` = alt+up), loaded/overridden from `keybindings.json` (via
`_oino/get_keybindings`-equivalent backend call); built-in slash commands (§1.3) with pi's
selectors (model selector, session resume picker with path/sort/named filters, tree navigator
with fold/label editing, fork picker); `!`/`!!` bash; `@` file completion; image paste;
external editor; settings screen incl. the extension settings hub (ui-protocol §6.6);
extension UI rendering per ui-protocol (dialogs as modal overlays, panels by placement,
widgets above/below editor, frame segments around the composer, status line, header splash);
extension shortcuts (ui-protocol §8) with frontend-reserved keys winning. Transcript
rendering of built-in tools mirrors pi (collapsed/expanded with `app.tools.expand`), thinking
blocks toggle (`app.thinking.toggle`), summaries as collapsible blocks, provenance cards.
`ctx.mode = tui`, `hasUI = true`.

### 7.3 Acceptance (tui-client)

`insta` snapshot tests with ratatui `TestBackend` for: streaming reply with tools, every
dialog kind, every panel placement, settings hub, tree navigator; reducer tests over golden
event streams from the json-mode suite; manual smoke on Linux, Windows (coffee) and macOS
(desmondgiam) terminals.

## 8. Acceptance (modes)

- Golden tests vs pi for print exit codes/stdout, JSON event streams, every RPC command
  success + error path (faux provider scripts, quality §4).
- `oino-client` contract tests run once against `InProcessBackend` and once against
  `RpcBackend` (stdio child) with identical expectations.
- `serve`: auth, TLS pinning, two clients on one session, first-answer-wins, attach replay.

---

## Appendix: settled deviations (folded at 1.0.0-alpha.2)

Sources: `deviations/modes.md`, `deviations/acp.md`, `deviations/frontend-cmds.md`, `deviations/tui-client.md`. pi 1.4.0 (`dist/main.js`, `cli/args.js`, `modes/*`) is the reference where this spec is silent.

### CLI, print, JSON, RPC (modes)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | modes §1 ("clap derive") | The CLI parser is a rule-for-rule port of pi `parseArgs` (hand written), not clap. | pi semantics clap cannot express: `-p` takes an optional message unless it starts with `-`/`@` (`---x` allowed), multi-letter short flags (`-nbt`, `-xt`, `-ne`), unknown long flags kept for extensions with "value unless next starts with `-`/`@`", `--name` without value is an error but other value flags without value are silently ignored. | none (same flags). |
| 2 | modes §1.1 | `--mode` errors list `acp` (`Invalid mode "x". Valid values: text, json, rpc, acp`, `--mode requires text, json, rpc, or acp`). | Oino adds `acp`. | Golden texts differ from pi there. |
| 3 | D13 / modes §1.1 | Help text: product substitutions plus pi-ai provider env vars, `mcp` and codemode lines removed; `serve`, `--remote`, `--agent-dir`, `OINO_*` variables added. | Providers are extensions; no core MCP. | Help is not byte-identical to pi's. |
| 4 | architecture §2, §4.2 | RPC records (`RpcCommand` = `PiCommand` ∪ `OinoCommand`, `RpcRequest`, `RpcResponse`, `RpcSessionState`) live in `oino_client::protocol`; client events and UI records come from `oino_core::{events, ui}`. | `oino-protocol` does not exist yet. | core-types-session: move them into `oino-protocol::rpc`; `oino-client` re-exports. |
| 5 | architecture §2 | `RpcBackend` (and its `EventDecoder`) live in `oino-rpc`, not `oino-client` behind feature `rpc`. | `oino-rpc` depends on `oino-client` (trait); the reverse optional edge would be a dependency cycle, which cargo rejects even for optional deps. | Frontends depend on `oino-rpc` for remote sessions. |
| 6 | architecture §4.2 | `SessionBackend` adds `send_with_id(request_id, command)` (default: `send`) and `shutdown()`. `send` returns the reply `data` as JSON (`RpcResponseData = Option<JsonValue>`); typed results are on `RpcClient` methods. | pi passes the RPC id to `executeBash` (`bash_execution_update.id`); RPC stdin EOF must dispose the runtime; one typed enum per command reply would duplicate core types. | tui-client/acp/gpui use `send` + `serde_json::from_value`, or `RpcClient`. |
| 7 | architecture §4.2 | `InProcessBackend::events()` subscribes to the **session** hub and follows replacements made through the backend (`new_session`, `switch_session`, `fork`, `clone`, `_oino/import_jsonl`), announcing them with a synthesized `session_replaced`. | The runtime hub is one forwarding task behind the session hub, so a reply could overtake the events its command caused; session hubs are filled synchronously. | Events the new session publishes between the swap and the rebind (e.g. UI requests from `session_start` handlers) are not seen. Replacements started elsewhere are not followed (core exposes none yet). See requests. |
| 8 | modes §3.1 | Ordering: the RPC writer drains every queued event before writing a reply, so events a command caused precede its reply (pi writes them synchronously); a `prompt` reply precedes that run's `agent_start` (held at most 1 s). | pi parity on a multi-threaded runtime. | none. |
| 9 | modes §3.1 | Malformed JSON: `Failed to parse command: <serde_json message>`; bad fields: serde messages (`missing field \`modelId\``, `unknown variant …`). | No JavaScript `JSON.parse` / TypeError texts in Rust. | Golden tests compare the prefix only. |
| 10 | ui-protocol §10 | Clients without `_oino/hello`: Oino-only dialogs resolve as cancelled (no degradation to `select`/`input`), Oino fire-and-forget methods are dropped, `extensionId`/`sessionId` and `extension_error.stack` are stripped. | Degradation renderers (`OusNode::to_lines`, form → sequential selects) need `oino-protocol::ui`. | pi clients never see Oino dialogs. |
| 11 | modes §3.1 | `ctx.shutdown()` sets `AgentSessionRuntime::shutdown_requested()` (runtime appendix #21), but no mode exits on it yet; RPC does not refresh model catalogs in the background (pi `modelRuntime.refresh`). | Mode-side wiring pending. | Extensions cannot end a session yet. |
| 12 | modes §2.1 | Print/JSON modes wait (≤ 5 s) for a started run's `agent_settled` to reach the output before disposing. | `AgentSessionRuntime::dispose` closes the runtime hub without draining the session forwarder (events in flight are lost). | none once core drains on dispose. |
| 13 | modes §1.1 | Interactive mode, ACP, `-e` packages and bundle discovery (integration), the package subcommands `install/remove/uninstall/update/list/config` and self-update (frontend-cmds #13–#15) exist. Still unavailable (exit 1 with `Error: … is not available in this build`): `--remote`, `auth`, `mcp`, `serve`, `ext`, `sessions`. `--session <id>` found in another project errors with a hint to use `--fork` (pi asks); `--session-id` of a missing session and `--fork` + `--session-id` error (core `SessionSource` has no explicit id); the missing-session-cwd prompt is not implemented. | Later work / core API. | — |
| 15 | modes §2.1 | `@file` images are attached as read (base64), without pi `processImage`. | core deviation 12 (no image processing in core). | Large images are not resized. |
| 16 | modes §1.1 | `--list-models <search>` uses a case-insensitive subsequence match per token (pi-tui `fuzzyFilter` approximation); sorting is case-insensitive then byte order (pi `localeCompare`). | No pi-tui port. | Ordering of non-ASCII ids may differ. |
| 17 | modes §1.1 | `--offline` is accepted but `OINO_OFFLINE` is not exported to the process (pi sets `PI_OFFLINE=1`). | `std::env::set_var` is `unsafe` in edition 2024 and crates forbid unsafe. | Extensions read the CLI flag only. |
| 18 | modes §1.1 | pi re-applies `setThinkingLevel` after creation when `--thinking`/`:level` was given (persisting it); not done. | Core already clamps at creation. | `defaultThinkingLevel` is not saved by `--thinking`. |
| 19 | runtime §8 | Built-in tools keep the `SessionToolContext` of the first session (core binds it once), so after `new_session`/`fork`/… their session env vars (`OINO_SESSION_ID`, …) describe the first session. | Core API. | see requests. |

### `_oino/*` frontend commands (frontend-cmds)

All commands are `OinoCommand` variants served by `InProcessBackend` (and therefore by `oino --mode rpc`) and by `RpcBackend`; typed calls for every backend: `oino_client::FrontendApi`.

All commands are `OinoCommand` variants, served by `InProcessBackend` and therefore by
`oino --mode rpc` (the RPC server forwards to the backend) and by `RpcBackend`. Typed calls for
every backend: `oino_client::FrontendApi` (blanket impl over `SessionBackend`).

| Command | Request | Response `data` | Core API |
|---|---|---|---|
| `_oino/get_completions` | `{text, cursor}` (cursor = character offset) | `{groups: [{source, from, items: [{value, label?, description?, tag?, tone?, sortTier?}]}]}` | `AgentSession::completions` |
| `_oino/ui_action` | `{panelId, actionId, value?}` | — | `AgentSession::ui_action` |
| `_oino/editor_event` | `{eventId, intent, text, tokenId?}` | `{consumed}` | `AgentSession::editor_event` |
| `_oino/invoke_shortcut` | `{shortcutId}` | — | `AgentSession::invoke_shortcut` |
| `_oino/get_shortcuts` | — | `{shortcuts: [{id, defaultKeys, description, extensionId}]}` | `ExtensionRunner::shortcuts` |
| `_oino/get_settings_schema` | — | `{namespaces}` | `ExtensionSettings::schema` |
| `_oino/get_extension_settings` | `{namespace, scope?: effective\|global\|project}` | `{values}` (secrets masked) | `ExtensionSettings::values` |
| `_oino/set_extension_setting` / `_oino/unset_extension_setting` | `{namespace, key, value?, scope}` | — | `ExtensionSettings::set` |
| `_oino/get_settings` | `{key?}` | `{settings}` or `{value}` | merged core settings |
| `_oino/set_setting` | `{key, value?, scope}` | — | `SettingsManager::set` |
| `_oino/service_call` | `{target?, name, payload?}` | the service's result | `AgentSession::service_call` (caller `oino.frontend`) |
| `_oino/auth_providers` | — | `{providers: [{id, name, oauth, configured, stored?, env}]}` | `AgentSession::auth_providers` |
| `_oino/login` | `{provider}` | — (after the flow finished) | `AgentSession::login` |
| `_oino/logout` | `{provider}` | — | `AgentSession::logout` |
| `_oino/set_api_key` | `{provider, key}` | — | `AgentSession::set_api_key` |

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | modes §4 `_oino/invoke_shortcut {id}` | The field is `shortcutId`. | `id` is the RPC correlation id of every command (pi framing); the shortcut id would be swallowed. | Clients send `{"type":"_oino/invoke_shortcut","shortcutId":"…"}`. |
| 2 | ui-protocol §7.3 | Completion groups carry `from` (character offset): an accepted item replaces `text[from..cursor]`. Core fills it for its own groups and defaults it to the token start for `complete-input` groups that omit it. Argument completion returns only the argument value (pi `getArgumentCompletions`). | The protocol did not say what an item replaces. | Frontends replace a range, not "the current token". |
| 3 | ui-protocol §7.3 | `@` paths: a `.gitignore`-aware walk of the cwd (depth 8, ≤ 20 000 entries, ≤ 50 results, fuzzy, prefix matches first); `~/` and absolute prefixes list one directory level. | No persistent core file index. | Large trees may miss deep files. |
| 4 | ui-protocol §6.1 | `ui_action` goes to the panel's owner (retained `openPanel`/`updatePanel` `extensionId`, else the `<ext>/panel-<n>` prefix) as `oino_ui_action` in a **command** context, and returns once dispatched (handler errors become `extension_error`). Same for `invoke_shortcut` (shortcut context). Owners receive the event whether or not they subscribed. | Panel buttons replace sessions, open dialogs, run commands; awaiting them would block the frontend's command queue. | Frontends get no handler result. |
| 5 | ui-protocol §7.2 | Consumable intents (`emptyArrowDown`, `tokenBackspace`) wait for every subscribed handler until one returns `{consume: true}` (normal hook deadlines; the 50 ms budget is the frontend's). Other intents are dispatched in the background and answer `{consumed: false}`. `editor_event` also records the composer text for `ui.editor-text`. | — | — |
| 6 | modes §4 `_oino/service_call` | New command (not in the table): calls `handle-service` of `target` or the first extension that registered the service, caller `oino.frontend`, 30 s timeout. Core learns service names from `ExtensionInstance::services()` (WASM: the host hub's registrations). | GPUI widgets (unipi app-bridge R7), dynamic settings options. | — |
| 7 | modes §4 `_oino/login` | Sign-in runs the provider component's `oauth-login` in the session's call context (its `ui.*` dialogs reach the frontend), strips `type`, stores an `oauth` credential in `auth.json` and re-runs `list-models` for that provider with the credential (Copilot `availableModelIds`). `_oino/auth_providers` (new) lists providers for the picker. Providers without `oauth: true` answer "… does not support OAuth sign-in; set an API key instead". | modes §4 left the flow open. | The reply arrives after the whole flow; frontends send it outside an ordered queue. |
| 8 | runtime §4 | Core installs an `OAuthRefresher` that routes `auth.json` refreshes to the registering component's `oauth-refresh` (15 s). | Refresh was implemented in `oino-models` but never wired, so OAuth tokens expired. | — |
| 9 | extension-api §3 `models` | `models.stream` streams the provider events of one request (same model/credential/virtual routing as `complete`, no retries, `cacheRetention: none`) as `AssistantMessageEvent` JSON; `models.credential-for` returns `{provider, headers, baseUrl?}` with `Authorization: Bearer <resolved key>` plus configured headers, or `null`. New `HostApi::model_stream` and `HostOp::CredentialFor`. | Known issue 7. | `classify` and `generate-image` remain unsupported. |
| 10 | extension-api §3 `ui` | `ui.editor-text` returns the text from the last `editor_event`; `editor-set` / `editor-insert` fire `set_editor_text` / `insertEditorText` (extension id `oino.host`, the WIT call has no caller); `clipboard-read-image` and `edit-externally` are dialogs (`clipboardReadImage`, `editExternally{text, extensionHint?}`) whose `value` is returned. | Known issue 7. | Frontends that never send `editor_event` report `""` (pi RPC behaviour). |
| 11 | extension-api §6 | `ProviderInstance::oauth_login` (`oino-extension-api`) takes the `CallContext`; core's `ExtensionInstance` gained `complete_input`, `services`, `handle_service`; core's `ProviderInstance` gained `oauth_login` / `oauth_refresh` (all defaulted). | The WASM `oauth-login` ran with the init context (no session), so its dialogs had nowhere to go. | Native test doubles implement them optionally. |
| 12 | modes §6 / integration #9 | ACP sessions resolve `--model` / `--provider` / `--models` / `:thinking` per cwd through `SessionDefaults::model_resolver` (`oino_acp::ModelResolver`, `ModelChoice`), the same rules as `startup::choose_model` without an existing session; diagnostics go to stderr per session. | The registry exists only once the cwd's services do. | Known issue 5 closed. |
| 13 | modes §1.2 `oino update` | Bare `oino update [--check]` is the self-update; packages update with `oino update <id>…` or `oino update --extensions` (`--project` for `.oino/extensions`). | `update [id]` (packages) and `update [--check]` (self) collide in modes §1.2. | — |
| 14 | runtime §15 | Package manager lives in `crates/oino` (`cli::packages`), not `oino-core::packages`; URL/archive sources need the system `tar` (`--zstd` for `.tar.zst`), git sources the system `git`; downloads use `reqwest`. `.source.json` = `{source, version, installedAt}`. `config` prints the settings paths instead of opening an editor. Permissions are printed; non-interactive installs need `--yes`. | The CLI is the only user; no archive crates in the workspace. | `oino install` without `tar`/`git` works for directories only. |
| 15 | modes §1.2 `oino update` | Self-update reads `release-manifest.json` (latest, `OINO_REF`, or `OINO_RELEASE_MANIFEST`, `file://` allowed), picks the `oino` artifact of `current_target()`, verifies sha256 and renames the running binary to `.old` before moving the new one in (also on Windows). | No installer artifact on Windows releases yet. | — |

### ACP (acp)

Reference: `agent-client-protocol =3.5.0` (schema 1.12.0), research-acp.md.

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | modes §6.2 `session/fork` | The fork is a new session file copying the source's whole current branch (`SessionSource::Fork`), not `fork(entryId = last user message, Before)`. | Fork-before drops the last turn and its text (meant for "edit message" flows); `AgentSessionRuntime::fork` replaces the runtime's session while built-in tools stay bound to the old one (modes deviation 19). Clients use `session/fork` to branch a thread. | Forks keep the last turn. |
| 2 | modes §6.2 `session/new` | `mcpServers` are not published on bus topic `oino:acp:mcp-servers`; a non-empty list only produces the `notice` "MCP servers ignored: no MCP extension enabled" (also on `session/load`). | Core has no API for a frontend to publish bus data. | unipi `mcp` cannot pick up client servers yet (see requests). |
| 3 | modes §6.2 `session/prompt` | `stopReason: "refusal"` is never produced. A provider error ends with `end_turn` after a `notice{severity: error, title: errorMessage}`. | No provider refusal classification exists in `oino-ai`/core. | Refusals look like errors. |
| 4 | modes §6.2 | `session/new`/`load` return no `modes` (`SessionModeState`); `session/set_mode` is still accepted (mode id = thinking level) and `current_mode_update` is never sent. | `configOptions` already carry the thinking level; returning both shows two selectors. | pi-acp-style clients that only read `modes` see no thinking selector. |
| 5 | modes §6.1 | `sessionCapabilities.fork: {}` is advertised too (SDK feature `unstable_session_fork`). | `session/fork` is implemented. | none. |
| 6 | modes §6.2 `_oino/*` | ACP extension methods are not session-routed, so every `_oino/<command>` takes `sessionId` in its params (removed before decoding). `_oino/rpc {sessionId, command}` runs any pi or `_oino/*` RPC command (`steer`, `follow_up`, `get_tree`, …). The SDK delivers extension methods without their leading `_`; the adapter restores it. | ACP v1 has no per-session routing for extension methods. | Clients add `sessionId`. |
| 7 | modes §6.3 tool kinds | Kinds are mapped by built-in tool name (`read`→read, `write`/`edit`→edit, `bash`/`powershell`→execute, `grep`/`find`/`ls`→search); every extension tool is `other`. | Core `ToolInfo` has no `kind` yet (runtime §7.4). | Extension tools render generically. |
| 8 | modes §6.3 `tool_execution_update` | When `bash` runs in a client terminal, `{type: terminal, terminalId}` is attached as soon as `terminal/create` returns and text partial results are not sent (the terminal shows the output); the final update keeps the terminal content, `rawOutput` holds the tool result (truncated output). | The client renders the live terminal. | none. |
| 9 | modes §6.3 commands | `available_commands_update` lists adapter built-ins `/compact [instructions]`, `/name <name>`, `/export [path]` (HTML), `/session` (stats), which the adapter runs through the session API (modes §1.3) and answers as agent text, followed by the session's extension commands, prompt templates and skills (`input.hint: "arguments"`). Other pi frontend built-ins map to ACP instead (`/model`, `/thinking` → config options; `/new`, `/resume`, `/fork` → session methods) and are not listed. | ACP has no built-in command surface. | none. |
| 10 | modes §6.3 (addition) | Plans: the fire-and-forget UI method `setPlan {entries: [{content, priority, status}]}` becomes a `plan` update. | Core has no plan concept; extensions (unipi workflow/todo) need a way to publish ACP plans. | ui-protocol should adopt `setPlan` (see requests); other frontends receive it as an unknown fire-and-forget request. |
| 11 | modes §6.4 dialogs | Elicitation forms have one property `value`: `select` (string enum), `input`/`editor` (string, `default` = prefill), `confirm` (boolean), `multiSelect` (array of enum). `form`, `custom`, `clipboardReadImage`, `editExternally` resolve `cancelled`; `approval.feedback` is not collected. `extension_ui_dismiss` is forwarded as `_oino/extension_ui`, but an open client request is not cancelled (`$/cancel_request` is not sent). | Restricted ACP form schemas; FormQuestion trees need a multi-step mapping. | unipi ask_user forms are unavailable over ACP. |
| 12 | modes §6.4 fs delegation | Reads go through `fs/read_text_file` only for text files (local sniff of the first 8 KiB: UTF-8 without NUL); binary files (images) are read locally. Client failures fall back to the local file system (Node-style errors). Writes of non-UTF-8 data are local. `access`/`stat`/`read_dir`/`realpath`/`mkdir_p` are always local. Client terminals are used only when no stdin is piped; `terminal/create.env` carries only variables that differ from the agent's environment; output is polled every 100 ms with `outputByteLimit` 16 MiB. Diff snapshots read the local file. | ACP fs is text-only; the agent runs on the same machine as the client. | none. |
| 13 | modes §6.3 `usage_update` | Sent after `turn_end`, `compaction_end` and `agent_settled` only when the context token count is known (it is `null` right after compaction). | `used` is required. | none. |
| 14 | modes §6.2 | The ACP session id is the Oino session id of the runtime's first session and stays fixed if the session is later replaced through `_oino/rpc` (`new_session`, `switch_session`, …). `session/list` titles fall back to the first line of the first message (≤ 80 chars). | ACP ids must stay stable for the client. | none. |
| 15 | modes §6.3 | Extension UI raised by `session_start` handlers before the adapter subscribes is not forwarded (recover with `_oino/rpc {command: {type: "_oino/ui_snapshot"}}`). | `AgentSessionRuntime::create` runs `session_start` before a backend exists (same as modes deviation 7). | none in practice. |
| 16 | modes §6.5 | Acceptance tests are scripted JSON-RPC transcripts with assertions (not byte-golden files) plus an interop test with the SDK's typed `Client` instead of `agent-client-protocol-test`; `crates/oino/tests/bundled.rs` drives `oino --mode acp` with the full bundle. Zed interop has not been tried by hand. | Event ids/timestamps vary; the typed client is the strict decoder. | none. |

### Interactive TUI (tui-client, frontend-cmds T1–T6)

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | architecture §2 (`oino-view` deps: `oino-protocol`) | `oino-view` and `oino-tui` depend on `oino-core` (types only: `events`, `ui`), `oino-ai`, `oino-session` and `oino-agent` for wire types. Neither calls any `oino-core` API. | `oino-protocol` does not exist yet; `oino-client` exposes `oino_core` types in its public API. | Switch to `oino-protocol` once it lands (mechanical import change). `cargo xtask layering` rule 5 still holds: no core internals are used. |
| 2 | modes §7.1 (`oino_tui::run(backend, TuiOptions)`) | `oino_tui::run(backend: Arc<dyn SessionBackend>, host: Arc<dyn TuiHost>, options)`. Panel actions, shortcuts, completions, editor intents, the extension settings hub and `/login` go through the backend (`FrontendApi`, T1–T5); `TuiHost` (`oino_tui::host::LocalHost`, feature `local-host`) still carries session listing, core settings writes, `keybindings.json`, clipboard images and renderer calls. | No `_oino/list_sessions`, `_oino/get_keybindings`, `_oino/render` yet. | `--remote` needs those commands. |
| 3 | modes §7.2 (fullscreen) | Default layout is the alternate screen with the client's own scrollback (pi `tui.altScreen.*`); the regular (inline) layout is `--tui-mode regular` / `tuiMode: "regular"` (T6). `previousPrompt`/`nextPrompt` scroll by half a page instead of jumping to prompt markers. | ratatui owns a frame. | — |
| 4 | ui-protocol §6.1 `image`, pi inline images | Images (user attachments, tool results, OUS `image`) render as `[image: <mime>]` placeholders; no kitty/iTerm/sixel output. Capability `images: false` is declared in `_oino/hello`. | Graphics protocols need per-terminal detection and raw escape output outside ratatui's cell buffer. | Visual only; data is unaffected. Follow-up: kitty graphics via a post-draw escape pass. |
| 8 | modes §1.3 built-ins | `/trust` and `/extensions` show an explanatory notice; `/reload` reloads keybindings, settings and the command list only; `/changelog` shows `TuiHost::changelog()` (none in `LocalHost`); `/share`, `/bug`, `/llama` are not offered (pi-only services). `/login` / `/logout` work (T4). | No backend commands for trust decisions, extension management or runtime reload. | Users edit `trust.json` or use `oino install`/`list`. |
| 9 | pi `app.model.cycleBackward` | Backward cycling selects the previous entry of `get_available_models` (not the scoped `--models` list). | RPC has only forward `cycle_model`. | With scoped models, backward cycling walks all available models. |
| 10 | pi session selector rename | Rename (`ctrl+r`) works for the current session only (`set_session_name`). | Renaming another file needs a session-file API outside the backend. | Other sessions show "Only the current session can be renamed here". |
| 11 | pi tree navigator | Selecting a different entry always asks "No summary / Summarize / Summarize with custom prompt" before `_oino/navigate_tree`; pi asks only when branch summaries are enabled. Label timestamps show `HH:MM` from the ISO time. | `branchSummary` settings are not exposed to frontends. | One extra confirmation when summaries are disabled. |
| 12 | pi footer | The cache-hit-rate segment (`CH…%`) and subscription marker are omitted; usage comes from `get_session_stats` (refreshed on `message_end`/`agent_settled`). | Needs per-message usage scanning that the backend does not expose. | Footer shows ↑ ↓ R W $ and context %, model, thinking level, routed model. |
| 13 | pi clipboard | Copy uses OSC 52; paste of images/text shells out to `wl-paste`/`xclip`/`pngpaste`/`pbpaste` (in `LocalHost`). Dropped image paths are attached as file tokens (`@path` on submit). | No native clipboard dependency (keeps the crate pure Rust, no system libs). | Terminals without OSC 52 do not receive copies. |
| 14 | pi edit diff | Diffs colour `+`/`-`/context lines; pi's intra-line inverse word highlighting is omitted. | No word-diff dependency in the client. | Visual only. |

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| T1 | tui-client #5/#6 | Every composer edit sends `_oino/get_completions`; backend groups other than `commands` (and `files` when a local popup is open) are merged into the popup and accepted with `Composer::replace_before_cursor(from, value)`. Local `/` and `@` completion stays (instant). | — | Extension argument and `complete-input` completions work. |
| T2 | tui-client #5 | Editor intents: `changed`/`cleared` debounced 150 ms (on ticks), `submit` before a prompt, `emptyArrowDown` on ↓ in an empty composer, `tokenBackspace` before deleting an attachment token (the deletion runs when the reply says nobody consumed it). | — | — |
| T3 | tui-client #7 | `/settings` lists the extension namespaces after the core rows; a namespace page (`oino_view::settings_hub`) renders sections/pages flattened, edits booleans/enums in place, text/number/secret/model fields in an inline input, runs `action` fields as `/command`, resets with Delete/ctrl+d, toggles the write scope with Tab (global-only/project-only fields ignore it). `multiselect`/`order` are shown read-only; no search, no advanced filter, no revert-on-open. `openSettings{namespace}` opens the page. | Scope of this pass. | Core settings still go through `TuiHost` (LocalHost). |
| T4 | tui-client #8 | `/login [provider]` lists OAuth providers (`_oino/auth_providers`) and runs `_oino/login` concurrently; `/logout [provider]` lists providers with stored credentials. | — | — |
| T5 | tui-client #2 | Panel actions and shortcuts go through the backend (`FrontendApi::ui_action` / `invoke_shortcut`); `TuiHost::ui_action` / `invoke_shortcut` were removed. | — | Works with `--remote` once it exists. |
| T6 | tui-client #3 | Regular (inline) layout: `--tui-mode regular` or setting `tuiMode: "regular"` (also a `/settings` row, next start). No alternate screen; an inline viewport (3/5 of the rows, ≥ 14) holds the live turn and the chrome; the header and finished rows are written above it (`Terminal::insert_before`) once the session is idle and never redrawn. | ratatui owns a frame; pi re-renders its components in place. | Rows of a running turn stay in the viewport until it settles; later changes to committed rows (labels, re-renders after resize) are not reflected in the scrollback. |
