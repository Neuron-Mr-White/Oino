---
title: "Oino Core Runtime — Implementation Plan"
type: plan
date: 2026-05-15
workbranch: ""
specs:
  - .unipi/docs/specs/2026-05-15-oino-core-design.md
---

# Oino Core Runtime — Implementation Plan

## Overview

Build Oino's first Rust core runtime from an empty repository. The implementation should mirror Pi's strongest architectural lessons while staying Rust-native: a small async agent loop, a stateful agent wrapper, append-only tree sessions, a high-level harness, typed notification/mutating hooks, pluggable execution environment, and tests driven by faux providers/tools.

Branch strategy: **main branch** (`workbranch` intentionally empty), per user selection.

The plan assumes the first implementation starts as a single Cargo workspace with crates split by clear boundaries from day one. If build complexity slows initial work, crates may temporarily be implemented as modules behind the same public boundaries, but the public API should already reflect the target layering.

## Planning Decisions From Spec Open Questions

- **Crate shape:** Start as a Cargo workspace with clear crates; allow temporary module consolidation only if implementation friction is high.
- **Tool schemas:** Use an isolated schema boundary. Prefer `schemars` initially if it does not slow progress; raw JSON Schema remains acceptable behind the same trait/type boundary.
- **Hook execution:** Run hooks sequentially in deterministic registration order for the first version. Parallel notification fan-out can be added later after ordering and cancellation semantics are stable.
- **Provider serialization:** Keep provider serialization outside the pure loop. The loop consumes/produces typed stream events; harness/provider adapters own serialized payload details.
- **Memory:** Keep memory hook-driven for now. Do not add a first-party memory database until core events, context hooks, and save points are stable.

## Pi Reference Inventory To Preserve

This inventory is based on Pi `@earendil-works/pi-coding-agent` v0.74.0 docs/types and `@earendil-works/pi-agent-core` v0.74.0 types. Oino does not need TypeScript-compatible names, but should preserve the semantics.

### Pi low-level agent-core APIs

- Loop functions: `agentLoop`, `agentLoopContinue`, `runAgentLoop`, `runAgentLoopContinue`.
- Core config surfaces: model, system prompt/context, tools, stream function, `convertToLlm`, `transformContext`, `getApiKey`, `shouldStopAfterTurn`, `getSteeringMessages`, `getFollowUpMessages`, `toolExecution`, `beforeToolCall`, `afterToolCall`, transport/options/retry/thinking settings.
- Core events: `agent_start`, `agent_end`, `turn_start`, `turn_end`, `message_start`, `message_update`, `message_end`, `tool_execution_start`, `tool_execution_update`, `tool_execution_end`.
- Agent wrapper APIs: `prompt`, `continue`, `steer`, `followUp`, `abort`, `waitForIdle`, `reset`, `subscribe`, state accessors for messages/tools/model/thinking/system prompt, queue modes `all` and `one-at-a-time`.
- Tool APIs: `AgentTool`, `AgentToolResult`, `AgentToolUpdateCallback`, `prepareArguments`, per-tool `executionMode`, tool errors by throwing/returning errors normalized to `isError` results.

### Pi extension hooks/events to model in Oino harness

- Resource: `resources_discover`.
- Session: `session_start`, `session_before_switch`, `session_before_fork`, `session_before_compact`, `session_compact`, `session_before_tree`, `session_tree`, `session_shutdown`.
- Input/user shell: `input`, `user_bash`.
- Agent lifecycle: `before_agent_start`, `agent_start`, `agent_end`, `turn_start`, `turn_end`.
- Message lifecycle: `message_start`, `message_update`, `message_end`.
- Provider: `context`, `before_provider_request`, `after_provider_response`.
- Tool: `tool_execution_start`, `tool_execution_update`, `tool_execution_end`, `tool_call`, `tool_result`.
- Model: `model_select`, `thinking_level_select`.

### Pi ExtensionAPI methods to account for

- Event and extension registration: `on`, `registerTool`, `registerCommand`, `registerShortcut`, `registerFlag`, `getFlag`, `registerMessageRenderer`.
- Message/session injection: `sendMessage`, `sendUserMessage`, `appendEntry`, `setSessionName`, `getSessionName`, `setLabel`.
- Runtime/tool controls: `exec`, `getActiveTools`, `getAllTools`, `setActiveTools`, `getCommands`, `setModel`, `getThinkingLevel`, `setThinkingLevel`.
- Provider controls: `registerProvider`, `unregisterProvider`.
- Inter-extension communication: `events` bus.

