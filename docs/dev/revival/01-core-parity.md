# P0 + P1: Core Parity

Goal: bring `oino-types`, `oino-agent-loop`, `oino-agent`, `oino-session`,
`oino-harness` and `oino-extension-core` to pi 0.87 semantics, with prefix stability as
the main outcome. No WASM work here; P1 must leave the TUI and the current provider
working.

Reference sources (pi at `b455975`):
`packages/ai/src/types.ts`, `packages/ai/src/utils/transcript.ts`,
`packages/agent/src/types.ts`, `packages/agent/src/agent-loop.ts`,
`packages/coding-agent/docs/session-format.md`, `packages/coding-agent/docs/extensions.md`.

## P0: hygiene

1. Fix the 5 clippy lints in `crates/oino-auth/src/lib.rs` (:756, :1271, :1698, :1717,
   :1730). For `unwrap_used` in tests, match the existing test style
   (`unwrap_or_else(|err| panic!(...))`).
2. Add a "Revival specs" pointer to `AGENT.md` linking this folder.

Done when fmt, clippy and tests are all green.

## P1.1 Types (`oino-types`)

### Problem

Oino cannot express a prompt or tool change as an append. Example: `/reload` calls
`harness.set_system_prompt(default_system_prompt(..))` (`oino-app/src/main.rs:4591`,
`:6818`). If `AGENT.md` changed by one line, the next request's first bytes differ and
every cached token after them is billed as fresh. On resume, the prompt is rebuilt from
disk because it is not persisted, with the same effect.

### Changes

```rust
pub enum Message {
    System {
        id: OinoId,
        /// Leading message: base prompt. Later messages: appended instructions.
        content: String,
        /// Ordered named sections. Leading message declares; later messages replace by
        /// name; `text: None` removes.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        sections: Vec<PromptSection>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tools_added: Vec<ToolDeclaration>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        tools_removed: Vec<String>,
    },
    // existing variants unchanged except as listed below
}

pub struct PromptSection { pub name: String, pub text: Option<String> }

/// Moved from oino-agent-loop::ToolDefinition (re-exported there for compatibility).
pub struct ToolDeclaration { pub name: String, pub description: String, pub input_schema: Value }
```

Other type changes:

- `ThinkingLevel::Max` after `XHigh`.
- `Usage.reasoning_tokens: u64` (`#[serde(default)]`).
- `ContentBlock::Thinking` gains `signature: Option<String>` (Anthropic replay) and
  `provider_data: Option<Value>` (OpenAI Responses encrypted reasoning, item ids).
- `ContentBlock::ToolCall` gains `native_id: Option<String>`. Oino keeps its UUID for
  internal linking; adapters must replay `native_id` verbatim when present, so request
  bytes stay identical across turns.
- `Message::ToolResult` gains `usage: Option<Usage>` and
  `added_tool_names: Vec<String>`.
- `Message::Assistant` gains `response_model: Option<String>` and
  `error_message: Option<String>`.
- `AssistantStreamEvent` gains `Start { response_id: Option<String>, response_model: Option<String> }`
  and `ThinkingSignature { signature: String }`. Keep existing variants.

### New module `oino_types::transcript`

Port of pi `utils/transcript.ts`, same semantics:

- `normalize(system_prompt: Option<String>, tools: Vec<ToolDeclaration>, messages) -> Transcript`
  (inserts the leading `System` only when prompt or tools are non-empty).
- `current_tools(&[Message]) -> Vec<ToolDeclaration>`: apply removes then adds per system
  message, in order; later adds replace by name; output order is first-declaration order.
- `current_system(&[Message]) -> Option<Message>`: content parts joined with `"\n\n"`,
  sections patched by name, tools from `current_tools`.
- `render_system_prompt(&Message) -> String`: `content`, then each section text in order,
  joined with `"\n\n"`.
- `collapse(Transcript) -> Transcript`: leading replayed system message plus all
  non-system messages.
