// Copies pre-v1 Oino session files with private text scrubbed (structure and UUIDs kept).
// Usage: node scrub-legacy.mjs <out-dir> <name>=<legacy.jsonl> ...
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";

const KEEP = new Set(["record", "kind", "id", "parent", "leaf_id", "session_id", "tool_call_id", "tool_name",
  "name", "stop_reason", "provider", "native_id", "media_type", "usage_kind", "thinking_level", "currency", "target"]);
const [out, ...pairs] = process.argv.slice(2);
mkdirSync(out, { recursive: true });
for (const pair of pairs) {
  const [name, file] = pair.split("=");
  let n = 0;
  const scrub = (value, key) => {
    if (typeof value === "string") {
      if (value === "") return value;
      if (key === "cwd") return "/work/project";
      return KEEP.has(key) ? value : `t${++n}`;
    }
    if (Array.isArray(value)) return value.map((v) => scrub(v, key));
    if (value && typeof value === "object") {
      return Object.fromEntries(Object.entries(value).map(([k, v]) => [k, scrub(v, k)]));
    }
    return value;
  };
  const lines = readFileSync(file, "utf8").split("\n").filter((l) => l.trim());
  writeFileSync(join(out, `${name}.jsonl`), `${lines.map((l) => JSON.stringify(scrub(JSON.parse(l), ""))).join("\n")}\n`);
}
