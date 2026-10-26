// pi 1.4.0 oracle for crates/oino-models golden tests.
// Usage: node generate.mjs [out-dir]   (default: crates/oino-models/tests/fixtures)
// PI_CODING_AGENT_PKG overrides the pi-coding-agent package root.
import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const here = dirname(fileURLToPath(import.meta.url));
const PI = process.env.PI_CODING_AGENT_PKG ??
  join(homedir(), ".local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent");
const AI = join(PI, "node_modules/@earendil-works/pi-ai/dist");
const load = (path) => import(pathToFileURL(path).href);

const resolver = await load(join(PI, "dist/core/model-resolver.js"));
const configValue = await load(join(PI, "dist/core/resolve-config-value.js"));
const { ModelConfig } = await load(join(PI, "dist/core/model-config.js"));
const composer = await load(join(PI, "dist/core/provider-composer.js"));
const virtual = await load(join(PI, "dist/core/virtual-models.js"));
const { envApiKeyAuth } = await load(join(AI, "auth/helpers.js"));

const out = resolve(process.argv[2] ?? join(here, "../../../../crates/oino-models/tests/fixtures"));
mkdirSync(out, { recursive: true });
const work = mkdtempSync(join(process.env.OINO_TEST_TMP ?? out, ".oracle-"));
const write = (name, value) => writeFileSync(join(out, name), `${JSON.stringify(value, null, 2)}\n`);

// ---------------------------------------------------------------------------------------------
// Resolver: a fixed catalog.
const model = (provider, id, extra = {}) => ({
  id, name: extra.name ?? id, api: extra.api ?? "openai-completions", provider,
  baseUrl: `https://${provider}.example/v1`, reasoning: extra.reasoning ?? false, input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 128000, maxTokens: 16384,
  ...(extra.thinkingLevelMap ? { thinkingLevelMap: extra.thinkingLevelMap } : {}),
});
const catalog = [
  model("anthropic", "claude-sonnet-4-5", { name: "Claude Sonnet 4.5", reasoning: true, api: "anthropic-messages" }),
  model("anthropic", "claude-sonnet-4-5-20250929", { name: "Claude Sonnet 4.5 (2025-09-29)", reasoning: true, api: "anthropic-messages" }),
  model("anthropic", "claude-opus-4-8", { name: "Claude Opus 4.8", reasoning: true, api: "anthropic-messages" }),
  model("anthropic", "claude-haiku-4-5-20251001", { name: "Claude Haiku 4.5", api: "anthropic-messages" }),
  model("openai", "gpt-5.5", { name: "GPT-5.5", reasoning: true, api: "openai-responses" }),
  model("openai", "gpt-5.5-mini", { name: "GPT-5.5 mini", reasoning: true, api: "openai-responses" }),
  model("openai", "gpt-4o-latest", { name: "GPT-4o" }),
  model("openai", "gpt-4o-20240513", { name: "GPT-4o 2024-05-13" }),
  model("openrouter", "anthropic/claude-sonnet-4-5", { name: "Anthropic: Claude Sonnet 4.5", reasoning: true }),
  model("openrouter", "qwen/qwen3-coder:exacto", { name: "Qwen3 Coder Exacto" }),
  model("openrouter", "openrouter/free", { name: "Free router" }),
  model("openrouter", "gpt-5.5", { name: "OpenAI: GPT-5.5", reasoning: true }),
  model("openrouter", "zai/glm-5", { name: "Z.ai: GLM 5" }),
  model("vercel-ai-gateway", "zai/glm-5", { name: "GLM 5" }),
  model("zai", "glm-5", { name: "GLM-5", reasoning: true }),
  model("groq", "bracketed-model[1m]", { name: "Bracketed 1M" }),
  model("groq", "Llama-3.3-70B", { name: "Llama 3.3 70B" }),
  model("local", "a-b", { name: "dash" }),
  model("local", "a_b", { name: "underscore" }),
  model("local", "a.b", { name: "dot" }),
  model("local", "aB", { name: "upper" }),
];

