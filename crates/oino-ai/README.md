# oino-ai

Message, content, model and streaming types for the Oino agent stack.

Every type serialises to a fixed JSON shape (field names, tags, optionality), so sessions, RPC
records and WASM-boundary payloads stay interchangeable across processes and tools that speak
the same format. The crate has no async runtime and builds for `wasm32-wasip2`; guests and host
share these types.

* `primitives` — timestamps, thinking levels, stop reasons, usage
* `content` — text, thinking, image and tool-call blocks
* `message` — LLM `Message` and coding-agent `AgentMessage` unions
* `model` — `Model`, model keys, thinking-level maps, costs and limits
* `stream` — assistant message events, provider events, partial assembly
* `transcript` / `transform` — system-message replay and provider-side transcript transforms
* `json` — insertion-ordered JSON objects and semantic comparison
