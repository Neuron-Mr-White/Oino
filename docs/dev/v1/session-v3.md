# Messages and session format v3

Status: binding. Owner: `core-types-session` (`crates/oino-ai`, `crates/oino-session`).
Reference: audit-pi §2, §3.2, §6; pi `docs/session-format.md`, `dist/core/session-manager.*`,
`dist/core/messages.*`, pi-ai `dist/types.d.ts`.

Rule: every type here serialises to **exactly** pi's JSON (field names, tags, optionality).
All structs use `#[serde(rename_all = "camelCase")]`; every `Option` field uses
`#[serde(default, skip_serializing_if = "Option::is_none")]` unless marked *required*; every
`Vec` that pi marks optional uses `skip_serializing_if = "Vec::is_empty"` **only** where pi
omits empty arrays (noted). `serde_json` is built with `preserve_order`. Arbitrary JSON is
`serde_json::Value` (`JsonValue`) or `serde_json::Map` (`JsonObject`). Unknown fields are
ignored on read (never `deny_unknown_fields`).

## 1. `oino-ai` primitives

```rust
/// Unix epoch milliseconds (pi `number` timestamps on messages).
pub type TimestampMs = i64;
pub type JsonObject = serde_json::Map<String, serde_json::Value>;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel { Off, Minimal, Low, Medium, High, Xhigh, Max } // "xhigh" on the wire

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason { Pending, Stop, Length, ToolUse, Error, Aborted, Deferred }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputModality { Text, Image }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CacheRetention { None, Short, Long }

pub struct UsageCost { pub input: f64, pub output: f64, pub cache_read: f64, pub cache_write: f64, pub total: f64 }
pub struct Usage {
    pub input: u64, pub output: u64, pub cache_read: u64, pub cache_write: u64,
    pub cache_write_1h: Option<u64>,   // serde rename "cacheWrite1h"
    pub reasoning: Option<u64>,        // None = provider did not report
    pub total_tokens: u64,
    pub cost: UsageCost,               // required
}
```

`ThinkingLevel::clamp(self, available: &[ThinkingLevel]) -> ThinkingLevel` implements pi's
clamping (nearest lower available level, else lowest available; `Off` for non-reasoning).

### 1.1 Content blocks

```rust
pub struct TextContent { pub text: String, pub text_signature: Option<String> }
pub struct ThinkingContent { pub thinking: String, pub thinking_signature: Option<String>, pub redacted: Option<bool> }
pub struct ImageContent { pub data: String /* base64 */, pub mime_type: String }
pub struct ToolCall {
    pub id: String, pub name: String, pub arguments: JsonObject,
    pub thought_signature: Option<String>, pub namespace: Option<String>, pub arguments_error: Option<String>,
}

#[serde(tag = "type")]
pub enum UserContent { #[serde(rename = "text")] Text(TextContent), #[serde(rename = "image")] Image(ImageContent) }
#[serde(tag = "type")]
pub enum AssistantContent {
    #[serde(rename = "text")] Text(TextContent),
    #[serde(rename = "thinking")] Thinking(ThinkingContent),
    #[serde(rename = "toolCall")] ToolCall(ToolCall),
}
/// pi `string | (TextContent|ImageContent)[]` — untagged; a JSON string stays a string.
#[serde(untagged)]
pub enum UserContentField { Text(String), Blocks(Vec<UserContent>) }
```

`ToolResult` content uses `Vec<UserContent>` (text|image).

### 1.2 `Message` (pi-ai) and `AgentMessage` (coding agent)

