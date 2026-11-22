import type { Fleet, FleetNode, SuperagentSession, SuperagentStatus } from './api';

/**
 * The agent canvas, minus the DOM (OIN-77 spec): the fleet the host reports (or, on an older
 * host, what the status + history calls already say), grouped per project, filtered, laid out
 * with one pure function, and the view maths. Nothing here reads the view or the DOM, so it is
 * all tested without a renderer.
 */

export type FleetStatus = FleetNode['status'];
export type Density = 'compact' | 'detailed';
export const STATUSES: readonly FleetStatus[] = ['working', 'waiting', 'stalled', 'idle', 'ended'];
export const STATUS_LABEL: Record<FleetStatus, string> = { working: 'Working', waiting: 'Needs you', stalled: 'Stalled', idle: 'Idle', ended: 'Done' };
export const MODE_LABEL: Record<FleetNode['mode'], string> = { plain: 'Plain', graph: 'Graph', swarm: 'Swarm' };

/** What the toolbar filters on. `statuses: []` means every status; `hidden` holds the group keys the owner unchecked. */
export type CanvasFilter = { statuses: FleetStatus[]; chats: boolean; hidden: string[] };
export const DEFAULT_FILTER: CanvasFilter = { statuses: [], chats: true, hidden: [] };
export const filterActive = (f: CanvasFilter) => f.statuses.length > 0 || !f.chats || f.hidden.length > 0;

export const OTHER_GROUP = 'other';
/** The Superagent's own band (the router chats) is not a project group. */
export const BAND = '__band';
/** The fallback lists at most this many finished sessions: a host with a long history would otherwise bury the canvas. */
export const FALLBACK_ENDED_MAX = 40;

// ---- Nodes ---------------------------------------------------------------------------------------

/** A node with every optional field at its default (hand-built fleets, the older-host fallback). */
export function makeNode(over: Partial<FleetNode> & Pick<FleetNode, 'id'>): FleetNode {
  return {
    parentId: null, project: null, projectId: null, groupKey: null, title: '', mode: 'plain', role: 'subagent', status: 'idle', model: null, name: null, cwd: null,
    ticketIds: [], step: null, tool: null, question: null, health: null, lastActiveAt: null, idleReason: null, startedAt: null, endedAt: null, ...over,
  };
}
export function makeFleet(over: Partial<Fleet> = {}): Fleet {
  return { nodes: [], edges: [], ...over };
}

const time = (value: string | number | null | undefined): number | null => {
  if (value === null || value === undefined) return null;
  const t = typeof value === 'number' ? value : Date.parse(value);
  return Number.isFinite(t) ? t : null;
};

/**
 * Leads from the status, running history rows as working, the rest of the history as ended.
 * No subagents: the old host never said who spawned what, and nothing is invented.
 */
export function fallbackFleet(status: SuperagentStatus | undefined, history: readonly SuperagentSession[] | undefined): Fleet {
  const nodes = new Map<string, FleetNode>();
  const waiting = new Set<string>();
  for (const item of status?.needsYou ?? []) {
    if (item.sessionId) waiting.add(item.sessionId);
    if (item.workerSessionId) waiting.add(item.workerSessionId);
  }
  const projectName = (id: string) => status?.projects?.find((p) => p.projectId === id)?.name || id;
  const projectIdOf = (name: string | null | undefined) => (name ? status?.projects?.find((p) => (p.name || p.projectId) === name)?.projectId ?? null : null);
  for (const lead of status?.leads ?? []) {
    nodes.set(lead.sessionId, makeNode({
      id: lead.sessionId, project: projectName(lead.projectId), projectId: lead.projectId, title: 'Lead',
      mode: lead.mode === 'graph' || lead.mode === 'swarm' ? lead.mode : 'plain', role: 'lead',
      status: waiting.has(lead.sessionId) ? 'waiting' : 'working', model: lead.model ?? null, ticketIds: lead.ticketIds, step: lead.step ?? null, tool: lead.tool ?? null,
      startedAt: time(lead.startedAt),
    }));
  }
  const ended: FleetNode[] = [];
  for (const item of history ?? []) {
    if (nodes.has(item.sessionId)) continue;
    const running = item.state?.kind === 'working' || !!item.pendingWork;
    const node = makeNode({
      id: item.sessionId, project: item.project ?? null, projectId: projectIdOf(item.project), title: item.title || 'Chat', role: 'chat',
      status: running ? (waiting.has(item.sessionId) ? 'waiting' : 'working') : 'ended', startedAt: time(item.createdAt), endedAt: running ? null : time(item.updatedAt),
    });
    if (running) nodes.set(node.id, node); else ended.push(node);
  }
  ended.sort((a, b) => (b.endedAt ?? 0) - (a.endedAt ?? 0));
  for (const node of ended.slice(0, FALLBACK_ENDED_MAX)) nodes.set(node.id, node);
  return makeFleet({ nodes: [...nodes.values()] });
}

/** Status is authoritative for active dispatcher leads, including on older hosts whose fleet marks them idle. */
export function mergeStatusLeads(fleet: Fleet, status: SuperagentStatus | undefined): Fleet {
  if (!status?.leads.length) return fleet;
  const leads = fallbackFleet(status, []).nodes;
  const nodes = new Map(fleet.nodes.map((node) => [node.id, node]));
  for (const lead of leads) {
    const existing = nodes.get(lead.id);
    nodes.set(lead.id, existing ? { ...existing, project: lead.project, projectId: lead.projectId, role: 'lead', mode: lead.mode, status: existing.status === 'working' || existing.status === 'waiting' ? existing.status : lead.status, model: existing.model ?? lead.model, endedAt: null } : lead);
  }
  return { ...fleet, nodes: [...nodes.values()] };
}

