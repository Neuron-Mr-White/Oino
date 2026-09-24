# Oino Anthropic Provider

Declarative `anthropic` provider backed by the bundled `provider-anthropic`
WASM component (a port of pi's `anthropic-messages` API adapter).

- Base URL: `https://api.anthropic.com` (override: `ANTHROPIC_BASE_URL` or
  extension config `base_url`).
- Auth: `ANTHROPIC_API_KEY` environment variable (sent as `x-api-key` +
  `anthropic-version: 2023-06-1` headers).
- Requires `providers.runtime = "wasm"`; under the native runtime the
  provider is skipped with a readiness diagnostic.

The component carries pi's generated Anthropic model catalog verbatim and
replays pi's request shape (system blocks, cache-control, thinking config,
native mid-conversation tool changes).
