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

`providers.runtime` in `~/.oino/settings.json` (or project `.oino/settings.json`,
project wins) selects how declarative extension providers are executed:

- `"wasm"` (default): bundled Wasmtime components, one instance per provider
  id, with
  host-resolved config and an `api_key` host secret:
  - `open_ai_chat_completions` → `provider-openai-compat`
  - `anthropic_messages` → `provider-anthropic`
  - `open_ai_responses` → `provider-openai-responses`

- `"native"` (fallback): the in-process OpenAI-compatible adapter
  (`open_ai_chat_completions` protocol only). The built-in
  `provider-anthropic` (`ANTHROPIC_API_KEY`, `api.anthropic.com`) and
  `provider-openai` (`OPENAI_API_KEY`, `api.openai.com/v1`) packages report a
  readiness diagnostic under `"native"`.

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