const key = (m) => `${m.provider}/${m.id}`;
const summarize = (r) => {
  if (r.status === "resolved") return { status: r.status, model: key(r.model), thinkingLevel: r.thinkingLevel ?? null, warning: r.warning ?? null };
  if (r.status === "ambiguous") return { status: r.status, candidates: r.candidates.map(key), usableCount: r.usableCount, thinkingLevel: r.thinkingLevel ?? null, warning: r.warning ?? null };
  return {
    status: r.status,
    target: r.target ? {
      provider: r.target.provider, providerModels: r.target.providerModels.map(key), modelText: r.target.modelText,
      modelId: r.target.modelId, thinkingLevel: r.target.thinkingLevel ?? null,
    } : null,
  };
};

const texts = [
  "claude-sonnet-4-5", "CLAUDE-SONNET-4-5", "sonnet", "sonnet:high", "sonnet:bogus", "claude", "opus", "haiku",
  "gpt-5.5", "gpt-5.5:low", "gpt-4o", "gpt", "openai/gpt-5.5", "openrouter/gpt-5.5", "anthropic/sonnet",
  "anthropic/claude-sonnet-4-5:high", "openrouter/anthropic/claude-sonnet-4-5", "anthropic/claude-sonnet-4-5",
  "qwen/qwen3-coder:exacto", "qwen3-coder:exacto:high", "openrouter/free", "zai/glm-5", "glm-5", "glm",
  "bracketed-model[1m]", "llama", "a", "dash", "nothing", "anthropic/nothing", "anthropic/nothing:high",
  "  gpt-5.5  ", "", "x:", "glm-5:off", "local/a", "Anthropic/Claude-Opus-4-8", "unknown/gpt-5.5",
];
const usableSets = [[], ["anthropic"], ["openrouter"], ["openai", "openrouter"], ["zai"], ["vercel-ai-gateway", "zai"]];
const referenceCases = [];
for (const text of texts) {
  for (const thinkingSuffix of ["none", "strict", "lenient"]) {
    for (const fuzzy of [false, true]) {
      for (const usable of usableSets) {
        const request = { text, thinkingSuffix, fuzzy };
        const result = resolver.resolveModelReference(request, catalog, (p) => usable.includes(p));
        referenceCases.push({ request: { ...request, usable }, result: summarize(result) });
      }
    }
  }
}
for (const [text, provider] of [["sonnet", "anthropic"], ["gpt-5.5", "openrouter"], ["custom-id:high", "openai"], ["claude-sonnet-4-5", "OPENROUTER"], ["openai/gpt-x", "openai"]]) {
  for (const thinkingSuffix of ["none", "strict"]) {
    const request = { text, provider, thinkingSuffix, fuzzy: true };
    referenceCases.push({ request: { ...request, usable: [] }, result: summarize(resolver.resolveModelReference(request, catalog, () => false)) });
  }
}

const scopePatterns = [
  ["sonnet"], ["*sonnet*"], ["anthropic/*"], ["anthropic/*:high"], ["gpt-5.5"], ["gpt-5.5:xhigh"], ["glm-5"],
  ["zai/glm-5"], ["bracketed-model[1m]"], ["groq/*"], ["*:low"], ["nothing", "claude-opus-4-8"], ["sonnet:bogus"],
  ["claude-*", "anthropic/claude-opus-4-8:max"], ["openai/gpt-?.?"], ["OPENAI/*"], ["{openai,zai}/*"], ["gpt-4o"],
  ["*/claude*"], ["local/a?b"], ["a"],
];
const scopeCases = scopePatterns.map((patterns) => {
  const r = resolver.resolveModelScopeFromModels(patterns, catalog);
  return {
    patterns,
    scopedModels: r.scopedModels.map((s) => ({ model: key(s.model), thinkingLevel: s.thinkingLevel ?? null })),
    diagnostics: r.diagnostics.map((d) => ({ code: d.code, message: d.message, pattern: d.pattern })),
  };
});

