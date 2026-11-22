<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-agent

Provider-agnostic agent runtime for Oino: turns, steering and follow-up queues, sequential and
parallel tool execution, abort, and every loop hook.

The crate knows nothing about sessions, extensions or providers; `oino-core` supplies those
through the hook traits and a `StreamFn`.

* `agent_loop` — the turn loop, tool calls, steering
* `agent` — the stateful `Agent` with queues and listeners
* `config` — `AgentLoopConfig` and hook traits
* `tool` / `event` — the tool contract and lifecycle events
* `validate` — JSON-schema tool-argument validation and coercion
