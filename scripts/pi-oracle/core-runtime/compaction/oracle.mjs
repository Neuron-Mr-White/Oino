// Golden generator for `oino-core::compaction` (pi-coding-agent 1.4.0 `core/compaction/*`).
// Builds sessions with pi's own SessionManager, runs pi's compaction/branch-summary functions
// and a recording fake `streamFn`, and writes JSON fixtures.
// Usage: node oracle.mjs [out-dir]   (default: crates/oino-core/tests/fixtures/compaction)
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PI =
  process.env.PI_CODING_AGENT_PKG ??
  `${process.env.HOME}/.local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent`;
const imp = (rel) => import(pathToFileURL(join(PI, rel)).href);
const C = await imp("dist/core/compaction/index.js");
const { SessionManager } = await imp("dist/core/session-manager.js");
const { convertToLlm } = await imp("dist/core/messages.js");
const AI = await imp("node_modules/@earendil-works/pi-ai/dist/index.js");
const { getOverflowPatterns } = await imp("node_modules/@earendil-works/pi-ai/dist/utils/overflow.js");

const here = dirname(fileURLToPath(import.meta.url));
const out = process.argv[2] ?? join(here, "../../../../crates/oino-core/tests/fixtures/compaction");
mkdirSync(out, { recursive: true });
const write = (name, value, indent = 1) => writeFileSync(join(out, name), `${JSON.stringify(value, null, indent)}\n`);

// ---------------------------------------------------------------------------------------------
// Message builders (deterministic timestamps)
// ---------------------------------------------------------------------------------------------
let clock = Date.parse("2026-01-01T00:00:00.000Z");
const tick = () => (clock += 1000);
const usage = (total, extra = {}) => ({
  input: Math.floor(total / 2), output: Math.ceil(total / 2), cacheRead: 0, cacheWrite: 0, totalTokens: total,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 }, ...extra,
});
const words = (n, seed = "w") => Array.from({ length: n }, (_, i) => `${seed}${i}`).join(" ");
const user = (content) => ({ role: "user", content, timestamp: tick() });
const assistant = (content, extra = {}) => ({
  role: "assistant",
  content: typeof content === "string" ? [{ type: "text", text: content }] : content,
  api: "faux", provider: "faux", model: "faux-1", usage: usage(0), stopReason: "stop", timestamp: tick(), ...extra,
});
const call = (id, name, args) => ({ type: "toolCall", id, name, arguments: args });
const toolResult = (toolCallId, toolName, text, extra = {}) => ({
  role: "toolResult", toolCallId, toolName, content: [{ type: "text", text }], isError: false, timestamp: tick(), ...extra,
});
const bash = (command, output, extra = {}) => ({
  role: "bashExecution", command, output, exitCode: 0, cancelled: false, truncated: false, timestamp: tick(), ...extra,
});
const system = (extra) => ({ role: "system", content: "", timestamp: tick(), ...extra });
const image = { type: "image", data: "AAAA", mimeType: "image/png" };

