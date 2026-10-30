// pi oracle: WP-C2 tool-dispatch record + resume repair (core spec §3 parity gate).
//
// Loads the post-repair fixture (oino.tool-dispatch, the repair toolResult and
// oino.resume-repair — produced by oino's `repair_interrupted_turn`, see
// crates/oino-session/tests/repair.rs) with pi's own SessionManager and asserts:
//   1. pi loads it: entry count and per-entry shapes match,
//   2. buildSessionContext is identical to the custom-entry-free twin (custom entries
//      never project into context),
//   3. a torn physical last line stays readable (pi skips it — the state our torn-tail
//      "\n" repair append leaves behind).
//
// Usage: node scripts/pi-oracle/core/session-load.mjs   (PI_CODING_AGENT_PKG overrides pi)
import { copyFileSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { loadSessionManager } from "../core-types-session/lib.mjs";

const { SessionManager } = await loadSessionManager();
const here = fileURLToPath(new URL(".", import.meta.url));
const fixtures = join(here, "fixtures");
const repaired = readFileSync(join(fixtures, "resume-repair.jsonl"), "utf8");

const lines = repaired.split("\n").filter((l) => l.trim() !== "");
const custom = (line) => {
  const value = JSON.parse(line);
  return value.type === "custom" && String(value.customType).startsWith("oino.");
};
const customCount = lines.filter(custom).length;

// The custom-entry-free twin: the same session as if the oino.* records had never been
// written. Custom entries are links in the parent chain, so removing one reparents its
// children onto its (rewired) parent — otherwise the tail would orphan.
const removed = new Map();
for (const line of lines.filter(custom)) {
  const entry = JSON.parse(line);
  removed.set(entry.id, entry.parentId ?? null);
}
const resolveParent = (id) => {
  let parent = id;
  while (removed.has(parent)) parent = removed.get(parent);
  return parent;
};
const twin =
  lines
    .filter((l) => !custom(l))
    .map((l) => {
      const entry = JSON.parse(l);
      if (entry.parentId != null && removed.has(entry.parentId)) {
        const parent = resolveParent(entry.parentId);
        entry.parentId = parent;
      }
      return JSON.stringify(entry);
    })
    .join("\n") + "\n";

const work = mkdtempSync(join(fixtures, ".load-"));
const open = (name, text) => {
  const file = join(work, name);
  writeFileSync(file, text);
  return SessionManager.open(file, work);
};

const full = open("full.jsonl", repaired);
const bare = open("twin.jsonl", twin);

const failed = [];
const check = (ok, what) => {
  console.log(`${ok ? "ok" : "FAIL"}  ${what}`);
  if (!ok) failed.push(what);
};

// 1. pi loads the fixture: entry count and shapes.
check(full.getEntryCount() === lines.length - 1, `entry count: ${full.getEntryCount()} === ${lines.length - 1} (header excluded)`);
const kinds = full.getEntries().map((e) => e.type);
check(
  JSON.stringify(kinds) === JSON.stringify(["message", "message", "message", "custom", "message", "custom"]),
  `entry types in order: ${JSON.stringify(kinds)}`,
);
const closer = full.getEntries().find(
  (e) => e.type === "message" && e.message?.role === "toolResult" && e.message.toolCallId === "call_2",
);
check(
  closer?.message?.isError === true &&
    String(closer?.message?.content?.[0]?.text ?? "").startsWith("Tool execution was interrupted"),
  "repair toolResult reads as a stock pi toolResult (isError, interrupted text)",
);

// 2. Context parity: custom entries never project.
const ctxFull = JSON.stringify(full.buildSessionContext());
const ctxBare = JSON.stringify(bare.buildSessionContext());
check(ctxFull === ctxBare, `buildSessionContext identical to the custom-entry-free twin (${ctxFull.length} chars)`);
check(
  bare.getEntryCount() === full.getEntryCount() - customCount,
  `twin entry count: ${bare.getEntryCount()} === ${full.getEntryCount()} - ${customCount}`,
);
check(
  ctxFull.includes("call_2") && ctxFull.includes("interrupted before completion"),
  "the closed tool call participates in context (balanced transcript)",
);

// 3. Torn tail tolerance: the state after a crash between the "\n" repair append and a
//    skipped fragment stays readable with the same entry count.
const torn = open("torn.jsonl", repaired + '{"type":"mess');
check(
  torn.getEntryCount() === full.getEntryCount(),
  `torn last line skipped by pi: ${torn.getEntryCount()} entries`,
);

rmSync(work, { recursive: true, force: true });
if (failed.length > 0) {
  console.error(`\n${failed.length} check(s) failed`);
  process.exit(1);
}
console.log("\nall checks passed");
