// Copies real pi sessions with private text scrubbed; structure, ids and timestamps kept.
// Usage: node scrub.mjs <out-dir> <name>=<session.jsonl> ...
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const KEEP = new Set(["type", "id", "parentId", "timestamp", "role", "provider", "model", "api", "stopReason",
  "toolName", "toolCallId", "mimeType", "customType", "modelId", "thinkingLevel", "kind",
  "responseId", "providerThinkingLevel", "status", "targetId", "firstKeptEntryId", "fromId", "rawStopReason", "name"]);
const [out, ...pairs] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
for (const pair of pairs) {
  const [name, file] = pair.split("=");
  let n = 0;
  const scrub = (value, key, parent) => {
    if (typeof value === "string") {
      if (value === "") return value;
      if (key === "cwd") return "/work/project";
      if (key === "data" && parent?.type === "image") return "AA==";
      if (KEEP.has(key) && !(key === "name" && parent?.type === "session_info")) return value;
      return `t${++n}`;
    }
    if (key === "parameters" || key === "constrainedSampling") return value; // public tool schemas
    if (Array.isArray(value)) return value.map((v) => scrub(v, key, parent));
    if (value && typeof value === "object") {
      const o = {};
      for (const [k, v] of Object.entries(value)) o[k] = scrub(v, k, value);
      return o;
    }
    return value;
  };
  const lines = readFileSync(file, "utf8").split("\n").filter((l) => l.trim());
  const scrubbed = lines.map((l) => {
    const e = JSON.parse(l);
    if (e.type === "session") return JSON.stringify({ ...e, cwd: "/work/project", ...(e.parentSession ? { parentSession: "/work/parent.jsonl" } : {}) });
    return JSON.stringify(scrub(e, "", null));
  });
  writeFileSync(join(out, `${name}.jsonl`), `${scrubbed.join("\n")}\n`);
}