const runtimeFor = (authed, models = catalog) => ({
  getModels: () => models,
  hasConfiguredAuth: (p) => authed.includes(p),
});
const cliInputs = [
  { cliModel: "sonnet" }, { cliModel: "gpt-5.5" }, { cliModel: "gpt-5.5", authed: ["openai"] },
  { cliModel: "gpt-5.5", authed: ["openai", "openrouter"] }, { cliModel: "gpt-5.5", inlineApiKey: true },
  { cliModel: "gpt-5.5", cliProvider: "openrouter" }, { cliModel: "x", cliProvider: "nope" },
  { cliModel: "anthropic/claude-new:high" }, { cliModel: "anthropic/claude-new:high", cliThinking: "low" },
  { cliModel: "anthropic/claude-new", cliThinking: "off" }, { cliModel: "nothing" },
  { cliModel: "openai/custom-thing" }, { cliModel: "custom", cliProvider: "openai" }, { cliModel: "sonnet:high" },
  { cliModel: "sonnet:bogus" }, { cliModel: "zai/glm-5" }, { cliModel: "zai/glm-5", authed: ["zai"] },
  { cliModel: "glm-5", authed: ["vercel-ai-gateway", "zai"] }, { cliModel: "openrouter/anthropic/claude-sonnet-4-5" },
  {}, { cliModel: "a", emptyCatalog: true },
];
const cliCases = cliInputs.map((input) => {
  const { authed = [], emptyCatalog, ...rest } = input;
  const r = resolver.resolveCliModel({ ...rest, modelRuntime: runtimeFor(authed, emptyCatalog ? [] : catalog) });
  return { input, model: r.model ?? null, thinkingLevel: r.thinkingLevel ?? null, warning: r.warning ?? null, error: r.error ?? null };
});

const collationIds = ["a-b", "a_b", "ab", "a.b", "aB", "Ab", "a1", "a-1", "a10", "a9", "A", "a", "gpt-5.5", "gpt-5", "gpt-5.5-mini",
  "claude-sonnet-4-5", "claude-sonnet-4", "claude-sonnet-4-5-20250929", "z", "Z", "a/b", "a:b", "a b", "a~b", "a+b"];
const collation = [...collationIds].sort((a, b) => a.localeCompare(b));

write("resolver.json", { catalog, referenceCases, scopeCases, cliCases, collation });

// ---------------------------------------------------------------------------------------------
// resolve-config-value
const env = { ORACLE_A: "alpha", ORACLE_B: "beta", ORACLE_EMPTY: "" };
for (const [k, v] of Object.entries(env)) process.env[k] = v;
const configs = [
  "literal", "$ORACLE_A", "${ORACLE_A}", "x-$ORACLE_A-${ORACLE_B}", "$$ORACLE_A", "$!bang", "$ORACLE_MISSING",
  "$ORACLE_EMPTY", "${not valid}", "${ORACLE_A", "trailing$", "$1abc", "${ORACLE_MISSING}${ORACLE_OTHER}",
  "pre${ORACLE_B}post", "$ORACLE_A$ORACLE_B", "a$$b$!c", "!printf ' cmd \\n'", "!exit 3", "!true",
];
const configValueCases = configs.map((config) => {
  let strict;
  try { strict = { ok: configValue.resolveConfigValueOrThrow(config, "API key") }; } catch (e) { strict = { error: e.message }; }
  return {
    config,
    resolved: configValue.resolveConfigValue(config) ?? null,
    envVarName: configValue.getConfigValueEnvVarName(config) ?? null,
    envVarNames: configValue.getConfigValueEnvVarNames(config),
    configured: configValue.isConfigValueConfigured(config),
    strict,
  };
});
const explicitCases = [["$ORACLE_A", { ORACLE_A: "explicit" }], ["$ORACLE_A", { ORACLE_A: "" }], ["$ORACLE_X", { ORACLE_X: "x" }]]
  .map(([config, explicit]) => ({ config, explicit, resolved: configValue.resolveConfigValue(config, explicit) ?? null }));
const headers = { "X-A": "$ORACLE_A", "X-Missing": "$ORACLE_MISSING", "X-Lit": "lit", "X-Empty": "$ORACLE_EMPTY" };
let strictHeaders;
try { strictHeaders = { ok: configValue.resolveHeadersOrThrow(headers, "provider \"p\"") }; } catch (e) { strictHeaders = { error: e.message }; }
write("config-value.json", { env, cases: configValueCases, explicitCases, headers, resolvedHeaders: configValue.resolveHeaders(headers) ?? null, strictHeaders });

