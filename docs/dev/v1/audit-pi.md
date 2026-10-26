# pi-coding-agent 1.4.0 runtime parity audit

Status: reference document for v1.alpha (`[audit-pi]`). Audited against `v1-alpha` @ `17dcdb7`.
Scope: the pi-coding-agent **runtime** (agent loop, AgentSession, SessionManager, extension
runner/API, built-in tools, resource loading, compaction, modes, model/auth/settings plumbing).
Out of scope per [CHARTER](CHARTER.md): pi-tui widgets, pi-ai provider implementations, pi-mcp,
pi-codemode, pi-telemetry (they are mentioned only where they shape a runtime contract).

This is the implementation reference. Every row gives the pi behaviour, the Oino status, the gap
and the proposed owning crate. "Exact" means: same JSON shape, same strings, same ordering.

## 0. How to read this document

### 0.1 Reference abbreviations

| Abbrev | Path |
|---|---|
| `PI/` | `~/.local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent/` |
| `PI/docs/x.md` | pi user docs (authoritative prose) |
| `PI/core/x` | `PI/dist/core/x.d.ts` (declarations) or `.js` (behaviour) |
| `PI/modes/x` | `PI/dist/modes/x` |
| `AC/x` | `PI/node_modules/@earendil-works/pi-agent-core/dist/x.d.ts` (`types`, `agent`, `agent-loop`) |
| `AI/types` | `PI/node_modules/@earendil-works/pi-ai/dist/types.d.ts` |
| `crates/…` | Oino crate paths in this repo |

### 0.2 Status legend

| Mark | Meaning |
|---|---|
| ✅ | Parity (shape and behaviour match) |
| 🟡 | Partial: concept exists, details or shape differ |
| ⚠️ | Divergent: exists but incompatible design; must be replaced, not extended |
| ❌ | Missing |
| ➖ | Not applicable to core (TUI-only, provider-only, or explicitly out of scope) |

### 0.3 Proposed future crate ownership

The current crates predate the charter. The "Owner" column uses this proposed layout (names are a
proposal for the architect; the split is what matters):

| Proposed crate | pi equivalent | Absorbs (current) |
|---|---|---|
| `oino-ai` | `pi-ai` types only: `Message`, content blocks, `Usage`, `StopReason`, `Model`, `AssistantMessageEvent`, `TranscriptContext`, transcript normalization/replay | `oino-types` |
| `oino-agent` | `pi-agent-core`: `agentLoop`, `runToolCall`, `Agent` class, queues | `oino-agent-loop`, `oino-agent` |
| `oino-session` | `session-manager.ts`, `messages.ts` (custom AgentMessage roles, `convertToLlm`) | `oino-session` |
| `oino-core` | `agent-session.ts`, `agent-session-runtime.ts`, `agent-session-services.ts`, `compaction/*`, `bash-executor.ts`, `system-prompt.ts`, `cache-warmer.ts`, `event-bus.ts`, `exec.ts`, `sdk.ts` | `oino-harness` (+ runtime parts of `oino-app/main.rs`, `llm_compact.rs`) |
| `oino-tools` | `core/tools/*` (read, write, edit, bash, grep, find, ls, truncate, file-mutation-queue, path-utils) | `oino-tools`, `oino-env` |
| `oino-resources` | `resource-loader.ts`, `skills.ts`, `prompt-templates.ts`, `slash-commands.ts`, `project-trust.ts`/`trust-manager.ts`, keybindings file loading | `oino-resource` |
| `oino-settings` | `settings-manager.ts`, `settings-schema.ts`, `settings-defaults.ts` | `oino-app/user_settings.rs` + ad-hoc settings code in `main.rs` |
| `oino-models` | `model-registry.ts`, `model-resolver.ts`, `model-runtime.ts`, `models-store.ts`, `model-config.ts`, `virtual-models.ts`, `auth-storage.ts`, `resolve-config-value.ts` | `oino-auth`, `oino-provider-catalog`, `oino-app/model_catalog.rs`, `provider_config.rs` |
| `oino-extension-host` | `core/extensions/{runner,loader,types,wrapper}.ts` mapped onto WIT + Wasmtime | `oino-extension-core`, `-runtime`, `-manager`, `-builtins`, `oino-wasm-host`, `oino-app/wasm_extensions.rs` |
| `oino-extension-sdk` | the `ExtensionAPI` surface as a Rust guest SDK | `oino-extension-sdk`, `extensions/crates/oino-extension-guest*` |
| `oino-cli` (bin `oino`) | `cli/args.ts`, `main.ts`, `modes/print-mode.ts`, `modes/json-event.ts`, package/auth subcommands | `oino-app/main.rs` (headless part) |
| `oino-rpc` | `modes/rpc/*` (rpc-mode, rpc-types, jsonl, rpc-client) + extension UI sub-protocol | — (new) |
| `oino-acp` | — (Oino addition, on top of `oino-rpc`) | — (new) |
| `oino-tui` | interactive mode | `oino-tui`, TUI part of `oino-app` |

## 1. Executive summary — biggest gaps

1. **Wire/data model is not pi's.** Oino `Message` (`crates/oino-types/src/lib.rs:174`) is
   `kind`-tagged snake_case with UUID ids on every message and no timestamps; pi `AgentMessage`
   is `role`-tagged camelCase, id-less, with ms timestamps, `api/provider/model` on assistant
   messages, pi `Usage` shape, pi `StopReason` names, `ToolCall.id: string`. Nothing that crosses
   a boundary (session file, JSON mode, RPC, WIT) can be pi-compatible until `oino-ai` adopts
   pi's types verbatim. **This is the root dependency of everything else.**
2. **Session file is not v3.** Oino writes `{"record":"header",…}` / `{"record":"entry",…}`
   with UUID ids, no timestamps, `version: 2`, persisted `LeafMove` entries, whole-file rewrite,
   `<root>/<uuid>.jsonl` path (`crates/oino-session/src/lib.rs:82-200,578-663`). pi v3 needs
   flat `type`-tagged entries, 8-hex ids, `parentId`, ISO timestamps, header line
   `{"type":"session","version":3,…}`, append-only persistence that starts only once a user or
   assistant message exists, `--<cwd>--/<ts>_<id>.jsonl` path, `buildSessionProjection`,
   migration v1→v2→v3. No `bashExecution`, `custom_message`(pi shape), `label` target,
   `session_info` name-only, `usage` provider/model/note, `compaction.systemMessage`.
3. **No AgentSession.** pi's `AgentSession` (prompt pipeline with command/skill/template
   expansion, steering/follow-up/nextTurn queues as strings, auto-compaction (threshold +
   overflow + length recovery), auto-retry with backoff, `executeBash`/`!`/`!!`,
   `navigateTree` with branch summaries, fork/clone/new/switch/import via
   `AgentSessionRuntime`, session stats, export) has no counterpart. `oino-harness::Harness`
   (`crates/oino-harness/src/lib.rs:475`) covers prompt/steer/abort/skill/template/compact
   (summary passed in)/navigate (no summary) and much logic lives in the 11.9k-line
   `crates/oino-app/src/main.rs`.
4. **No auto-retry, no overflow recovery, no compaction algorithm in core.** Oino has no
   `retry.*` support at all; compaction is a percent threshold in the app
   (`main.rs:7367 try_auto_compact`) and an LLM summarizer in `oino-app/src/llm_compact.rs`
   without pi's cut-point algorithm, split-turn handling, file tracking, `tokensBefore`
   estimate, `session_before_compact`/`session_compact` events or `compaction_start/_end`.
5. **Extension API surface is ~15% covered.** WIT (`wit/extension.wit`) exposes `handle-event`
   + `run-tool` + `run-command` and host imports `log/secret/kv/session-append/session-entries/
   complete/notify/confirm/tool-progress/exec`. Missing: dynamic `registerTool/Command/
   Shortcut/Flag/Provider/VirtualModel`, `sendMessage`, `sendUserMessage`, `appendEntry` (pi
   shape), `setSessionName/getSessionName/setLabel`, `get/setActiveTools`, `getAllTools`,
   `getCommands`, `getSettings`, `setModel`, `get/setThinkingLevel`, `events` bus,
   `ctx.executeTool`, every `ExtensionContext`/`ExtensionCommandContext` method, and all UI
   methods except notify/confirm. Of pi's 41 events only ~13 are wired, mostly observe-only and
   with Oino-specific payloads (`crates/oino-app/src/wasm_extensions.rs:1165-1520`).
6. **Agent loop contract differs.** Oino loop has a hard `max_turns: 16` cap
   (`crates/oino-agent-loop/src/lib.rs:536,610`; pi has none), Oino-specific events
   (`AgentEvent` `kind`-tagged, no `message_update` with `AssistantMessageEvent`, `agent_end`
   without `messages`, `turn_end` without `message/toolResults`), no `convertToLlm`, no
   `prepareNextTurn`, no `getApiKey`, shallow schema validation (type+required only), no
   `durationMs`, no nested `runToolCall`, default queue modes `All` (pi: `one-at-a-time`).
7. **Built-in tools differ in exact behaviour.** `read` has no images (placeholder text) and an
   Oino-only `paths` multi-read; `bash` buffers instead of streaming, separates stdout/stderr,
   has no temp full-output file, no `PI_*` env, no process-tree kill, no `structuredContent`;
   `edit` has no CRLF/BOM preservation, no fuzzy (NFKC/quote/dash/space) matching, no
   diff/patch details, no file mutation queue; `write` says "Successfully wrote N bytes to"
   (pi: "Successfully wrote to {path}"). grep/find/ls/powershell built-ins are absent.
8. **Resource loading is not pi's.** Oino reads `~/.oino/SYSTEM.md` and `.oino/AGENT.md` and
   writes default files into the user's tree; pi discovers `AGENTS.override.md|AGENTS.md|
   AGENTS.MD|CLAUDE.md|CLAUDE.MD` in agent dir + cwd + all ancestors, treats `SYSTEM.md` as a
   *replacement* and `APPEND_SYSTEM.md` as an addendum (project wins over global), builds the
   structured section prompt (`preamble/tools/rules/docs/addendum/project_context/skills/cwd`),
   discovers skills from `~/.agents/skills` and ancestor `.agents/skills`, expands
   `/skill:name` into a `<skill name location>` block, and supports full bash-style template
   args (`${1:-x}`, `${@:N:L}`, quoting). Project trust (`trust.json`, `--approve`) is absent.
9. **No RPC mode, no pi JSON mode, no pi print mode.** `--json` emits Oino `AgentEvent` + a
   summary line; there is no `--mode json|rpc`, no session header line, no delta-only
   `message_update`, no RPC commands, no `extension_ui_request/response`. ACP depends on RPC.
10. **No virtual models, no pi models.json, no pi auth.json semantics.** Model identity is
    `provider:name` with a string map (`crates/oino-types/src/lib.rs:65`), not pi's `Model`
    object (`api`, `baseUrl`, `reasoning`, `input`, `cost`, `contextWindow`, `maxTokens`,
    `thinkingLevelMap`, `promptCache`, …). `--model` fuzzy/`provider/id:thinking` resolution,
    `--models` scope, cycling, `enabledModels`, `modelThinkingLevels`, and `$ENV`/`!command`
    key resolution are absent.

Things Oino already has that are close to pi and should be kept/re-shaped rather than rewritten:
transcript-carried system prompt with section patches and `toolsAdded/Removed`
(`oino-types/src/transcript.rs`), context edits (session), compaction `first_kept` semantics incl.
retain-none and system-message folding (`oino-session/src/lib.rs:453-570`), cache warming tracker
with a veto hook (`oino-harness/src/cache.rs`), parallel/sequential tool execution with
`terminate` batch rule, `finish_turn`/`prepare_request`/`transform_context` hooks, WASM host +
permissioned host imports, provider extensions.

## 2. Messages and content types (`oino-ai`)

pi: `AI/types` (lines ~247-436), `PI/core/messages.d.ts`, `PI/docs/message-types.md`.
Oino: `crates/oino-types/src/lib.rs:125-410`.

### 2.1 Content blocks

| pi type (exact JSON) | Oino | Gap |
|---|---|---|
| `{type:"text", text, textSignature?}` | `ContentBlock::Text{text, signature}` tag `kind:"text"` | ⚠️ tag key `kind`→`type`; `signature`→`textSignature` |
| `{type:"thinking", thinking, thinkingSignature?, redacted?}` | `Thinking{text, redacted, signature, provider_data}` | ⚠️ field names; `provider_data` has no pi equivalent (must fold into `thinkingSignature` or drop) |
| `{type:"image", data, mimeType}` | `Image{media_type, data}` | ⚠️ `media_type`→`mimeType` |
| `{type:"toolCall", id:string, name, arguments:JsonObject, thoughtSignature?, namespace?, argumentsError?}` | `ToolCall{id: Uuid, name, arguments, native_id}` | ⚠️ pi `id` is the provider's string id (no separate native id); missing `thoughtSignature`, `namespace`, `argumentsError` (invalid-JSON args → `{}` + error text, validation fails with it) |

### 2.2 Messages (`Message` = system | user | assistant | toolResult)

| pi | Fields (exact) | Oino | Gap |
|---|---|---|---|
| `SystemMessage` | `role:"system"`, `content: string\|TextContent[]`, `sections?: Record<string,string\|null>`, `toolsAdded?: Tool[]` (`{name,description,parameters,constrainedSampling?}`), `toolsRemoved?: {name}[]`, `timestamp` | `Message::System{id, content:String, sections:Vec<PromptSection>, tools_added:Vec<ToolDeclaration>, tools_removed:Vec<String>}` | 🟡 semantics match (leading = base prompt; later = patches; replay yields state — `oino-types/src/transcript.rs:136-235`). Shape differs: map vs vec (pi object key order is significant: insertion order), `input_schema`→`parameters`, removed as `{name}` objects, no timestamp, extra `id` |
| `UserMessage` | `role:"user"`, `content: string\|(Text\|Image)[]`, `timestamp` | `User{id, content:Vec<ContentBlock>}` | ⚠️ string form, timestamp, no id |
| `AssistantMessage` | `role:"assistant"`, `content:(Text\|Thinking\|ToolCall)[]`, `api`, `provider`, `model`, `responseModel?`, `responseId?`, `providerThinkingLevel?`, `thinkingLevel?`, `diagnostics?`, `usage`, `stopReason`, `deferred?`, `errorMessage?`, `rawStopReason?`, `endTurn?`, `timestamp`, `durationMs?` | `Assistant{id, content, stop_reason:Option, usage:Option, provider:Option<ProviderMetadata>, response_model, error_message, provider_thinking_level}` | ⚠️ missing `api`, flat `provider`/`model` strings, `responseId`, `thinkingLevel` (requested pi level), `diagnostics`, `deferred`, `rawStopReason`, `endTurn`, `timestamp`, `durationMs`; `usage`/`stopReason` are required in pi |
| `ToolResultMessage` | `role:"toolResult"`, `toolCallId`, `toolName`, `content:(Text\|Image)[]`, `details?`, `usage?`, `nestedCalls?:{calls:NestedToolCallRecord[],complete}`, `isError`, `timestamp`, `durationMs?` | `ToolResult{id, tool_call_id:Uuid, tool_name, content, is_error, terminate, details, usage, added_tool_names}` | ⚠️ `terminate` and `added_tool_names` are Oino-only on the message (pi: `terminate` lives on `AgentToolResult`, not persisted; tool additions are expressed by system messages); missing `nestedCalls`, `timestamp`, `durationMs` |

`NestedToolCallRecord` = `{id, name, arguments?, argumentsBytes?, status:"ok"|"error"|"unfinished", durationMs?, error?}`; limits: args > 8 KiB per call or 32 KiB per result omitted, ≤256 calls, `complete:false` when anything dropped (`PI/docs/extensions.md` "Tools"). Oino: ❌.

### 2.3 Coding-agent custom roles (`PI/core/messages.d.ts`)

| pi role | Fields | LLM conversion (`convertToLlm`, `messages.js`) | Oino | Gap |
|---|---|---|---|---|
| `bashExecution` | `command, output, exitCode\|undefined, cancelled, truncated, fullOutputPath?, timestamp, excludeFromContext?` | → user text `Ran \`cmd\`\n```\n{output}\n```` (or `(no output)`), then `\n\n(command cancelled)` or `\n\nCommand exited with code N`, then `\n\n[Output truncated. Full output: {path}]`; skipped when `excludeFromContext` | ❌ | new role + converter |
| `custom` | `customType, content: string\|(Text\|Image)[], display, details?, timestamp` | → user message with content (string→one text block) | ⚠️ `Custom{name, payload, model_visible}` | replace; `display` ≠ `model_visible` (pi custom messages are *always* in context; `display` is UI only) |
| `branchSummary` | `summary, fromId: string\|null, timestamp` | → user text `BRANCH_SUMMARY_PREFIX + summary + "</summary>"`; prefix `"The following is a summary of a branch that this conversation came back from:\n\n<summary>\n"` | 🟡 `BranchSummary{id, summary}` | add `fromId`, timestamp, exact prefix/suffix |
| `compactionSummary` | `summary, tokensBefore, timestamp` | → user text `"The conversation history before this point was compacted into the following summary:\n\n<summary>\n" + summary + "\n</summary>"` | 🟡 `CompactionSummary{id, summary}` | add `tokensBefore`, timestamp, exact wrapping |