// ---- Words ---------------------------------------------------------------------------------------

/** Ignore host placeholders, which can otherwise expose a session identifier as a title. */
const usable = (title: string | undefined, id: string) => !!title && title !== id && title.toLowerCase() !== 'untitled' && title.toLowerCase() !== 'agent';

/** "Superagent", "Lead", the subagent's spec name, or a chat's own title. "Agent" is never a title. */
export function nodeTitle(node: FleetNode): string {
  if (node.role === 'superagent') return 'Superagent';
  if (node.role === 'lead') return 'Lead';
  if (node.role === 'chat') return usable(node.title, node.id) ? node.title : 'Chat';
  if (node.name?.trim()) return node.name.trim();
  return usable(node.title, node.id) ? node.title : node.role === 'worker' ? 'Worker' : 'Subagent';
}

/** 12s, 4m, 2h 05m, 3d. */
export function age(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h${m % 60 ? ` ${String(m % 60).padStart(2, '0')}m` : ''}`;
  return `${Math.floor(h / 24)}d`;
}
/** m:ss for the stale chip ("Updated 0:42 ago"). */
export const clock = (ms: number) => { const s = Math.max(0, Math.floor(ms / 1000)); return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`; };

export const elapsed = (node: FleetNode, now: number): string | null => (node.startedAt ? age((node.endedAt ?? now) - node.startedAt) : null);

/** The second compact line: "Working · 4m", "Needs you", "Stalled", "Idle", "Done 2h ago". */
export function stateText(node: FleetNode, now: number): string {
  switch (node.status) {
    case 'working': { const e = elapsed(node, now); return e ? `Working · ${e}` : 'Working'; }
    case 'waiting': return 'Needs you';
    case 'stalled': return 'Stalled';
    case 'idle': return 'Idle';
    default: return node.endedAt ? `Done ${age(now - node.endedAt)} ago` : 'Done';
  }
}

/** The detailed node's fourth row: what it is doing, asking or failing at. */
export function detailLine(node: FleetNode): string {
  if (node.status === 'waiting') return node.question?.excerpt || 'Waiting for your answer';
  if (node.status === 'stalled') return node.health?.error || 'Stopped with an error';
  if (node.status === 'working') return [node.step, node.tool].filter(Boolean).join(' · ') || 'Working';
  if (node.status === 'idle') return node.idleReason === 'unloaded' ? 'Unloaded, wakes on demand' : 'Idle';
  return 'Finished';
}

export const sizeKind = (n: FleetNode): 'superagent' | 'lead' | 'sub' | 'chat' => (n.role === 'superagent' ? 'superagent' : n.role === 'lead' ? 'lead' : n.role === 'chat' ? 'chat' : 'sub');

/** A node's role as the toolbar and the legend name it. */
export const ROLE_LABEL: Record<FleetNode['role'], string> = { superagent: 'Superagent', lead: 'Lead', worker: 'Worker', subagent: 'Subagent', chat: 'Chat' };

// ---- Tree: who sits under whom, and in which group ------------------------------------------------

export type FleetGroup = { key: string; name: string; hasBoard: boolean; roots: string[] };
export type FleetTree = {
  nodes: Map<string, FleetNode>;
  /** Effective parent after orphan re-attachment; null for a root. */
  parent: Map<string, string | null>;
  /** Ordered by `startedAt` ascending, then id. */
  children: Map<string, string[]>;
  /** Group key of every node (`BAND` for the Superagent and the router chats). */
  group: Map<string, string>;
  /** Groups in display order, without the band. */
  groups: FleetGroup[];
  /** The Superagent node, and the other router chats ("+N earlier chats"). */
  superagentId: string | null;
  earlier: string[];
  /** True when the host sends groups, a Superagent id or a rev: an older host's payload has none of them. */
  modern: boolean;
  hasEdges: boolean;
};

export function dedupeNodes(nodes: readonly FleetNode[]): FleetNode[] {
  const byId = new Map<string, FleetNode>();
  for (const node of nodes) byId.set(node.id, node); // the later report wins, the first position stays
  return [...byId.values()];
}

export function fleetCounts(nodes: readonly FleetNode[]): Record<FleetStatus | 'all', number> {
  const out: Record<FleetStatus | 'all', number> = { working: 0, waiting: 0, stalled: 0, idle: 0, ended: 0, all: 0 };
  for (const node of dedupeNodes(nodes)) { out[node.status]++; out.all++; }
  return out;
}

const byStart = (a: FleetNode, b: FleetNode) => (a.startedAt ?? Infinity) - (b.startedAt ?? Infinity) || (a.id < b.id ? -1 : a.id > b.id ? 1 : 0);
const byName = (a: { name: string; key: string }, b: { name: string; key: string }) => a.name.localeCompare(b.name, undefined, { sensitivity: 'base' }) || a.key.localeCompare(b.key);

