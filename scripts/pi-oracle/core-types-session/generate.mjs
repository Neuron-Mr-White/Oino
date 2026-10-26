// Generates synthetic v3 session fixtures with pi's own SessionManager.
// Usage: node generate.mjs <out-dir>   (writes <out-dir>/<scenario>.jsonl)
import { copyFileSync, mkdirSync, mkdtempSync, readdirSync, rmSync } from "node:fs";
import { join } from "node:path";
import { loadSessionManager } from "./lib.mjs";

const { SessionManager } = await loadSessionManager();
const out = process.argv[2];
if (!out) throw new Error("usage: generate.mjs <out-dir>");
mkdirSync(out, { recursive: true });
const work = mkdtempSync(join(process.env.OINO_TEST_TMP ?? out, ".gen-"));

let clock = Date.parse("2026-01-01T00:00:00.000Z");
const tick = () => (clock += 1000);
const zeroUsage = () => ({
  input: 0, output: 0, cacheRead: 0, cacheWrite: 0, totalTokens: 0,
  cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 },
});
const user = (content) => ({ role: "user", content, timestamp: tick() });
const assistant = (content, extra = {}) => ({
  role: "assistant",
  content: typeof content === "string" ? [{ type: "text", text: content }] : content,
  api: "anthropic-messages", provider: "anthropic", model: "claude-sonnet-4-5",
  usage: { input: 10, output: 5, cacheRead: 2, cacheWrite: 1, totalTokens: 18,
    cost: { input: 0.00003, output: 0.000075, cacheRead: 0.0000006, cacheWrite: 0.00000375, total: 0.00010935 } },
  stopReason: "stop", timestamp: tick(), ...extra,
});
const system = (extra) => ({ role: "system", content: "", timestamp: tick(), ...extra });
const tool = (name) => ({ name, description: `${name} tool`, parameters: { type: "object", properties: { path: { type: "string" } } } });
const toolResult = (toolCallId, text, extra = {}) => ({
  role: "toolResult", toolCallId, toolName: "read", content: [{ type: "text", text }], isError: false, timestamp: tick(), ...extra,
});
const bash = (extra) => ({ role: "bashExecution", command: "ls", output: "a\nb", exitCode: 0, cancelled: false, truncated: false, timestamp: tick(), ...extra });

