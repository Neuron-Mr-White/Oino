# Frontend API for the GUI (gui-audit area E)

Work id `core-gui-gaps`. This is the core/backend surface the GPUI batches consume. Every
call works the same in-process (`oino_client::InProcessBackend`) and over RPC
(`oino_rpc::RpcBackend`, `oino --mode rpc`), because both implement
`oino_client::SessionBackend` and get the typed methods of `oino_client::FrontendApi`.
The wire shapes extend [modes.md](modes.md) §4. Events arrive on `SessionBackend::events()`
as `ClientEvent::Oino(OinoEvent::…)`. Over RPC they are only sent after `_oino/hello`.

Contents:

- [0. Wiring checklist for the GUI batches](#0-wiring-checklist-for-the-gui-batches)
- [1. Runtime-agnostic backend (GA-1 / GA-40)](#1-runtime-agnostic-backend-ga-1--ga-40)
- [2. Sticky topic feed: `_oino/topics`, `_oino/bus_get` (R3)](#2-sticky-topic-feed-_oinotopics-_oinobus_get-r3)
- [3. Nested agents: `_oino/agents` (GA-44 / GA-15, R5)](#3-nested-agents-_oinoagents-ga-44--ga-15-r5)
- [4. Single-item queue edits: `_oino/queue_edit` (design §8 row 33)](#4-single-item-queue-edits-_oinoqueue_edit-design-8-row-33)
- [5. File checkpoints, undo and rewind (rows 24–25)](#5-file-checkpoints-undo-and-rewind-rows-2425)
- [6. Bash detach: watchdog kill and background adopt](#6-bash-detach-watchdog-kill-and-background-adopt)
- [7. Extensions and `disabledExtensions`](#7-extensions-and-disabledextensions)
- [8. Shutdown (`ctx.shutdown()`)](#8-shutdown-ctxshutdown)
- [9. Extension shortcuts (GA-16 / GA-43)](#9-extension-shortcuts-ga-16--ga-43)
- [10. Frontend identity (GA-45)](#10-frontend-identity-ga-45)
- [11. Approval dialogs and handler deadlines (GA-30 / GA-42)](#11-approval-dialogs-and-handler-deadlines-ga-30--ga-42)
- [12. Companion binaries (GA-41)](#12-companion-binaries-ga-41)
- [13. Transcript render specs: `_oino/render`](#13-transcript-render-specs-_oinorender)
- [14. Not covered here](#14-not-covered-here)

## 0. Wiring checklist for the GUI batches

| GUI surface | Wire it to | Notes |
|---|---|---|
| Embedded host (`apps/oino-desktop/src/embedded.rs`) | `mode: ExtensionMode::Desktop` (mobile: `ExtensionMode::Mobile`). Send `_oino/hello{client:{name:"oino-desktop"}}` once per backend | GA-45: `/doctor` and `host-info.frontends` then report `desktop` |
| Composer edits, panel actions, shortcuts | Await `FrontendApi` futures on any executor (GPUI `background_executor`) | GA-1 / GA-40 is fixed in the backend. `session_pane.rs:966` no longer needs `Tokio::spawn` |
| Status panel, footer git, sidebar badges, Tasks tab (GA-14), mode chip (GA-25) | `backend.topics(&["unipi:*", "oino:git"], &[])` once, then `OinoEvent::Topic` | Subscribe again after `session_replaced` to get a fresh snapshot |
| AgentRow → Agent tab, live child timelines, btw / sidekick views (GA-15) | `backend.agents(Some(true))`, `OinoEvent::AgentsChanged`, `OinoEvent::AgentEvent`, `backend.agent_messages(id)` for the backlog | Feed the nested `ClientEvent`s into the timeline reducer, keyed by `agentId` |
| Queue panel buttons (remove, send now, drag reorder, edit) | `queue_remove` / `queue_send_now` / `queue_move` / `queue_set_text` | Indexes refer to the last `queue_update` lists |
| Turn undo / rewind buttons (rows 24–25) | `backend.checkpoints()` after `agent_settled`; `backend.rewind(entry_id, RewindOptions{…})` | Rewind with `conversation: true` replaces the session (`session_replaced{reason:"fork"}`); put `result.text` back in the composer |
| Watchdog kill, "move to background" on a running bash row | Show `backend.running_bash()`. Detach goes through the extensions (`/bg-detach`, watchdog `control.bash-detach`, `tasks.adopt-bash`) | The adopted task appears in `unipi:bg:tasks` |
| Extensions page enable / disable | `backend.extensions()`, `backend.set_extension_enabled(id, on)` | Applies to sessions or workspaces created afterwards; the core loader honours it |
| App exit on `ctx.shutdown()` | `backend.shutdown_requested().await` or `OinoEvent::ShutdownRequested` | Close the session pane (or the app when it was the only one) |
| Keyboard shortcuts (GA-16 / GA-43) | Retained `setShortcuts` UI request (also in `_oino/ui_snapshot`); `invoke_shortcut(id)` | Bind `keys` in `components/keys.rs`; list them in the palette and on Settings → Keyboard shortcuts |
| Approval cards (GA-30 / GA-42) | Nothing new; handle `extension_ui_dismiss{reason:"cancelled"}` | Handlers wait for the user without a deadline, and orphan cards are withdrawn |
| Kanboard daemon (GA-41) | Ship `oino-kanboard` next to `oino-desktop` | The host now finds programs in the directory of the running executable |

## 1. Runtime-agnostic backend (GA-1 / GA-40)

```rust
impl InProcessBackend {
    pub fn new(runtime: Arc<AgentSessionRuntime>) -> Self;          // captures the current tokio Handle
    pub fn with_server_info(runtime: Arc<AgentSessionRuntime>, server: ServerInfo) -> Self;
    pub fn runtime(&self) -> &Arc<AgentSessionRuntime>;
    pub fn state(&self) -> RpcSessionState;
}
impl SessionBackend for InProcessBackend { /* send, send_with_id, shutdown, shutdown_requested */ }
```

`send`, `send_with_id`, `shutdown` and `shutdown_requested` run on the tokio runtime the
backend was created on whenever they are awaited outside a tokio context, for example on a
GPUI executor or in `futures::executor::block_on`. Core background work started by frontend
calls (`editor_event` intents that are not consumable, `ui_action`, `invoke_shortcut`) is
spawned with `AgentSession::spawn_task`. That uses the session's stored runtime `Handle`, so
`tokio::spawn` can no longer panic off the runtime. Test: `commands_work_from_a_non_tokio_thread`
(`crates/oino-client/tests/in_process_gui.rs`).

## 2. Sticky topic feed: `_oino/topics`, `_oino/bus_get` (R3)

```rust
// oino_client::FrontendApi
fn topics(&self, subscribe: &[&str], unsubscribe: &[&str]) -> BoxFuture<'_, Result<Vec<TopicValue>, ClientError>>;
fn bus_get(&self, topic: &str) -> BoxFuture<'_, Result<Option<JsonValue>, ClientError>>;

// oino_core::topics (re-exported as oino_client::protocol::TopicValue)
pub struct TopicValue { pub topic: String, pub data: JsonValue, pub sticky: bool, pub publisher: String }
pub const GIT_TOPIC: &str = "oino:git";
pub const SETTINGS_TOPIC_PREFIX: &str = "oino:settings:";   // + <namespace>, every settings write
```

| Wire | Request | Response `data` |
|---|---|---|
| `_oino/topics` | `{subscribe?: [pattern], unsubscribe?: [pattern]}`. A pattern is an exact topic, `prefix*` or `*` | `{topics: [TopicValue], patterns: [string]}`: the sticky values matching the new subscription |
| `_oino/bus_get` | `{topic}` | `{data \| null, publisher?}` |
| event `topic` | — | `{type:"topic", topic, data, sticky, publisher}` for each publish that matches the subscription |

How it works:

- **Source.** Every extension `bus.publish` is mirrored into core: the WASM host `Composite`
  forwards to `CoreHost::bus_publish`, which sends `HostOp::BusPublish` to the `HostRouter`.
  Core keeps the last value of each sticky topic per services, which is per `(cwd, agent dir)`
  and the same scope as the host bus.
- **Fan-out.** Each publish is offered to every root session. A session emits `topic` only
  when the value matches its patterns. A sticky publish whose value did not change is not
  repeated.
- **Replacement.** Subscriptions move to the replacement session on `new`, `resume`, `fork`
  and `import`.

unipi sticky topics (`unipi_kit::topics::STICKY`):

- `unipi:long-horizon:state`, `unipi:long-horizon:progress`
- `unipi:workflow:status`: plan mode and permission mode. GA-25: drive the mode chip from
  this, not from the footer
- `unipi:bg:tasks`: `{tasks:[…]}` (GA-14 Tasks tab)
- `unipi:fusion:status`, `unipi:kanboard:status`, `unipi:subagents:list`, `unipi:footer:tps`

Non-sticky topics such as `unipi:work-tray:changed` and `unipi:module:ready` stream while
subscribed but have no snapshot.

Core topic `oino:git` (publisher `oino.core`):
`{branch: string|null, head: string|null, dirty: bool, changed: n, ahead: n, behind: n}`,
or `null` outside a repository. It is recomputed when a subscription matches it and after
every `agent_settled`.

## 3. Nested agents: `_oino/agents` (GA-44 / GA-15, R5)

```rust
fn agents(&self, follow: Option<bool>) -> BoxFuture<'_, Result<AgentTree, ClientError>>;
fn agent_messages(&self, agent_id: &str) -> BoxFuture<'_, Result<Vec<AgentMessage>, ClientError>>;
fn agent_abort(&self, agent_id: &str) -> BoxFuture<'_, Result<(), ClientError>>;

pub struct AgentTree { pub agents: Vec<AgentInfo>, pub follow: bool }
pub struct AgentInfo {               // oino_core::agents, camelCase on the wire
    pub id: String,                  // child session id
    pub parent_id: Option<String>,   // None = child of the root session
    pub label: String,               // "Subagent …", "Sidekick", "btw", "session name", …
    pub owner: String,               // spawning extension (unipi.subagents, unipi.fusion, unipi.btw, …)
    pub role: Option<String>,        // AgentSpec.childRole, else the owner
    pub state: AgentRunState,        // running | idle | closed
    pub depth: u8,
    pub session_file: Option<String>,
}
// core: AgentSession::{agent_tree, find_agent, follow_agents, follows_agents}
```

| Wire | Request | Response `data` / event |
|---|---|---|
| `_oino/agents` | `{follow?: bool}` | `{agents: [AgentInfo], follow}` |
| `_oino/agent_messages` | `{agentId}` | `{messages: [AgentMessage]}`: the child transcript, for a late-opened tab |
| `_oino/agent_abort` | `{agentId}` | — |
| event `agents_changed` | — | `{agents}`: the whole tree after a spawn or close, and when a child starts or settles |
| event `agent_event` | — | `{agentId, event: ClientEvent}`: only while `follow` is on |

- **Scope.** `follow` applies to the whole tree of the root session and carries over on
  session replacement.
- **Over RPC.** A nested `message_update` travels as a delta, like top-level ones.
  `RpcBackend`'s `EventDecoder` rebuilds the partial message for each agent, so in-process
  and RPC consumers see the same `ClientEvent`s. Consecutive child `message_update`s are
  coalesced for slow subscribers.

## 4. Single-item queue edits: `_oino/queue_edit` (design §8 row 33)

```rust
fn queue_edit(&self, edit: QueueEdit) -> BoxFuture<'_, Result<(), ClientError>>;
fn queue_remove(&self, queue: QueueKind, index: usize) -> …;
fn queue_send_now(&self, queue: QueueKind, index: usize) -> …;
fn queue_move(&self, queue: QueueKind, from: usize, to: usize) -> …;
fn queue_set_text(&self, queue: QueueKind, index: usize, text: &str) -> …;

pub enum QueueKind { Steering, FollowUp }                          // "steering" | "followUp"
pub enum QueueEdit {                                               // {op, queue, …}, op camelCase
    Remove { queue, index }, Edit { queue, index, text }, Move { queue, from, to }, SendNow { queue, index },
}
// core: AgentSession::edit_queue(QueueEdit) -> Result<(), SessionOpError>
```

Wire: `{"type":"_oino/queue_edit","op":"move","queue":"followUp","from":2,"to":0}`. After
each edit a `queue_update` event is published.

- **Indexes** refer to the lists in the last `queue_update`.
- **`edit`** keeps the item's images and expands prompt templates.
- **`sendNow`** while running moves the item to the front of the steering queue, so it is
  delivered after the current tool calls. While idle it removes the item and prompts it.
- **Errors:** `Queue index {i} out of range ({n} queued)`.

Each queued text is matched to its agent message (user messages in order). Custom messages
that extensions queued are left untouched.

## 5. File checkpoints, undo and rewind (rows 24–25)

```rust
fn checkpoints(&self) -> BoxFuture<'_, Result<Vec<Checkpoint>, ClientError>>;
fn rewind(&self, entry_id: &str, options: RewindOptions) -> BoxFuture<'_, Result<RewindResult, ClientError>>;

pub struct RewindOptions { pub files: bool, pub conversation: bool }   // Default: both true
pub struct Checkpoint { pub entry_id: String, pub created_at: i64, pub files: Vec<CheckpointFile> }
pub struct CheckpointFile { pub path: String, pub existed: bool }
pub struct RewindResult {
    pub restored: Vec<String>, pub deleted: Vec<String>, pub failed: Vec<RewindFailure>,
    pub text: Option<String>,    // the user message text, to refill the composer
    pub cancelled: bool,         // session_before_fork cancelled the fork
}
// core: AgentSession::{checkpoints, rewind_files, has_checkpoint}
```

| Wire | Request | Response `data` |
|---|---|---|
| `_oino/checkpoints` | — | `{checkpoints: [Checkpoint]}`, oldest first |
| `_oino/rewind` | `{entryId, files?: true, conversation?: true}` | `RewindResult` |

**Recording.** Before a built-in `write` or `edit` runs (after `tool_call` handlers, unless
blocked), core snapshots the target file once per checkpoint. A checkpoint is keyed by the
entry of the user message that started the run. A file that did not exist is recorded as
absent.

- **Storage.** Persisted sessions write to `<agentDir>/checkpoints/<sessionId>/`
  (`index.json`, `blobs/`). In-memory sessions keep checkpoints in memory.
- **Limits.** At most 100 checkpoints per session, and files up to 16 MiB.
- **Not tracked:** `bash` and extension tools.

**Rewind** to user message X:

1. With `files: true`, every file touched at or after X's checkpoint gets its content from
   before X back. Files created after X are deleted. Those checkpoints are dropped.
2. With `conversation: true`, the session forks before X (pi `fork`) and the backend rebinds
   (`session_replaced{reason:"fork"}`). The new session keeps the checkpoints of the entries
   it kept.

The call is refused while the agent runs. With `files: true, conversation: false` and no
checkpoint for X, it fails with `No file checkpoint for entry X`.

- **Undo last turn:** rewind to the last user message.
- **Files only:** `{conversation: false}`.

## 6. Bash detach: watchdog kill and background adopt

| Layer | API |
|---|---|
| WIT (extensions) | `control.running-bash() -> [{toolCallId, command, startedAt, pid?}]`, `control.bash-detach(toolCallId, reason) -> taskId`, `tasks.adopt-bash(toolCallId, reason) -> taskId` |
| `oino-extension-api` | `HostServices::take_bash(ext, cx, tool_call_id, task_id, reason) -> DetachedBash`, `DetachedBash { command, pid, output_so_far, full_output_path, output: BoxStream<Vec<u8>>, exit: BoxFuture<Option<i32>>, kill: CancellationToken }` |
| `oino-core` | `HostApi::detach_bash`, `AgentSession::{running_bash, detach_bash, bind_bash_detach}`, `HostOp::RunningBash`, `RunningBash`, `DetachedBash` |
| `oino-tools` | `BashDetachRegistry::{pid, running}` (process id of a running call) |
| `oino-wasm-host` | `LocalServices` adopts the process as a background task (`TaskService::adopt`); the task log starts with the output so far |
| Frontend | `FrontendApi::running_bash() -> Vec<RunningBash>` (`_oino/running_bash` → `{calls}`) |

Flow:

1. Watchdog or background-tasks calls `bash-detach` or `adopt-bash`.
2. The host allocates a task id.
3. Core makes the running `bash` tool call return at once, with its output so far plus
   `[Detached to background task <id>: <reason>]`. The process keeps running.
4. The host task service owns the process. `tasks.kill(id)` kills the process tree, which is
   watchdog's kill action. Completion delivers `oino_task` and the usual
   `background-task-notification`.

`SessionToolContext::bind` wires each session's shell tools to its detach registry. Every
host that builds sessions with `builtin_tool_entries` gets this for free, including desktop
`embedded.rs`.

## 7. Extensions and `disabledExtensions`

```rust
fn extensions(&self) -> BoxFuture<'_, Result<Vec<ExtensionInfo>, ClientError>>;
fn set_extension_enabled(&self, id: &str, enabled: bool) -> BoxFuture<'_, Result<(), ClientError>>;
pub struct ExtensionInfo { pub id: String, pub kind: ExtensionKind, pub enabled: bool, pub loaded: bool, pub path: Option<String> }
// core loader: ExtensionLoader::load_enabled(host, project_trusted, disabled) (default: load + filter)
```

`Services::create` reads `disabledExtensions` from the merged settings:

- `CoreLoader` and the CLI loader skip those packages before instantiating them.
- Any other loader's instances with those ids are dropped before `init`.

`_oino/set_extension_enabled` writes the global `disabledExtensions`. It takes effect for
services created afterwards, which means a new desktop workspace or a restart.

## 8. Shutdown (`ctx.shutdown()`)

- `SessionBackend::shutdown_requested() -> BoxFuture<'static, ()>`. The default never
  resolves. In-process, it resolves once requested and the session is idle. Over RPC, it
  resolves on the event.
- The `shutdown_requested` event is published once when an extension calls
  `control.shutdown`.
- **Modes:** RPC (`RpcServer::run` returns), TUI (interactive mode) and print / JSON exit
  with code 0 once idle. ACP leaves its lifecycle to the client.

## 9. Extension shortcuts (GA-16 / GA-43)

Core now fires the retained UI request `setShortcuts` ([ui-protocol.md](ui-protocol.md) §4)
after `session_start` and on every `register-shortcut` or `unregister-shortcut`:
`{method:"setShortcuts", shortcuts:[{id, keys, description, extensionId}]}`. It is part of
`_oino/ui_snapshot`. `_oino/get_shortcuts` is unchanged.

## 10. Frontend identity (GA-45)

- `ExtensionMode` gains `Desktop` and `Mobile`, which map to WIT `mode::desktop` and
  `mode::mobile`.
- `host-info.get().frontends` = the mode's frontend (`desktop`, `mobile`, `tui`, `rpc`,
  `acp`) plus every `_oino/hello` `client.name` (`AgentSession::register_frontend`).
- `AgentSession::frontends()` returns the same list.

## 11. Approval dialogs and handler deadlines (GA-30 / GA-42)

- **No deadline while the user decides.** A handler whose extension has an open dialog
  (`approval`, `form`, `select`, …) does not time out. Both deadlines pause while the dialog
  is open: core's runner deadline (`extensions::deadline::with_deadline`, keyed by the
  extension's open dialogs on the session `UiBridge`) and the WASM host's `on-event`
  deadline (per instance `dialogs_open`). After the dialog closes, the handler gets a fresh
  deadline.
- **No orphan cards.** When a waiting handler is dropped or times out, its dialogs are
  withdrawn: `extension_ui_dismiss{reason:"cancelled"}`, and they are removed from
  `_oino/ui_snapshot`. API: `UiBridge::{pending_for, cancel_for}`.

## 12. Companion binaries (GA-41)

The WASM host resolves a bare program name that is not on `PATH` against the directory of
the running executable (`process::resolve_program`). Packaging must put `oino-kanboard`
(`.exe`) next to `oino-desktop`; the app bundle and AppImage need the same.

## 13. Transcript render specs: `_oino/render`

```rust
// oino_client::FrontendApi
fn render(&self, request: RenderRow) -> BoxFuture<'_, Result<Option<JsonValue>, ClientError>>;

pub struct RenderRow {  // ui-protocol §6.4
    pub target: String,          // customMessage | customEntry | tool
    pub key: String,             // customType or tool name (built-in names included)
    pub entry_id: Option<String>,
    pub density: Option<String>, // simple | regular | advanced
    pub expanded: bool,
    pub width: Option<String>,   // narrow | normal | wide
    pub data: JsonValue,
}
```

| Wire | Request | Response `data` |
|---|---|---|
| `_oino/render` | the `RenderRow` fields | `{node: OusNode \| null}`; `null` = the frontend renders the row itself |

`AgentSession::render_spec` dispatches to the extension that registered the render spec for
`target`/`key` (`guest.render`, call kind `render`) and passes the request JSON as
`{target, key, entryId, density, expanded, width, data}`. Render specs for the **built-in**
tools (`read`, `write`, `edit`, `bash`, `grep`, `find`, `ls`) are render-only overrides: core
accepts them when the manifest lists the tool in `tools.override` and drops them otherwise
(U1–U2, `utility.render.style`). The TUI asks for every tool row through this path when its
host has a session (`LocalHost::with_render`); the GPUI app still renders tool rows natively.

## 14. Not covered here

These are GUI-side or tracked elsewhere:

- GA-46: desktop perf, the GPUI refresh loop.
- The other GA items in areas A–D.
- `oino serve` / `--remote`.
- `set-ui-policy`.
- Render-only `tools.override`.
