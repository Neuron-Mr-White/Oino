# Oino

Oino is a Rust-native agent runtime inspired by Pi's core architecture. This first workspace intentionally focuses on the headless core runtime: typed messages and stream events, a low-level agent loop, stateful agent wrapper, append-only sessions, deterministic harness hooks, and an execution environment boundary.

## Layer boundaries

- `oino-types`: model-visible/runtime-visible data types. No async runtime, provider, session, or filesystem dependencies.
- `oino-agent-loop`: pure async loop, stream consumption, event sink, tool protocol, and faux test utilities. Provider serialization stays outside this crate.
- `oino-agent`: stateful wrapper around the loop with queues, subscribers, cancellation, and idle settlement.
- `oino-session`: append-only session trees plus JSONL persistence. It reconstructs model context without owning providers/tools.
- `oino-env`: execution-environment abstraction and local filesystem/process adapter for future tools.
- `oino-harness`: high-level binding of agent, sessions, env, providers, resources, and typed hooks.

Out of scope for this phase: Ratatui UI, real provider integrations, MCP, subagents, first-party memory database, dynamic plugin ABI, and permissions UI.