### Pi ExtensionContext / CommandContext APIs to account for

- Event context: `ui`, `hasUI`, `cwd`, `sessionManager`, `modelRegistry`, `model`, `isIdle`, `signal`, `abort`, `hasPendingMessages`, `shutdown`, `getContextUsage`, `compact`, `getSystemPrompt`.
- Command-only context: `waitForIdle`, `newSession`, `fork`, `navigateTree`, `switchSession`, `reload`.
- Replacement-session helpers: `sendMessage`, `sendUserMessage` bound to the replacement session.

### Pi UI API categories to keep harness-ready, but not implement in core

- Dialogs: `select`, `confirm`, `input`, `editor`, `custom`.
- Notifications/status: `notify`, `setStatus`, `setWorkingMessage`, `setWorkingVisible`, `setWorkingIndicator`, `setHiddenThinkingLabel`.
- Layout/editor: `setWidget`, `setFooter`, `setHeader`, `setTitle`, `setEditorText`, `getEditorText`, `pasteToEditor`, `addAutocompleteProvider`, `setEditorComponent`, `getEditorComponent`.
- Theme/tool display: `theme`, `getAllThemes`, `getTheme`, `setTheme`, `getToolsExpanded`, `setToolsExpanded`.

### Pi SDK/session APIs to learn from

- Factories/runtime: `createAgentSession`, `createAgentSessionRuntime`, `createAgentSessionServices`, `createAgentSessionFromServices`, `AgentSessionRuntime`.
- `AgentSession`: `prompt`, `steer`, `followUp`, `subscribe`, `setModel`, `setThinkingLevel`, `cycleModel`, `cycleThinkingLevel`, `navigateTree`, `compact`, `abortCompaction`, `abort`, `dispose`, state fields like `sessionFile`, `sessionId`, `agent`, `model`, `thinkingLevel`, `messages`, `isStreaming`.
- `SessionManager`: creation/open/list/fork factories; append message/model/thinking/compaction/custom/session-info/custom-message/label entries; tree APIs `getLeafId`, `getLeafEntry`, `getEntry`, `getBranch`, `getTree`, `getChildren`, `branch`, `branchWithSummary`, `resetLeaf`; context APIs `buildSessionContext`, `getEntries`, `getHeader`, `getSessionName`, `getCwd`, `getSessionDir`, `getSessionId`, `getSessionFile`, `isPersisted`.
- Settings/resource concepts: settings manager with global/project merge and async flush; default resource loader for extensions, skills, prompts, themes, and context files.
- Built-in tool operations: read/write/edit/bash/grep/find/ls tool factories; remote operation interfaces; file mutation queue; output truncation helpers.

### Oino API targets derived from Pi

- `oino_agent_loop`: `run_agent_loop`, `run_agent_loop_continue`, `AgentLoopConfig`, `AgentEventSink`, `StreamFn`, `ToolExecutionMode`, typed tool hooks, steering/follow-up drain callbacks.
- `oino_agent`: `Agent`, `AgentState`, `AgentEventSubscriber`, queue modes, prompt/continue/steer/follow-up/abort/wait APIs.
- `oino_session`: `SessionManager`, `SessionStorage`, `SessionEntry`, `SessionContext`, append-only tree and JSONL persistence APIs.
- `oino_harness`: `Harness`, `HarnessConfig`, `HookRegistry`, resource/model/tool/session APIs, provider/auth boundary, save points, session navigation/compaction orchestration.
- `oino_env`: `ExecutionEnv` trait and local implementation.

## Tasks

- completed: Task 1 — Bootstrap Rust Workspace and Architectural Boundaries
  - Completed: Initialized git/Cargo workspace, added six crates with boundary docs, baseline dependencies, README scope notes, and verified compilation.
  - Description: Create the initial Cargo workspace/crate layout and dependency baseline for the core runtime.
  - Dependencies: None.
  - Acceptance Criteria: Repository has a compiling Rust workspace; crate/module names match the spec; no TUI/provider implementation is introduced; README or crate docs briefly explain layer boundaries.
  - Steps:
    1. Add root `Cargo.toml` with workspace members for `crates/oino-types`, `crates/oino-agent-loop`, `crates/oino-agent`, `crates/oino-session`, `crates/oino-harness`, and `crates/oino-env`.
    2. Add minimal `lib.rs` files with crate-level docs describing responsibilities and forbidden dependencies.
    3. Add baseline dependencies (`tokio`, `futures`, `serde`, `serde_json`, `thiserror`, `uuid`, `async-trait` or native async trait strategy, `schemars` if chosen).
    4. Add a top-level docs note explaining that TUI, real providers, MCP, memory DB, and plugin ABI remain out of scope.