export function buildTree(fleet: Pick<Fleet, 'nodes'> & Partial<Pick<Fleet, 'edges' | 'groups' | 'superagentId' | 'rev'>>): FleetTree {
  const list = dedupeNodes(fleet.nodes);
  const nodes = new Map(list.map((n) => [n.id, n]));
  const modern = !!(fleet.groups?.length || fleet.superagentId || fleet.rev);
  const edgeParent = new Map<string, string>();
  for (const e of fleet.edges ?? []) if (nodes.has(e.from) && nodes.has(e.to) && e.from !== e.to && !edgeParent.has(e.to)) edgeParent.set(e.to, e.from);

  const parent = new Map<string, string | null>();
  for (const n of list) {
    const declared = n.parentId || edgeParent.get(n.id) || null;
    parent.set(n.id, declared && declared !== n.id && nodes.has(declared) ? declared : null);
  }
  // A cycle has no root: cut it at the node the walk started from.
  for (const n of list) {
    const seen = new Set<string>([n.id]);
    for (let p = parent.get(n.id); p; p = parent.get(p)) {
      if (seen.has(p)) { if (p === n.id) parent.set(n.id, null); break; }
      seen.add(p);
    }
  }
  const rootOf = (id: string) => { let cur = id; const seen = new Set<string>(); while (parent.get(cur) && !seen.has(cur)) { seen.add(cur); cur = parent.get(cur)!; } return cur; };

  const superagentId = fleet.superagentId && nodes.has(fleet.superagentId) ? fleet.superagentId : list.find((n) => n.role === 'superagent')?.id ?? null;
  const ownGroup = (n: FleetNode): string => {
    if (n.id === superagentId || n.role === 'superagent') return BAND;
    if (!modern) return n.projectId || OTHER_GROUP;
    if (n.groupKey) return n.groupKey;
    return n.role === 'chat' && !n.projectId ? BAND : n.projectId || OTHER_GROUP;
  };
  const group = new Map<string, string>();
  // A subagent inherits its root's group; the band's own sessions stay in the band.
  for (const n of list) group.set(n.id, ownGroup(nodes.get(rootOf(n.id))!));

  const children = new Map<string, string[]>(list.map((n) => [n.id, []]));
  for (const n of [...list].sort(byStart)) { const p = parent.get(n.id); if (p) children.get(p)!.push(n.id); }

  const info = new Map<string, { name: string; hasBoard: boolean }>();
  for (const g of fleet.groups ?? []) info.set(g.key, { name: g.name || g.key, hasBoard: g.hasBoard });
  const keys: string[] = [];
  const seen = new Set<string>();
  const used = new Set<string>();
  for (const [id, k] of group) if (k !== BAND && !!nodes.get(id)) used.add(k);
  for (const g of fleet.groups ?? []) if (used.has(g.key) && !seen.has(g.key)) { keys.push(g.key); seen.add(g.key); }
  // Groups the host did not list (an older host): board projects A-Z, then the rest, Other chats last.
  const extra: { key: string; name: string; hasBoard: boolean }[] = [];
  for (const k of used) {
    if (seen.has(k) || k === OTHER_GROUP) continue;
    const sample = list.find((n) => group.get(n.id) === k && !parent.get(n.id)) ?? list.find((n) => group.get(n.id) === k)!;
    const hasBoard = !!sample.projectId && sample.projectId === k;
    info.set(k, { name: sample.project || k, hasBoard });
    extra.push({ key: k, name: sample.project || k, hasBoard });
  }
  extra.sort((a, b) => Number(info.get(b.key)!.hasBoard) - Number(info.get(a.key)!.hasBoard) || byName(a, b));
  for (const e of extra) { keys.push(e.key); seen.add(e.key); }
  if (used.has(OTHER_GROUP) && !seen.has(OTHER_GROUP)) { keys.push(OTHER_GROUP); info.set(OTHER_GROUP, { name: 'Other chats', hasBoard: false }); }
  // 'other' is always last, whatever order the host listed it in.
  const ordered = [...keys.filter((k) => k !== OTHER_GROUP), ...keys.filter((k) => k === OTHER_GROUP)];
  const groups: FleetGroup[] = ordered.map((key) => ({
    key, name: key === OTHER_GROUP ? 'Other chats' : info.get(key)?.name ?? key, hasBoard: info.get(key)?.hasBoard ?? false,
    roots: list.filter((n) => !parent.get(n.id) && group.get(n.id) === key).sort(byStart).map((n) => n.id),
  }));
  const earlier = list.filter((n) => group.get(n.id) === BAND && !parent.get(n.id) && n.id !== superagentId).sort((a, b) => (b.startedAt ?? 0) - (a.startedAt ?? 0)).map((n) => n.id);
  return { nodes, parent, children, group, groups, superagentId, earlier, modern, hasEdges: (fleet.edges?.length ?? 0) > 0 || [...parent.values()].some(Boolean) };
}

// ---- Filter and search ---------------------------------------------------------------------------------

export type Filtered = { visible: Set<string>; dimmed: Set<string>; matches: number; hiddenGroups: number };

const inChatTree = (tree: FleetTree, id: string) => { for (let cur: string | null | undefined = id; cur; cur = tree.parent.get(cur)) if (tree.nodes.get(cur)?.role === 'chat') return true; return false; };

