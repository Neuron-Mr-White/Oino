---
title: "Oino Core Runtime"
type: brainstorm
date: 2026-05-15
---

# Oino Core Runtime

## Problem Statement

Build Oino's Rust core as a hook-first, UI-independent agent runtime inspired by Pi core, so the future Ratatui interface, subagents, memory, permissions, workflow modes, and provider routing can plug in without rewriting the agent loop.

The immediate goal is not to build the TUI or real provider integrations. The goal is to define and then implement a stable core that can stream model output, execute tools, persist conversation state, expose typed events, and allow behavior-changing hooks at safe extension points.

## Context

We studied Pi's core packages at `/tmp/pi-main` commit `b7ea821`:

- `packages/agent/src/agent-loop.ts` — low-level async loop, stream handling, tool execution, queue drains.
- `packages/agent/src/agent.ts` — stateful wrapper around the loop with subscribers, cancellation, and prompt/continue APIs.
- `packages/agent/src/harness/agent-harness.ts` — higher-level runtime with sessions, resources, auth hooks, provider hooks, and pending session writes.
- `packages/agent/src/harness/session/session.ts` — append-only session tree with movable leaf and context reconstruction.
- `packages/agent/src/harness/env/nodejs.ts` — execution environment abstraction with result-style errors.

Key lessons from Pi:

1. Keep the low-level loop independent from UI, sessions, and provider-specific code.
2. Emit rich lifecycle events so UI, persistence, tracing, and memory can observe without becoming part of the loop.
3. Separate notification events from mutating hooks.
4. Support steering and follow-up queues as first-class runtime behavior.
5. Execute tool calls in parallel by default, but preserve assistant source order for final tool-result messages.
6. Model sessions as append-only trees rather than mutable linear transcripts.
7. Provide a higher-level harness that binds loop, session, model, tools, resources, auth, and hooks together.

## Chosen Approach

Use a layered Rust design: **core loop + agent wrapper + session + harness**.

The design has four main layers:

1. `oino-agent-loop` — pure async agent loop and event protocol.
2. `oino-agent` — stateful runtime wrapper around the loop.
3. `oino-session` — append-only tree session model and storage.
4. `oino-harness` — high-level orchestration layer with hooks, resources, auth, execution env, and future memory/subagent integration points.

This is a "Layer core + harness" approach: first make the loop small and testable, then build the harness around it so product features do not leak into the loop.

## Why This Approach

### Alternative 1: Minimal agent loop only

This would be fastest to implement, but it would postpone important decisions around sessions, hooks, and resource injection. Since Oino is explicitly intended to grow into memory, subagents, and a rich TUI, postponing those boundaries would likely cause rewrites.

### Alternative 2: Harness-first

This would define the high-level product runtime immediately, but it risks over-abstracting before the core loop behavior is proven. It also makes testing harder because too many concepts exist before the stream/tool/event protocol is stable.

### Alternative 3: Plugin-first

This is the most future-flexible option, but it is premature. A plugin ABI or dynamic extension system should come after the core event and hook semantics are stable.

### Decision

Use the layered core + harness approach. It gives us a small testable loop, while still designing enough higher-level structure for hooks, subagents, memory, and session persistence.

## Design

## Architecture

```text
oino-core/
  oino-agent-loop     # pure loop, stream protocol, tool execution
  oino-agent          # stateful runtime wrapper, queues, subscribers
  oino-session        # append-only tree sessions, in-memory + JSONL storage
  oino-harness        # hooks, resources, env, auth/provider boundaries
  oino-types          # shared message/model/tool/event types if useful
```

This may be one crate with modules at first, or multiple crates later. The first implementation should optimize for clarity and tests, not package granularity. Module boundaries should be strong enough that splitting into crates later is easy.

## Core Data Model

The core types should be Rust-native equivalents of Pi's model:

- `Model` — provider/model metadata needed by the loop.
- `Message` — user, assistant, and tool-result messages.
- `ContentBlock` — text, image, thinking, and tool-call blocks.
- `Usage` — token and cost accounting.
- `StopReason` — stop, length, tool-use, error, aborted.
- `ToolDefinition` / `Tool` — schema, metadata, and async execute function.
- `ToolResult` — content, details, error status, optional termination hint.
- `AgentEvent` — lifecycle and streaming events.
- `AssistantStreamEvent` — provider stream protocol consumed by the loop.

Tool arguments and details should use `serde_json::Value` initially. Tool schemas can use `schemars` or a JSON Schema representation, but schema validation should be isolated so it can evolve.

## Layer 1: Agent Loop

The low-level loop should be a pure async function that receives:

- initial prompt messages or an existing context for continuation,
- current system prompt,
- active model,
- active tools,
- stream function,
- hook callbacks needed by the loop,
- queue drain callbacks,
- cancellation token,
- event sink.