- `resolve(Transcript, supports_mid_convo_system: bool) -> Transcript`.
- `Transcript` is a newtype over `Vec<Message>` constructed only by `normalize`/`resolve`,
  matching pi's branded `TranscriptContext`.

Tests: port the pi cases for replay order, section removal, tool re-add after removal,
empty transcript, collapse. Round-trip serde for all new variants.

## P1.2 Loop (`oino-agent-loop`, `oino-agent`)

### StreamRequest

```rust
pub struct StreamRequest {
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub transcript: Transcript,          // replaces system_prompt, messages, tools
    pub session_id: Option<String>,      // cache affinity key
    pub cache_retention: CacheRetention, // None | Short | Long, default Short
}
```

The loop never re-sends a separately held prompt. Tool execution looks tools up by name
in the registry, but only names in `current_tools(transcript)` are callable; calls to
others become error tool results.

### Hooks

Replace `should_stop_after_turn` with pi's two hooks:

```rust
pub struct TurnContext<'a> {
    pub message: &'a Message,            // assistant message that ended the turn
    pub tool_results: &'a [Message],
    pub transcript: &'a Transcript,
}
pub enum TurnDecision { Continue, End }
pub type FinishTurn = Arc<dyn Fn(TurnContext<'_>) -> BoxFuture<'static, LoopResult<Option<TurnDecision>>> + Send + Sync>;

pub struct RequestUpdate { pub transcript: Option<Transcript>, pub model: Option<Model>, pub thinking_level: Option<ThinkingLevel> }
pub type PrepareRequest = Arc<dyn Fn(PrepareRequestContext) -> BoxFuture<'static, LoopResult<Option<RequestUpdate>>> + Send + Sync>;
```

Semantics, copied from pi:

- `prepare_request` runs before every provider request including the first, after
  pending steering/follow-up messages are appended. Updates persist for the rest of the
  run. It does not poll queues.
- `finish_turn` runs after the assistant message and all tool results are finalized and
  before `TurnEnd`; its decision applies after `TurnEnd`. `End` stops without polling
  queues. `Continue` guarantees exactly one more request (satisfied by pending tool
  results/steering/follow-up if any). Error and aborted turns are hard exits; the hook
  still runs but its decision is ignored.
- `BeforeToolCallResult::Block { reason: String, terminate: bool }`. If every call in a
  batch terminates (via block `terminate` or `ToolResult.terminate`), skip the follow-up
  request.
- `Agent::peek_queued_messages()` returns the next batch without consuming it.

### Events