// ---------------------------------------------------------------------------------------------
// models.json schema diagnostics and parsing defaults
const modelsJsonInputs = {
  valid: { providers: { ollama: { baseUrl: "http://localhost:11434/v1", api: "openai-completions", apiKey: "ollama", models: [{ id: "qwen2.5-coder:7b" }] } } },
  notObject: [],
  noProviders: {},
  providersArray: { providers: [] },
  badBaseUrl: { providers: { a: { baseUrl: 5 } } },
  missingId: { providers: { a: { models: [{ name: "x" }] } } },
  requiredAndType: { providers: { a: { models: [{ name: 5 }] } } },
  emptyId: { providers: { a: { models: [{ id: "" }] } } },
  zeroContext: { providers: { a: { models: [{ id: "m", contextWindow: 0 }] } } },
  badInput: { providers: { a: { models: [{ id: "m", input: ["audio"] }] } } },
  badOauth: { providers: { a: { oauth: "x" } } },
  extraFields: { providers: { a: { foo: 1, baseUrl: "x" } }, other: 2 },
  multi: { providers: { a: { baseUrl: 5, apiKey: true, models: [{ id: 1 }] } } },
  partialCost: { providers: { a: { models: [{ id: "m", cost: { input: 1 } }] } } },
  headerType: { providers: { a: { headers: { x: 1 } } } },
  thinkingMap: { providers: { a: { models: [{ id: "m", thinkingLevelMap: { high: 5, foo: "x" } }] } } },
  overrideCost: { providers: { a: { modelOverrides: { m: { cost: { input: "x" } } } } } },
  providerNull: { providers: { a: null } },
  compatType: { providers: { a: { compat: 5 } } },
  integers: { providers: { a: { models: [{ id: "m", inputLimits: { maxRequestBytes: 1.5, images: { resize: { jpegQuality: 101, maxWidth: 0 } } } }] } } },
  modelsObject: { providers: { a: { models: {} } } },
  promptCache: { providers: { a: { models: [{ id: "m", promptCache: { short: 0, foo: 1 } }] } } },
  tiers: { providers: { a: { models: [{ id: "m", cost: { input: 1, output: 1, cacheRead: 1, cacheWrite: 1, tiers: [{ input: 1 }] } }] } } },
  samplingByLevel: { providers: { a: { models: [{ id: "m", samplingParamsByThinkingLevel: { high: 5, foo: {} } }] } } },
  schemaField: { $schema: 5, providers: {} },
  authHeaderType: { providers: { a: { authHeader: "yes" } } },
  overridesArray: { providers: { a: { modelOverrides: [] } } },
  reasoningType: { providers: { a: { models: [{ id: "m", reasoning: "yes", maxTokens: -1 }] } } },
  numberString: { providers: { a: { models: [{ id: "m", contextWindow: "5" }] } } },
  samplingArray: { providers: { a: { models: [{ id: "m", samplingParams: [] }] } } },
  emptyName: { providers: { a: { name: "" } } },
  twoProviders: { providers: { a: { baseUrl: 1 }, b: { baseUrl: 2 } } },
  requiredFirst: { providers: { a: { models: [{ baseUrl: 1 }] } } },
  slashKey: { providers: { "a/b~c": { baseUrl: 1 } } },
};
const modelsJsonCases = [];
for (const [name, value] of Object.entries(modelsJsonInputs)) {
  const file = join(work, "models.json");
  const content = JSON.stringify(value);
  writeFileSync(file, content);
  const config = await ModelConfig.load(file);
  modelsJsonCases.push({ name, content, error: config.getError()?.replaceAll(file, "<FILE>") ?? null, providerIds: config.getProviderIds() });
}
{
  const file = join(work, "models.json");
  const content = "\uFEFF{ // comment\n \"providers\": {\"a\": {\"baseUrl\": \"http://x // not a comment\",},},}";
  writeFileSync(file, content);
  const config = await ModelConfig.load(file);
  modelsJsonCases.push({ name: "commentsAndTrailingCommas", content, error: config.getError() ?? null, providerIds: config.getProviderIds() });
}
write("models-json.json", { cases: modelsJsonCases });

