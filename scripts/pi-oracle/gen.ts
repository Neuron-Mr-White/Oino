/**
 * pi oracle: capture the exact request (url/headers/body) and normalized
 * events pi emits for canned provider SSE responses.
 *
 * Usage: OINO_REPO=<oino checkout> PI_DIR=<pi checkout> node --experimental-strip-types gen.ts
 * (run `npm install --ignore-scripts && npm --prefix packages/ai run generate-models`
 *  in PI_DIR first so the model catalog exists).
 */
import { writeFileSync, mkdirSync } from "node:fs";
import { join } from "node:path";

const PI = process.env.PI_DIR ?? "/tmp/pi-upstream";
const OINO = process.env.OINO_REPO ?? ".";
const pi = (p: string) => `file://${PI}/${p}`;

const { streamSimple: anthropicStream } = await import(pi("packages/ai/src/api/anthropic-messages.ts"));
const { streamSimple: responsesStream } = await import(pi("packages/ai/src/api/openai-responses.ts"));
const { getBuiltinModel } = await import(pi("packages/ai/src/providers/all.ts"));
const { normalizeContext } = await import(pi("packages/ai/src/compat.ts"));

interface Capture {
	url?: string;
	headers?: Record<string, string>;
	body?: unknown;
}

function captureFetch(capture: Capture, body: string, status = 200, contentType = "text/event-stream") {
	return async (input: any, init?: any): Promise<Response> => {
		const url = typeof input === "string" ? input : input.url ?? input.toString();
		const headers: Record<string, string> = {};
		const h = init?.headers;
		if (h instanceof Headers) h.forEach((v, k) => (headers[k] = v));
		else if (h) for (const [k, v] of Object.entries(h)) headers[k.toLowerCase()] = String(v);
		capture.url = url;
		capture.headers = headers;
		capture.body = init?.body ? JSON.parse(init.body as string) : undefined;
		return new Response(body, { status, headers: { "content-type": contentType } });
	};
}

function normalizeEvents(events: any[]): unknown[] {
	return events.map((e) => {
		const base: any = { type: e.type };
		if (e.contentIndex !== undefined) base.contentIndex = e.contentIndex;
		if (e.delta !== undefined) base.delta = e.delta;
		if (e.content !== undefined) base.content = e.content;
		if (e.toolCall !== undefined) {
			base.toolCall = {
				id: e.toolCall.id,
				name: e.toolCall.name,
				arguments: e.toolCall.arguments,
			};
		}
		if (e.reason !== undefined) base.reason = e.reason;
		if (e.type === "done") {
			base.stopReason = e.message?.stopReason;
			base.usage = e.message?.usage;
			base.responseId = e.message?.responseId;
			base.responseModel = e.message?.responseModel;
		}
		if (e.type === "error") base.errorMessage = e.error?.errorMessage;
		return base;
	});
}

async function runCase(streamFn: any, model: any, contextMessages: any[], sse: string, options: any = {}, status = 200) {
	const capture: Capture = {};
	const context = normalizeContext({ messages: contextMessages });
	const events: any[] = [];
	const eventStream = streamFn(model, context, {
		apiKey: "test-key",
		sessionId: "sess-1234",
		fetch: captureFetch(capture, sse, status),
		...options,
	});
	for await (const event of eventStream) events.push(event);
	return {
		context: contextMessages,
		sse,
		status,
		model: { provider: model.provider, id: model.id },
		request: { url: capture.url, headers: capture.headers, body: capture.body },
		events: normalizeEvents(events),
	};
}

// ---- canned SSE -----------------------------------------------------------

const anthropicTextSse = [
	["message_start", { type: "message_start", message: { id: "msg_01", model: "claude-x", usage: { input_tokens: 12, output_tokens: 0, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 } } }],
	["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }],
	["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "Hello" } }],
	["content_block_stop", { type: "content_block_stop", index: 0 }],
	["message_delta", { type: "message_delta", delta: { stop_reason: "end_turn" }, usage: { input_tokens: 12, output_tokens: 5, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 } }],
	["message_stop", { type: "message_stop" }],
];

const anthropicToolSse = [
	["message_start", { type: "message_start", message: { id: "msg_02", usage: { input_tokens: 30, output_tokens: 0, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 } } }],
	["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "tool_use", id: "toolu_01A", name: "lookup", input: {} } }],
	["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: "{\"q\":" } }],
	["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "input_json_delta", partial_json: "\"x\"}" } }],
	["content_block_stop", { type: "content_block_stop", index: 0 }],
	["message_delta", { type: "message_delta", delta: { stop_reason: "tool_use" }, usage: { input_tokens: 30, output_tokens: 9, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 } }],
	["message_stop", { type: "message_stop" }],
];