- completed: Task 2 — Define Shared Core Types
  - Description: Implement Rust-native message, model, stream, usage, and stop-reason types used by every layer.
  - Dependencies: Task 1.
  - Acceptance Criteria: Shared types serialize/deserialize where needed; tests cover basic JSON round trips; API docs identify which types are model-visible vs runtime-only.
  - Steps:
    1. Define `Model`, `Usage`, `UsageCost`, `ThinkingLevel`, `StopReason`, and provider metadata types.
    2. Define `Message` enum with user, assistant, tool-result, custom, compaction summary, and branch summary variants.
    3. Define `ContentBlock` enum for text, image, thinking, and tool-call content.
    4. Define `AssistantStreamEvent` variants for text/thinking deltas, tool-call deltas/finalization, usage, completion, error, and abort.
    5. Add serde tests for representative messages and stream events.

- completed: Task 3 — Define Event Sink, Tool Traits, and Faux Test Utilities
  - Description: Establish the public event and tool protocol before implementing loop behavior.
  - Dependencies: Task 2.
  - Acceptance Criteria: `AgentEvent` covers lifecycle/message/tool events; tool trait supports updates, cancellation, schema boundary, execution mode, and termination hints; faux streams/tools can be used in tests without external APIs.
  - Steps:
    1. Define `AgentEvent` variants: `AgentStart`, `AgentEnd`, `TurnStart`, `TurnEnd`, `MessageStart`, `MessageUpdate`, `MessageEnd`, `ToolExecutionStart`, `ToolExecutionUpdate`, `ToolExecutionEnd`, plus Oino harness-ready `QueueUpdate`, `SavePoint`, and `Settled` where appropriate.
    2. Define `EventSink` abstraction with async dispatch semantics and deterministic ordering.
    3. Define `Tool`, `ToolDefinition`, `ToolResult`, `ToolUpdateCallback`, `ToolExecutionMode`, and schema-validation boundary types.
    4. Implement test-only faux stream builders and fake tools with controllable delay, errors, updates, and termination.
    5. Document that thrown/internal tool errors are normalized into error tool results.

- completed: Task 4 — Implement Low-Level Text Streaming Loop
  - Description: Implement `run_agent_loop` for basic user prompt to assistant text response without tools.
  - Dependencies: Tasks 2 and 3.
  - Acceptance Criteria: Lifecycle event order is deterministic and tested; assistant text deltas produce `MessageUpdate`; provider stream errors become assistant messages with `StopReason::Error` or `Aborted` where possible.
  - Steps:
    1. Define `AgentLoopConfig` with model, system prompt, tools, stream function, context transform, provider conversion, queue callbacks, and stop predicate placeholders.
    2. Add prompt messages to context and emit prompt message lifecycle events.
    3. Call `transform_context` and provider conversion before each model request.
    4. Consume faux assistant stream events into an assistant message.
    5. Emit `TurnStart`, assistant `MessageStart/Update/End`, `TurnEnd`, and `AgentEnd`.
    6. Add tests for normal stop, length stop, stream error, and abort during streaming.

- completed: Task 5 — Implement Tool Execution Pipeline
  - Description: Add tool-call detection, validation/preparation, sequential and parallel execution, tool hooks, result messages, and termination semantics.
  - Dependencies: Task 4.
  - Acceptance Criteria: Tests prove Pi-compatible behavior: preflight in assistant source order; parallel completion events in completion order; final tool-result messages in assistant source order; sequential override serializes a whole batch; blocked/failed tools produce error results.
  - Steps:
    1. Parse finalized assistant messages for tool-call blocks.
    2. Run `prepare_arguments` and schema validation in source order.
    3. Add `before_tool_call` hook result handling for block/rewrite semantics.
    4. Execute allowed tools concurrently by default, but switch the whole batch to sequential if global mode or any tool requires sequential execution.
    5. Emit `ToolExecutionStart`, `ToolExecutionUpdate`, and `ToolExecutionEnd` with Pi-compatible ordering.
    6. Add `after_tool_call` hook patching for content, details, error state, and termination hint.
    7. Append tool-result message events in assistant source order.
    8. Stop after a tool batch only when every finalized tool result requests termination.

