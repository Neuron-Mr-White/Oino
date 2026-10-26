# Providers and credentials

Providers are WASM components (D14), bundled into `oino`:

| Component | API (`model.api`) | Providers |
|---|---|---|
| `oino.anthropic` | `anthropic-messages` | `anthropic` (API key or Claude Pro/Max OAuth token), `minimax`, `minimax-cn` |
| `oino.openai-responses` | `openai-responses` | `openai`, `xai`, `meta` |
| `oino.openai-compat` | `openai-completions` | `openrouter`, `groq`, `deepseek`, `together`, `fireworks`, `cerebras`, `huggingface`, `nvidia`, `moonshotai`, `kimi-coding`, `qwen-token-plan*`, `xiaomi*`, `zai`, `opencode`, `baseten`, `ant-ling`, `vercel-ai-gateway`, `ollama`, `lmstudio`, `mistral`, `deepinfra`, `chutes`, `perplexity` |
| `oino.router` | `oino-router` | `oino` (Oino Router; Claude models stream over Anthropic Messages, the rest over chat completions) |

Core dispatches every request by the **model's** `api` to the component that declares it, so
a provider may list models that use another wire (for example Vercel AI Gateway's Claude
models).

## Credentials

Resolution order per provider (pi semantics):

1. `--api-key <key>` (with `--model`) for this run;
2. `~/.oino/auth.json`;
3. the provider's environment variables.

`auth.json` uses pi's format and is written with mode `0600`:

```json
{
  "anthropic": { "type": "api_key", "key": "sk-ant-..." },
  "openrouter": { "type": "api_key", "key": "sk-or-..." },
  "oino": { "type": "api_key", "key": "..." }
}
```

Environment variables (first match wins):

| Provider | Variables |
|---|---|
| anthropic | `ANTHROPIC_AUTH_TOKEN`, `ANTHROPIC_OAUTH_TOKEN`, `ANTHROPIC_API_KEY` |
| openai | `OPENAI_API_KEY` |
| xai / meta | `XAI_API_KEY` / `META_API_KEY` |
| openrouter | `OPENROUTER_API_KEY` |
| groq, deepseek, together, fireworks, cerebras, mistral, deepinfra, chutes, perplexity, nvidia | `<NAME>_API_KEY` |
| huggingface | `HF_TOKEN` |
| moonshotai / kimi-coding | `MOONSHOT_API_KEY` / `KIMI_API_KEY` |
| zai | `ZAI_API_KEY` |
| minimax | `MINIMAX_API_KEY` |
| vercel-ai-gateway | `AI_GATEWAY_API_KEY` |
| ollama, lmstudio | none (local servers on `localhost:11434` / `localhost:1234`) |

The full list is in each component's `catalog.json` (`extensions/providers/*/catalog.json`,
generated from pi-ai 1.4.0's model data).

`/login [provider]` and `/logout [provider]` (TUI, desktop Settings → Models & Providers, RPC
`_oino/login`) run the provider's OAuth flow and store the credential in `auth.json`:
Anthropic (Claude Pro/Max), GitHub Copilot, OpenAI Codex and OpenAI (Sign in with ChatGPT).
The flows are tested against recorded fixtures; live accounts have not been tried in this alpha.

## Models

`oino --list-models [search]` shows every model with credentials. Catalogs are static (from
pi) except for `models: "dynamic"` providers (Ollama, LM Studio, Mistral, DeepInfra, Chutes,
Perplexity, the Oino Router), which are listed at startup when credentials exist (keyless
local servers are probed with a 5 s timeout). `--offline` / `OINO_OFFLINE=1` skips that.

Custom providers and model overrides go into `models.json` ([models.md](models.md)).

## Not available yet

WebSocket transports and Anthropic workload-identity federation are not ported; Bedrock
reads env keys, bearer tokens and static `~/.aws` profiles only; Vertex reads
`authorized_user` / `service_account` ADC files. Details: `extensions/providers/README.md` and
[providers deviations](dev/v1/deviations/providers-v1.md).