```rust
pub struct ToolDecl { pub name: String, pub description: String, pub parameters: serde_json::Value,
                      pub constrained_sampling: Option<ConstrainedSampling> }
#[serde(untagged)] pub enum ConstrainedSampling { Disabled(FalseLiteral), Config(ConstrainedSamplingConfig) }
pub struct ConstrainedSamplingConfig { #[serde(rename = "type")] pub kind: String /* "json_schema" */, pub strict: Option<StrictMode> }
#[serde(rename_all = "lowercase")] pub enum StrictMode { Prefer, Require, Off }
pub struct ToolRef { pub name: String }

pub struct SystemMessage {
    pub content: UserTextField,                         // string | TextContent[]
    pub sections: Option<IndexMap<String, Option<String>>>, // null = remove section; order significant
    pub tools_added: Option<Vec<ToolDecl>>,
    pub tools_removed: Option<Vec<ToolRef>>,
    pub timestamp: TimestampMs,
}
pub struct UserMessage { pub content: UserContentField, pub timestamp: TimestampMs }
pub struct AssistantMessage {
    pub content: Vec<AssistantContent>,
    pub api: String, pub provider: String, pub model: String,
    pub response_model: Option<String>, pub response_id: Option<String>,
    pub provider_thinking_level: Option<String>, pub thinking_level: Option<ThinkingLevel>,
    pub diagnostics: Option<Vec<AssistantDiagnostic>>,
    pub usage: Usage, pub stop_reason: StopReason,          // required
    pub deferred: Option<DeferredHandle>,
    pub error_message: Option<String>, pub raw_stop_reason: Option<String>, pub end_turn: Option<bool>,
    pub timestamp: TimestampMs, pub duration_ms: Option<u64>,
}
pub struct AssistantDiagnostic { #[serde(rename="type")] pub kind: String, pub timestamp: TimestampMs,
                                 pub error: Option<DiagnosticError>, pub details: Option<JsonObject> }
pub struct DiagnosticError { pub name: Option<String>, pub message: String, pub stack: Option<String>, pub code: Option<serde_json::Value> }
pub struct DeferredHandle { pub provider: String, pub model_id: String, pub api: String, pub id: String,
                            pub expires_at: Option<i64>, pub poll_after_ms: Option<u64>, pub data: Option<serde_json::Value> }
pub struct NestedToolCallRecord { pub id: String, pub name: String, pub arguments: Option<JsonObject>,
    pub arguments_bytes: Option<u64>, pub status: NestedStatus /* ok|error|unfinished */, pub duration_ms: Option<u64>, pub error: Option<String> }
pub struct NestedToolCalls { pub calls: Vec<NestedToolCallRecord>, pub complete: bool }
pub struct ToolResultMessage {
    pub tool_call_id: String, pub tool_name: String, pub content: Vec<UserContent>,
    pub details: Option<serde_json::Value>, pub usage: Option<Usage>, pub nested_calls: Option<NestedToolCalls>,
    pub is_error: bool, pub timestamp: TimestampMs, pub duration_ms: Option<u64>,
}

#[serde(tag = "role")]
pub enum Message {                                       // pi-ai Message
    #[serde(rename = "system")] System(SystemMessage),
    #[serde(rename = "user")] User(UserMessage),
    #[serde(rename = "assistant")] Assistant(AssistantMessage),
    #[serde(rename = "toolResult")] ToolResult(ToolResultMessage),
}

pub struct BashExecutionMessage { pub command: String, pub output: String, pub exit_code: Option<i32>,
    pub cancelled: bool, pub truncated: bool, pub full_output_path: Option<String>,
    pub timestamp: TimestampMs, pub exclude_from_context: Option<bool> }
pub struct CustomMessage { pub custom_type: String, pub content: UserContentField, pub display: bool,
    pub details: Option<serde_json::Value>, pub timestamp: TimestampMs }
pub struct BranchSummaryMessage { pub summary: String, pub from_id: Option<String> /* JSON null allowed, field always present */, pub timestamp: TimestampMs }
pub struct CompactionSummaryMessage { pub summary: String, pub tokens_before: u64, pub timestamp: TimestampMs }

#[serde(tag = "role")]
pub enum AgentMessage {
    #[serde(rename = "system")] System(SystemMessage),
    #[serde(rename = "user")] User(UserMessage),
    #[serde(rename = "assistant")] Assistant(AssistantMessage),
    #[serde(rename = "toolResult")] ToolResult(ToolResultMessage),
    #[serde(rename = "bashExecution")] BashExecution(BashExecutionMessage),
    #[serde(rename = "custom")] Custom(CustomMessage),
    #[serde(rename = "branchSummary")] BranchSummary(BranchSummaryMessage),
    #[serde(rename = "compactionSummary")] CompactionSummary(CompactionSummaryMessage),
}
```

`BranchSummaryMessage.from_id` serialises as `"fromId": null` when `None` (pi field is
`string | null`, always present) — use `#[serde(default)]` without `skip_serializing_if`.
`UserTextField` = untagged `string | Vec<TextContent-with-type-tag>`.
`impl From<Message> for AgentMessage` and `AgentMessage::as_llm(&self) -> Option<&Message>`.

### 1.3 Model (pi-ai `Model`)

```rust
pub struct Model {
    pub id: String, pub name: String, pub api: String, pub provider: String, pub base_url: String,
    pub input: Vec<InputModality>, pub input_limits: Option<InputLimits>, pub cost: ModelCost,
    pub headers: Option<IndexMap<String, String>>,
    #[serde(rename = "type")] pub kind: Option<ModelKindChat>,  // "chat" only for chat models
    pub reasoning: bool, pub thinking_level_map: Option<ThinkingLevelMap>, pub prompt_cache: Option<PromptCache>,
    pub context_window: u64, pub max_tokens: u64,
    pub sampling_params: Option<JsonObject>, pub sampling_params_by_thinking_level: Option<IndexMap<ThinkingLevel, JsonObject>>,
    pub compat: Option<serde_json::Value>,          // opaque to core; interpreted by provider components
}
pub struct ModelCost { pub input: f64, pub output: f64, pub cache_read: f64, pub cache_write: f64, pub tiers: Option<Vec<ModelCostTier>> }
pub struct ModelCostTier { pub input: f64, pub output: f64, pub cache_read: f64, pub cache_write: f64, pub input_tokens_above: u64 }
pub struct ThinkingLevelMap { /* off, minimal, low, medium, high, xhigh, max: Option<Option<String>> (absent | null | string) */ }
pub struct PromptCache { pub short: Option<f64>, pub long: Option<f64> }
pub struct InputLimits { pub max_request_bytes: Option<u64>, pub images: Option<ImageInputLimits> }
pub struct ImageInputLimits { pub resize: Option<ImageResize>, pub max_per_message: Option<u32>, pub max_per_request: Option<u32> }
pub struct ImageResize { pub max_width: Option<u32>, pub max_height: Option<u32>, pub max_bytes: Option<u64>, pub jpeg_quality: Option<u8> }

impl Model { pub fn key(&self) -> ModelKey /* "provider/id" */; pub fn available_thinking_levels(&self) -> Vec<ThinkingLevel>; }
pub struct ModelKey { pub provider: String, pub id: String }  // Display "provider/id"
```

`ImageModel`/`ClassifierModel` are `AnyModel` variants (`type: "image" | "classifier"`), stored
in the registry but never selectable as chat models.

### 1.4 Streaming (`AssistantMessageEvent`) and request types

