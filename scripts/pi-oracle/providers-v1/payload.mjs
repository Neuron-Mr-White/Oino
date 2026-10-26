// pi oracle for providers-v1: drives pi-ai 1.4.0's `streamSimple` for anthropic-messages,
// openai-responses and openai-completions with an injected `fetch` that captures the request
// (url, headers, JSON body) and serves a canned SSE (or error) response. Records pi's events
// (without `partial`) into JSON fixtures that the Rust provider kit replays.
//
// Usage (repo root): node scripts/pi-oracle/providers-v1/payload.mjs
// Output: extensions/sdk/oino-provider-kit/tests/fixtures/<api>/<case>.json with
//   {name, model, context, options, apiKey, status, sse, request: {url, headers, body}, events}
import { mkdirSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";

const PI = process.env.PI_AI_DIST ??
  join(homedir(), ".local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent/node_modules/@earendil-works/pi-ai/dist");
const OINO = process.env.OINO_REPO ?? ".";
const { streamSimple: anthropic } = await import(join(PI, "api/anthropic-messages.js"));
const { streamSimple: responses } = await import(join(PI, "api/openai-responses.js"));
const { streamSimple: completions } = await import(join(PI, "api/openai-completions.js"));
const { getBuiltinModel } = await import(join(PI, "providers/all.js"));

const STREAMS = { "anthropic-messages": anthropic, "openai-responses": responses, "openai-completions": completions };

function model(provider, id) {
  const m = getBuiltinModel(provider, id);
  if (!m) throw new Error(`missing catalog model ${provider}/${id}`);
  return structuredClone(m);
}

function captureFetch(capture, body, status, contentType) {
  return async (input, init) => {
    const url = typeof input === "string" ? input : (input.url ?? input.toString());
    const headers = {};
    const h = init?.headers;
    if (h instanceof Headers) h.forEach((v, k) => (headers[k.toLowerCase()] = v));
    else if (h) for (const [k, v] of Object.entries(h)) headers[k.toLowerCase()] = String(v);
    capture.url = url;
    capture.headers = headers;
    capture.body = init?.body ? JSON.parse(init.body) : undefined;
    return new Response(body, { status, headers: { "content-type": contentType } });
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

async function run(name, api, m, messages, { sse, status = 200, options = {}, apiKey = "test-key", contentType } = {}) {
  const capture = {};
  const events = [];
  const stream = STREAMS[api](m, { messages }, {
    apiKey,
    sessionId: "sess-1234",
    fetch: captureFetch(capture, sse, status, contentType ?? (status === 200 ? "text/event-stream" : "application/json")),
    ...options,
  });
  for await (const event of stream) events.push(event);
  return {
    name,
    model: m,
    context: { messages },
    options: { sessionId: "sess-1234", ...options },
    apiKey,
    status,
    sse,
    request: { url: capture.url, headers: capture.headers, body: capture.body },
    events: normalize(events),
  };
}

// ---- SSE builders -----------------------------------------------------------------------------

const named = (events) => events.map(([event, data]) => `event: ${event}\ndata: ${JSON.stringify(data)}\n\n`).join("");
const dataOnly = (events) => events.map((data) => `data: ${JSON.stringify(data)}\n\n`).join("") + "data: [DONE]\n\n";
const respSse = (events) => named(events.map((e) => [e.type, e]));

const aUsage = (input, output, read = 0, write = 0, extra = {}) => ({ input_tokens: input, output_tokens: output, cache_read_input_tokens: read, cache_creation_input_tokens: write, ...extra });
const aText = (id, text, usage = aUsage(12, 5), stop = "end_turn", modelId = "claude-sonnet-5") => named([
  ["message_start", { type: "message_start", message: { id, model: modelId, usage: aUsage(usage.input_tokens, 0, usage.cache_read_input_tokens, usage.cache_creation_input_tokens, usage.cache_creation ? { cache_creation: usage.cache_creation } : {}) } }],
  ["ping", { type: "ping" }],
  ["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: text.slice(0, 3) } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: text.slice(3) } }],
  ["content_block_stop", { type: "content_block_stop", index: 0 }],
  ["message_delta", { type: "message_delta", delta: { stop_reason: stop }, usage }],
  ["message_stop", { type: "message_stop" }],
]);
const aTool = named([
  ["message_start", { type: "message_start", message: { id: "msg_tool", model: "claude-sonnet-5", usage: aUsage(30, 0) } }],
  ["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Looking." } }],
  ["content_block_stop", { type: "content_block_stop", index: 0 }],
  ["content_block_start", { type: "content_block_start", index: 1, content_block: { type: "tool_use", id: "toolu_01B", name: "lookup", input: {} } }],
  ["content_block_delta", { type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: "{\"q\":" } }],
  ["content_block_delta", { type: "content_block_delta", index: 1, delta: { type: "input_json_delta", partial_json: "\"x\"}" } }],
  ["content_block_stop", { type: "content_block_stop", index: 1 }],
  ["message_delta", { type: "message_delta", delta: { stop_reason: "tool_use" }, usage: aUsage(30, 9) }],
  ["message_stop", { type: "message_stop" }],
]);
const aThinking = named([
  ["message_start", { type: "message_start", message: { id: "msg_think", model: "claude-sonnet-5", usage: aUsage(20, 0) } }],
  ["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "", signature: "" } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "ponder" } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "ing" } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "signature_delta", signature: "sigABC" } }],
  ["content_block_stop", { type: "content_block_stop", index: 0 }],
  ["content_block_start", { type: "content_block_start", index: 1, content_block: { type: "redacted_thinking", data: "opaque==" } }],
  ["content_block_stop", { type: "content_block_stop", index: 1 }],
  ["content_block_start", { type: "content_block_start", index: 2, content_block: { type: "text", text: "" } }],
  ["content_block_delta", { type: "content_block_delta", index: 2, delta: { type: "text_delta", text: "answer" } }],
  ["content_block_stop", { type: "content_block_stop", index: 2 }],
  ["message_delta", { type: "message_delta", delta: { stop_reason: "end_turn" }, usage: aUsage(20, 40, 0, 0, { output_tokens_details: { thinking_tokens: 25 } }) }],
  ["message_stop", { type: "message_stop" }],
]);
const aRefusal = named([
  ["message_start", { type: "message_start", message: { id: "msg_ref", model: "claude-sonnet-5", usage: aUsage(5, 0) } }],
  ["message_delta", { type: "message_delta", delta: { stop_reason: "refusal", stop_details: { explanation: "Declined." } }, usage: aUsage(5, 1) }],
  ["message_stop", { type: "message_stop" }],
]);
const aTruncated = named([
  ["message_start", { type: "message_start", message: { id: "msg_cut", model: "claude-sonnet-5", usage: aUsage(5, 0) } }],
  ["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }],
  ["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "partial" } }],
]);

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
const rDone = (id, output, usage, status = "completed", extra = {}) => ({ type: status === "completed" ? "response.completed" : "response.incomplete", response: { id, status, output, usage, ...extra } });
const rUsage = (input, output, cached = 0, reasoning = 0, write = 0) => ({ input_tokens: input, input_tokens_details: { cached_tokens: cached, ...(write ? { cache_write_tokens: write } : {}) }, output_tokens: output, output_tokens_details: { reasoning_tokens: reasoning }, total_tokens: input + output });
const rText = (text = "Hello", usage = rUsage(12, 5)) => respSse([rCreated("resp_1"), ...rMsg(0, "msg_o1", text, "final_answer"), rDone("resp_1", [], usage)]);
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
const rIncomplete = respSse([rCreated("resp_5"), ...rMsg(0, "msg_o5", "trunc"), rDone("resp_5", [], rUsage(10, 16), "incomplete", { incomplete_details: { reason: "max_output_tokens" } })]);
const rFailed = respSse([rCreated("resp_6"), { type: "response.failed", response: { id: "resp_6", status: "failed", error: { code: "server_error", message: "boom" } } }]);

const cChunk = (delta, extra = {}) => ({ id: "chatcmpl-1", object: "chat.completion.chunk", created: 1, model: extra.model ?? "m", choices: [{ index: 0, delta, finish_reason: extra.finish ?? null }], ...(extra.usage ? { usage: extra.usage } : {}) });
const cUsage = (prompt, completion, cached = 0, reasoning = 0, write = 0) => ({ prompt_tokens: prompt, completion_tokens: completion, total_tokens: prompt + completion, prompt_tokens_details: { cached_tokens: cached, ...(write ? { cache_write_tokens: write } : {}) }, completion_tokens_details: { reasoning_tokens: reasoning } });
const cText = (text = "Hello", usage = cUsage(12, 5), finish = "stop", modelId = "m") => dataOnly([
  cChunk({ role: "assistant", content: "" }, { model: modelId }),
  cChunk({ content: text.slice(0, 3) }, { model: modelId }),
  cChunk({ content: text.slice(3) }, { model: modelId }),
  cChunk({}, { finish, model: modelId }),
  { id: "chatcmpl-1", object: "chat.completion.chunk", created: 1, model: modelId, choices: [], usage },
]);
const cTool = dataOnly([
  cChunk({ role: "assistant", content: null, tool_calls: [{ index: 0, id: "call_1", type: "function", function: { name: "lookup", arguments: "" } }] }),
  cChunk({ tool_calls: [{ index: 0, function: { arguments: "{\"q\":" } }] }),
  cChunk({ tool_calls: [{ index: 0, function: { arguments: "\"x\"}" } }] }),
  cChunk({ tool_calls: [{ index: 1, id: "call_2", type: "function", function: { name: "write_file", arguments: "{\"path\":\"a\"}" } }] }),
  cChunk({}, { finish: "tool_calls", usage: cUsage(30, 9) }),
]);
const cReasoning = dataOnly([
  cChunk({ role: "assistant", reasoning_content: "think" }),
  cChunk({ reasoning_content: "ing" }),
  cChunk({ content: "answer" }),
  cChunk({}, { finish: "stop", usage: cUsage(20, 40, 4, 25) }),
]);
const cDetails = dataOnly([
  cChunk({ role: "assistant", reasoning: "plan", reasoning_details: [{ type: "reasoning.text", text: "plan", format: "anthropic-claude-v1", index: 0 }] }),
  cChunk({ reasoning_details: [{ type: "reasoning.text", text: "", signature: "sig1", format: "anthropic-claude-v1", index: 0 }] }),
  cChunk({ content: "done" }),
  cChunk({}, { finish: "stop", usage: cUsage(50, 10, 30, 2, 10) }),
]);

// ---- contexts ---------------------------------------------------------------------------------

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

function toolsRoundtrip(m, id) {
  return [
    sys("Sys", [lookup, writeFile]),
    user("look up x"),
    asst(m, [{ type: "text", text: "Sure." }, { type: "toolCall", id, name: "lookup", arguments: { q: "x" } }], "toolUse"),
    result(id, "lookup", [{ type: "text", text: "42" }]),
    user("thanks", 4),
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

const cases = [];
const add = (api, promise) => cases.push(promise.then((fixture) => ({ api, fixture })));

// anthropic-messages
{
  const api = "anthropic-messages";
  const sonnet = model("anthropic", "claude-sonnet-5");
  const haiku = model("anthropic", "claude-haiku-4-5");
  add(api, run("a_system_user", api, sonnet, [sys("Sys"), user("hi")], { sse: aText("msg_01", "Hello") }));
  add(api, run("b_tools_roundtrip", api, sonnet, toolsRoundtrip(sonnet, "toolu_01A"), { sse: aText("msg_02", "Hello") }));
  add(api, run("c_thinking_adaptive", api, sonnet, [sys("Sys"), user("q"), asst(sonnet, [{ type: "thinking", thinking: "t1", thinkingSignature: "sig0" }, { type: "thinking", thinking: "[Reasoning redacted]", thinkingSignature: "red0", redacted: true }, { type: "thinking", thinking: "unsigned" }, { type: "text", text: "a1" }]), user("again", 4)], { sse: aThinking, options: { reasoning: "high" } }));
  add(api, run("c_thinking_budget", api, haiku, [sys("Sys"), user("q")], { sse: aThinking, options: { reasoning: "medium", maxTokens: 4000 } }));
  add(api, run("c_thinking_xhigh_clamped", api, haiku, [sys("Sys"), user("q")], { sse: aThinking, options: { reasoning: "xhigh" } }));
  for (const retention of ["none", "short", "long"]) {
    add(api, run(`d_retention_${retention}`, api, sonnet, [sys("Sys", [lookup]), user("hi")], { sse: aText("msg_d", "Hello"), options: { cacheRetention: retention } }));
  }
  add(api, run("e_midconvo_collapsed", api, sonnet, midConvo(sonnet), { sse: aText("msg_e", "Hello") }));
  add(api, run("f_image", api, sonnet, images(sonnet, "toolu_img"), { sse: aText("msg_f", "Hello") }));
  add(api, run("f_error_tool_image", api, sonnet, errorImages(sonnet, "toolu_err"), { sse: aText("msg_f2", "Hello") }));
  add(api, run("g_usage_cache", api, sonnet, [sys("Sys"), user("hi")], { sse: aText("msg_g", "okay!", aUsage(100, 7, 60, 40, { cache_creation: { ephemeral_1h_input_tokens: 40 } })), options: { cacheRetention: "long" } }));
  add(api, run("h_error_400", api, sonnet, [sys("Sys"), user("hi")], { status: 400, sse: JSON.stringify({ type: "error", error: { type: "invalid_request_error", message: "bad request" } }) }));
  add(api, run("h_error_429", api, sonnet, [sys("Sys"), user("hi")], { status: 429, sse: JSON.stringify({ type: "error", error: { type: "rate_limit_error", message: "slow down" } }) }));
  add(api, run("i_tool_stream", api, sonnet, [sys("Sys", [lookup]), user("look up x")], { sse: aTool }));
  add(api, run("j_options", api, haiku, [sys("Sys", [lookup, strictTool]), user("hi")], { sse: aText("msg_j", "Hello"), options: { temperature: 0.5, toolChoice: "any", metadata: { user_id: "u-1" }, maxTokens: 1000 } }));
  add(api, run("k_length", api, sonnet, [sys("Sys"), user("hi")], { sse: aText("msg_k", "Hello", aUsage(5, 1), "max_tokens") }));
  add(api, run("l_oauth", api, sonnet, [sys("Sys", [readTool]), user("hi")], { sse: aText("msg_l", "Hello"), apiKey: "sk-ant-oat01-test" }));
  add(api, run("m_refusal", api, sonnet, [sys("Sys"), user("hi")], { sse: aRefusal }));
  add(api, run("n_truncated", api, sonnet, [sys("Sys"), user("hi")], { sse: aTruncated }));
  add(api, run("o_response_model", api, sonnet, [sys("Sys"), user("hi")], { sse: aText("msg_o", "Hello", aUsage(12, 5), "end_turn", "claude-sonnet-5-20261001") }));
}

// openai-responses
{
  const api = "openai-responses";
  const gpt55 = model("openai", "gpt-5.5");
  const gpt41 = model("openai", "gpt-4.1");
  add(api, run("a_system_user", api, gpt55, [sys("Sys"), user("hi")], { sse: rText() }));
  add(api, run("b_tools_additional", api, gpt55, toolsRoundtrip(gpt55, "call_01|fc_aaa"), { sse: rText() }));
  add(api, run("b_tools_toplevel", api, gpt41, [...toolsRoundtrip(gpt41, "call_01|fc_aaa").slice(0, 1).map((s) => ({ ...s, toolsAdded: [lookup, writeFile, strictTool] })), ...toolsRoundtrip(gpt41, "call_01|fc_aaa").slice(1)], { sse: rText() }));
  add(api, run("b_tools_foreign", api, gpt41, toolsRoundtrip({ ...gpt41, provider: "anthropic", api: "anthropic-messages", id: "claude" }, "toolu_01A"), { sse: rText() }));
  add(api, run("c_thinking_replay", api, gpt55, [sys("Sys"), user("q"), asst(gpt55, [{ type: "thinking", thinking: "pondering", thinkingSignature: JSON.stringify(rsItem) }, { type: "text", text: "a1", textSignature: JSON.stringify({ v: 1, id: "msg_prev", phase: "final_answer" }) }]), user("again", 4)], { sse: rThinking, options: { reasoning: "high" } }));
  add(api, run("c_thinking_minimal_clamped", api, gpt55, [sys("Sys"), user("q")], { sse: rThinking, options: { reasoning: "minimal" } }));
  for (const retention of ["none", "short", "long"]) {
    add(api, run(`d_retention_${retention}`, api, gpt41, [sys("Sys"), user("hi")], { sse: rText(), options: { cacheRetention: retention } }));
  }
  add(api, run("e_midconvo_native", api, gpt55, midConvo(gpt55), { sse: rText() }));
  add(api, run("e_midconvo_collapsed", api, gpt41, midConvo(gpt41), { sse: rText() }));
  add(api, run("f_image", api, gpt41, images(gpt41, "call_img|fc_img"), { sse: rText() }));
  add(api, run("g_usage_cache", api, gpt41, [sys("Sys"), user("hi")], { sse: rText("ok", rUsage(100, 7, 60, 0, 10)) }));
  add(api, run("h_error_400", api, gpt41, [sys("Sys"), user("hi")], { status: 400, sse: JSON.stringify({ error: { message: "bad request", type: "invalid_request_error", code: null } }) }));
  add(api, run("h_error_429", api, gpt41, [sys("Sys"), user("hi")], { status: 429, sse: JSON.stringify({ error: { message: "rate limited", type: "rate_limit_error", code: "rate_limit_exceeded" } }) }));
  add(api, run("i_tool_stream", api, gpt41, [sys("Sys", [lookup]), user("look up x")], { sse: rTool }));
  add(api, run("j_incomplete_length", api, gpt41, [sys("Sys"), user("hi")], { sse: rIncomplete, options: { maxTokens: 8, temperature: 0.2 } }));
  add(api, run("k_failed", api, gpt41, [sys("Sys"), user("hi")], { sse: rFailed }));
}

// openai-completions
{
  const api = "openai-completions";
  const llama = model("groq", "llama-3.3-70b-versatile");
  const oss = model("groq", "openai/gpt-oss-120b");
  const ds = model("deepseek", "deepseek-v4-pro");
  const orClaude = model("openrouter", "anthropic/claude-sonnet-4.6:batch");
  const zai = model("zai", "glm-5.2");
  const orOpenai = model("openrouter", "openai/gpt-4");
  add(api, run("a_system_user", api, llama, [sys("Sys"), user("hi")], { sse: cText() }));
  add(api, run("b_tools_roundtrip", api, oss, toolsRoundtrip(oss, "call_1"), { sse: cText(), options: { reasoning: "high" } }));
  add(api, run("b_tools_strict", api, oss, [sys("Sys", [lookup, strictTool]), user("hi")], { sse: cText() }));
  add(api, run("c_deepseek_thinking", api, ds, [sys("Sys"), user("q"), asst(ds, [{ type: "thinking", thinking: "t1", thinkingSignature: "reasoning_content" }, { type: "text", text: "a1" }]), user("again", 4)], { sse: cReasoning, options: { reasoning: "high" } }));
  add(api, run("c_deepseek_off", api, ds, [sys("Sys"), user("q")], { sse: cText() }));
  add(api, run("c_zai_thinking", api, zai, [sys("Sys"), user("q")], { sse: cReasoning, options: { reasoning: "medium" } }));
  add(api, run("d_openrouter_cache_long", api, orClaude, [sys("Sys", [lookup]), user("hi")], { sse: cDetails, options: { cacheRetention: "long", reasoning: "high" } }));
  add(api, run("d_openrouter_cache_none", api, orClaude, [sys("Sys", [lookup]), user("hi")], { sse: cText(), options: { cacheRetention: "none" } }));
  add(api, run("d_openrouter_replay_details", api, orClaude, [sys("Sys"), user("q"), asst(orClaude, [{ type: "thinking", thinking: "plan", thinkingSignature: JSON.stringify([{ type: "reasoning.text", text: "plan", signature: "sig1", format: "anthropic-claude-v1", index: 0 }]) }, { type: "text", text: "done" }]), user("again", 4)], { sse: cText() }));
  add(api, run("e_midconvo_collapsed", api, llama, midConvo(llama), { sse: cText() }));
  add(api, run("e_midconvo_native", api, ds, midConvo(ds), { sse: cText() }));
  add(api, run("f_image", api, orOpenai, images(orOpenai, "call_img"), { sse: cText() }));
  add(api, run("f_image_text_only", api, llama, images(llama, "call_img"), { sse: cText() }));
  add(api, run("g_usage_cache", api, orOpenai, [sys("Sys"), user("hi")], { sse: cText("ok", cUsage(100, 7, 60, 0, 10)) }));
  add(api, run("h_error_400", api, llama, [sys("Sys"), user("hi")], { status: 400, sse: JSON.stringify({ error: { message: "bad request", type: "invalid_request_error" } }) }));
  add(api, run("h_error_429_metadata", api, orOpenai, [sys("Sys"), user("hi")], { status: 429, sse: JSON.stringify({ error: { message: "Provider returned error", code: 429, metadata: { raw: "upstream rate limited" } } }) }));
  add(api, run("i_tool_stream", api, llama, [sys("Sys", [lookup, writeFile]), user("go")], { sse: cTool }));
  add(api, run("j_options", api, llama, [sys("Sys"), user("hi")], { sse: cText("Hello", cUsage(1, 1), "length"), options: { maxTokens: 100, temperature: 0.7, toolChoice: "auto" } }));
  add(api, run("k_content_filter", api, llama, [sys("Sys"), user("hi")], { sse: cText("Hel", cUsage(1, 1), "content_filter") }));
  add(api, run("l_no_finish", api, llama, [sys("Sys"), user("hi")], { sse: dataOnly([cChunk({ content: "x" })]) }));
}

const root = join(OINO, "extensions/sdk/oino-provider-kit/tests/fixtures");
const done = await Promise.all(cases);
for (const { api, fixture } of done) {
  const dir = join(root, api);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, `${fixture.name}.json`), JSON.stringify(fixture, null, 1) + "\n");
}
console.log(`wrote ${done.length} fixtures to ${root}`);