- completed: Task 6 — Add Steering, Follow-Up, Stop Predicate, and Agent Wrapper
  - Description: Wrap the loop in a stateful `Agent` that owns transcript, queues, subscribers, cancellation, and public runtime methods.
  - Dependencies: Task 5.
  - Acceptance Criteria: `Agent` prevents concurrent prompts, supports abort, drains steering/follow-up queues in configured modes, and remains non-idle until awaited subscribers finish after `AgentEnd`.
  - Steps:
    1. Implement `AgentState` with copied message/tool array assignment semantics where applicable.
    2. Implement `prompt`, `continue_from_current_context`, `steer`, `follow_up`, `abort`, `wait_for_idle`, `reset`, and state getters/setters.
    3. Implement queue modes `all` and `one_at_a_time` for steering and follow-up delivery.
    4. Wire `should_stop_after_turn`, `get_steering_messages`, and `get_follow_up_messages` into loop continuation.
    5. Implement subscriber registration and await subscriber futures as part of run settlement.
    6. Add tests for concurrent prompt guard, queue delivery, abort during tools, and subscriber wait/idle semantics.

- completed: Task 7 — Implement In-Memory Append-Only Session Tree
  - Description: Build session entries and context reconstruction before persistence.
  - Dependencies: Task 2.
  - Acceptance Criteria: In-memory sessions support append-only tree branching, leaf movement, labels, model/thinking changes, compaction entries, branch summaries, custom entries/messages, and context reconstruction.
  - Steps:
    1. Define session header and entry variants: message, model change, thinking-level change, compaction, branch summary, custom, custom message, label, session info.
    2. Implement `SessionManager` over an in-memory storage backend.
    3. Implement append methods returning entry IDs.
    4. Implement tree APIs: get entries, get leaf, get path/branch, get children, branch, branch with summary, reset leaf.
    5. Implement labels and session name lookup.
    6. Implement `build_session_context` including compaction replacement semantics and branch/custom-message conversion.
    7. Add tests for branch navigation and compaction reconstruction.

- completed: Task 8 — Implement JSONL Session Storage
  - Description: Persist and reload session trees using a Pi-like JSONL format.
  - Dependencies: Task 7.
  - Acceptance Criteria: JSONL sessions round-trip; malformed entries produce typed errors; reload preserves tree/leaf behavior; tests use temp directories and do not depend on real Pi sessions.
  - Steps:
    1. Define JSONL header and entry serialization format with versioning.
    2. Implement file storage append/load operations.
    3. Add repository helpers for create/open/list/continue-most-recent if practical for this phase.
    4. Add migration placeholder hooks for future format changes.
    5. Test persistence, reload, labels, compaction, and branch summaries.

- completed: Task 9 — Define Execution Environment Trait and Local Implementation
  - Description: Create the tool execution environment boundary so tools do not call filesystem/process APIs directly.
  - Dependencies: Task 1.
  - Acceptance Criteria: `ExecutionEnv` exposes typed result errors for shell, file, path, directory, stat, temp, and cleanup operations; local implementation is tested independently.
  - Steps:
    1. Define `ExecutionEnv` trait and error enum.
    2. Add methods for shell execution, read/write/append text and binary, list directory, stat, realpath, create/remove files/directories, temp files/dirs, and cleanup.
    3. Implement local filesystem/process environment with cancellation-aware command execution where feasible.
    4. Add tests for expected error cases and path resolution.
    5. Document that sandbox/remote/container implementations are future adapters.