const anthropicThinkingSse = [
	["message_start", { type: "message_start", message: { id: "msg_03", usage: { input_tokens: 20, output_tokens: 0, cache_read_input_tokens: 0, cache_creation_input_tokens: 0 } } }],
	["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "thinking", thinking: "", signature: "" } }],
	["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "thinking_delta", thinking: "pondering" } }],
	["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "signature_delta", signature: "sigABC" } }],
	["content_block_stop", { type: "content_block_stop", index: 0 }],
	["content_block_start", { type: "content_block_start", index: 1, content_block: { type: "text", text: "" } }],
	["content_block_delta", { type: "content_block_delta", index: 1, delta: { type: "text_delta", text: "answer" } }],
	["content_block_stop", { type: "content_block_stop", index: 1 }],
	["message_delta", { type: "message_delta", delta: { stop_reason: "end_turn" }, usage: { input_tokens: 20, output_tokens: 40, cache_read_input_tokens: 0, cache_creation_input_tokens: 0, output_tokens_details: { thinking_tokens: 25 } } }],
	["message_stop", { type: "message_stop" }],
];

const anthropicUsageSse = [
	["message_start", { type: "message_start", message: { id: "msg_04", usage: { input_tokens: 100, output_tokens: 0, cache_read_input_tokens: 60, cache_creation_input_tokens: 40, cache_creation: { ephemeral_1h_input_tokens: 40 } } } }],
	["content_block_start", { type: "content_block_start", index: 0, content_block: { type: "text", text: "" } }],
	["content_block_delta", { type: "content_block_delta", index: 0, delta: { type: "text_delta", text: "ok" } }],
	["content_block_stop", { type: "content_block_stop", index: 0 }],
	["message_delta", { type: "message_delta", delta: { stop_reason: "end_turn" }, usage: { input_tokens: 100, output_tokens: 7, cache_read_input_tokens: 60, cache_creation_input_tokens: 40 } }],
	["message_stop", { type: "message_stop" }],
];

const responsesTextSse = [
	[null, { type: "response.created", response: { id: "resp_1", status: "in_progress", output: [] } }],
	[null, { type: "response.output_item.added", output_index: 0, item: { type: "message", id: "msg_o1", role: "assistant", status: "in_progress", content: [] } }],
	[null, { type: "response.content_part.added", output_index: 0, item_id: "msg_o1", content_index: 0, part: { type: "output_text", text: "", annotations: [] } }],
	[null, { type: "response.output_text.delta", output_index: 0, item_id: "msg_o1", content_index: 0, delta: "Hello" }],
	[null, { type: "response.output_item.done", output_index: 0, item: { type: "message", id: "msg_o1", role: "assistant", status: "completed", content: [{ type: "output_text", text: "Hello", annotations: [] }] } }],
	[null, { type: "response.completed", response: { id: "resp_1", status: "completed", output: [{ type: "message", id: "msg_o1", role: "assistant", status: "completed", content: [{ type: "output_text", text: "Hello", annotations: [] }] }], usage: { input_tokens: 12, output_tokens: 5, total_tokens: 17 } } }],
];

const responsesToolSse = [
	[null, { type: "response.created", response: { id: "resp_2", status: "in_progress", output: [] } }],
	[null, { type: "response.output_item.added", output_index: 0, item: { type: "function_call", id: "fc_aaa", call_id: "call_01", name: "lookup", arguments: "", status: "in_progress" } }],
	[null, { type: "response.function_call_arguments.delta", output_index: 0, item_id: "fc_aaa", delta: "{\"q\":" }],
	[null, { type: "response.function_call_arguments.delta", output_index: 0, item_id: "fc_aaa", delta: "\"x\"}" }],
	[null, { type: "response.output_item.done", output_index: 0, item: { type: "function_call", id: "fc_aaa", call_id: "call_01", name: "lookup", arguments: "{\"q\":\"x\"}", status: "completed" } }],
	[null, { type: "response.completed", response: { id: "resp_2", status: "completed", output: [{ type: "function_call", id: "fc_aaa", call_id: "call_01", name: "lookup", arguments: "{\"q\":\"x\"}", status: "completed" }], usage: { input_tokens: 30, output_tokens: 9, total_tokens: 39 } } }],
];

const reasoningItem = { type: "reasoning", id: "rs_1", summary: [], content: [], encrypted_content: "enc_abc", status: "completed" };
const responsesThinkingSse = [
	[null, { type: "response.created", response: { id: "resp_3", status: "in_progress", output: [] } }],
	[null, { type: "response.output_item.added", output_index: 0, item: { type: "reasoning", id: "rs_1", summary: [], content: [] } }],
	[null, { type: "response.reasoning_summary_text.delta", output_index: 0, item_id: "rs_1", summary_index: 0, delta: "pondering" }],
	[null, { type: "response.output_item.done", output_index: 0, item: reasoningItem }],
	[null, { type: "response.output_item.added", output_index: 1, item: { type: "message", id: "msg_o3", role: "assistant", status: "in_progress", content: [] } }],
	[null, { type: "response.content_part.added", output_index: 1, item_id: "msg_o3", content_index: 0, part: { type: "output_text", text: "", annotations: [] } }],
	[null, { type: "response.output_text.delta", output_index: 1, item_id: "msg_o3", content_index: 0, delta: "answer" }],
	[null, { type: "response.output_item.done", output_index: 1, item: { type: "message", id: "msg_o3", role: "assistant", status: "completed", content: [{ type: "output_text", text: "answer", annotations: [] }] } }],
	[null, { type: "response.completed", response: { id: "resp_3", status: "completed", output: [reasoningItem, { type: "message", id: "msg_o3", role: "assistant", status: "completed", content: [{ type: "output_text", text: "answer", annotations: [] }] }], usage: { input_tokens: 20, output_tokens: 40, output_tokens_details: { reasoning_tokens: 25 }, total_tokens: 60 } } }],
];

const responsesUsageSse = [
	[null, { type: "response.created", response: { id: "resp_4", status: "in_progress", output: [] } }],
	[null, { type: "response.output_item.added", output_index: 0, item: { type: "message", id: "msg_o4", role: "assistant", status: "in_progress", content: [] } }],
	[null, { type: "response.content_part.added", output_index: 0, item_id: "msg_o4", content_index: 0, part: { type: "output_text", text: "", annotations: [] } }],
	[null, { type: "response.output_text.delta", output_index: 0, item_id: "msg_o4", content_index: 0, delta: "ok" }],
	[null, { type: "response.output_item.done", output_index: 0, item: { type: "message", id: "msg_o4", role: "assistant", status: "completed", content: [{ type: "output_text", text: "ok", annotations: [] }] } }],
	[null, { type: "response.completed", response: { id: "resp_4", status: "completed", output: [{ type: "message", id: "msg_o4", role: "assistant", status: "completed", content: [{ type: "output_text", text: "ok", annotations: [] }] }], usage: { input_tokens: 100, input_tokens_details: { cached_tokens: 60, cache_write_tokens: 40 }, output_tokens: 7, total_tokens: 107 } } }],
];

function sse(events: Array<[string | null, unknown]>): string {
	return events
		.map(([event, data]) => {
			const name = event ?? (data as { type?: string }).type ?? "";
			return `event: ${name}\ndata: ${JSON.stringify(data)}\n`;
		})
		.join("\n") + "\n";
}

// ---- contexts (pi message shapes; ids normalized to native formats) -------

const lookupTool = { name: "lookup", description: "Look up a value", parameters: { type: "object", properties: { q: { type: "string" } }, required: ["q"] } };
const writeTool = { name: "write_file", description: "Write a file", parameters: { type: "object", properties: { path: { type: "string" }, data: { type: "string" } }, required: ["path"] } };

function base(systemPrompt: string, tools: any[]): any[] {
	return [{ role: "system", content: systemPrompt, toolsAdded: tools, timestamp: 0 }];
}

async function main() {
	const outRoot = join(OINO, "extensions/crates");
	const results: Record<string, unknown> = {};

	const anthropicSonnet = getBuiltinModel("anthropic", "claude-sonnet-5");
	const anthropicOpus = getBuiltinModel("anthropic", "claude-opus-5-5");
	const anthropicHaiku = getBuiltinModel("anthropic", "claude-haiku-4-5");
	const gpt55 = getBuiltinModel("openai", "gpt-5.5");
	const gpt5 = getBuiltinModel("openai", "gpt-5");
	for (const [name, m] of [["claude-sonnet-5", anthropicSonnet], ["claude-opus-5-5", anthropicOpus], ["claude-haiku-4-5", anthropicHaiku], ["gpt-5.5", gpt55], ["gpt-5", gpt5]]) {
		if (!m) throw new Error(`missing catalog model ${name}`);
	}

	const anthropicCases: Record<string, () => Promise<unknown>> = {
		"a_system_user": () => runCase(anthropicStream, anthropicSonnet, [
			...base("You are helpful.", []),
			{ role: "user", content: "Hello", timestamp: 1 },
		], sse(anthropicTextSse)),
		"b_tools_roundtrip": () => runCase(anthropicStream, anthropicSonnet, [
			...base("Sys", [lookupTool, writeTool]),
			{ role: "user", content: "look up x", timestamp: 1 },
			{ role: "assistant", api: "anthropic-messages", provider: "anthropic", model: anthropicSonnet.id, content: [{ type: "toolCall", id: "toolu_01A", name: "lookup", arguments: { q: "x" } }], stopReason: "toolUse", usage: {input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}}, timestamp: 2 },
			{ role: "toolResult", toolCallId: "toolu_01A", toolName: "lookup", content: [{ type: "text", text: "42" }], isError: false, timestamp: 3 },
			{ role: "user", content: "thanks", timestamp: 4 },
		], sse(anthropicTextSse)),
		"c_thinking_replay": () => runCase(anthropicStream, anthropicSonnet, [
			...base("Sys", []),
			{ role: "user", content: "q", timestamp: 1 },
			{ role: "assistant", api: "anthropic-messages", provider: "anthropic", model: anthropicSonnet.id, content: [{ type: "thinking", thinking: "pondering", thinkingSignature: "sigABC" }, { type: "text", text: "answer" }], stopReason: "stop", usage: {input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}}, timestamp: 2 },
			{ role: "user", content: "again", timestamp: 3 },
		], sse(anthropicThinkingSse), { reasoning: "medium" }),
		"d_retention_short": () => runCase(anthropicStream, anthropicSonnet, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(anthropicTextSse), { cacheRetention: "short" }),
		"d_retention_long": () => runCase(anthropicStream, anthropicSonnet, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(anthropicTextSse), { cacheRetention: "long" }),
		"d_retention_none": () => runCase(anthropicStream, anthropicSonnet, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(anthropicTextSse), { cacheRetention: "none" }),
		"e_midconvo_capable": () => runCase(anthropicStream, anthropicOpus, [
			...base("Sys", [lookupTool]),
			{ role: "user", content: "hi", timestamp: 1 },
			{ role: "system", content: "", sections: { "project/AGENT.md": "new section text" }, toolsAdded: [writeTool], timestamp: 2 },
			{ role: "user", content: "more", timestamp: 3 },
		], sse(anthropicTextSse)),
		"e_midconvo_collapsed": () => runCase(anthropicStream, anthropicHaiku, [
			...base("Sys", [lookupTool]),
			{ role: "user", content: "hi", timestamp: 1 },
			{ role: "system", content: "", sections: { "project/AGENT.md": "new section text" }, toolsAdded: [writeTool], timestamp: 2 },
			{ role: "user", content: "more", timestamp: 3 },
		], sse(anthropicTextSse)),
		"f_image": () => runCase(anthropicStream, anthropicSonnet, [
			...base("Sys", []),
			{ role: "user", content: [{ type: "text", text: "what is this" }, { type: "image", mimeType: "image/png", data: "aGk=" }], timestamp: 1 },
		], sse(anthropicTextSse)),
		"g_usage_cache": () => runCase(anthropicStream, anthropicSonnet, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(anthropicUsageSse), { cacheRetention: "long" }),
		"h_error_429": () => runCase(anthropicStream, anthropicSonnet, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], '{"type":"error","error":{"type":"rate_limit_error","message":"slow down"}}', {}, 429),
		"h_error_400": () => runCase(anthropicStream, anthropicSonnet, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], '{"type":"error","error":{"type":"invalid_request_error","message":"bad request"}}', {}, 400),
	};

	const responsesCases: Record<string, () => Promise<unknown>> = {
		"a_system_user": () => runCase(responsesStream, gpt55, [
			...base("You are helpful.", []),
			{ role: "user", content: "Hello", timestamp: 1 },
		], sse(responsesTextSse)),
		"b_tools_roundtrip": () => runCase(responsesStream, gpt55, [
			...base("Sys", [lookupTool, writeTool]),
			{ role: "user", content: "look up x", timestamp: 1 },
			{ role: "assistant", api: "openai-responses", provider: "openai", model: gpt55.id, content: [{ type: "toolCall", id: "call_01|fc_aaa", name: "lookup", arguments: { q: "x" } }], stopReason: "toolUse", usage: {input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}}, timestamp: 2 },
			{ role: "toolResult", toolCallId: "call_01|fc_aaa", toolName: "lookup", content: [{ type: "text", text: "42" }], isError: false, timestamp: 3 },
			{ role: "user", content: "thanks", timestamp: 4 },
		], sse(responsesTextSse)),
		"c_thinking_replay": () => runCase(responsesStream, gpt55, [
			...base("Sys", []),
			{ role: "user", content: "q", timestamp: 1 },
			{ role: "assistant", api: "openai-responses", provider: "openai", model: gpt55.id, content: [{ type: "thinking", thinking: "pondering", thinkingSignature: JSON.stringify(reasoningItem) }, { type: "text", text: "answer", textSignature: JSON.stringify({ v: 1, id: "msg_o3" }) }], stopReason: "stop", usage: {input:0,output:0,cacheRead:0,cacheWrite:0,totalTokens:0,cost:{input:0,output:0,cacheRead:0,cacheWrite:0,total:0}}, timestamp: 2 },
			{ role: "user", content: "again", timestamp: 3 },
		], sse(responsesThinkingSse), { reasoning: "medium" }),
		"d_retention_short": () => runCase(responsesStream, gpt55, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(responsesTextSse), { cacheRetention: "short" }),
		"d_retention_long": () => runCase(responsesStream, gpt55, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(responsesTextSse), { cacheRetention: "long" }),
		"d_retention_none": () => runCase(responsesStream, gpt55, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(responsesTextSse), { cacheRetention: "none" }),
		"e_midconvo_capable": () => runCase(responsesStream, gpt55, [
			...base("Sys", [lookupTool]),
			{ role: "user", content: "hi", timestamp: 1 },
			{ role: "system", content: "", sections: { "project/AGENT.md": "new section text" }, toolsAdded: [writeTool], timestamp: 2 },
			{ role: "user", content: "more", timestamp: 3 },
		], sse(responsesTextSse)),
		"e_midconvo_collapsed": () => runCase(responsesStream, gpt5, [
			...base("Sys", [lookupTool]),
			{ role: "user", content: "hi", timestamp: 1 },
			{ role: "system", content: "", sections: { "project/AGENT.md": "new section text" }, toolsAdded: [writeTool], timestamp: 2 },
			{ role: "user", content: "more", timestamp: 3 },
		], sse(responsesTextSse)),
		"f_image": () => runCase(responsesStream, gpt55, [
			...base("Sys", []),
			{ role: "user", content: [{ type: "text", text: "what is this" }, { type: "image", mimeType: "image/png", data: "aGk=" }], timestamp: 1 },
		], sse(responsesTextSse)),
		"g_usage_cache": () => runCase(responsesStream, gpt55, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], sse(responsesUsageSse)),
		"h_error_429": () => runCase(responsesStream, gpt55, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], '{"error":{"message":"rate limited","type":"rate_limit_error","code":"rate_limit_exceeded"}}', {}, 429),
		"h_error_400": () => runCase(responsesStream, gpt55, [...base("Sys", []), { role: "user", content: "hi", timestamp: 1 }], '{"error":{"message":"bad request","type":"invalid_request_error","code":"invalid"}}', {}, 400),
	};

	for (const [dir, cases] of [["provider-anthropic", anthropicCases], ["provider-openai-responses", responsesCases]] as const) {
		const out = join(outRoot, dir, "fixtures/pi");
		mkdirSync(out, { recursive: true });
		for (const [name, run] of Object.entries(cases)) {
			const result = await run();
			writeFileSync(join(out, `${name}.json`), JSON.stringify(result, null, 2));
			console.log(`${dir}/${name}: ok (${(result as any).events.length} events)`);
			results[`${dir}/${name}`] = (result as any).events.length;
		}
	}
}

main().catch((err) => {
	console.error(err);
	process.exit(1);
});