### 2.4 Usage, stop reasons, JSON values

| pi | Oino | Gap |
|---|---|---|
| `Usage{input, output, cacheRead, cacheWrite, cacheWrite1h?, reasoning?, totalTokens, cost{input,output,cacheRead,cacheWrite,total}}` (USD) | `Usage{input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cache_write_1h_tokens, reasoning_tokens, cost: Option<{amount,currency}>}` | ⚠️ rename, add `totalTokens`, per-bucket cost breakdown; `reasoning` optional (undefined = not reported) |
| `StopReason = "pending"\|"stop"\|"length"\|"toolUse"\|"error"\|"aborted"\|"deferred"` | `EndTurn\|ToolUse\|Length\|Error\|Aborted\|Unknown` (snake_case) | ⚠️ rename (`stop`, `toolUse`), add `pending` (used for streaming partials: `message_start` assistant has `stopReason:"pending"`), `deferred`; drop `Unknown` |
| `ThinkingLevel = off\|minimal\|low\|medium\|high\|xhigh\|max` | same 7 variants (`lib.rs:104-115`) with `#[serde(rename_all = "snake_case")]` | ⚠️ `XHigh` serializes as `"x_high"`; pi wire value is `"xhigh"` (use `lowercase`) |

### 2.5 Owner and notes

Owner: `oino-ai`. Adopt pi-ai types verbatim (serde `rename_all="camelCase"`, `tag="role"` /
`tag="type"`, `skip_serializing_if` matching pi's optional fields). Message identity in Oino
should move to the *session entry id*; `context_edit.targetId` addresses entries, not messages
(see §6.4). The `Transcript` replay helpers in `oino-types/src/transcript.rs` (`current_system`,
`declared_tools`, `collapse`, `resolve(supports_mid_convo_system)`) map onto pi-ai's
`normalizeContext()`/system replay and should be kept, re-typed.

## 3. Streaming protocol and Model object (`oino-ai`)

### 3.1 `AssistantMessageEvent` (`AI/types` ~575-628)

pi events: `start{partial}`, `text_start/text_delta/text_end{contentIndex, delta|content, partial}`,
`thinking_start/_delta/_end`, `toolcall_start/_delta{delta}/_end{toolCall}`,
`done{reason: stop|length|toolUse|deferred, message}`, `error{reason: aborted|error, error}`.
Rules: `start` precedes updates; failures after start end with `error`; `partial` is the live
message; `*_end` content is authoritative; `toolcall_start` args are provider-specific.

Oino: `AssistantStreamEvent{Start, TextDelta, ThinkingDelta, ThinkingSignature, ToolCallDelta,
ToolCallDone, Usage, Done, Error, Aborted}` (`oino-types/src/lib.rs:367-410`). ⚠️ No
`contentIndex`, no `*_start/*_end`, no partial message; `Usage` and `ThinkingSignature` are
separate events. Provider WIT `provider.response.next()` returns `list<assistant-event>`
(`wit/extension.wit:46,125-135`).

Gap: replace with pi's event union (minus `partial`, which the host reconstructs — the JSON/RPC
wire drops `partial` anyway, `PI/docs/json.md`). Providers are WASM; WIT `assistant-event` must
carry the pi shape. Owner: `oino-ai` (types) + `wit/` (provider interface).

### 3.2 `Model` (`AI/types` 630-668)

pi `Model<Api>`: `id, name, api, provider, baseUrl, input:("text"|"image")[], inputLimits?, cost
{input,output,cacheRead,cacheWrite,tiers?}, headers?, type?:"chat", reasoning,
thinkingLevelMap?, promptCache?, contextWindow, maxTokens, samplingParams?,
samplingParamsByThinkingLevel?, compat?`. Also `ImageModel`, `ClassifierModel`
(`type:"image"|"classifier"`).

Oino `Model{provider, name, metadata: BTreeMap<String,String>}` with `provider:name` identifiers
(`oino-types/src/lib.rs:65-100`); catalogue data is in `oino-app/src/model_catalog.rs` and
provider `list-models` JSON. ⚠️ Gap: adopt pi `Model` (JSON object everywhere: RPC `get_state`,
`set_model` responses return the full object). pi identifiers are `provider/id` (slash), with
`:thinking` suffix in CLI patterns. Owner: `oino-ai` (type), `oino-models` (registry).

## 4. Agent loop (`oino-agent`, from `pi-agent-core`)

pi: `AC/types.d.ts`, `AC/agent-loop.d.ts`, `PI/docs/extensions.md` lifecycle diagrams.
Oino: `crates/oino-agent-loop/src/lib.rs`.

### 4.1 Loop configuration hooks

| pi `AgentLoopConfig` member | Semantics (pi) | Oino | Gap |
|---|---|---|---|
| `model`, `SimpleStreamOptions` (sessionId, cacheRetention, thinkingBudgets, transport, maxRetryDelayMs, identity, onPayload, onResponse, onProviderStreamEvent) | Passed to `streamFn` per request | `model`, `thinking_level`, `session_id`, `cache_retention` | 🟡 add transport, thinkingBudgets, maxRetryDelayMs, payload/response/stream-event callbacks (these back `before_provider_request`, `after_provider_response`, `provider_stream_event`) |
| `convertToLlm(messages)` | AgentMessage[] → Message[] before each call; must not throw | ❌ (custom handled ad hoc in harness `lib.rs:1316`) | add; pi default is `messages.ts convertToLlm` (§2.3) |
| `transformContext(messages, signal)` | AgentMessage-level transform before convert | `transform_context` | ✅ concept |
| `getApiKey(provider)` | Resolved per LLM call (short-lived OAuth) | ❌ (auth resolver lives in harness `AuthResolver`) | add |
| `finishTurn(turn, signal)` → `{action:"continue"\|"end"}\|void` | After assistant + tool results, before `turn_end`; `end` exits without polling queues; `continue` ensures one more request (satisfied by tool/steer/follow-up scheduling); error/aborted remain hard exits | `finish_turn` → `TurnDecision{Continue,End}` | ✅ concept; verify "continue satisfied by scheduling, adds no extra request" |
| `prepareRequest(ctx, signal)` → `{context?,model?,thinkingLevel?}` | Before *every* request incl. first; replacements persist for the run; does not poll queues | `prepare_request` → `RequestUpdate` | ✅ concept |
| `prepareNextTurn(ctx)` → `{context?,messages?,model?,thinkingLevel?}` | After `turn_end` when continuing; messages appended with lifecycle events (AgentSession compacts here) | ❌ | add (needed for mid-run threshold compaction) |
| `getSteeringMessages()` | Polled after the tool batch unless `finishTurn` ended | `get_steering_messages` | ✅ |
| `getFollowUpMessages()` | Polled when no tool calls and no steering | `get_follow_up_messages` | ✅ |
| `toolExecution: "sequential"\|"parallel"` (default parallel) | parallel: prepare sequentially, execute allowed concurrently; `tool_execution_end` in completion order; toolResult message events in assistant source order | `tool_execution` + per-tool `execution_mode`; any sequential tool forces whole batch sequential (`lib.rs:1186-1205`) | 🟡 check event ordering contract (end in completion order, result messages in source order) |
| `beforeToolCall(ctx, signal)` → `{block?,reason?,terminate?}` | After validation; block → error result with `reason` or default text | `before_tool_call` → `Allow(ToolCall)\|Block{reason,terminate}` | 🟡 pi ctx includes `assistantMessage`, `toolCall`, validated `args`, `context`; Oino allows rewriting the call (pi does it via in-place `event.input` mutation in the session layer) |
| `afterToolCall(ctx, signal)` → partial override | Field-wise replace of `content/details/structuredContent/isError/usage/terminate`; replacing `content` without `structuredContent` drops it | `after_tool_call` (whole `ToolResult`) | 🟡 implement merge semantics and `structuredContent` drop rule |
| (no turn cap) | Loop runs until no tool calls / end / error / abort | `max_turns: 16` default; loop stops at cap (`lib.rs:536,610`) | ⚠️ remove default cap (keep as opt-in `--max-turns` harness feature if desired, not core default) |

### 4.2 Tool contract (`AgentTool`, `AgentToolResult`)

| pi | Oino (`Tool` trait `lib.rs:358`, `ToolResult` `lib.rs:265`) | Gap |
|---|---|---|
| `name, description, parameters (TypeBox JSON Schema), label, prepareArguments?, outputSchema?, execute(id, params, signal, onUpdate), replay?:"never"\|"safe", executionMode?, constrainedSampling?` | `definition() -> ToolDeclaration{name,description,input_schema}`, `execution_mode()`, `prepare_arguments()`, `execute(call, updates, signal)` | 🟡 add `label`, `outputSchema`, `replay`, `constrainedSampling` |
| Validation: full JSON Schema validation of args after `prepareArguments`; `argumentsError` fails validation | `validate_arguments` checks only top-level `type` and `required` (`lib.rs:1146-1180`) | ⚠️ use a real JSON-Schema validator (draft matching TypeBox output) |
| Result: `{content, details, structuredContent?, usage?, isError?, terminate?}`; throw → error result with message text | `ToolResult{content, is_error, terminate, details, usage, added_tool_names}`; `LoopError::Tool(String)` → error | 🟡 add `structuredContent`; drop `added_tool_names` |
| `onUpdate(partial)` ignored after settle | `ToolUpdateCallback` | ✅ |
| `AgentToolCallOutcome{toolCall, result, isError, durationMs?}`; `durationMs` monotonic around `execute()` | ❌ durations | add |
| `runToolCall(toolCall, {tools, assistantMessage, context, signal, onUpdate, before/afterToolCall})`: same pipeline as model calls, never rejects | ❌ | add (backs `ctx.executeTool`, §10.4) |
| Terminate rule: stop after batch only if *every* finalized result sets `terminate` | `all_terminate` (`lib.rs:690-726`) | ✅ |

### 4.3 Agent events (`AC/types` `AgentEvent`)

| pi event | Payload | Oino `AgentEvent` (`lib.rs:94-140`) | Gap |
|---|---|---|---|
| `agent_start` | — | `AgentStart{run_id}` | 🟡 drop run_id or keep internal |
| `agent_end` | `messages` (new messages of the run) | `AgentEnd{run_id, stop_reason}` | ⚠️ add messages |
| `turn_start` | — | `TurnStart{turn}` | 🟡 |
| `turn_end` | `message`, `toolResults` | `TurnEnd{turn, stop_reason}` | ⚠️ |
| `message_start` | `message` (full AgentMessage; assistant partial `stopReason:"pending"`) | `MessageStart{message_id, role}` | ⚠️ |
| `message_update` | `message` (partial), `assistantMessageEvent` | `MessageDelta{message_id, delta: ContentDelta}` | ⚠️ |
| `message_end` | `message` | `MessageEnd{message}` | 🟡 shape via §2 |
| `tool_execution_start` | `toolCallId, toolName, args` | `ToolExecutionStart{call}` | 🟡 |
| `tool_execution_update` | `toolCallId, toolName, args, partialResult` | `ToolExecutionUpdate{call_id, update}` | 🟡 |
| `tool_execution_end` | `toolCallId, toolName, result, isError, durationMs?` | `ToolExecutionEnd{call_id, result}` | 🟡 |
| — | — | `QueueUpdate{queue, pending}`, `SavePoint{label}`, `Settled` | Oino-only at loop level; pi has `queue_update`/`agent_settled` at **session** level (§5.3). Move up. |

Message lifecycle events also fire for user and toolResult messages (pi), not only assistant.

### 4.4 `Agent` class (`AC/agent.d.ts`)