- completed: Task 10 — Implement Harness Skeleton and Typed Hook Registry
  - Description: Bind loop, agent, session, model/tools/resources, execution environment, auth/provider boundaries, and hook registry into a high-level runtime.
  - Dependencies: Tasks 6, 7, and 9.
  - Acceptance Criteria: Harness can load context from session, run a prompt through `Agent`, persist safe save points, expose notification and mutating hooks, and pass tests for prompt/save/queue/provider/tool hook behavior using fake providers.
  - Steps:
    1. Define `HarnessConfig` and `Harness` with active model, thinking level, tools, resources, stream options, auth/provider resolver, execution env, session manager, and agent.
    2. Define `HookRegistry` with deterministic registration order.
    3. Implement notification hooks: `agent_start`, `turn_start`, `message_start`, `message_update`, `message_end`, `tool_execution_start`, `tool_execution_update`, `tool_execution_end`, `turn_end`, `agent_end`, `queue_update`, `save_point`, `settled`.
    4. Implement mutating hooks: `before_agent_start`, `context`, `before_provider_request`, `before_provider_payload`, `after_provider_response`, `before_tool_call`, `after_tool_call`, `before_compaction`, `before_tree_navigation`.
    5. Implement high-level APIs: `prompt`, `skill`, `prompt_template`, `compact`, `navigate_tree`, `set_model`, `set_tools`, `set_resources`, and context/system prompt build entry points.
    6. Persist user/assistant/tool messages and runtime changes at save points after subscriber/hook settlement.
    7. Add tests for hook ordering, typed return behavior, queue update events, save points, provider hooks, tool hook patch/block behavior, and pending session writes.

- completed: Task 11 — Public API Documentation and Pi Compatibility Notes
  - Description: Document the public Rust APIs and the compatibility decisions before Ratatui integration begins.
  - Dependencies: Tasks 2 through 10.
  - Acceptance Criteria: Generated or hand-written docs identify all public Oino hooks/APIs; docs explicitly map Oino APIs to Pi concepts; out-of-scope future layers are called out.
  - Steps:
    1. Add crate/module docs for `oino-types`, `oino-agent-loop`, `oino-agent`, `oino-session`, `oino-env`, and `oino-harness`.
    2. Add a compatibility markdown document listing Pi-inspired hook/event/API semantics and Oino names.
    3. Document hook ordering, mutability boundaries, cancellation expectations, and subscriber settlement semantics.
    4. Document session JSONL format and context reconstruction rules.
    5. Document tool authoring patterns, schema validation boundary, termination hints, and execution environment usage.

- completed: Task 12 — End-to-End Test Matrix and Quality Gate
  - Description: Complete the test suite required by the spec and ensure the workspace is ready for future TUI/provider work.
  - Dependencies: Tasks 1 through 11.
  - Acceptance Criteria: All required tests from the spec exist and pass; workspace formatting/lint/build pass; no real API keys/providers are required; TODOs are limited to explicitly out-of-scope future work.
  - Steps:
    1. Add integration tests for full prompt lifecycle, streaming text, tool execution, parallel/sequential behavior, steering/follow-up, aborts, sessions, JSONL persistence, compaction, branch summaries, hooks, and execution env.
    2. Add faux provider fixtures for deterministic stream/tool scenarios.
    3. Run `cargo fmt`, `cargo clippy`, and `cargo test` during implementation review.
    4. Verify public docs build with `cargo doc` if applicable.
    5. Prepare a short completion note identifying remaining future work: Ratatui UI, real providers, subagents, memory DB, dynamic plugins, permissions UI, MCP/workflow packages.

## Sequencing

1. Tasks 1–3 establish the compileable foundation and public protocol.
2. Tasks 4–6 implement the core loop and stateful runtime wrapper.
3. Tasks 7–8 implement session semantics and persistence in parallel with or after the wrapper.
4. Task 9 can run after workspace bootstrap and before real built-in tools are added.
5. Task 10 binds the layers and is blocked on loop, wrapper, session, and env APIs.
6. Tasks 11–12 finalize docs and test coverage.

Dependency graph:

```text
Task 1
  ├─ Task 2 ── Task 3 ── Task 4 ── Task 5 ── Task 6 ──┐
  │            └───────────────────────────────────────┤
  ├─ Task 7 ── Task 8 ─────────────────────────────────┤
  └─ Task 9 ────────────────────────────────────────────┤
                                                         └─ Task 10 ── Task 11 ── Task 12
```

## Risks