// ---------------------------------------------------------------------------------------------
// Composition (registration + models.json + override + modelOverrides) and auth resolution.
const baseModels = [
  { ...model("p", "base-a", { api: "anthropic-messages", reasoning: true, thinkingLevelMap: { high: "high", xhigh: "max" } }),
    headers: { "X-Model": "m" }, compat: { supportsStore: true, openRouterRouting: { order: ["a"] } },
    inputLimits: { maxRequestBytes: 1000, images: { maxPerMessage: 2, resize: { maxWidth: 100 } } }, promptCache: { short: 300 },
    samplingParams: { temperature: 1 }, samplingParamsByThinkingLevel: { off: { top_p: 0.5 } } },
  model("p", "base-b", { api: "openai-completions" }),
];
const baseProvider = (models, envVars = ["P_KEY"], oauth = false) => ({
  id: "p", name: "Base P", baseUrl: "https://p.example/v1",
  auth: { ...(envVars ? { apiKey: envApiKeyAuth("P key", envVars) } : {}), ...(oauth ? { oauth: { name: "P OAuth", login: async () => ({}), refresh: async (c) => c, toAuth: async (c) => ({ apiKey: c.access }) } } : {}) },
  getModels: () => models, stream: () => { throw new Error("unused"); }, streamSimple: () => { throw new Error("unused"); },
});
const composeInputs = {
  baseOnlyBaseUrl: { modelsJson: { baseUrl: "https://proxy/v1" } },
  upsertAndOverride: { modelsJson: {
    models: [{ id: "new-model", contextWindow: 5000 }, { id: "base-b", name: "Replaced B", api: "openai-responses", cost: { input: 1, output: 2, cacheRead: 3, cacheWrite: 4 } }],
    modelOverrides: {
      "base-a": { name: "Renamed A", thinkingLevelMap: { high: null, low: "lo" }, cost: { output: 9 }, compat: { openRouterRouting: { only: ["x"] }, extra: 1 },
        inputLimits: { images: { resize: { maxHeight: 50 } } }, promptCache: { long: 3600 }, samplingParams: { top_k: 3 }, samplingParamsByThinkingLevel: { off: { min_p: 0.1 }, high: { top_k: 20 } }, contextWindow: 999, maxTokens: 77, reasoning: false, input: ["text", "image"] },
      "new-model": { maxTokens: 1234 },
      missing: { name: "ignored" },
    },
  } },
  providerCompat: { modelsJson: { compat: { supportsStore: false, chatTemplateKwargs: { a: 1 } }, models: [{ id: "c", compat: { chatTemplateKwargs: { b: 2 } } }] } },
  emptyEntry: { modelsJson: {} },
  oauthNoBaseUrl: { modelsJson: { oauth: "radius" } },
  noApiCustom: { noBase: true, modelsJson: { baseUrl: "https://x", models: [{ id: "m" }] } },
  noBaseUrlCustom: { noBase: true, modelsJson: { api: "openai-completions", models: [{ id: "m" }] } },
  customProvider: { noBase: true, modelsJson: { baseUrl: "https://ollama/v1", api: "openai-completions", apiKey: "ollama", models: [{ id: "qwen", reasoning: true, input: ["text", "image"] }, { id: "llama", api: "anthropic-messages", baseUrl: "https://other" }] } },
  extensionBaseUrl: { extension: { baseUrl: "https://ext/v1" } },
  extensionModels: { extension: { api: "openai-completions", models: [{ id: "ext-1", name: "Ext 1", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 100 }] } },
  extensionAndModelsJson: { modelsJson: { baseUrl: "https://proxy/v1", modelOverrides: { "ext-1": { name: "Overridden ext" } } }, extension: { models: [{ id: "ext-1", name: "Ext 1", reasoning: false, input: ["text"], cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 100 }] } },
};
const composeCases = [];
for (const [name, input] of Object.entries(composeInputs)) {
  const file = join(work, "models.json");
  writeFileSync(file, JSON.stringify({ providers: input.modelsJson ? { p: input.modelsJson } : {} }));
  const config = await ModelConfig.load(file);
  if (config.getError()) throw new Error(`${name}: ${config.getError()}`);
  let result;
  try {
    const provider = composer.composeModelProvider("p", input.noBase ? undefined : baseProvider(baseModels), config, input.extension);
    result = { models: provider.getAllModels() };
  } catch (e) { result = { error: e.message }; }
  composeCases.push({ name, base: input.noBase ? null : baseModels, modelsJson: input.modelsJson ?? null, extension: input.extension ?? null, ...result });
}

