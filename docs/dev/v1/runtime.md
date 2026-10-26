# Runtime (`oino-core` and its libraries)

Status: binding. Owner: `core-runtime` (`crates/oino-agent`, `oino-tools`, `oino-resources`,
`oino-settings`, `oino-models`, `oino-core`). Types come from `oino-ai`/`oino-session`
([session-v3.md](session-v3.md)) and `oino-protocol`. Extension contracts come from
`oino-extension-api` ([extension-api.md](extension-api.md)).

**Parity rule.** Every behaviour in audit-pi §4–§20 is in scope and must match pi 1.4.0
(strings, ordering, defaults, JSON) except the explicit Oino deltas listed in §16. Where this
spec abbreviates, the pi source under `PI/dist/core/**` is normative; implementers read it.

Internal sub-order for core-runtime (one item, several crates): `oino-agent` → `oino-tools`
→ `oino-settings` → `oino-resources` → `oino-models` → `oino-core`. Each crate lands with
tests before the next depends on it.

## 1. Agent loop (`oino-agent`, pi-agent-core)

### 1.1 Tool contract

```rust
pub struct ToolDefinitionMeta {             // what the loop needs; core adds prompt/exposure fields
    pub name: String, pub label: String, pub description: String,
    pub parameters: serde_json::Value,       // JSON Schema (draft 2020-12 subset TypeBox emits)
    pub output_schema: Option<serde_json::Value>,
    pub execution_mode: ExecutionMode,       // Parallel (default) | Sequential
    pub replay: Option<Replay>,              // Never | Safe
    pub constrained_sampling: Option<ConstrainedSampling>,
}
pub struct AgentToolResult {
    pub content: Vec<UserContent>, pub details: Option<serde_json::Value>,
    pub structured_content: Option<serde_json::Value>, pub usage: Option<Usage>,
    pub is_error: bool, pub terminate: bool,
}
pub type ToolUpdate = AgentToolResult;      // partial results
pub trait AgentTool: Send + Sync {
    fn meta(&self) -> &ToolDefinitionMeta;
    fn prepare_arguments(&self, raw: serde_json::Value) -> serde_json::Value { raw }
    fn execute(&self, call: ToolInvocation, updates: UpdateSink, cancel: CancellationToken)
        -> BoxFuture<'static, Result<AgentToolResult, ToolError>>;
}
pub struct ToolInvocation { pub tool_call_id: String, pub args: JsonObject, pub parent_tool_call_id: Option<String> }
```

Pipeline per tool call (pi `runToolCall`, never returns `Err` to the loop):
`argumentsError` set ⇒ error result `argumentsError` text; unknown tool ⇒ error
`Tool {name} not found`; `prepare_arguments`; full JSON-Schema validation (crate
`jsonschema`, draft 2020-12) — failure text identical to pi (`Validation failed for tool "{name}":\n{errors}` with pi's
error formatting, verified by golden test); `before_tool_call` hook → `{block, reason,
terminate}` (blocked ⇒ error result with `reason` or `Tool call blocked`); `execute` with
monotonic `durationMs`; `ToolError` ⇒ error result with the error's display text;
`after_tool_call` hook → field-wise override (`content`, `details`, `structuredContent`,
`isError`, `usage`, `terminate`; replacing `content` without `structuredContent` drops
`structuredContent`). Updates after settle are ignored.

### 1.2 Loop config and events

```rust
pub struct AgentLoopConfig {
    pub model: Model, pub thinking_level: ThinkingLevel, pub stream_options: StreamOptions,
    pub stream: Arc<dyn StreamFn>,                       // provider dispatch (core supplies; resolves component)
    pub convert_to_llm: Arc<dyn Fn(&[AgentMessage]) -> Vec<Message> + Send + Sync>,
    pub transform_context: Option<Arc<dyn TransformContext>>,
    pub get_api_key: Arc<dyn GetApiKey>,                 // per request (short-lived OAuth)
    pub before_tool_call: Option<Arc<dyn BeforeToolCall>>, pub after_tool_call: Option<Arc<dyn AfterToolCall>>,
    pub finish_turn: Option<Arc<dyn FinishTurn>>,        // -> Continue | End | None
    pub prepare_request: Option<Arc<dyn PrepareRequest>>,// before every request incl. first
    pub prepare_next_turn: Option<Arc<dyn PrepareNextTurn>>, // after turn_end when continuing
    pub get_steering_messages: Arc<dyn QueueSource>, pub get_follow_up_messages: Arc<dyn QueueSource>,
    pub tool_execution: ToolExecution,                   // Parallel (default) | Sequential
    pub on_payload: Option<Arc<dyn OnPayload>>, pub on_response: Option<Arc<dyn OnResponse>>,
    pub on_provider_stream_event: Option<Arc<dyn OnProviderStreamEvent>>,
}
pub trait StreamFn: Send + Sync {
    fn stream(&self, model: &Model, ctx: Context, opts: StreamOptions, cancel: CancellationToken)
        -> BoxStream<'static, AssistantMessageEvent>;     // always ends with Done or Error
}
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {   // pi AgentEvent (oino_protocol re-exports with parentToolCallId)
    AgentStart, AgentEnd { messages: Vec<AgentMessage> },
    TurnStart, TurnEnd { message: AgentMessage, tool_results: Vec<ToolResultMessage> },
    MessageStart { message: AgentMessage }, MessageUpdate { message: AgentMessage, assistant_message_event: AssistantMessageEvent },
    MessageEnd { message: AgentMessage },
    ToolExecutionStart { tool_call_id: String, tool_name: String, args: serde_json::Value },
    ToolExecutionUpdate { tool_call_id: String, tool_name: String, args: serde_json::Value, partial_result: serde_json::Value },
    ToolExecutionEnd { tool_call_id: String, tool_name: String, result: serde_json::Value, is_error: bool, duration_ms: Option<u64> },
}
pub async fn agent_loop(prompts: Vec<AgentMessage>, ctx: AgentContext, cfg: &AgentLoopConfig,
    sink: &dyn EventSink, cancel: CancellationToken) -> Vec<AgentMessage>;
pub async fn agent_loop_continue(ctx: AgentContext, cfg: &AgentLoopConfig, sink: &dyn EventSink, cancel: CancellationToken) -> Vec<AgentMessage>;
pub async fn run_tool_call(call: ToolCall, opts: RunToolCallOptions<'_>) -> ToolCallOutcome; // {tool_call, result, is_error, duration_ms}
```

Rules (pi): no turn cap (Oino `max_turns` removed); message lifecycle events fire for user and
toolResult messages too; the assistant `message_start` carries `stopReason: "pending"`;
parallel mode prepares sequentially, executes allowed calls concurrently, emits
`tool_execution_end` in completion order and toolResult `message_*` in assistant source order;
any `Sequential` tool in a batch runs the whole batch sequentially; batch terminates only if
every finalized result has `terminate`; steering polled after the batch unless `finish_turn`
ended; follow-ups polled only when no tool calls and no steering; `finish_turn → Continue` is
satisfied by already-scheduled work and never adds an extra request; error/aborted stop
reasons are hard exits. `EventSink::emit` is awaited (listeners run in order).

### 1.3 `Agent` (stateful wrapper)

`Agent` mirrors pi `agent.d.ts`: state (`system_prompt` read-only replay, `model`,
`thinking_level`, `tools` copy-on-set, `messages` copy-on-set, `is_streaming`,
`streaming_message`, `pending_tool_calls`, `error_message`), `subscribe(listener) ->
Subscription` (drop = unsubscribe), `steering_mode`/`follow_up_mode` (default
**`OneAtATime`**; `All`), `steer`, `follow_up`, `clear_steering_queue`, `clear_follow_up_queue`,
`clear_all_queues`, `has_queued_messages`, `peek_queued_messages`, `abort`, `wait_for_idle`
(returns after `agent_end` listeners), `reset`, `prompt(Vec<AgentMessage>)`, `continue_run()`
(last message must convert to user/toolResult, else `AgentError::CannotContinue`). Tool set
changes are announced to the model by a system patch message before the next request
(`oino_ai::transcript::diff`).

## 2. `AgentSession` (`oino-core::session`)

