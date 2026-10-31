<img src="https://raw.githubusercontent.com/Neuron-Mr-White/Oino/main/docs/assets/brand/png/oino-128.png" alt="oino" width="64">

# oino-core

The coding-agent runtime for Oino: `AgentSession`, extension runner, compaction and retry.

* `runtime` — `AgentSessionRuntime`: create, new/switch/fork/clone/import sessions
* `session` — prompts, queues, models, tools, compaction flow, retry, bash, tree, stats, export
* `extensions` — instance contract, host operations, registries, composition dispatch
* `compaction` — cut points, summaries, branch summaries, overflow/retry classification
* `agents` — nested agent sessions behind `agents.spawn`
* `events`, `hub`, `topics` — client events, ordered fan-out, sticky bus topics
* `ui` — dialogs, fire-and-forget requests, dismissals over the UI registry
* `builtin` — built-in tools bound to session context
* `testing` (feature) — faux provider component, fake extensions, in-memory sessions