// ---------------------------------------------------------------------------------------------
// Session scenarios
// ---------------------------------------------------------------------------------------------
const scenarios = {
  empty: () => {},
  "plain-chat": (sm) => {
    for (let i = 0; i < 6; i++) {
      sm.appendMessage(user(`question ${i}: ${words(20 + i * 7, "q")}`));
      sm.appendMessage(assistant(`answer ${i}: ${words(30 + i * 5, "a")}`, { usage: usage(100 * (i + 1)) }));
    }
  },
  "tool-turns": (sm) => {
    sm.appendMessage(user("Fix the bug in src/lib.rs"));
    sm.appendMessage(assistant([{ type: "thinking", thinking: words(40, "t") }, call("c1", "read", { path: "src/lib.rs" })], { stopReason: "toolUse", usage: usage(500) }));
    sm.appendMessage(toolResult("c1", "read", words(600, "line")));
    sm.appendMessage(assistant([{ type: "text", text: "Editing now." }, call("c2", "edit", { path: "src/lib.rs", oldText: "a", newText: "b" })], { stopReason: "toolUse", usage: usage(900) }));
    sm.appendMessage(toolResult("c2", "edit", "Edited src/lib.rs"));
    sm.appendMessage(assistant([call("c3", "write", { path: "src/new.rs", content: words(50, "x") }), call("c4", "read", { path: "README.md" })], { stopReason: "toolUse" }));
    sm.appendMessage(toolResult("c3", "write", "Wrote src/new.rs"));
    sm.appendMessage(toolResult("c4", "read", words(100, "readme")));
    sm.appendMessage(assistant("Done fixing.", { usage: usage(1500) }));
    sm.appendMessage(user("Thanks, now run tests"));
    sm.appendMessage(assistant([call("c5", "bash", { command: "cargo test", timeout: 60 })], { stopReason: "toolUse" }));
    sm.appendMessage(toolResult("c5", "bash", words(300, "ok")));
    sm.appendMessage(assistant("All tests pass.", { usage: usage(2100) }));
  },
  "long-single-turn": (sm) => {
    sm.appendMessage(user("Refactor everything"));
    for (let i = 0; i < 10; i++) {
      sm.appendMessage(assistant([{ type: "text", text: `step ${i}` }, call(`s${i}`, "read", { path: `file${i}.ts` })], { stopReason: "toolUse", usage: usage(1000 + i * 300) }));
      sm.appendMessage(toolResult(`s${i}`, "read", words(80 + i * 10, `f${i}_`)));
    }
    sm.appendMessage(assistant("Refactor complete.", { usage: usage(5000) }));
  },
  "bash-and-custom": (sm) => {
    sm.appendMessage(user(words(30, "u")));
    sm.appendMessage(bash("ls -la", words(50, "file")));
    sm.appendMessage(bash("false", "", { exitCode: 1 }));
    sm.appendMessage(bash("secret", "hidden output", { excludeFromContext: true }));
    sm.appendCustomMessageEntry("note", words(25, "note"), true, { k: 1 });
    sm.appendMessage(assistant(words(40, "reply"), { usage: usage(700) }));
    sm.appendCustomMessageEntry("blocks", [{ type: "text", text: words(10, "blk") }, image], false);
    sm.appendMessage(user([{ type: "text", text: "look at this" }, image]));
    sm.appendMessage(assistant(words(60, "seen")));
    sm.appendMessage(bash("cancelled", "", { cancelled: true, exitCode: undefined }));
    sm.appendMessage(user("final"));
  },
  metadata: (sm) => {
    sm.appendModelChange("faux", "faux-1");
    sm.appendThinkingLevelChange("high");
    const u = sm.appendMessage(user(words(40, "m")));
    sm.appendLabelChange(u, "start");
    sm.appendMessage(assistant(words(40, "n"), { usage: usage(300) }));
    sm.appendSessionInfo("named");
    sm.appendCustomEntry("ext-state", { a: 1 });
    sm.appendMessage(user(words(40, "o")));
    sm.appendModelChange("faux", "faux-2");
    sm.appendThinkingLevelChange("low");
    sm.appendMessage(assistant(words(40, "p"), { usage: usage(600) }));
    sm.appendCustomEntry("ext-state", { a: 2 });
    sm.appendMessage(user(words(40, "q")));
    sm.appendMessage(assistant(words(40, "r")));
  },
  "repeated-compaction": (sm) => {
    const ids = [];
    for (let i = 0; i < 4; i++) {
      ids.push(sm.appendMessage(user(`r${i} ${words(30, "x")}`)));
      sm.appendMessage(assistant([{ type: "text", text: `ok ${i}` }, call(`rc${i}`, i % 2 ? "edit" : "read", { path: `p${i}.txt` })], { stopReason: "toolUse", usage: usage(200 * (i + 1)) }));
      sm.appendMessage(toolResult(`rc${i}`, "read", words(40, "res")));
      sm.appendMessage(assistant(`fin ${i}`));
    }
    sm.appendCompaction("First summary", ids[2], 1234, { readFiles: ["old-read.txt"], modifiedFiles: ["old-mod.txt"] });
    for (let i = 4; i < 7; i++) {
      ids.push(sm.appendMessage(user(`r${i} ${words(30, "y")}`)));
      sm.appendMessage(assistant(`after ${i} ${words(30, "z")}`, { usage: usage(300 * i) }));
    }
  },
  "double-compaction": (sm) => {
    const ids = [];
    for (let i = 0; i < 3; i++) {
      ids.push(sm.appendMessage(user(`d${i} ${words(30, "x")}`)));
      sm.appendMessage(assistant(`a${i} ${words(30, "y")}`, { usage: usage(100 * (i + 1)) }));
    }
    sm.appendCompaction("Summary one", ids[1], 500, { readFiles: ["one.txt"], modifiedFiles: [] });
    for (let i = 3; i < 6; i++) {
      ids.push(sm.appendMessage(user(`d${i} ${words(30, "x")}`)));
      sm.appendMessage(assistant(`a${i} ${words(30, "y")}`));
    }
    sm.appendCompaction("Summary two (hook)", ids[4], 800, { custom: true }, true);
    for (let i = 6; i < 9; i++) {
      ids.push(sm.appendMessage(user(`d${i} ${words(30, "x")}`)));
      sm.appendMessage(assistant(`a${i} ${words(30, "y")}`, { usage: usage(150 * i) }));
    }
  },
  "ends-with-compaction": (sm) => {
    const a = sm.appendMessage(user("hello"));
    sm.appendMessage(assistant("hi"));
    sm.appendCompaction("sum", a, 10);
  },
  "system-messages": (sm) => {
    sm.appendMessage(system({ content: "base prompt", sections: { cwd: "/work" }, toolsAdded: [{ name: "read", description: "Read", parameters: { type: "object" } }] }));
    sm.appendMessage(user(words(50, "s")));
    sm.appendMessage(assistant(words(50, "t"), { usage: usage(400) }));
    sm.appendMessage(system({ sections: { cwd: "/other", skills: "<skills/>" } }));
    sm.appendMessage(user(words(50, "u")));
    sm.appendMessage(assistant(words(50, "v")));
    sm.appendMessage(user(words(50, "w")));
  },
  "usage-variants": (sm) => {
    sm.appendMessage(user(words(30, "a")));
    sm.appendMessage(assistant(words(20, "b"), { usage: usage(900) }));
    sm.appendMessage(user(words(30, "c")));
    sm.appendMessage(assistant(words(20, "d"), { usage: { ...usage(0), input: 10, output: 20, cacheRead: 30, cacheWrite: 40, totalTokens: 0 } }));
    sm.appendMessage(user(words(30, "e")));
    sm.appendMessage(assistant("", { stopReason: "error", errorMessage: "overloaded", usage: usage(5000) }));
    sm.appendMessage(assistant("", { stopReason: "aborted", usage: usage(6000) }));
    sm.appendMessage(assistant("zero", { usage: usage(0) }));
    sm.appendMessage(user(words(30, "f")));
  },
  "recovery-omission": (sm) => {
    for (let i = 0; i < 3; i++) {
      sm.appendMessage(user(`q${i} ${words(40, "x")}`));
      sm.appendMessage(assistant(`a${i} ${words(40, "y")}`, { usage: usage(400 * (i + 1)) }));
    }
    sm.appendMessage(user(`overflowing ${words(40, "big")}`));
    const failed = sm.appendMessage(assistant("", { stopReason: "error", errorMessage: "prompt is too long: 213462 tokens > 200000 maximum", usage: usage(0) }));
    sm.appendContextEdit(failed, null);
  },
  "recovery-omission-tools": (sm) => {
    for (let i = 0; i < 3; i++) {
      sm.appendMessage(user(`q${i} ${words(40, "x")}`));
      sm.appendMessage(assistant(`a${i} ${words(40, "y")}`, { usage: usage(400 * (i + 1)) }));
    }
    sm.appendMessage(user(`run ${words(40, "cmd")}`));
    const a = sm.appendMessage(assistant([call("o1", "read", { path: "big.log" })], { stopReason: "toolUse", usage: usage(2000) }));
    const t = sm.appendMessage(toolResult("o1", "read", words(400, "log")));
    const failed = sm.appendMessage(assistant("", { stopReason: "length", usage: usage(0) }));
    sm.appendContextEdit(failed, null);
    sm.appendContextEdit(t, null);
    sm.appendContextEdit(a, null);
  },
  "external-replacement": (sm) => {
    const first = sm.appendMessage(user(`q0 ${words(40, "x")}`));
    sm.appendMessage(assistant(`a0 ${words(40, "y")}`, { usage: usage(400) }));
    sm.appendMessage(user(`q1 ${words(40, "x")}`));
    sm.appendMessage(assistant(`a1 ${words(40, "y")}`, { usage: usage(800) }));
    sm.appendMessage(user(`q2 ${words(40, "x")}`));
    const failed = sm.appendMessage(assistant("", { stopReason: "error", errorMessage: "boom", usage: usage(0) }));
    sm.appendContextEdit(failed, null);
    sm.appendContextEdit(first, { content: "edited first question" });
  },
  "context-edits": (sm) => {
    const u = sm.appendMessage(user(words(80, "orig")));
    const a = sm.appendMessage(assistant([{ type: "text", text: words(80, "resp") }, call("e1", "read", { path: "x.md" })], { stopReason: "toolUse", usage: usage(700) }));
    const t = sm.appendMessage(toolResult("e1", "read", words(300, "body")));
    sm.appendMessage(assistant("short", { usage: usage(900) }));
    sm.appendContextEdit(t, { content: "[elided]" });
    sm.appendContextEdit(u, { content: [{ type: "text", text: "rewritten" }] });
    sm.appendMessage(user(words(30, "next")));
    sm.appendContextEdit(a, null);
    sm.appendMessage(assistant(words(30, "fine")));
  },
  "nested-calls": (sm) => {
    sm.appendMessage(user("use codemode"));
    sm.appendMessage(assistant([call("n1", "codemode", { code: "..." })], { stopReason: "toolUse" }));
    sm.appendMessage(toolResult("n1", "codemode", "done", {
      nestedCalls: { complete: true, calls: [
        { id: "x1", name: "read", arguments: { path: "nested-read.ts" }, status: "ok" },
        { id: "x2", name: "write", arguments: { path: "nested-write.ts" }, status: "ok" },
        { id: "x3", name: "edit", arguments: { path: "Zeta.ts" }, status: "error" },
        { id: "x4", name: "read", arguments: { file: "ignored" }, status: "ok" },
      ] },
    }));
    sm.appendMessage(assistant([call("n2", "read", { path: "nested-write.ts" }), call("n3", "read", { path: "alpha.ts" }), call("n4", "read", { path: 5 })], { stopReason: "toolUse" }));
    sm.appendMessage(toolResult("n2", "read", "x"));
    sm.appendMessage(toolResult("n3", "read", "y"));
    sm.appendMessage(toolResult("n4", "read", "z"));
    sm.appendMessage(assistant(words(200, "final"), { usage: usage(3000) }));
    sm.appendMessage(user(words(200, "later")));
    sm.appendMessage(assistant(words(200, "end")));
  },
  branches: (sm) => {
    const root = sm.appendMessage(user(`root ${words(20, "r")}`));
    const a1 = sm.appendMessage(assistant(`a1 ${words(20, "r")}`, { usage: usage(100) }));
    sm.appendMessage(user(`left ${words(30, "l")}`));
    sm.appendMessage(assistant([call("b1", "edit", { path: "left.rs" })], { stopReason: "toolUse" }));
    sm.appendMessage(toolResult("b1", "edit", "ok"));
    sm.appendMessage(assistant(`left done ${words(30, "l")}`));
    const leftLeaf = sm.getLeafId();
    sm.branch(a1);
    sm.appendMessage(user(`right ${words(30, "g")}`));
    sm.appendMessage(assistant([call("b2", "read", { path: "right.rs" })], { stopReason: "toolUse" }));
    sm.appendMessage(toolResult("b2", "read", words(50, "rr")));
    sm.appendMessage(assistant(`right done ${words(30, "g")}`));
    sm.branchWithSummary(a1, "Summary of left branch", { readFiles: ["bs-read.md"], modifiedFiles: ["left.rs", "bs-mod.md"] });
    sm.appendMessage(user(`after summary ${words(20, "s")}`));
    sm.appendMessage(assistant(`ack ${words(20, "s")}`));
    sm.branchWithSummary(root, "Hook summary", { anything: 1 }, true);
    sm.appendMessage(user(`third ${words(10, "t")}`));
    void leftLeaf;
  },
};

