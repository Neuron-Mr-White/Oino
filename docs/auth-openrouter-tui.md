# Auth, OpenRouter, and TUI Shell

This guide explains the first real conversation path and the user-facing controls that sit on top of it. For adjacent guides, see [commands](commands.md), [transcript rendering](transcript-rendering.md), [sessions](sessions.md), [resources](resources.md), [themes](theme-system/README.md), and [extensions](extension-kernel/user-guide.md).

```bash
OPENROUTER_API_KEY=sk-or-... mise run dev
```

Default model: `openrouter:openai/gpt-4o-mini`.

The conversation path is:

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

The app wires Oino's default tool set into the harness on startup:

- `read` — read text files with offset/limit and truncation notices; image files are detected but currently returned as text notes because the first OpenRouter adapter does not yet serialize image tool results.
- `bash` — execute shell commands in the startup working directory with optional timeout and truncated stdout/stderr output.
- `edit` — exact, unique, non-overlapping text replacements in one file.
- `write` — create or overwrite files, creating parent directories automatically.

## TUI shell

Optional env vars:

- `OINO_MODEL` — startup model in `provider:model-id` form, for example `openrouter:xai/glm-5.1`; overrides the persisted startup model when set.
- `OINO_OPENROUTER_REFERER` — optional OpenRouter attribution referer.
- `OINO_OPENROUTER_TITLE` — optional OpenRouter attribution title.

Everyday controls:

| Control | Behavior |
|---|---|
| `/help` | Open current shortcuts and commands. `/` inside Help searches the help text. |
| `Enter` | Submit non-empty input. While a response is running, send the current input as steering. |
| `Ctrl-J`, `Alt-Enter`, `Shift-Enter` | Insert a newline. `Shift-Enter` depends on terminal enhanced-key reporting. |
| `/` at the start of input | Open command suggestions. Arrows choose, Tab inserts/completes, Enter runs, Esc dismisses. See the [commands guide](commands.md). |
| `@` | Fuzzy-search project file paths. |
| `/prompt:<name>`, `/skill:<name>` | Include explicit resources. See the [resources guide](resources.md). |
| `Ctrl-O e` | Expand a collapsed paste block or prompt reference before sending. |
| `Ctrl-O t` | Focus the transcript for navigation; Esc returns to the composer. See the [transcript guide](transcript-rendering.md). |
| `Ctrl-O q` | Open the send panel for steering history, queue, and drafts. |
| `Ctrl-O s` or `/settings` | Open settings. |
| `Esc` | Close overlays/search, close focused extension surfaces, or stop a running response. It does not quit Oino. |
| `Ctrl-C` twice | Quit Oino. |

Useful command paths:

```text
/model openrouter:xai/glm-5.1
/thinking high
/settings model openrouter:xai/glm-5.1
/settings collapse thinking truncate
/settings chat-style agentic
/settings keymaps
/settings theme
```

Model identifiers use the single `provider:model-id` form. OpenRouter models load from `~/.oino/openrouter-models.json` first, then refresh in the background. Thinking levels are limited by the selected model's OpenRouter `supported_parameters`.

Selected user settings persist to `~/.oino/settings.json`. Sessions persist under `~/.oino/sessions`; see the [sessions guide](sessions.md) for `/new`, `/sessions`, `/title`, and `oino --session <uuid>` workflows. For chat styles, Markdown rendering, `/inspect`, and HTML export, see the [transcript guide](transcript-rendering.md).

## Boundary rule

Do not add provider JSON, API keys, or terminal state to `oino-agent-loop`. The loop consumes only typed `AssistantStreamEvent`s and emits typed `AgentEvent`s.