```rust
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    Start { partial: AssistantMessage },
    TextStart { content_index: usize, partial: AssistantMessage },
    TextDelta { content_index: usize, delta: String, partial: AssistantMessage },
    TextEnd { content_index: usize, content: String, partial: AssistantMessage },
    ThinkingStart { .. }, ThinkingDelta { .. }, ThinkingEnd { .. },            // same shapes as text
    #[serde(rename = "toolcall_start")] ToolcallStart { content_index: usize, partial: AssistantMessage },
    #[serde(rename = "toolcall_delta")] ToolcallDelta { content_index: usize, delta: String, partial: AssistantMessage },
    #[serde(rename = "toolcall_end")] ToolcallEnd { content_index: usize, tool_call: ToolCall, partial: AssistantMessage },
    Done { reason: DoneReason /* stop|length|toolUse|deferred */, message: AssistantMessage },
    Error { reason: ErrorReason /* aborted|error */, error: AssistantMessage },
}
```

(Field names inside variants are camelCase: `contentIndex`, `toolCall`.) Providers emit
**`ProviderEvent`** = the same union **without `partial`** (WIT boundary, D5); `oino-ai`
provides `PartialAssembler` that rebuilds `partial` from `ProviderEvent`s, so the host
reconstructs pi events exactly. `toolcall_start` carries optional `id` and `toolName` in
`ProviderEvent` (pi JSON wire adds them; see modes §3).

```rust
pub struct Context { pub messages: Vec<Message> }    // normalised transcript (system msgs carry prompt + tools)
pub struct StreamOptions {
    pub reasoning: Option<ThinkingLevel>, pub max_tokens: Option<u64>, pub temperature: Option<f64>,
    pub sampling_params: Option<JsonObject>, pub cache_retention: Option<CacheRetention>,
    pub session_id: Option<String>, pub transport: Option<Transport /* sse|websocket|websocket-cached|auto */>,
    pub thinking_budgets: Option<ThinkingBudgets>, pub tool_choice: Option<serde_json::Value>,
    pub timeout_ms: Option<u64>, pub max_retries: Option<u32>, pub max_retry_delay_ms: Option<u64>,
    pub metadata: Option<JsonObject>, pub headers: Option<IndexMap<String, Option<String>>>,
}
```

`oino_ai::transcript` keeps Oino's existing replay logic re-typed to pi shapes:
`current_system(&[Message]) -> SystemState{content, sections, tools}`, `diff_sections(old,
new) -> Option<SystemMessage>` (pi `diffSystemPromptSections`), `normalize_context(Context)`
(collapse leading system messages, fold later ones for providers without mid-conversation
system support; signature dropping when `api/provider/model` differ, pi `transform-messages`).

## 2. Session entries (`oino-session::entry`)

```rust
/// 8 lowercase hex chars normally; any string accepted on read (UUID fallback, legacy files).
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EntryId(String);
/// ISO-8601 UTC with millisecond precision and `Z` (JS `toISOString`). Original text preserved.
#[serde(transparent)] pub struct IsoTimestamp(String);
impl IsoTimestamp { pub fn now() -> Self; pub fn from_ms(ms: i64) -> Self; pub fn to_ms(&self) -> Option<i64>; }

pub struct SessionHeader {            // first line, `"type":"session"`
    pub version: u32,                 // 3 after migration
    pub id: String,                   // session id (UUID by default)
    pub timestamp: IsoTimestamp,
    pub cwd: String,
    pub parent_session: Option<String>,
}

pub struct SessionEntry {
    pub id: EntryId,
    pub parent_id: Option<EntryId>,   // serialised as `"parentId": null` for roots (always present)
    pub timestamp: IsoTimestamp,
    #[serde(flatten)] pub kind: EntryKind,
}

#[serde(tag = "type", rename_all = "snake_case")]
pub enum EntryKind {
    Message { message: AgentMessage },
    ThinkingLevelChange { #[serde(rename = "thinkingLevel")] thinking_level: String },
    ModelChange { provider: String, #[serde(rename = "modelId")] model_id: String },
    Usage { kind: String, provider: String, model: String, usage: Usage, note: Option<String> },
    Compaction(CompactionEntry),
    ContextEdit { #[serde(rename = "targetId")] target_id: EntryId, replacement: Option<ContextReplacement> /* null kept */ },
    BranchSummary(BranchSummaryEntry),
    Custom { #[serde(rename = "customType")] custom_type: String, data: Option<serde_json::Value> },
    CustomMessage { #[serde(rename = "customType")] custom_type: String, content: UserContentField, display: bool, details: Option<serde_json::Value> },
    Label { #[serde(rename = "targetId")] target_id: EntryId, label: Option<String> },
    SessionInfo { name: Option<String> },
}
pub struct CompactionEntry { pub summary: String, pub first_kept_entry_id: EntryId, pub tokens_before: u64,
    pub details: Option<serde_json::Value>, pub usage: Option<Usage>, pub from_hook: Option<bool>,
    pub system_message: Option<SystemMessage> }
pub struct BranchSummaryEntry { pub from_id: EntryId, pub summary: String, pub details: Option<serde_json::Value>,
    pub usage: Option<Usage>, pub from_hook: Option<bool> }
pub struct ContextReplacement { pub content: EditableContent }
#[serde(untagged)] pub enum EditableContent { Text(String), User(Vec<UserContent>), Assistant(Vec<AssistantContent>) }

pub enum FileLine { Header(SessionHeader), Entry(SessionEntry) }   // parse helper
```

`thinkingLevel` in `thinking_level_change` stays a `String` (pi stores free strings; parse
with `ThinkingLevel::from_str` when building context, unknown → `Off`).
`context_edit.replacement` is serialised as `null` when `None` (always present).

## 3. Message conversion for the LLM (`oino-session::messages`)

`pub fn convert_to_llm(messages: &[AgentMessage]) -> Vec<Message>` — exact pi `messages.js`:

