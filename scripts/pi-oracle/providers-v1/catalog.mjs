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

const APIS = new Set([
  "openai-completions", "openai-responses", "anthropic-messages",
  // providers-more
  "azure-openai-responses", "openai-codex-responses", "bedrock-converse-stream", "google-generative-ai", "google-vertex",
]);
const EXCLUDED = new Set([]);
const OWNER = {
  anthropic: "anthropic", minimax: "anthropic", "minimax-cn": "anthropic",
  openai: "openai-responses", xai: "openai-responses", meta: "openai-responses",
  azure: "azure", "openai-codex": "openai-codex", "amazon-bedrock": "bedrock", google: "google", "google-vertex": "google",
};
const PRIMARY = {
  anthropic: "anthropic-messages", "openai-responses": "openai-responses", "openai-compat": "openai-completions",
  azure: "azure-openai-responses", "openai-codex": "openai-codex-responses", bedrock: "bedrock-converse-stream", google: "google-generative-ai",
};
// Providers whose component implements `oauth-login` (pi `auth.oauth`); Anthropic is implied.
const OAUTH = new Set(["github-copilot", "openai-codex", "openai"]);
// Credential markers for providers pi resolves from ambient credentials (pi `resolve`): core only
// knows API-key env vars, so the variables whose presence means "configured" are listed and the
// component ignores a marker value passed as the key.
const AUTH_ENV = {
  "amazon-bedrock": ["AWS_BEARER_TOKEN_BEDROCK", "AWS_PROFILE", "AWS_ACCESS_KEY_ID", "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "AWS_CONTAINER_CREDENTIALS_FULL_URI", "AWS_WEB_IDENTITY_TOKEN_FILE"],
  "google-vertex": ["GOOGLE_CLOUD_API_KEY", "GOOGLE_CLOUD_PROJECT", "GCLOUD_PROJECT"],
};

const out = { anthropic: [], "openai-responses": [], "openai-compat": [], azure: [], "openai-codex": [], bedrock: [], google: [] };
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
    authEnv: AUTH_ENV[p.id] ?? envKeys.findEnvKeys(p.id, probe) ?? [],
    ...(OAUTH.has(p.id) ? { oauth: true } : {}),
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
