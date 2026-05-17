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

The TUI opens a configurable transcript and bottom composer. Type `/help` for a focused shortcuts and commands overlay instead of keeping persistent help text under the input; press `/` inside Help to fuzzy-search the docs. Type a prompt, press Enter to submit, use Ctrl-J, Alt-Enter, or Shift-Enter for a newline, paste multi-line text safely, use Up/Down to move through multi-line input, watch the assistant response stream into the transcript, and exit only by pressing Ctrl-C twice. Esc never exits the app: it dismisses transient UI or stops a running response. Large pastes collapse visually into a `pasted N lines` block that still submits the full text; place the cursor beside/inside the block and press `Ctrl-O e` to expand it. Scroll the transcript with PgUp/PgDn, Alt-Up/Alt-Down, Ctrl-Home/Ctrl-End, or bare Up/Down when the composer is empty; `Ctrl-O t` enters transcript focus for j/k, Home/End, and Esc back to the composer. Long transcripts show a right-side scrollbar with a bold thumb so you can see the current position. While a prompt is running, the composer stays live; Enter sends the current input as steering, `Ctrl-O s` opens the send panel for queued follow-ups and drafts, and the newest transcript line shows runtime status such as `Calling OpenRouter…` or tool activity. In the send panel, Up/Down select Steer/Queue/Draft items, `q` queues the current input, `d` moves the current input to Draft, Enter loads the selected item into the composer, and `x` asks for delete confirmation. The app starts with Pi-like default coding tools: `read`, `bash`, `edit`, and `write`. Type `/` at the start of the composer to open Nucleo-backed fuzzy command suggestions, or type `@` anywhere in the composer to fuzzy search up to 10 project file paths; arrows choose and Tab inserts the highlighted command/path. Dropping file paths into terminals that paste dropped files inserts `@relative/path` mentions. Type `/help` for the help overlay, `/new` to start a fresh local session after the current one has messages, `/sessions` to browse saved sessions, press `/` inside the browser for Nucleo fuzzy search, and press Enter to continue one; type `/settings` to open the reusable settings overlay, or use command paths such as `/model openrouter:xai/glm-5.1`, `/thinking high`, `/settings model openrouter:xai/glm-5.1`, `/settings collapse thinking truncate`, and `/settings chat-style agentic`. The first settings page is a menu with arrow-marked choices, Enter opens dedicated child pages such as Model Selection, Thinking Level, Collapse Mode, or Chat Style, and `/` inside Model Selection opens a Nucleo-backed inline model search box that Esc clears back to the normal list UX. Chat Style switches immediately between `chat` (current bubble-style transcript), `agentic` (Codex-like activity rows), and `minimal` (jcode-like compact rows). Assistant output renders Markdown in every chat style, including visually distinct H1/H2/H3 headings, emphasis, styled links with visible URL fallbacks plus OSC8/Ctrl-click open support where terminals allow it, lists, colored task-list status markers, block quotes, labelled code-block boxes with line numbers and Syntect/bat-backed syntax coloring for many common languages, image placeholders that Ctrl-click open externally, and wrapped box-grid tables with alignment; markdown fences that only wrap a table are unwrapped so LLM showcase tables render as tables. Collapse Mode cycles thinking and tool display through Full, Truncate, and Collapse. The composer expands as drafts grow, remains editable while a prompt is running, and tiny terminals get a safe fallback message.

OpenRouter model names are cached at `~/.oino/openrouter-models.json`. The app loads that cache immediately, refreshes the full model list in the background on an interval, and uses each model's supported parameters to limit available thinking levels. Model identifiers use the single `provider:model-id` format, for example `openrouter:xai/glm-5.1`. Thinking `Off` is sent to OpenRouter explicitly as reasoning `none` with reasoning excluded, rather than relying on provider defaults. User-selected settings persist at `~/.oino/settings.json`; `OINO_MODEL` remains an environment override for the startup model. Sessions persist as one JSONL file per session under `~/.oino/sessions/<uuid>.jsonl`; the first line is the session header, later lines are append-only entries, and non-interactive continuation uses `oino --session <uuid> <message-or-command>`. A blank startup session is kept in memory and is not written to disk just because you open `/sessions`.

Oino now owns an explicit resource convention instead of silently reading Pi, Claude, or generic agent paths. On launch it creates visible defaults without overwriting user edits: `~/.oino/SYSTEM.md`, `~/.oino/settings.json`, `~/.oino/skills/`, `<project>/.oino/AGENT.md`, `<project>/.oino/prompts/`, and `<project>/.oino/skills/`. The global `SYSTEM.md` is loaded first, project `AGENT.md` is loaded after it, and available skills are advertised by name/description so the agent can load full `SKILL.md` files on demand. Prompt templates are single Markdown files under `<project>/.oino/prompts/` and expand as slash commands such as `/review`; skills use `skills/<name>/SKILL.md` and can be invoked as `/skill:<name> [args]`.

The command palette labels resource types explicitly: `[SYS]` for built-in commands, `[PROMPT]` for prompt templates, and `[SKILL]` for skills. Use `/prompts` and `/skills` to browse resources with fuzzy search, `/reload` to rescan `SYSTEM.md`, `AGENT.md`, prompts, and skills, `/P:<query>` to search only prompt commands, and `/S:<query>` to search only skill commands.

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

The first shell supports token-by-token transcript updates for provider text/thinking deltas, Markdown-rendered assistant output, local coding tool calls, persisted JSONL sessions, non-interactive `--session <uuid>` continuation, Oino-owned resource files, prompt templates, skills, and `/new`/`/sessions`/`/settings`/`/prompts`/`/skills`/`/reload`/`/model`/`/thinking` commands. It does not yet include `/login`, MCP, dynamic plugins/packages, memory DB, migration/import commands, or permissions UI.
