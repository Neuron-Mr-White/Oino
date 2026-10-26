// Golden generator: runs pi-agent-core `Agent` over scripted scenarios with a faux stream
// function and records every event plus the final transcript.
// Usage: node agent-loop.mjs > ../../../../crates/oino-agent/tests/fixtures/agent-loop.json
import { readFileSync } from "node:fs";
import { pathToFileURL } from "node:url";
const PI = process.env.PI_CODING_AGENT_PKG ?? `${process.env.HOME}/.local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent`;
const nm = `${PI}/node_modules/@earendil-works`;
const { Agent } = await import(pathToFileURL(`${nm}/pi-agent-core/dist/index.js`).href);
const { createAssistantMessageEventStream } = await import(pathToFileURL(`${nm}/pi-ai/dist/index.js`).href);
const scenarios = JSON.parse(readFileSync(new URL("./scenarios.json", import.meta.url), "utf8"));

const model = { id: "faux-1", name: "Faux", api: "faux", provider: "faux", baseUrl: "", reasoning: true, input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 1000, maxTokens: 100 };
const zeroUsage = { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };

function fauxStream(responses) {
  let index = 0;
  return () => {
    const spec = responses[index++];
    const stream = createAssistantMessageEventStream();
    queueMicrotask(() => {
      const base = { role: "assistant", content: [], api: "faux", provider: "faux", model: "faux-1", usage: zeroUsage, stopReason: "pending", timestamp: 0 };
      const partial = () => structuredClone(base);
      stream.push({ type: "start", partial: partial() });
      spec.content.forEach((block, i) => {
        if (block.type === "text") {
          base.content.push({ type: "text", text: "" });
          stream.push({ type: "text_start", contentIndex: i, partial: partial() });
          base.content[i].text = block.text;
          stream.push({ type: "text_delta", contentIndex: i, delta: block.text, partial: partial() });
          stream.push({ type: "text_end", contentIndex: i, content: block.text, partial: partial() });
        } else if (block.type === "thinking") {
          base.content.push({ type: "thinking", thinking: "" });
          stream.push({ type: "thinking_start", contentIndex: i, partial: partial() });
          base.content[i].thinking = block.thinking;
          stream.push({ type: "thinking_delta", contentIndex: i, delta: block.thinking, partial: partial() });
          stream.push({ type: "thinking_end", contentIndex: i, content: block.thinking, partial: partial() });
        } else {
          base.content.push({ type: "toolCall", id: block.id, name: block.name, arguments: {} });
          stream.push({ type: "toolcall_start", contentIndex: i, partial: partial() });
          base.content[i].arguments = block.arguments;
          stream.push({ type: "toolcall_delta", contentIndex: i, delta: JSON.stringify(block.arguments), partial: partial() });
          stream.push({ type: "toolcall_end", contentIndex: i, toolCall: block, partial: partial() });
        }
      });
      const final = { ...partial(), stopReason: spec.stopReason, ...(spec.errorMessage ? { errorMessage: spec.errorMessage } : {}) };
      if (spec.stopReason === "error" || spec.stopReason === "aborted") stream.push({ type: "error", reason: spec.stopReason, error: final });
      else stream.push({ type: "done", reason: spec.stopReason, message: final });
      stream.end();
    });
    return stream;
  };
}

function strip(value) {
  if (Array.isArray(value)) return value.map(strip);
  if (value && typeof value === "object") {
    const out = {};
    for (const [k, v] of Object.entries(value)) {
      if (k === "timestamp" || k === "durationMs" || k === "partial") continue;
      out[k] = strip(v);
    }
    return out;
  }
  return value;
}

const results = [];
for (const scenario of scenarios) {
  let agent;
  const tools = scenario.tools.map((spec) => ({
    name: spec.name, label: spec.name, description: `${spec.name} tool`, parameters: spec.parameters,
    ...(spec.executionMode ? { executionMode: spec.executionMode } : {}),
    execute: async (_id, _params, _signal, onUpdate) => {
      const b = spec.behaviour;
      for (const u of b.updates ?? []) onUpdate?.(u);
      for (const s of b.steer ?? []) agent.steer({ role: "user", content: [{ type: "text", text: s }], timestamp: 0 });
      for (const f of b.followUp ?? []) agent.followUp({ role: "user", content: [{ type: "text", text: f }], timestamp: 0 });
      if (b.throws) throw new Error(b.throws);
      return b.result;
    },
  }));
  const hooks = scenario.hooks ?? {};
  let turn = 0;
  agent = new Agent({
    initialState: { systemPrompt: scenario.systemPrompt, model, thinkingLevel: scenario.thinkingLevel ?? "off", tools },
    streamFn: fauxStream(scenario.responses),
    steeringMode: scenario.steeringMode, toolExecution: scenario.toolExecution,
    beforeToolCall: hooks.block ? async ({ toolCall }) => (hooks.block[toolCall.name] ? { block: true, reason: hooks.block[toolCall.name] } : undefined) : undefined,
    afterToolCall: hooks.after ? async ({ toolCall }) => hooks.after[toolCall.name] : undefined,
    finishTurn: hooks.finishTurn ? async () => { const d = hooks.finishTurn[turn++]; return d ? { action: d } : undefined; } : undefined,
  });
  const events = [];
  agent.subscribe((event) => { events.push(structuredClone(event)); });
  await agent.prompt(scenario.prompt);
  results.push({ name: scenario.name, events: strip(events), messages: strip(agent.state.messages), errorMessage: agent.state.errorMessage ?? null });
}
console.log(JSON.stringify(results, null, 2));