const scenarios = {
  "plain-chat": (sm) => {
    sm.appendMessage(user("Hello"));
    sm.appendMessage(assistant("Hi!"));
    sm.appendMessage(user([{ type: "text", text: "again" }]));
    sm.appendMessage(assistant("Sure."));
  },
  "system-and-tools": (sm) => {
    sm.appendMessage(system({ content: "base prompt", sections: { preamble: "You are a coding assistant.", cwd: "/work/project" }, toolsAdded: [tool("read"), tool("write")] }));
    sm.appendMessage(user("hi"));
    sm.appendMessage(assistant("hello"));
    sm.appendMessage(system({ sections: { skills: "<skills/>", cwd: null }, toolsRemoved: [{ name: "write" }] }));
    sm.appendMessage(user("next"));
  },
  "tool-calls": (sm) => {
    sm.appendMessage(user("read a.txt"));
    sm.appendMessage(assistant([{ type: "thinking", thinking: "plan", thinkingSignature: "sig" }, { type: "toolCall", id: "call_1", name: "read", arguments: { path: "a.txt" } }], { stopReason: "toolUse", responseId: "resp_1", thinkingLevel: "high", durationMs: 42 }));
    sm.appendMessage(toolResult("call_1", "contents", { details: { lines: 1 }, durationMs: 3 }));
    sm.appendMessage(assistant("done"));
  },
  "bash-executions": (sm) => {
    sm.appendMessage(user("run"));
    sm.appendMessage(bash({}));
    sm.appendMessage(bash({ command: "false", output: "", exitCode: 1 }));
    sm.appendMessage(bash({ command: "sleep 9", output: "", exitCode: undefined, cancelled: true }));
    sm.appendMessage(bash({ command: "cat big", output: "x", truncated: true, fullOutputPath: "/tmp/full.log" }));
    sm.appendMessage(bash({ command: "secret", excludeFromContext: true }));
    sm.appendMessage(assistant("ok"));
  },
  "custom-messages": (sm) => {
    sm.appendCustomMessageEntry("my-ext", "Injected context", true);
    sm.appendMessage(user("hi"));
    sm.appendCustomMessageEntry("my-ext", [{ type: "text", text: "block" }, { type: "image", data: "iVBORw0KGgo=", mimeType: "image/png" }], false, { a: 1 });
    sm.appendCustomEntry("my-ext-state", { count: 42 });
    sm.appendCustomEntry("no-data");
    sm.appendMessage(assistant("seen"));
  },
  "model-thinking": (sm) => {
    sm.appendModelChange("openai", "gpt-4o");
    sm.appendThinkingLevelChange("high");
    sm.appendMessage(user("hi"));
    sm.appendMessage(assistant("hey"));
    sm.appendThinkingLevelChange("xhigh");
  },
  "model-change-after-assistant": (sm) => {
    sm.appendMessage(user("hi"));
    sm.appendMessage(assistant("hey"));
    sm.appendModelChange("openai", "gpt-5");
  },
  "usage-entries": (sm) => {
    sm.appendMessage(user("hi"));
    sm.appendUsage("cache_warm", "anthropic", "claude-sonnet-4-5", { ...zeroUsage(), cacheRead: 50000, totalTokens: 50000, cost: { input: 0, output: 0, cacheRead: 0.015, cacheWrite: 0, total: 0.015 } });
    sm.appendUsage("custom_kind", "p", "m", zeroUsage(), "a note");
    sm.appendMessage(assistant("hey"));
  },
  "compaction-keep": (sm) => {
    sm.appendMessage(system({ content: "sys", toolsAdded: [tool("read")] }));
    sm.appendMessage(user("one"));
    sm.appendMessage(assistant("1"));
    const keep = sm.appendMessage(user("two"));
    sm.appendMessage(system({ sections: { extra: "E" } }));
    sm.appendMessage(assistant("2"));
    sm.appendCompaction("Summary of one", keep, 1234, { readFiles: ["a"], modifiedFiles: [] }, false, zeroUsage());
    sm.appendMessage(user("three"));
    sm.appendMessage(assistant("3"));
  },
  "compaction-retain-none": (sm) => {
    sm.appendMessage(user("one"));
    sm.appendMessage(assistant("1"));
    sm.appendCompaction("Everything", null, 99);
    sm.appendMessage(user("after"));
  },
  "compaction-no-system": (sm) => {
    sm.appendMessage(user("one"));
    const keep = sm.appendMessage(assistant("1"));
    sm.appendCompaction("S", keep, 5, undefined, true);
    sm.appendMessage(user("two"));
  },
  "compaction-twice": (sm) => {
    sm.appendMessage(system({ content: "sys" }));
    sm.appendMessage(user("a"));
    const k1 = sm.appendMessage(assistant("A"));
    sm.appendCompaction("first", k1, 10);
    const k2 = sm.appendMessage(user("b"));
    sm.appendMessage(assistant("B"));
    sm.appendCompaction("second", sm.getBranch()[2].id, 20);
    sm.appendMessage(user("c"));
    void k2;
  },
  "compaction-first-kept-off-path": (sm) => {
    const a = sm.appendMessage(user("a"));
    const other = sm.appendMessage(assistant("A"));
    sm.branch(a);
    sm.appendMessage(assistant("A2"));
    sm.appendCompaction("off-path", other, 7);
    sm.appendMessage(user("z"));
  },
  "context-edit-user": (sm) => {
    const u = sm.appendMessage(user("original"));
    sm.appendMessage(assistant("x"));
    sm.appendContextEdit(u, { content: "replaced" });
    sm.appendMessage(user("next"));
  },
  "context-edit-assistant-string": (sm) => {
    sm.appendMessage(user("q"));
    const a = sm.appendMessage(assistant([{ type: "thinking", thinking: "t" }, { type: "text", text: "long" }]));
    sm.appendContextEdit(a, { content: "short" });
  },
  "context-edit-omit-tool-result": (sm) => {
    sm.appendMessage(user("q"));
    sm.appendMessage(assistant([{ type: "toolCall", id: "c1", name: "read", arguments: {} }], { stopReason: "toolUse" }));
    const r = sm.appendMessage(toolResult("c1", "huge"));
    sm.appendContextEdit(r, null);
    sm.appendMessage(assistant("fine"));
  },
  "context-edit-blocks": (sm) => {
    const u = sm.appendMessage(user("q"));
    sm.appendMessage(assistant("a"));
    sm.appendContextEdit(u, { content: [{ type: "text", text: "b1" }, { type: "image", data: "AA==", mimeType: "image/png" }] });
  },
  "context-edit-latest-wins": (sm) => {
    const u = sm.appendMessage(user("v0"));
    sm.appendContextEdit(u, { content: "v1" });
    sm.appendContextEdit(u, null);
    sm.appendContextEdit(u, { content: "v3" });
    sm.appendMessage(assistant("ok"));
  },
  "context-edit-branch-relative": (sm) => {
    const u = sm.appendMessage(user("orig"));
    const a = sm.appendMessage(assistant("a"));
    sm.appendContextEdit(u, { content: "edited" });
    sm.appendMessage(user("on edited branch"));
    sm.branch(a);
    sm.appendMessage(user("on original branch"));
  },
  "context-edit-custom-message": (sm) => {
    sm.appendMessage(user("q"));
    const c = sm.appendCustomMessageEntry("ext", "visible", true);
    sm.appendContextEdit(c, { content: "trimmed" });
    sm.appendMessage(assistant("ok"));
  },
  "context-edit-with-compaction": (sm) => {
    sm.appendMessage(user("old"));
    const keep = sm.appendMessage(user("kept"));
    sm.appendMessage(assistant("A"));
    sm.appendContextEdit(keep, { content: "kept-edited-before" });
    sm.appendCompaction("sum", keep, 3);
    sm.appendContextEdit(keep, { content: "kept-edited-after" });
    sm.appendMessage(user("new"));
  },
  "labels": (sm) => {
    const u = sm.appendMessage(user("hi"));
    const a = sm.appendMessage(assistant("yo"));
    sm.appendLabelChange(u, "checkpoint-1");
    sm.appendLabelChange(a, "answer");
    sm.appendLabelChange(a, undefined);
    sm.appendLabelChange(u, "checkpoint-2");
    sm.appendMessage(user("more"));
  },
  "session-names": (sm) => {
    sm.appendSessionInfo("First name");
    sm.appendMessage(user("hi"));
    sm.appendSessionInfo("Multi\r\n\nline  name ");
    sm.appendMessage(assistant("ok"));
  },
  "session-name-cleared": (sm) => {
    sm.appendMessage(user("hi"));
    sm.appendSessionInfo("Named");
    sm.appendSessionInfo("   ");
  },
  "branches": (sm) => {
    const u1 = sm.appendMessage(user("start"));
    const a1 = sm.appendMessage(assistant("a1"));
    sm.appendMessage(user("path one"));
    sm.appendMessage(assistant("one"));
    sm.branch(a1);
    sm.appendMessage(user("path two"));
    sm.appendMessage(assistant("two"));
    sm.branch(u1);
    sm.appendMessage(assistant("alt answer"));
  },
  "branch-with-summary": (sm) => {
    const u1 = sm.appendMessage(user("start"));
    sm.appendMessage(assistant("a"));
    sm.appendMessage(user("explore"));
    sm.branchWithSummary(u1, "Branch explored approach A", { readFiles: ["x"], modifiedFiles: [] }, false, zeroUsage());
    sm.appendMessage(assistant("continuing"));
  },
  "branch-summary-empty": (sm) => {
    const u1 = sm.appendMessage(user("start"));
    sm.appendMessage(assistant("a"));
    sm.branchWithSummary(u1, "");
    sm.appendMessage(user("next"));
  },
  "branch-summary-root": (sm) => {
    sm.appendMessage(user("start"));
    sm.appendMessage(assistant("a"));
    sm.branchWithSummary(null, "From scratch", undefined, true);
    sm.appendMessage(user("fresh"));
  },
  "reset-leaf": (sm) => {
    sm.appendMessage(user("first root"));
    sm.appendMessage(assistant("a"));
    sm.resetLeaf();
    sm.appendMessage(user("second root"));
    sm.appendMessage(assistant("b"));
  },
  "images-and-unicode": (sm) => {
    sm.appendMessage(user([{ type: "text", text: "what is this? 🦀 ünïcödé \u0000 \u2028" }, { type: "image", data: "R0lGODlhAQABAAAAACw=", mimeType: "image/gif" }]));
    sm.appendMessage(assistant([{ type: "text", text: "a crab 🦀", textSignature: "{\"v\":1,\"id\":\"msg_1\"}" }]));
  },
  "assistant-errors": (sm) => {
    sm.appendMessage(user("q"));
    sm.appendMessage(assistant([], { stopReason: "error", errorMessage: "rate limit exceeded (429)", diagnostics: [{ type: "retry", timestamp: tick(), error: { name: "Error", message: "429", code: 429 } }] }));
    sm.appendMessage(assistant([{ type: "text", text: "partial" }], { stopReason: "aborted", errorMessage: "Request was aborted" }));
    sm.appendMessage(assistant("ok", { stopReason: "length", rawStopReason: "max_tokens", endTurn: false, responseModel: "claude-x", providerThinkingLevel: "high" }));
  },
  "tool-result-extras": (sm) => {
    sm.appendMessage(user("q"));
    sm.appendMessage(assistant([{ type: "toolCall", id: "c1", name: "agent", arguments: { task: "x" }, thoughtSignature: "ts", namespace: "ns", argumentsError: "bad" }], { stopReason: "toolUse" }));
    sm.appendMessage(toolResult("c1", "r", { isError: true, usage: zeroUsage(), nestedCalls: { calls: [{ id: "n1", name: "read", arguments: { p: 1 }, status: "ok", durationMs: 2 }, { id: "n2", name: "bash", argumentsBytes: 9000, status: "unfinished" }], complete: false }, details: null }));
  },
  "long-linear": (sm) => {
    for (let i = 0; i < 40; i++) {
      sm.appendMessage(user(`question ${i}`));
      sm.appendMessage(assistant(`answer ${i}`));
    }
  },
};

for (const [name, build] of Object.entries(scenarios)) {
  const dir = join(work, name);
  const sm = SessionManager.create("/work/project", dir);
  build(sm);
  const files = readdirSync(dir).filter((f) => f.endsWith(".jsonl"));
  if (files.length !== 1) throw new Error(`${name}: expected one file, got ${files.length}`);
  copyFileSync(join(dir, files[0]), join(out, `${name}.jsonl`));
}

// /clone of a labelled branch (pi createBranchedSession, persisted).
{
  const dir = join(work, "clone");
  const sm = SessionManager.create("/work/project", dir);
  const u = sm.appendMessage(user("hi"));
  sm.appendLabelChange(u, "lbl");
  const a = sm.appendMessage(assistant("yo"));
  sm.appendCompaction("sum", sm.getBranch()[1].id, 1);
  sm.appendMessage(user("x"));
  void a;
  const file = sm.createBranchedSession(sm.getLeafId());
  copyFileSync(file, join(out, "clone-branched.jsonl"));
}

rmSync(work, { recursive: true, force: true });
console.log(`wrote ${Object.keys(scenarios).length + 1} scenarios to ${out}`);
