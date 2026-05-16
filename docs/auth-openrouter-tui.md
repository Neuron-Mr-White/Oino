# Auth, OpenRouter, and TUI Shell

This milestone adds the first real conversation path:

```text
Ratatui input -> Harness::prompt -> OpenRouterProvider -> Oino stream events -> message list
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

## TUI shell

Run:

```bash
OPENROUTER_API_KEY=sk-or-... cargo run -p oino-app --bin oino
```

Optional env vars:

- `OINO_MODEL` — OpenRouter model name, default `openai/gpt-4o-mini`.
- `OINO_OPENROUTER_REFERER` — optional OpenRouter attribution referer.
- `OINO_OPENROUTER_TITLE` — optional OpenRouter attribution title.

Controls:

- printable keys append to the input box
- Backspace edits input
- Enter submits non-empty input
- Esc or Ctrl-C exits

## Boundary rule

Do not add provider JSON, API keys, or terminal state to `oino-agent-loop`. The loop consumes only typed `AssistantStreamEvent`s and emits typed `AgentEvent`s.