- `MessageUpdate { message_id, content }` resends all content each time, which is
  quadratic. Replace it with
  `MessageDelta { message_id, delta: ContentDelta }` where `ContentDelta` is
  `Text { index, text } | Thinking { index, text } | ToolCallArgs { index, id, name, json }`.
  The TUI keeps its own accumulated message (add an accumulator helper in
  `oino-agent-loop` so consumers don't hand-roll it).
- Add `AgentEvent::TurnEnd { turn, stop_reason, entries: Vec<Message> }` for actionable
  turn-end (see P1.5).

### Tool set changes mid-session

`Agent::set_tools` no longer replaces a header. It diffs declarations against
`current_tools(transcript)` and appends one `System { tools_added, tools_removed }`
message only when something changed. Declarations compare by canonical JSON (sorted
keys) so reordering map keys never counts as a change.

## P1.3 Sessions (`oino-session`)

Header `version` goes from 1 to 2.

New entry kinds:

```rust
ContextEdit { target: OinoId, replacement: Option<Vec<ContentBlock>> }
Usage { kind: String, model: Model, usage: Usage }   // e.g. "cache_warm", "compaction"
```

`Compaction` becomes
`{ summary, first_kept: OinoId, tokens_before: u64, details: Option<Value> }`.
Retain-none compaction sets `first_kept` to the compaction entry's own id.

Context building for the active branch:

1. Walk root to leaf. Start from the latest compaction on the path: emit its summary,
   then messages from `first_kept` onward.
2. `System` messages before the compaction boundary are **not** dropped: fold them into
   one replayed leading `System` (via `current_system`) so the prompt and tools survive
   compaction.
3. Apply `ContextEdit`s last-wins by `target`: `None` omits, `Some` replaces content.
   Raw history, usage and UI history are untouched.
4. `Usage` entries never enter context.

Migration of v1 files (read side only, never rewrite files):

- v1 `Compaction { replaces }` maps to `first_kept` = first non-replaced message on the
  path.
- v1 sessions have no `System` message. On resume the harness appends one (see P1.4).
  Nothing is inserted at the root.

## P1.4 Harness and app (`oino-harness`, `oino-app`)

- `default_system_prompt` already builds titled sections. Change it to return
  `(base: String, sections: Vec<PromptSection>)`, one section per resource file
  (name = resource path relative to its root, e.g. `project/AGENT.md`), plus
  `oino/resource-policy` and `oino/cwd`.
- New session: the first persisted entry is `System { content: base, sections, tools_added: all tools }`.
- `/reload`, resume, cwd change and tool/extension changes all go through one function,
  `Harness::sync_system(base, sections, tools)`. It diffs against the replayed state and
  appends a single delta `System` with only the changed sections and tool additions or
  removals, or appends nothing. This is the core prefix-stability mechanism.
- Resume of a v1 session: `sync_system` against an empty replayed state appends a full
  `System` at the current leaf.
- `inspect_full_prompt` shows the replayed prompt and lists delta messages.
- Nothing time-varying goes into system content (no dates, no clocks). Add a unit test
  that builds the prompt twice across a sleep and asserts byte equality.

## P1.5 Extension events (`oino-extension-core`, `oino-extension-runtime`)

Replace stringly `HookEventPayload::Context { messages: Vec<String> }` with typed
payloads carrying `oino_types::Message`. Add the missing pi events:

`before_agent_start`, `context_with_system`, `before_provider_headers`,
`provider_stream_event`, `turn_end` (actionable: may return
`{ entries: Vec<Message>, continue: bool }`), `agent_before_settle`, `agent_settled`,
`session_before_fork`, `session_before_switch`, `session_before_tree`,
`session_compact_failed`, `session_info_changed`, `resources_discover`,
`project_trust`, `ui_prompt_start`, `ui_prompt_end`, `cache_warming_decision`
(dispatch only; the decision logic lands in P4).

`before_agent_start` handlers change the prompt by returning section edits, never a
whole replacement string, so they flow through `sync_system`.

## P1.6 Interim provider cache hints (`oino-provider-openrouter`)

Temporary until P3 moves providers to WASM; keep the change small.

- Accept `Transcript`, and `collapse` it (the current adapter has no mid-convo system
  support).
- Send `prompt_cache_key = clamp64(session_id)` and, behind a `compat.session_affinity`
  flag, `x-session-affinity` / `session_id` headers.
- For `anthropic/*` models via OpenRouter, add `cache_control: {type: "ephemeral"}` on
  the last system block, the last tool, and the last user/assistant content block (pi
  `openai-completions.ts` `cacheControlFormat`).
- Serialize tools in `current_tools` order and JSON with stable key order.

## Acceptance

- Unit tests for transcript replay, loop hook semantics (port the pi `finishTurn` /
  `prepareRequest` cases using `FauxStream`), block-terminate, context edits, v1
  session load, `sync_system` diffing (no-op when unchanged; single section change
  gives a one-section delta).
- **Prefix stability test**: drive a 5-turn faux session with a `/reload` after turn 3
  that changes one resource. Capture the serialized provider request bodies and assert
  that request N+1 starts with request N's bytes up to its last message, except where the
  provider collapses system messages. For collapsing providers, assert the collapsed
  prompt is byte-identical when resources are unchanged.
- The TUI renders `System` delta messages as a collapsed one-line notice
  ("prompt updated: project/AGENT.md").
- fmt, clippy, tests green; `docs/architecture.md` and the sessions guide updated.
