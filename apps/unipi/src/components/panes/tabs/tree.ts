/**
 * Session tree helpers (WP-A10, Z-18): the `get_tree` reply → flat rows in
 * pi's `/tree` layout. Ports the unipi 1.0 TreeSheet logic (itself a port of
 * pi's TUI `tree-selector.js`): pre-order flattening with the active branch
 * first among siblings, fork connectors, the five pi filter modes with hidden
 * rows' children re-attached to their nearest visible ancestor, and depth
 * compression past 6 levels.
 */
import { textOf, type Part } from '../../../lib/chat/model';
import type { SessionEntry } from '../../../lib/session/types';

/** One tree node as the rows see it (the unipi 1.0 `TreeNode` shape). */
export type TreeNode = {
  id: string;
  parentId: string | null;
  kind: string;
  preview: string;
  timestamp?: string;
  label?: string;
  onPath: boolean;
  current: boolean;
};

/** Max indent levels drawn before the depth is compressed into a badge. */
export const TREE_MAX_DEPTH = 6;

/** One row of the flattened tree. */
export type TreeRow = { node: TreeNode; depth: number; cells: string[] };

const KIND_OF_SETTINGS = new Set(['label', 'context_edit', 'custom', 'custom_message', 'model_change', 'thinking_level_change', 'session_info', 'branch_summary', 'compaction']);

/** The display kind of an entry (pi's kinds; settings kinds filter out by default). */
export function kindOfEntry(entry: SessionEntry): string {
  if (entry.type === 'message') {
    const role = String(entry.message?.role ?? '');
    return role === 'bashExecution' ? 'bash' : role;
  }
  if (entry.type === 'compaction') return 'compaction';
  if (entry.type === 'branch_summary') return 'branch_summary';
  if (entry.type === 'model_change' || entry.type === 'thinking_level_change') return entry.type;
  if (entry.type === 'custom_message') return 'custom_message';
  return 'custom';
}

const excerptOf = (text: string, max = 120): string => {
  const flat = text.replace(/\s+/g, ' ').trim();
  return flat.length > max ? `${flat.slice(0, max - 1)}…` : flat;
};

/**
 * The `get_tree` reply (`{tree: [{entry, children, label?}], leafId}`) → the
 * flat node list: children by timestamp, `onPath` = on the leaf's branch,
 * `current` = the leaf.
 */
export function buildTreeNodes(
  tree: Array<{ entry: SessionEntry; children?: unknown[]; label?: unknown }>,
  leafId: string | null | undefined,
): TreeNode[] {
  const out: TreeNode[] = [];
  const walk = (nodes: Array<{ entry: SessionEntry; children?: unknown[]; label?: unknown }>, parentId: string | null) => {
    for (const n of nodes) {
      const entry = n.entry;
      if (!entry || typeof entry.id !== 'string') continue;
      const preview =
        entry.type === 'compaction'
          ? excerptOf(String(entry.summary ?? 'Context compacted'))
          : entry.type === 'branch_summary'
            ? excerptOf(String((entry as { summary?: unknown }).summary ?? 'Branch summary'))
            : excerptOf(textOf(entry.message?.content as string | Part[] | undefined) || String((entry as { customType?: unknown }).customType ?? ''));
      out.push({
        id: entry.id,
        parentId,
        kind: kindOfEntry(entry),
        preview,
        ...(typeof entry.timestamp === 'string' ? { timestamp: entry.timestamp } : {}),
        ...(typeof n.label === 'string' && n.label ? { label: n.label } : {}),
        onPath: false,
        current: entry.id === leafId,
      });
      const kids = Array.isArray(n.children) ? (n.children as Array<{ entry: SessionEntry; children?: unknown[]; label?: unknown }>) : [];
      if (kids.length) walk(kids, entry.id);
    }
  };
  walk(tree, null);
  const byId = new Map(out.map((n) => [n.id, n]));
  let cur = leafId ? byId.get(leafId) : undefined;
  const path = new Set<string>();
  while (cur) {
    path.add(cur.id);
    cur = cur.parentId ? byId.get(cur.parentId) : undefined;
  }
  for (const n of out) n.onPath = path.has(n.id);
  return out;
}

/** pi /tree filter modes (tree-selector.js filterMode). */
export type TreeFilter = 'default' | 'no-tools' | 'user-only' | 'labeled-only' | 'all';
export const TREE_FILTERS: Array<{ id: TreeFilter; label: string }> = [
  { id: 'default', label: 'Default' },
  { id: 'no-tools', label: 'No tools' },
  { id: 'user-only', label: 'User' },
  { id: 'labeled-only', label: 'Labeled' },
  { id: 'all', label: 'All' },
];
export const TREE_FILTER_KEY = 'unipi.chat.treeFilter';

export function readTreeFilter(): TreeFilter {
  try {
    const v = localStorage.getItem(TREE_FILTER_KEY);
    return TREE_FILTERS.some((f) => f.id === v) ? (v as TreeFilter) : 'default';
  } catch {
    return 'default';
  }
}
export function rememberTreeFilter(mode: TreeFilter): void {
  try {
    localStorage.setItem(TREE_FILTER_KEY, mode);
  } catch {
    // ignore
  }
}