const KEEPS = [0, 1, 10, 50, 100, 200, 400, 800, 1500, 3000, 20000];
const BUDGETS = [0, 5, 50, 200, 1000, 100000];

const toJson = (v) => JSON.parse(JSON.stringify(v ?? null));
const fileOpsJson = (ops) => ({ read: [...ops.read], written: [...ops.written], edited: [...ops.edited] });
const prepJson = (p) => (p ? toJson({ ...p, fileOps: fileOpsJson(p.fileOps) }) : null);

const sessions = {};
const managers = {};
for (const [name, build] of Object.entries(scenarios)) {
  const sm = SessionManager.inMemory("/work");
  build(sm);
  managers[name] = sm;
  const entries = sm.getEntries();
  const branch = sm.getBranch();
  const projection = sm.buildSessionProjection();
  const compactionCases = KEEPS.map((keepRecentTokens) => {
    const settings = { enabled: true, reserveTokens: 16384, keepRecentTokens };
    return {
      keepRecentTokens,
      prepare: prepJson(C.prepareCompaction(branch, settings)),
      findCutPoint: C.findCutPoint(branch, 0, branch.length, keepRecentTokens),
    };
  });
  // Every leaf pair for branch collection.
  const childCount = new Map();
  for (const e of entries) if (e.parentId) childCount.set(e.parentId, (childCount.get(e.parentId) ?? 0) + 1);
  const leaves = entries.filter((e) => !childCount.has(e.id)).map((e) => e.id);
  const targets = [...new Set([...leaves, ...entries.filter((_, i) => i % 3 === 0).map((e) => e.id)])];
  const branchCases = [];
  for (const oldLeaf of [...leaves, null]) {
    for (const target of targets) {
      const collected = C.collectEntriesForBranchSummary(sm, oldLeaf, target);
      branchCases.push({
        oldLeaf,
        target,
        entryIds: collected.entries.map((e) => e.id),
        commonAncestorId: collected.commonAncestorId,
        prepared: BUDGETS.map((budget) => {
          const p = C.prepareBranchEntries(collected.entries, budget);
          return { budget, messages: toJson(p.messages), fileOps: fileOpsJson(p.fileOps), totalTokens: p.totalTokens };
        }),
      });
    }
  }
  sessions[name] = {
    entries: toJson(entries),
    leafId: sm.getLeafId(),
    branchIds: branch.map((e) => e.id),
    compaction: compactionCases,
    turnStarts: branch.map((_, i) => C.findTurnStartIndex(branch, i, 0)),
    lastAssistantUsage: toJson(C.getLastAssistantUsage(branch)),
    estimate: C.estimateContextTokens(projection.messages),
    projectedEstimate: C.estimateProjectedContextTokens(projection, branch),
    messageTokens: projection.messages.map((m) => C.estimateTokens(m)),
    serialized: C.serializeConversation(convertToLlm(projection.messages)),
    branchCases,
  };
}
write("sessions.json", sessions, 0);