/** Matching nodes plus their ancestors (dimmed), so a kept node is never cut loose from its tree. */
export function filterTree(tree: FleetTree, filter: CanvasFilter): Filtered {
  const visible = new Set<string>();
  const dimmed = new Set<string>();
  const matched = new Set<string>();
  const statusOk = (n: FleetNode) => filter.statuses.length === 0 || filter.statuses.includes(n.status);
  for (const [id, node] of tree.nodes) {
    const g = tree.group.get(id)!;
    if (g !== BAND && filter.hidden.includes(g)) continue;
    if (!filter.chats && inChatTree(tree, id)) continue;
    if (!statusOk(node)) continue;
    matched.add(id); visible.add(id);
    for (let p = tree.parent.get(id); p && !visible.has(p); p = tree.parent.get(p)) { visible.add(p); dimmed.add(p); }
  }
  for (const id of [...dimmed]) if (matched.has(id)) dimmed.delete(id);
  // The Superagent is the hub: it stays, dimmed when the filter leaves it out.
  if (tree.superagentId && !visible.has(tree.superagentId)) { visible.add(tree.superagentId); dimmed.add(tree.superagentId); }
  const shown = new Set<string>();
  for (const id of visible) { const g = tree.group.get(id)!; if (g !== BAND) shown.add(g); }
  return { visible, dimmed, matches: matched.size, hiddenGroups: tree.groups.filter((g) => !shown.has(g.key)).length };
}

/** Search: title, name, project or ticket id. */
export function searchMatches(tree: FleetTree, query: string): Set<string> {
  const q = query.trim().toLowerCase();
  const out = new Set<string>();
  if (!q) return out;
  for (const [id, n] of tree.nodes) {
    if ([nodeTitle(n), n.title, n.name, n.project, n.projectId, ...n.ticketIds].some((v) => !!v && v.toLowerCase().includes(q))) out.add(id);
  }
  return out;
}

// ---- Layout ---------------------------------------------------------------------------------------------

export const GUTTER = 64;
export const GROUP_PAD = 20;
export const GROUP_HEADER = 44;
export const LEVEL_GAP = 56;
export const BUS_DROP = 20;
const ROW_GAP = 40;
const SPINE = 14;
const CAPTION = 26;
const CORNER = 6;
export const SIBLING_GAP: Record<Density, number> = { detailed: 16, compact: 20 };
export const PER_ROW: Record<Density, number> = { detailed: 6, compact: 8 };
export const CHAT_COLS: Record<Density, number> = { detailed: 4, compact: 6 };
export const GROUP_MAX: Record<Density, number> = { detailed: 1040, compact: 1240 };
export const MIN_ROW_WIDTH = 1200;
export const MAX_ROW_WIDTH = 2800;
export const ORB: Record<ReturnType<typeof sizeKind>, number> = { superagent: 72, lead: 60, sub: 44, chat: 48 };
const DETAILED: Record<ReturnType<typeof sizeKind>, { w: number; h: number }> = { superagent: { w: 264, h: 96 }, lead: { w: 248, h: 112 }, sub: { w: 216, h: 76 }, chat: { w: 232, h: 76 } };
export const LABEL_W = 128;
const LABEL_H = 40;
export const boxOf = (n: FleetNode, d: Density): { w: number; h: number } => {
  const k = sizeKind(n);
  return d === 'detailed' ? DETAILED[k] : { w: LABEL_W, h: ORB[k] + LABEL_H };
};
const PSEUDO: Record<Density, { w: number; h: number }> = { detailed: { w: 216, h: 44 }, compact: { w: 128, h: 44 } };

export type Rect = { x: number; y: number; w: number; h: number };
export type NodeCounts = { working: number; waiting: number; stalled: number; idle: number; ended: number };
export type PlacedGroup = Rect & { key: string; name: string; hasBoard: boolean; collapsed: boolean; counts: NodeCounts; chatsLabel: { x: number; y: number } | null };
/** `fold` is the "+N done" stack, `earlier` the "+N earlier chats" stack. */
export type PlacedNode = Rect & { id: string; group: string; depth: number; parent: string | null; pseudo?: 'fold' | 'earlier'; count?: number; open?: boolean; owner?: string };
export type PlacedEdge = { id: string; from: string; to: string; d: string; kind: 'base' | 'flow'; status: FleetStatus; dimmed: boolean; box: Rect };
export type Layout = { width: number; height: number; bounds: Rect; band: Rect | null; groups: PlacedGroup[]; nodes: PlacedNode[]; edges: PlacedEdge[]; hiddenGroups: number; matches: number };

type Pt = [number, number];
/** A polyline with rounded corners (radius capped by half of each neighbouring segment). */
export function roundedPath(points: Pt[], radius = CORNER): string {
  const pts = points.filter((p, i) => i === 0 || p[0] !== points[i - 1]![0] || p[1] !== points[i - 1]![1]);
  const clean = pts.filter((p, i) => {
    if (i === 0 || i === pts.length - 1) return true;
    const a = pts[i - 1]!; const b = pts[i + 1]!;
    return !((a[0] === p[0] && p[0] === b[0]) || (a[1] === p[1] && p[1] === b[1]));
  });
  if (clean.length < 2) return '';
  let d = `M${clean[0]![0]} ${clean[0]![1]}`;
  for (let i = 1; i < clean.length - 1; i++) {
    const a = clean[i - 1]!; const p = clean[i]!; const b = clean[i + 1]!;
    const r = Math.min(radius, Math.hypot(p[0] - a[0], p[1] - a[1]) / 2, Math.hypot(b[0] - p[0], b[1] - p[1]) / 2);
    const ux = Math.sign(p[0] - a[0]); const uy = Math.sign(p[1] - a[1]); const vx = Math.sign(b[0] - p[0]); const vy = Math.sign(b[1] - p[1]);
    d += ` L${p[0] - ux * r} ${p[1] - uy * r} Q${p[0]} ${p[1]} ${p[0] + vx * r} ${p[1] + vy * r}`;
  }
  const last = clean[clean.length - 1]!;
  return `${d} L${last[0]} ${last[1]}`;
}
const boxOfPoints = (pts: Pt[]): Rect => {
  const xs = pts.map((p) => p[0]); const ys = pts.map((p) => p[1]);
  const x = Math.min(...xs); const y = Math.min(...ys);
  return { x, y, w: Math.max(...xs) - x, h: Math.max(...ys) - y };
};