```rust
pub struct AgentSession { /* Arc<Inner>; cheap clone */ }
pub enum PromptDisposition { Handled, Queued, Started }        // serde lowercase
pub enum StreamingBehavior { Steer, FollowUp }                 // serde "steer" | "followUp"
pub enum Delivery { Steer, FollowUp, NextTurn }                // serde "steer" | "followUp" | "nextTurn"
pub enum InputSource { Interactive, Rpc, Extension }
pub struct PromptOptions { pub expand_prompt_templates: bool /* default true */, pub images: Vec<ImageContent>,
    pub streaming_behavior: Option<StreamingBehavior>, pub source: InputSource, pub provenance: Option<Provenance> }

impl AgentSession {
    // events
    pub fn subscribe(&self) -> EventStream;                       // ClientEvent stream (§6), bounded, coalescing (architecture §4.2)
    // state (read-only snapshots)
    pub fn state(&self) -> SessionStateSnapshot;                  // == RpcSessionState + extras (modes §3.2)
    pub fn model(&self) -> Option<Model>;  pub fn routed_model(&self) -> Option<Model>;
    pub fn thinking_level(&self) -> ThinkingLevel;  pub fn is_streaming(&self) -> bool;  pub fn is_idle(&self) -> bool;
    pub fn system_prompt(&self) -> String;  pub fn messages(&self) -> Vec<AgentMessage>;
    pub fn session_file(&self) -> Option<PathBuf>;  pub fn session_id(&self) -> String;  pub fn session_name(&self) -> Option<String>;
    pub fn pending_message_count(&self) -> usize;  pub fn retry_attempt(&self) -> u32;
    pub fn is_compacting(&self) -> bool;  pub fn is_retrying(&self) -> bool;  pub fn is_bash_running(&self) -> bool;
    pub fn context_usage(&self) -> ContextUsage;                  // {tokens: Option<u64>, contextWindow, percent: Option<f64>}
    pub fn session_stats(&self) -> SessionStats;
    pub fn cache_warming_status(&self) -> CacheWarmingStatus;
    pub fn last_assistant_text(&self) -> Option<String>;
    pub fn user_messages_for_forking(&self) -> Vec<ForkMessage>;  // {entryId, text}
    pub fn commands(&self) -> Vec<SlashCommandInfo>;
    pub fn session(&self) -> ReadonlySessionSnapshot;             // entries, tree, leaf (for get_entries/get_tree)
    // input
    pub async fn prompt(&self, text: String, opts: PromptOptions) -> Result<PromptDisposition, SessionOpError>;
    pub async fn steer(&self, text: String, images: Vec<ImageContent>, source: InputSource) -> Result<PromptDisposition, SessionOpError>;
    pub async fn follow_up(&self, text: String, images: Vec<ImageContent>, source: InputSource) -> Result<PromptDisposition, SessionOpError>;
    pub async fn send_custom_message(&self, msg: CustomMessageDraft, trigger_turn: bool, deliver_as: Option<Delivery>) -> Result<(), SessionOpError>;
    pub async fn send_user_message(&self, content: UserContentField, deliver_as: Option<Delivery>, expand_prompt_templates: bool, provenance: Option<Provenance>) -> Result<(), SessionOpError>;
    pub fn clear_queue(&self) -> ClearedQueues;                   // {steering: Vec<String>, followUp: Vec<String>}
    pub fn steering_messages(&self) -> Vec<String>;  pub fn follow_up_messages(&self) -> Vec<String>;
    pub async fn abort(&self);  pub async fn wait_for_idle(&self);
    // model / thinking / queues
    pub async fn set_model(&self, model: ModelKey, persist: bool) -> Result<Model, SessionOpError>;
    pub async fn cycle_model(&self, dir: CycleDirection) -> Result<Option<CycleModelResult>, SessionOpError>;
    pub async fn set_thinking_level(&self, level: ThinkingLevel) -> Result<ThinkingLevel, SessionOpError>; // returns clamped
    pub async fn cycle_thinking_level(&self) -> Option<ThinkingLevel>;
    pub fn available_thinking_levels(&self) -> Vec<ThinkingLevel>;
    pub fn set_steering_mode(&self, m: QueueMode);  pub fn set_follow_up_mode(&self, m: QueueMode);
    pub fn set_scoped_models(&self, scope: Vec<ScopedModel>);
    // tools
    pub fn active_tool_names(&self) -> Vec<String>;  pub fn callable_tool_names(&self) -> Vec<String>;
    pub fn all_tools(&self) -> Vec<ToolInfo>;  pub fn set_active_tools_by_name(&self, names: &[String]);
    // compaction / retry / bash / tree
    pub async fn compact(&self, custom_instructions: Option<String>) -> Result<CompactionResult, SessionOpError>;
    pub fn abort_compaction(&self);  pub fn abort_branch_summary(&self);  pub fn set_auto_compaction_enabled(&self, on: bool);
    pub fn abort_retry(&self);  pub fn set_auto_retry_enabled(&self, on: bool);
    pub async fn execute_bash(&self, command: String, exclude_from_context: bool, id: Option<String>) -> Result<BashResult, SessionOpError>;
    pub fn abort_bash(&self);
    pub async fn set_session_name(&self, name: Option<String>) -> Result<(), SessionOpError>;
    pub async fn set_label(&self, entry: EntryId, label: Option<String>) -> Result<(), SessionOpError>;
    pub async fn navigate_tree(&self, target: EntryId, opts: NavigateTreeOptions) -> Result<NavigateTreeResult, SessionOpError>;
    // export / lifecycle
    pub async fn export_html(&self, path: Option<PathBuf>) -> Result<PathBuf, SessionOpError>;
    pub async fn export_jsonl(&self, path: Option<PathBuf>) -> Result<PathBuf, SessionOpError>; // header + current branch
    pub async fn reload(&self) -> Result<(), SessionOpError>;
    pub async fn dispose(&self);
    // frontend → extension UI plumbing (ui-protocol §3)
    pub fn ui(&self) -> UiBridge;
}
```