| AgentMessage | Output |
|---|---|
| system/user/assistant/toolResult | unchanged |
| `bashExecution` with `excludeFromContext == Some(true)` | skipped |
| `bashExecution` | `User{content: [text]}`, timestamp kept, where text (pi `bashExecutionToText`) = ``"Ran `{command}`\n"`` + (output non-empty ? ``"```\n{output}\n```"`` : `"(no output)"`) + (`cancelled` ? `"\n\n(command cancelled)"` : `exitCode` is `Some(n)` with n ≠ 0 ? `"\n\nCommand exited with code {n}"` : nothing) + (`truncated && full_output_path.is_some()` ? `"\n\n[Output truncated. Full output: {path}]"` : nothing) |
| `custom` | `User{content}` (string content → one text block), timestamp kept |
| `branchSummary` | `User` text `"The following is a summary of a branch that this conversation came back from:\n\n<summary>\n" + summary + "</summary>"` |
| `compactionSummary` | `User` text `"The conversation history before this point was compacted into the following summary:\n\n<summary>\n" + summary + "\n</summary>"` |

Constants `BRANCH_SUMMARY_PREFIX`, `BRANCH_SUMMARY_SUFFIX`, `COMPACTION_SUMMARY_PREFIX`,
`COMPACTION_SUMMARY_SUFFIX` are public.

## 4. Context building and projection (`oino-session::context`, pure functions)

```rust
pub struct ProjectedEntry { pub source_entry: SessionEntry, pub messages: Vec<AgentMessage> }
pub struct SessionProjection { pub entries: Vec<ProjectedEntry>, pub messages: Vec<AgentMessage>,
                               pub thinking_level: String, pub model: Option<ModelRef> }
pub struct ModelRef { pub provider: String, pub model_id: String }
pub struct SessionContext { pub messages: Vec<AgentMessage>, pub thinking_level: String /* default "off" */, pub model: Option<ModelRef> }

pub fn path_to(entries: &EntryIndex, leaf: Option<&EntryId>) -> Vec<&SessionEntry>;      // root → leaf
pub fn build_context_entries(entries: &EntryIndex, leaf: Option<&EntryId>) -> Vec<SessionEntry>;
pub fn build_session_projection(entries: &EntryIndex, leaf: Option<&EntryId>) -> SessionProjection;
pub fn build_session_context(entries: &EntryIndex, leaf: Option<&EntryId>) -> SessionContext;
pub fn entry_to_context_messages(entry: &SessionEntry, compaction_system: Option<&SystemMessage>) -> Vec<AgentMessage>;
pub fn latest_compaction(path: &[&SessionEntry]) -> Option<&SessionEntry>;
pub fn branch_selection(path: &[&SessionEntry]) -> BranchSelection; // latest model_change + thinking change on path
pub struct BranchSelection { pub model: Option<ModelRef>, pub model_entry_id: Option<EntryId>, pub thinking_level: Option<String> }
```

Rules (pi `session-format.md` "Context Building", audit-pi §6.4), applied in order:

1. **Path.** Walk `leaf → root` via `parentId`, reverse to root→leaf. Missing parent ⇒ stop
   (orphan becomes root).
2. **Compaction selection** (`build_context_entries`). If the path contains ≥1 `compaction`,
   take the latest one `C`: output `C`, then the entries strictly between
   `C.firstKeptEntryId` (inclusive) and `C` (exclusive) **excluding entries whose message is
   `role:"system"`**, then all entries after `C`. If `firstKeptEntryId == C.id` nothing before
   `C` is kept. If `firstKeptEntryId` is not on the path, keep nothing before `C`. Non-message
   entries stay in the selection.
3. **Context edits** (`build_session_projection`). Collect `context_edit` entries **on the
   path** (branch-relative). For each target id the **latest** edit on the path wins.
   `replacement: null` ⇒ the target projects to zero messages. Otherwise the target message's
   content is replaced, role/metadata kept. A string replacement on an assistant or toolResult
   target becomes `[TextContent{text}]`; on user/custom it stays a string. Valid targets:
   `message` entries with roles user/assistant/toolResult, and `custom_message` entries; edits to
   other targets are ignored.
4. **Entry → messages** (`entry_to_context_messages`):
   - `message` → its message;
   - `compaction` → first the **system checkpoint**: `C.systemMessage` if present, else the
     fold of all system messages on the path before `firstKeptEntryId` into one
     `SystemMessage` (replay of content/sections/tools via `oino_ai::transcript`), omitted if no
     system message existed; then `CompactionSummary{summary, tokensBefore, timestamp:
     C.timestamp as ms}`;
   - `branch_summary` → `BranchSummary{summary, fromId: Some(from_id), timestamp}`;
   - `custom_message` → `Custom{customType, content, display, details, timestamp}`;
   - `thinking_level_change`, `model_change`, `usage`, `custom`, `label`, `session_info`,
     `context_edit` → nothing.
5. **Model/thinking** come from the **full path** (not the compacted selection):
   latest `model_change` → `ModelRef`; latest `thinking_level_change` → string, default
   `"off"`. Virtual-model aware selection is resolved by core (runtime §13) using
   `branch_selection` plus assistant `provider/model` fields.

`build_session_context` = projection's `messages`, `thinking_level`, `model`.

## 5. Migration (`oino-session::migrate`)

`pub fn detect(first_line: &str) -> FileFormat` with
`FileFormat::{PiV1, PiV2, PiV3, PiFuture(u32), OinoLegacy{version: 1|2}, Empty, Unknown}`:
a first line with key `"record"` is Oino legacy; `"type":"session"` is pi (missing `version` ⇒ 1).
`PiFuture` (>3) ⇒ `SessionError::UnsupportedVersion` (never rewrite).