/** The measured subtree: the node (or pseudo node) on top, its children in wrapped rows below. */
type M = {
  id: string; pseudo?: 'fold' | 'earlier'; count?: number; open?: boolean; owner?: string;
  nodeW: number; nodeH: number; w: number; h: number; margin: number; blockW: number;
  rows: { w: number; h: number; items: M[] }[];
};

/** Greedy rows of at most `per` items that fit `cap`; a lone wider item gets its own row. */
function pack(items: M[], per: number, cap: number, gap: number): { w: number; h: number; items: M[] }[] {
  const rows: { w: number; h: number; items: M[] }[] = [];
  let cur: M[] = []; let w = 0;
  const flush = () => { if (cur.length) rows.push({ w, h: Math.max(...cur.map((m) => m.h)), items: cur }); cur = []; w = 0; };
  for (const m of items) {
    const next = cur.length ? w + gap + m.w : m.w;
    if (cur.length && (cur.length >= per || next > cap)) flush();
    w = cur.length ? w + gap + m.w : m.w;
    cur.push(m);
  }
  flush();
  return rows;
}

const countOf = (tree: FleetTree, key: string): NodeCounts => {
  const out: NodeCounts = { working: 0, waiting: 0, stalled: 0, idle: 0, ended: 0 };
  for (const [id, n] of tree.nodes) if (tree.group.get(id) === key) out[n.status]++;
  return out;
};
/** The header's width, estimated: the lead's trunk must cross the header to the right of its text. */
export function headerWidth(name: string, hasBoard: boolean, counts: NodeCounts): number {
  const shown = (['working', 'waiting', 'stalled', 'idle'] as const).filter((k) => counts[k] > 0).length;
  return 28 + name.length * 8.6 + (hasBoard ? 0 : 76) + shown * 38 + (counts.ended > 0 ? 62 : 0);
}

