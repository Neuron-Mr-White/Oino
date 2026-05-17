# Oino

Oino is a Rust-native agent runtime inspired by Pi's architecture. The workspace now has a headless core plus the first rebuilt interactive shell: API-key auth, an OpenRouter provider adapter, Pi-like built-in coding tools, and a modular Ratatui chat interface with a transcript and composer.

## Quickstart: OpenRouter TUI

Set an OpenRouter API key and run the binary:

```bash
OPENROUTER_API_KEY=sk-or-... mise run dev
```

Equivalent Cargo command:

```bash
OPENROUTER_API_KEY=sk-or-... cargo run -p oino-app --bin oino
```

Standalone markdown rendering proof-of-concept:

```bash
mise run tui:render-test
```

Optional configuration:

```bash
OINO_MODEL=openrouter:openai/gpt-4o-mini \
OINO_OPENROUTER_REFERER=https://example.invalid \
OINO_OPENROUTER_TITLE=Oino \
OPENROUTER_API_KEY=sk-or-... \
mise run dev
```

Default model: `openrouter:openai/gpt-4o-mini`.

The TUI opens a configurable transcript and bottom composer. Type a prompt, press Enter to submit, use Ctrl-J, Alt-Enter, or Shift-Enter for a newline, paste multi-line text directly without pasted newlines submitting early, use Up/Down to move through multi-line input, watch the assistant response stream into the transcript, and exit with Esc or Ctrl-C. Scroll the transcript with PgUp/PgDn, Alt-Up/Alt-Down, Ctrl-Home/Ctrl-End, or bare Up/Down when the composer is empty; `Ctrl-O t` enters transcript focus for j/k, Home/End, and Esc back to the composer. Long transcripts show a right-side scrollbar with a bold thumb so you can see the current position. While a prompt is running, the transcript/composer/footer show `Generating…` and the composer input pauses. The app starts with Pi-like default coding tools: `read`, `bash`, `edit`, and `write`. Type `/` at the start of the composer to open command suggestions; arrows choose, Tab completes, Enter runs the highlighted command, and Esc dismisses suggestions before quitting. Type `/settings` or the `Ctrl-O s` chord to open the reusable settings overlay, or use command paths such as `/model openrouter:xai/glm-5.1`, `/thinking high`, `/settings model openrouter:xai/glm-5.1`, `/settings collapse thinking truncate`, and `/settings chat-style agentic`. The first settings page is a menu with arrow-marked choices, Enter opens dedicated child pages such as Model Selection, Thinking Level, Collapse Mode, or Chat Style, and `/` inside Model Selection opens an inline model search box that Esc clears back to the normal list UX. Chat Style switches immediately between `chat` (current bubble-style transcript), `agentic` (Codex-like activity rows), and `minimal` (jcode-like compact rows). Assistant output renders Markdown in every chat style, including headings, emphasis, links, lists, task lists, block quotes, labelled code-block boxes, and wrapped box-grid tables with alignment; markdown fences that only wrap a table are unwrapped so LLM showcase tables render as tables. Collapse Mode cycles thinking and tool display through Full, Truncate, and Collapse. The composer expands as drafts grow, input pauses while a prompt is running, and tiny terminals get a safe fallback message.

OpenRouter model names are cached at `~/.oino/openrouter-models.json`. The app loads that cache immediately, refreshes the full model list in the background on an interval, and uses each model's supported parameters to limit available thinking levels. Model identifiers use the single `provider:model-id` format, for example `openrouter:xai/glm-5.1`. Thinking `Off` is sent to OpenRouter explicitly as reasoning `none` with reasoning excluded, rather than relying on provider defaults. User-selected settings persist at `~/.oino/settings.json`; `OINO_MODEL` remains an environment override for the startup model. Sessions persist as JSONL under `~/.oino/sessions`; non-interactive continuation uses `oino --session <uuid> <message-or-command>`.

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
- `oino-env`: execution-environment abstraction and local filesystem/process adapter for tools.
- `oino-tools`: built-in Pi-like local coding tools (`read`, `bash`, `edit`, `write`) implemented on `ExecutionEnv`.
- `oino-harness`: high-level binding of agent, sessions, env, providers, resources, and typed hooks.
- `oino-auth`: generic credential storage/resolution. It knows provider ids/env-var mappings, not HTTP protocols.
- `oino-provider-openrouter`: OpenRouter model listing, request serialization, HTTP streaming, SSE parsing, and conversion into `AssistantStreamEvent`.
- `oino-tui`: modular Ratatui state, slash-command suggestions, reusable overlay/settings state, composer input handling, theming, and chat transcript rendering. No provider/auth logic.
- `oino-app`: binary/runtime wiring for auth + provider + harness + session + TUI, including non-blocking model-cache refresh.

Provider code is intentionally separate from auth: auth answers “what credential should provider `openrouter` use?”, while the provider knows OpenRouter's base URL, endpoint, headers, request JSON, SSE chunks, finish reasons, and tool-call shape. Neither concern leaks into `oino-agent-loop`.

## Troubleshooting

- Missing key: set `OPENROUTER_API_KEY` or create `~/.oino/auth.json` as shown above.
- 401/403: verify the OpenRouter key and account access.
- 429: wait for rate-limit reset or choose another model/account.
- 5xx/network errors: retry later or check connectivity.
- Terminal looks broken after a crash: run `reset` in the shell.

## Current limitations

The first shell supports token-by-token transcript updates for provider text/thinking deltas, Markdown-rendered assistant output, local coding tool calls, persisted JSONL sessions, non-interactive `--session <uuid>` continuation, and `/settings`/`/model`/`/thinking` settings commands. It does not yet include `/login`, a persisted session browser, MCP, plugins, memory DB, or permissions UI.
