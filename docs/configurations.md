# Configuration

Oino works with defaults, then saves user choices as you use it.

## Important locations

| Path | Purpose |
| --- | --- |
| `~/.oino/settings.json` | User settings. |
| `~/.oino/sessions/` | Saved sessions. |
| `~/.oino/model-catalogs/` | Cached model lists. |
| `~/.oino/extension-packages/` | Globally installed extension packages. |
| `~/.oino/SYSTEM.md` | Global instructions. |
| `.oino/AGENT.md` | Project instructions. |
| `.oino/prompts/` | Project prompt templates. |
| `.oino/skills/` | Project skills. |

## Useful environment variables

| Variable | Purpose |
| --- | --- |
| `OINO_MODEL` | Initial model, for example `router:openai/gpt-4.1`. |
| `OMNIROUTE_BASE_URL` | OmniRoute OpenAI-compatible base URL. Defaults to `http://localhost:20128/v1`. |
| `OMNIROUTE_API_KEY` | Optional API key if your OmniRoute endpoint requires one. |
| `OINO_HOME` | Override the home root used by install helper scripts. |
| `OINO_PREFIX` | Override where installers place the `oino` binary. |
| `OINO_REPO` | Override the repository cloned by installers. |
| `OINO_REF` | Ask installers to check out a specific branch, tag, or commit. |

Example:

```bash
OINO_MODEL=router:openai/gpt-4.1 oino
```

## Provider runtime

Declarative extension providers always run as bundled Wasmtime components —
one instance per provider id, with host-resolved config and an `api_key` host
secret:

- `open_ai_chat_completions` → `provider-openai-compat`
- `anthropic_messages` → `provider-anthropic`
- `open_ai_responses` → `provider-openai-responses`

(The native in-process adapter and the `providers.runtime` setting were
removed in the P3c cutover; provider behavior is frozen as goldens under
`crates/oino-app/tests/fixtures/openai_compat/`.)

## Headless extension flags

One-shot runs can drive `oino.budget` and `oino.presets` from the CLI (the
packages are force-enabled for the run even when disabled in settings):

- `--max-cost <usd>`, `--max-wall <secs>`, `--max-tool-errors <n>` — budget
  caps merged into the `oino.budget` init config.
- `--preset <name>` — select an `oino.presets` preset (`minimal`/`standard`/
  `thorough`) for the run.

## Debugging provider HTTP traffic

`OINO_DEBUG_PROVIDER_DIR=<dir>` makes the WASM host dump every provider
request body to `<dir>/<n>-request.json` and every streamed response to
`<dir>/<n>-response.sse`. Auth headers are redacted; files are numbered in
request order across all provider components.

## Oino Router provider

The built-in `oino.oino_router` package serves `oino:*` model ids from
`https://router.oino.dev` as a WASM component:

- `OINO_API_KEY` (fallback `OMNI_API_KEY`) — required, resolved via the
  standard extension secret lookup (extension config.json first, then env).
- `OINO_BASE_URL` — override the router base URL (granted through
  `hosts_from_config`, so a custom endpoint host is permitted).
- `OINO_FORMAT` — `auto` (default; claude-family → Anthropic Messages,
  everything else → OpenAI chat-completions), `openai`, or `anthropic`.
- `~/.oino/extensions/oino.oino_router/config.json` keys: `base_url`,
  `format`, `format_overrides` (map of endpoint id → `"openai"|"anthropic"`).

`config_env` in a manifest maps config keys to env vars (env wins over
`config.json`); `network.hosts_from_config` grants network access to the
host of a config URL key. Both are declared by `oino.oino_router`.

## Prompt-cache telemetry and warming

- `cacheWarming`: `"off"` | `"tools"` (default) | `"tools+idle"` — sends a
  1-output-token warming request before the provider-side prompt cache
  expires when a tool is still running (`tools`) or also when idle
  (`tools+idle`, at most 3 in a row). Requires the model's catalog to expose
  `prompt_cache` lifetimes and prices; warm requests are never added to the
  context and record a `cache_warm` usage entry. Extensions may veto a warm
  via the `cache_warming_decision` hook (`{"veto": true}`).
- `showCacheMissNotices`: `true`/`false` (default `false`) — surface a notice
  when a request's cache hit rate drops below 50% of the previous prompt,
  naming the first diverging prefix block. The last diagnosis is always
  visible via `/session` along with the last hit rate and next warming
  decision.

## Reloading

Use `/reload` after editing settings, prompts, skills, themes, or extension files. Use `/router fetch-models` when you want fresh model data from OmniRoute.
