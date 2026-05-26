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

- `oino-app` — binary wiring, TUI loop, extension loading, 9router commands, model cache.
- `oino-tui` — Ratatui state, rendering, commands, settings, keymaps, themes.
- `oino-harness` — connects agent, sessions, resources, tools, and providers.
- `oino-agent-loop` / `oino-agent` — model event loop, tool calls, cancellation, streaming.
- `oino-session` — append-only JSONL sessions in `~/.oino/sessions`.
- `oino-resource` — Oino-owned prompts, skills, and instruction files.
- `oino-tools` / `oino-env` — local read/bash/edit/write tools on an execution environment.
- `oino-extension-*` — manifests, manager, runtime boundary, SDK, and built-ins.
- `oino-auth` — provider-neutral credential document primitives used by extensions.
- `oino-provider-openrouter` — OpenAI-compatible transport used by extension providers such as 9router.

## Design rules

- Providers and auth are extension/router concerns, not hard-coded TUI behavior.
- Rendering is state-driven; slow work happens outside render paths.
- Oino loads Oino-owned paths by default: `~/.oino/` and project `.oino/`.
- Model identifiers use `provider:model`, for example `9router:openai/gpt-4.1`.
