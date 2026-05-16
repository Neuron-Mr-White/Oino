# Oino

Oino is a Rust-native agent runtime inspired by Pi's architecture. The workspace now has a headless core plus the first interactive shell: API-key auth, an OpenRouter provider adapter, and a minimal Ratatui UI with a message list and input box.

## Quickstart: OpenRouter TUI

Set an OpenRouter API key and run the binary:

```bash
OPENROUTER_API_KEY=sk-or-... cargo run -p oino-app --bin oino
```

Optional configuration:

```bash
OINO_MODEL=openai/gpt-4o-mini \
OINO_OPENROUTER_REFERER=https://example.invalid \
OINO_OPENROUTER_TITLE=Oino \
OPENROUTER_API_KEY=sk-or-... \
cargo run -p oino-app --bin oino
```

Default model: `openai/gpt-4o-mini`.

The TUI opens a message panel and a bottom input box. Type a prompt, press Enter, wait for the assistant response, and exit with Esc or Ctrl-C.

## Auth file

Oino can also read an API key from `~/.oino/auth.json`:

```json
{
  "openrouter": { "type": "api_key", "key": "sk-or-REDACTED" }
}
```

Resolution order is:

1. runtime/test override
2. `~/.oino/auth.json`
3. `OPENROUTER_API_KEY`

The auth crate writes the file with user-only permissions on Unix where feasible and avoids logging secret values.

## Layer boundaries

- `oino-types`: model-visible/runtime-visible data types. No async runtime, provider, session, or filesystem dependencies.
- `oino-agent-loop`: pure async loop, stream consumption, event sink, tool protocol, and faux test utilities. Provider serialization stays outside this crate.
- `oino-agent`: stateful wrapper around the loop with queues, subscribers, cancellation, and idle settlement.
- `oino-session`: append-only session trees plus JSONL persistence. It reconstructs model context without owning providers/tools.
- `oino-env`: execution-environment abstraction and local filesystem/process adapter for future tools.
- `oino-harness`: high-level binding of agent, sessions, env, providers, resources, and typed hooks.
- `oino-auth`: generic credential storage/resolution. It knows provider ids/env-var mappings, not HTTP protocols.
- `oino-provider-openrouter`: OpenRouter request serialization, HTTP streaming, SSE parsing, and conversion into `AssistantStreamEvent`.
- `oino-tui`: Ratatui state/rendering for messages and one-line input. No provider/auth logic.
- `oino-app`: binary/runtime wiring for auth + provider + harness + session + TUI.

Provider code is intentionally separate from auth: auth answers “what credential should provider `openrouter` use?”, while the provider knows OpenRouter's base URL, endpoint, headers, request JSON, SSE chunks, finish reasons, and tool-call shape. Neither concern leaks into `oino-agent-loop`.

## Troubleshooting

- Missing key: set `OPENROUTER_API_KEY` or create `~/.oino/auth.json` as shown above.
- 401/403: verify the OpenRouter key and account access.
- 429: wait for rate-limit reset or choose another model/account.
- 5xx/network errors: retry later or check connectivity.
- Terminal looks broken after a crash: run `reset` in the shell.

## Current limitations

The first shell renders final responses after the provider call completes. It does not yet include token-by-token TUI streaming, `/login`, model picker, persisted session browser, markdown rendering, MCP, plugins, memory DB, or permissions UI.