Responsibilities:

1. Emit `AgentStart` and `TurnStart`.
2. Emit message lifecycle events for prompt messages.
3. Build provider context via `transform_context` and `convert_to_llm`.
4. Consume assistant stream events and emit message updates.
5. Execute requested tool calls.
6. Emit tool execution lifecycle events.
7. Append tool-result messages in assistant source order.
8. Drain steering messages after a turn.
9. Drain follow-up messages when the agent would otherwise stop.
10. Stop on error, abort, explicit `should_stop_after_turn`, or terminating tool batch.

Parallel tool execution should match Pi's important behavior:

- preflight/prepare tool calls in assistant source order,
- execute allowed tools concurrently,
- emit `ToolExecutionEnd` as tools complete,
- emit final tool-result message events later in assistant source order.

If any tool in a batch requires sequential mode, the whole batch should execute sequentially.

## Layer 2: Agent Wrapper

The `Agent` wrapper owns mutable runtime state:

- transcript,
- active model,
- active tools,
- system prompt,
- thinking level,
- active run state,
- cancellation token,
- steering queue,
- follow-up queue,
- subscribers.

Public behavior:

- `prompt(...)`
- `continue_from_current_context(...)`
- `steer(...)`
- `follow_up(...)`
- `abort(...)`
- `wait_for_idle(...)`
- state getters/setters for model/tools/system prompt.

Important semantic: `agent_end` is the last loop event, but the agent is not idle until awaited subscribers finish. This prevents persistence/UI races.

## Layer 3: Session

Sessions should be append-only trees.

Entry types:

- session metadata/header,
- message,
- model change,
- thinking-level change,
- compaction summary,
- branch summary,
- custom entry,
- custom message,
- label,
- session info/name.

The session has a movable `leaf_id`. Building context walks path-to-root from leaf and converts entries into model-visible messages.

Compaction semantics:

- A compaction entry replaces older context with a summary.
- Messages from `first_kept_entry_id` onward remain visible.
- Later entries after compaction remain visible.

Branch navigation semantics:

- Moving to a previous user message restores that user text for editing by the UI/harness.
- Moving to non-user entries places leaf at that entry.
- Optional branch summaries can be attached when leaving a branch.

Initial storage implementations:

1. In-memory storage for tests.
2. JSONL storage for persistence.

## Layer 4: Harness

The harness binds the core pieces into the product runtime.

Responsibilities:

- Load/build context from session.
- Build system prompt from resources and active tools.
- Own active model, thinking level, stream options, and active tool set.
- Provide high-level APIs: `prompt`, `skill`, `prompt_template`, `compact`, `navigate_tree`, `set_model`, `set_tools`, `set_resources`.
- Persist messages and runtime changes at safe save points.
- Expose hook registry.
- Provide execution environment to tools.

The harness should be where future memory, subagents, permissions, and workflows attach.

## Hook Design

Hooks are central to Oino.

There are two classes of extension points:

### Notification Events

These are observable and cannot modify behavior:

- `agent_start`
- `turn_start`
- `message_start`
- `message_update`
- `message_end`
- `tool_execution_start`
- `tool_execution_update`
- `tool_execution_end`
- `turn_end`
- `agent_end`
- `queue_update`
- `save_point`
- `settled`

Use cases: TUI, logs, tracing, memory indexing, analytics, persistence.

### Mutating Hooks

These can modify or block behavior through explicit typed return values:

- `before_agent_start` — inject messages or alter system prompt.
- `context` — prune, transform, or augment context.
- `before_provider_request` — patch stream/provider options.
- `before_provider_payload` — inspect or replace serialized payload.
- `after_provider_response` — observe provider response metadata.
- `before_tool_call` — block or rewrite tool calls.
- `after_tool_call` — patch tool result content/details/error/terminate.
- `before_compaction` — cancel or provide custom summary.
- `before_tree_navigation` — cancel or provide branch summary.

Design rules:

1. Hook order is deterministic.
2. Hook results are strongly typed.
3. Notification handlers cannot mutate loop state directly.
4. Mutating hooks modify only their explicit return surface.
5. Hooks receive cancellation where relevant.
6. UI is not available in the low-level loop.
7. Harness-level hooks may later coordinate with UI, subagents, memory, or permission systems.

## Execution Environment

Tools should not depend directly on local filesystem/process APIs. Define an `ExecutionEnv` trait with result-style errors.

Capabilities:

- execute shell commands,
- read text/binary files,
- write/append files,
- list directories,
- stat paths,
- resolve real paths,
- create/remove directories/files,
- create temp files/dirs,
- cleanup.

Local implementation comes first. Remote/sandbox/container implementations can come later without changing tool definitions.