| pi member | Oino `Agent` (`crates/oino-agent/src/lib.rs`) | Gap |
|---|---|---|
| `constructor({initialState, convertToLlm, transformContext, streamFn, getApiKey, onPayload, onResponse, onProviderStreamEvent, before/afterToolCall, finishTurn, prepareRequest, prepareNextTurn, prepareNextTurnWithContext, steeringMode, followUpMode, sessionId, thinkingBudgets, transport, maxRetryDelayMs, toolExecution, identity})` | `new(config)`, `new_with_messages`, `with_queue_modes` | 🟡 |
| `state: {systemPrompt (replayed, read-only), model, thinkingLevel, tools (copy-on-set), messages (copy-on-set), isStreaming, streamingMessage?, pendingToolCalls, errorMessage?}` | `AgentState{model,…}` | 🟡 add streamingMessage, pendingToolCalls, errorMessage; tool diffs announced by system message before next request |
| `subscribe(listener(event, signal))` → unsubscribe; listeners awaited in order; `agent_end` listeners are part of settlement | `subscribe(subscriber)` (no unsubscribe, no signal) | 🟡 |
| `steeringMode`/`followUpMode` get/set; default **`one-at-a-time`** (`agent.js:141-142`) | `QueueMode::All` default (`lib.rs:130-131`) | ⚠️ default |
| `steer(msg)`, `followUp(msg)`, `clearSteeringQueue()`, `clearFollowUpQueue()`, `clearAllQueues()`, `hasQueuedMessages()`, `peekQueuedMessages()` | `steer`, `follow_up`, `peek_queued_messages` | 🟡 add clears/has |
| `signal` getter, `abort()`, `waitForIdle()` (after `agent_end` listeners) | `abort`, `wait_for_idle` | 🟡 |
| `reset()` | `reset` | ✅ |
| `prompt(msg\|msgs)`, `prompt(text, images?)`, `continue()` (last message must convert to user/toolResult) | `prompt(Message)`, `continue_from_current_context` | 🟡 |

## 5. AgentSession (`oino-core`)

pi: `PI/core/agent-session.d.ts` (934 lines), `agent-session.js`.
Oino: `crates/oino-harness/src/lib.rs` (`Harness`, `HookRegistry`) + `crates/oino-app/src/main.rs`.

### 5.1 Public API

| pi member | Behaviour (pi) | Oino | Gap |
|---|---|---|---|
| `constructor(AgentSessionConfig)` | `agent, sessionManager, settingsManager, cwd, scopedModels, resourceLoader, customTools, modelRuntime, cacheWarmer, initialActiveToolNames (default read,bash,edit,write), usesDefaultTools, defaultToolModifiers, allowedToolNames (globs), excludedToolNames, baseToolsOverride, extensionRunnerRef, sessionStartEvent` | `HarnessConfig{model, stream, session, …}` | ⚠️ |
| `subscribe(listener)` → unsubscribe | Session persistence is internal (messages saved on `message_end`) | `Harness` persists in app code | ⚠️ |
| `dispose()`, `refreshContext()` | | ❌ | |
| `state`, `model`, `thinkingLevel`, `routedModel`, `isStreaming`, `isIdle` (no run, compaction, branch summary, retry or queued continuation), `systemPrompt`, `retryAttempt`, `isCompacting`, `messages`, `steeringMode`, `followUpMode`, `sessionFile`, `sessionId`, `sessionName`, `scopedModels`, `promptTemplates`, `resourceLoader`, `pendingMessageCount`, `autoCompactionEnabled`, `autoRetryEnabled`, `isRetrying`, `isBashRunning`, `hasPendingBashMessages`, `cacheWarmingStatus`, `extensionRunner` | Read-only state | partial: `last_run_stop_reason`, `session_handle` | ❌ most |
| `prompt(text, {expandPromptTemplates=true, images, streamingBehavior, source, preflightResult})` | 1) extension commands dispatched immediately even while streaming; 2) `input` event (may `transform`/`handled`); 3) skill `/skill:name` + prompt template expansion; 4) streaming → requires `streamingBehavior` (`steer`/`followUp`) else throws; 5) idle → validate model+API key, compaction pre-check, `before_agent_start`, apply prompt/tool loadout, start run. Disposition `handled\|queued\|started` | `Harness::prompt(Message)`, `skill()`, `prompt_template()`; `on_input` hook | ⚠️ pipeline missing |
| `steer(text, images?, {source})`, `followUp(…)` → `handled\|queued` | Expand skills/templates; **throw** on extension commands | `steer` only | 🟡 |
| `sendCustomMessage({customType,content,display,details}, {triggerTurn, deliverAs: steer\|followUp\|nextTurn})` | 4 cases: streaming→queue (`nextTurn` waits for next user prompt); streaming+!trigger→appended after current turn's tool results; idle+trigger→append and start run (`before_agent_start` with `messageType:"custom"`); idle+!trigger→append only | ❌ | |
| `sendUserMessage(content, {deliverAs, expandPromptTemplates=false})` | Always triggers a turn | ❌ | |
| `clearQueue()` → `{steering[], followUp[]}`; `getSteeringMessages()`, `getFollowUpMessages()` (strings) | | ❌ | |
| `abort()` (waits idle), `waitForIdle()` | | `abort` | 🟡 |
| `setModel(model, {persist})` | Validates auth; appends `model_change`; emits `model_select{source:"set"}`; persists default only when `persist` | `set_model` | 🟡 |
| `cycleModel("forward"\|"backward", opts)` → `{model, thinkingLevel, isScoped}` | Scoped (`--models`/`enabledModels`) else all available | ❌ | |
| `setThinkingLevel(level, opts)` | Clamped to model; appends `thinking_level_change` only if changed; emits `thinking_level_select` + session `thinking_level_changed` | `set_thinking_level` | 🟡 clamping |
| `cycleThinkingLevel()`, `getAvailableThinkingLevels()` (`["off"]` for non-reasoning), `supportsThinking()` | | ❌ | |
| `setSteeringMode`, `setFollowUpMode` (saved to settings) | | ❌ | |
| `getActiveToolNames()`, `getCallableToolNames()`, `getAllTools()`, `getToolDefinition()`, `setActiveToolsByName()` (unknown/hidden ignored; rebuilds prompt; effective next turn) | Exposure model (§10.5) | `set_tools`, deferred tools | 🟡 |
| `compact(customInstructions?)` → `CompactionResult` | Aborts current run first; never retries/continues | `compact(summary)` (caller supplies summary) | ⚠️ |
| `abortCompaction()`, `abortBranchSummary()`, `setAutoCompactionEnabled()` | | ❌ | |
| `bindExtensions({uiContext, mode, commandContextActions, abortHandler, shutdownHandler, onError})` | | ❌ | |
| `reload({beforeSessionStart})` | Shutdown(reason reload) → reload resources/extensions → `session_start{reason:"reload"}` → `resources_discover{reason:"reload"}`; tools newly added to `defaultTools` activated | `/reload` in app | ⚠️ |
| `abortRetry()`, `setAutoRetryEnabled()` | §8 | ❌ | |
| `executeBash(cmd, onChunk, {excludeFromContext, id, operations})` → `BashResult{output, exitCode, cancelled, truncated, fullOutputPath?}` | Emits `user_bash` first (handler may supply `operations` or `result`); records `bashExecution` message; while streaming, pending until turn ends | ❌ | |
| `recordBashResult()`, `abortBash()` | | ❌ | |
| `setSessionName(name)` | Appends `session_info`; emits `session_info_changed` | `set_session_title` (custom entry + tool) | ⚠️ |
| `navigateTree(targetId, {summarize, customInstructions, replaceInstructions, label})` → `{editorText?, cancelled, aborted?, summaryEntry?}` | §7.3 | `navigate_tree(leaf)` (no summary, `before_tree_navigation` string hook) | ⚠️ |
| `getUserMessagesForForking()` → `{entryId, text}[]` | | ❌ | |
| `getSessionStats()` → `{sessionFile, sessionId, userMessages, assistantMessages, toolCalls, toolResults, totalMessages, tokens{input,output,cacheRead,cacheWrite,total}, cost, contextUsage?}` (all entries incl. compacted history and `usage` entries) | | `oino-app/src/usage.rs` | 🟡 move to core, pi shape |
| `getContextUsage()` → `{tokens\|null, contextWindow, percent\|null}` | Last valid assistant usage + chars/4 estimate of trailing messages (`compaction.js estimateContextTokens`); `null` right after compaction | app-side percent | 🟡 |
| `exportToHtml(path?, {themeName})`, `exportToJsonl(path?)` (header + current branch) | | `export_chat_html` from TUI state | ⚠️ export must be session-based |
| `summarizeForBugReport()` | | ➖ (pi bug-report flow; optional) | |
| `getLastAssistantText()` | | ❌ | |
| `createReplacedSessionContext()`, `hasExtensionHandlers(type)` | | ❌ | |
| `parseSkillBlock(text)` export | Regex `^<skill name="…" location="…">\n…\n</skill>(?:\n\n(…))?$` | ❌ | |

### 5.2 Prompt-time system prompt & tool loadout

pi (agent-session.js, `system-prompt.js`): each run builds `BuildSystemPromptOptions`
(`customPrompt` from SYSTEM.md or `--system-prompt`, `selectedTools`, `hiddenTools`,
`toolSnippets`, `toolGuidelines`, `promptGuidelines`, `appendSystemPrompt`, `sections`, `cwd`,
`contextFiles`, `skills`), lets `before_agent_start` mutate `systemPromptOptions` in place, then
diffs sections against the replayed transcript (`diffSystemPromptSections`) and prepends a
system patch message to the run's starting messages only when changed; the first request of a
session persists a full system message. A handler-returned `systemPrompt`
(`forceSystemPrompt`) is **not** recorded: it is projected onto the request after `context`
handlers (system messages collapse into one head with forced text + current tools).
`prepareLoadout` hidden declarations are filtered from every request.

Oino: harness merges section edits (`lib.rs:655,908`) and records System messages; tool
declaration diffs via `tool_declaration_canonical` (`lib.rs:1035`). 🟡 Missing: pi section set
and exact texts (§12.4), forced-prompt projection, hidden declarations, persistence of the first
full system message at first request (not at session creation).

### 5.3 Session events (`AgentSessionEvent`)

| Event | Payload | Oino | Gap |
|---|---|---|---|
| core events with `parentToolCallId?` on tool_execution_* | §4.3 | — | ❌ nested ids |
| `agent_end` | `messages, willRetry` | — | ❌ |
| `agent_settled` | `aborted` | `Settled` | 🟡 |
| `queue_update` | `steering: string[], followUp: string[]` (complete queues) | `QueueUpdate{queue,pending}` | ⚠️ |
| `compaction_start` | `reason: manual\|threshold\|overflow` | ❌ | |
| `compaction_end` | `reason, result?: CompactionResult, aborted, willRetry, errorMessage?` | ❌ | |
| `entry_appended` | `entry` (only for `pi.appendEntry`) | ❌ | |
| `session_info_changed` | `name\|undefined` | ❌ | |
| `thinking_level_changed` | `level` | ❌ | |
| `auto_retry_start` | `attempt, maxAttempts, delayMs, errorMessage` | ❌ | |
| `auto_retry_end` | `success, attempt, finalError?` | ❌ | |
| `summarization_retry_scheduled` | `attempt, maxAttempts, delayMs, errorMessage` | ❌ | |
| `summarization_retry_attempt_start` | `source:"branchSummary"` or `source:"compaction", reason` | ❌ | |
| `summarization_retry_finished` | — | ❌ | |
| `bash_execution_update` | `id?, delta` | ❌ | |

Owner: `oino-core`.

### 5.4 `AgentSessionRuntime` / services (`PI/core/agent-session-runtime.d.ts`, `agent-session-services.d.ts`)

| pi | Behaviour | Oino | Gap |
|---|---|---|---|
| `createAgentSessionServices({cwd, agentDir, settingsManager, modelRuntime, extensionFlagValues, resourceLoaderOptions})` → `{cwd, agentDir, modelRuntime, settingsManager, resourceLoader, diagnostics}` | cwd-bound services | ❌ (all in `main.rs`) | |
| `createAgentSessionFromServices({services, sessionManager, sessionStartEvent, model, thinkingLevel, scopedModels, tools, excludeTools, noTools, customTools})` | | ❌ | |
| `createAgentSession(options)` (SDK, `PI/core/sdk.d.ts`) | one-call SDK entry | ❌ | |
| `AgentSessionRuntime.newSession({parentSession, setup, withSession})` | `session_before_switch{reason:"new"}` (cancellable) → `session_shutdown{reason:"new", targetSessionFile}` → new runtime → `session_start{reason:"new", previousSessionFile}` → `withSession(freshCtx)` | `/new` in TUI | ⚠️ |
| `switchSession(path, {cwdOverride, withSession, projectTrustContextFactory})` | reason `resume`; may re-run `project_trust` for new cwd | `--session <uuid>` | ⚠️ |
| `fork(entryId, {position: "before"\|"at", withSession})` → `{cancelled, selectedText?}` | `session_before_fork` (cancel, `skipConversationRestore`) → new file with path to entry (`before` = parent of a user message; selected text returned for editor) | ❌ | |
| clone (`/clone`, RPC `clone`) | fork at current leaf, `position:"at"` | ❌ | |
| `importFromJsonl(path, cwdOverride)` | `SessionImportFileNotFoundError`, `MissingSessionCwdError` | ❌ | |
| `setRebindSession`, `setBeforeSessionInvalidate`, `dispose()`, `diagnostics`, `modelFallbackMessage` | | ❌ | |

## 6. SessionManager and session format v3 (`oino-session`)

pi: `PI/docs/session-format.md`, `PI/core/session-manager.d.ts`. Oino: `crates/oino-session/src/lib.rs`.