### 5.1 Pi v1/v2 → v3

Exactly pi `migrateToCurrentVersion` (see pi source quoted in audit): v1 → assign
`generate_id` 8-hex ids (collision-checked, 100 tries then UUID) in file order, `parentId` =
previous entry; compaction `firstKeptEntryIndex` → `firstKeptEntryId`; v2 → message role
`hookMessage` → `custom`; header `version = 3`. The file is **rewritten in place** atomically
(write `<file>.tmp`, fsync, rename). Malformed lines are skipped (pi `parseSessionEntries`).

### 5.2 Oino legacy v1/v2 → v3

Legacy line shapes: `{"record":"header","header":{version,session_id,name,cwd},"leaf_id"}` and
`{"record":"entry","entry":{id,parent,kind:{kind:…}}}` (see `legacy/crates/oino-session`).
Procedure (`migrate_oino_legacy(path, sessions_root) -> Result<MigratedSession>`):

1. Parse all records; unknown/malformed entry lines are dropped with a `MigrationWarning`.
2. Ids: every legacy entry UUID gets a fresh `EntryId` (8-hex). Keep `HashMap<Uuid, EntryId>`
   for `parent`, `first_kept`, `target`, `leaf_id`. Also map legacy **message ids** (the `id`
   inside each message) to the containing entry id, used for `ContextEdit.target`.
3. Timestamps: `base = file mtime`; entry *i* of *N* (file order) gets
   `base − (N − 1 − i)` ms, both as entry `timestamp` (ISO) and message `timestamp` (ms).
4. Header: `{type:"session", version:3, id: session_id.to_string(), timestamp: first entry
   timestamp (or base), cwd}`.
5. Entry mapping:

| Legacy `kind` | v3 |
|---|---|
| `message{message}` | `message` with message mapped per table below |
| `model_change{model{provider,name}}` | `model_change{provider, modelId: name}` |
| `thinking_level_change{thinking_level}` | `thinking_level_change{thinkingLevel}` (`"x_high"` → `"xhigh"`) |
| `compaction{summary, first_kept, tokens_before, details, replaces}` | `compaction{summary, firstKeptEntryId, tokensBefore, details}`; `firstKeptEntryId` = mapped `first_kept`; if absent (v1): the first entry on the compaction's path after the last id in `replaces`; if none, the compaction's own id |
| `context_edit{target, replacement}` | `context_edit{targetId: message-id map, else entry-id map, replacement: {content: mapped blocks} \| null}`; unresolved target ⇒ dropped |
| `usage{usage_kind, model, usage}` | `usage{kind: usage_kind, provider: model.provider, model: model.name, usage}` |
| `branch_summary{summary}` | `branch_summary{fromId: mapped parent id (own id if root), summary}` |
| `custom{name, payload}` | `custom{customType: name, data: payload}` |
| `extension_custom{entry}` | `custom{customType: "oino.legacy/" + owner_extension_id + "/" + key, data: payload}` |
| `custom_message{message: Custom{name,payload,model_visible}}` | `model_visible` ⇒ `custom_message{customType: name, content: payload as string (JSON-stringified if not a string), display: true, details: payload if not a string}`; else `custom{customType: name, data: payload}` |
| `label{label}` | `label{targetId: mapped parent, label}` |
| `session_info{name, cwd}` | `session_info{name}` |
| `leaf_move{leaf_id}` | dropped (updates the running leaf only) |

Legacy message mapping:

| Legacy `Message` | v3 `AgentMessage` |
|---|---|
| `System{content, sections[{name,text?}], tools_added[{name,description,input_schema}], tools_removed[name]}` | `System{content, sections: ordered map name→text\|null (omit if empty), toolsAdded: [{name, description, parameters: input_schema}] (omit if empty), toolsRemoved: [{name}] (omit if empty), timestamp}` |
| `User{content}` | `User{content: Blocks(mapped)}` |
| `Assistant{content, stop_reason, usage, provider{request_id, model{provider,name}}, response_model, error_message, provider_thinking_level}` | `Assistant{content, api: "oino-legacy", provider: model.provider or "unknown", model: model.name or "unknown", responseModel, responseId: request_id, providerThinkingLevel, usage: mapped or zero usage, stopReason: end_turn→stop, tool_use→toolUse, length→length, error→error, aborted→aborted, unknown/None→stop with rawStopReason "unknown", errorMessage, timestamp}` |
| `ToolResult{tool_call_id, tool_name, content, is_error, details, usage}` | `ToolResult{toolCallId: id of the mapped ToolCall (see blocks), toolName, content, details, usage, isError, timestamp}`; `terminate`, `added_tool_names` dropped |
| `Custom{…}` inside a `message` entry | as `custom_message` rule above, but as an `AgentMessage::Custom` when model-visible, else entry becomes `custom` |
| `CompactionSummary`/`BranchSummary` inside a `message` entry | dropped with warning |

Blocks: `Text{text,signature}` → `text{text, textSignature}`; `Thinking{text, redacted,
signature, provider_data}` → `thinking{thinking: text, thinkingSignature: signature, or
provider_data if it is a JSON string; redacted: Some(true) only when true}`; `Image{media_type,
data}` → `image{data, mimeType}`; `ToolCall{id, name, arguments, native_id}` →
`toolCall{id: native_id or id.to_string(), name, arguments (non-object ⇒ {} + argumentsError
"legacy arguments were not an object")}` and record `legacy uuid → pi id` for tool results.
Usage: `input_tokens→input, output_tokens→output, cache_read_tokens→cacheRead,
cache_write_tokens→cacheWrite, cache_write_1h_tokens→cacheWrite1h, reasoning_tokens→reasoning
(Some), totalTokens = input+output+cacheRead+cacheWrite, cost: all 0 except total = amount when
currency == "USD"`.