- **Scope size:** The plan covers a foundational runtime, not a tiny MVP. If a single work pass becomes too large, complete Tasks 1–6 first, then resume with sessions/harness.
- **Rust async trait ergonomics:** Tool and hook traits need object-safe async patterns. Use boxed futures or `async-trait` initially if native async traits become cumbersome.
- **Schema validation choice:** `schemars` vs raw JSON Schema remains an implementation detail; keep the boundary isolated so it can change.
- **Hook overreach:** Notification handlers must not mutate loop state. Mutating hooks must only affect explicit typed return surfaces.
- **Provider abstraction leakage:** Keep provider serialization and request payload mutation behind harness/provider boundaries, not inside the pure loop.
- **Parallel tool races:** File-mutating tools must eventually use an Oino equivalent of Pi's file mutation queue.
- **Session leaf persistence:** A JSONL append-only file does not naturally encode mutable leaf unless modeled carefully. The first implementation should define whether leaf is in manager memory only or persisted via explicit navigation entries.
- **Testing without real providers:** Faux streams must be rich enough to exercise streaming, errors, tool calls, usage, and aborts.

---

## Reviewer Remarks

REVIEWER-REMARK: Partially Done 7/12
- Strong foundation is present and committed on `main` as `c9c7521`: the workspace compiles, all six planned crates exist, core shared types serialize, the basic loop streams text, tools execute, sessions persist to JSONL, the local execution env works, the harness exposes typed hooks, and Pi compatibility docs exist.
- Fully verified against acceptance criteria: Tasks 1, 2, 3, 7, 8, 9, and 11.
- Partially complete: Task 4 lacks a length-stop test and still has an `AgentStart`/`AgentEnd` run-id mismatch in `oino-agent-loop` (`AgentStart` uses a fresh UUID before `run_agent_loop_continue_inner`; `AgentEnd` uses the inner UUID). `ToolCallDelta` is currently ignored rather than accumulated.
- Partially complete: Task 5 has the tool protocol and execution pipeline, but no actual JSON Schema validation beyond storing `input_schema`; tests do not yet prove preflight order, completion-order tool events, blocked tool behavior, failed tool behavior, or hook patch/block behavior.
- Partially complete: Task 6 implements the wrapper, queues, abort, and subscribers, but tests do not yet cover the concurrent prompt guard or abort during tools; `AgentState` does not expose tools.
- Partially complete: Task 10 provides a usable harness skeleton, but provider/auth resolver boundaries are placeholders, provider hooks are exposed as string mutators rather than integrated into request flow, and `QueueUpdate` events are defined but not emitted/tested.
- Partially complete: Task 12 has passing unit/e2e coverage, but the required matrix is not complete for length stops, concurrent prompt guard, abort during tools, queue updates, and tool hook block/patch behavior.
- Ralph context: `.unipi/ralph/oino-core-runtime.state.json` still marks the loop active at iteration 1, and `.unipi/ralph/oino-core-runtime.md` has an unchecked checklist even though the plan file marks tasks completed. This appears to be workflow housekeeping rather than a code failure, but it should be reconciled before treating the Ralph loop as closed.

Codebase Checks:
- ✓ Format passed: `cargo fmt --all -- --check`
- ✓ Lint passed: `cargo clippy --workspace --all-targets -- -D warnings`
- ✓ Tests passed: `cargo test --workspace` (25 tests total: 20 unit + 5 integration, plus doctests)
- ✓ Docs build passed: `cargo doc --workspace --no-deps`
- ✓ Git branch: `main`; no worktree merge required.


---

## Work Follow-Up After Review

Status: completed. Addressed reviewer remarks on Tasks 4, 5, 6, 10, and 12.

- Fixed low-level loop lifecycle consistency: `AgentStart` and `AgentEnd` now share one run id.
- Added length-stop coverage and accumulated `ToolCallDelta` handling.
- Added a JSON Schema validation boundary for common `type` and `required` constraints.
- Added tests for preflight ordering, parallel completion-order tool events, source-order result messages, blocked tools, failed tools, and after-hook patching.
- Exposed active tool definitions in `AgentState`; added tests for concurrent prompt rejection and abort during tool execution.
- Emitted `QueueUpdate` events from steering/follow-up queues and tested them.
- Integrated provider request/payload/response hooks into the harness stream lifecycle and added an auth resolver boundary.
- Updated docs and Ralph checklist, then re-ran the full quality gate.

Follow-up Quality Gate:
- ✓ `cargo fmt --all -- --check`
- ✓ `cargo clippy --workspace --all-targets -- -D warnings`
- ✓ `cargo test --workspace` (37 tests total plus doctests)
- ✓ `cargo doc --workspace --no-deps`