// ---------------------------------------------------------------------------------------------
// Pure utils
// ---------------------------------------------------------------------------------------------
const serializeCases = [
  [user("plain"), user([{ type: "text", text: "a" }, image, { type: "text", text: "b" }]), user("")],
  [assistant([{ type: "thinking", thinking: "hidden" }, { type: "text", text: "one" }, { type: "text", text: "two" }, call("1", "read", { path: "a", n: 1, f: 1.5, b: true, z: null, arr: [1, "x"], o: { k: "v\"q" } }), call("2", "noargs", {})])],
  [assistant([call("1", "bash", { command: "echo \"hi\"\nnext" })]), toolResult("1", "bash", "x".repeat(2500)), toolResult("1", "bash", "y".repeat(2000)), toolResult("1", "bash", "")],
  [toolResult("1", "read", "😀".repeat(1500)), toolResult("1", "read", "é".repeat(2001))],
  [{ role: "toolResult", toolCallId: "1", toolName: "x", content: [{ type: "text", text: "p1" }, image, { type: "text", text: "p2" }], isError: true, timestamp: 1 }],
  convertToLlm([bash("ls", "out"), { role: "custom", customType: "c", content: "custom text", display: true, timestamp: 1 }, { role: "branchSummary", summary: "bs", fromId: "x", timestamp: 1 }, { role: "compactionSummary", summary: "cs", tokensBefore: 1, timestamp: 1 }]),
  [system({ content: "sys" }), assistant([])],
].map((messages) => ({ messages: toJson(messages), text: C.serializeConversation(messages) }));
const estimateCases = [
  user("abcd"), user("abcde"), user([{ type: "text", text: "abc" }, image]), user([{ type: "text", text: "" }]),
  assistant([{ type: "text", text: "hello" }, { type: "thinking", thinking: "think" }, call("1", "read", { path: "a.txt", n: 1.0, u: "ü😀" })]),
  toolResult("1", "read", "😀😀😀"), bash("ls", "abc"),
  { role: "custom", customType: "c", content: [{ type: "text", text: "1234567" }, image], display: false, timestamp: 1 },
  { role: "branchSummary", summary: "branch!", fromId: "x", timestamp: 1 },
  { role: "compactionSummary", summary: "compaction", tokensBefore: 3, timestamp: 1 },
  system({ content: "sys", sections: { a: "abc", b: null }, toolsAdded: [{ name: "read", description: "Read files", parameters: { type: "object", properties: { path: { type: "string" } } } }] }),
  system({ content: [{ type: "text", text: "block" }] }),
].map((message) => ({ message: toJson(message), tokens: C.estimateTokens(message) }));
const fileListCases = [
  { read: ["b", "a", "B", "é", "z"], written: ["w"], edited: ["a"] },
  { read: [], written: [], edited: [] },
  { read: ["only"], written: [], edited: [] },
  { read: [], written: ["x", "W"], edited: ["x", "e"] },
].map((ops) => {
  const lists = C.computeFileLists({ read: new Set(ops.read), written: new Set(ops.written), edited: new Set(ops.edited) });
  return { ops, ...lists, formatted: C.formatFileOperations(lists.readFiles, lists.modifiedFiles) };
});
write("utils.json", {
  systemPrompt: C.SUMMARIZATION_SYSTEM_PROMPT,
  serialize: serializeCases,
  estimate: estimateCases,
  fileLists: fileListCases,
  shouldCompact: [[1000, 2000, { enabled: true, reserveTokens: 500, keepRecentTokens: 1 }], [1501, 2000, { enabled: true, reserveTokens: 500, keepRecentTokens: 1 }], [1500, 2000, { enabled: true, reserveTokens: 500, keepRecentTokens: 1 }], [99999, 2000, { enabled: false, reserveTokens: 500, keepRecentTokens: 1 }], [10, 100, { enabled: true, reserveTokens: 200, keepRecentTokens: 1 }]].map(([t, w, s]) => ({ tokens: t, window: w, settings: s, result: C.shouldCompact(t, w, s) })),
  calculateContextTokens: [usage(10), { ...usage(0), input: 1, output: 2, cacheRead: 3, cacheWrite: 4 }].map((u) => ({ usage: u, tokens: C.calculateContextTokens(u) })),
  defaults: C.DEFAULT_COMPACTION_SETTINGS,
});