6. **Leaf restoration.** Compute the effective legacy leaf (header `leaf_id`, else last
   `leaf_move`, else last entry). Write entries in legacy file order, then **append one
   `session_info` entry** with `parentId = effective leaf` and `name = header.name` when
   non-empty (else `name` omitted). On load the last entry is the leaf, so the leaf is
   preserved and the session name survives. Sessions with no `message` entries are not
   migrated (moved to `.legacy/` only).
7. Write to `<sessions_root>/--<cwd>--/<ts>_<session_id>.jsonl` (`ts` from the header timestamp
   per §6.1), then move the source to `<sessions_root>/.legacy/<original name>`. Return
   `MigratedSession{path, warnings}`. Never overwrite an existing target (append `-1`, `-2`…
   to the timestamp part).

`SessionManager::migrate_legacy_dir(sessions_root)` migrates every `*.jsonl` directly in
`sessions_root` (flat legacy layout) and is called by `Services` startup once per process.

## 6. `SessionManager` (`oino-session::manager`, feature `manager`)

### 6.1 Files

- Directory for a cwd: `<session_dir>/--<cwd>--/` where `<cwd>` is the resolved absolute cwd
  with one leading `/` or `\` removed and every `/`, `\`, `:` replaced by `-`.
- File name: `<ts>_<sessionId>.jsonl`, `ts` = header ISO timestamp with `:` and `.` replaced by
  `-` (e.g. `2026-10-26T03-00-07-623Z_<uuid>.jsonl`).
- Session dir precedence (resolved by core, passed in): `--session-dir` >
  `OINO_SESSION_DIR` > setting `sessionDir` (relative to cwd) > `<agent_dir>/sessions`.
- Session id validity: `^[A-Za-z0-9](?:[A-Za-z0-9._-]*[A-Za-z0-9])?$`, error text exactly
  pi's: `Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character`.
- **Persistence**: append-only, one JSON object per line, `\n` terminated, `O_APPEND`, flush
  after each line. The file is created only when the first `message` entry whose message role
  is `user` or `assistant` is appended; at that moment the header and all buffered entries are
  written in order. Before that, `is_persisted() == false` and entries live in memory only.
- Leaf on load = last entry in file order.

### 6.2 API

```rust
pub struct CreateOptions { pub id: Option<String>, pub parent_session: Option<PathBuf> }
pub struct SessionInfo { pub path: PathBuf, pub id: String, pub cwd: String, pub name: Option<String>,
    pub parent_session_path: Option<PathBuf>, pub created: IsoTimestamp, pub modified: SystemTime,
    pub message_count: usize, pub first_message: String, pub all_messages_text: String }
pub struct SessionTreeNode { pub entry: SessionEntry, pub children: Vec<SessionTreeNode>,
    pub label: Option<String>, pub label_timestamp: Option<IsoTimestamp> }
pub trait ProgressSink { fn progress(&self, done: usize, total: usize); }

impl SessionManager {
    // construction
    pub fn create(cwd: &Path, session_dir: &Path, opts: CreateOptions) -> Result<Self, SessionError>;
    pub fn open(path: &Path, session_dir: &Path, cwd_override: Option<&Path>) -> Result<Self, SessionError>; // migrates pi v1/v2 and Oino legacy
    pub fn continue_recent(cwd: &Path, session_dir: &Path) -> Result<Self, SessionError>; // newest by mtime, else create
    pub fn in_memory(cwd: &Path, opts: CreateOptions, entries: Vec<SessionEntry>) -> Self;
    pub fn fork_from(source: &Path, target_cwd: &Path, session_dir: &Path, opts: CreateOptions) -> Result<Self, SessionError>;
    pub fn find_by_id(cwd: &Path, id: &str, session_dir: &Path) -> Result<Option<PathBuf>, SessionError>; // exact, then unique prefix (≥4 chars) in cwd dir, then all dirs; ambiguity ⇒ SessionError::AmbiguousId
    pub fn list(cwd: &Path, session_dir: &Path, progress: Option<&dyn ProgressSink>, cancel: Option<&AtomicBool>) -> Result<Vec<SessionInfo>, SessionError>; // newest modified first
    pub fn list_all(session_dir: &Path, progress: Option<&dyn ProgressSink>, cancel: Option<&AtomicBool>) -> Result<Vec<SessionInfo>, SessionError>;
    pub fn find_most_recent(cwd: &Path, session_dir: &Path) -> Result<Option<PathBuf>, SessionError>;
    pub fn migrate_legacy_dir(sessions_root: &Path) -> Result<Vec<MigratedSession>, SessionError>;

    // identity
    pub fn new_session(&mut self, opts: CreateOptions) -> Result<(), SessionError>; // fresh header, empty entries
    pub fn set_session_file(&mut self, path: &Path) -> Result<(), SessionError>;
    pub fn is_persisted(&self) -> bool;
    pub fn cwd(&self) -> &Path;  pub fn session_dir(&self) -> &Path;  pub fn uses_default_session_dir(&self) -> bool;
    pub fn session_id(&self) -> &str;  pub fn session_file(&self) -> Option<&Path>;  pub fn header(&self) -> &SessionHeader;
    pub fn entry_count(&self) -> usize;

