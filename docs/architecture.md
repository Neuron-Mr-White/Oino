# Architecture

Oino is a Rust-native terminal agent with a small core and extension-first provider routing.

## Big picture

```text
TUI/composer ──> app runtime ──> harness ──> agent loop ──> provider runtime
                    │              │             │
                    │              │             └── tools
                    │              └── sessions/resources
                    └── extensions/settings/model cache
```

## Core crates

- `oino-app` — binary wiring, TUI loop, extension loading, OmniRoute commands, model cache.
- `oino-tui` — Ratatui state, rendering, commands, settings, keymaps, themes.
- `oino-harness` — connects agent, sessions, resources, tools, and providers.
- `oino-agent-loop` / `oino-agent` — model event loop, tool calls, cancellation, streaming.
- `oino-session` — append-only JSONL sessions in `~/.oino/sessions`.
- `oino-resource` — Oino-owned prompts, skills, and instruction files.
- `oino-tools` / `oino-env` — local read/bash/edit/write tools on an execution environment.
- `oino-extension-*` — manifests, manager, runtime boundary, SDK, and built-ins.
- `oino-auth` — provider-neutral credential document primitives used by extensions.
- `oino-provider-openrouter` — OpenAI-compatible transport used by extension providers such as OmniRoute.

## WASM extension host

`crates/oino-wasm-host` runs extension components built against
`oino:extension@0.1.0` (`wit/`). A shared `WasmHostEngine` (epoch interruption
every 10 ms, wasmtime disk cache under the configured dir) serves many
`WasmExtension`s; each is one lazily-instantiated store+instance with calls
serialized by a mutex. `host` imports are gated through the manifest
`ExtensionPermissions` (network hosts, filesystem preopens, secrets, exec,
kv, ui, session). `WasmStreamProvider` adapts provider-world components to the
agent loop's `StreamProvider`. Guest-side SDK crates live in the separate
`extensions/` workspace (`oino-extension-guest`,
`oino-extension-guest-provider`).

## Transcript and prompt sync

Sessions persist `System` messages inside the transcript (session format v2).
The leading `System` carries the base prompt and tool declarations; later
`System` entries are deltas — appended instructions, named-section patches
(`text: None` removes), and tool additions/removals. `oino_types::transcript`
replays them into the current prompt (`current_system`, `render_system_prompt`)
and tool set (`current_tools`), and `collapse` rebuilds a single leading system
message for providers without mid-conversation support.

`Harness::sync_system(base, sections)` records the desired prompt state;
`flush_system` runs before each prompt and appends at most one delta `System`
to both the agent context and the session. A changed base is carried as the
pseudo-section `oino/base`. Sessions also support `ContextEdit` (omit/replace a
message at context-build time) and `Usage` entries (never enter context).

## Design rules

- Providers and auth are extension/router concerns, not hard-coded TUI behavior.
- Rendering is state-driven; slow work happens outside render paths.
- Oino loads Oino-owned paths by default: `~/.oino/` and project `.oino/`.
- Model identifiers use `provider:model`, for example `router:openai/gpt-4.1`.
