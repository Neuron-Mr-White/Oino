# Auth, OpenRouter, and TUI Shell

This milestone adds the first real conversation path:

```text
Ratatui composer -> Harness::prompt -> OpenRouterProvider -> Oino stream/tool events -> live chat transcript
```

## Auth

`oino-auth` stores credentials in `~/.oino/auth.json` and resolves OpenRouter using:

- provider id: `openrouter`
- auth file key: `openrouter`
- env var: `OPENROUTER_API_KEY`

Auth resolution order is runtime override, auth file, then environment variable. The JSON shape is:

```json
{
  "openrouter": { "type": "api_key", "key": "sk-or-REDACTED" }
}
```

## OpenRouter provider

`oino-provider-openrouter` implements `StreamProvider` and keeps provider-specific details outside the pure loop:

- base URL: `https://openrouter.ai/api/v1`
- endpoint: `/chat/completions`
- auth header: `Authorization: Bearer <key>`
- optional headers: `HTTP-Referer`, `X-OpenRouter-Title`
- streaming format: SSE `data:` frames ending in `[DONE]`

OpenRouter finish reasons are normalized to Oino stop reasons:

| OpenRouter | Oino |
| --- | --- |
| `stop` | `EndTurn` |
| `length` | `Length` |
| `tool_calls` | `ToolUse` |
| `error` | `Error` |
| unknown/null | `Unknown` |

Automated tests use serialization and SSE fixtures only; they do not call OpenRouter.

Provider text/thinking deltas are forwarded as they arrive, so the TUI can update the current assistant chat bubble before the full provider call completes. Tool calls are executed by the harness and rendered back into the transcript as tool result bubbles. The provider also exposes OpenRouter model listing and serializes Oino thinking levels as OpenRouter reasoning effort. `Off` is sent explicitly as `reasoning: { effort: "none", exclude: true }` instead of omitting reasoning, because OpenRouter may otherwise include reasoning tokens by default when a model decides to emit them.

## Built-in tools

The app wires Oino's first Pi-like default tool set into the harness on startup:

- `read` — read text files with offset/limit and truncation notices; image files are detected but currently returned as text notes because the first OpenRouter adapter does not yet serialize image tool results.
- `bash` — execute shell commands in the startup working directory with optional timeout and truncated stdout/stderr output.
- `edit` — exact, unique, non-overlapping text replacements in one file.
- `write` — create or overwrite files, creating parent directories automatically.

## TUI shell

Run:

```bash
OPENROUTER_API_KEY=sk-or-... mise run dev
```

Optional env vars:

- `OINO_MODEL` — OpenRouter model name, default `openai/gpt-4o-mini`; overrides the persisted startup model when set.
- `OINO_OPENROUTER_REFERER` — optional OpenRouter attribution referer.
- `OINO_OPENROUTER_TITLE` — optional OpenRouter attribution title.

Controls:

- printable keys append to the composer when input is enabled
- Backspace/Delete edit input; Ctrl-W deletes the previous word
- Ctrl-J or Alt-Enter inserts a newline; Shift-Enter also works on terminals that report enhanced key modifiers
- Up/Down navigates between lines in multi-line input
- the composer expands as the draft grows, up to the composer cap
- Enter submits non-empty input
- typing `/` as the first composer token opens command suggestions above the composer
- in command suggestions: arrows choose, Tab completes, Enter runs or advances the highlighted command path, Esc dismisses
- model identifiers use the single `provider:model-id` form, e.g. `openrouter:xai/glm-5.1`
- `/settings` or `Ctrl-O s` opens the reusable settings overlay
- command paths such as `/model openrouter:xai/glm-5.1`, `/thinking high`, `/settings model openrouter:xai/glm-5.1`, and `/settings collapse tool truncate` apply settings directly
- settings starts on a menu page with arrow-marked choices; Enter opens a dedicated Model Selection, Thinking Level, or Collapse Mode page
- in Model Selection: `/` opens model search, typing filters the model list, arrows move matching models, Esc clears search back to normal list UX
- in settings child pages: arrows/jk move, Enter applies, Esc/Left returns to the settings menu when not searching
- OpenRouter models load from `~/.oino/openrouter-models.json` first, then refresh in a background interval
- selected user settings persist to `~/.oino/settings.json`, currently including model, thinking level, and thinking/tool collapse modes
- sessions persist under `~/.oino/sessions`, and `oino --session <uuid> <message-or-command>` continues a session in non-interactive mode
- thinking levels are limited by the selected model's OpenRouter `supported_parameters`
- input pauses while a prompt is running
- Esc or Ctrl-C exits when no overlay is open

## Boundary rule

Do not add provider JSON, API keys, or terminal state to `oino-agent-loop`. The loop consumes only typed `AssistantStreamEvent`s and emits typed `AgentEvent`s.
