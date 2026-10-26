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
| `_oino/bus_get` | `{topic}` | `{data | null}` |
| `_oino/navigate_tree` | `{targetId, summarize?, customInstructions?, replaceInstructions?, label?}` | `NavigateTreeResult` |
| `_oino/set_label` | `{entryId, label?}` | — |
| `_oino/import_jsonl` | `{path}` | `{cancelled}` |
| `_oino/export_jsonl` | `{outputPath?}` | `{path}` |
| `_oino/list_sessions` | `{scope: cwd\|all, cwd?}` | `{sessions: [SessionInfo]}` |
| `_oino/extensions` | — | `[{id, name, version, kind, enabled, source, permissions, error?}]` |
| `_oino/set_extension_enabled` | `{id, enabled}` | — (takes effect after reload) |
| `_oino/login` / `_oino/logout` | `{provider}` | runs provider OAuth via UI dialogs / clears auth |
| `_oino/set_api_key` | `{provider, key}` | stores in auth.json |
| `_oino/get_context_usage` | — | `ContextUsage` |
| `_oino/agents` | — | nested agents of the session (runtime §12) |

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