// ---------------------------------------------------------------------------------------------
// LLM calls: recorded requests
// ---------------------------------------------------------------------------------------------
const model = (extra = {}) => ({
  id: "sum-1", name: "Summarizer", api: "faux", provider: "faux", baseUrl: "", reasoning: true, input: ["text"],
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0 }, contextWindow: 50000, maxTokens: 4096, ...extra,
});
const reply = (spec) => ({
  role: "assistant", content: spec.content ?? [{ type: "text", text: spec.text ?? "SUMMARY" }], api: "faux", provider: "faux", model: "sum-1",
  usage: spec.usage ?? usage(10), stopReason: spec.stopReason ?? "stop", ...(spec.errorMessage ? { errorMessage: spec.errorMessage } : {}), timestamp: 0,
});
function recorder(responses) {
  const requests = [];
  let i = 0;
  const streamFn = async (m, context, options) => {
    const { signal: _s, ...rest } = options;
    requests.push(toJson({
      model: m.id,
      context: { messages: context.messages.map((msg) => (msg.role === "user" ? { ...msg, timestamp: 0 } : msg)) },
      options: { ...rest, sessionId: rest.sessionId === "route-1" ? "route-1" : rest.sessionId === undefined ? undefined : "<uuidv7>" },
    }));
    const spec = responses[Math.min(i++, responses.length - 1)];
    return { result: async () => reply(spec) };
  };
  return { requests, streamFn };
}
const events = () => {
  const log = [];
  return {
    log,
    callbacks: {
      onRetryScheduled: (attempt, maxAttempts, delayMs, errorMessage) => log.push({ type: "scheduled", attempt, maxAttempts, delayMs, errorMessage }),
      onRetryAttemptStart: () => log.push({ type: "attempt_start" }),
      onRetryFinished: (success, attempt, finalError) => log.push({ type: "finished", success, attempt, ...(finalError !== undefined ? { finalError } : {}) }),
    },
  };
};
const branchOf = (name) => managers[name].getBranch();
const prep = (name, keep) => C.prepareCompaction(branchOf(name), { enabled: true, reserveTokens: 16384, keepRecentTokens: keep });
const retry = { enabled: true, maxRetries: 3, baseDelayMs: 1 };