### 6.1 File and lifecycle

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Location | `<sessionDir>/--<cwd with leading sep removed, / \ : → ->--/<timestamp>_<sessionId>.jsonl`; default `~/.pi/agent/sessions` → Oino `~/.oino/sessions` (charter) | `~/.oino/sessions/<uuid>.jsonl` (`oino-app/src/main.rs:481-505`) | ⚠️ |
| Session dir precedence | `--session-dir` > `PI_CODING_AGENT_SESSION_DIR` > `sessionDir` setting (relative to cwd) > default | none | ❌ |
| Session id | UUID default; custom via `--session-id`/SDK; `assertValidSessionId`: letters, digits, `.`, `_`, `-`, start/end alnum | UUID only | ❌ |
| Header (line 1) | `{"type":"session","version":3,"id","timestamp"(ISO),"cwd","parentSession"?}` | `{"record":"header","header":{version:2,session_id,name,cwd},"leaf_id"}` | ⚠️ (name lives in `session_info`; leaf is *not* persisted — leaf = last appended entry on load) |
| Entry base | `{"type","id"(8 hex, UUID fallback),"parentId"\|null,"timestamp"(ISO)}` | `{"record":"entry","entry":{id:Uuid,parent,kind:{kind:…}}}` | ⚠️ |
| Persistence | Append-only line writes (`_persist`); file created only once a user **or assistant** message exists (setup entries stay in memory; #10000); `_rewriteFile` only for migration | Whole-file rewrite on every `save_jsonl` (`lib.rs:578-607`); `is_persisted()` always false | ⚠️ |
| Versions | v1 linear → v2 tree → v3 (`hookMessage`→`custom`); `migrateSessionEntries` on load, rewrite | Loads Oino v1/v2; rejects `>2` (`lib.rs:620`) | ❌ pi v1/v2/v3 load + Oino v1/v2 → v3 migration (charter) |
| Leaf on load | Last entry in file order | `leaf_id` from header + `LeafMove` entries | ⚠️ drop `LeafMove` (pi records branch moves implicitly: next append's `parentId`) |

### 6.2 Entry types

| pi `type` | Fields | Oino `SessionEntryKind` | Gap |
|---|---|---|---|
| `message` | `message: AgentMessage` (not compaction/branch summaries — `appendMessage` rejects them) | `Message{message}` | 🟡 via §2 |
| `thinking_level_change` | `thinkingLevel: string` | `ThinkingLevelChange` | 🟡 rename |
| `model_change` | `provider, modelId` | `ModelChange{model: Model}` | ⚠️ |
| `usage` | `kind, provider, model, usage, note?` (contributes to totals, hidden from tree, unknown kinds accepted) | `Usage{usage_kind, model, usage}` | 🟡 |
| `compaction` | `summary, firstKeptEntryId (required; self id = retain none), tokensBefore, details?, usage?, fromHook?, systemMessage?` | `Compaction{summary, first_kept: Option, tokens_before, details, replaces}` | 🟡 add `usage`, `fromHook`, `systemMessage`; `null` firstKept only in drafts |
| `context_edit` | `targetId, replacement: {content}\|null` | `ContextEdit{target, replacement: Option<Vec<ContentBlock>>}` | 🟡 wrapper object `{content}`; string replacement normalization (§6.4) |
| `branch_summary` | `fromId, summary, details?, usage?, fromHook?` | `BranchSummary{summary}` | ⚠️ |
| `custom` | `customType, data?` (never in context) | `Custom{name,payload}` + `ExtensionCustom{entry}` | ⚠️ unify |
| `custom_message` | `customType, content, display, details?` (in context as user) | `CustomMessage{message}` | ⚠️ |
| `label` | `targetId, label\|undefined` (undefined/empty clears) | `Label{label}` (labels the entry itself) | ⚠️ |
| `session_info` | `name?` | `SessionInfo{name, cwd}` | 🟡 drop cwd |
| — | — | `LeafMove{leaf_id}` | ⚠️ remove |

### 6.3 SessionManager API

| pi method | Oino | Gap |
|---|---|---|
| `static create(cwd, sessionDir?, {id?, parentSession?})`, `open(path, sessionDir?, cwdOverride?)`, `continueRecent(cwd, sessionDir?)`, `inMemory(cwd?, opts?, entries?)`, `forkFrom(sourcePath, targetCwd, sessionDir?, opts?)`, `findById(cwd, id, sessionDir?)`, `list(cwd, sessionDir?, onProgress?, signal?)`, `listAll(…)` | `new(header)`, `load_jsonl`, `SessionRepository{create,open,list}` | ❌ most |
| `SessionInfo{path, id, cwd, name?, parentSessionPath?, created, modified, messageCount, firstMessage, allMessagesText}` | ❌ | |
| `setSessionFile`, `newSession(opts)`, `isPersisted`, `getCwd`, `getSessionDir`, `usesDefaultSessionDir`, `getSessionId`, `getSessionFile`, `getHeader`, `getEntryCount` | `header()`, `is_persisted` (stub) | ❌ |
| `appendMessage`, `appendThinkingLevelChange`, `appendModelChange(provider, modelId)`, `appendUsage(kind, provider, model, usage, note?)` → entry, `appendCompaction(summary, firstKeptEntryId\|null, tokensBefore, details?, fromHook?, usage?)`, `appendCustomEntry(customType, data?)`, `appendSessionInfo(name)`, `appendCustomMessageEntry(customType, content, display, details?)`, `appendContextEdit(targetId, replacement)`, `appendLabelChange(targetId, label)` | `append_message`, `append_model`, `append_thinking_level`, `append_compaction`, `append_context_edit`, `append_usage`, `append_label`, `append_extension_custom` | 🟡 signatures |
| `getSessionName()` (latest `session_info`) | `get_session_name()` (derives from title/first message) | ⚠️ |
| `getLeafId`, `getLeafEntry`, `getEntry`, `getChildren(parentId)` → entries, `getLabel(id)`, `getBranch(fromId?)` (root→leaf path, all types), `getEntries()`, `getTree()` → `SessionTreeNode{entry, children, label?, labelTimestamp?}[]` (orphans as roots) | `get_leaf_id`, `get_leaf_entry`, `get_entry`, `get_children` (ids), `get_branch`, `get_entries`, `get_tree` (id map) | 🟡 |
| `branch(fromId)`, `resetLeaf()`, `branchWithSummary(fromId\|null, summary, details?, fromHook?, usage?)` → id, `createBranchedSession(leafId)` → new file path | `branch`, `reset_leaf`, `branch_with_summary` | 🟡 + `createBranchedSession` ❌ |
| `buildContextEntries()`, `buildSessionProjection()` → `{entries: ProjectedSessionEntry{sourceEntry, messages}[], messages, thinkingLevel, model}`, `buildSessionContext()` → `{messages, thinkingLevel (string, default "off"), model\|null}` | `build_session_context()` → `{messages, model, thinking_level, warnings}` | 🟡 projection with provenance missing |
| Free functions: `parseSessionEntries`, `migrateSessionEntries`, `getLatestCompactionEntry`, `sessionEntryToContextMessages`, `buildContextEntries/Projection/Context(entries, leafId?, byId?)`, `getDefaultSessionDir(cwd, agentDir?)`, `loadEntriesFromFile`, `findMostRecentSession` | ❌ | |
| `ReadonlySessionManager` (what extensions get) | `session-entries` WIT import (JSON of Oino entries) | ⚠️ |

### 6.4 Context building, projection and `context_edit`

pi rules (`session-format.md` "Context Building", `compaction.md`):
1. Walk leaf→root. If ≥1 compaction on path, use the latest: emit the compaction entry, then
   **non-system** entries from `firstKeptEntryId` up to (excl.) the compaction, then everything
   after. Non-message entries are kept in the selection (for rendering).
2. `buildSessionProjection` applies the latest `context_edit` per target **on the active branch**
   (branch-relative: navigating before the edit reveals the original). `null` omits; replacement
   swaps content only, keeps role/metadata. String replacements on assistant/toolResult are
   normalized to one text block. Valid targets: user, assistant, toolResult, custom_message.
3. `buildSessionContext`: model/thinking from the full path (virtual-aware: `getBranchSelection`,
   §16); `message`→stored message; `compaction`→**complete system checkpoint**
   (`compaction.systemMessage`, falling back to folded pre-compaction system messages) followed by
   `compactionSummary`; `branch_summary`→`branchSummary`; `custom_message`→`custom`;
   `context_edit`, `usage`, `custom`, `label`, `session_info`, model/thinking changes → nothing.

Oino (`lib.rs:453-570`): implements 1 (incl. v1 `replaces` fallback and folding dropped System
messages into one leading System message) and a simplified 2 (edits collected from the branch,
applied to *message ids*, last-wins). 🟡 Gaps: edits must key by **entry id** and be resolved
within the projection (Oino applies them after compaction selection by message id — same effect
only while message id == entry id invariant holds); no `ProjectedSessionEntry` provenance (needed
by `turn_end`/`agent_before_settle` `BoundaryContextPreview`, recovery omissions and
`_findPersistedMessageEntryId`); `systemMessage` checkpoint not stored on compaction; custom
message entry conversion; thinking level default `"off"` string.

Owner: `oino-session`.

## 7. Compaction, branch summaries, recovery (`oino-core::compaction`)

pi: `PI/docs/compaction.md`, `PI/core/compaction/{compaction,branch-summarization,utils}.{d.ts,js}`.
Oino: `crates/oino-app/src/llm_compact.rs`, `main.rs:7367 try_auto_compact`, `oino-harness::append_compaction`,
`oino-types::CompactMethod{Vcc,Llm}`, `extensions/crates/ext-vcc`.

### 7.1 Compaction

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Trigger | `contextTokens > contextWindow - reserveTokens` (`compaction.reserveTokens` 16384, per-model `compaction.modelOverrides["provider/id"]`); checked (a) before prompt submission, (b) mid-run in `prepareNextTurn` after tool results, (c) after `agent_end` | `usage_pct >= threshold_pct` in TUI loop only | ⚠️ |
| Context tokens | `estimateContextTokens`: last non-error/aborted assistant usage `totalTokens` (or sum) + `ceil(chars/4)` for trailing messages; projected variant over projection (`estimateProjectedContextTokens`) | app estimate | ⚠️ |
| Model limits | Limits of the physical model that produced the latest response (virtual-aware) | selected model metadata | 🟡 |
| Cut point (`findCutPoint`, `prepareCompaction`) | Walk back over the finalized projection accumulating estimates until `keepRecentTokens` (20000); valid cuts: user, assistant, bashExecution, custom_message, branch_summary; never toolResult; split-turn when one user span exceeds budget (`isSplitTurn`, `turnPrefixMessages`, two summaries merged); repeated compaction starts from previous `firstKeptEntryId` (fallback: entry after previous compaction); advance kept boundary over context-invisible suffix with omitted assistant attempts | caller chooses `first_kept` | ❌ |
| `CompactionPreparation` (event payload) | `messagesToSummarize, turnPrefixMessages, isSplitTurn, previousSummary, fileOps, firstKeptEntryId, tokensBefore, settings…` | ❌ | |
| Summary generation | Structured prompt: `## Goal`, `## Constraints & Preferences`, `## Progress` (`### Done/In Progress/Blocked`), `## Key Decisions`, `## Next Steps`, `## Critical Context`; then `<read-files>`/`<modified-files>`; iterative with previous summary; custom instructions appended; serialization `[User]: …`, `[Assistant]: …`, `[Assistant tool calls]: read(path="…"); …`, `[Tool result]: …` truncated at 2000 chars with marker; thinking omitted; prompt-cache writes disabled; output = model max tokens clamped to fit window | `llm_compact.rs` initial/iterative prompts, serialization (similar spirit) | 🟡 exact prompts/format/limits |
| File tracking | `CompactionDetails{readFiles, modifiedFiles}` from tool calls (read → read; write/edit → modified), cumulative from previous pi-generated compaction (not from `fromHook`) | ❌ | |
| Result | `CompactionResult{summary, firstKeptEntryId, tokensBefore, estimatedTokensAfter, usage?, details?}`; entry stores `systemMessage` checkpoint and `usage` | `append_compaction(summary, first_kept, tokens_before)` | 🟡 |
| Events | `compaction_start/_end` (session), `session_before_compact` (cancel / supply `compaction`), `session_compact{compactionEntry, fromExtension, reason, willRetry}`, `session_compact_failed{reason, errorMessage?, aborted, willRetry, fromExtension}` | `before_compaction` string mutation hook; WIT kind `compaction` | ❌ |
| Retry | Summarization uses `settings.retry` budget; `summarization_retry_*` events | ❌ | |
| Manual | `/compact [instructions]`, RPC `compact`, `ctx.compact({customInstructions,onComplete,onError})` — aborts run, no retry/continue | `/compact`, `/compact prompt`, `/compact threshold` | ⚠️ extra Oino subcommands are TUI/extension concerns |
| VCC method | — | `CompactMethod::Vcc` + `ext-vcc` | ➖ Oino-only; charter removes built-in vcc (port as extension via `session_before_compact`) |

### 7.2 Overflow and length recovery (pi ordering, `compaction.md`)

persist final assistant → `turn_end` → `agent_end` → append `context_edit` omissions for the failed
attempt (and its tool results) → for overflow/length: `session_before_compact` + compaction →
retry as a fresh run (`willRetry: true`, virtual route reason `retry`, `failed` passed). On
compaction failure/cancel: keep omissions, no compaction, no retry. Successful overflow without
retry (response exceeded window) compacts but keeps the response. Oino: ❌ entirely.

### 7.3 Branch summarization (`/tree`, `navigateTree`)

| Step | pi | Oino | Gap |
|---|---|---|---|
| Prepare | `TreePreparation{targetId, oldLeafId, commonAncestorId, entriesToSummarize (old leaf → common ancestor), userWantsSummary, customInstructions?, replaceInstructions?, label?}` | ❌ | |
| Event | `session_before_tree{preparation, signal}` → `{cancel?, summary?{summary,details?,usage?}, customInstructions?, replaceInstructions?, label?}` | `before_tree_navigation` (string) | ⚠️ |
| Budget | `branchSummary.reserveTokens` 16384; newest-first within budget; `branchSummary.skipPrompt` | ❌ | |
| Format | Same structure as compaction minus Critical Context; file lists cumulative from pi-generated summaries | ❌ | |
| Apply | `branchWithSummary(target, …)`; label attached; result `editorText` when target is a user message (leaf moves to its parent, text returned for re-editing) | `reset_leaf` | ❌ |
| After | `session_tree{newLeafId, oldLeafId, summaryEntry?, fromExtension?}` | ❌ | |

### 7.4 Context edit (append-only)

Covered in §6.4. Also used internally by pi for recovery omissions (`_omitRecoveryAttempt`) and
available to extensions via `turn_end`/`agent_before_settle` boundary drafts
(`ContextEditEntryDraft`). Oino has the entry; no producers other than app code. Owner:
`oino-session` (entry) + `oino-core` (producers).

## 8. Auto-retry (`oino-core`)

pi (`agent-session.js` `_isRetryableError`, `_prepareRetry`): retryable = not context overflow
(`isContextOverflow(message, contextWindow)`) and `isRetryableAssistantError` (overloaded, rate
limit, 5xx, transient stream drops). Settings `retry.enabled` (true), `retry.maxRetries` (3),
`retry.baseDelayMs` (2000, exponential), `retry.maxAgentDelayMs` (60000); provider-level
`retry.provider.{timeoutMs,maxRetries=0,maxRetryDelayMs=60000}`. Flow: failed attempt kept in
raw history, omitted via `context_edit`, `auto_retry_start`, sleep (cancellable via
`abortRetry`), `agent.continue()` with route reason `retry`; `auto_retry_end{success, attempt,
finalError?}`; cancel → `finalError:"Retry cancelled"`. `agent_end.willRetry` reflects it.
Oino: ❌ (no retry anywhere). Owner: `oino-core`.

## 9. Extension runtime and API (`oino-extension-host` + `oino-extension-sdk` + `wit/`)

pi: `PI/docs/extensions.md`, `PI/core/extensions/types.d.ts` (1756 lines), `runner.d.ts`.
Oino: `wit/extension.wit`, `crates/oino-wasm-host/src/lib.rs`, `crates/oino-extension-core/src/lib.rs`
(manifest/contribution registry), `crates/oino-extension-runtime/src/lib.rs` (`HookRunner`),
`crates/oino-app/src/wasm_extensions.rs` (wiring), `crates/oino-harness/src/lib.rs:65-310`
(`HookRegistry`).

Design note: pi extensions are in-process TS closures; Oino extensions are WASM components.
Parity means every `ExtensionAPI`/`ctx` capability is reachable from a guest via WIT host imports
(gated by manifest permissions) and every event is dispatched with pi's payload and result
semantics. Closures (`execute`, `route`, `reachesTools`, `prepareLoadout`, `getArgumentCompletions`,
handlers) become guest exports called by the host with a handle id.

### 9.1 Lifecycle and loading

| pi | Oino | Gap |
|---|---|---|
| Discovery: `~/.pi/agent/extensions/*.ts\|js` and `*/index.ts\|js`, `.pi/extensions` (trusted), `settings.extensions[]`, packages, `-e/--extension` (repeatable, `builtin:<name>`), `-ne` | `oino-extension-manager` package dirs, builtins, `/extensions` install | 🟡 different packaging (WASM packages); map discovery/settings arrays/CLI flags |
| Factory may be async; startup waits; no side effects in factory; long-lived resources from `session_start`, released idempotently in `session_shutdown` | `init(config) -> registration` (sync, static) | 🟡 |
| Registration is dynamic (anytime); provider/virtual-model registrations queued until runner binds | static `registration{tools, commands, hooks, providers}` + manifest contributions; dynamic tools only via `tool-output.added-tool-names` | ⚠️ |
| Handlers run in load/registration order; `pi.on()` returns unsubscribe | manifest `priority` ordering | 🟡 |
| Reload invalidates runtime (`assertActive`, stale instance errors) | `rebuild_wasm` replaces holder | 🟡 |
| Errors: handler errors reported (`ExtensionError{extensionPath, event, error, stack?}`, RPC `extension_error`), continue; `tool_call` handler failure **blocks** the tool; tool throw → error result; `user_bash` handler failure blocks | `hook_diag` diagnostics | 🟡 semantics |
| `InlineExtension{name, factory, hidden?, replaceable?, builtin?}` (SDK) | builtins crate | ➖/🟡 |

### 9.2 `ExtensionAPI` methods

| Method | pi semantics | Oino | Gap / WIT need |
|---|---|---|---|
| `on(event, handler)` → unsubscribe | 41 events (§9.3) | manifest hooks + `handle-event(kind,payload)` | 🟡 runtime subscribe/unsubscribe import |
| `registerTool(def)` | §9.5; direct/model-only activate on register unless `defaultActive:false`; re-register with `exposure:"hidden"` to withdraw | static tools + `added-tool-names` | ⚠️ `host.register-tool(decl)` |
| `registerCommand(name, {description, getArgumentCompletions, handler(args, cmdCtx)})` | handler gets `ExtensionCommandContext`; `ResolvedCommand.invocationName` (collisions get suffixes) | static `command-decl{name,description}` + `run-command(name,args)->json` | 🟡 completions, cmd ctx, dynamic |
| `registerShortcut(keyId, {description, handler(ctx)})` | | manifest `KeymapContribution` | 🟡 |
| `registerFlag(name, {type: boolean\|string, default, description})`, `getFlag(name)` | Flags parsed from CLI (`unknownFlags`), shown in `--help` | ❌ | `host.get-flag` + manifest flags |
| `registerMessageRenderer(customType, r)`, `registerEntryRenderer`, `registerToolRenderer(resolver)`, `registerMarkdownTransformer` | TUI rendering | `RendererContribution` | ➖ for core; frontend-agnostic replacement = structured UI (charter) |
| `sendMessage({customType, content, display, details}, {triggerTurn, deliverAs})` | §5.1 `sendCustomMessage` | ❌ | `host.send-message` |
| `sendUserMessage(content, {deliverAs, expandPromptTemplates})` | | ❌ | `host.send-user-message` |
| `appendEntry(customType, data?)` | `custom` entry; emits `entry_appended` | `session-append(kind, payload)` (Oino ExtensionSessionEntry) | ⚠️ shape |
| `setSessionName(name)`, `getSessionName()` | | ❌ | |
| `setLabel(entryId, label\|undefined)` | | ❌ | |
| `exec(command, args, {signal, timeout, cwd})` → `{stdout, stderr, code, killed}` | `execCommand` | `exec(cmd, args, cwd) -> {status, stdout:bytes, stderr:bytes}` | 🟡 add timeout/abort/killed; strings |
| `getActiveTools()`, `setActiveTools(names)`, `getAllTools()` → `ToolInfo{name, description, parameters, promptGuidelines, reachesTools, exposure, namespace?, annotations?, sourceInfo}` | | ❌ | |
| `getSettings()` | merged effective settings copy | ❌ (`init(config)` extras only) | |
| `getCommands()` → `SlashCommandInfo{name, description, source: extension\|prompt\|skill, sourceInfo}` | | ❌ | |
| `setModel(model)` → bool (false if no auth) | session-only | ❌ | |
| `getThinkingLevel()`, `setThinkingLevel(level)` (clamped) | | ❌ | |
| `registerProvider(name, ProviderConfig)` / `registerProvider(Provider)` / `unregisterProvider` | models replace provider's models; baseUrl-only overrides; `oauth`; `streamSimple`; `refreshModels`; immediate after bind | provider WIT world (`list-models`, `stream`) | 🟡 static providers only; no runtime override/oauth |
| `registerMcpServer`, `unregisterMcpServer`, `getMcpServers` | | — | ➖ (pi-mcp out of scope; reserve) |
| `registerVirtualModel(def)`, `unregisterVirtualModel(provider, id)` | §16 | ❌ | |
| `events: EventBus{emit(channel, data), on(channel, h)→unsub}` | shared across extensions, cleared on reload | ❌ | `host.events-emit` + event kind |

### 9.3 Events: payloads, results, composition

Composition patterns (`extensions.md`): *notification* (result ignored), *in-place mutation*
(`before_provider_headers.headers`, `before_agent_start.systemPromptOptions`, `tool_call.input`),
*chained transform* (`input`, `context`, `context_with_system`, `message_end`, `tool_result`,
`before_provider_request`: later handler gets earlier result), *gate* (handled input, blocked
tool call, cancelled session op, handled `user_bash`), *actionable boundary* (`turn_end`,
`agent_before_settle` see entries/continuation proposed so far), *last override*
(`cache_warming_decision`).

Oino wired today (`wasm_extensions.rs`): `input` (text mutate), `context` (messages), `tool_call`
(call mutate/block), `tool_result` (result mutate), `before_agent_start`, `turn_end`, observe-only
`agent_start/agent_end/agent_settled/message_start/message_end/turn_start/tool_execution_start/
tool_execution_end`, `cache_warming_decision` (veto). Harness also has string hooks
`before_provider_request/payload`, `after_provider_response`, `before_compaction`,
`before_tree_navigation` (`oino-harness/src/lib.rs:191-205`). `HookEventKind` enum
(`oino-extension-core/src/lib.rs:673`) lists many more kinds that are not dispatched.

| pi event | Payload (exact) | Result & semantics | Oino | Gap |
|---|---|---|---|---|
| `project_trust` | `{cwd}`; ctx `ProjectTrustContext{cwd, mode, hasUI, ui: select/confirm/input/notify}` | `{trusted: yes\|no\|undecided, remember?}`; only user-level + CLI extensions; first yes/no wins | kind `ProjectTrust` (not dispatched) | ❌ |
| `resources_discover` | `{cwd, reason: startup\|reload}` (after `session_start`) | `{skillPaths?, promptPaths?, themePaths?}` | kind only | ❌ |
| `mcp_servers_change` | `{servers}` | notify | — | ➖ |
| `session_start` | `{reason: startup\|reload\|new\|resume\|fork, previousSessionFile?}` | notify | `Session` kind | ❌ |
| `session_info_changed` | `{name\|undefined}` | notify | kind only | ❌ |
| `session_before_switch` | `{reason: new\|resume, targetSessionFile?}` | `{cancel?}` | kind only | ❌ |
| `session_before_fork` | `{entryId, position: before\|at}` | `{cancel?, skipConversationRestore?}` | kind only | ❌ |
| `session_before_compact` | `{preparation, branchEntries, customInstructions?, reason, willRetry, signal}` | `{cancel?, compaction?: CompactionResult}` | string hook | ❌ |
| `session_compact` | `{compactionEntry, fromExtension, reason, willRetry}` | notify | — | ❌ |
| `session_compact_failed` | `{reason, errorMessage?, aborted, willRetry, fromExtension}` | notify | kind only | ❌ |
| `session_shutdown` | `{reason: quit\|reload\|new\|resume\|fork, targetSessionFile?}` | notify (await, idempotent) | — | ❌ |
| `session_before_tree` | `{preparation: TreePreparation, signal}` | `{cancel?, summary?, customInstructions?, replaceInstructions?, label?}` | string hook | ❌ |
| `session_tree` | `{newLeafId, oldLeafId, summaryEntry?, fromExtension?}` | notify | — | ❌ |
| `context` | `{messages}` **without** system messages (pi restores prompt/tool state after) | `{messages?}` chained | `on_context(Vec<Message>)` runs as the loop's `transform_context` over the full transcript incl. System messages (`oino-harness/src/lib.rs:266-271,547-552`) | ⚠️ today's Oino `context` = pi `context_with_system`; split into the two pi events and strip/restore system messages for `context` |
| `context_with_system` | `{messages}` full transcript, system at index 0 | `{messages?}` chained; handler owns prompt/tools | kind only | ❌ |
| `cache_warming_decision` | `{warmCost, missCost, continuationProbability, action}` | `{action?: warm\|stop}` last wins | veto bool | 🟡 |
| `before_provider_headers` | `{headers}` mutate in place; `null` deletes | return ignored | kind only | ❌ (needs provider WIT support) |
| `before_provider_request` | `{payload}` (serialized provider payload) | replacement payload, chained | string hook (not wired to wasm) | 🟡 |
| `after_provider_response` | `{status, headers}` | notify | string hook | 🟡 |
| `provider_stream_event` | `{provider, api, model, data}` | notify, awaited in stream order, not persisted | kind only | ❌ |
| `before_agent_start` | base `{systemPrompt (readonly), systemPromptOptions (mutable)}` + `messageType:"user", prompt, images?` or `messageType:"custom", message` | `{message?: custom draft, systemPrompt?}`; message added to either run kind | `on_before_agent_start()` (no payload; returns sections/message) | 🟡 |
| `agent_start` | `{}` | notify | observe | 🟡 payload |
| `agent_end` | `{messages}` | notify | observe | 🟡 |
| `agent_before_settle` | `BoundaryState{entries: drafts, continue, context: BoundaryContextPreview{contextEntries, contextMessages, llmMessages, pendingMessages, canContinue}, outcome: completed\|aborted\|error}` | `{entries?, continue?}`; drafts = `custom`, `custom_message`, `context_edit`, `compaction`; continue → one more request | kind only | ❌ |
| `agent_settled` | `{aborted}` | notify | observe | 🟡 |
| `ui_prompt_start` / `ui_prompt_end` | `{reason:"ui_prompt", kind: select\|confirm\|input\|editor\|custom, title?}` | notify | kind only | ❌ |
| `turn_start` | `{turnIndex, timestamp}` | notify | observe | 🟡 |
| `turn_end` | `BoundaryState` + `{turnIndex, message, toolResults, messageEntryId, toolResultEntryIds}` | `{entries?, continue?}` | `on_turn_end` (Oino `TurnEndResult`) | 🟡 shape |
| `message_start` / `message_update` / `message_end` | `{message}` / `{message, assistantMessageEvent}` / `{message}` | `message_end` → `{message?}` replace (same role), chained | observe start/end only | 🟡 |
| `tool_execution_start/_update/_end` | §4.3 + `parentToolCallId?` | notify | observe start/end | 🟡 |
| `model_select` | `{model, previousModel, source: set\|cycle\|restore}` | notify | `ModelSelection` kind only | ❌ |
| `thinking_level_select` | `{level, previousLevel}` | notify | kind only | ❌ |
| `tool_call` | `{toolCallId, parentToolCallId?, toolName, input}` (typed per built-in) | `{block?, reason?, terminate?}`; mutate `input` in place, no re-validation; handler error blocks | `on_before_tool_call` | 🟡 |
| `tool_result` | `{toolCallId, parentToolCallId?, toolName, input, content, structuredContent?, isError, usage?, details}` | `{content?, details?, structuredContent?, isError?, usage?}` chained; content without structuredContent drops it | `on_after_tool_call` | 🟡 |
| `user_bash` | `{command, excludeFromContext, cwd}` | `{operations}` or `{result: BashResult}` stops propagation; undefined → next/local | — | ❌ |
| `input` | `{text, images?, source: interactive\|rpc\|extension, streamingBehavior?}` | `{action: continue}` / `{action: transform, text, images?}` / `{action: handled}` | text mutation only | 🟡 |

### 9.4 Contexts passed to handlers

| pi member | Semantics | Oino | Gap |
|---|---|---|---|
| `ctx.ui` | §9.6 | `notify`, `confirm` imports | ❌ most |
| `ctx.mode` (`tui\|rpc\|json\|print`), `ctx.hasUI` (true in tui+rpc) | | ❌ | |
| `ctx.cwd`, `ctx.sessionManager` (`ReadonlySessionManager`), `ctx.modelRegistry` (find, streamSimple, classify, findOfType, auth), `ctx.model`, `ctx.scopedModels`, `ctx.thinkingLevel` | | `session-entries`, `complete` (non-streaming, `model.complete` capability) | 🟡 |
| `ctx.isIdle()`, `ctx.isProjectTrusted()`, `ctx.signal`, `ctx.abort()`, `ctx.hasPendingMessages()`, `ctx.shutdown()` (orderly; RPC completes after command/settle), `ctx.getContextUsage()`, `ctx.compact(opts)`, `ctx.getSystemPrompt()` | | ❌ | |
| `ExtensionToolContext.tools` + `executeTool(name, args, {signal, onUpdate})` → `AgentToolCallOutcome` | nested id `<parent>/<n>`, hooks apply, `nestedCalls` record, usage rolled up | ❌ | |
| `ExtensionCommandContext`: `getSystemPromptOptions()`, `waitForIdle()`, `newSession({parentSession, setup, withSession})`, `fork(entryId, {position, withSession})`, `navigateTree(targetId, opts)`, `switchSession(path, {withSession})`, `reload()` | command-only (deadlock otherwise) | ❌ | |
| `ReplacedSessionContext` (+ `sendMessage`, `sendUserMessage`) given to `withSession` | old ctx invalid after replacement | ❌ | |

### 9.5 `ToolDefinition` fields

`name, label, description, promptSnippet?` (one line in default prompt tool list; custom tools
omitted without it), `promptGuidelines?` (bullets appended to Guidelines when active),
`parameters`, `constrainedSampling?`, `renderShell?` (➖), `prepareArguments?`, `outputSchema?`,
`exposure?: direct|model-only|codemode|deferred|hidden`, `namespace?{name, aliases?, description?,
instructions?}`, `reachesTools?(input) → "all" | words[]`, `annotations?{readOnlyHint,
destructiveHint, idempotentHint, openWorldHint}`, `defaultActive?`, `prepareLoadout?(loadout) →
{descriptions?, hiddenDeclarations?}`, `executionMode?`, `execute(id, params, signal, onUpdate,
ctx)`, `renderCall?/renderResult?` (➖). Helper `withFileMutationQueue(path, fn)`.

Oino `ToolContribution` (`oino-extension-core/src/lib.rs:610`) + WIT `tool-declaration{name,
description, input-schema}` + `tool-output{content, is-error, details, added-tool-names,
terminate}`. 🟡 Missing: label, snippet, guidelines, outputSchema/structuredContent, exposure,
namespace, annotations, defaultActive, prepareLoadout, reachesTools, usage on result. Oino's
"deferred tools" + host `tool_search` (`oino-tools/src/lib.rs:1096`) approximate `deferred`
exposure.

### 9.6 `ExtensionUIContext` (frontend-agnostic contract)

The runtime owns the *contract*; frontends render. RPC behaviour is the normative degraded mode
(`PI/docs/rpc-extension-ui.md`).

| Method | pi | RPC behaviour | Oino | Gap |
|---|---|---|---|---|
| `select(title, options, {signal, timeout})` → string\|undefined | dialog | request/response; timeout auto-resolves undefined | ❌ | |
| `confirm(title, message, opts)` → bool | | `confirmed` | `confirm(title, body)` | 🟡 no timeout/signal |
| `input(title, placeholder?, opts)` | | | ❌ (ask-user tool in app) | |
| `editor(title, prefill?)` | | | ❌ | |
| `notify(message, type: info\|warning\|error)` | fire-and-forget | `notifyType` | `notify(level:u8, text)` | 🟡 |
| `setStatus(key, text\|undefined)` | | fire-and-forget | `UiSurfaceContribution` (manifest) | ⚠️ |
| `setWidget(key, string[]\|factory\|undefined, {placement: aboveEditor\|belowEditor})` | | string arrays only | UI surfaces | ⚠️ |
| `setTitle(title)` | | fire-and-forget | ❌ | |
| `setEditorText`, `getEditorText` (`""` in RPC), `pasteToEditor` (→ setEditorText in RPC) | | `set_editor_text` | ❌ | |
| `onTerminalInput`, `setWorkingMessage/Visible/Indicator`, `setHiddenThinkingLabel`, `setFooter`, `setHeader`, `custom`, `addAutocompleteProvider`, `setEditorComponent/get…`, `theme/getAllThemes/getTheme/setTheme`, `get/setToolsExpanded` | TUI-only | no-op / undefined / `{success:false}` | ➖ | Oino replacement: structured widgets (charter) |

## 10. Built-in tools (`oino-tools`)

pi: `PI/core/tools/*.js`. Oino: `crates/oino-tools/src/lib.rs`, `crates/oino-env/src/lib.rs`.
Defaults: active `read, bash, edit, write`; also built-in but inactive `grep, find, ls,
powershell` (Windows); built-in extensions `codemode`, `tool_search` (➖ out of scope, but the
exposure plumbing they rely on is core). Constants: `DEFAULT_MAX_LINES=2000`,
`DEFAULT_MAX_BYTES=50*1024`, `GREP_MAX_LINE_LENGTH=500`.

### 10.1 Shared

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Path resolution `resolveToCwd` | `~` expansion, strip leading `@`, normalize Unicode spaces, relative to `ctx.cwd` | `resolve_to_cwd` | 🟡 `@`, `~`, unicode spaces |
| Read path fallbacks `resolveReadPath(Async)` | try as-is, macOS screenshot AM/PM narrow NBSP variant, NFD variant, curly-quote variant, NFD+curly | ❌ | |
| Truncation `truncateHead/Tail/Line/Middle`, `formatSize`, `TruncationResult{content, truncated, truncatedBy, totalLines, totalBytes, outputLines, outputBytes, lastLinePartial, firstLineExceedsLimit, maxLines, maxBytes}` | Never partial lines except tail edge case | `truncate_head`, `truncate_tail_with_notice` | 🟡 details object |
| File mutation queue | `withFileMutationQueue(absPath, fn)`: serialize mutations per file, parallel across files (edit, write) | write/edit declared `Sequential` (serializes whole batch) | 🟡 |
| `constrainedSampling: {type:"json_schema", strict:"prefer"}` on built-ins | | ❌ | |
| Abort | All tools check signal, "Operation aborted" | `abort_if_needed` | 🟡 messages |
| Prompt contributions | snippet + guidelines per tool (§12.4) | ❌ | |
| Pluggable `Operations` (read/write/edit/bash ops for SSH etc.) | | `ExecutionEnv` trait | ✅ concept |

### 10.2 `read`

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Schema | `{path, offset?, limit?}`; descriptions "Path to the file to read (relative or absolute)", "Line number to start reading from (1-indexed)", "Maximum number of lines to read" | same + Oino-only `paths: [{path,offset,limit}]` multi-read (30k char cap) | ⚠️ remove `paths` (or move to extension) |
| Description | `Read the contents of a file. Supports text files and images (jpg, png, gif, webp, bmp). Images are sent as attachments. For text files, output is truncated to 2000 lines or 50KB (whichever is hit first). Use offset/limit for large files. When you need the full file, continue with offset until complete.` | text-only variant | ⚠️ |
| Images | detect MIME (magic), `processImage` resize (`images.autoResize`, model `inputLimits.images.resize` or 2000×2000 fallback), content `[text "Read image file [mime]" + hints, image block]`; non-vision model note; failure → text note | text placeholder "[Image omitted: …OpenRouter adapter…]" (`lib.rs:273-284`) | ❌ |
| Offset OOB | `Offset N is beyond end of file (M lines total)` | `offset N is beyond end of file (M lines total)` (`lib.rs:69`) | 🟡 capitalize "Offset" |
| First line > 50KB | `[Line N is X, exceeds 50.0KB limit. Use bash: sed -n 'Np' path \| head -c 51200]` | ❌ | |
| Truncation footers | lines: `\n\n[Showing lines A-B of T. Use offset=N to continue.]`; bytes: `\n\n[Showing lines A-B of T (50.0KB limit). Use offset=N to continue.]`; user limit: `\n\n[R more lines in file. Use offset=N to continue.]` | `[Showing lines {o}-{e} of {t} ({reason}). …]` | ⚠️ exact strings |
| Empty file | content "" | `"{path} is empty"` | ⚠️ |
| `outputSchema` | `string \| {type:"image", data, mimeType, note}` | ❌ | |
| details | `{truncation?}` | none | ❌ |

### 10.3 `write`

pi: schema `{path, content}`; description `Write content to a file. Creates the file if it doesn't
exist, overwrites if it does. Automatically creates parent directories.`; mkdir -p; mutation
queue; result `Successfully wrote to {path}`; details undefined. Oino: same schema/description;
result `Successfully wrote {n} bytes to {path}` (`lib.rs:474-482`). 🟡 Fix result text, queue.

### 10.4 `edit`

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Schema | `{path, edits:[{oldText, newText}]}` + `prepareArguments` (accepts JSON-string `edits`, legacy top-level `oldText/newText`) | `{path, edits}` | 🟡 prepareArguments |
| Description/guidelines | identical text (`edit.js:85`) | identical (`lib.rs:521`) | ✅ |
| Access error | `Could not edit file: {path}. Error code: {code}.` | env error | 🟡 |
| BOM | stripped before matching, restored on write | ❌ | |
| Line endings | detect first of CRLF/LF; normalize to LF for matching (also edits' text); restore on write | ❌ | |
| Matching | exact first; else fuzzy in `normalizeForFuzzyMatch` space (NFKC, trailing whitespace per line, smart quotes→ASCII, dashes→`-`, special spaces→space); when fuzzy used, replacements computed on normalized base and unchanged lines copied back from original (`applyReplacementsPreservingUnchangedLines`) | exact only | ❌ |
| Errors | not found (1 edit): `Could not find the exact text in {path}. The old text must match exactly including all whitespace and newlines.`; (n): `Could not find edits[i] in {path}. The oldText must match exactly including all whitespace and newlines.`; duplicate: `Found K occurrences of the text in {path}. The text must be unique. Please provide more context to make it unique.` / `Found K occurrences of edits[i] in {path}. Each oldText must be unique. Please provide more context to make it unique.`; empty: `oldText must not be empty in {path}.` / `edits[i].oldText must not be empty in {path}.`; overlap; no change: `No changes made to {path}. The replacement produced identical content. This might indicate an issue with special characters or the text not existing as expected.` / `… The replacements produced identical content.` | `EditNotFound`, `EditNotUnique`, `EditOverlap`, `EmptyEdits` (`lib.rs:66-85`) | ⚠️ exact strings |
| Result | `Successfully replaced N block(s) in {path}.` + details `{diff, patch (unified), firstChangedLine?}` | text matches (`lib.rs:602`), no details | 🟡 |

### 10.5 `bash`

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Schema | `{command, timeout?}` ("Shell command to execute", "Timeout in seconds (optional, no default timeout)") | "Bash command to execute" | 🟡 description text |
| Description | `Execute a bash command in the current working directory. Returns stdout and stderr. Output is truncated to last 2000 lines or 50KB (whichever is hit first). If truncated, full output is saved to a temp file. Optionally provide a timeout in seconds.` | similar without temp-file sentence | 🟡 |
| Timeout validation | finite > 0 else `Invalid timeout: must be a finite number of seconds`; max 2147483.647 s | none | ❌ |
| Shell | `shellPath` setting / platform default; `shellCommandPrefix` prepended (`prefix\ncommand`); spawn detached (non-Windows), stdin ignored, stdout+stderr **interleaved** into one stream | `ExecutionEnv::shell` buffered, `stdout:`/`stderr:` sections (`format_command_output`) | ⚠️ |
| Env | `PI_SESSION_ID`, `PI_SESSION_FILE`, `PI_PROVIDER`, `PI_MODEL`, `PI_REASONING_LEVEL` (inherited copies removed first); guideline "You can inspect PI_* environment variables…"; `spawnHook` | ❌ | `OINO_*`? — decide naming; keep `PI_*` for compatibility of skills |
| Missing cwd | `Working directory does not exist: {cwd}\nCannot execute bash commands.` | ❌ | |
| Streaming | `onUpdate` with `{content:[text snapshot], details:{truncation?, fullOutputPath?}}`, throttled (`BASH_UPDATE_THROTTLE_MS`), initial empty update | none | ❌ |
| Output accumulator | streaming UTF-8 decode, rolling tail, temp file `pi-bash-*.log` opened when truncation needed | ❌ | |
| Truncation footer (tail) | partial last line: `[Showing last X of line N (line is Y). Full output: P]`; lines: `[Showing lines A-B of T. Full output: P]`; bytes: `[Showing lines A-B of T (50.0KB limit). Full output: P]`; empty → `(no output)` | own notice | ⚠️ |
| Abort / timeout | kill process tree; error `{output}\n\nCommand aborted` / `{output}\n\nCommand timed out after N seconds`; signal exit → 128+n; null exit → `Command terminated without an exit code` | `EnvError::Timeout` | ⚠️ |
| Non-zero exit | **result** (not throw) `{output}\n\nCommand exited with code N`, `isError:true` | error result with same suffix | 🟡 |
| `structuredContent` | `{output (≤1 MiB), truncated, full_output_path?, exit_code, wall_time_seconds}` | ❌ | |

### 10.6 `grep`, `find`, `ls`, `powershell`

pi built-ins (inactive by default; enabled via `--tools`/`defaultTools`). `grep`
(ripgrep-backed, `GREP_MAX_LINE_LENGTH` 500, details), `find` (glob, fd/rg), `ls`, `powershell`
(Windows shell tool sharing `createShellToolDefinition`, `tempFilePrefix` differs). Oino: ❌ (the
harness provides `ffgrep`/search via extensions). Owner: `oino-tools`. Lower priority but needed
for `--tools read,grep,find,ls` parity and typed `tool_call` events.

## 11. Bash executor and `!` commands (`oino-core`)

pi `bash-executor.ts` + `AgentSession.executeBash`: `!cmd` → included in context,
`!!cmd` → `excludeFromContext`; `user_bash` event first; result `BashResult{output, exitCode,
cancelled, truncated, fullOutputPath?}`; recorded as `bashExecution` message entry; if streaming,
held in pending bash messages and flushed after the turn; `bash_execution_update{id?, delta}`
streamed (RPC). Oino: ❌ (TUI has no `!`). Owner: `oino-core` (TUI just calls it).

## 12. Resource loading (`oino-resources`)

pi: `PI/docs/configuration.md`, `skills.md`, `prompt-templates.md`, `settings.md#resources`,
`security.md`, `PI/core/resource-loader.js`, `skills.js`, `prompt-templates.js`, `system-prompt.js`.
Oino: `crates/oino-resource/src/lib.rs`.

### 12.1 Directories

| pi | Oino equivalent (charter) | Oino today | Gap |
|---|---|---|---|
| Agent dir `~/.pi/agent` (`PI_CODING_AGENT_DIR`) | `~/.oino` (`OINO_DIR`?) | `~/.oino` | 🟡 env override |
| `<agent>/settings.json`, `keybindings.json`, `models.json`, `auth.json`, `trust.json`, `mcp.json` | same names | `settings.json`, auth file | 🟡 |
| `<agent>/{extensions,skills,prompts,themes}/` | same | skills, themes (prompts only project) | 🟡 global prompts missing |
| `<agent>/SYSTEM.md`, `APPEND_SYSTEM.md` | same | `SYSTEM.md` treated as **additional section** and auto-created with Oino default text (`lib.rs:45,213`) | ⚠️ pi: SYSTEM.md *replaces* default preamble; never auto-created |
| Project `.pi/` at **cwd** (not git root) | `.oino/` | `.oino/` at git root (`find_project_root`, `lib.rs:433`) | ⚠️ |
| `.pi/{settings.json, mcp.json, SYSTEM.md, APPEND_SYSTEM.md, extensions, skills, prompts, themes}` (trust-gated) | | `.oino/{settings.json, AGENT.md, prompts, skills, themes, exports}`; `ensure_skeleton` creates them | ⚠️ never create project files implicitly; `AGENT.md` is not a pi file |

### 12.2 Context files (AGENTS.md)

pi `loadProjectContextFiles`: candidates per dir, first match wins:
`AGENTS.override.md, AGENTS.md, AGENTS.MD, CLAUDE.md, CLAUDE.MD`. Order: agent dir file first,
then ancestors from filesystem root down to cwd (each dir once; nested git worktree shadows the
main repo copy). Not trust-gated; disabled by `-nc/--no-context-files`; BOM stripped. Rendered as
`project_context` section (`renderProjectContext`). Oino: only `.oino/AGENT.md`. ❌

### 12.3 SYSTEM.md / APPEND_SYSTEM.md / CLI

Precedence: `--system-prompt <text|path>` > trusted `.pi/SYSTEM.md` > `<agent>/SYSTEM.md` >
built-in preamble. Append: `--append-system-prompt` (repeatable, text or path) + trusted
`.pi/APPEND_SYSTEM.md` else `<agent>/APPEND_SYSTEM.md` (project wins, not combined) →
`addendum` section. Oino: ❌.

### 12.4 System prompt sections (exact, `system-prompt.js:67-121`)

Ordered sections; `preamble` untagged, every other wrapped `<name>\n…\n</name>`:
1. `preamble`: custom prompt, or `You are an expert coding assistant operating inside pi, a coding agent harness. You help users by reading files, executing commands, editing code, and writing new files.` (Oino must substitute its product name consistently.)
2. `tools` (default prompt only): `- name: snippet` lines for declared (non-hidden) tools with snippets, or `(none)`, then `\n\nIn addition to the tools above, you may have access to other custom tools depending on the project.`
3. `rules` (default only): `buildRules(declaredTools, toolGuidelines, promptGuidelines)`.
4. `docs` (default only): pi doc paths (Oino: its own docs paths).
5. `addendum`: append text.
6. `project_context`: context files.
7. `skills`: `formatSkillsForPrompt(skills, reader)` where reader = first declared of `read`/`bash`, or `indirect` if only hidden; omitted when neither.
8. `cwd`: cwd with `\`→`/`.
9. custom `sections` (name regex `SYSTEM_PROMPT_SECTION_NAME`, not `preamble`).
`forceSystemPrompt` → `{content: forced, sections: none}`. Oino: different sections
(`system_prompt_parts`, `main.rs:3472`). ⚠️

### 12.5 Skills

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Locations | `<agent>/skills`, trusted `.pi/skills`, `~/.agents/skills`, project `.agents/skills` from cwd up to repo root, `settings.skills[]`, `--skill` (repeatable), packages, `resources_discover.skillPaths`; `-ns` disables discovered/configured | global + project skills dirs | 🟡 |
| Format | dir with `SKILL.md` (recursive discovery); some standalone `.md` accepted | dir/SKILL.md | 🟡 |
| Frontmatter | `name` (lowercase a-z0-9-, no lead/trail/double hyphen, ≤64), `description` (≤1024, required), `license`, `compatibility`, `metadata`, `allowed-tools`, `disable-model-invocation`; most invalid fields warn | requires name == parent dir (`lib.rs:643`) | ⚠️ pi does **not** require dir match |
| Collisions | first discovered wins + warning | `DuplicateSkill` diagnostic | 🟡 |
| Prompt | `\n\nThe following skills provide specialized instructions for specific tasks.\nUse the read tool to load a skill's file when the task matches its description.` (bash/indirect variants) `\nWhen a skill file references a relative path, resolve it against the skill directory (parent of SKILL.md / dirname of the path) and use that absolute path in tool commands.\n\n<available_skills>` + per skill `  <skill>\n    <name>…</name>\n    <description>…</description>\n    <location>…</location>\n  </skill>` (XML-escaped) + `</available_skills>`; excludes `disable-model-invocation` | ❌ | |
| `/skill:name args` | `<skill name="{name}" location="{filePath}">\nReferences are relative to {baseDir}.\n\n{body}\n</skill>` + `\n\n{args}` when args; `enableSkillCommands` controls discovery only | `"Use the `x` skill…"` text (`lib.rs:344`) | ⚠️ |

### 12.6 Prompt templates

| Aspect | pi | Oino | Gap |
|---|---|---|---|
| Locations | `<agent>/prompts` (direct `.md` children), trusted `.pi/prompts`, `settings.prompts[]` (nested allowed), `--prompt-template`, packages, `resources_discover.promptPaths`; `-np` | project prompts only | 🟡 |
| Frontmatter | `description` (fallback: first non-empty line), `argument-hint` | both | ✅ |
| Args | `parseCommandArgs` shell-like quoting; `$1…`, `$@`, `$ARGUMENTS`, `${1:-default}`, `${@:-default}`, `${@:N}`, `${@:N:L}` | `$ARGUMENTS`, `$@`, `$N` (raw input) (`lib.rs:772`) | ⚠️ |
| Expansion order | extension command with same name wins; `input` event sees raw text first | — | ❌ |

### 12.7 Settings resource arrays and packages

`packages`, `extensions`, `skills`, `prompts`, `themes` arrays from user (relative to agent dir)
and project (relative to `.pi`) are **combined**; entries support `!glob` exclusion, `+path`
force-include, `-path` exclude; `builtin:<name>` entries for built-in extensions. Packages:
`npm:`/git/local sources (`PI/docs/packages.md`) — Oino uses WASM packages
(`oino-extension-manager`). 🟡 Keep Oino packaging; adopt array semantics. Owner:
`oino-resources` + `oino-extension-host`.

### 12.8 Project trust

pi: protected resources (`.pi/settings.json`, `.pi/mcp.json`, `.pi/{extensions,skills,prompts,
themes}`, `.pi/SYSTEM.md|APPEND_SYSTEM.md`, project `.agents/skills`). Decision order: CLI
`-a/--approve` / `-na/--no-approve` → `project_trust` extension event → nearest saved decision in
`<agent>/trust.json` (canonical paths) → `defaultProjectTrust` (`ask|always|never`, agent-dir
only). Non-interactive modes: `ask`/`never` skip. `sessionDir` read before trust. `/trust` saves.
Context files never gated. Oino: external-auth trust only (`oino-auth` `ExternalAuthTrust*`). ❌
Owner: `oino-resources`.

### 12.9 Keybindings file

`<agent>/keybindings.json` loaded by `KeybindingsManager` (`PI/core/keybindings.d.ts`,
`migrateKeybindingsConfig`), reloaded by `/reload`. Runtime owns load/validate/migrate; TUI owns
semantics. Oino: TUI keymaps in settings (`/settings keymaps`). 🟡 Owner: `oino-resources`
(file) + `oino-tui`.

## 13. Slash commands: runtime vs TUI

pi `BUILTIN_SLASH_COMMANDS` (`PI/core/slash-commands.d.ts`, `PI/docs/slash-commands.md`). Runtime
(`AgentSession.prompt`) handles: extension commands (any time), `/skill:name`, prompt templates.
Built-ins are interactive-mode commands that call runtime APIs; in RPC they map to commands.

| Command | Runtime API it calls | RPC equivalent | Oino |
|---|---|---|---|
| `/settings` | SettingsManager | — | TUI ✅ (different) |
| `/model [p/m]` | `setModel` | `set_model` | TUI 🟡 (`provider:model`) |
| `/thinking [lvl]` | `setThinkingLevel` | `set_thinking_level` | TUI 🟡 |
| `/scoped-models` | `setScopedModels` | — | ❌ |
| `/login`, `/logout` | ModelRuntime auth | — | `/auth`, `/account` 🟡 |
| `/new` | `runtime.newSession` | `new_session` | 🟡 |
| `/resume` | `switchSession` | `switch_session` | `/sessions` 🟡 |
| `/name [name]` | `setSessionName` / show | `set_session_name` | `/title` ⚠️ |
| `/session` | `getSessionStats` + cache warming status | `get_session_stats` | `/usage`, `/inspect` 🟡 |
| `/tree` | `navigateTree` | — (`get_tree`) | ❌ |
| `/fork` | `runtime.fork` + `getUserMessagesForForking` | `fork`, `get_fork_messages` | ❌ |
| `/clone` | fork at leaf | `clone` | ❌ |
| `/compact [instr]` | `compact` | `compact` | 🟡 |
| `/import <path>` | `importFromJsonl` | — | ❌ |
| `/copy` | `getLastAssistantText` | `get_last_assistant_text` | ❌ |
| `/export [path]` | `exportToHtml` / `exportToJsonl` | `export_html` | 🟡 TUI-state HTML |
| `/share`, `/bug` | upload | — | ➖ |
| `/trust` | trust manager | — | ❌ |
| `/reload` | `reload` | — | 🟡 |
| `/hotkeys`, `/changelog`, `/quit`, `/llama` | TUI / extension | — | ➖ |
| Oino-only: `/btw`, `/router …`, `/ralph …`, `/mode`, `/recall`, `/prompts`, `/skills`, `/extensions`, `/theme`, `/usage`, `/inspect` | — | — | charter: move to unipi extension ports or TUI |

## 14. CLI (`oino-cli`)

pi: `PI/docs/cli.md`, `PI/dist/cli/args.d.ts` (`Args`). Oino: hand parser `CliArgs`
(`crates/oino-app/src/main.rs:171-326`).

| pi flag | Semantics | Oino | Gap |
|---|---|---|---|
| positional `[messages…]`, `@file` (text/image into first prompt), piped stdin prepended, `--` | | single input string; `-p -` reads stdin | ⚠️ |
| `-p/--print` | one-shot, final assistant text to stdout | `-p/--prompt <text>` | ⚠️ semantics (pi `-p` is a mode switch, prompt is positional) |
| `--mode text\|json\|rpc` | | `--json` | ❌ |
| auto print mode when stdin/stdout redirected | | ❌ | |
| `--export <in> [out]` | HTML export and exit | ❌ | |
| `--provider`, `--model <pattern[:thinking]>`, `--api-key`, `--thinking`, `--models <patterns>`, `--list-models [search]` | §17 | `--model provider:id` | ❌ |
| `-c/--continue`, `-r/--resume`, `--session <path\|id\|partial>`, `--session-id <id>`, `--fork <path\|id>`, `--session-dir`, `--no-session`, `-n/--name` + constraints | | `--session <uuid>`, `--no-session` | ❌ |
| `-t/--tools <list\|+x,-y>` (globs), `-xt/--exclude-tools`, `-nbt/--no-builtin-tools`, `-nt/--no-tools` | | settings-based | ❌ |
| `-e/--extension` (repeatable, `builtin:`), `-ne`, `--no-mcp`, `--skill`, `-ns`, `--prompt-template`, `-np`, `--theme`, `--use-theme`, `--no-themes`, `-nc` | | ❌ | |
| `--system-prompt`, `--append-system-prompt` (repeatable) | | ❌ | |
| `--tui-mode`, `--verbose` | TUI | ❌ | ➖ TUI client |
| `-a/--approve`, `-na/--no-approve`, `--offline` (`PI_OFFLINE`) | | ❌ | |
| `-h/--help` (includes extension flags), `-v/--version` | | `--help` | 🟡 |
| extension long flags (`unknownFlags`) | | ❌ | |
| subcommands `install/remove/uninstall/update/list/config`, `auth check\|print-api-key\|print-bearer-token`, `mcp …` | | `settings …`, `/extensions update`, updater | 🟡 (Oino packages differ; `auth check` exit codes 0/1/2 worth matching) |
| Oino-only: `--settings`, `--max-turns`, `--max-cost`, `--max-wall`, `--max-tool-errors`, `--preset` | | | charter: budget/presets become extensions (via `registerFlag`) |

## 15. Print, JSON and RPC modes (`oino-cli`, `oino-rpc`)

### 15.1 Print mode (`PI/modes/print-mode.js`)

Run initial message (with `@file` images) then each extra message via `session.prompt`; text
mode prints every text block of the last assistant message, each followed by `\n`; exit 1 with
stderr `errorMessage || "Request {stopReason}"` on error/aborted; exit 1 with
`Response was truncated: the model reached its output token limit.` on `length`; SIGHUP→129,
SIGTERM→143; extension errors to stderr `Extension error ({path}): {error}`; ctx.mode `print`,
hasUI false. Oino `headless_exit_code` (`main.rs:910`) uses Oino caps (exit 2). ⚠️

### 15.2 JSON mode (`PI/docs/json.md`, `PI/modes/json-event.js`)

First line = session header object. Then every `AgentSessionEvent` (§5.3) as JSON. Wire
`message_update` is delta-only: drop cumulative `message` and every `partial`; add top-level
`usage` (latest cumulative); `toolcall_start` carries `id`, `toolName`. Strict LF framing;
stdout backpressure honoured; logs to stderr. Oino: `JsonLinesEventSink` emits Oino
`AgentEvent` + final summary object (`main.rs:780-870`). ⚠️ Owner: `oino-cli`.

### 15.3 RPC mode (`PI/docs/rpc.md`, `rpc-commands.md`, `PI/modes/rpc/rpc-types.d.ts`)

Framing: JSONL on stdin/stdout, split only on LF (strip CR), no U+2028/2029 splitting; optional
`id` echoed in `response`; malformed JSON → `{"type":"response","command":"parse",
"success":false,"error":"Failed to parse command: …"}`; failures `{…,"success":false,"error"}`;
events have no id except `bash_execution_update`; close stdin → orderly shutdown; `ctx.shutdown()`
completes after current command / `agent_settled`; `@file` args rejected; no session header.
Extra event `extension_error{extensionPath, event, error}`. Oino: ❌ entirely.

| Command | Request fields | Response `data` | Notes |
|---|---|---|---|
| `prompt` | `message, images?, streamingBehavior?` | `{disposition: handled\|queued\|started}` | error if streaming without behavior; ext commands run immediately |
| `steer` | `message, images?` | `{disposition: handled\|queued}` | ext commands rejected |
| `follow_up` | `message, images?` | same | |
| `abort` | — | — | waits idle |
| `clear_queue` | — | `{steering[], followUp[]}` | |
| `new_session` | `parentSession?` | `{cancelled}` | |
| `get_state` | — | `RpcSessionState{model?, thinkingLevel, isStreaming, isCompacting, steeringMode, followUpMode, sessionFile?, sessionId, sessionName?, autoCompactionEnabled, messageCount, pendingMessageCount}` | |
| `set_model` | `provider, modelId` | full `Model` | |
| `cycle_model` | — | `{model, thinkingLevel, isScoped}\|null` | |
| `get_available_models` | — | `{models: Model[]}` | |
| `set_thinking_level` | `level` | — | |
| `cycle_thinking_level` | — | `{level}\|null` | |
| `get_available_thinking_levels` | — | `{levels}` | |
| `set_steering_mode` / `set_follow_up_mode` | `mode` | — | |
| `compact` | `customInstructions?` | `CompactionResult` | |
| `set_auto_compaction` / `set_auto_retry` | `enabled` | — | |
| `abort_retry` | — | — | |
| `bash` | `command, excludeFromContext?` | `BashResult` | streams `bash_execution_update{id, delta}` |
| `abort_bash` | — | — | |
| `get_session_stats` | — | `SessionStats` | |
| `export_html` | `outputPath?` | `{path}` | |
| `switch_session` | `sessionPath` | `{cancelled}` | |
| `fork` | `entryId` | `{text, cancelled}` | |
| `clone` | — | `{cancelled}` | |
| `get_fork_messages` | — | `{messages: [{entryId, text}]}` | |
| `get_entries` | `since?` | `{entries, leafId}` | |
| `get_tree` | — | `{tree: SessionTreeNode[], leafId}` | |
| `get_last_assistant_text` | — | `{text\|null}` | |
| `set_session_name` | `name` | — | |
| `get_messages` | — | `{messages: AgentMessage[]}` | |
| `get_commands` | — | `{commands: RpcSlashCommand{name, description?, source: extension\|prompt\|skill, sourceInfo}[]}` | |

Extension UI sub-protocol: stdout `extension_ui_request{id, method, …}` with methods `select
{title, options, timeout?}`, `confirm{title, message, timeout?}`, `input{title, placeholder?,
timeout?}`, `editor{title, prefill?}` (dialogs, block for response) and fire-and-forget `notify
{message, notifyType?}`, `setStatus{statusKey, statusText?}`, `setWidget{widgetKey, widgetLines?,
widgetPlacement?}`, `setTitle{title}`, `set_editor_text{text}`; stdin `extension_ui_response{id,
value}` | `{id, confirmed}` | `{id, cancelled:true}`; agent-side timeouts auto-resolve
(undefined/false). Charter: this is the base for Oino's frontend-agnostic UI; Oino may *extend*
it (structured widgets) but must keep these records verbatim. Owner: `oino-rpc`; ACP adapter in
`oino-acp`. Also port `RpcClient` (typed client) as a Rust crate API for GPUI/TUI-over-RPC.

## 16. Virtual models (`oino-models` + `oino-core`)

pi: `PI/docs/virtual-models.md`, `PI/core/virtual-models.d.ts`. `VirtualModelDefinition{provider,
id (≠ physical id of provider), name, thinkingLevels? (default ["off"]), contextWindow?,
maxTokens?, input? (default text+image), route(request)}`; `VIRTUAL_MODEL_API = "pi-virtual"`;
route request `{model, thinkingLevel, reason: user|continuation|retry|direct, previous?,
failed?{model, thinkingLevel, message}, state?, messages (incl. system), signal}` → `{model
(physical, with credentials), thinkingLevel (clamped), state?}`; route throw / virtual / no-cred
target → error response; state stored as `custom` entry `pi.virtual-model-state` `{provider,
modelId, state}` before the request (unless `undefined` or identical object), branch-scoped,
survives compaction, ignored for `direct`; selection recorded in `model_change`, dispatch in
assistant message; `getBranchSelection` (virtual `model_change` holds until next; unregistered
virtual falls back to last physical response); context limits from latest physical response;
`routedModel` exposed; `PI_MODEL` shows selection. Oino: ❌ (router is a provider extension, not a
virtual model). Owner: `oino-models` (registry) + `oino-core` (routing in `prepareRequest`).

## 17. Models, registry, resolution, auth (`oino-models`)

### 17.1 models.json (`PI/docs/models-configuration.md`)

`providers.<id>`: `name, baseUrl, apiKey ("literal" | "$ENV" / "${ENV}" | "!command"), api,
oauth, headers (values resolve like apiKey), compat, authHeader, models[], modelOverrides{}`.
Model entry: `id, name, api, baseUrl, reasoning=false, thinkingLevelMap, input=["text"],
inputLimits{maxRequestBytes, images{resize{maxWidth,maxHeight,maxBytes,jpegQuality},
maxPerMessage, maxPerRequest}}, cost{input,output,cacheRead,cacheWrite,tiers[]}, promptCache,
contextWindow=128000, maxTokens=16384, samplingParams, samplingParamsByThinkingLevel, headers,
compat`. Overrides merge object fields field-wise. `compat` is per-API (provider extension
concern; core passes it through). Oino: `provider_config.rs`, provider extension `list-models`,
`oino-provider-catalog` static descriptors. ⚠️ Owner: `oino-models`; compat interpretation stays
in provider WASM extensions.

### 17.2 Model selection

`--model` exact id or fuzzy id/name; `provider/id`; `:thinking` suffix; multiple providers →
prefer one with credentials else require `--provider`; `--models` comma list with globs + suffix
→ scope for startup and cycling (`enabledModels` setting same format);
`defaultProvider/defaultModel/defaultThinkingLevel ("medium")`, `modelThinkingLevels
["provider/id"]`; restore from session `model_change`/branch selection (`model_select{source:
"restore"}`); `modelFallbackMessage` when stored model unavailable. Oino: `provider:model`
identifiers, `validate_model_identifier_with_extensions`. ❌

### 17.3 Auth storage (`PI/core/auth-storage.d.ts`)

`auth.json` map providerId → `Credential` (api key or OAuth `{access, refresh, expires, …}`);
file lock (`withLock`/`withLockAsync`) for concurrent processes; `read/modify/delete/list`;
`ReadOnlyAuthStorage`, `InMemory`; `readStoredCredential`; resolution order: `--api-key`
override > auth.json > provider env var > models.json `apiKey`; OAuth refresh via provider
`oauth.refreshToken`; `pi auth check` exit codes. Oino `oino-auth` (`AuthDocument`,
`AuthConfig{env_overrides, process_env}`, readiness/assessment, external trust) — richer
readiness model, different file shape, no locking. 🟡 Owner: `oino-models` (absorb `oino-auth`).

## 18. Settings (`oino-settings`)

pi `SettingsManager` (`PI/core/settings-manager.d.ts`): global `<agent>/settings.json` + project
`.pi/settings.json` (trusted), **project overrides global**, resource arrays combined, nested
objects merged (`compaction`, `retry`, …), file locking (`SettingsStorage.withLock`), migrations,
`SettingsError` diagnostics, `getGlobalSettings/getProjectSettings`, ~64 typed getters/setters,
`reload()`, `inMemory()`. Some keys agent-dir only (`defaultProjectTrust`, `httpProxy`,
`cacheWarming` global only). Oino: `ToolSettingsSnapshot` / `UserSettings` with Oino keys
(`tools`, `extensions`, `notify`, `compact.threshold_pct`, `theme`, collapse/chat-style…). ⚠️
Charter allows breaking keys → adopt pi keys.

Runtime-relevant pi keys (TUI-only keys listed last):

| Key | Default | Consumer |
|---|---|---|
| `defaultProvider`, `defaultModel`, `defaultThinkingLevel` (`medium`), `modelThinkingLevels`, `thinkingBudgets{minimal,low,medium,high}`, `enabledModels` | | oino-models / core |
| `steeringMode`, `followUpMode` (`one-at-a-time`) | | core |
| `defaultTools` (`read,bash,edit,write`; `+x/-x` modifiers; project applies on top) | | core |
| `sessionDir` | | session |
| `compaction.{enabled=true, reserveTokens=16384, keepRecentTokens=20000, modelOverrides}` | | core |
| `branchSummary.{reserveTokens=16384, skipPrompt=false}` | | core |
| `retry.{enabled=true, maxRetries=3, baseDelayMs=2000, maxAgentDelayMs=60000, provider.{timeoutMs, maxRetries=0, maxRetryDelayMs=60000}}` | | core / providers |
| `transport` (`auto`), `httpProxy`, `httpIdleTimeoutMs` (300000), `websocketConnectTimeoutMs` (15000) | | providers (host HTTP) |
| `cacheWarming` (`off\|streaming\|idle`, default `streaming`), `showCacheMissNotices` | | core (Oino has `cache_warming_mode_setting`) |
| `shellPath`, `shellCommandPrefix` | | tools |
| `images.{autoResize=true, blockImages=false}` | | tools / core |
| `packages`, `extensions`, `skills`, `prompts`, `themes`, `enableSkillCommands=true` | | resources |
| `defaultProjectTrust` (`ask`) | | resources |
| `codemode.{mode, inlineBudget}` | | ➖ codemode |
| `enableInstallTelemetry`, `enableAnalytics`, `collapseChangelog`, `warnings.anthropicExtraUsage`, `sessionShareDestination`, `npmCommand` | | ➖ / optional |
| TUI: `hideThinkingBlock`, `externalEditor`, `doubleEscapeAction`, `treeFilterMode`, `theme`, `quietStartup`, `tuiMode`, `fullscreen*`, `editorPaddingX`, `outputPad`, `autocompleteMaxVisible`, `showHardwareCursor`, `terminal.*`, `markdown.*` | | oino-tui |

## 19. Environment variables

Runtime: `PI_CODING_AGENT_DIR`, `PI_CODING_AGENT_SESSION_DIR`, `PI_OFFLINE`, `PI_CACHE_RETENTION`
(`long`), `HTTP(S)_PROXY`, `VISUAL/EDITOR`; exported to tools: `PI_SESSION_ID`,
`PI_SESSION_FILE`, `PI_PROVIDER`, `PI_MODEL`, `PI_REASONING_LEVEL`. Oino: none of these (some
`OINO_*` in app). Decide `OINO_*` names with `PI_*` aliases for skill compatibility. Owner:
`oino-core`/`oino-tools`.

## 20. Cache warming (`oino-core`)

pi `CacheWarmer` (`PI/core/cache-warmer.d.ts`): modes `off|streaming|idle`; warms only when model
`promptCache` known and estimated avoided miss cost ≥ $0.05; refresh recorded as `usage` entry
`kind:"cache_warm"`; `cache_warming_decision` event (last action wins); `/session` shows next
decision; `setCacheWarmingMode`. Oino `oino-harness/src/cache.rs` (`CacheTracker`,
`CacheWarmingMode`, veto hook, notices) — ✅ conceptually closest area; re-type to pi shapes
(`usage` entry with provider/model, event payload `{warmCost, missCost,
continuationProbability, action}` and `{action}` result).

## 21. Oino-only behaviour to remove or relocate

| Oino feature | Location | Action |
|---|---|---|
| `max_turns` default 16 | `oino-agent-loop/src/lib.rs:536` | remove default cap |
| `set_session_title` model tool + `has_explicit_session_title` | `oino-tools/src/lib.rs:61-180`, `oino-harness/src/lib.rs:317-330,1066-1090` | replace with `session_info` / `setSessionName`; tool → extension if wanted |
| `read.paths` multi-read | `oino-tools/src/lib.rs:233-330` | remove or extension |
| Host `tool_search` | `oino-tools/src/lib.rs:1096` | re-implement pi exposure (`deferred`) + optional extension |
| `LeafMove` entries, header `leaf_id` | `oino-session` | remove |
| `ExtensionCustom` entries | `oino-session` | fold into pi `custom` |
| `CompactMethod::Vcc`, budget/presets/notify/ralph/search/... | app + `extensions/crates/*` | charter: delete; unipi ports |
| Auto-created `SYSTEM.md`/`AGENT.md`/settings skeleton | `oino-resource/src/lib.rs:204-222` | remove (pi never writes resources implicitly) |
| `--max-cost/--max-wall/--max-tool-errors/--preset/--settings` | `oino-app/src/main.rs:171-326` | extension flags |
| Oino `HookEventKind` taxonomy (Startup, Install, Update, PackageLifecycle, Reload, …) | `oino-extension-core/src/lib.rs:673` | replace with pi's 41 event names verbatim |

## 22. Recommended implementation order

1. `oino-ai`: pi types + event protocol + Model; WIT `types` updated (providers re-built).
2. `oino-session`: v3 format, append-only persistence, projection, migration (pi v1/v2, Oino v1/v2).
3. `oino-agent`: loop contract (events, convertToLlm, prepareNextTurn, getApiKey, validation,
   runToolCall, durations, no cap, queue defaults) + `Agent` class.
4. `oino-tools`: exact read/write/edit/bash (+ truncation, mutation queue, images, PI_* env),
   then grep/find/ls.
5. `oino-resources` + `oino-settings`: context files, SYSTEM/APPEND, section prompt, skills,
   templates, trust, settings merge.
6. `oino-core`: AgentSession (prompt pipeline, queues, sendMessage, bash executor, model/thinking
   control, stats), compaction + branch summary + recovery + retry, runtime (new/switch/fork/
   clone/import), events.
7. `oino-extension-host`: WIT v2 covering §9 (all events, ExtensionAPI imports, contexts, UI
   contract, executeTool), runner semantics, flags/shortcuts/commands registries.
8. `oino-models`: models.json, resolver, scope/cycling, auth.json with locking, virtual models.
9. `oino-cli`: pi flags, print + JSON modes; `oino-rpc`: RPC + extension UI; then `oino-acp`.
10. TUI and GPUI clients on top of the same runtime / RPC client.

## 23. Verification approach (for implementers)

- Golden fixtures: record pi 1.4.0 sessions/JSON/RPC transcripts (`pi --mode json`,
  `pi --mode rpc` with a faux provider) and assert byte-level shape equality (modulo ids/timestamps).
- Session round-trip: load real `~/.pi/agent/sessions/**/*.jsonl` files, rebuild context, compare
  with pi's `buildSessionContext` output (node script against `PI/dist/core/session-manager.js`).
- Tool strings: unit tests asserting every message string quoted in §10.
- Compaction: port `compaction.test.ts`-style cases for `findCutPoint`/split turns.
- Upstream source for deeper reading: `github.com/earendil-works/pi` (`packages/coding-agent/src`,
  `packages/agent/src`, `packages/ai/src/types.ts`), tag for 1.4.0.