Semantics are pi's `agent-session.d.ts` (audit-pi §5.1) member-for-member. Notable exact
behaviours: `prompt` while streaming without `streaming_behavior` ⇒
`SessionOpError::StreamingWithoutBehavior` (RPC error text pi's); extension commands run
immediately even while streaming; `steer`/`follow_up` reject extension commands
(`SessionOpError::CommandNotAllowed`); `send_custom_message` four cases (streaming→queue;
`nextTurn` waits for next user prompt; streaming+!trigger→appended after current turn's tool
results; idle+trigger→append + run with `before_agent_start{messageType:"custom"}`;
idle+!trigger→append only); `send_user_message` always triggers; `set_model` validates auth,
appends `model_change`, emits `model_select{source:"set"}`, persists default only when
`persist`; `set_thinking_level` clamps, appends `thinking_level_change` only if changed;
`compact` aborts the current run first and never retries/continues.

`Provenance` (Oino addition replacing unipi harness tags, audit-unipi §1.3.10):
`{extensionId, source, title?, synopsis?, severity?, delivery}` — `extensionId` and `delivery`
are filled by the host from the caller and the send options; stored on the persisted message as
`details.oinoProvenance` for `custom` messages and in a sibling `custom` entry
(`customType: "oino.provenance"`, `data: {targetId, provenance}`) for user messages, so the
pi-shaped message is unchanged. Frontends render provenance from either.

## 3. Prompt pipeline (`AgentSession::prompt`)

1. If text starts with `/` and names a registered **extension command** (invocation name,
   §7.4) ⇒ run its handler now with an `ExtensionCommandContext`; return `Handled`.
2. Dispatch `input` (`{text, images?, source, streamingBehavior?}`): chained; `transform`
   replaces text/images for the next handler; `handled` stops ⇒ return `Handled`.
3. If `expand_prompt_templates`: `/skill:name args` ⇒ skill block (§9.6);
   `/template args` ⇒ template expansion (§9.6). Built-in interactive commands
   (`/model`, `/compact`, …) are **not** handled here; frontends map them (modes §1.3).
4. Streaming: with `streaming_behavior` ⇒ queue (steer/follow-up), emit `queue_update`,
   return `Queued`; without ⇒ error.
5. Idle: validate model + credentials (`SessionOpError::NoModel` / `NoApiKey{provider}` with pi's
   texts); compaction pre-check (§11.1 trigger a); build prompt state (§9.4) and dispatch
   `before_agent_start` (`messageType:"user"`), apply mutated `systemPromptOptions`, optional
   returned `message` (custom draft appended after the user message) and `systemPrompt`
   (forced, not recorded, §9.4); persist pending system patch; start the run; return `Started`.

## 4. `AgentSessionRuntime` and `Services` (`oino-core::runtime`, `::services`)

```rust
pub struct ServicesOptions { pub cwd: PathBuf, pub agent_dir: PathBuf, pub mode: ExtensionMode /* tui|rpc|json|print|acp */,
    pub flags: CliResourceFlags, pub extension_flag_values: IndexMap<String, FlagValue>,
    pub extension_runtime: Arc<dyn ExtensionRuntime>, pub bundled: Arc<dyn BundledSource>,
    pub env: Arc<dyn ExecutionEnv>, pub offline: bool, pub approve: Option<bool> /* -a / -na */ }
pub struct Services { pub cwd: PathBuf, pub agent_dir: PathBuf, pub settings: SettingsManager,
    pub resources: ResourceLoader, pub models: ModelRegistry, pub packages: PackageManager,
    pub extensions: ExtensionRunner, pub diagnostics: Vec<Diagnostic> }
impl Services { pub async fn create(opts: ServicesOptions) -> Result<Arc<Services>, ServicesError>; }

pub struct SessionOptions { pub session: SessionSource /* New | Open(PathBuf) | ContinueRecent | Resume(id) | Fork(PathBuf) | InMemory | Id(String) */,
    pub session_dir: Option<PathBuf>, pub name: Option<String>, pub model: Option<ModelSelector>,
    pub thinking_level: Option<ThinkingLevel>, pub scoped_models: Vec<ScopedModel>,
    pub tools: ToolSelection /* default | list with +/- globs | none | no-builtin */, pub exclude_tools: Vec<String>,
    pub custom_tools: Vec<Arc<dyn AgentTool>>, pub ui: Arc<dyn UiHost> }

pub struct AgentSessionRuntime { /* owns current AgentSession, rebinds subscribers */ }
impl AgentSessionRuntime {
    pub async fn create(services: Arc<Services>, opts: SessionOptions) -> Result<Self, ServicesError>;
    pub fn session(&self) -> AgentSession;                     // current
    pub fn subscribe(&self) -> EventStream;                    // survives session replacement
    pub async fn new_session(&self, parent: Option<PathBuf>) -> Result<Cancelled, SessionOpError>;
    pub async fn switch_session(&self, path: PathBuf, cwd_override: Option<PathBuf>) -> Result<Cancelled, SessionOpError>;
    pub async fn fork(&self, entry: EntryId, position: ForkPosition /* Before | At */) -> Result<ForkResult, SessionOpError>; // {cancelled, selectedText}
    pub async fn clone_session(&self) -> Result<Cancelled, SessionOpError>;   // fork at leaf, position At
    pub async fn import_jsonl(&self, path: PathBuf, cwd_override: Option<PathBuf>) -> Result<Cancelled, SessionOpError>;
    pub async fn dispose(&self);
}
pub async fn create_agent_session(opts: SdkOptions) -> Result<AgentSessionRuntime, ServicesError>; // one-call SDK
```

Sequencing is pi's (audit-pi §5.4): `session_before_switch{reason}` (cancellable) →
`session_shutdown{reason, targetSessionFile}` → new `AgentSession` →
`session_start{reason, previousSessionFile}` → subscribers rebound → a
`ClientEvent::SessionReplaced{sessionFile, sessionId}` (Oino addition) is emitted so clients
reset view state. Fork: `session_before_fork{entryId, position}` (cancel,
`skipConversationRestore`); `Before` ⇒ new file with path to the parent of the user message,
selected text returned; `At` ⇒ path including the entry.
Startup order: `project_trust` → settings/resources load → extensions load (`init`) →
`session_start{reason:"startup"}` → `resources_discover{reason:"startup"}` → model restore
(`model_select{source:"restore"}`).

## 5. Queues, abort, idle

`is_idle` = no run, no compaction, no branch summary, no retry wait, no queued continuation.
`agent_settled{aborted}` is emitted exactly once when the session becomes idle after any
activity. `abort()` cancels the run, waits for idle, discards nothing from queues (pi).

## 6. Client events (`oino_protocol::events`)

`ClientEvent` = `#[serde(untagged)]` over `AgentSessionEvent`, `ExtensionUiRequest`
(ui-protocol §4), `ExtensionErrorEvent`, `OinoEvent`. All are `type`-tagged objects.

### 6.1 `AgentSessionEvent` (pi, exact)

| `type` | Payload | Notes |
|---|---|---|
| `agent_start` | — | |
| `agent_end` | `messages`, `willRetry` | |
| `agent_settled` | `aborted` | |
| `turn_start` | — | (extension event adds `turnIndex`, `timestamp`; client event has none) |
| `turn_end` | `message`, `toolResults` | |
| `message_start` / `message_end` | `message` | all roles |
| `message_update` | `message`, `assistantMessageEvent` | in-process; JSON/RPC wire form drops `message` and every `partial`, adds `usage` (modes §3.1) |
| `tool_execution_start` | `toolCallId`, `toolName`, `args`, `parentToolCallId?` | |
| `tool_execution_update` | `toolCallId`, `toolName`, `args`, `partialResult`, `parentToolCallId?` | |
| `tool_execution_end` | `toolCallId`, `toolName`, `result`, `isError`, `durationMs?`, `parentToolCallId?` | |
| `queue_update` | `steering: string[]`, `followUp: string[]` | complete queues |
| `compaction_start` | `reason: manual\|threshold\|overflow` | |
| `compaction_end` | `reason`, `result?`, `aborted`, `willRetry`, `errorMessage?` | |
| `entry_appended` | `entry` | only for extension `append-entry` |
| `session_info_changed` | `name?` | absent = cleared |
| `thinking_level_changed` | `level` | |
| `auto_retry_start` | `attempt`, `maxAttempts`, `delayMs`, `errorMessage` | |
| `auto_retry_end` | `success`, `attempt`, `finalError?` | |
| `summarization_retry_scheduled` | `attempt`, `maxAttempts`, `delayMs`, `errorMessage` | |
| `summarization_retry_attempt_start` | `source: "branchSummary"` or `source: "compaction", reason` | |
| `summarization_retry_finished` | — | |
| `bash_execution_update` | `id?`, `delta` | |

### 6.2 Other client events

| `type` | Payload | Origin |
|---|---|---|
| `extension_ui_request` | ui-protocol §4 | pi RPC (+ Oino methods) |
| `extension_error` | `extensionPath`, `event`, `error` | pi RPC |
| `session_replaced` | `sessionFile?`, `sessionId`, `reason: new\|resume\|fork\|clone\|import` | Oino |
| `model_changed` | `model`, `routedModel?`, `source: set\|cycle\|restore\|extension` | Oino (pi clients poll `get_state`; Oino frontends need push) |
| `run_state` | `state: idle\|running\|compacting\|retrying\|waiting`, `waitingOn?: string[]` | Oino; `waiting` = idle but an arbiter wait source reports a reason (§7.6) |
| `tools_changed` | `active: string[]` | Oino |
| `commands_changed` | — | Oino; clients refetch `get_commands` |
| `lagged` | `dropped: u64` | Oino (architecture §4.2) |

## 7. Extension runner (`oino-core::extensions`)

### 7.1 Loading

Sources in order (pi discovery mapped to WASM packages, D14/D15): bundled components
enabled by settings → `<agent>/extensions/<id>/` → trusted `.oino/extensions/<id>/` →
`settings.extensions[]` entries (paths, `builtin:<id>`, `!glob`, `+path`, `-path`, user and
project arrays combined) → `-e/--extension` (repeatable, `builtin:<id>` allowed). `-ne`
disables all `kind:"extension"` discovery except explicit `-e`. Duplicate ids: first wins,
diagnostic. Load order = discovery order (pi semantics; there is no priority field). Bundled
components load providers first, then unipi components in the umbrella order (unipi-port §1).

Each component: `ExtensionRuntime::load` → `init(InitInput)` → `Registration` (tools,
commands, shortcuts, flags, event subscriptions, providers, virtual models, renderers,
settings namespaces, services, arbiter roles). Registration is also dynamic at any time via
host calls (`register-tool`, `unregister-tool`, …). Provider/virtual-model registrations made
before the runner binds are queued and applied on bind (pi).

### 7.2 Dispatch semantics

Handlers run sequentially in load order; within one extension, in subscription order. A
per-handler deadline applies (default 5 s for hooks, `tool_call` 30 s, `session_before_*` 60 s,
`project_trust` unlimited when a UI dialog is open; manifest may raise up to 300 s). Timeout =
handler error. Error semantics: notification/transform handlers — error reported as
`extension_error` and ignored (chain continues with the previous value); `tool_call` handler
error or timeout **blocks** the tool (reason `Extension error: {message}`); `user_bash`
handler error blocks the command.

Composition per event (C = chained, the next handler sees the previous result; G = gate,
first decisive result stops; M = in-place mutation of a mutable payload field; N = notify;
B = boundary; L = last wins):

| Event | Payload (exact pi) | Result | Comp. |
|---|---|---|---|
| `project_trust` | `{cwd}` (ctx: `mode`, `hasUI`, ui select/confirm/input/notify) | `{trusted: yes\|no\|undecided, remember?}` | G (first yes/no; only bundled + user-level + CLI extensions are asked) |
| `resources_discover` | `{cwd, reason: startup\|reload}` | `{skillPaths?, promptPaths?, themePaths?}` | all results merged in order |
| `mcp_servers_change` | `{servers}` | — | N (never emitted by core v1; reserved) |
| `session_start` | `{reason: startup\|reload\|new\|resume\|fork, previousSessionFile?}` | — | N |
| `session_info_changed` | `{name?}` | — | N |
| `session_before_switch` | `{reason: new\|resume, targetSessionFile?}` | `{cancel?}` | G |
| `session_before_fork` | `{entryId, position: before\|at}` | `{cancel?, skipConversationRestore?}` | G (cancel) / OR (skip) |
| `session_before_compact` | `{preparation, branchEntries, customInstructions?, reason, willRetry}` (+ cancel token) | `{cancel?, compaction?: CompactionResult}` | G (first cancel or compaction) |
| `session_compact` | `{compactionEntry, fromExtension, reason, willRetry}` | — | N |
| `session_compact_failed` | `{reason, errorMessage?, aborted, willRetry, fromExtension}` | — | N |
| `session_shutdown` | `{reason: quit\|reload\|new\|resume\|fork, targetSessionFile?}` | — | N (awaited) |
| `session_before_tree` | `{preparation: TreePreparation}` (+ cancel token) | `{cancel?, summary?{summary,details?,usage?}, customInstructions?, replaceInstructions?, label?}` | G (cancel/summary) ; later fields L |
| `session_tree` | `{newLeafId, oldLeafId, summaryEntry?, fromExtension?}` | — | N |
| `context` | `{messages}` (system messages removed; restored after) | `{messages?}` | C |
| `context_with_system` | `{messages}` (full, system first) | `{messages?}` | C |
| `cache_warming_decision` | `{warmCost, missCost, continuationProbability, action}` | `{action?: warm\|stop}` | L |
| `before_provider_headers` | `{headers}` | ignored; handlers return mutated `headers` (`null` deletes) | M (expressed as C over `headers`) |
| `before_provider_request` | `{payload}` | replacement payload (any JSON) | C |
| `after_provider_response` | `{status, headers}` | — | N |
| `provider_stream_event` | `{provider, api, model, data}` | — | N (in stream order) |
| `before_agent_start` | `{systemPrompt, systemPromptOptions, messageType:"user", prompt, images?}` or `{…, messageType:"custom", message}` | `{message?: {customType, content, display, details?}, systemPrompt?, systemPromptOptions?}` | C over `systemPromptOptions` (M in pi; Oino returns the mutated object); `message`s accumulate; `systemPrompt` L |
| `agent_start` | `{}` | — | N |
| `agent_end` | `{messages}` | — | N |
| `agent_before_settle` | `BoundaryState{entries, continue, context: BoundaryContextPreview, outcome}` | `{entries?, continue?}` | B (C over entries/continue) |
| `agent_settled` | `{aborted}` | — | N |
| `ui_prompt_start` / `ui_prompt_end` | `{reason:"ui_prompt", kind: select\|confirm\|input\|editor\|custom, title?}` | — | N (Oino emits for every dialog method; Oino-only kinds map to `custom`) |
| `turn_start` | `{turnIndex, timestamp}` | — | N |
| `turn_end` | `BoundaryState + {turnIndex, message, toolResults, messageEntryId, toolResultEntryIds}` | `{entries?, continue?}` | B |
| `message_start` / `message_update` / `message_end` | `{message}` / `{message, assistantMessageEvent}` / `{message}` | `message_end`: `{message?}` (same role) | N / N / C |
| `tool_execution_start/_update/_end` | as §6.1 | — | N |
| `model_select` | `{model, previousModel?, source: set\|cycle\|restore}` | — | N |
| `thinking_level_select` | `{level, previousLevel}` | — | N |
| `tool_call` | `{toolCallId, parentToolCallId?, toolName, input}` | `{block?, reason?, terminate?, input?}` | G (block) ; `input` C (Oino: returned `input` replaces pi's in-place mutation; no re-validation) |
| `tool_result` | `{toolCallId, parentToolCallId?, toolName, input, content, structuredContent?, isError, usage?, details}` | `{content?, details?, structuredContent?, isError?, usage?}` | C (content without structuredContent drops it) |
| `user_bash` | `{command, excludeFromContext, cwd}` | `{result: BashResult}` or `{operations: "host"}` | G (Oino: extensions cannot supply JS `BashOperations`; `operations` is unsupported and treated as `undefined`) |
| `input` | `{text, images?, source, streamingBehavior?}` | `{action: continue}` / `{action: transform, text, images?}` / `{action: handled}` | C / G |

Payload typing: `oino_protocol::ext::events::*` defines one struct per event (with
`#[serde(tag="type")]` matching pi `type` strings) and one result struct per event.
`BoundaryState` drafts are `SessionBoundaryDraft = custom | custom_message | context_edit |
compaction` (pi shapes). `TreePreparation{targetId, oldLeafId, commonAncestorId,
entriesToSummarize, userWantsSummary, customInstructions?, replaceInstructions?, label?}`;
`CompactionPreparation` per pi `compaction.d.ts`.

Oino-only host events delivered through the same dispatch (prefixed `oino_`, never sent to
clients): `oino_timer{timerId}`, `oino_bus{topic, data, sticky}`, `oino_ui_action{…}`
(ui-protocol §5), `oino_editor{…}` (ui-protocol §7), `oino_settings_changed{namespace,
keys}`, `oino_process{…}` (extension-api §5.6), `oino_agent{…}` (nested agent events, §12).

### 7.3 Contexts

Every guest call carries a `CallContext{session_id, mode, has_ui, cwd, is_child, child_role?,
call_kind: event|tool|command|shortcut|service}`. Capabilities pi gates by context are
enforced by the host: session-replacing operations (`new_session`, `fork`, `switch_session`,
`navigate_tree`, `reload`, `wait_for_idle`) are allowed **only** while serving a command or
shortcut call (`SessionOpError::CommandContextRequired`), as in pi. After a session
replacement the old context is invalid; the host rejects calls carrying a stale session id
with `HostError::StaleContext` (pi `assertActive`). `ctx.executeTool` (pi 1.4) runs a
registered tool through `run_tool_call` with nested id `<parent>/<n>`, hooks applied, usage
rolled up, and `nestedCalls` recorded on the parent result (limits: per-call args > 8 KiB or
per-result > 32 KiB omitted, ≤256 calls, `complete:false` when anything dropped).

### 7.4 Registries

- **Tools** (`ToolInfo{name, description, parameters, promptSnippet?, promptGuidelines?,
  reachesTools, exposure, namespace?, annotations?, sourceInfo}` pi shape plus Oino `kind`
  (ACP tool kind, extension-api §4.2)). Exposure semantics (pi): `direct` and `model-only`
  activate on register unless `defaultActive:false`; `hidden` withdraws; `deferred` declared
  only via the tool-search mechanism (core keeps pi's plumbing; no `tool_search` tool ships in
  v1). Built-ins and extension tools share one namespace; later registration of an existing name
  by another extension is rejected with a diagnostic unless the manifest has permission
  `tools.override` (needed by unipi utility re-renders — it may re-register `read`, `bash`,
  `edit`, `write` *render metadata only*: schema and execution stay host-owned).
- **Commands**: invocation name = `name`; collisions get suffixes `:2`, `:3` in load order
  (pi `ResolvedCommand.invocationName`); argument completions via guest export; command echo
  (unipi §1.3.11) is a host feature: every extension command run appends a `custom` entry
  `oino.command-echo{name, args}` and records `notify`/panel output of the run in
  `oino.command-output{name, text(≤4000), panel?}`. Cross-extension `command-invoke(name,
  args)` runs a command with a command context (replaces unipi command runners); invoked from a
non-command context (event, tool, service), the command is queued to run once the session is
idle and the call returns `None` immediately after queuing (prevents run/replace deadlocks);
from a command or shortcut context it runs inline and returns the command's result; hidden
  commands (`hidden: true`) are invokable but not listed in `get_commands`.
- **Shortcuts**: `{id (stable action id), defaultKeys: [KeyId], description}`; frontends map keys
  (ui-protocol §8). Conflicts with built-in frontend bindings: frontend wins, diagnostic.
- **Flags**: `{name, type: boolean|string, default?, description}`; parsed by the CLI from
  unknown long flags, shown in `--help`, read via `get-flag`.
- **Providers / virtual models**: §13.
- **Renderers**: message/entry/tool render specs (ui-protocol §6.4).
- **Settings namespaces**: §10.3.
- **Services**: named request/response endpoints (extension-api §5.10).

### 7.5 Bus

`bus-publish(topic, data, sticky)`, `bus-subscribe(topic-pattern)` (exact or `prefix:*`),
`bus-get(topic)`. Sticky values are kept per session and replayed to new subscribers;
cleared on `session_shutdown`. Delivery to subscribers is via `oino_bus` events, async, in
publish order per topic, errors isolated. pi `pi.events` maps to the same bus (topics without
`unipi:` prefix). Frontends may read sticky topics through `_oino/bus_get` (modes §4) — used
for footer/status integrations.

### 7.6 Turn arbiter (D9)

At `agent_before_settle`, after all subscribed handlers ran: if `outcome != completed`, or
pending messages exist, or `continue` is already true ⇒ stop. Else ask every extension with
role `wait-source` for `wait-reason()` (2 s each, concurrently); any reason ⇒ stop and set run
state `waiting{waitingOn: reasons}`. Else ask every `nudge-provider` for
`propose-nudge()` (2 s each, concurrently) → `{priority, customType, content, display,
details?}`; choose highest priority (tie: earliest loaded); call `nudge-delivered(id)` on the
winner; append its `custom_message` draft to the boundary entries; `continue = true`. At most
one nudge per settle. In nested (child) sessions the arbiter is disabled.

### 7.7 Host services implementation

`oino-core` implements `HostServices` (extension-api §6) on top of `AgentSession`,
`Services`, the bus, settings, secrets, the UI bridge and nested agents. The wasm host calls
it; core never depends on wasmtime.

## 8. Built-in tools (`oino-tools`)

Copy every name, schema, description, `promptSnippet`, `promptGuidelines`, error string,
footer string and details shape from `PI/dist/core/tools/{read,write,edit,bash,grep,find,ls,
powershell,truncate,path-utils,file-mutation-queue}.js` (audit-pi §10 quotes the key ones).
Constants: `DEFAULT_MAX_LINES = 2000`, `DEFAULT_MAX_BYTES = 51200`, `GREP_MAX_LINE_LENGTH =
500`, bash update throttle = pi `BASH_UPDATE_THROTTLE_MS`.

- Active by default: `read`, `bash`, `edit`, `write` (setting `defaultTools`). Built-in but
  inactive: `grep`, `find`, `ls`; `powershell` on Windows only (D19).
- All file tools go through `ExecutionEnv` (`read_file`, `write_file`, `stat`, `mkdir_p`,
  `spawn`) so ACP clients can host fs/terminal (modes §6.4). `LocalEnv` is the default.
- `read`: images (magic sniffing, resize per `images.autoResize`/model `inputLimits` else
  2000×2000, `images.blockImages`), all truncation footers, path fallbacks
  (`resolveReadPath`: macOS screenshot narrow-NBSP, NFD, curly quote variants), no Oino
  `paths` multi-read.
- `write`: mkdir -p, file-mutation queue, result `Successfully wrote to {path}`.
- `edit`: `prepareArguments` (JSON-string `edits`, legacy `oldText/newText`), BOM + line
  ending preservation, exact then fuzzy matching (NFKC, trailing whitespace, quotes, dashes,
  spaces) with `applyReplacementsPreservingUnchangedLines`, all error strings, details
  `{diff, patch, firstChangedLine?}`.
- `bash`: `shellPath`/`shellCommandPrefix`, spawn detached in its own process group, stdin
  null, stdout+stderr interleaved, streaming `onUpdate` throttled, rolling tail + temp file
  `<agent>/tmp/oino-bash-<random>.log` when truncation is needed (pi name prefix `pi-bash-`
  substituted, D13), env D12 (inherited `OINO_*`/`PI_*` copies removed first), missing cwd
  error, timeout validation, process-tree kill on abort/timeout (Unix: `killpg`; Windows:
  `taskkill /T /F`), exit-code rules, `structuredContent{output ≤1 MiB, truncated,
  full_output_path?, exit_code, wall_time_seconds}`.
  **Detach (Oino addition, C11):** a running bash call can be detached into a background task
  via `HostServices::bash_detach(tool_call_id, reason)`: the process keeps running under the
  background-task service, the tool returns immediately with pi's normal output so far plus
  `\n\n[Detached to background task {taskId}: {reason}]`, `isError: false`.
- `grep` (ripgrep library crates, not a binary), `find` (globset + ignore), `ls`: pi schemas,
  limits and strings.
- Tool events for built-ins carry pi's typed inputs/details.

## 9. Resources and system prompt (`oino-resources`)

### 9.1 Directories

Agent dir and project dir per architecture §5. Agent dir override `OINO_AGENT_DIR`.

### 9.2 Context files

pi `loadProjectContextFiles`: per directory, first of `AGENTS.override.md`, `AGENTS.md`,
`AGENTS.MD`, `CLAUDE.md`, `CLAUDE.MD`; agent-dir file first, then ancestors from filesystem
root down to cwd, each directory once, nested git worktree shadows main repo copy; BOM
stripped; not trust-gated; disabled by `-nc`. Rendered as `project_context`
(`Project-specific instructions and guidelines:` + `<project_instructions path="…">` blocks).

### 9.3 SYSTEM / APPEND

`--system-prompt <text|path>` > trusted `.oino/SYSTEM.md` > `<agent>/SYSTEM.md` > built-in
preamble. Append: all `--append-system-prompt` values (text or path, in order) + (trusted
`.oino/APPEND_SYSTEM.md` else `<agent>/APPEND_SYSTEM.md`) joined by `\n\n` → `addendum`.
Never auto-created.

### 9.4 System prompt build

Port pi `system-prompt.js` (`normalizeBuildSystemPromptOptions`, `buildRules`,
`buildSystemPromptSections`, `buildSystemPromptState`, `diffSystemPromptSections`) exactly,
with these substitutions (D13):

- preamble: `You are an expert coding assistant operating inside Oino, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.`
- docs section:

```text
Oino documentation (read only when the user asks about Oino itself, its SDK, extensions, themes, skills, or TUI):
- Main documentation: {docs_root}/README.md
- Additional docs: {docs_root}/docs
- Examples: {docs_root}/examples (extensions, custom tools, SDK)
- When reading Oino docs or examples, resolve docs/... under Additional docs and examples/... under Examples, not the current working directory
- When asked about: extensions (docs/extensions.md, examples/extensions/), extension API and WIT (docs/extension-api.md), themes (docs/themes.md), skills (docs/skills.md), prompt templates (docs/prompt-templates.md), TUI (docs/tui.md), keybindings (docs/keybindings.md), SDK integrations (docs/sdk.md), providers and models (docs/providers.md, docs/models.md), Oino packages (docs/packages.md), environment variables (docs/environment-variables.md), RPC and ACP (docs/rpc.md, docs/acp.md)
- When working on Oino topics, read the docs and examples, and follow .md cross-references before implementing
- Always read Oino .md files completely and follow links to related docs (e.g., tui.md for TUI details)
```

  `{docs_root}` = extracted bundled docs `~/.oino/bundled/oino.docs@<version>-<hash8>`
  (integration writes those docs in P4 with exactly these file names).
- tool guideline `You can inspect PI_* environment variables for current model and session details.` → `You can inspect OINO_* environment variables (also exported as PI_*) for current model and session details.`

Per run: options (`customPrompt`, `selectedTools`, `hiddenTools`, `toolSnippets`,
`toolGuidelines`, `promptGuidelines`, `appendSystemPrompt`, `sections`, `cwd`,
`contextFiles`, `skills`) → `before_agent_start` → diff against replayed transcript → system
patch message prepended to the run's start messages only when changed; the first request of a
session persists the full system message. A handler `systemPrompt` (forced) is not recorded; it
is projected onto the request after `context` handlers. `prepareLoadout` hidden declarations
are filtered from every request. Prefix-cache rule: core never puts clocks/counters into the
system prompt.

### 9.5 Environment exported to tools (D12)

`OINO_SESSION_ID`, `OINO_SESSION_FILE`, `OINO_PROVIDER`, `OINO_MODEL` (selected model id;
virtual selection shown), `OINO_REASONING_LEVEL`, plus the same values as `PI_SESSION_ID`,
`PI_SESSION_FILE`, `PI_PROVIDER`, `PI_MODEL`, `PI_REASONING_LEVEL`. Process env read by core:
`OINO_AGENT_DIR`, `OINO_SESSION_DIR`, `OINO_OFFLINE`, `OINO_CACHE_RETENTION` (`long`),
`HTTP_PROXY`/`HTTPS_PROXY`/`NO_PROXY`, `VISUAL`/`EDITOR`.

### 9.6 Skills and templates

Exactly pi (audit-pi §12.5, §12.6): locations (`<agent>/skills`, trusted `.oino/skills`,
`~/.agents/skills`, `.agents/skills` from cwd up to repo root, `settings.skills[]`,
`--skill`, packages/extension manifests, `resources_discover.skillPaths`), frontmatter
validation (pi rules, **no** dir-name match requirement), first-wins collisions, the
`formatSkillsForPrompt` text, `/skill:name args` block `<skill name="{name}"
location="{filePath}">\nReferences are relative to {baseDir}.\n\n{body}\n</skill>` +
`\n\n{args}`, `enableSkillCommands`. Templates: `<agent>/prompts`, trusted `.oino/prompts`,
settings, `--prompt-template`, `resources_discover.promptPaths`; `parseCommandArgs` quoting,
`$1…`, `$@`, `$ARGUMENTS`, `${1:-d}`, `${@:-d}`, `${@:N}`, `${@:N:L}`; extension command with
the same name wins.

### 9.7 Project trust

pi semantics: protected resources (`.oino/settings.json`, `.oino/{extensions,skills,prompts,
themes}`, `.oino/SYSTEM.md|APPEND_SYSTEM.md`, project `.agents/skills`); order: `-a`/`-na` →
`project_trust` event → nearest saved decision in `<agent>/trust.json` (canonical paths) →
setting `defaultProjectTrust` (`ask|always|never`, agent-dir only). Non-interactive modes treat
`ask` as untrusted. `sessionDir` is read before trust. Context files never gated.

### 9.8 Keybindings file

`<agent>/keybindings.json` loaded/validated/migrated by core (pi `KeybindingsManager` format);
exposed to frontends via `get_keybindings` (modes §4). Frontends own semantics.

## 10. Settings (`oino-settings`)

### 10.1 Manager

Global `<agent>/settings.json` + project `.oino/settings.json` (trusted); project overrides
global; nested objects deep-merged; resource arrays (`packages`, `extensions`, `skills`,
`prompts`, `themes`) combined (user entries relative to agent dir, project entries relative to
`.oino`); writes go to one scope; file lock (`fs4` exclusive lock on `<file>.lock`); atomic
write; unknown keys preserved on write; `reload()`; `in_memory()`; diagnostics
`SettingsError{path, message}`.

### 10.2 Keys

pi keys verbatim (audit-pi §18 table, defaults included): `defaultProvider`,
`defaultModel`, `defaultThinkingLevel` (`medium`), `modelThinkingLevels`, `thinkingBudgets`,
`enabledModels`, `steeringMode`, `followUpMode` (`one-at-a-time`), `defaultTools`
(`read,bash,edit,write`), `sessionDir`, `compaction.{enabled, reserveTokens, keepRecentTokens,
modelOverrides}`, `branchSummary.{reserveTokens, skipPrompt}`, `retry.{enabled, maxRetries,
baseDelayMs, maxAgentDelayMs, provider.{timeoutMs, maxRetries, maxRetryDelayMs}}`,
`transport`, `httpProxy`, `httpIdleTimeoutMs`, `websocketConnectTimeoutMs`, `cacheWarming`
(`streaming`), `showCacheMissNotices`, `shellPath`, `shellCommandPrefix`, `images.{autoResize,
blockImages}`, `packages`, `extensions`, `skills`, `prompts`, `themes`,
`enableSkillCommands`, `defaultProjectTrust`. Frontend keys (`theme`, `hideThinkingBlock`,
`externalEditor`, `doubleEscapeAction`, `treeFilterMode`, `quietStartup`, `markdown.*`, …)
are stored and returned untyped (`frontend` getters return `serde_json::Value`).

Oino keys:

| Key | Default | Meaning |
|---|---|---|
| `extensionSettings` | `{}` | per-namespace extension settings (D11) |
| `disabledExtensions` | `[]` | bundled extension ids not to load |
| `serve.{bind, tls, allowRemote}` | `127.0.0.1:7420`, `true`, `false` | `oino serve` (modes §5) |
| `ui.density` | `regular` | `simple\|regular\|advanced` transcript density for render specs |

### 10.3 Extension settings registry

`registry.register-settings(namespace)` (or `registration.settings` at `init`); schema =
ui-protocol §6.6 settings schema (+ optional `secretReaders` id patterns). Implemented by
`oino_core::extensions::settings::ExtensionSettings` (one per `Services`). Reads layer
`defaults ⊕ global ⊕ project` (`registry.extension-settings`); writes go to one scope
(`registry.set-extension-setting` from the owner, `ExtensionSettings::set` from frontends);
`use default` deletes the key; keys are dot paths stored as nested objects. Each write sends
`oino_settings_changed {namespace, keys, scope}` to the owning extension and to
`ExtensionSettings::subscribe()` listeners (the `oino:settings:<namespace>` bus topic is not
published, core-runtime #22). Secret fields are stored in the secret store at
`<owner>/<namespace>/<key>`, never in `settings.json`; frontends see a mask, the owner (and
`secretReaders`) the value. Frontends render the settings hub from `get_settings_schema`
(modes §4) = `ExtensionSettings::schema()`.

## 11. Compaction, branch summaries, recovery, retry (`oino-core::compaction`, `::retry`)

### 11.1 Compaction

Port pi `compaction/{compaction,branch-summarization,utils}.js` exactly: trigger
`contextTokens > contextWindow − reserveTokens` (per-model `compaction.modelOverrides
["provider/id"]`), checked (a) before prompt submission, (b) mid-run in `prepare_next_turn`
after tool results, (c) after `agent_end`; `estimateContextTokens` (last valid assistant usage
+ `ceil(chars/4)` trailing) over the projection; limits of the physical model of the latest
response; `findCutPoint`/`prepareCompaction` incl. split turns, valid cut types, repeated
compaction start, invisible-suffix advance; structured summary prompts and serialization
(`[User]:`, `[Assistant]:`, `[Assistant tool calls]:`, `[Tool result]:` truncated at 2000
chars), file tracking (`readFiles`, `modifiedFiles`, cumulative from previous non-hook
compaction), prompt-cache writes disabled, output budget = model max tokens clamped to fit;
`CompactionResult{summary, firstKeptEntryId, tokensBefore, estimatedTokensAfter?, usage?,
details?}`; entry stores `systemMessage` checkpoint and `usage`; events `compaction_start/_end`,
`session_before_compact` (cancel or supply), `session_compact`, `session_compact_failed`;
summarization uses the `retry` budget with `summarization_retry_*` events. The default
summarizer is exposed to extensions as `summarize-default(entries, instructions)` (C9).

### 11.2 Overflow / length recovery

pi order: persist final assistant → `turn_end` → `agent_end` → append `context_edit`
omissions for the failed attempt and its tool results → overflow/length:
`session_before_compact` + compaction → retry as a fresh run (`willRetry: true`, virtual route
reason `retry`, `failed` passed). Compaction failure/cancel: keep omissions, no retry.

### 11.3 Branch summaries (`navigate_tree`)

pi: `TreePreparation`, `session_before_tree`, budget `branchSummary.reserveTokens`
newest-first, `skipPrompt`, compaction-like format without Critical Context, cumulative file
lists, `branch_with_summary` + label, `editorText` when the target is a user message (leaf →
its parent), `session_tree` event. Result `NavigateTreeResult{editorText?, cancelled,
aborted?, summaryEntry?}`.

### 11.4 Auto-retry

pi `_isRetryableError` (not overflow; overloaded, rate limit, 5xx, transient stream drops —
classification table copied from pi `isRetryableAssistantError`), settings `retry.*`,
exponential `baseDelayMs·2^(attempt−1)` capped by `maxAgentDelayMs`, failed attempt omitted via
`context_edit`, `auto_retry_start`, cancellable sleep, `continue` with route reason `retry`,
`auto_retry_end{success, attempt, finalError?}`, cancel ⇒ `finalError: "Retry cancelled"`,
`agent_end.willRetry`.

## 12. Nested agents (`oino-core::agents`, D8)

`HostServices::agents_spawn(spec) -> AgentHandle` creates an `AgentSession` in the same
process:

```rust
pub struct AgentSpec {
    pub label: String,                                   // shown in run state / UI
    pub model: Option<ModelKey>, pub thinking: Option<ThinkingLevel>,
    pub system_prompt: Option<SystemPromptSpec>,         // Replace(String) | Append(String) | Default
    pub tools: ToolPolicy,                               // {builtin: Vec<String>, extension_tools: Vec<String>, deny: Vec<String>}
    pub extensions: ExtensionPolicy,                     // None (default) | Inherit | Only(Vec<ExtensionId>)
    pub seed: Seed,                                      // None | BranchCopy | SessionFile(PathBuf) | Messages(Vec<AgentMessage>)
    pub persistence: Persistence,                        // Memory | File{session_dir: Option<PathBuf>} (child sessions dir default: <parent session dir>/children/)
    pub cwd: Option<PathBuf>, pub child_role: Option<String>,
    pub ui: UiPolicy,                                    // Forward (to parent UI) | AutoDeny{note: String} | None
    pub max_depth: Option<u8>,
}
```

Handle operations: `prompt(text, delivery)`, `steer`, `follow_up`, `abort`,
`set_thinking`, `last_assistant_text`, `events()` (stream of child `ClientEvent`s; delivered to
the guest as `oino_agent` events or via a WIT `stream`), `usage()`, `session_file()`,
`close(grace_ms)`. Limits: depth ≤ `max_depth` (default 2, hard cap 4); concurrent children per
root session ≤ 8 (setting `extensionSettings.subagents.maxConcurrent`, hard cap 16); cancel
grace 5 s. Child usage is appended to the parent as `usage` entries (`kind:
"child:<label>"`, provider/model of the child) so `getSessionStats` includes it (C29). Child
sessions have the arbiter disabled and `is_child = true`, `child_role` visible to extensions.
`UiPolicy::Forward` routes child dialogs to the parent's UI bridge (title prefixed with
`[label] `); `AutoDeny` answers dialogs with cancel and returns `note` as the reason to
`approval` dialogs.

## 13. Models, auth, virtual models (`oino-models`)

- **Registry** merges: provider registrations from provider components (`list-models` per
  registered provider, cached in `~/.oino/cache/models/<provider>.json`, refreshed on
  `refreshModels` or `/login`), then `models.json` (pi format, audit-pi §17.1: providers with
  `api`, `baseUrl`, `apiKey` (`literal | $ENV | ${ENV} | !command`), `headers`, `compat`,
  `authHeader`, `models[]`, `modelOverrides{}`; defaults `reasoning=false`, `input=["text"]`,
  `contextWindow=128000`, `maxTokens=16384`). A `models.json` provider routes to the provider
  component that declares its `api` (extension-api §4.4); unknown `api` ⇒ diagnostic, models
  unavailable.
- **Resolution**: pi `--model` (exact id, fuzzy id/name, `provider/id`, `:thinking` suffix,
  multi-provider preference for credentialed provider else require `--provider`), `--models`
  scope (globs + suffix) and `enabledModels`, cycling, `defaultProvider/defaultModel`,
  `modelThinkingLevels`, restore from branch selection, `modelFallbackMessage`.
- **Auth** (`auth.json`, pi format): providerId → `{type:"api_key", key}` or
  `{type:"oauth", access, refresh, expires, …}`; exclusive file lock for read-modify-write;
  resolution order `--api-key` > auth.json > provider env var(s) from the provider
  registration > models.json `apiKey`; OAuth refresh via the provider component's
  `oauth-refresh` export; `oino auth check` exit codes 0/1/2 (pi).
- Core passes credentials per request (`get_api_key`) to the provider component as
  `StreamOptions` `apiKey` + resolved headers; components never read `auth.json`.
- **Virtual models** (pi `virtual-models.md`): registered by extensions (`register-virtual-model`
  with guest export `route`); `VIRTUAL_MODEL_API = "oino-virtual"`; route request `{model,
  thinkingLevel, reason: user|continuation|retry|direct, previous?, failed?, state?, messages,
  signal}` → `{model (physical key), thinkingLevel (clamped), state?}`; state persisted as
  `custom` entry `oino.virtual-model-state{provider, modelId, state}` before the request (unless
  unchanged/undefined), branch-scoped; selection recorded in `model_change`, dispatch in the
  assistant message; `routed_model()`; context limits from latest physical response.

## 14. Bash executor, cache warming, stats, export

- `execute_bash` (pi `bash-executor.ts`): `user_bash` first; `!cmd` included in context,
  `!!cmd` excluded; result recorded as `bashExecution` message entry; while streaming, held and
  flushed after the turn; `bash_execution_update{id?, delta}` streamed.
- Cache warming: port pi `CacheWarmer` (modes off|streaming|idle, threshold $0.05, `usage`
  entry `kind:"cache_warm"`, `cache_warming_decision` event, status for `/session`). Legacy
  `CacheTracker` logic may be reused re-typed.
- `session_stats` (pi shape, all entries incl. compacted history and `usage` entries) and
  `context_usage` (pi; `null` tokens right after compaction).
- Export: `export_html` renders the current branch from session entries (pi
  `session-export.ts` structure: header, messages, tool calls, collapsed thinking, embedded CSS,
  no external assets), default path `<cwd>/oino-session-<sessionId8>.html`; `export_jsonl`
  writes header + current branch.

## 15. Package manager (`oino-core::packages`)

Installs WASM packages (extension-api §7) from: local dir, `.tar.zst` file, `https://` URL to
an archive, `git+https://…[#ref]` (cloned with the system `git`), into
`<agent>/extensions/<id>/` (or `.oino/extensions/<id>/` with `--project`). Verifies manifest,
`apiVersion`, sha256 of `component.wasm` against `manifest.checksums` when present, and shows
requested permissions for confirmation (interactive) or requires `--yes` (non-interactive).
`update` re-fetches from the recorded source (`<id>/.source.json`). `remove`, `list`,
`config` (opens `settings.json` path). Bundled extensions cannot be removed, only disabled
(`disabledExtensions`).

## 16. Explicit Oino deltas from pi (exhaustive)

1. Product strings (D13) and env names (D12).
2. Extensions are WASM components; `user_bash.operations`, `registerMessageRenderer` & co.
   become data (ui-protocol), `tool_call.input` mutation is returned, `before_agent_start`
   options are returned.
3. Additional client events (§6.2) and `_oino/*` RPC commands (modes §4).
4. Host features: command echo, arbiter, bus, services, nested agents, bash detach,
   provenance, extension settings registry.
5. `-ne` keeps provider components (D15).
6. No MCP in core; `mcp_servers_change` never emitted by core.
7. No `/share`, `/bug`, telemetry, `enableInstallTelemetry`, `codemode`, `tool_search`.

## 17. Acceptance (core-runtime)

- Unit tests per crate; golden tests against pi for: tool strings (every quoted string),
  system prompt sections (pi oracle renders the same options), compaction cut points (ported
  `compaction.test.ts` cases), convert/context building, JSON event stream of scripted runs
  with the faux provider (quality §4).
- `oino-core` tests use an in-memory `FakeExtensionRuntime` (provided in
  `oino-extension-api::testing`) and the faux provider; no wasmtime in core tests.

---

## Appendix: settled deviations (folded at 1.0.0-alpha.2)

From `deviations/core-runtime.md` (core-runtime, core-agents, core-settings) and `deviations/integration.md` #5. pi 1.4.0 behaviour won where this spec abbreviated or contradicted pi without listing the difference in §16; pi-parity choices are golden-tested against the oracle (`scripts/pi-oracle/core-runtime/`).

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | runtime §1.2 | `StreamFn::stream(StreamRequest{model, context, options, api_key, cancel})`; `onPayload`/`onResponse`/`onProviderStreamEvent` are not loop config fields. | In Oino the provider hooks run between core and the provider component (provider-host), not in the loop. | wit-v1: route `provider-host` hooks to the session runner (`emit_chain` exists for `before_provider_request`/`_headers`). |
| 2 | runtime §1.1 | Blocked tool default text is pi's `Tool execution was blocked` (spec: `Tool call blocked`). | pi `prepareToolCall`. | none. |
| 3 | runtime §1.1/§7.2 | `BeforeToolCallResult.input` (Oino) replaces validated arguments; pi mutates `args` in place. | WASM handlers return values. | none. |
| 4 | runtime §1.1 | Validation messages follow TypeBox (golden-tested, 80 cases); the `format` keyword is not checked. | No TypeBox in Rust. | Tools with `format` validate less strictly. |
| 5 | runtime §2 | `AgentSession::prompt` returns `Started` once the run is prepared (`before_agent_start` done); the run continues in a task — await `wait_for_idle()` for completion. pi awaits the whole run (RPC uses `preflightResult`). | RPC/ACP need the disposition before completion. | modes: respond on return, stream events. |
| 6 | runtime §2 | A prompt or triggering `send_custom_message` made while `agent_settled` handlers run is deferred until they finish and reports `Queued`. | pi `_deferredSettledActions` returns `void`. | none. |
| 7 | runtime §7.2 | `tool_call` handler errors block with `Extension error: {message}` (spec); pi blocks with the raw message. | Spec text explicit. | none. |
| 8 | runtime §7.2 | A `message_end` replacement replaces the agent transcript's last message and the persisted entry; the loop's local copy (tool calls of that turn) is not changed. pi mutates the object in place. | No shared mutable objects across the WASM boundary. | Extensions rewriting tool calls at `message_end` do not change execution. |
| 9 | runtime §4/§9.7 | `Services::create` resolves trust from `-a/-na`, `trust.json` and `defaultProjectTrust` (`ask` ⇒ untrusted); the `project_trust` extension event is not dispatched yet. | Extensions load after trust (they include trust-gated `.oino/extensions`). | Frontends ask interactively before `Services::create` and pass `approve`. |
| 10 | runtime §4 | Extensions are loaded through `services::ExtensionLoader::load(host, project_trusted)`; core does no manifest/package discovery. | Discovery and manifests belong to the WASM host and package manager. | wit-v1/integration implement the loader. |
| 11 | runtime §7 | `oino-protocol` was not created. Client events, UI types and host operations live in `oino_core::{events, ui, extensions::{api, host}}` with the specced JSON shapes; host calls are one `HostApi::call(ext, cx, HostOp)` enum instead of one trait method per WIT function. The WASM contract crate `oino-extension-api` exists; `oino_wasm_host::core_bridge` adapts it to core. | Contract crates landed in a different order. | Frontends import wire types from `oino-core` / `oino-client` (modes appendix #4, tui-client #1). |
| 12 | runtime §8 (tools) | Tool-result image normalisation (`normalizeToolResultImages`) and prompt image processing (`processImage`) are not done by core; `oino-tools` `read` resizes its own images. | Deferred. | Extension tools returning large images are passed through. |
| 13 | runtime §14 | `export_jsonl` resolves a relative path against the session cwd (pi: process cwd). | Libraries do not read the process cwd. | modes pass absolute paths. |
| 14 | runtime §6.1 tests | Golden session-event comparison ignores `message_update.message` (pi's shallow copy keeps mutating after the event) and the content of the first assistant partial. | pi artefacts, not behaviour; the wire form drops them (modes §3.1). | none. |
| 15 | runtime §12 | `Persistence::File` writes the child file directly in `sessionDir` (pi `--session-dir` semantics, no `--<cwd>--` subdir) at spawn, header `parentSession` = parent file; default dir `<parent session dir>/children/`; a file child of an in-memory parent without `sessionDir` stays in memory. `Seed::SessionFile` + file persistence continues that file in place (resume). | Guests (fusion, dream) locate their child files by directory. | `/resume` lists never show child files. |
| 16 | runtime §12 | `AgentSpec` defaults: `ui: none`, `extensions: none`, `tools.builtin` = `defaultTools`, `childRole` = spawning extension id; `tools.extensionTools` patterns match only tools of the child's extensions; `deny` applies to both. `max_depth`: the root allows 4 levels; a child's allowance is `min(maxDepth ?? 2, parent allowance)`. Manifest `agents.maxDepth` is not enforced. | Spec gave no defaults. | — |
| 17 | runtime §12 | `UiPolicy::Forward` forwards dialogs (title prefixed `[label] `) and `notify` only; other fire-and-forget requests (status, widgets, panels) of a child are dropped. `AutoDeny` answers `approval` with its first `reject_once`/`reject_*` option and `feedback: note`, cancels other dialogs, and gives the child `hasUI: true`; `None` gives `hasUI: false`. | Inherited footer/tray extensions would overwrite the parent's chrome. | — |
| 18 | runtime §12 | Handle: `prompt` while the child runs queues (`steer` or follow-up); `steer`/`follow-up` on an idle child start a run; `close(grace ≤ 5 s)` aborts after the grace, emits `session_shutdown{quit}` to the child's extensions and ends `events()`; disposing a session closes its children; dropping the last guest handle closes with grace 0. `events()` are `ClientEvent` JSON without the RPC `message_update` transform. `oino_agent` events go only to the spawning extension, when it subscribed. | — | — |
| 19 | runtime §7.6 | The arbiter also runs when no `agent_before_settle` handler exists. Proposal JSON `{id?, priority, customType, content?, display?(true), details?}`; `nudge-delivered` is synchronous. While `waiting`, wait sources are re-polled every 2 s; `run_state{idle}` is published when they go quiet, `run_state{running}` when a run starts. | Frontends otherwise stay in `waiting`. | — |
| 20 | runtime §7.2 | Manifest `limits.hookTimeoutMs` / `toolCallHookTimeoutMs` raise core's handler deadline (never below core's default, cap 300 s). | long-horizon's verifier runs in `agent_before_settle` (90 s). | — |
| 21 | extension-api §3 `control` | `shutdown` sets `AgentSessionRuntime::shutdown_requested()` (a watch flag); modes decide when to exit. `new-session`/`fork`/`switch-session`/`reload`/`system-prompt-options` need a command context of a root session (children: `Failed`). `bash-detach`/`running-bash` stay unsupported. | pi `shutdownHandler` is mode-owned. | modes: exit on the flag once idle. |
| 22 | runtime §10.3 | Extension settings registry (`extensions::settings::ExtensionSettings`, one per `Services`, also `AgentSessionRuntime::extension_settings()`): keys are dot paths written as nested objects (legacy flat dotted keys are expanded on read and removed on write); `None`/JSON `null` deletes ("use default"); light type checks for `boolean`/`number`/`string`/`enum`/`multiselect`/`order`/`model`/`secret` fields and field `scope` enforcement; field `default`s fill missing `defaults` keys. Secret fields live in `secrets.json` at `<owner-ext-id>/<namespace>/<key>` (the key a guest using `secrets.set("<namespace>/<key>")` already had); frontends read `SECRET_MASK` (writing it back is a no-op, `""` deletes); guests read the value when they own the namespace or match its schema `secretReaders` patterns (Oino addition), else the key is absent. Extensions may write only namespaces they registered; frontends may also write unregistered namespaces. `oino_settings_changed {type, namespace, keys, scope}` is delivered to the owner (context: any live root session) for every write; the `oino:settings:<namespace>` bus topic is **not** published (the bus lives in the wasm host). `InitInput.settings` carries `{namespace: values}` of every namespace known so far (an extension's own secrets resolve on its first `extension-settings` read). Settings operations (`register-settings`, `extension-settings`, `set-extension-setting`, `get-settings`) are served by the `HostRouter` without a session. | Spec left these open; the bus is not in core. | Hub UI/RPC commands: modes, tui-client, gpui-desktop. |
| I5 | runtime §4 | `Services::create` refreshes `models: "dynamic"` providers **concurrently** with a 5 s timeout per provider, skips providers that need a key and have none, and skips the refresh entirely with `--offline` / `OINO_OFFLINE` (`ResourceFlags::offline`). | Startup with the bundle took 3–4 s of serial network round trips. | Startup ≈ 0.3–1 s with warm compile cache. `--offline` is now honoured (modes deviation #17 partly). |

**Subcrate notes (binding):**

- **oino-settings / oino-resources:** CLI `--append-system-prompt` values replace the discovered
  `APPEND_SYSTEM.md` (pi), not combined (§9.3). `mcp.json` is not a protected resource (§16.6).
  Invalid enum values fall back to defaults. Locks use std `File::lock` on `<file>.lock`.
- **oino-models:** credential order is pi's: `--api-key` > `auth.json` > `models.json`/override
  `apiKey` > registration `authEnv` (§13 lists env before `apiKey`). A model is available only
  when a loaded component implements its API. Router state equal to the stored state is not
  re-stored. OAuth refresh has no built-in timeout (host enforces it).
- **oino-tools:** bash temp files are `<agent>/tmp/oino-bash-<16hex>.log` (§8); `blockImages`
  is applied in core's `convertToLlm` (pi), not in `read`; no JS `spawnHook`; grep/find walk the
  local fs with the ripgrep/ignore crates (results in sorted path order, rg per-file error texts
  approximated, non-local `ExecutionEnv` unsupported); `ls` approximates ICU `localeCompare`
  outside ASCII; re-encoded image bytes differ from Photon (dimensions/MIME/notes match);
  detached bash results carry no `structuredContent`; Windows paths compile but are untested.
- **compaction:** summary requests set no `maxTokens` (pi 1.4.0, issue #9512); prompt-cache
  writes disabled via `cacheRetention: none`; pi's provider `env` option is dropped.
