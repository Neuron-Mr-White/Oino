# Credentials and models

Oino uses pi's `auth.json` and `models.json` formats and pi-ai's provider ids, so most
entries copy over unchanged. Provider code lives in bundled WASM components; an entry only
works when a component implements its provider id (built-ins) or its `api` (custom
`models.json` providers).

## 1. Provider components (`oino --list-models` shows what is usable)

| Component | `api` | Built-in provider ids |
|---|---|---|
| `oino.anthropic` | `anthropic-messages` | `anthropic`, `minimax`, `minimax-cn` |
| `oino.openai-responses` | `openai-responses` | `openai`, `xai`, `meta` |
| `oino.openai-codex` | `openai-codex-responses` | `openai-codex` |
| `oino.azure` | `azure-openai-responses` | `azure` |
| `oino.google` | `google-generative-ai`, `google-vertex` | `google`, `google-vertex` |
| `oino.bedrock` | `bedrock-converse-stream` | `amazon-bedrock` |
| `oino.openai-compat` | `openai-completions` | `openrouter`, `groq`, `deepseek`, `together`, `fireworks`, `cerebras`, `huggingface`, `nvidia`, `moonshotai`, `moonshotai-cn`, `kimi-coding`, `qwen-token-plan*`, `xiaomi*`, `zai`, `zai-coding-cn`, `opencode`, `opencode-go`, `baseten`, `ant-ling`, `vercel-ai-gateway`, `ollama`, `lmstudio`, `mistral`, `deepinfra`, `chutes`, `perplexity`, `github-copilot`, `cloudflare-ai-gateway`, `cloudflare-workers-ai` |
| `oino.router` | `oino-router` | `oino` |

The live list: `printf '%s\n' '{"type":"_oino/auth_providers","id":"p"}' | oino --mode rpc`
(`{providers: [{id, name, oauth, configured, stored, env}]}`).

## 2. `auth.json` (`$PI_DIR/auth.json` → `$OINO/auth.json`, mode 0600)

Per entry `"<id>": {"type": "api_key", "key": …}` or `{"type": "oauth", "access", "refresh",
"expires", …}`:

| Entry | Action |
|---|---|
| `api_key` for a built-in id (§1) or for a provider defined in the migrated `models.json` | copy unchanged |
| `oauth` for `anthropic`, `github-copilot`, `openai-codex`, `openai` | copy unchanged (Oino refreshes it with the same flow); if a request fails later, `/login <id>` |
| `oauth` for any other id (`google-gemini-cli`, `google-antigravity`, ids added by pi TypeScript extensions such as `antigravity`, `devin`) | **not migrated**: Oino has no OAuth flow for it; suggest an API key provider instead |
| `api_key` for an unknown id with no `models.json` provider | **not migrated** (nothing would use it); list it |
| id already present in `$OINO/auth.json` | keep Oino's, record conflict |

Write with python, then `chmod 600 "$OINO/auth.json"`. Never echo key values.

## 3. `models.json` (`$PI_DIR/models.json` → `$OINO/models.json`)

Same schema (`{"providers": {"<id>": {baseUrl, api, apiKey, authHeader, headers, compat,
models: [...]}}}`).

| Provider entry | Action |
|---|---|
| `api` in §1's api column (or no `api`: an override of a built-in provider such as `{"openrouter": {"baseUrl": …}}`) | copy unchanged; `apiKey` keeps pi semantics (literal, env var name, `!command`) |
| `api` not implemented (`google-gemini-cli`, `mistral-conversations`, any custom api of a pi TypeScript extension) | **not migrated**; list provider id and api |
| `apiKey: "!<command>"` | copy, but warn that the command runs at Oino start; check the command still exists |
| per-model `compat` / `headers` / `cost` | copy unchanged (pi schema) |
| provider id already in `$OINO/models.json` | keep Oino's, record conflict |

Not migrated (pi- or extension-private caches): `$PI_DIR/models-store.json`,
`$PI_DIR/model-catalogs`, `$UNIPI/config/models-cache.json`,
`$UNIPI/config/image-models-cache.json`, `$PI_DIR/omniroute-bridge/` and other TypeScript
extension state (they regenerate or belong to code that does not run in Oino).

Verify: `oino --list-models | head -50` shows the providers; for each provider with
credentials, `oino --list-models <provider>/` is non-empty.