## Memory and Subagent Future Fit

Memory can be added as hooks:

- observe `message_end` / `agent_end` for extraction,
- query memory during `context`,
- inject relevant memories in `before_agent_start`,
- persist decisions at `save_point`.

Subagents can be added as:

- a tool (`spawn_subagent`) executed by the main loop,
- a harness service that runs nested agents,
- a hook that routes some prompts or tool calls to a child runtime,
- a future scheduler outside the low-level loop.

The core requirement is that subagents communicate through messages/events/tools rather than special TUI logic.

## Error Handling

Provider stream failures should be represented as assistant messages with `stop_reason = error` or `aborted` where possible, not as panics.

Tool failures should be represented as tool-result messages with `is_error = true`. Tool implementations may return expected errors or throw/return unexpected errors, but the loop normalizes them.

Expected filesystem/process failures in `ExecutionEnv` should use typed result errors, not exceptions/panics.

Unexpected internal bugs may return `anyhow::Error` at API boundaries, but the agent loop should emit a full failure lifecycle where possible.

## Testing Strategy

Testing should drive the core before any Ratatui work.

Required test groups:

1. Agent state initialization and mutation.
2. Full lifecycle event order for a basic prompt.
3. Assistant streaming text updates.
4. Tool call execution and tool-result message creation.
5. Parallel tool execution completion order vs source-order persistence.
6. Sequential tool override behavior.
7. `before_tool_call` blocking and argument rewrite.
8. `after_tool_call` result patching and termination hints.
9. Steering queue modes: all vs one-at-a-time.
10. Follow-up queue behavior.
11. Abort behavior during streaming and during tools.
12. Subscriber wait/idle semantics.
13. In-memory session context reconstruction.
14. JSONL session persistence and reload.
15. Branch navigation and compaction context reconstruction.
16. Execution environment local filesystem/process behavior.

Use faux stream providers and fake tools to avoid external APIs.

## Implementation Checklist

- [x] Define core message, content, model, usage, stop reason, and stream event types — planned in Task 2.
- [x] Define agent event enum and event sink/subscriber abstractions — planned in Task 3.
- [x] Define tool trait, tool result, tool execution mode, and schema-validation boundary — planned in Task 3.
- [x] Implement faux stream provider utilities for tests — planned in Task 3.
- [x] Implement low-level `run_agent_loop` for basic text streaming without tools — planned in Task 4.
- [x] Add message lifecycle and turn/agent lifecycle event tests — planned in Tasks 4 and 12.
- [x] Add tool call execution for sequential mode — planned in Task 5.
- [x] Add parallel tool execution with completion-order events and source-order tool-result messages — planned in Task 5.
- [x] Add steering and follow-up queue drain callbacks to the loop — planned in Task 6.
- [x] Add `prepare_next_turn`, `should_stop_after_turn`, and terminating tool batch behavior — planned in Tasks 5 and 6.
- [x] Implement `Agent` wrapper with transcript state, subscribers, cancellation, prompt/continue/abort, and queues — planned in Task 6.
- [x] Test subscriber await semantics and concurrent prompt guard — planned in Task 6.
- [x] Define session entry types and in-memory append-only tree storage — planned in Task 7.
- [x] Implement session context reconstruction, compaction entries, branch summaries, labels, and session info — planned in Task 7.
- [x] Implement JSONL session storage and repository operations — planned in Task 8.
- [x] Define `ExecutionEnv` trait with typed result errors — planned in Task 9.
- [x] Implement local execution environment — planned in Task 9.
- [x] Implement harness with model/tools/resources/system prompt/auth/provider hooks — planned in Task 10.
- [x] Implement typed hook registry for notification events and mutating hooks — planned in Task 10.
- [x] Add harness tests for prompt, queue update, save point, provider hooks, tool hooks, and pending session writes — planned in Tasks 10 and 12.
- [x] Document public core APIs before starting Ratatui integration — planned in Task 11.

## Open Questions

1. Should the first implementation be one crate with modules, or a Cargo workspace with multiple crates from the start?
2. Which schema system should tools use initially: `schemars`, raw JSON Schema, or a custom minimal schema enum?
3. Should hooks run sequentially only, or should notification events support parallel fan-out later?
4. How much of provider serialization should live in core versus a separate provider crate?
5. Should memory be designed as a first-party harness service early, or remain purely hook-driven until the core is stable?

## Out of Scope

- Ratatui UI implementation.
- Real LLM provider implementations.
- Real subagent implementation.
- Real memory database or vector search.
- Dynamic plugin ABI.
- Permissions UI.
- MCP compatibility.
- Workflow/package system.

These are future layers that should attach to the core through typed events, hooks, tools, sessions, and the harness.
