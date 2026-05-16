# Oino/Pi Compatibility Notes

Oino is Rust-native and does not preserve Pi's TypeScript names mechanically, but this core preserves the important runtime semantics learned from `@earendil-works/pi-coding-agent` and `@earendil-works/pi-agent-core`.

## Core loop mapping

| Pi concept | Oino API |
| --- | --- |
| `agentLoop`, `runAgentLoop` | `oino_agent_loop::run_agent_loop` |
| `agentLoopContinue`, `runAgentLoopContinue` | `oino_agent_loop::run_agent_loop_continue` |
| `convertToLlm` / provider payloads | Kept outside the pure loop; adapters emit `AssistantStreamEvent` |
| `transformContext` | `AgentLoopConfig::transform_context` |
| `beforeToolCall` / `afterToolCall` | `AgentLoopConfig::before_tool_call` / `after_tool_call` |
| `shouldStopAfterTurn` | `AgentLoopConfig::should_stop_after_turn` |
| steering/follow-up callbacks | `get_steering_messages` / `get_follow_up_messages` |
| tool execution mode | `ToolExecutionMode::{Parallel, Sequential}` |

## Event mapping

Oino's `AgentEvent` covers Pi lifecycle events:

- `agent_start`, `agent_end` → `AgentStart`, `AgentEnd`
- `turn_start`, `turn_end` → `TurnStart`, `TurnEnd`
- `message_start`, `message_update`, `message_end` → `MessageStart`, `MessageUpdate`, `MessageEnd`
- `tool_execution_start`, `tool_execution_update`, `tool_execution_end` → matching Oino variants
- harness-ready events → `QueueUpdate`, `SavePoint`, `Settled`

Event dispatch is deterministic and sequential per registered sink/hook order. Notification hooks observe state; mutation occurs only through explicit typed return values.

## Default built-in tools

Pi's default model-visible tools are `read`, `bash`, `edit`, and `write`. Oino now wires the same initial set through `oino-tools`:

| Pi default tool | Oino implementation |
| --- | --- |
| `read` | `ReadTool` with `path`, `offset`, and `limit`; text output truncates with continuation hints |
| `bash` | `BashTool` with `command` and optional timeout seconds; stdout/stderr truncate from the tail |
| `edit` | `EditTool` with exact unique `edits[].oldText` replacements |
| `write` | `WriteTool` that creates parents and overwrites content |

`write` and `edit` request sequential execution to avoid concurrent file mutations. Image reads currently return a text note instead of a model-visible image block because the first OpenRouter adapter still serializes only text content.

## Tool semantics

Tools implement `Tool` and expose:

- `ToolDefinition` with an isolated JSON Schema boundary.
- Optional `prepare_arguments` before execution.
- `execution_mode` for per-tool sequential override.
- `ToolUpdateCallback` for progress updates.
- `ToolResult` with `is_error`, `details`, and `terminate` hints.

Tool errors are normalized into error `ToolResult`s by the loop. Tool arguments pass through a JSON Schema boundary that currently validates common `type` and `required` constraints. Parallel tool completion events may complete in runtime order, while final tool-result messages preserve assistant source order.

## Agent wrapper mapping

| Pi Agent API | Oino API |
| --- | --- |
| `prompt` | `Agent::prompt` |
| `continue` | `Agent::continue_from_current_context` |
| `steer` | `Agent::steer` |
| `followUp` | `Agent::follow_up` |
| `abort` | `Agent::abort` |
| `waitForIdle` | `Agent::wait_for_idle` |
| `reset` | `Agent::reset` |
| `subscribe` | `Agent::subscribe` |
| queue mode `all` / `one-at-a-time` | `QueueMode::All` / `QueueMode::OneAtATime` |

The agent prevents concurrent prompts and remains non-idle until loop/subscriber settlement completes.

## Session mapping

`oino_session::SessionManager` mirrors Pi's append-only session tree concepts:

- session header and entries
- append message/model/thinking/compaction/custom/label/session-info entries
- `get_leaf_id`, `get_leaf_entry`, `get_entry`, `get_branch`, `get_tree`, `get_children`
- `branch`, `branch_with_summary`, `reset_leaf`
- `build_session_context` with compaction replacement and branch summaries
- JSONL save/load with versioned header

## Harness hooks

Oino `HookRegistry` currently models these Pi-inspired hook categories:

### Notification hooks

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

### Mutating hooks

- `before_agent_start`
- `context`
- `before_provider_request`
- `before_provider_payload`
- `after_provider_response`
- `before_tool_call`
- `after_tool_call`
- `before_compaction`
- `before_tree_navigation`

Provider request/payload/response hooks are typed string placeholders in this first runtime because provider-specific serialization is intentionally outside the pure loop. The harness now runs those hooks around the stream-provider request lifecycle and exposes an auth resolver boundary for provider adapters.

## Execution environment

Pi's local/remote operation boundary maps to `oino_env::ExecutionEnv`, with `LocalExecutionEnv` implementing shell, file, directory, stat, realpath, temp, and cleanup operations. Future sandbox, remote, and container adapters should implement the same trait.

## Deferred layers

The following remain intentionally out of scope for this core pass: subagents, memory database, dynamic plugin ABI, permissions UI, MCP, workflow packages, image tool-result serialization, and per-file mutation queues.
