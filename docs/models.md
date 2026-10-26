# Models and `models.json`

Model selection follows pi 1.4.0:

- `--model <pattern>` accepts `provider/id`, a bare id, a fuzzy pattern, and an optional
  `:<thinking>` suffix (`--model sonnet:high`). `--provider` narrows the search.
- `--models a,b,c` sets the scope cycled with `ctrl+p` (globs like `anthropic/*` work).
- Without `--model`, Oino uses `defaultProvider` / `defaultModel` from settings, then the first
  available model.
- `--thinking off|minimal|low|medium|high|xhigh|max` sets the thinking level; it is clamped to
  what the model supports.

## `models.json`

`~/.oino/models.json` adds providers and
models or overrides built-in ones, with pi's schema:

```json
{
  "providers": {
    "my-gateway": {
      "baseUrl": "https://llm.example.com/v1",
      "api": "openai-completions",
      "apiKey": "MY_GATEWAY_KEY",
      "authHeader": true,
      "models": [
        { "id": "qwen3-coder", "name": "Qwen3 Coder", "contextWindow": 262144, "maxTokens": 32768,
          "reasoning": true, "input": ["text"], "cost": { "input": 0.3, "output": 1.2 } }
      ]
    },
    "openrouter": { "baseUrl": "https://my-proxy.example.com/api/v1" }
  }
}
```

- `api` must be implemented by a loaded component: `openai-completions`, `openai-responses`,
  `anthropic-messages` or `oino-router`.
- `apiKey` is a literal, an environment variable name, or `!command` (pi semantics).
- `authHeader: true` sends `Authorization: Bearer <key>`.
- A provider entry with only `baseUrl`/`headers`/`models` overrides an existing provider.

## Virtual models

Extensions can register virtual models whose route is chosen per request (pi virtual models).
The footer shows the routed model next to the selected one.