const compactCases = [];
async function compactCase(label, { session, keep, model: m = model(), apiKey = "key-1", headers, customInstructions, thinkingLevel, responses = [{}], retryPolicy, sessionId }) {
  const preparation = prep(session, keep);
  const rec = recorder(responses);
  const ev = events();
  let result;
  let error;
  try {
    result = await C.compact(preparation, m, apiKey, headers, customInstructions, undefined, thinkingLevel, rec.streamFn, undefined, retryPolicy, ev.callbacks, sessionId);
  } catch (e) {
    error = e.message;
  }
  compactCases.push(toJson({ label, session, keep, model: m, apiKey, headers, customInstructions, thinkingLevel, replies: responses.map(reply), retryPolicy, sessionId, preparation: prepJson(preparation), requests: rec.requests, events: ev.log, result, error }));
}
await compactCase("simple", { session: "plain-chat", keep: 200 });
await compactCase("custom-instructions", { session: "plain-chat", keep: 200, customInstructions: "Focus on tests", headers: { "x-h": "1" } });
await compactCase("thinking-high", { session: "plain-chat", keep: 200, thinkingLevel: "high" });
await compactCase("thinking-off", { session: "plain-chat", keep: 200, thinkingLevel: "off" });
await compactCase("thinking-nonreasoning", { session: "plain-chat", keep: 200, thinkingLevel: "high", model: model({ reasoning: false }) });
await compactCase("session-id", { session: "plain-chat", keep: 200, sessionId: "route-1" });
await compactCase("split-turn", { session: "long-single-turn", keep: 400, responses: [{ text: "HISTORY", usage: usage(7, { reasoning: 2 }) }, { text: "PREFIX", usage: usage(5, { cacheWrite1h: 1 }) }] });
await compactCase("split-turn-custom", { session: "long-single-turn", keep: 400, customInstructions: "keep ids", responses: [{ text: "HISTORY" }, { text: "PREFIX" }] });
await compactCase("files", { session: "tool-turns", keep: 400 });
await compactCase("previous-summary", { session: "repeated-compaction", keep: 50 });
await compactCase("previous-hook-summary", { session: "double-compaction", keep: 50 });
await compactCase("bash-custom", { session: "bash-and-custom", keep: 50 });
await compactCase("nested", { session: "nested-calls", keep: 100 });
await compactCase("error", { session: "plain-chat", keep: 200, responses: [{ stopReason: "error", errorMessage: "invalid api key" }] });
await compactCase("error-empty", { session: "plain-chat", keep: 200, responses: [{ stopReason: "error" }] });
await compactCase("length", { session: "plain-chat", keep: 200, responses: [{ stopReason: "length" }] });
await compactCase("tool-call", { session: "plain-chat", keep: 200, responses: [{ content: [call("t", "read", {})] }] });
await compactCase("prefix-error", { session: "long-single-turn", keep: 400, responses: [{}, { stopReason: "length" }] });
await compactCase("prefix-tool-call", { session: "long-single-turn", keep: 400, responses: [{}, { content: [call("t", "read", {})] }] });
await compactCase("aborted", { session: "plain-chat", keep: 200, responses: [{ stopReason: "aborted", content: [] }] });
await compactCase("multi-text", { session: "plain-chat", keep: 200, responses: [{ content: [{ type: "thinking", thinking: "x" }, { type: "text", text: "A" }, { type: "text", text: "B" }] }] });
await compactCase("retry-success", { session: "plain-chat", keep: 200, retryPolicy: retry, responses: [{ stopReason: "error", errorMessage: "overloaded_error" }, { stopReason: "error", errorMessage: "fetch failed" }, { text: "RECOVERED" }] });
await compactCase("retry-exhausted", { session: "plain-chat", keep: 200, retryPolicy: { enabled: true, maxRetries: 2, baseDelayMs: 1 }, responses: [{ stopReason: "error", errorMessage: "503 Service Unavailable" }] });
await compactCase("retry-nonretryable", { session: "plain-chat", keep: 200, retryPolicy: retry, responses: [{ stopReason: "error", errorMessage: "insufficient_quota: 429" }] });
await compactCase("retry-disabled", { session: "plain-chat", keep: 200, retryPolicy: { enabled: false, maxRetries: 3, baseDelayMs: 1 }, responses: [{ stopReason: "error", errorMessage: "overloaded" }] });
await compactCase("retry-then-abort", { session: "plain-chat", keep: 200, retryPolicy: retry, responses: [{ stopReason: "error", errorMessage: "terminated" }, { stopReason: "aborted", content: [] }] });
await compactCase("retry-then-fatal", { session: "plain-chat", keep: 200, retryPolicy: retry, responses: [{ stopReason: "error", errorMessage: "socket hang up" }, { stopReason: "error", errorMessage: "bad request" }] });