    // append (each returns the new entry id; parent = current leaf; leaf moves to new entry)
    pub fn append_message(&mut self, message: AgentMessage) -> Result<EntryId, SessionError>; // rejects BranchSummary/CompactionSummary roles
    pub fn append_thinking_level_change(&mut self, level: &str) -> Result<EntryId, SessionError>;
    pub fn append_model_change(&mut self, provider: &str, model_id: &str) -> Result<EntryId, SessionError>;
    pub fn append_usage(&mut self, kind: &str, provider: &str, model: &str, usage: Usage, note: Option<String>) -> Result<EntryId, SessionError>;
    pub fn append_compaction(&mut self, c: CompactionDraft) -> Result<EntryId, SessionError>; // first_kept None ⇒ self id
    pub fn append_custom_entry(&mut self, custom_type: &str, data: Option<serde_json::Value>) -> Result<EntryId, SessionError>;
    pub fn append_session_info(&mut self, name: Option<&str>) -> Result<EntryId, SessionError>; // name: CR/LF runs → " ", trimmed
    pub fn append_custom_message_entry(&mut self, custom_type: &str, content: UserContentField, display: bool, details: Option<serde_json::Value>) -> Result<EntryId, SessionError>;
    pub fn append_context_edit(&mut self, target: &EntryId, replacement: Option<EditableContent>) -> Result<EntryId, SessionError>;
    pub fn append_label_change(&mut self, target: &EntryId, label: Option<&str>) -> Result<EntryId, SessionError>; // empty ⇒ clear
    pub fn append_entry(&mut self, kind: EntryKind) -> Result<EntryId, SessionError>; // generic (used by boundary drafts)

    // reads
    pub fn session_name(&self) -> Option<String>;           // latest session_info in FILE order, trimmed, empty ⇒ None
    pub fn leaf_id(&self) -> Option<&EntryId>;  pub fn leaf_entry(&self) -> Option<&SessionEntry>;
    pub fn entry(&self, id: &EntryId) -> Option<&SessionEntry>;
    pub fn children(&self, parent: Option<&EntryId>) -> Vec<&SessionEntry>;   // file order
    pub fn label(&self, id: &EntryId) -> Option<&str>;     // latest label entry for id in file order
    pub fn branch(&self, from: Option<&EntryId>) -> Vec<&SessionEntry>;      // root → from (default leaf), all types
    pub fn entries(&self) -> &[SessionEntry];               // file order, excluding header
    pub fn tree(&self) -> Vec<SessionTreeNode>;             // roots in file order; orphans are roots; usage entries included (frontends hide them)
    pub fn build_context_entries(&self) -> Vec<SessionEntry>;
    pub fn build_session_projection(&self) -> SessionProjection;
    pub fn build_session_context(&self) -> SessionContext;

    // leaf movement
    pub fn set_leaf(&mut self, id: &EntryId) -> Result<(), SessionError>;   // pi `branch(fromId)`; no write
    pub fn reset_leaf(&mut self);                                           // leaf = None; next entry is a root
    pub fn branch_with_summary(&mut self, from: Option<&EntryId>, summary: BranchSummaryDraft) -> Result<EntryId, SessionError>;
    pub fn create_branched_session(&self, leaf: &EntryId) -> Result<PathBuf, SessionError>; // new file: path root→leaf, new id, header.parentSession = this file
}

pub struct CompactionDraft { pub summary: String, pub first_kept_entry_id: Option<EntryId>, pub tokens_before: u64,
    pub details: Option<serde_json::Value>, pub from_hook: Option<bool>, pub usage: Option<Usage>, pub system_message: Option<SystemMessage> }
pub struct BranchSummaryDraft { pub from_id: EntryId, pub summary: String, pub details: Option<serde_json::Value>,
    pub from_hook: Option<bool>, pub usage: Option<Usage> }