export function layoutFleet(tree: FleetTree, filter: CanvasFilter, density: Density, expanded: ReadonlySet<string>, wrapWidth: number, collapsed: ReadonlySet<string> = new Set(), prefiltered?: Filtered): Layout {
  const filtered = prefiltered ?? filterTree(tree, filter);
  const { visible, dimmed } = filtered;
  const gap = SIBLING_GAP[density];
  const capW = GROUP_MAX[density] - 2 * GROUP_PAD;
  const rowW = Math.max(wrapWidth, 0);
  const pseudo = PSEUDO[density];

  const endedMemo = new Map<string, boolean>();
  const allEnded = (id: string): boolean => {
    let v = endedMemo.get(id);
    if (v === undefined) { v = tree.nodes.get(id)!.status === 'ended' && (tree.children.get(id) ?? []).filter((c) => visible.has(c)).every(allEnded); endedMemo.set(id, v); }
    return v;
  };

  function measure(id: string, per = PER_ROW[density], cap = capW): M {
    const nb = boxOf(tree.nodes.get(id)!, density);
    const kids = (tree.children.get(id) ?? []).filter((c) => visible.has(c));
    const foldable = kids.filter(allEnded);
    const folds = foldable.length >= 4;
    const open = folds && expanded.has(id);
    const items: M[] = [];
    for (const k of kids) if (!(folds && !open && foldable.includes(k))) items.push(measure(k, per, cap));
    if (folds) items.push({ id: `fold:${id}`, pseudo: 'fold', count: foldable.length, open, owner: id, nodeW: pseudo.w, nodeH: pseudo.h, w: pseudo.w, h: pseudo.h, margin: 0, blockW: pseudo.w, rows: [] });
    const rows = pack(items, per, cap, gap);
    const blockW = Math.max(0, ...rows.map((r) => r.w));
    const margin = rows.length > 1 ? SPINE : 0;
    const w = Math.max(nb.w, blockW + margin);
    const h = rows.length ? nb.h + LEVEL_GAP + rows.reduce((s, r) => s + r.h, 0) + ROW_GAP * (rows.length - 1) : nb.h;
    return { id, nodeW: nb.w, nodeH: nb.h, w, h, margin, blockW, rows };
  }

  // ---- Phase 1: size every group ---------------------------------------------------------------------
  type Plan = { key: string; name: string; hasBoard: boolean; collapsed: boolean; counts: NodeCounts; w: number; h: number; leadRows: { w: number; h: number; items: M[] }[]; chatRows: { w: number; h: number; items: M[] }[]; chatsLabel: boolean; x: number; y: number; shelf: number };
  const plans: Plan[] = [];
  for (const g of tree.groups) {
    const roots = g.roots.filter((r) => visible.has(r));
    if (!roots.length) continue;
    const isCollapsed = collapsed.has(g.key);
    const counts = countOf(tree, g.key);
    const hdr = headerWidth(g.name, g.hasBoard, counts);
    const hasLead = roots.some((r) => tree.nodes.get(r)!.role === 'lead');
    let minW = hdr + 2 * GROUP_PAD + 120;
    if (hasLead) minW = Math.max(minW, 2 * (GROUP_PAD + hdr + 24));
    if (isCollapsed) { plans.push({ key: g.key, name: g.name, hasBoard: g.hasBoard, collapsed: true, counts, w: Math.max(minW, 280), h: GROUP_HEADER, leadRows: [], chatRows: [], chatsLabel: false, x: 0, y: 0, shelf: 0 }); continue; }
    const leadMs = roots.filter((r) => tree.nodes.get(r)!.role !== 'chat').map((r) => measure(r));
    const chatMs = roots.filter((r) => tree.nodes.get(r)!.role === 'chat').map((r) => measure(r, PER_ROW[density], capW));
    const leadRows = pack(leadMs, 99, capW, gap * 2);
    const chatRows = pack(chatMs, CHAT_COLS[density], capW, gap);
    const contentW = Math.max(0, ...leadRows.map((r) => r.w), ...chatRows.map((r) => r.w));
    const chatsLabel = leadRows.length > 0 && chatRows.length > 0;
    let h = GROUP_HEADER;
    h += leadRows.reduce((s, r) => s + r.h, 0) + Math.max(0, leadRows.length - 1) * ROW_GAP;
    if (chatsLabel) h += ROW_GAP / 2 + CAPTION;
    else if (leadRows.length && chatRows.length) h += ROW_GAP;
    h += chatRows.reduce((s, r) => s + r.h, 0) + Math.max(0, chatRows.length - 1) * gap;
    h += GROUP_PAD;
    plans.push({ key: g.key, name: g.name, hasBoard: g.hasBoard, collapsed: false, counts, w: Math.max(contentW + 2 * GROUP_PAD, minW), h, leadRows, chatRows, chatsLabel, x: 0, y: 0, shelf: 0 });
  }

  // ---- The Superagent band ----------------------------------------------------------------------------
  const sa = tree.superagentId && visible.has(tree.superagentId) ? tree.superagentId : null;
  const saM = sa ? measure(sa) : null;
  const earlierIds = tree.earlier.filter((id) => visible.has(id));
  const earlierOpen = expanded.has('earlier');
  let earlierRows: { w: number; h: number; items: M[] }[] = [];
  if (sa && earlierIds.length) {
    const items: M[] = [{ id: 'earlier:chats', pseudo: 'earlier', count: earlierIds.length, open: earlierOpen, nodeW: pseudo.w, nodeH: pseudo.h, w: pseudo.w, h: pseudo.h, margin: 0, blockW: pseudo.w, rows: [] }];
    if (earlierOpen) for (const id of earlierIds) items.push(measure(id, PER_ROW[density], capW));
    earlierRows = pack(items, density === 'detailed' ? 3 : 4, capW, gap);
  }
  const earlierW = Math.max(0, ...earlierRows.map((r) => r.w));
  const earlierH = earlierRows.reduce((s, r) => s + r.h, 0) + Math.max(0, earlierRows.length - 1) * gap;
  const saKids = !!saM && saM.rows.length > 0;
  const bandH = saM ? Math.max(saM.h, earlierH) : 0;

  // ---- Shelf packing (fixed order, status never reorders) -----------------------------------------------
  let cx = GUTTER; let shelf = 0; let shelfTop = saM ? bandH + GUTTER : 0; let shelfH = 0;
  const shelfTops: number[] = [];
  const shelfExtent: { from: number; to: number }[] = [];
  for (const p of plans) {
    if (cx > GUTTER && cx - GUTTER + p.w > rowW) { shelfTop += shelfH + GUTTER; shelfH = 0; cx = GUTTER; shelf++; }
    if (shelfTops[shelf] === undefined) { shelfTops[shelf] = shelfTop; shelfExtent[shelf] = { from: cx, to: cx }; }
    p.x = cx; p.y = shelfTop; p.shelf = shelf;
    shelfH = Math.max(shelfH, p.h);
    cx += p.w; shelfExtent[shelf]!.to = cx; cx += GUTTER;
  }

  // The band sits centred over the first row; if it would hang off the left edge, everything shifts right.
  const row1 = shelfExtent[0];
  const centreX = row1 ? (row1.from + row1.to) / 2 : GUTTER + (saM?.w ?? 0) / 2;
  const leftCluster = earlierRows.length ? earlierW + 40 : 0;
  const saLeft0 = centreX - (saM?.w ?? 0) / 2;
  const dx = saM ? Math.max(0, GUTTER + leftCluster - saLeft0) : 0;

  const nodes: PlacedNode[] = []; const edges: PlacedEdge[] = []; const placedGroups: PlacedGroup[] = [];
  const at = new Map<string, PlacedNode>();
  const stateOf = (id: string) => tree.nodes.get(id)?.status ?? 'ended';
  const isDim = (id: string) => dimmed.has(id);

  const addEdge = (id: string, from: string, to: string, status: FleetStatus, dim: boolean, pts: Pt[]) => {
    const d = roundedPath(pts);
    if (!d) return;
    const box = boxOfPoints(pts);
    edges.push({ id: `${id}:base`, from, to, d, kind: 'base', status: 'ended', dimmed: dim, box });
    if (status !== 'ended') edges.push({ id: `${id}:flow`, from, to, d, kind: 'flow', status, dimmed: dim, box });
  };

  function emit(m: M, ax: number, ay: number, parent: string | null, groupKey: string, depth: number) {
    const placed: PlacedNode = { id: m.id, x: ax + (m.w - m.nodeW) / 2, y: ay, w: m.nodeW, h: m.nodeH, group: groupKey, depth, parent, ...(m.pseudo ? { pseudo: m.pseudo, count: m.count, open: m.open, owner: m.owner } : {}) };
    nodes.push(placed); at.set(m.id, placed);
    if (!m.rows.length) return;
    const px = ax + m.w / 2; const py = ay + m.nodeH;
    const bx = ax + m.margin + (m.w - m.margin - m.blockW) / 2;
    let ry = ay + m.nodeH + LEVEL_GAP;
    m.rows.forEach((row, r) => {
      let x = bx + (m.blockW - row.w) / 2;
      for (const item of row.items) {
        const cxm = x + item.w / 2;
        const status = item.pseudo ? 'ended' : stateOf(item.id);
        const pts: Pt[] = r === 0
          ? [[px, py], [px, py + BUS_DROP], [cxm, py + BUS_DROP], [cxm, ry]]
          : [[px, py], [px, py + BUS_DROP], [ax + SPINE / 2, py + BUS_DROP], [ax + SPINE / 2, ry - BUS_DROP], [cxm, ry - BUS_DROP], [cxm, ry]];
        addEdge(`${m.id}>${item.id}`, m.id, item.id, status, !item.pseudo && isDim(item.id), pts);
        emit(item, x, ry, m.id, groupKey, depth + 1);
        x += item.w + gap;
      }
      ry += row.h + ROW_GAP;
    });
  }

  // Band.
  let band: Rect | null = null;
  let saPort: Pt[] = [];
  if (saM && sa) {
    emit(saM, saLeft0 + dx, 0, null, BAND, 0);
    const placed = at.get(sa)!;
    // With subagents under it the trunk leaves from the side and runs past them: nothing may cross them.
    saPort = saKids
      ? [[placed.x + placed.w, placed.y + placed.h / 2], [saLeft0 + dx + saM.w + 24, placed.y + placed.h / 2]]
      : [[placed.x + placed.w / 2, placed.y + placed.h]];
    let ey = 0;
    for (const row of earlierRows) {
      let x = GUTTER;
      for (const item of row.items) { emit(item, x, ey, null, BAND, 0); x += item.w + gap; }
      ey += row.h + gap;
    }
    const mine = nodes.filter((n) => n.group === BAND);
    const bx0 = Math.min(...mine.map((n) => n.x)); const bx1 = Math.max(...mine.map((n) => n.x + n.w));
    band = { x: bx0, y: 0, w: bx1 - bx0, h: bandH };
  }

  // Groups.
  for (const p of plans) {
    const gx = p.x + dx; const gy = p.y;
    const pg: PlacedGroup = { key: p.key, name: p.name, hasBoard: p.hasBoard, collapsed: p.collapsed, counts: p.counts, x: gx, y: gy, w: p.w, h: p.h, chatsLabel: null };
    placedGroups.push(pg);
    const inner = p.w - 2 * GROUP_PAD;
    let y = gy + GROUP_HEADER;
    const ports: { id: string; x: number; y: number }[] = [];
    p.leadRows.forEach((row, i) => {
      let x = gx + GROUP_PAD + (inner - row.w) / 2;
      for (const m of row.items) {
        emit(m, x, y, null, p.key, 0);
        const n = at.get(m.id)!;
        if (tree.nodes.get(m.id)!.role === 'lead') ports.push({ id: m.id, x: n.x + n.w / 2, y: n.y });
        x += m.w + gap * 2;
      }
      y += row.h + (i < p.leadRows.length - 1 ? ROW_GAP : 0);
    });
    if (p.chatsLabel) { y += ROW_GAP / 2; pg.chatsLabel = { x: gx + GROUP_PAD, y }; y += CAPTION; } else if (p.leadRows.length && p.chatRows.length) y += ROW_GAP;
    p.chatRows.forEach((row, i) => {
      let x = gx + GROUP_PAD + (inner - row.w) / 2;
      for (const m of row.items) { emit(m, x, y, null, p.key, 0); x += m.w + gap; }
      y += row.h + (i < p.chatRows.length - 1 ? gap : 0);
    });
    // Trunk: Superagent -> the group's lead, along the gutters and never across a group. A collapsed group takes it at its top edge.
    if (sa && band) {
      const g0 = shelfTops[0]! - GUTTER / 2;
      const gk = shelfTops[p.shelf]! - GUTTER / 2;
      const chx = dx + GUTTER / 2;
      const lead = p.collapsed ? tree.groups.find((g) => g.key === p.key)!.roots.find((r) => visible.has(r) && tree.nodes.get(r)!.role === 'lead') : undefined;
      const targets = p.collapsed ? (lead ? [{ id: lead, x: gx + p.w / 2, y: gy }] : []) : ports;
      for (const t of targets) {
        const down: Pt[] = [...saPort, [saPort[saPort.length - 1]![0], g0]];
        const pts: Pt[] = p.shelf === 0 ? [...down, [t.x, g0], [t.x, t.y]] : [...down, [chx, g0], [chx, gk], [t.x, gk], [t.x, t.y]];
        addEdge(`${sa}>${t.id}`, sa, t.id, stateOf(t.id), isDim(t.id), pts);
      }
    }
  }

  const xs = [...nodes.map((n) => n.x + n.w), ...placedGroups.map((g) => g.x + g.w)];
  const ys = [...nodes.map((n) => n.y + n.h), ...placedGroups.map((g) => g.y + g.h)];
  const minX = Math.min(Infinity, ...nodes.map((n) => n.x), ...placedGroups.map((g) => g.x));
  const maxX = Math.max(0, ...xs); const maxY = Math.max(0, ...ys);
  const bounds: Rect = nodes.length || placedGroups.length ? { x: Math.min(minX, GUTTER), y: 0, w: maxX - Math.min(minX, GUTTER), h: maxY } : { x: 0, y: 0, w: 0, h: 0 };
  return { width: maxX + GUTTER, height: maxY + GUTTER, bounds, band, groups: placedGroups, nodes, edges, hiddenGroups: filtered.hiddenGroups, matches: filtered.matches };
}