const branchCases = [];
async function branchCase(label, { session, oldLeaf, target, m = model(), customInstructions, replaceInstructions, reserveTokens, responses = [{}], retryPolicy, headers }) {
  const sm = managers[session];
  const entries = sm.getEntries();
  const old = oldLeaf === "leaf" ? sm.getLeafId() : entries[oldLeaf].id;
  const tgt = entries[target].id;
  const { entries: toSummarize } = C.collectEntriesForBranchSummary(sm, old, tgt);
  const rec = recorder(responses);
  const ev = events();
  const result = await C.generateBranchSummary(toSummarize, {
    model: m, apiKey: "key-b", headers, signal: new AbortController().signal, customInstructions, replaceInstructions, reserveTokens,
    streamFn: rec.streamFn, retry: retryPolicy, callbacks: ev.callbacks,
  });
  branchCases.push(toJson({ label, session, entryIds: toSummarize.map((e) => e.id), model: m, headers, customInstructions, replaceInstructions, reserveTokens, replies: responses.map(reply), retryPolicy, requests: rec.requests, events: ev.log, result }));
}
await branchCase("left-branch", { session: "branches", oldLeaf: 5, target: 9 });
await branchCase("custom", { session: "branches", oldLeaf: 5, target: 9, customInstructions: "Mention files", headers: { a: "b" } });
await branchCase("replace", { session: "branches", oldLeaf: 5, target: 9, customInstructions: "Only list files", replaceInstructions: true });
await branchCase("replace-without-text", { session: "branches", oldLeaf: 5, target: 9, replaceInstructions: true });
await branchCase("nested-summaries", { session: "branches", oldLeaf: "leaf", target: 1 });
await branchCase("tight-budget", { session: "branches", oldLeaf: 9, target: 0, m: model({ contextWindow: 16384 + 60 }) });
await branchCase("negative-budget", { session: "branches", oldLeaf: 9, target: 0, reserveTokens: 999999 });
await branchCase("zero-window", { session: "branches", oldLeaf: 9, target: 0, m: model({ contextWindow: 0 }), reserveTokens: 127990 });
await branchCase("aborted", { session: "branches", oldLeaf: 5, target: 9, responses: [{ stopReason: "aborted", content: [] }] });
await branchCase("error", { session: "branches", oldLeaf: 5, target: 9, responses: [{ stopReason: "error", errorMessage: "nope" }] });
await branchCase("length", { session: "branches", oldLeaf: 5, target: 9, responses: [{ stopReason: "length" }] });
await branchCase("tool-call", { session: "branches", oldLeaf: 5, target: 9, responses: [{ content: [call("t", "read", {})] }] });
await branchCase("empty-text", { session: "branches", oldLeaf: 5, target: 9, responses: [{ content: [] }] });
await branchCase("retry", { session: "branches", oldLeaf: 5, target: 9, retryPolicy: retry, responses: [{ stopReason: "error", errorMessage: "rate limit exceeded" }, {}] });
await branchCase("compaction-entries", { session: "repeated-compaction", oldLeaf: "leaf", target: 0 });
write("llm.json", { compact: compactCases, branch: branchCases });