```

`ReadonlySession` is a trait implemented by `SessionManager` exposing only the read methods;
core hands extensions a snapshot implementing it (extension-api §5.2).

### 6.3 Errors

```rust
#[non_exhaustive]
pub enum SessionError {
    Io { path: PathBuf, source: std::io::Error },
    Json { path: PathBuf, line: usize, source: serde_json::Error },
    InvalidSessionId(String), UnknownEntry(EntryId), AmbiguousId(Vec<PathBuf>),
    UnsupportedVersion(u32), MissingHeader(PathBuf), MissingCwd(PathBuf),
    InvalidAppend(&'static str),           // e.g. summary roles via append_message
    NotFound(PathBuf),
}
```

## 7. Acceptance (core-types-session)

1. Round-trip: every example line in pi `docs/session-format.md` parses and re-serialises to an
   equal JSON value.
2. Pi fixture corpus (quality §4.3): for each file, `build_session_context` equals pi's
   `buildSessionContext` output produced by the pi oracle script.
3. Legacy corpus: Oino v1/v2 fixtures (generated from `legacy/` test data) migrate; leaf, name,
   tool-call linkage and context-edit targets preserved; golden v3 output committed.
4. Persistence: setup entries before the first user/assistant message are not on disk; after
   it, file = header + buffered entries; subsequent appends are single-line writes.
5. `oino-ai` and `oino-session --no-default-features` build for `wasm32-wasip2`.

---

## Appendix: settled deviations (folded at 1.0.0-alpha.2)

From `deviations/core-types-session.md`. Where the spec text and pi 1.4.0 disagreed, pi won when §7.2 requires equality with pi's `buildSessionContext`. Every row is covered by a test.

| # | Section | Change | Reason | Impact |
|---|---|---|---|---|
| 1 | session-v3 §4 rule 5 | `SessionContext.model` / `SessionProjection.model` are also updated by assistant messages on the path (`provider`/`model` of the physical model that answered), not only by `model_change`. `branch_selection` still reports only explicit `model_change` / `thinking_level_change`. | pi `getSessionContextSettings` does this; golden equality (§7.2) needs it. | core-runtime: virtual-model resolution uses `branch_selection` as specced. |
| 2 | session-v3 §4 rule 4 | A `compaction` without `systemMessage` projects **only** the `compactionSummary` (pi `sessionEntryToContextMessages`). The fold fallback is available through `entry_to_context_messages(entry, Some(fallback))`; projection passes `None`. Legacy migration instead **stores** the folded checkpoint in `systemMessage`, so migrated sessions keep their prompt. `append_compaction` without a draft checkpoint folds the current projection like pi `appendCompaction`. | pi parity of old pi sessions; spec intent kept for migrated Oino sessions. | none for new sessions. |
| 3 | session-v3 §4 rule 3 | Context edits are collected from the compaction-selected entries (pi), and `replacement: null` omits *any* targeted selected entry (pi `projectContextEntry`). Mismatched block kinds (images on assistants, thinking/tool calls on users) are dropped when a replacement is applied. | pi parity. `append_context_edit` still only accepts user/assistant/toolResult/custom_message targets on the active branch. | none. |
| 4 | session-v3 §6.1/§6.2 | `is_persisted()` keeps pi semantics (file-backed vs in-memory). New `is_flushed()` reports whether the file has been created. | Core ports pi code that uses `isPersisted()` to mean "not in-memory". | core-runtime: use `is_flushed()` for the "file exists yet" question. |
| 5 | session-v3 §6.2 | Every `session_dir` argument is the **sessions root**; files always go to `<root>/--<cwd>--/`. `session_dir()` returns the root, `uses_default_session_dir()` compares it with `$OINO_AGENT_DIR` or `~/.oino` + `/sessions`. Listing and `find_by_id` do not filter by header cwd (pi only does that for flat custom dirs). | §6.1 defines the per-cwd layout under the resolved dir; pi's flat custom-dir mode does not exist in Oino. | core-runtime / modes: pass the resolved root (`--session-dir` > `OINO_SESSION_DIR` > setting > `<agent>/sessions`). |
| 6 | session-v3 §6.2 | `create_branched_session(&self, leaf)` writes the new file immediately (even without a user/assistant message) and fails with `InvalidAppend` for in-memory sessions. Added `branch_into_new_session(&mut self, leaf) -> Option<PathBuf>`, which follows pi `createBranchedSession` exactly (replaces `self`, writes only once a conversation exists, works in memory). | `&self` cannot switch the manager; the caller reopens the returned path. | core-runtime `/clone`, `/fork`: either method. |
| 7 | session-v3 §6.3 | `SessionError::Cancelled` added (listing with the cancel flag). Error is `#[non_exhaustive]`. | Needed to report cancellation. | additive. |
| 8 | session-v3 §5.2 step 7 | `MigratedSession { path: Option<PathBuf>, legacy_path, warnings }`: `path` is `None` when the session had no messages and was only moved aside. | Spec says such sessions are not migrated. | additive. |
| 9 | session-v3 §5.2 steps 3, 6 | Legacy entries are written **parents first** (pre-order, siblings in file order) before timestamps are assigned. Legacy writers dumped a `BTreeMap` keyed by UUID, so "file order" was random. Labels and branch-summary `fromId` resolve through dropped ancestors (for example `leaf_move` or an unresolved `context_edit`). A `label` on a root entry is dropped with a warning. | Keeps timestamps causal and the tree order sensible. | none. |
| 10 | session-v3 §6.2 | `append_session_info(None)` leaves out `name`. `append_label_change` with an empty label writes the label entry without a `label` field (pi writes `""`); both clear the label. | Option-typed API. | none. |
| 11 | session-v3 §1.2 | `AgentMessage::as_llm(&self) -> Option<Message>` returns an owned value, since `AgentMessage` holds no `Message` to borrow. | Not representable as `Option<&Message>`. | callers clone implicitly. |
| 12 | session-v3 §1.2 | `ConstrainedSamplingConfig` gained `variants: Option<IndexMap<String,String>>` (pi `{type:"grammar", variants}`). | pi 1.4.0 has two config shapes. | additive. |
| 13 | session-v3 §2 | `SessionHeader` is serialised as `{"type":"session",…}` via `#[serde(tag)]`. `CompactionEntry.system_message` is serialised with `"role":"system"` and read with or without it. Optional `details`/`data` keep an explicit JSON `null` (`oino_ai::json::keep_null`). | Byte-level pi shapes; the round-trip test covers them. | none. |
| 14 | session-v3 §1.1 | On read, missing message `content` defaults to `""` (system) or `[]` (others), as pi's projection does for `null`. `Usage.cost` defaults to zeros when missing. | Lenient reading of older files. | none. |
| 15 | session-v3 §4 | Free functions treat `leaf: None` as pi's `null` leaf (empty path). An unknown id falls back to the last entry. Use `EntryIndex::last_id()` for pi's default leaf. | `Option` cannot express pi's `undefined` vs `null`. | none. |

**Additive API (no spec change):**

- `oino_ai::transform::transform_messages` (pi `transform-messages.js`), `split_error_tool_result_images`.
- `oino_ai::transcript::{SystemState, resolve_transcript_tools, tool_state_changes, render_system_message_update, initial_system_message, collapse_system_messages, …}`.
- `oino_ai::json::{json_semantic_eq, keep_null}`, `Usage::accumulate`, `AssistantMessage::empty`, `AnyModel`.
- `oino_session::time` (ISO formatting/parsing like `Date#toISOString`), `manager::{encode_cwd, cwd_session_dir, session_file_name, resolve_path, validate_session_id, default_agent_dir, default_sessions_root, migrate_legacy_file, session_info}`, `migrate::{parse_session_text, migrate_pi_values, legacy::migrate_oino_legacy_text}`, `SessionManager::index()`.