// Auth: check + resolve of the composed api-key method.
const authInputs = {
  envOnly: { env: { P_KEY: "env-key" } },
  envSecond: { envVars: ["P_FIRST", "P_KEY"], env: { P_KEY: "env-key" } },
  nothing: { env: {} },
  storedWins: { env: { P_KEY: "env-key" }, credential: { type: "api_key", key: "stored" } },
  storedNoKeyFallsBackToEnv: { env: { P_KEY: "env-key" }, credential: { type: "api_key" } },
  modelsJsonKeyBeatsEnv: { env: { P_KEY: "env-key" }, modelsJson: { apiKey: "literal-key" } },
  modelsJsonEnvKey: { env: { MY_KEY: "mine" }, modelsJson: { apiKey: "$MY_KEY" } },
  modelsJsonMissingEnv: { env: { P_KEY: "env-key" }, modelsJson: { apiKey: "${MISSING_KEY}" } },
  authHeader: { env: { P_KEY: "env-key" }, modelsJson: { authHeader: true, headers: { "X-Org": "$ORG", "X-Lit": "lit" } }, envExtra: { ORG: "org-1" } },
  authHeaderNoKey: { noBase: true, modelsJson: { baseUrl: "https://x", api: "openai-completions", authHeader: true, models: [{ id: "m" }] } },
  headerMissingEnv: { env: { P_KEY: "env-key" }, modelsJson: { headers: { "X-Org": "$ORG_MISSING" } } },
  customNoKey: { noBase: true, modelsJson: { baseUrl: "https://x", api: "openai-completions", models: [{ id: "m" }] } },
  customLiteral: { noBase: true, modelsJson: { baseUrl: "https://x", api: "openai-completions", apiKey: "lit", models: [{ id: "m" }] } },
  extensionKeyBeatsModelsJson: { modelsJson: { apiKey: "mj" }, extension: { apiKey: "ext" } },
  oauthOnly: { envVars: null, oauth: true, env: {} },
};
const authCases = [];
for (const [name, input] of Object.entries(authInputs)) {
  const file = join(work, "models.json");
  writeFileSync(file, JSON.stringify({ providers: input.modelsJson ? { p: input.modelsJson } : {} }));
  const config = await ModelConfig.load(file);
  const envMap = { ...(input.env ?? {}), ...(input.envExtra ?? {}) };
  const ctx = { env: async (n) => envMap[n] || undefined, fileExists: async () => false };
  for (const n of Object.keys(envMap)) process.env[n] = envMap[n];
  const base = input.noBase ? undefined : baseProvider(baseModels, input.envVars === undefined ? ["P_KEY"] : input.envVars, input.oauth);
  const provider = config.getProvider("p") || input.extension
    ? composer.composeModelProvider("p", base, config, input.extension)
    : base;
  const apiKey = provider.auth.apiKey;
  const signal = new AbortController().signal;
  const credential = input.credential;
  let check = null;
  let resolved = null;
  let error = null;
  if (apiKey) {
    try {
      check = apiKey.check ? await apiKey.check({ ctx, credential, signal }) ?? null : null;
      if (!apiKey.check) {
        const r = await apiKey.resolve({ ctx, credential, signal });
        check = r ? { type: "api_key", source: r.source } : null;
      }
    } catch (e) { check = { error: e.message }; }
    try {
      const r = await apiKey.resolve({ ctx, credential, signal });
      resolved = r ? { apiKey: r.auth.apiKey ?? null, headers: r.auth.headers ?? null, source: r.source } : null;
    } catch (e) { error = e.message; }
  }
  for (const n of Object.keys(envMap)) delete process.env[n];
  authCases.push({ name, input, hasApiKeyMethod: Boolean(apiKey), hasOAuth: Boolean(provider.auth.oauth), check, resolved, error });
}
write("compose.json", { composeCases, authCases });

// ---------------------------------------------------------------------------------------------
// Virtual models
const definitions = [
  { provider: "router", id: "auto", name: "Auto", thinkingLevels: ["low", "high"] },
  { provider: "router", id: "plain", name: "Plain" },
  { provider: "router", id: "limits", name: "Limits", thinkingLevels: ["off", "max"], contextWindow: 1000, maxTokens: 10, input: ["text"] },
];
write("virtual-models.json", { api: virtual.VIRTUAL_MODEL_API, stateEntry: virtual.VIRTUAL_MODEL_STATE_ENTRY, models: definitions.map((d) => ({ definition: d, model: virtual.createVirtualModel(d) })) });

rmSync(work, { recursive: true, force: true });
console.log(`wrote fixtures to ${out}`);
