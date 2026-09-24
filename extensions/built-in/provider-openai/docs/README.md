# Oino OpenAI Provider

Declarative `openai` provider backed by the bundled
`provider-openai-responses` WASM component (a port of pi's `openai-responses`
API adapter).

- Base URL: `https://api.openai.com/v1` (override: `OPENAI_BASE_URL` or
  extension config `base_url`).
- Auth: `OPENAI_API_KEY` environment variable (sent as `Authorization: Bearer`).
- Requires `providers.runtime = "wasm"`; under the native runtime the
  provider is skipped with a readiness diagnostic.

The component carries pi's generated OpenAI catalog verbatim and replays
pi's Responses request shape (developer role, prompt cache key/retention,
reasoning effort, encrypted reasoning replay, additional tools).