/**
 * pi's filter, then hidden rows' children re-attach to their nearest visible
 * ancestor so the layout stays a real tree. Usage entries never show, an
 * assistant row with no text hides unless it is the current leaf, and the
 * current leaf always shows.
 */
export function filterTree(nodes: TreeNode[], mode: TreeFilter): TreeNode[] {
  const keep = (n: TreeNode): boolean => {
    if (n.kind === 'usage') return false;
    if (n.current) return true;
    if (n.kind === 'assistant' && !n.preview.trim() && mode !== 'all') return false;
    const settings = KIND_OF_SETTINGS.has(n.kind);
    switch (mode) {
      case 'user-only':
        return n.kind === 'user';
      case 'no-tools':
        return !settings && n.kind !== 'toolResult';
      case 'labeled-only':
        return n.label !== undefined;
      case 'all':
        return true;
      default:
        return !settings;
    }
  };
  if (mode === 'all') return nodes.filter((n) => n.kind !== 'usage');
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const visible = new Set(nodes.filter(keep).map((n) => n.id));
  const out: TreeNode[] = [];
  for (const n of nodes) {
    if (!visible.has(n.id)) continue;
    let parent = n.parentId ? byId.get(n.parentId) : undefined;
    const guard = new Set<string>();
    while (parent && !visible.has(parent.id) && !guard.has(parent.id)) {
      guard.add(parent.id);
      parent = parent.parentId ? byId.get(parent.parentId) : undefined;
    }
    out.push(parent && visible.has(parent.id) ? (n.parentId === parent.id ? n : { ...n, parentId: parent.id }) : n.parentId === null ? n : { ...n, parentId: null });
  }
  return out;
}

/**
 * The flattening (pi's TUI layout): pre-order, the branch holding the current
 * leaf first among siblings; a fork's children step in +1 and so does the
 * first generation after a fork; single-child chains stay flat; several roots
 * hang off a virtual root. Connectors (├ └) mark each fork child and gutters
 * (│) carry a still-open sibling branch down past it.
 */
export function flattenTree(nodes: TreeNode[]): TreeRow[] {
  const byId = new Map(nodes.map((n) => [n.id, n]));
  const children = new Map<string, TreeNode[]>();
  const roots: TreeNode[] = [];
  for (const n of nodes) {
    const parent = n.parentId ? byId.get(n.parentId) : undefined;
    if (!parent) roots.push(n);
    else children.set(parent.id, [...(children.get(parent.id) ?? []), n]);
  }
  // Which subtrees hold the current leaf (iterative post-order, cycle-safe).
  const hasActive = new Map<string, boolean>();
  const order: TreeNode[] = [];
  const seen = new Set<string>();
  const pre = [...roots];
  while (pre.length) {
    const n = pre.pop()!;
    if (seen.has(n.id)) continue;
    seen.add(n.id);
    order.push(n);
    for (const c of children.get(n.id) ?? []) pre.push(c);
  }
  for (let i = order.length - 1; i >= 0; i--) {
    const n = order[i]!;
    hasActive.set(n.id, n.current || (children.get(n.id) ?? []).some((c) => hasActive.get(c.id)));
  }
  const activeFirst = (list: TreeNode[]) => [...list.filter((c) => hasActive.get(c.id)), ...list.filter((c) => !hasActive.get(c.id))];

  type Gutter = { position: number; show: boolean };
  type Item = [TreeNode, number, boolean, boolean, boolean, Gutter[], boolean];
  const multipleRoots = roots.length > 1;
  const stack: Item[] = [];
  const orderedRoots = activeFirst(roots);
  for (let i = orderedRoots.length - 1; i >= 0; i--) {
    stack.push([orderedRoots[i]!, multipleRoots ? 1 : 0, multipleRoots, multipleRoots, i === orderedRoots.length - 1, [], multipleRoots]);
  }
  const out: TreeRow[] = [];
  const emitted = new Set<string>();
  while (stack.length) {
    const [node, indent, justBranched, showConnector, isLast, gutters, virtualRootChild] = stack.pop()!;
    if (emitted.has(node.id)) continue; // malformed (cyclic) host data
    emitted.add(node.id);
    const depth = multipleRoots ? Math.max(0, indent - 1) : indent;
    const connector = showConnector && !virtualRootChild;
    const cells: string[] = [];
    for (let level = 0; level < depth; level++) {
      const gutter = gutters.find((g) => g.position === level);
      if (gutter) cells.push(gutter.show ? '│' : '');
      else if (connector && level === depth - 1) cells.push(isLast ? '└' : '├');
      else cells.push('');
    }
    out.push({ node, depth, cells });
    const kids = activeFirst(children.get(node.id) ?? []);
    const forks = kids.length > 1;
    const childIndent = forks || (justBranched && indent > 0) ? indent + 1 : indent;
    const childGutters = connector ? [...gutters, { position: Math.max(0, depth - 1), show: !isLast }] : gutters;
    for (let i = kids.length - 1; i >= 0; i--) {
      stack.push([kids[i]!, childIndent, forks, forks, i === kids.length - 1, childGutters, false]);
    }
  }
  return out;
}
