// Dump pi-ai 1.4.0 built-in chat providers and models into the bundled `catalog.json` of each
// provider component. Usage (repo root): node scripts/pi-oracle/providers-v1/catalog.mjs
//
// Every provider is registered by exactly one component. Its catalog keeps every chat model whose
// `api` one of the Oino provider components implements; core dispatches each model by `api`, so
// e.g. OpenRouter's Claude models (anthropic-messages) are served by `oino.anthropic`.
import { writeFileSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";

const PI = process.env.PI_AI_DIST ??
  join(homedir(), ".local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent/node_modules/@earendil-works/pi-ai/dist");
const OINO = process.env.OINO_REPO ?? ".";
const all = await import(join(PI, "providers/all.js"));
const envKeys = await import(join(PI, "env-api-keys.js"));

const APIS = new Set(["openai-completions", "openai-responses", "anthropic-messages"]);
// Per-account URL templates (azure, cloudflare-*) and providers with their own auth flows
// (github-copilot, openai-codex) are not bundled.
const EXCLUDED = new Set(["azure", "cloudflare-ai-gateway", "cloudflare-workers-ai", "github-copilot", "openai-codex"]);
const OWNER = { anthropic: "anthropic", minimax: "anthropic", "minimax-cn": "anthropic", openai: "openai-responses", xai: "openai-responses", meta: "openai-responses" };
const PRIMARY = { anthropic: "anthropic-messages", "openai-responses": "openai-responses", "openai-compat": "openai-completions" };

const out = { anthropic: [], "openai-responses": [], "openai-compat": [] };
for (const p of all.builtinProviders()) {
  if (EXCLUDED.has(p.id)) continue;
  const models = all.getBuiltinModels(p.id).filter((m) => (m.type ?? "chat") === "chat" && APIS.has(m.api));
  if (models.length === 0) continue;
  const owner = OWNER[p.id] ?? "openai-compat";
  const probe = new Proxy({}, { get: () => "x", has: () => true });
  out[owner].push({
    id: p.id,
    name: p.name,
    api: models.some((m) => m.api === PRIMARY[owner]) ? PRIMARY[owner] : models[0].api,
    baseUrl: p.baseUrl ?? models[0].baseUrl,
    authEnv: envKeys.findEnvKeys(p.id, probe) ?? [],
    models,
  });
}
// OpenAI-compatible profiles pi does not ship (pre-v1 Oino catalog); models come from `/models`.
out["openai-compat"].push(
  { id: "ollama", name: "Ollama", api: "openai-completions", baseUrl: "http://localhost:11434/v1", authEnv: [], local: true, models: "dynamic" },
  { id: "lmstudio", name: "LM Studio", api: "openai-completions", baseUrl: "http://localhost:1234/v1", authEnv: [], local: true, models: "dynamic" },
  { id: "mistral", name: "Mistral", api: "openai-completions", baseUrl: "https://api.mistral.ai/v1", authEnv: ["MISTRAL_API_KEY"], models: "dynamic" },
  { id: "deepinfra", name: "DeepInfra", api: "openai-completions", baseUrl: "https://api.deepinfra.com/v1/openai", authEnv: ["DEEPINFRA_API_KEY"], models: "dynamic" },
  { id: "chutes", name: "Chutes", api: "openai-completions", baseUrl: "https://llm.chutes.ai/v1", authEnv: ["CHUTES_API_KEY"], models: "dynamic" },
  { id: "perplexity", name: "Perplexity", api: "openai-completions", baseUrl: "https://api.perplexity.ai", authEnv: ["PERPLEXITY_API_KEY"], models: "dynamic" },
);
for (const [dir, providers] of Object.entries(out)) {
  const path = join(OINO, "extensions/providers", dir, "catalog.json");
  writeFileSync(path, JSON.stringify({ generatedFrom: "@earendil-works/pi-ai 1.4.0", providers }) + "\n");
  console.log(path, providers.map((p) => `${p.id}:${Array.isArray(p.models) ? p.models.length : p.models}`).join(" "));
}