// ---- Navigation over a layout --------------------------------------------------------------------------

export type Nav = { parent: Map<string, string | null>; kids: Map<string, string[]> };
/** Who is above, below and beside whom, in layout order, so the arrow keys match what the owner sees. */
export function navOf(layout: Layout): Nav {
  const parent = new Map<string, string | null>(); const kids = new Map<string, string[]>();
  for (const n of layout.nodes) { parent.set(n.id, n.parent); if (n.parent) (kids.get(n.parent) ?? kids.set(n.parent, []).get(n.parent)!).push(n.id); }
  return { parent, kids };
}

/** The next node asking for the owner, starting after `from`, in layout order. */
export function nextNeedingYou(tree: FleetTree, layout: Layout, from?: string): string | undefined {
  const ids = layout.nodes.filter((n) => !n.pseudo && tree.nodes.get(n.id)?.status === 'waiting').map((n) => n.id);
  if (!ids.length) return undefined;
  const i = from ? ids.indexOf(from) : -1;
  return ids[(i + 1) % ids.length];
}

// ---- View maths ------------------------------------------------------------------------------------------

export type View = { x: number; y: number; scale: number };
export const MIN_SCALE = 0.2;
export const MAX_SCALE = 2;
export const FIT_PADDING = 40;
/** Scale 1 with the world's origin a little inside the viewport's corner. */
export const RESET_VIEW: View = { x: 24, y: 24, scale: 1 };
export const clampScale = (s: number) => Math.min(MAX_SCALE, Math.max(MIN_SCALE, s));
/** Below this scale nodes draw as Compact; below the second, as dots. */
export const LOD_COMPACT = 0.55;
export const LOD_DOTS = 0.35;
export const lodOf = (scale: number): 0 | 1 | 2 => (scale < LOD_DOTS ? 2 : scale < LOD_COMPACT ? 1 : 0);

