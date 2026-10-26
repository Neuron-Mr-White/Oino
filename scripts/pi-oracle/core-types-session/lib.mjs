// Shared helpers for the core-types-session pi oracle (pi-coding-agent 1.4.0).
// PI_CODING_AGENT_PKG overrides the package root; default is the mise global install.
import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import { pathToFileURL } from "node:url";

const defaultPkg = join(
  homedir(),
  ".local/share/mise/installs/node/24.16.0/lib/node_modules/@earendil-works/pi-coding-agent",
);

export const PI_PKG = process.env.PI_CODING_AGENT_PKG ?? defaultPkg;

export async function loadSessionManager() {
  const file = join(PI_PKG, "dist/core/session-manager.js");
  if (!existsSync(file)) throw new Error(`pi session-manager.js not found at ${file}`);
  return import(pathToFileURL(file).href);
}

/** Everything the Rust golden test compares for one session file. */
export function snapshot(sm) {
  const entries = sm.getEntries();
  const childCount = new Map();
  for (const e of entries) {
    if (e.parentId) childCount.set(e.parentId, (childCount.get(e.parentId) ?? 0) + 1);
  }
  const leaves = entries.filter((e) => !childCount.has(e.id)).map((e) => e.id);
  const original = sm.getLeafId();
  const perLeaf = leaves.map((leaf) => {
    sm.branch(leaf);
    const projection = sm.buildSessionProjection();
    return {
      leaf,
      branchIds: sm.getBranch().map((e) => e.id),
      contextEntryIds: sm.buildContextEntries().map((e) => e.id),
      projection: projection.entries.map((p) => ({ id: p.sourceEntry.id, messages: p.messages })),
      context: sm.buildSessionContext(),
    };
  });
  if (original) sm.branch(original);
  // Flat pre-order tree (deep linear sessions would exceed JSON nesting limits).
  const tree = [];
  const stack = [...sm.getTree()].reverse();
  while (stack.length > 0) {
    const n = stack.pop();
    tree.push({
      id: n.entry.id,
      ...(n.label !== undefined ? { label: n.label } : {}),
      ...(n.labelTimestamp !== undefined ? { labelTimestamp: n.labelTimestamp } : {}),
      children: n.children.map((c) => c.entry.id),
    });
    stack.push(...[...n.children].reverse());
  }
  const roots = sm.getTree().map((n) => n.entry.id);
  return JSON.parse(
    JSON.stringify({
      leafId: sm.getLeafId(),
      sessionName: sm.getSessionName() ?? null,
      entryCount: sm.getEntryCount(),
      header: sm.getHeader(),
      roots,
      tree,
      labels: entries.filter((e) => sm.getLabel(e.id) !== undefined).map((e) => [e.id, sm.getLabel(e.id)]),
      leaves: perLeaf,
    }),
  );
}
