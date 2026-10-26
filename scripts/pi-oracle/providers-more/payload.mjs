// pi oracle for providers-more: drives pi-ai 1.4.0 for the remaining wires (Google Generative
// AI, Google Vertex, Amazon Bedrock Converse, OpenAI Codex Responses, Azure OpenAI Responses)
// and the per-provider adaptations of the shared wires (GitHub Copilot, Cloudflare AI Gateway /
// Workers AI, Azure chat completions). Requests are captured from an injected `fetch` (or, for
// the SDKs that ignore it, a patched global `fetch` / Smithy HTTP handler) and answered with a
// canned SSE / AWS event stream / error body. pi's events (without `partial`) go into fixtures
// the Rust provider kit replays (`tests/pi_fixtures.rs`).
//
// Usage (repo root): node scripts/pi-oracle/providers-more/payload.mjs
// Output: extensions/sdk/oino-provider-kit/tests/fixtures/<api>/<case>.json with
//   {name, model, context, options, apiKey, status, sse | sseBase64, responseHeaders?,
//    request: {url, headers, body}, events, sigv4?}
// `model`/`options`/`apiKey` are the Oino-side inputs (catalog model, `options.env` instead of
// pi's resolved auth headers); pi was driven with what its auth layer would pass.
import { mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { Readable } from "node:stream";
import zlib from "node:zlib";

const NM = process.env.PI_NODE_MODULES ??
  join(homedir(), ".local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent/node_modules");
const PI = join(NM, "@earendil-works/pi-ai/dist");
const OINO = process.env.OINO_REPO ?? ".";

// Freeze the clock (SigV4 dates, message timestamps) before the SDKs load.
const RealDate = Date;
const FIXED = RealDate.UTC(2026, 9, 26, 12, 0, 0);
globalThis.Date = class extends RealDate {
  constructor(...args) {
    if (args.length === 0) super(FIXED);
    else super(...args);
  }
  static now() {
    return FIXED;
  }
};

const { getBuiltinModel } = await import(join(PI, "providers/all.js"));
const { streamSimple: google } = await import(join(PI, "api/google-generative-ai.js"));
const { streamSimple: vertex } = await import(join(PI, "api/google-vertex.js"));
const { streamSimple: bedrock } = await import(join(PI, "api/bedrock-converse-stream.js"));
const { streamSimple: codex } = await import(join(PI, "api/openai-codex-responses.js"));
const { streamSimple: azureResponses } = await import(join(PI, "api/azure-openai-responses.js"));
const { streamSimple: anthropic } = await import(join(PI, "api/anthropic-messages.js"));
const { streamSimple: responses } = await import(join(PI, "api/openai-responses.js"));
const { streamSimple: completions } = await import(join(PI, "api/openai-completions.js"));
const { resolveCloudflareModel } = await import(join(PI, "providers/cloudflare-stream.js"));
const { resolveAzureBaseUrl, resolveDeploymentName } = await import(join(PI, "api/azure-openai-config.js"));
const { GoogleAuth } = await import(join(NM, "google-auth-library/build/src/index.js"));
const { lazyStream } = await import(join(PI, "api/lazy.js"));
const smithy = await import(join(NM, "@smithy/node-http-handler/dist-cjs/index.js"));

const STREAMS = {
  "google-generative-ai": google,
  "google-vertex": vertex,
  "bedrock-converse-stream": bedrock,
  "openai-codex-responses": codex,
  "azure-openai-responses": azureResponses,
  "anthropic-messages": anthropic,
  "openai-responses": responses,
  "openai-completions": completions,
};

function model(provider, id) {
  const m = getBuiltinModel(provider, id);
  if (!m) throw new Error(`missing catalog model ${provider}/${id}`);
  return structuredClone(m);
}

// ---- capture ----------------------------------------------------------------------------------

let current = null; // {capture, response: {status, body, contentType, headers}}

function recordHeaders(h) {
  const headers = {};
  if (h instanceof Headers) h.forEach((v, k) => (headers[k.toLowerCase()] = v));
  else if (h) for (const [k, v] of Object.entries(h)) headers[k.toLowerCase()] = String(v);
  return headers;
}

function decodeBody(body, headers) {
  if (body === undefined || body === null) return undefined;
  let text;
  if (typeof body === "string") text = body;
  else if (headers["content-encoding"] === "zstd") text = zlib.zstdDecompressSync(Buffer.from(body)).toString();
  else text = Buffer.from(body).toString();
  try {
    return JSON.parse(text);
  } catch {
    return text;
  }
}

function respond() {
  const r = current.response;
  return new Response(r.body, { status: r.status, headers: { "content-type": r.contentType, ...(r.headers ?? {}) } });
}

async function captureFetch(input, init) {
  const url = typeof input === "string" ? input : (input.url ?? input.toString());
  const headers = recordHeaders(init?.headers ?? input.headers);
  current.capture.url = url;
  current.capture.headers = headers;
  current.capture.body = decodeBody(init?.body, headers);
  return respond();
}

// @google/genai ignores options.fetch: patch the global one.
globalThis.fetch = captureFetch;
// Vertex ADC: google-auth-library would hit the network; hand out a fixed token.
GoogleAuth.prototype.getRequestHeaders = async function () {
  return new Headers({ authorization: "Bearer ya29.adc-token" });
};
// Bedrock: the AWS SDK speaks through Smithy handlers.
for (const C of [smithy.NodeHttpHandler, smithy.NodeHttp2Handler]) {
  C.prototype.handle = async function (request) {
    const headers = recordHeaders(request.headers);
    const query = new URLSearchParams(request.query ?? {}).toString();
    current.capture.url = `${request.protocol}//${request.hostname}${request.path}${query ? `?${query}` : ""}`;
    current.capture.headers = headers;
    current.capture.body = decodeBody(request.body, headers);
    const r = current.response;
    return {
      response: {
        statusCode: r.status,
        reason: r.status === 200 ? "OK" : "Error",
        headers: { "content-type": r.contentType, "x-amzn-requestid": "req-1", ...(r.headers ?? {}) },
        body: Readable.from([Buffer.from(r.body)]),
      },
    };
  };
}

function normalize(events) {
  return events.map((e) => {
    const { partial: _p, ...rest } = e;
    const out = structuredClone(rest);
    for (const key of ["message", "error"]) if (out[key]) out[key].timestamp = 0;
    return out;
  });
}

/**
 * Runs one case. `oino` = what Oino passes ({model, options, apiKey}); `pi` = overrides for the
 * pi call (resolved model/auth headers) when they differ.
 */
function run(name, api, m, messages, opts = {}) {
  return () => runNow(name, api, m, messages, opts);
}

async function runNow(name, api, m, messages, opts) {
  const { sse = "", status = 200, options = {}, apiKey = "test-key", contentType, responseHeaders, pi = {}, binary = false, stream } = opts;
  current = {
    capture: {},
    response: {
      status,
      body: sse,
      contentType: contentType ?? (status === 200 ? (binary ? "application/vnd.amazon.eventstream" : "text/event-stream") : "application/json"),
      headers: responseHeaders,
    },
  };
  const events = [];
  const piOptions = { apiKey, sessionId: "sess-1234", fetch: captureFetch, ...options, ...(pi.options ?? {}) };
  if ("apiKey" in pi) piOptions.apiKey = pi.apiKey;
  if (piOptions.apiKey === null || piOptions.apiKey === undefined) delete piOptions.apiKey;
  if (api.startsWith("google")) delete piOptions.fetch;
  const fn = stream ?? STREAMS[api];
  // pi's provider layer runs streams through lazyStream: setup throws become an error event.
  const target = pi.model ?? m;
  for await (const event of lazyStream(target, async () => fn(target, { messages }, piOptions))) events.push(event);
  const fixture = {
    name,
    model: m,
    context: { messages },
    options: { sessionId: "sess-1234", ...options },
    apiKey: apiKey ?? null,
    status,
    ...(binary ? { sseBase64: Buffer.from(sse).toString("base64") } : { sse: typeof sse === "string" ? sse : Buffer.from(sse).toString() }),
    ...(responseHeaders ? { responseHeaders } : {}),
    request: structuredClone(current.capture),
    events: normalize(events),
  };
  return fixture;
}

// ---- shared contexts --------------------------------------------------------------------------

const lookup = { name: "lookup", description: "Look up a value", parameters: { type: "object", properties: { q: { type: "string" } }, required: ["q"] } };
const writeFile = { name: "write_file", description: "Write a file", parameters: { type: "object", properties: { path: { type: "string" }, data: { type: "string" } }, required: ["path"] } };
const readTool = { name: "read", description: "Read a file", parameters: { type: "object", properties: { path: { type: "string" } }, required: ["path"] } };
const strictTool = { ...lookup, name: "strict_lookup", constrainedSampling: { type: "json_schema", strict: "prefer" } };
const ZERO = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };
const sys = (content, tools, extra = {}) => ({ role: "system", content, ...(tools ? { toolsAdded: tools } : {}), timestamp: 0, ...extra });
const user = (content, ts = 1) => ({ role: "user", content, timestamp: ts });
const asst = (m, content, stopReason = "stop", extra = {}) => ({ role: "assistant", api: m.api, provider: m.provider, model: m.id, content, stopReason, usage: ZERO, timestamp: 2, ...extra });
const result = (id, name, content, isError = false) => ({ role: "toolResult", toolCallId: id, toolName: name, content, isError, timestamp: 3 });
const IMG = { type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" };

function toolsRoundtrip(m, id, extra = {}) {
  return [
    sys("Sys", [lookup, writeFile]),
    user("look up x"),
    asst(m, [{ type: "text", text: "Sure." }, { type: "toolCall", id, name: "lookup", arguments: { q: "x" }, ...extra }], "toolUse"),
    result(id, "lookup", [{ type: "text", text: "42" }]),
    user("thanks", 4),
  ];
}
function images(m, id) {
  return [
    sys("Sys", [readTool]),
    user([{ type: "text", text: "what is this" }, IMG]),
    asst(m, [{ type: "toolCall", id, name: "read", arguments: { path: "a.png" } }], "toolUse"),
    result(id, "read", [{ type: "text", text: "image:" }, IMG]),
    user([IMG], 4),
  ];
}
function errorImages(m, id) {
  return [
    sys("Sys", [readTool]),
    user("read it"),
    asst(m, [{ type: "toolCall", id, name: "read", arguments: { path: "a.png" } }], "toolUse"),
    result(id, "read", [{ type: "text", text: "failed" }, IMG], true),
  ];
}
function midConvo(m) {
  return [
    sys("Sys", [lookup], { sections: { env: "cwd=/a" } }),
    user("one"),
    asst(m, [{ type: "text", text: "ok" }]),
    sys("", [writeFile], { sections: { env: "cwd=/b" }, toolsRemoved: [{ name: "lookup" }], timestamp: 3 }),
    user("two", 4),
  ];
}

const cases = [];
const add = (api, thunk) => cases.push({ api, thunk });

// ---- Google (Generative AI + Vertex) ----------------------------------------------------------

const gSse = (chunks) => chunks.map((c) => `data: ${JSON.stringify(c)}\r\n\r\n`).join("");
const gUsage = (prompt, candidates, extra = {}) => ({ promptTokenCount: prompt, candidatesTokenCount: candidates, totalTokenCount: prompt + candidates + (extra.thoughtsTokenCount ?? 0), ...extra });
const gText = (text = "Hello", finish = "STOP", usage = gUsage(12, 5)) => gSse([
  { candidates: [{ content: { parts: [{ text: text.slice(0, 2) }], role: "model" }, index: 0 }], responseId: "resp-g1", modelVersion: "m" },
  { candidates: [{ content: { parts: [{ text: text.slice(2) }], role: "model" }, finishReason: finish, index: 0 }], usageMetadata: usage, responseId: "resp-g1" },
]);
const gThinking = gSse([
  { candidates: [{ content: { parts: [{ text: "ponder", thought: true, thoughtSignature: "c2lnMQ==" }], role: "model" } }], responseId: "resp-g2" },
  { candidates: [{ content: { parts: [{ text: "ing", thought: true }], role: "model" } }] },
  { candidates: [{ content: { parts: [{ text: "answer", thoughtSignature: "c2lnMg==" }], role: "model" } }] },
  { candidates: [{ content: { parts: [{ text: "" }], role: "model" }, finishReason: "STOP" }], usageMetadata: gUsage(20, 10, { thoughtsTokenCount: 30, cachedContentTokenCount: 8 }) },
]);
const gTool = gSse([
  { candidates: [{ content: { parts: [{ text: "Looking." }], role: "model" } }], responseId: "resp-g3" },
  { candidates: [{ content: { parts: [{ functionCall: { id: "fc-1", name: "lookup", args: { q: "x" } }, thoughtSignature: "dG9vbA==" }, { functionCall: { id: "fc-2", name: "write_file", args: { path: "a" } } }], role: "model" }, finishReason: "STOP" }], usageMetadata: gUsage(30, 9) },
]);
{
  const api = "google-generative-ai";
  const flash = model("google", "gemini-2.5-flash");
  const pro3 = model("google", "gemini-3.1-pro-preview");
  const lite = model("google", "gemini-2.5-flash-lite");
  const key = { apiKey: "gem-key" };
  add(api, run("a_system_user", api, flash, [sys("Sys"), user("hi")], { sse: gText(), ...key }));
  add(api, run("b_tools_roundtrip", api, pro3, toolsRoundtrip(pro3, "call|1"), { sse: gText(), ...key }));
  add(api, run("b_tools_roundtrip_v2", api, flash, toolsRoundtrip(flash, "call_1"), { sse: gText(), ...key }));
  add(api, run("c_thinking_budget", api, flash, [sys("Sys"), user("q")], { sse: gThinking, options: { reasoning: "medium" }, ...key }));
  add(api, run("c_thinking_budget_custom", api, lite, [sys("Sys"), user("q")], { sse: gThinking, options: { reasoning: "high", thinkingBudgets: { high: 4000 } }, ...key }));
  add(api, run("c_thinking_level", api, pro3, [sys("Sys"), user("q")], { sse: gThinking, options: { reasoning: "high" }, ...key }));
  add(api, run("c_thinking_off", api, pro3, [sys("Sys"), user("q")], { sse: gText(), ...key }));
  add(api, run("c_thinking_off_budget", api, flash, [sys("Sys"), user("q")], { sse: gText(), ...key }));
  add(api, run("d_replay_signatures", api, pro3, [
    sys("Sys", [lookup]), user("q"),
    asst(pro3, [{ type: "thinking", thinking: "t1", thinkingSignature: "c2lnMQ==" }, { type: "thinking", thinking: "", thinkingSignature: "c2lnMw==" }, { type: "text", text: "", textSignature: "c2lnNA==" }, { type: "text", text: "a1", textSignature: "bad sig" }, { type: "toolCall", id: "fc-9", name: "lookup", arguments: { q: "y" }, thoughtSignature: "dG9vbA==" }], "toolUse"),
    result("fc-9", "lookup", [{ type: "text", text: "1" }]),
    result("fc-9b", "lookup", [{ type: "text", text: "2" }], true),
    asst(model("anthropic", "claude-sonnet-5"), [{ type: "thinking", thinking: "foreign", thinkingSignature: "x" }, { type: "text", text: "foreign text" }]),
    user("again", 4),
  ], { sse: gText(), ...key }));
  add(api, run("e_midconvo_collapsed", api, flash, midConvo(flash), { sse: gText(), ...key }));
  add(api, run("f_image_multimodal_response", api, pro3, images(pro3, "img-1"), { sse: gText(), ...key }));
  add(api, run("f_image_separate_turn", api, flash, images(flash, "img-1"), { sse: gText(), ...key }));
  add(api, run("g_usage_cache", api, flash, [sys("Sys"), user("hi")], { sse: gText("ok", "STOP", gUsage(100, 7, { cachedContentTokenCount: 60, thoughtsTokenCount: 3 })), ...key }));
  add(api, run("h_error_400", api, flash, [sys("Sys"), user("hi")], { status: 400, sse: JSON.stringify({ error: { code: 400, message: "Invalid JSON payload", status: "INVALID_ARGUMENT" } }), ...key }));
  add(api, run("h_error_429", api, flash, [sys("Sys"), user("hi")], { status: 429, sse: JSON.stringify({ error: { code: 429, message: "Resource exhausted", status: "RESOURCE_EXHAUSTED" } }), ...key }));
  add(api, run("h_no_key", api, flash, [sys("Sys"), user("hi")], { sse: gText(), apiKey: null }));
  add(api, run("i_tool_stream", api, pro3, [sys("Sys", [lookup, writeFile, strictTool]), user("go")], { sse: gTool, ...key }));
  add(api, run("j_options", api, flash, [sys("Sys", [lookup]), user("hi")], { sse: gText(), options: { temperature: 0.4, maxTokens: 300, toolChoice: "any" }, ...key }));
  add(api, run("j_tool_choice_none", api, pro3, [sys("Sys", [lookup, strictTool]), user("hi")], { sse: gText(), options: { toolChoice: "none" }, ...key }));
  add(api, run("k_length", api, flash, [sys("Sys"), user("hi")], { sse: gText("Hello", "MAX_TOKENS"), ...key }));
  add(api, run("k_safety", api, flash, [sys("Sys"), user("hi")], { sse: gText("Hel", "SAFETY"), ...key }));
  add(api, run("l_no_finish", api, flash, [sys("Sys"), user("hi")], { sse: gSse([{ candidates: [{ content: { parts: [{ text: "x" }], role: "model" } }] }]), ...key }));
  add(api, run("m_headers", api, flash, [sys("Sys"), user("hi")], { sse: gText(), options: { headers: { "x-extra": "1" } }, ...key }));
}
{
  const api = "google-vertex";
  const flash = model("google-vertex", "gemini-2.5-flash");
  const pro3 = model("google-vertex", "gemini-3.1-pro-preview");
  const env = { GOOGLE_CLOUD_PROJECT: "my-proj", GOOGLE_CLOUD_LOCATION: "us-central1" };
  add(api, run("a_api_key", api, flash, [sys("Sys"), user("hi")], { sse: gText(), apiKey: "vertex-key" }));
  add(api, run("a_adc", api, flash, [sys("Sys"), user("hi")], { sse: gText(), apiKey: null, options: { env } }));
  add(api, run("a_adc_global", api, pro3, [sys("Sys"), user("hi")], { sse: gText(), apiKey: "gcp-vertex-credentials", options: { env: { ...env, GOOGLE_CLOUD_LOCATION: "global" } } }));
  add(api, run("a_custom_base_url", api, { ...flash, baseUrl: "https://proxy.example.com/v1" }, [sys("Sys"), user("hi")], { sse: gText(), apiKey: "vertex-key" }));
  add(api, run("a_missing_project", api, flash, [sys("Sys"), user("hi")], { sse: gText(), apiKey: null, options: { env: { GOOGLE_CLOUD_LOCATION: "us-central1" } } }));
  add(api, run("b_tools_roundtrip", api, pro3, toolsRoundtrip(pro3, "call_1"), { sse: gTool, apiKey: "vertex-key" }));
  add(api, run("c_thinking_budget", api, flash, [sys("Sys"), user("q")], { sse: gThinking, apiKey: "vertex-key", options: { reasoning: "low" } }));
  add(api, run("c_thinking_level", api, pro3, [sys("Sys"), user("q")], { sse: gThinking, apiKey: "vertex-key", options: { reasoning: "minimal" } }));
  add(api, run("h_error_403", api, flash, [sys("Sys"), user("hi")], { status: 403, sse: JSON.stringify({ error: { code: 403, message: "Permission denied", status: "PERMISSION_DENIED" } }), apiKey: "vertex-key" }));
  add(api, run("l_no_finish", api, flash, [sys("Sys"), user("hi")], { sse: gSse([{ candidates: [{ content: { parts: [{ text: "x" }], role: "model" } }] }]), apiKey: "vertex-key" }));
}

// ---- Amazon Bedrock ---------------------------------------------------------------------------

function esHeader(name, value) {
  const n = Buffer.from(name);
  const v = Buffer.from(value);
  const b = Buffer.alloc(1 + n.length + 1 + 2 + v.length);
  let o = 0;
  b[o++] = n.length;
  n.copy(b, o);
  o += n.length;
  b[o++] = 7;
  b.writeUInt16BE(v.length, o);
  o += 2;
  v.copy(b, o);
  return b;
}
function esFrame(type, payload, messageType = "event") {
  const typeHeader = messageType === "exception" ? ":exception-type" : ":event-type";
  const hs = Buffer.concat([esHeader(typeHeader, type), esHeader(":content-type", "application/json"), esHeader(":message-type", messageType)]);
  const p = Buffer.from(JSON.stringify(payload));
  const pre = Buffer.alloc(8);
  pre.writeUInt32BE(12 + hs.length + p.length + 4, 0);
  pre.writeUInt32BE(hs.length, 4);
  const pc = Buffer.alloc(4);
  pc.writeUInt32BE(zlib.crc32(pre), 0);
  const body = Buffer.concat([pre, pc, hs, p]);
  const mc = Buffer.alloc(4);
  mc.writeUInt32BE(zlib.crc32(body), 0);
  return Buffer.concat([body, mc]);
}
const es = (frames) => Buffer.concat(frames.map(([t, p, k]) => esFrame(t, p, k)));
const bUsage = (input, output, extra = {}) => ({ usage: { inputTokens: input, outputTokens: output, totalTokens: input + output, ...extra }, metrics: { latencyMs: 10 } });
const bText = (text = "Hello", stop = "end_turn", usage = bUsage(12, 5)) => es([
  ["messageStart", { role: "assistant", p: "abc" }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { text: text.slice(0, 3) } }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { text: text.slice(3) } }],
  ["contentBlockStop", { contentBlockIndex: 0 }],
  ["messageStop", { stopReason: stop }],
  ["metadata", usage],
]);
const bTool = es([
  ["messageStart", { role: "assistant" }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { text: "Looking." } }],
  ["contentBlockStop", { contentBlockIndex: 0 }],
  ["contentBlockStart", { contentBlockIndex: 1, start: { toolUse: { toolUseId: "tooluse_1", name: "lookup" } } }],
  ["contentBlockDelta", { contentBlockIndex: 1, delta: { toolUse: { input: "{\"q\":" } } }],
  ["contentBlockDelta", { contentBlockIndex: 1, delta: { toolUse: { input: "\"x\"}" } } }],
  ["contentBlockStop", { contentBlockIndex: 1 }],
  ["messageStop", { stopReason: "tool_use" }],
  ["metadata", bUsage(30, 9)],
]);
const bThinking = es([
  ["messageStart", { role: "assistant" }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { reasoningContent: { text: "ponder" } } }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { reasoningContent: { text: "ing" } } }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { reasoningContent: { signature: "sigA" } } }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { reasoningContent: { signature: "BC" } } }],
  ["contentBlockStop", { contentBlockIndex: 0 }],
  ["contentBlockDelta", { contentBlockIndex: 1, delta: { reasoningContent: { redactedContent: Buffer.from("opaque-1").toString("base64") } } }],
  ["contentBlockDelta", { contentBlockIndex: 1, delta: { reasoningContent: { redactedContent: Buffer.from("-2").toString("base64") } } }],
  ["contentBlockStop", { contentBlockIndex: 1 }],
  ["contentBlockDelta", { contentBlockIndex: 2, delta: { text: "answer" } }],
  ["contentBlockStop", { contentBlockIndex: 2 }],
  ["messageStop", { stopReason: "end_turn" }],
  ["metadata", bUsage(20, 40, { cacheReadInputTokens: 5, cacheWriteInputTokens: 7, cacheDetails: [{ ttl: "1h", inputTokens: 7 }] })],
]);
const bStreamError = es([
  ["messageStart", { role: "assistant" }],
  ["contentBlockDelta", { contentBlockIndex: 0, delta: { text: "par" } }],
  ["throttlingException", { message: "Too many tokens, please wait before trying again." }, "exception"],
]);
const SIG = { AWS_ACCESS_KEY_ID: "AKIDEXAMPLE", AWS_SECRET_ACCESS_KEY: "wJalrXUtnFEMI/K7MDENG+bPxRfiCYEXAMPLEKEY", AWS_REGION: "us-west-2" };
{
  const api = "bedrock-converse-stream";
  const sonnet = model("amazon-bedrock", "global.anthropic.claude-sonnet-4-6");
  const opus7 = model("amazon-bedrock", "anthropic.claude-opus-4-7");
  const haiku = model("amazon-bedrock", "anthropic.claude-haiku-4-5-20251001-v1:0");
  const nova = model("amazon-bedrock", "amazon.nova-2-lite-v1:0");
  const oss = getBuiltinModel("amazon-bedrock", "openai.gpt-oss-120b-1:0") ? model("amazon-bedrock", "openai.gpt-oss-120b-1:0") : null;
  const bearer = { apiKey: "bedrock-token", binary: true };
  add(api, run("a_bearer", api, sonnet, [sys("Sys"), user("hi")], { sse: bText(), ...bearer }));
  add(api, run("a_sigv4", api, nova, [sys("Sys"), user("hi")], { sse: bText(), apiKey: null, binary: true, options: { env: SIG } }));
  add(api, run("a_sigv4_session_token", api, sonnet, [sys("Sys"), user("hi")], { sse: bText(), apiKey: null, binary: true, options: { env: { ...SIG, AWS_SESSION_TOKEN: "session-tok", AWS_REGION: undefined, AWS_DEFAULT_REGION: "eu-west-1" } } }));
  add(api, run("a_custom_endpoint", api, { ...sonnet, baseUrl: "https://vpce-1.bedrock-runtime.example.com" }, [sys("Sys"), user("hi")], { sse: bText(), ...bearer }));
  add(api, run("a_bearer_env", api, sonnet, [sys("Sys"), user("hi")], { sse: bText(), apiKey: null, binary: true, options: { env: { AWS_BEARER_TOKEN_BEDROCK: "env-token", AWS_REGION: "ap-south-1" } } }));
  add(api, run("b_tools_roundtrip", api, sonnet, toolsRoundtrip(sonnet, "toolu|01"), { sse: bText(), ...bearer }));
  add(api, run("b_tools_strict_choice", api, nova, [sys("Sys", [lookup, strictTool]), user("hi")], { sse: bText(), ...bearer, options: { toolChoice: { type: "tool", name: "lookup" } } }));
  add(api, run("b_tool_choice_none", api, nova, [sys("Sys", [lookup]), user("hi")], { sse: bText(), ...bearer, options: { toolChoice: "none" } }));
  add(api, run("c_thinking_adaptive", api, sonnet, [sys("Sys"), user("q")], { sse: bThinking, ...bearer, options: { reasoning: "high" } }));
  add(api, run("c_thinking_block_binding", api, opus7, [sys("Sys"), user("q")], { sse: bThinking, ...bearer, options: { reasoning: "xhigh" } }));
  add(api, run("c_thinking_budget", api, haiku, [sys("Sys"), user("q")], { sse: bThinking, ...bearer, options: { reasoning: "medium", maxTokens: 6000 } }));
  add(api, run("c_thinking_nova", api, nova, [sys("Sys"), user("q")], { sse: bText(), ...bearer, options: { reasoning: "low" } }));
  if (oss) add(api, run("c_thinking_gpt_oss", api, oss, [sys("Sys"), user("q")], { sse: bText(), ...bearer, options: { reasoning: "xhigh" } }));
  add(api, run("d_replay_thinking", api, sonnet, [
    sys("Sys"), user("q"),
    asst(sonnet, [{ type: "thinking", thinking: "t1", thinkingSignature: "sig0" }, { type: "thinking", thinking: "[Reasoning redacted]", thinkingSignature: Buffer.from("opaque").toString("base64"), redacted: true }, { type: "thinking", thinking: "unsigned" }, { type: "text", text: "a1" }, { type: "text", text: "  " }]),
    user("again", 4),
  ], { sse: bText(), ...bearer }));
  add(api, run("d_replay_thinking_nova", api, nova, [sys("Sys"), user("q"), asst(nova, [{ type: "thinking", thinking: "t1", thinkingSignature: "sig0" }, { type: "text", text: "a1" }]), user("", 4)], { sse: bText(), ...bearer }));
  add(api, run("e_retention_none", api, sonnet, [sys("Sys", [lookup]), user("hi")], { sse: bText(), ...bearer, options: { cacheRetention: "none" } }));
  add(api, run("e_retention_long", api, sonnet, [sys("Sys", [lookup]), user("hi")], { sse: bText(), ...bearer, options: { cacheRetention: "long" } }));
  add(api, run("e_midconvo_collapsed", api, sonnet, midConvo(sonnet), { sse: bText(), ...bearer }));
  add(api, run("f_image", api, sonnet, images(sonnet, "toolu_img"), { sse: bText(), ...bearer }));
  add(api, run("f_error_tool_image", api, sonnet, errorImages(sonnet, "toolu_err"), { sse: bText(), ...bearer }));
  add(api, run("f_error_tool_image_nova", api, nova, errorImages(nova, "toolu_err"), { sse: bText(), ...bearer }));
  add(api, run("g_usage_cache", api, sonnet, [sys("Sys"), user("hi")], { sse: bThinking, ...bearer }));
  add(api, run("h_error_400", api, sonnet, [sys("Sys"), user("hi")], { status: 400, sse: JSON.stringify({ message: "The provided model identifier is invalid." }), responseHeaders: { "x-amzn-errortype": "ValidationException:http://internal.amazon.com/coral/com.amazon.bedrock/" }, ...bearer }));
  add(api, run("h_error_429", api, sonnet, [sys("Sys"), user("hi")], { status: 429, sse: JSON.stringify({ message: "Too many requests, please wait before trying again." }), responseHeaders: { "x-amzn-errortype": "ThrottlingException:http://internal.amazon.com/coral/com.amazon.bedrock/" }, ...bearer }));
  add(api, run("h_error_403_plain", api, sonnet, [sys("Sys"), user("hi")], { status: 403, sse: "<html>Forbidden</html>", contentType: "text/html", ...bearer }));
  add(api, run("i_tool_stream", api, sonnet, [sys("Sys", [lookup]), user("look up x")], { sse: bTool, ...bearer }));
  add(api, run("j_options", api, nova, [sys("Sys"), user("hi")], { sse: bText(), ...bearer, options: { temperature: 0.2, maxTokens: 512, headers: { "x-custom": "1", "x-amz-date": "nope" } } }));
  add(api, run("k_length", api, sonnet, [sys("Sys"), user("hi")], { sse: bText("Hello", "max_tokens"), ...bearer }));
  add(api, run("k_guardrail", api, sonnet, [sys("Sys"), user("hi")], { sse: bText("Hello", "guardrail_intervened"), ...bearer }));
  add(api, run("k_stream_exception", api, sonnet, [sys("Sys"), user("hi")], { sse: bStreamError, ...bearer }));
  add(api, run("l_no_stop", api, sonnet, [sys("Sys"), user("hi")], { sse: es([["messageStart", { role: "assistant" }], ["contentBlockDelta", { contentBlockIndex: 0, delta: { text: "x" } }]]), ...bearer }));
}

