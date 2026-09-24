# P3: Providers as WASM Extensions

Goal: every model provider is a WASM component implementing the `provider` interface
(P2). Core keeps no wire formats. `crates/oino-provider-openrouter` and
`ProviderRuntimeProtocol` are deleted at the end of this phase.

## Components (guest workspace `extensions/`)

| Crate | Serves | Wire API |
| --- | --- | --- |
| `oino-provider-kit` (library, not a component) | shared SSE parser, usage normalization, cache-hint helpers, catalog types, tool-call JSON accumulation | n/a |
| `provider-openai-compat` | OpenRouter, Fireworks, DeepSeek, Kimi, Groq, Cerebras, vLLM/llama.cpp, generic — **and every declarative `open_ai_chat_completions` provider contribution (incl. OmniRoute), instantiated once per provider id with injected config** | Chat Completions |
| `provider-openai-responses` | OpenAI API, Azure OpenAI | Responses |
| `provider-anthropic` | Anthropic, Claude-compatible gateways | Messages |

No separate `router` component: declarative provider contributions (manifest
`providers[].runtime.protocol = open_ai_chat_completions`, e.g.
`extensions/built-in/router`) are served by `provider-openai-compat`. The
artifact is bundled in `oino-app` (`include_bytes!`); host-resolved config
(base_url, headers, auth spec) is passed via `init`, the api key via
`host.secret("api_key")`, and network permissions are synthesized from the
resolved URLs.

Google/Vertex, Bedrock, Codex WebSocket and Mistral are follow-ups, not P3.

Each component owns: catalog, auth shaping (headers from `host.secret`), request
building, SSE parsing, usage and cost normalization, thinking signature and encrypted
reasoning replay, and cache hints. OAuth flows that need a browser stay host-owned in
`oino-auth`; the component asks for the resulting token through `host.secret`.

## Catalog (`list-models` result)

```jsonc
{
  "provider": "anthropic",
  "models": [{
    "id": "claude-opus-5.5",
    "name": "Claude Opus 5.5",
    "context_window": 1000000,
    "max_output": 64000,
    "thinking_levels": ["off","low","medium","high","xhigh","max"],
    "input": ["text","image"],
    "cost": { "input": 0.0, "output": 0.0, "cache_read": 0.0, "cache_write_short": 0.0, "cache_write_long": 0.0 },
    "prompt_cache": { "short_secs": 300, "long_secs": 3600 },   // absent = not eligible for warming
    "compat": {
      "supports_mid_convo_system": true,
      "supports_mid_convo_tools": true,
      "session_affinity": "openai" | "openrouter" | "none",
      "cache_control_format": "anthropic" | null,
      "supports_long_cache_retention": true
    }
  }]
}
```

The host caches catalogs under `~/.oino/cache/models/<provider>.json` and lets users
override fields in `~/.oino/settings.json` `modelOverrides`, as pi does.

## Cache-hint rules (port of pi `packages/ai/src/api/*`)

Every adapter must be **deterministic**: same `StreamRequest`, same request bytes.
Tools in `current_tools` order; JSON objects with stable key order; no request-time
timestamps or random ids in the body.

- **Anthropic Messages** (`anthropic-messages.ts`):
  - `cache_control` on the last system block, the last tool definition, and the last
    user or assistant content block.
  - `ttl: "1h"` when `cache_retention = Long` and supported; none when retention is
    `None`.
  - Mid-convo system messages become system blocks in place when
    `supports_mid_convo_system`. `toolsAdded` after the first message use native
    `tool_addition` blocks (deferred), so tools added late stay out of the cached prefix.
  - Replay thinking blocks with `signature`.
- **OpenAI Responses** (`openai-responses*.ts`, `openai-prompt-cache.ts`):
  - `prompt_cache_key = clamp64(session_id)`, plus `session_id` and
    `x-client-request-id` headers.
  - Long retention: `prompt_cache_retention: "24h"`, or `prompt_cache_options.ttl: "30m"`
    for models that require it (catalog flag).
  - Late tools use message-anchored `additional_tools` where supported.
  - Replay `provider_data` (encrypted reasoning, item ids).
- **Chat Completions compat** (`openai-completions.ts`):
  - `prompt_cache_key` when the model supports it.
  - Session-affinity headers per `compat.session_affinity`.
  - `cache_control_format: "anthropic"` adds breakpoints like the Anthropic rules (for
    OpenRouter `anthropic/*`, Qwen).
  - DeepSeek `prompt_cache_hit_tokens` and Kimi top-level `cached_tokens` map to
    `cache_read_tokens`.
  - Omit `finish_reason`-dependent logic when `compat.supports_finish_reason = false`.
- **Usage invariant** for every adapter: `input_tokens` excludes cached tokens, so
  `input + cache_read + cache_write` is the prompt size. Cost is computed from the catalog
  unless the provider reports `cost`.

## Router (OmniRoute) port

The OmniRoute provider is a **declarative** contribution served by the bundled
`provider-openai-compat` component — no dedicated component exists (see the
table above). Host-owned concerns stay native in `oino-app`: Docker/process
bootstrap, onboarding UI, and base_url/api-key resolution
(`resolved_extension_runtime_base_url`, `ProviderRuntimeSecret`), which are
injected into the component at `init`/`host.secret` time.

## Testing

- Recorded SSE fixtures per adapter under `extensions/<crate>/fixtures/*.sse`, with the
  expected `assistant-event` sequence and expected request body JSON (golden files).
- Host-side replay server (`oino-wasm-host` test util, `127.0.0.1:0`) serves fixtures;
  tests allow only that host in the manifest.
- Determinism test per adapter: build the request twice from the same `StreamRequest`
  and assert byte equality; build turn N and N+1 and assert N's body is a prefix of N+1
  up to the last message boundary (for non-collapsing adapters).
- Cache-hint golden tests for each rule above.
- No real provider calls in CI.

## Cutover

1. Ship components next to the native provider behind a setting
   (`providers.runtime = "wasm" | "native"`, default `native`).
2. Flip default to `wasm` once fixtures, determinism and a manual smoke run against the
   router pass.
3. Delete `oino-provider-openrouter`, `ProviderRuntimeProtocol`, and the setting.

## Acceptance

- All components build for `wasm32-wasip2`, install through the extension manager,
  and appear in model selection.
- Golden and determinism tests pass.
- Manual smoke run (user-run, real keys): a 10-turn tool-using session through the
  router and through Anthropic shows cache hit >= 90% after turn 2 in `/usage`.