/** Everything visible, centred, never magnified past 1; no lower floor, so nothing is ever off-screen. */
export function fitView(content: { x?: number; y?: number; width?: number; height?: number; w?: number; h?: number }, viewport: { width: number; height: number }, padding = FIT_PADDING): View {
  const cw = content.w ?? content.width ?? 0; const ch = content.h ?? content.height ?? 0;
  const cx = content.x ?? 0; const cy = content.y ?? 0;
  if (cw <= 0 || ch <= 0 || viewport.width <= 0 || viewport.height <= 0) return RESET_VIEW;
  const scale = clampScale(Math.min(1, (viewport.width - 2 * padding) / cw, (viewport.height - 2 * padding) / ch));
  return { scale, x: (viewport.width - cw * scale) / 2 - cx * scale, y: Math.max(padding, (viewport.height - ch * scale) / 2) - cy * scale };
}

/** Zoom keeping the world point under (cx, cy) fixed. */
export function zoomAt(view: View, factor: number, cx: number, cy: number): View {
  const scale = clampScale(view.scale * factor);
  const k = scale / view.scale;
  return { scale, x: cx - (cx - view.x) * k, y: cy - (cy - view.y) * k };
}

/** The world rectangle a viewport shows, grown by `margin` of its size on each side (culling). */
export function worldRect(view: View, viewport: { width: number; height: number }, margin = 0.25): Rect {
  const w = viewport.width / view.scale; const h = viewport.height / view.scale;
  return { x: -view.x / view.scale - w * margin, y: -view.y / view.scale - h * margin, w: w * (1 + 2 * margin), h: h * (1 + 2 * margin) };
}
export const intersects = (a: Rect, b: Rect) => a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h;

/** The view that centres `box` in the viewport at the current scale, only when it is not already comfortably inside `inner`. */
export function panToReveal(view: View, box: Rect, viewport: { width: number; height: number }, inner: { left?: number; right?: number } = {}): View | null {
  const left = box.x * view.scale + view.x; const top = box.y * view.scale + view.y;
  const right = left + box.w * view.scale; const bottom = top + box.h * view.scale;
  const pad = 24;
  const ok = left >= pad + (inner.left ?? 0) && right <= viewport.width - pad - (inner.right ?? 0) && top >= pad && bottom <= viewport.height - pad;
  if (ok) return null;
  return { ...view, x: viewport.width / 2 - (box.x + box.w / 2) * view.scale, y: viewport.height / 2 - (box.y + box.h / 2) * view.scale };
}

// ---- Opening a node -----------------------------------------------------------------------------------------

/**
 * The session a click opens. The Superagent, leads and chats open themselves; a subagent or worker has no chat of its own
 * in the Superagent pane, so it opens the nearest session above it (itself when it is a stray root).
 */
export function openTarget(tree: FleetTree, id: string): string {
  for (let cur: string | null | undefined = id; cur; cur = tree.parent.get(cur)) {
    const role = tree.nodes.get(cur)?.role;
    if (role !== 'subagent' && role !== 'worker') return cur;
  }
  return id;
}