// ---- OpenAI Codex Responses -------------------------------------------------------------------

const named = (events) => events.map(([event, data]) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`).join("");
const respSse = (events) => named(events.map((e) => [e.type, e]));
const rCreated = (id) => ({ type: "response.created", response: { id, status: "in_progress", output: [] } });
const rMsg = (oi, id, text, phase) => {
  const item = { type: "message", id, role: "assistant", status: "completed", content: [{ type: "output_text", text, annotations: [] }], ...(phase ? { phase } : {}) };
  return [
    { type: "response.output_item.added", output_index: oi, item: { ...item, status: "in_progress", content: [] } },
    { type: "response.content_part.added", output_index: oi, item_id: id, content_index: 0, part: { type: "output_text", text: "", annotations: [] } },
    { type: "response.output_text.delta", output_index: oi, item_id: id, content_index: 0, delta: text.slice(0, 2) },
    { type: "response.output_text.delta", output_index: oi, item_id: id, content_index: 0, delta: text.slice(2) },
    { type: "response.output_item.done", output_index: oi, item },
  ];
};
const rUsage = (input, output, cached = 0, reasoning = 0) => ({ input_tokens: input, input_tokens_details: { cached_tokens: cached }, output_tokens: output, output_tokens_details: { reasoning_tokens: reasoning }, total_tokens: input + output });
const rDone = (id, output, usage, type = "response.completed", extra = {}) => ({ type, response: { id, status: "completed", output, usage, ...extra } });
const rText = (text = "Hello", usage = rUsage(12, 5), doneType = "response.completed", extra = {}) => respSse([rCreated("resp_1"), ...rMsg(0, "msg_o1", text, "final_answer"), rDone("resp_1", [], usage, doneType, extra)]);
const fcItem = { type: "function_call", id: "fc_aaa", call_id: "call_01", name: "lookup", arguments: "{\"q\":\"x\"}", status: "completed" };
const rTool = respSse([
  rCreated("resp_2"),
  { type: "response.output_item.added", output_index: 0, item: { ...fcItem, arguments: "", status: "in_progress" } },
  { type: "response.function_call_arguments.delta", output_index: 0, item_id: "fc_aaa", delta: "{\"q\":" },
  { type: "response.function_call_arguments.delta", output_index: 0, item_id: "fc_aaa", delta: "\"x\"}" },
  { type: "response.function_call_arguments.done", output_index: 0, item_id: "fc_aaa", arguments: "{\"q\":\"x\"}" },
  { type: "response.output_item.done", output_index: 0, item: fcItem },
  rDone("resp_2", [fcItem], rUsage(30, 9)),
]);
const rsItem = { type: "reasoning", id: "rs_1", summary: [{ type: "summary_text", text: "pondering" }], encrypted_content: "enc_abc" };
const rThinking = respSse([
  rCreated("resp_3"),
  { type: "response.output_item.added", output_index: 0, item: { type: "reasoning", id: "rs_1", summary: [] } },
  { type: "response.reasoning_summary_text.delta", output_index: 0, item_id: "rs_1", summary_index: 0, delta: "ponder" },
  { type: "response.reasoning_summary_text.delta", output_index: 0, item_id: "rs_1", summary_index: 0, delta: "ing" },
  { type: "response.output_item.done", output_index: 0, item: rsItem },
  ...rMsg(1, "msg_o3", "answer"),
  rDone("resp_3", [rsItem], rUsage(20, 40, 0, 25)),
]);
const jwt = (claims) => `eyJhbGciOiJub25lIn0.${Buffer.from(JSON.stringify(claims)).toString("base64url")}.sig`;
const CODEX_TOKEN = jwt({ "https://api.openai.com/auth": { chatgpt_account_id: "acct-123" }, exp: 2000000000 });
{
  const api = "openai-codex-responses";
  const luna = model("openai-codex", "gpt-5.6-luna");
  const sol = model("openai-codex", "gpt-6-sol");
  const tok = { apiKey: CODEX_TOKEN, options: { transport: "sse" } };
  const withOpts = (extra) => ({ apiKey: CODEX_TOKEN, options: { transport: "sse", ...extra } });
  add(api, run("a_system_user", api, luna, [sys("Sys"), user("hi")], { sse: rText(), ...tok }));
  add(api, run("a_no_system", api, luna, [user("hi")], { sse: rText(), ...tok }));
  add(api, run("b_tools_roundtrip", api, luna, toolsRoundtrip(luna, "call_01|fc_aaa"), { sse: rText(), ...tok }));
  add(api, run("b_tools_strict", api, sol, [sys("Sys", [lookup, strictTool]), user("hi")], { sse: rText(), ...tok }));
  add(api, run("c_thinking", api, luna, [sys("Sys"), user("q"), asst(luna, [{ type: "thinking", thinking: "pondering", thinkingSignature: JSON.stringify(rsItem) }, { type: "text", text: "a1", textSignature: JSON.stringify({ v: 1, id: "msg_prev", phase: "final_answer" }) }]), user("again", 4)], { sse: rThinking, ...withOpts({ reasoning: "high" }) }));
  add(api, run("c_thinking_minimal_mapped", api, luna, [sys("Sys"), user("q")], { sse: rThinking, ...withOpts({ reasoning: "minimal" }) }));
  add(api, run("d_retention_none", api, luna, [sys("Sys"), user("hi")], { sse: rText(), ...withOpts({ cacheRetention: "none" }) }));
  add(api, run("e_midconvo_native", api, luna, midConvo(luna), { sse: rText() , ...tok }));
  add(api, run("f_image", api, luna, images(luna, "call_img|fc_img"), { sse: rText(), ...tok }));
  add(api, run("g_service_tier", api, luna, [sys("Sys"), user("hi")], { sse: rText("ok", rUsage(100, 7, 60), "response.done", { service_tier: "default" }), ...withOpts({ serviceTier: "priority", temperature: 0.5, toolChoice: "required" }) }));
  add(api, run("h_error_400", api, luna, [sys("Sys"), user("hi")], { status: 400, sse: JSON.stringify({ error: { message: "Unsupported parameter", type: "invalid_request_error" } }), ...tok }));
  add(api, run("h_error_429_usage", api, luna, [sys("Sys"), user("hi")], { status: 429, sse: JSON.stringify({ error: { type: "usage_limit_reached", message: "limit", plan_type: "PLUS" } }), ...tok }));
  add(api, run("h_error_plain", api, luna, [sys("Sys"), user("hi")], { status: 502, sse: "Bad gateway", contentType: "text/plain", ...tok }));
  add(api, run("h_bad_token", api, luna, [sys("Sys"), user("hi")], { sse: rText(), apiKey: "not-a-jwt", options: { transport: "sse" } }));
  add(api, run("i_tool_stream", api, luna, [sys("Sys", [lookup]), user("look up x")], { sse: rTool, ...tok }));
  add(api, run("k_failed", api, luna, [sys("Sys"), user("hi")], { sse: respSse([rCreated("resp_6"), { type: "response.failed", response: { id: "resp_6", status: "failed", error: { code: "server_error", message: "boom" } } }]), ...tok }));
  add(api, run("k_error_event", api, luna, [sys("Sys"), user("hi")], { sse: respSse([rCreated("resp_7"), { type: "error", code: "context_length_exceeded", message: "too long" }]), ...tok }));
  add(api, run("k_incomplete", api, luna, [sys("Sys"), user("hi")], { sse: respSse([rCreated("resp_8"), ...rMsg(0, "m8", "trunc"), { type: "response.incomplete", response: { id: "resp_8", status: "incomplete", incomplete_details: { reason: "max_output_tokens" }, output: [], usage: rUsage(5, 16) } }]), ...tok }));
  add(api, run("l_headers_override", api, { ...luna, baseUrl: "https://proxy.example.com/codex" }, [sys("Sys"), user("hi")], { sse: rText(), ...withOpts({ headers: { originator: "oino", "x-extra": "1" } }) }));
}

// ---- Azure OpenAI Responses -------------------------------------------------------------------
{
  const api = "azure-openai-responses";
  const g55 = model("azure", "gpt-5.5");
  const g41 = model("azure", "gpt-4.1");
  const env = { AZURE_OPENAI_RESOURCE_NAME: "my-resource" };
  const k = { apiKey: "azure-key" };
  add(api, run("a_resource_name", api, g41, [sys("Sys"), user("hi")], { sse: rText(), ...k, options: { env } }));
  add(api, run("a_base_url", api, g41, [sys("Sys"), user("hi")], { sse: rText(), ...k, options: { env: { AZURE_OPENAI_BASE_URL: "https://my-resource.cognitiveservices.azure.com/openai", AZURE_OPENAI_API_VERSION: "2025-04-01-preview" } } }));
  add(api, run("a_custom_host", api, g41, [sys("Sys"), user("hi")], { sse: rText(), ...k, options: { env: { AZURE_OPENAI_BASE_URL: "https://gateway.example.com/azure/" } } }));
  add(api, run("a_deployment_map", api, g55, [sys("Sys"), user("hi")], { sse: rText(), ...k, options: { env: { ...env, AZURE_OPENAI_DEPLOYMENT_NAME_MAP: "gpt-4.1=a, gpt-5.5 = prod-55 ,bad" } } }));
  add(api, run("a_no_endpoint", api, g41, [sys("Sys"), user("hi")], { sse: rText(), ...k }));
  add(api, run("b_tools_roundtrip", api, g41, toolsRoundtrip(g41, "call_01|fc_aaa"), { sse: rText(), ...k, options: { env } }));
  add(api, run("c_thinking", api, g55, [sys("Sys"), user("q")], { sse: rThinking, ...k, options: { env, reasoning: "high" } }));
  add(api, run("c_thinking_off", api, g55, [sys("Sys"), user("q")], { sse: rText(), ...k, options: { env } }));
  add(api, run("f_image", api, g41, images(g41, "call_img|fc_img"), { sse: rText(), ...k, options: { env } }));
  add(api, run("h_error_400", api, g41, [sys("Sys"), user("hi")], { status: 400, sse: JSON.stringify({ error: { message: "bad request", type: "invalid_request_error", code: null } }), ...k, options: { env } }));
  add(api, run("i_tool_stream", api, g41, [sys("Sys", [lookup]), user("look up x")], { sse: rTool, ...k, options: { env } }));
  add(api, run("j_options", api, g41, [sys("Sys"), user("hi")], { sse: rText(), ...k, options: { env, maxTokens: 8, temperature: 0.1, toolChoice: "auto", headers: { "x-extra": "1" } } }));
}

// ---- provider adaptations of the shared wires --------------------------------------------------

const aText = named([
  ["message_start", { type: "message_start", message: { id: "msg_1", model: "claude", usage: { input_tokens: 12, output_tokens: 0 } } }],
  ["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hello" } }],
  ["content_block_stop", { type: "content_block_stop", index: 0 }],
  ["message_delta", { type: "message_delta", delta: { stop_reason: "end_turn" }, usage: { input_tokens: 12, output_tokens: 5 } }],
  ["message_stop", { type: "message_stop" }],
]);
const cChunk = (delta, extra = {}) => ({ id: "chatcmpl-1", object: "chat.completion.chunk", created: 1, model: "m", choices: [{ index: 0, delta, finish_reason: extra.finish ?? null }], ...(extra.usage ? { usage: extra.usage } : {}) });
const cText = `${[cChunk({ role: "assistant", content: "Hel" }), cChunk({ content: "lo" }), cChunk({}, { finish: "stop", usage: { prompt_tokens: 12, completion_tokens: 5, total_tokens: 17 } })].map((c) => `data: ${JSON.stringify(c)}\n\n`).join("")}data: [DONE]\n\n`;

const COPILOT_TOKEN = "tid=abc;exp=2000000000;sku=copilot;proxy-ep=proxy.business.githubcopilot.com;8kp=1";
function copilot(id) {
  const m = model("github-copilot", id);
  return { m, pi: { ...m, baseUrl: "https://api.business.githubcopilot.com" } };
}
{
  const c = copilot("claude-sonnet-4.6");
  add("anthropic-messages", run("p_copilot_vision", "anthropic-messages", c.m, images(c.m, "toolu_1").slice(0, 2), { sse: aText, apiKey: COPILOT_TOKEN, pi: { model: c.pi }, options: { reasoning: "high" } }));
  add("anthropic-messages", run("p_copilot_agent", "anthropic-messages", c.m, toolsRoundtrip(c.m, "toolu_1").slice(0, 4), { sse: aText, apiKey: COPILOT_TOKEN, pi: { model: c.pi } }));
}
{
  const c = copilot("gpt-5.6-luna");
  add("openai-responses", run("p_copilot_user", "openai-responses", c.m, [sys("Sys"), user("hi")], { sse: rText(), apiKey: COPILOT_TOKEN, pi: { model: c.pi } }));
  add("openai-responses", run("p_copilot_agent_vision", "openai-responses", c.m, images(c.m, "call_1|fc_1").slice(0, 4), { sse: rText(), apiKey: COPILOT_TOKEN, pi: { model: c.pi }, options: { reasoning: "medium" } }));
  const noProxy = model("github-copilot", "gpt-5.6-luna");
  add("openai-responses", run("p_copilot_no_proxy_ep", "openai-responses", noProxy, [sys("Sys"), user("hi")], { sse: rText(), apiKey: "ghu_plain_token" }));
}
{
  const ids = getBuiltinModel("github-copilot", "kimi-k3") ? "kimi-k3" : "gemini-3.8-flash";
  const c = copilot(ids);
  add("openai-completions", run("p_copilot_agent", "openai-completions", c.m, toolsRoundtrip(c.m, "call_1").slice(0, 4), { sse: cText, apiKey: COPILOT_TOKEN, pi: { model: c.pi } }));
}

const CF = { CLOUDFLARE_ACCOUNT_ID: "acct-cf", CLOUDFLARE_GATEWAY_ID: "gw-1" };
const cfGatewayAuth = (key) => ({ "cf-aig-authorization": `Bearer ${key}`, Authorization: null, "x-api-key": null });
for (const [api, id, sse] of [
  ["anthropic-messages", "claude-fable-5", aText],
  ["openai-responses", null, rText()],
  ["openai-completions", null, cText],
]) {
  const pick = id ?? (await import(join(PI, "providers/all.js"))).getBuiltinModels("cloudflare-ai-gateway").find((m) => m.api === api)?.id;
  const m = model("cloudflare-ai-gateway", pick);
  add(api, run("p_cloudflare_gateway", api, m, [sys("Sys"), user("hi")], {
    sse,
    apiKey: "cf-key",
    options: { env: CF },
    pi: { model: resolveCloudflareModel(m, CF), apiKey: undefined, options: { headers: cfGatewayAuth("cf-key") } },
  }));
}
{
  const m = model("cloudflare-workers-ai", getBuiltinModel("cloudflare-workers-ai", "@cf/moonshotai/kimi-k2.7") ? "@cf/moonshotai/kimi-k2.7" : (await import(join(PI, "providers/all.js"))).getBuiltinModels("cloudflare-workers-ai")[0].id);
  add("openai-completions", run("p_cloudflare_workers", "openai-completions", m, [sys("Sys"), user("hi")], {
    sse: cText,
    apiKey: "cf-key",
    options: { env: { CLOUDFLARE_ACCOUNT_ID: "acct-cf" } },
    pi: { model: resolveCloudflareModel(m, { CLOUDFLARE_ACCOUNT_ID: "acct-cf" }) },
  }));
}
{
  const m = model("azure", "deepseek-v4-pro");
  const env = { AZURE_OPENAI_RESOURCE_NAME: "my-resource", AZURE_OPENAI_DEPLOYMENT_NAME_MAP: "deepseek-v4-pro=ds-prod" };
  const piModel = { ...m, baseUrl: resolveAzureBaseUrl(m, { env }) };
  const deployment = resolveDeploymentName(m, { env });
  add("openai-completions", run("p_azure_completions", "openai-completions", m, [sys("Sys"), user("hi")], {
    sse: cText,
    apiKey: "azure-key",
    options: { env, reasoning: "high" },
    pi: { model: piModel, options: { onPayload: (p) => ({ ...p, model: deployment }) } },
  }));
}

const root = join(OINO, "extensions/sdk/oino-provider-kit/tests/fixtures");
// Sequential: the global fetch / Smithy capture is shared.
const done = [];
for (const { api, thunk } of cases) done.push({ api, fixture: await thunk() });
for (const { api, fixture } of done) {
  const dir = join(root, api);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${fixture.name}.json`), JSON.stringify(fixture, null, 1) + "\n");
}
console.log(`wrote ${done.length} fixtures to ${root}`);