// ---------------------------------------------------------------------------------------------
// Error classification
// ---------------------------------------------------------------------------------------------
const retrySource = readFileSync(join(PI, "node_modules/@earendil-works/pi-ai/dist/utils/retry.js"), "utf8");
const arrays = [...retrySource.matchAll(/buildProviderErrorPattern\(\[([\s\S]*?)\n\]\)/g)].map((m) => Function(`return [${m[1]}]`)());
const overflow = getOverflowPatterns();
const cerebrasSrc = readFileSync(join(PI, "node_modules/@earendil-works/pi-ai/dist/utils/overflow.js"), "utf8");
const cerebras = cerebrasSrc.match(/CEREBRAS_BODYLESS_OVERFLOW_PATTERN = \/(.*)\/i;/)[1];
const nonOverflow = [...cerebrasSrc.match(/NON_OVERFLOW_PATTERNS = \[([\s\S]*?)\];/)[1].matchAll(/\/(.+)\/i,/g)].map((m) => m[1]);
const allSources = [...overflow.map((r) => r.source), cerebras, ...nonOverflow, ...arrays[0], ...arrays[1]];
const corpus = [
  "prompt is too long: 213462 tokens > 200000 maximum", "Prompt too long", "prompt exceeds max length",
  '413 {"error":{"type":"request_too_large","message":"Request exceeds the maximum size"}}',
  "Input is too long for requested model.", "Your input exceeds the context window of this model",
  "Requested token count exceeds the model's maximum context length of 131072 tokens",
  "Input length (265330) exceeds model's maximum context length (262144).", "exceeds maximum context length (1,000)",
  "exceeds the models maximum context length of 12 token", "exceeds maximum context length",
  "The input token count (1196265) exceeds the maximum number of tokens allowed (1048575)",
  "This model's maximum prompt length is 131072 but the request contains 537812 tokens",
  "Please reduce the length of the messages or completion",
  "This endpoint's maximum context length is 8192 tokens. However, you requested about 9000 tokens",
  "Input length 5000 exceeds the maximum allowed input length of 4,096 tokens.",
  "The input (9000 tokens) is longer than the model's context length (8192 tokens).",
  "the request exceeds the available context size, try increasing it",
  "tokens to keep from the initial prompt is greater than the context length",
  "prompt token count of 200000 exceeds the limit of 128000",
  'github-copilot API error (413): {"message":"failed to parse request","code":""}',
  "invalid params, context window exceeds limit", "Your request exceeded model token limit: 262144 (requested: 300000)",
  "Prompt has 70,000 tokens, but the configured context size is 65,536 tokens",
  "400 status code (no body)", "413 (no body)", "413 status code (no body)", "4000 status code (no body)",
  "Prompt contains 140000 tokens and 0 draft tokens, too large for model with 131072 maximum context length",
  '{"code":"1261","message":"Prompt too long"}', "model_context_window_exceeded",
  "prompt too long; exceeded max context length by 1200 tokens", "Range of input length should be [1, 98304]",
  "context_length_exceeded", "context length exceeded", "Too many tokens", "token limit exceeded",
  "413", "413: too big", "413 Payload Too Large", "Anthropic API error (413): request too large", "error 413 occurred",
  "ThrottlingException: Too many tokens, please wait before trying again.",
  "Throttling error: Too many tokens, please wait", "Service unavailable: too many tokens", "rate limit: too many tokens",
  "Too Many Requests: token limit exceeded",
  "overloaded_error", "Overloaded", "server_busy", "Servers are currently busy", "We are currently experiencing high demand",
  "model is at capacity", "rate limit exceeded", "ratelimit", "rate_limit_error", "429 Too Many Requests", "HTTP 500", "502 Bad Gateway",
  "status 503", "error 504", "520", "524 timeout", "request id req_5290", "sent 4290 bytes", "x429", "429x", "a500b", "(500)",
  "The service is temporarily unavailable", "service unavailable", "Service_Unavailable", "server error", "Internal error", "internal_error",
  "Provider returned error", "The upstream provider returned an error", "exceeded request buffer limit while retrying upstream",
  "unable to forward upstream response", "upstream request failed", "upstream_stream_read_error", "stream was interrupted",
  "HTTP/2 stream failed", "http2 stream failed", "network error", "Connection error.", "connection refused", "connection lost",
  "other side closed", "fetch failed", "request to https://oauth2.googleapis.com/token failed, reason: ", "request to  failed, reason:",
  "getaddrinfo ENOTFOUND api.example.com", "EAI_AGAIN", "upstream connect error or disconnect/reset before headers",
  "socket hang up", "The socket connection was closed unexpectedly", "Request timed out", "timeout", "time out", "terminated",
  "WebSocket closed 1006", "websocket error", "stream ended without message_stop", "Anthropic stream ended before message_stop",
  "stream ended before a terminal response event", "http2 request did not get a response",
  "stream disconnected before completion: stream closed before response.completed", "pending stream has been canceled",
  "Provider retry delay 120000ms exceeds cap", "You can retry your request", "Please try your request again", "please retry your request",
  "ResourceExhausted", "subscription_sharing_usage_unavailable", "subscription_sharing_user_unavailable",
  "GoUsageLimitError", "FreeUsageLimitError: 429", "Monthly usage limit reached", "insufficient available balance",
  "insufficient_quota", "You are out of budget", "Quota exceeded for 503", "billing hard limit", "subscription_sharing_usage_limit_exceeded",
  "subscription_sharing_workplace_individual_usage_limit_exceeded", "invalid api key", "bad request", "", "OVERLOADED",
  "Rate-Limit", "line1\nrate limit", "INTERNAL ERROR", "Ünïcödé 503 fehler", "timeouts", "retry delay", "connection\terror",
];
const classify = (message) => ({
  message,
  matches: allSources.map((s, i) => (new RegExp(s, "i").test(message) ? i : -1)).filter((i) => i >= 0),
});
const overflowCases = [];
for (const errorMessage of corpus) {
  for (const provider of ["faux", "cerebras"]) {
    const msg = reply({ stopReason: "error", errorMessage });
    msg.provider = provider;
    overflowCases.push({ message: toJson(msg), contextWindow: 0, overflow: AI.isContextOverflow(msg), retryable: AI.isRetryableAssistantError(msg) });
  }
}
for (const [stopReason, input, cacheRead, output, cw] of [
  ["stop", 900, 200, 5, 1000], ["stop", 900, 100, 5, 1000], ["stop", 2000, 0, 5, 0], ["length", 990, 0, 0, 1000],
  ["length", 989, 0, 0, 1000], ["length", 995, 0, 1, 1000], ["toolUse", 5000, 0, 0, 1000], ["error", 5000, 0, 0, 1000],
]) {
  const msg = reply({ stopReason, usage: { ...usage(0), input, cacheRead, output } });
  overflowCases.push({ message: toJson(msg), contextWindow: cw, overflow: AI.isContextOverflow(msg, cw), retryable: AI.isRetryableAssistantError(msg) });
}
write("errors.json", {
  sources: { overflow: overflow.map((r) => r.source), cerebras, nonOverflow, nonRetryable: arrays[0], retryable: arrays[1] },
  patterns: corpus.map(classify),
  cases: overflowCases,
  retryDelay: [[2000, undefined, 1], [2000, undefined, 2], [2000, undefined, 6], [2000, 5000, 3], [1, undefined, 0], [2000, 1e20, 60]].map(([baseDelayMs, maxAgentDelayMs, attempt]) => ({ baseDelayMs, maxAgentDelayMs: maxAgentDelayMs ?? null, attempt, delay: AI.retryDelayMs({ baseDelayMs, maxAgentDelayMs }, attempt) })),
});
console.error(`wrote fixtures to ${out}`);
